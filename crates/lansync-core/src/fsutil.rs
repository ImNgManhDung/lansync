//! File utilities: Blake3 hashing and nanosecond mtime.
use crate::protocol::Hash;
use filetime::FileTime;
use std::fs::{File, Metadata};
use std::path::Path;
use std::time::UNIX_EPOCH;

pub fn hash_file(path: &Path) -> std::io::Result<Hash> {
    let mut f = File::open(path)?;
    let mut h = blake3::Hasher::new();
    std::io::copy(&mut f, &mut h)?;
    Ok(*h.finalize().as_bytes())
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
    fn mtime_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.txt");
        std::fs::write(&p, b"x").unwrap();
        let ns: i64 = 1_700_000_000_123_456_700; // multiple of 100ns to match NTFS resolution
        filetime::set_file_mtime(&p, filetime_from_ns(ns)).unwrap();
        assert_eq!(mtime_ns(&std::fs::metadata(&p).unwrap()), ns);
    }
}
