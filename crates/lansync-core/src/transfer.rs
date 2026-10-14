//! File transfer over temporary files with hash validation and atomic rename.
use crate::fsutil::{filetime_from_ns, hash_file, mtime_ns};
use crate::paths::to_fs_path;
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
    let full = to_fs_path(root, path);
    let Ok(mut f) = tokio::fs::File::open(&full).await else { return Ok(false) };
    let meta = f.metadata().await?;
    write_msg(w, &Message::FileHeader { path: path.to_string(), size: meta.len(), hash: *hash, mtime_ns: mtime_ns(&meta) }).await?;
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

/// Incoming file transfer. Writes to `.lansync-tmp-*` in destination directory.
/// If dropped before `finish` succeeds, the temporary file is deleted.
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
}

impl Incoming {
    pub async fn begin(root: &Path, dest_rel: &str, size: u64, hash: Hash, mtime_ns: i64) -> Self {
        let dest = to_fs_path(root, dest_rel);
        let dir = dest.parent().map(Path::to_path_buf).unwrap_or_else(|| root.to_path_buf());
        let tmp = dir.join(format!(".lansync-tmp-{:016x}", rand::random::<u64>()));
        let mut me = Self {
            root: root.to_path_buf(), dest_rel: dest_rel.to_string(), tmp: tmp.clone(), file: None,
            hasher: blake3::Hasher::new(), size, received: 0, hash, mtime_ns, error: None, done: false,
        };
        let opened = async {
            tokio::fs::create_dir_all(&dir).await?;
            tokio::fs::File::create(&tmp).await
        }
        .await;
        match opened {
            Ok(f) => me.file = Some(f),
            Err(e) => me.error = Some(e.to_string()),
        }
        me
    }

    /// Writes a chunk. Errors (e.g. disk full) are recorded and reported on finish.
    pub async fn chunk(&mut self, data: &[u8]) {
        if self.error.is_some() {
            return;
        }
        self.received += data.len() as u64;
        self.hasher.update(data);
        if let Some(f) = self.file.as_mut() {
            if let Err(e) = f.write_all(data).await {
                self.error = Some(e.to_string());
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
        Ok(Entry { path: self.dest_rel.clone(), size: meta.len(), mtime_ns: mtime_ns(&meta), hash: Some(self.hash) })
    }
}

impl Drop for Incoming {
    fn drop(&mut self) {
        if !self.done {
            drop(self.file.take());
            let _ = std::fs::remove_file(&self.tmp);
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
            .filter(|n| n.starts_with(".lansync-tmp-")).collect()
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
}
