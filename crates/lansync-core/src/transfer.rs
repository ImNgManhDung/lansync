//! File transfer over temporary files with hash validation and atomic rename.
use crate::fsutil::{filetime_from_ns, hash_file, mtime_ns};
use crate::paths::{to_fs_path, validate_rel_path};
use crate::protocol::{write_msg, Entry, Hash, Message, CHUNK_SIZE};
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub async fn send_file<W: AsyncWrite + Unpin>(
    w: &mut W,
    root: &Path,
    path: &str,
    hash: &Hash,
) -> Result<bool> {
    let Some(valid_rel) = validate_rel_path(path) else {
        return Ok(false);
    };
    let full = to_fs_path(root, &valid_rel);
    let Ok(mut f) = tokio::fs::File::open(&full).await else { return Ok(false) };
    let meta = f.metadata().await?;
    write_msg(w, &Message::FileHeader { path: valid_rel, size: meta.len(), hash: *hash, mtime_ns: mtime_ns(&meta) }).await?;
    let mut buf = vec![0u8; CHUNK_SIZE];
    loop {
        let n = f.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        write_msg(w, &Message::FileChunk { data: buf[..n].to_vec() }).await?;
    }
    write_msg(w, &Message::FileEnd).await?;
    Ok(true)
}

pub async fn send_file_range<W: AsyncWrite + Unpin>(
    w: &mut W,
    root: &Path,
    path: &str,
    hash: &Hash,
    offset: u64,
) -> Result<bool> {
    let Some(valid_rel) = validate_rel_path(path) else {
        return Ok(false);
    };
    let full = to_fs_path(root, &valid_rel);
    let Ok(mut f) = tokio::fs::File::open(&full).await else { return Ok(false) };
    let meta = f.metadata().await?;
    let size = meta.len();
    if offset > size {
        return Ok(false);
    }
    use tokio::io::AsyncSeekExt;
    if offset > 0 {
        f.seek(std::io::SeekFrom::Start(offset)).await?;
    }
    write_msg(
        w,
        &Message::FileHeaderRange {
            path: valid_rel,
            size,
            hash: *hash,
            mtime_ns: mtime_ns(&meta),
            offset,
        },
    )
    .await?;
    let mut buf = vec![0u8; CHUNK_SIZE];
    loop {
        let n = f.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        write_msg(w, &Message::FileChunk { data: buf[..n].to_vec() }).await?;
    }
    write_msg(w, &Message::FileEnd).await?;
    Ok(true)
}

#[derive(Debug, thiserror::Error)]
pub enum FinishError {
    #[error("received data does not match expected hash")]
    HashMismatch,
    #[error("file on this device was modified locally")]
    LocalChanged,
    #[error("file I/O error: {0}")]
    Io(String),
}

fn io_err(e: impl std::fmt::Display) -> FinishError {
    FinishError::Io(e.to_string())
}

pub fn partial_download_path(root: &Path, dest_rel: &str, hash: &Hash) -> Option<PathBuf> {
    let safe_rel = validate_rel_path(dest_rel)?;
    let dest = to_fs_path(root, &safe_rel);
    let dir = dest.parent().map(Path::to_path_buf).unwrap_or_else(|| root.to_path_buf());
    let hex_hash = data_encoding::HEXLOWER.encode(hash);
    Some(dir.join(format!(".lansync-part-{hex_hash}.tmp")))
}

pub async fn check_resume_offset(root: &Path, dest_rel: &str, hash: &Hash, expected_size: u64) -> u64 {
    let Some(part_file) = partial_download_path(root, dest_rel, hash) else {
        return 0;
    };
    let Ok(meta) = tokio::fs::metadata(&part_file).await else {
        return 0;
    };
    let part_len = meta.len();
    if part_len == 0 || part_len >= expected_size {
        return 0;
    }
    part_len
}

/// Incoming file transfer. Writes to `.lansync-part-*` or `.lansync-tmp-*` in destination directory.
pub struct Incoming {
    root: PathBuf,
    dest_rel: String,
    tmp: PathBuf,
    file: Option<tokio::fs::File>,
    hasher: blake3::Hasher,
    size: u64,
    received: u64,
    hash: Hash,
    mtime_ns: i64,
    error: Option<String>,
    done: bool,
    is_resumable: bool,
}

async fn verify_partial_offset(path: &Path, offset: u64, hasher: &mut blake3::Hasher) -> bool {
    let Ok(meta) = tokio::fs::metadata(path).await else { return false };
    if meta.len() != offset { return false };
    let Ok(mut existing_f) = tokio::fs::File::open(path).await else { return false };
    let mut buf = vec![0u8; CHUNK_SIZE];
    let mut read_total = 0;
    while read_total < offset {
        let to_read = std::cmp::min(buf.len() as u64, offset - read_total) as usize;
        match existing_f.read(&mut buf[..to_read]).await {
            Ok(0) => return false,
            Ok(n) => {
                hasher.update(&buf[..n]);
                read_total += n as u64;
            }
            Err(_) => return false,
        }
    }
    read_total == offset
}

impl Incoming {
    pub async fn begin_resumable(
        root: &Path,
        dest_rel: &str,
        size: u64,
        hash: Hash,
        mtime_ns: i64,
        offset: u64,
    ) -> Self {
        let safe_rel = match validate_rel_path(dest_rel) {
            Some(p) => p,
            None => {
                return Self {
                    root: root.to_path_buf(),
                    dest_rel: dest_rel.to_string(),
                    tmp: root.join(format!(".lansync-tmp-{:016x}", rand::random::<u64>())),
                    file: None,
                    hasher: blake3::Hasher::new(),
                    size,
                    received: 0,
                    hash,
                    mtime_ns,
                    error: Some(format!("Invalid path or path traversal rejected: {dest_rel:?}")),
                    done: false,
                    is_resumable: false,
                };
            }
        };
        let dest = to_fs_path(root, &safe_rel);
        let dir = dest.parent().map(Path::to_path_buf).unwrap_or_else(|| root.to_path_buf());
        let hex_hash = data_encoding::HEXLOWER.encode(&hash);
        let tmp = dir.join(format!(".lansync-part-{hex_hash}.tmp"));

        let mut hasher = blake3::Hasher::new();
        let mut actual_offset = 0;

        if offset > 0 && verify_partial_offset(&tmp, offset, &mut hasher).await {
            actual_offset = offset;
        } else {
            hasher = blake3::Hasher::new();
        }

        let is_resumable = true;

        // Verify sufficient free disk space before receiving
        let remaining_needed = size.saturating_sub(actual_offset);
        if let Ok(available) = crate::fsutil::available_disk_space(root) {
            let min_required = remaining_needed.saturating_add(10 * 1024 * 1024);
            if available < min_required {
                return Self {
                    root: root.to_path_buf(),
                    dest_rel: safe_rel,
                    tmp,
                    file: None,
                    hasher,
                    size,
                    received: actual_offset,
                    hash,
                    mtime_ns,
                    error: Some(format!(
                        "Insufficient disk space: required {} bytes, available {} bytes",
                        min_required, available
                    )),
                    done: false,
                    is_resumable,
                };
            }
        }

        let mut me = Self {
            root: root.to_path_buf(),
            dest_rel: safe_rel,
            tmp: tmp.clone(),
            file: None,
            hasher,
            size,
            received: actual_offset,
            hash,
            mtime_ns,
            error: None,
            done: false,
            is_resumable,
        };

        let opened = async {
            tokio::fs::create_dir_all(&dir).await?;
            if actual_offset > 0 {
                tokio::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&tmp)
                    .await
            } else {
                tokio::fs::File::create(&tmp).await
            }
        }
        .await;

        match opened {
            Ok(f) => me.file = Some(f),
            Err(e) => me.error = Some(e.to_string()),
        }
        me
    }

    pub async fn begin(root: &Path, dest_rel: &str, size: u64, hash: Hash, mtime_ns: i64) -> Self {
        Self::begin_resumable(root, dest_rel, size, hash, mtime_ns, 0).await
    }

    /// Writes a chunk. Errors (e.g. disk full) are recorded and reported on finish.
    pub async fn chunk(&mut self, data: &[u8]) {
        if self.error.is_some() {
            return;
        }
        self.received += data.len() as u64;
        self.hasher.update(data);
        if let Some(f) = self.file.as_mut() {
            let res = f.write_all(data).await;
            if let Err(e) = res {
                self.error = Some(e.to_string());
            } else {
                let _ = f.flush().await;
            }
        }
    }

    pub async fn finish(mut self, expected_local: Option<Hash>) -> Result<Entry, FinishError> {
        if let Some(e) = self.error.take() {
            return Err(FinishError::Io(e));
        }
        let mut f = self.file.take().ok_or_else(|| io_err("temporary file not open"))?;
        f.flush().await.map_err(io_err)?;
        f.sync_all().await.map_err(io_err)?;
        drop(f);
        if self.received != self.size || *self.hasher.finalize().as_bytes() != self.hash {
            let _ = std::fs::remove_file(&self.tmp);
            return Err(FinishError::HashMismatch);
        }

        // Do not overwrite if user modified the target file after plan generation
        let dest = to_fs_path(&self.root, &self.dest_rel);
        let d = dest.clone();
        let current = tokio::task::spawn_blocking(move || if d.is_file() { hash_file(&d).map(Some) } else { Ok(None) })
            .await
            .map_err(io_err)?
            .map_err(io_err)?;
        if current != expected_local {
            return Err(FinishError::LocalChanged);
        }

        filetime::set_file_mtime(&self.tmp, filetime_from_ns(self.mtime_ns)).map_err(io_err)?;

        // 3.1 File Versioning: Archive existing file to .lansync-versions/ before overwriting
        let _ = crate::versioning::archive_version(&self.root, &self.dest_rel, 5).await;

        // Windows: file currently open by another app (e.g. Office) -> retry every 10 seconds, up to 30 times
        let mut attempts = 0;
        loop {
            match tokio::fs::rename(&self.tmp, &dest).await {
                Ok(()) => break,
                Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied && attempts < 30 => {
                    attempts += 1;
                    tokio::time::sleep(Duration::from_secs(10)).await;
                }
                Err(e) => return Err(io_err(e)),
            }
        }
        self.done = true;
        let meta = std::fs::metadata(&dest).map_err(io_err)?;
        Ok(Entry::simple(self.dest_rel.clone(), meta.len(), mtime_ns(&meta), Some(self.hash)))
    }
}

impl Drop for Incoming {
    fn drop(&mut self) {
        if !self.done {
            drop(self.file.take());
            if !self.is_resumable || self.received == 0 || self.error.is_some() {
                let _ = std::fs::remove_file(&self.tmp);
            }
        }
    }
}

