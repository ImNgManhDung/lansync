//! File utilities: Blake3 hashing, nanosecond mtime, and robust locking/stabilization.
use crate::protocol::Hash;
use filetime::FileTime;
use std::fs::{File, Metadata};
use std::path::Path;
use std::time::{Duration, Instant, UNIX_EPOCH};

pub fn hash_file(path: &Path) -> std::io::Result<Hash> {
    let mut f = File::open(path)?;
    let mut h = blake3::Hasher::new();
    std::io::copy(&mut f, &mut h)?;
    Ok(*h.finalize().as_bytes())
}

/// Hashes file only when stable (size and mtime do not change during or before reading).
/// Returns Ok(None) if file is actively being written to, deferring to the next cycle.
pub fn hash_file_stable(path: &Path) -> std::io::Result<Option<Hash>> {
    let before_meta = std::fs::metadata(path)?;
    let before_size = before_meta.len();
    let before_mtime = mtime_ns(&before_meta);

    let mut f = match File::open(path) {
        Ok(f) => f,
        Err(e) if is_transient_lock_error(&e) => {
            // File is temporarily locked by another app (e.g. active copy on Windows)
            return Ok(None);
        }
        Err(e) => return Err(e),
    };
    let mut h = blake3::Hasher::new();
    if let Err(e) = std::io::copy(&mut f, &mut h) {
        if is_transient_lock_error(&e) {
            return Ok(None);
        }
        return Err(e);
    }

    let after_meta = std::fs::metadata(path)?;
    if after_meta.len() != before_size || mtime_ns(&after_meta) != before_mtime {
        // File was modified during hashing (e.g. large file copy or active append)
        return Ok(None);
    }

    Ok(Some(*h.finalize().as_bytes()))
}

/// Attempts to open a file with exponential backoff if temporarily locked by another application.
/// Backoff starts at 20ms and caps at max_timeout.
pub async fn open_with_retry(path: &Path, max_timeout: Duration) -> std::io::Result<File> {
    let start = Instant::now();
    let mut delay = Duration::from_millis(20);
    let max_delay = Duration::from_millis(500);

    loop {
        match File::open(path) {
            Ok(file) => return Ok(file),
            Err(e) if is_transient_lock_error(&e) && start.elapsed() < max_timeout => {
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(max_delay);
            }
            Err(e) => return Err(e),
        }
    }
}

pub fn is_transient_lock_error(err: &std::io::Error) -> bool {
    if matches!(err.kind(), std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::WouldBlock) {
        return true;
    }
    #[cfg(windows)]
    {
        if let Some(raw) = err.raw_os_error() {
            return raw == 32 || raw == 33;
        }
    }
    false
}

pub fn mtime_ns(meta: &Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

pub fn filetime_from_ns(ns: i64) -> FileTime {
    FileTime::from_unix_time(ns.div_euclid(1_000_000_000), ns.rem_euclid(1_000_000_000) as u32)
}

#[cfg(unix)]
pub fn available_disk_space(path: &Path) -> std::io::Result<u64> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let target = if path.exists() {
        path.to_path_buf()
    } else {
        path.parent().map(Path::to_path_buf).unwrap_or_else(|| path.to_path_buf())
    };

    let c_path = CString::new(target.as_os_str().as_bytes())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    unsafe {
        let mut stat: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c_path.as_ptr(), &mut stat) == 0 {
            let free = (stat.f_bavail as u64).saturating_mul(stat.f_frsize as u64);
            Ok(free)
        } else {
            Err(std::io::Error::last_os_error())
        }
    }
}

#[cfg(not(unix))]
pub fn available_disk_space(_path: &Path) -> std::io::Result<u64> {
    Ok(u64::MAX)
}

/// Cleans up orphaned temporary download files (.lansync-tmp-* and .lansync-part-*)
/// older than max_age to prevent disk clutter from dropped or crashed transfers.
pub fn cleanup_orphaned_temps(root: &Path, max_age: Duration) -> std::io::Result<usize> {
    let mut cleaned = 0;
    let Ok(entries) = std::fs::read_dir(root) else {
        return Ok(0);
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let is_temp = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|name| name.starts_with(".lansync-tmp-") || name.starts_with(".lansync-part-"));

        if is_temp {
            let is_expired = entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|elapsed| elapsed >= max_age);

            if is_expired && std::fs::remove_file(&path).is_ok() {
                cleaned += 1;
            }
        }
    }
    Ok(cleaned)
}

