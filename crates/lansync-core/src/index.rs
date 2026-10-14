//! Local file state and peer base-hashes backed by SQLite.
use crate::fsutil::{hash_file, mtime_ns};
use crate::paths::{is_ignored, normalize_rel, windows_invalid_reason};
use crate::protocol::{Entry, Hash};
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension, Row};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Default, Debug)]
pub struct ScanReport {
    /// New, modified, or deleted files (hash = None)
    pub changed: Vec<Entry>,
    /// (path, reason) for files that cannot be synchronized
    pub skipped: Vec<(String, String)>,
}

pub struct Index {
    conn: Connection,
    root: PathBuf,
}

const SCHEMA: &str = "
PRAGMA journal_mode = WAL;
CREATE TABLE IF NOT EXISTS files (
  path     TEXT PRIMARY KEY,
  size     INTEGER NOT NULL,
  mtime_ns INTEGER NOT NULL,
  hash     BLOB,
  deleted  INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS peer_base (
  peer_id   TEXT NOT NULL,
  path      TEXT NOT NULL,
  base_hash BLOB NOT NULL,
  PRIMARY KEY (peer_id, path)
);";

fn row_to_entry(row: &Row) -> rusqlite::Result<Entry> {
    let hash: Option<Vec<u8>> = row.get(3)?;
    Ok(Entry {
        path: row.get(0)?,
        size: row.get::<_, i64>(1)? as u64,
        mtime_ns: row.get(2)?,
        hash: hash.and_then(|v| v.try_into().ok()),
    })
}

impl Index {
    pub fn open(db_path: &Path, root: &Path) -> Result<Self> {
        let conn = Connection::open(db_path)?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn, root: root.to_path_buf() })
    }

    pub fn entries(&self) -> Result<Vec<Entry>> {
        let mut st = self.conn.prepare("SELECT path, size, mtime_ns, hash FROM files ORDER BY path")?;
        let rows = st.query_map([], row_to_entry)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn get(&self, path: &str) -> Result<Option<Entry>> {
        Ok(self
            .conn
            .query_row("SELECT path, size, mtime_ns, hash FROM files WHERE path = ?1", [path], row_to_entry)
            .optional()?)
    }

    pub fn record_file(&mut self, e: &Entry) -> Result<()> {
        self.conn.execute(
            "INSERT INTO files (path, size, mtime_ns, hash, deleted) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(path) DO UPDATE SET size = excluded.size, mtime_ns = excluded.mtime_ns,
               hash = excluded.hash, deleted = excluded.deleted",
            params![e.path, e.size as i64, e.mtime_ns, e.hash.map(|h| h.to_vec()), e.hash.is_none() as i64],
        )?;
        Ok(())
    }

    pub fn bases(&self, peer_id: &str) -> Result<HashMap<String, Hash>> {
        let mut st = self.conn.prepare("SELECT path, base_hash FROM peer_base WHERE peer_id = ?1")?;
        let rows = st
            .query_map([peer_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows.into_iter().filter_map(|(p, h)| h.try_into().ok().map(|h| (p, h))).collect())
    }

    pub fn set_base(&mut self, peer_id: &str, path: &str, hash: &Hash) -> Result<()> {
        self.conn.execute(
            "INSERT INTO peer_base (peer_id, path, base_hash) VALUES (?1, ?2, ?3)
             ON CONFLICT(peer_id, path) DO UPDATE SET base_hash = excluded.base_hash",
            params![peer_id, path, hash.to_vec()],
        )?;
        Ok(())
    }

    /// Scans directory. Only re-hashes files with changed size or mtime.
    pub fn scan(&mut self) -> Result<ScanReport> {
        let mut report = ScanReport::default();
        let known: HashMap<String, Entry> = self.entries()?.into_iter().map(|e| (e.path.clone(), e)).collect();
        let mut seen = HashSet::new();

        for item in WalkDir::new(&self.root).follow_links(false).into_iter().filter_map(|r| r.ok()) {
            // skip directories and symlinks
            if !item.file_type().is_file() {
                continue;
            }
            let Some(rel) = normalize_rel(&self.root, item.path()) else { continue };
            if is_ignored(&rel) {
                continue;
            }
            if let Some(reason) = windows_invalid_reason(&rel) {
                report.skipped.push((rel, reason));
                continue;
            }
            let Ok(meta) = item.metadata() else { continue };
            let (size, mtime) = (meta.len(), mtime_ns(&meta));
            seen.insert(rel.clone());
            if let Some(old) = known.get(&rel) {
                if old.hash.is_some() && old.size == size && old.mtime_ns == mtime {
                    continue;
                }
            }
            let hash = match hash_file(item.path()) {
                Ok(h) => h,
                Err(err) => {
                    tracing::warn!("failed to read {rel}: {err}");
                    continue;
                }
            };
            // file is being written to (size/mtime changed during hash) -> defer to next scan
            match std::fs::metadata(item.path()) {
                Ok(after) if after.len() == size && mtime_ns(&after) == mtime => {}
                _ => continue,
            }
            let e = Entry { path: rel, size, mtime_ns: mtime, hash: Some(hash) };
            self.record_file(&e)?;
            report.changed.push(e);
        }

        for (path, old) in known {
            if old.hash.is_some() && !seen.contains(&path) {
                let e = Entry { path, size: 0, mtime_ns: 0, hash: None };
                self.record_file(&e)?;
                report.changed.push(e);
            }
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup() -> (TempDir, PathBuf, Index) {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("sync");
        fs::create_dir_all(&root).unwrap();
        let idx = Index::open(&t.path().join("index.db"), &root).unwrap();
        (t, root, idx)
    }

    fn paths(r: &ScanReport) -> Vec<String> {
        let mut v: Vec<String> = r.changed.iter().map(|e| e.path.clone()).collect();
        v.sort();
        v
    }

    #[test]
    fn scan_finds_new_files_and_skips_ignored() {
        let (_t, root, mut idx) = setup();
        fs::write(root.join("a.txt"), b"aaa").unwrap();
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("sub/b.txt"), b"bb").unwrap();
        fs::write(root.join(".DS_Store"), b"x").unwrap();
        let r = idx.scan().unwrap();
        assert_eq!(paths(&r), vec!["a.txt", "sub/b.txt"]);
        let a = idx.get("a.txt").unwrap().unwrap();
        assert_eq!(a.hash, Some(*blake3::hash(b"aaa").as_bytes()));
        assert_eq!(a.size, 3);
    }

    #[test]
    fn second_scan_without_changes_reports_nothing() {
        let (_t, root, mut idx) = setup();
        fs::write(root.join("a.txt"), b"aaa").unwrap();
        idx.scan().unwrap();
        assert!(idx.scan().unwrap().changed.is_empty());
    }

    #[test]
    fn modified_file_gets_new_hash() {
        let (_t, root, mut idx) = setup();
        fs::write(root.join("a.txt"), b"1").unwrap();
        idx.scan().unwrap();
        fs::write(root.join("a.txt"), b"22").unwrap();
        let r = idx.scan().unwrap();
        assert_eq!(r.changed.len(), 1);
        assert_eq!(r.changed[0].hash, Some(*blake3::hash(b"22").as_bytes()));
    }

    #[test]
    fn deleted_file_is_kept_with_none_hash() {
        let (_t, root, mut idx) = setup();
        fs::write(root.join("a.txt"), b"1").unwrap();
        idx.scan().unwrap();
        fs::remove_file(root.join("a.txt")).unwrap();
        let r = idx.scan().unwrap();
        assert_eq!(r.changed.len(), 1);
        assert_eq!(r.changed[0].hash, None);
        assert_eq!(idx.entries().unwrap()[0].hash, None);
        // second scan does not re-report
        assert!(idx.scan().unwrap().changed.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn windows_invalid_names_are_skipped() {
        let (_t, root, mut idx) = setup();
        fs::write(root.join("a:b.txt"), b"1").unwrap();
        let r = idx.scan().unwrap();
        assert!(r.changed.is_empty());
        assert_eq!(r.skipped.len(), 1);
        assert_eq!(r.skipped[0].0, "a:b.txt");
    }

    #[test]
    fn record_file_and_bases() {
        let (_t, _root, mut idx) = setup();
        let e = Entry { path: "x.txt".into(), size: 5, mtime_ns: 9, hash: Some([3; 32]) };
        idx.record_file(&e).unwrap();
        assert_eq!(idx.get("x.txt").unwrap(), Some(e));
        idx.set_base("PEER", "x.txt", &[1; 32]).unwrap();
        idx.set_base("PEER", "x.txt", &[2; 32]).unwrap();
        assert_eq!(idx.bases("PEER").unwrap().get("x.txt"), Some(&[2; 32]));
        assert!(idx.bases("OTHER").unwrap().is_empty());
    }
}
