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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fsutil::{hash_file, mtime_ns};
    use crate::protocol::{read_msg, Message};
    use std::path::Path;

    fn tmp_files(dir: &Path) -> Vec<String> {
        walkdir::WalkDir::new(dir).into_iter().filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.starts_with(".lansync-tmp-") || n.starts_with(".lansync-part-")).collect()
    }

    #[tokio::test]
    async fn sends_and_receives_file() {
        let (src, dst) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        std::fs::create_dir_all(src.path().join("sub")).unwrap();
        let data: Vec<u8> = (0..1_000_000u32).map(|i| (i % 251) as u8).collect();
        let sp = src.path().join("sub/x.bin");
        std::fs::write(&sp, &data).unwrap();
        let hash = hash_file(&sp).unwrap();

        let (mut a, mut b) = tokio::io::duplex(1 << 20);
        let root = src.path().to_path_buf();
        let sender = tokio::spawn(async move { send_file(&mut a, &root, "sub/x.bin", &hash).await.unwrap() });

        let Message::FileHeader { size, hash: h, mtime_ns: m, .. } = read_msg(&mut b).await.unwrap() else { panic!("expected FileHeader") };
        let mut inc = Incoming::begin(dst.path(), "sub/x.bin", size, h, m).await;
        loop {
            match read_msg(&mut b).await.unwrap() {
                Message::FileChunk { data } => inc.chunk(&data).await,
                Message::FileEnd => break,
                other => panic!("unexpected message {other:?}"),
            }
        }
        assert!(sender.await.unwrap());
        let entry = inc.finish(None).await.unwrap();

        assert_eq!(std::fs::read(dst.path().join("sub/x.bin")).unwrap(), data);
        assert_eq!(entry.hash, Some(hash));
        let src_m = mtime_ns(&std::fs::metadata(&sp).unwrap());
        assert!((entry.mtime_ns - src_m).abs() < 1_000, "mtime must match sender");
        assert!(tmp_files(dst.path()).is_empty());
    }

    #[tokio::test]
    async fn hash_mismatch_is_rejected_and_tmp_removed() {
        let dst = tempfile::tempdir().unwrap();
        let mut inc = Incoming::begin(dst.path(), "a.txt", 3, [0; 32], 0).await;
        inc.chunk(b"abc").await;
        assert!(matches!(inc.finish(None).await, Err(FinishError::HashMismatch)));
        assert!(!dst.path().join("a.txt").exists());
        assert!(tmp_files(dst.path()).is_empty());
    }

    #[tokio::test]
    async fn refuses_to_overwrite_locally_changed_file() {
        let dst = tempfile::tempdir().unwrap();
        std::fs::write(dst.path().join("a.txt"), b"new local version").unwrap();
        let new = *blake3::hash(b"from remote peer").as_bytes();
        let mut inc = Incoming::begin(dst.path(), "a.txt", 16, new, 0).await;
        inc.chunk(b"from remote peer").await;
        let old = *blake3::hash(b"old version").as_bytes();
        assert!(matches!(inc.finish(Some(old)).await, Err(FinishError::LocalChanged)));
        assert_eq!(std::fs::read(dst.path().join("a.txt")).unwrap(), b"new local version");
    }

    #[tokio::test]
    async fn send_file_returns_false_when_missing() {
        let src = tempfile::tempdir().unwrap();
        let (mut a, _b) = tokio::io::duplex(1024);
        assert!(!send_file(&mut a, src.path(), "missing.txt", &[0; 32]).await.unwrap());
    }

    #[tokio::test]
    async fn incoming_rejects_path_traversal() {
        let dst = tempfile::tempdir().unwrap();
        let mut inc = Incoming::begin(dst.path(), "../../etc/passwd", 5, [0; 32], 0).await;
        inc.chunk(b"hello").await;
        let res = inc.finish(None).await;
        assert!(res.is_err());
        assert!(!dst.path().join("etc/passwd").exists());
        assert!(!dst.path().join("../../etc/passwd").exists());
    }

    #[tokio::test]
    async fn incoming_archives_previous_version_before_overwrite() {
        let dst = tempfile::tempdir().unwrap();
        // Existing local file
        let initial_data = b"original content to be preserved";
        std::fs::write(dst.path().join("report.txt"), initial_data).unwrap();
        let expected_hash = *blake3::hash(initial_data).as_bytes();

        let new_data = b"incoming overwritten content";
        let new_hash = *blake3::hash(new_data).as_bytes();

        let mut inc = Incoming::begin(dst.path(), "report.txt", new_data.len() as u64, new_hash, 1000).await;
        inc.chunk(new_data).await;
        inc.finish(Some(expected_hash)).await.unwrap();

        // Target file has new content
        assert_eq!(std::fs::read(dst.path().join("report.txt")).unwrap(), new_data);

        // Previous version was archived in .lansync-versions/
        let versions = crate::versioning::list_versions(dst.path(), "report.txt").await.unwrap();
        assert_eq!(versions.len(), 1, "Must have exactly 1 archived backup");
        assert_eq!(std::fs::read(&versions[0].path).unwrap(), initial_data);
    }

    #[tokio::test]
    async fn test_resumable_file_transfer() {
        let (src, dst) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let data: Vec<u8> = (0..500_000u32).map(|i| (i % 251) as u8).collect();
        let hash = *blake3::hash(&data).as_bytes();
        std::fs::write(src.path().join("bigfile.bin"), &data).unwrap();

        // 1. Partial transfer: only receive first 200,000 bytes, then disconnect/drop
        {
            let mut inc = Incoming::begin_resumable(dst.path(), "bigfile.bin", data.len() as u64, hash, 1000, 0).await;
            inc.chunk(&data[..200_000]).await;
            // Dropping inc simulates connection interruption without calling finish()
            drop(inc);
        }

        // Verify partial download exists and offset matches
        let resume_offset = check_resume_offset(dst.path(), "bigfile.bin", &hash, data.len() as u64).await;
        assert_eq!(resume_offset, 200_000, "Must detect 200,000 bytes available for resume");

        // 2. Resume transfer: send from byte 200,000 to end
        let (mut a, mut b) = tokio::io::duplex(256 * 1024);
        let src_path = src.path().to_path_buf();
        let sender = tokio::spawn(async move {
            send_file_range(&mut a, &src_path, "bigfile.bin", &hash, resume_offset).await.unwrap();
        });

        let mut inc_resumed = Incoming::begin_resumable(dst.path(), "bigfile.bin", data.len() as u64, hash, 1000, resume_offset).await;
        let header = read_msg(&mut b).await.unwrap();
        assert!(matches!(header, Message::FileHeaderRange { offset: 200_000, .. }));

        loop {
            match read_msg(&mut b).await.unwrap() {
                Message::FileChunk { data } => inc_resumed.chunk(&data).await,
                Message::FileEnd => break,
                other => panic!("Unexpected msg: {:?}", other),
            }
        }
        sender.await.unwrap();

        let entry = inc_resumed.finish(None).await.unwrap();
        assert_eq!(entry.size, 500_000);
        assert_eq!(std::fs::read(dst.path().join("bigfile.bin")).unwrap(), data);
        assert!(tmp_files(dst.path()).is_empty(), "All temporary part files must be removed");
    }
}
