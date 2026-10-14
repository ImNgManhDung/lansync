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

    let mut f = File::open(path)?;
    let mut h = blake3::Hasher::new();
    std::io::copy(&mut f, &mut h)?;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_matches_blake3() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.txt");
        std::fs::write(&p, b"hello world").unwrap();
        assert_eq!(hash_file(&p).unwrap(), *blake3::hash(b"hello world").as_bytes());
    }

    #[test]
    fn hash_file_stable_returns_hash_when_settled() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("stable.txt");
        std::fs::write(&p, b"settled content").unwrap();
        let h = hash_file_stable(&p).unwrap();
        assert_eq!(h, Some(*blake3::hash(b"settled content").as_bytes()));
    }

    #[tokio::test]
    async fn open_with_retry_succeeds_on_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("retry.txt");
        std::fs::write(&p, b"test content").unwrap();
        let res = open_with_retry(&p, Duration::from_secs(1)).await;
        assert!(res.is_ok());
    }

    #[test]
    fn mtime_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.txt");
        std::fs::write(&p, b"x").unwrap();
        let ns: i64 = 1_700_000_000_123_456_700; // multiple of 100ns to match NTFS resolution
        filetime::set_file_mtime(&p, filetime_from_ns(ns)).unwrap();
        assert_eq!(mtime_ns(&std::fs::metadata(&p).unwrap()), ns);
    }
}
