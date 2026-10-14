//! Local file state and peer base-hashes backed by SQLite.
use crate::fsutil::mtime_ns;
use crate::paths::{normalize_rel, windows_invalid_reason};
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

use crate::version_vector::VersionVector;

pub struct Index {
    conn: Connection,
    root: PathBuf,
    device_id: String,
}

const SCHEMA: &str = "
PRAGMA journal_mode = WAL;
CREATE TABLE IF NOT EXISTS files (
  path     TEXT PRIMARY KEY,
  size     INTEGER NOT NULL,
  mtime_ns INTEGER NOT NULL,
  hash     BLOB,
  deleted  INTEGER NOT NULL DEFAULT 0,
  vv       TEXT NOT NULL DEFAULT '{}'
);
CREATE TABLE IF NOT EXISTS peer_base (
  peer_id   TEXT NOT NULL,
  path      TEXT NOT NULL,
  base_hash BLOB NOT NULL,
  PRIMARY KEY (peer_id, path)
);
CREATE TABLE IF NOT EXISTS tombstones (
  path       TEXT PRIMARY KEY,
  hash       BLOB NOT NULL,
  deleted_at INTEGER NOT NULL
);";

fn row_to_entry(row: &Row) -> rusqlite::Result<Entry> {
    let hash: Option<Vec<u8>> = row.get(3)?;
    let vv_str: String = row.get(4).unwrap_or_else(|_| "{}".to_string());
    let vv = serde_json::from_str(&vv_str).unwrap_or_default();
    Ok(Entry {
        path: row.get(0)?,
        size: row.get::<_, i64>(1)? as u64,
        mtime_ns: row.get(2)?,
        hash: hash.and_then(|v| v.try_into().ok()),
        vv,
    })
}

impl Index {
    pub fn open(db_path: &Path, root: &Path, device_id: &str) -> Result<Self> {
        let conn = Connection::open(db_path)?;
        conn.execute_batch(SCHEMA)?;
        // Ensure vv column exists on existing databases
        let _ = conn.execute("ALTER TABLE files ADD COLUMN vv TEXT NOT NULL DEFAULT '{}'", []);

        // Initialize empty vv on active files for device_id
        let mut init_vv = VersionVector::new();
        init_vv.set(device_id, 1);
        let init_vv_str = serde_json::to_string(&init_vv)?;
        let _ = conn.execute(
            "UPDATE files SET vv = ?1 WHERE (vv IS NULL OR vv = '' OR vv = '{}') AND deleted = 0",
            params![init_vv_str],
        );

        // Create root directory and marker if available
        if root.exists() && root.is_dir() {
            let marker = root.join(".lansync-root");
            if !marker.exists() {
                let _ = std::fs::write(&marker, b"lansync-root-marker\n");
            }
        }
        Ok(Self {
            conn,
            root: root.to_path_buf(),
            device_id: device_id.to_string(),
        })
    }

    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    pub fn entries(&self) -> Result<Vec<Entry>> {
        let mut st = self.conn.prepare("SELECT path, size, mtime_ns, hash, vv FROM files ORDER BY path")?;
        let rows = st.query_map([], row_to_entry)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn get(&self, path: &str) -> Result<Option<Entry>> {
        Ok(self
            .conn
            .query_row("SELECT path, size, mtime_ns, hash, vv FROM files WHERE path = ?1", [path], row_to_entry)
            .optional()?)
    }

    pub fn record_file(&mut self, e: &Entry) -> Result<()> {
        let vv_str = serde_json::to_string(&e.vv)?;
        self.conn.execute(
            "INSERT INTO files (path, size, mtime_ns, hash, deleted, vv) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(path) DO UPDATE SET size = excluded.size, mtime_ns = excluded.mtime_ns,
               hash = excluded.hash, deleted = excluded.deleted, vv = excluded.vv",
            params![e.path, e.size as i64, e.mtime_ns, e.hash.map(|h| h.to_vec()), e.hash.is_none() as i64, vv_str],
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

    pub fn tombstones(&self) -> Result<HashMap<String, Hash>> {
        let mut st = self.conn.prepare("SELECT path, hash FROM tombstones")?;
        let rows = st
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows.into_iter().filter_map(|(p, h)| h.try_into().ok().map(|h| (p, h))).collect())
    }

    pub fn record_tombstone(&mut self, path: &str, hash: &Hash) -> Result<()> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        self.conn.execute(
            "INSERT INTO tombstones (path, hash, deleted_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(path) DO UPDATE SET hash = excluded.hash, deleted_at = excluded.deleted_at",
            params![path, hash.to_vec(), now],
        )?;
        Ok(())
    }

    pub fn remove_tombstone(&mut self, path: &str) -> Result<bool> {
        let count = self.conn.execute("DELETE FROM tombstones WHERE path = ?1", [path])?;
        Ok(count > 0)
    }

    /// Scans directory. Only re-hashes files with changed size or mtime.
    /// Protects against external drive unmount by checking root and marker.
    pub fn scan(&mut self) -> Result<ScanReport> {
        let mut report = ScanReport::default();

        // Safe root check: if drive disconnected, abort without marking deleted
        if !self.root.exists() || !self.root.is_dir() {
            tracing::warn!("Sync root {:?} does not exist. Aborting scan to prevent mass deletion.", self.root);
            return Ok(report);
        }

        let known: HashMap<String, Entry> = self.entries()?.into_iter().map(|e| (e.path.clone(), e)).collect();
        let mut seen = HashSet::new();

        // 3.2 Load custom user ignore patterns from .lansyncignore
        let custom_ignores: Vec<String> = {
            let ignore_file = self.root.join(".lansyncignore");
            if ignore_file.is_file() {
                std::fs::read_to_string(&ignore_file)
                    .unwrap_or_default()
                    .lines()
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty() && !l.starts_with('#'))
                    .collect()
            } else {
                Vec::new()
            }
        };

        for item in WalkDir::new(&self.root).follow_links(false).into_iter().filter_map(|r| r.ok()) {
            // skip directories and symlinks
            if !item.file_type().is_file() {
                continue;
            }
            let Some(rel) = normalize_rel(&self.root, item.path()) else { continue };
            if crate::paths::is_ignored_custom(&rel, &custom_ignores) {
                continue;
            }
            if let Some(reason) = windows_invalid_reason(&rel) {
                report.skipped.push((rel, reason));
                continue;
            }
            let Ok(meta) = item.metadata() else { continue };
            let (size, mtime) = (meta.len(), mtime_ns(&meta));
            seen.insert(rel.clone());
            // If file was previously deleted and restored locally, clear tombstone
            let _ = self.remove_tombstone(&rel);

            if known.get(&rel).is_some_and(|old| old.hash.is_some() && old.size == size && old.mtime_ns == mtime) {
                continue;
            }
            let hash = match crate::fsutil::hash_file_stable(item.path()) {
                Ok(Some(h)) => h,
                Ok(None) => {
                    tracing::debug!("File {rel} modified during read, deferring to next scan");
                    continue;
                }
                Err(err) => {
                    tracing::warn!("failed to read {rel}: {err}");
                    continue;
                }
            };
            let mut vv = match known.get(&rel) {
                Some(old) => old.vv.clone(),
                None => VersionVector::new(),
            };
            vv.increment(&self.device_id);
            let e = Entry { path: rel, size, mtime_ns: mtime, hash: Some(hash), vv };
            self.record_file(&e)?;
            report.changed.push(e);
        }

        // Safety check: if known had active files, but zero files seen on disk and root marker missing,
        // it may be an unmounted mountpoint! Do not delete files.
        let had_active_files = known.values().any(|e| e.hash.is_some());
        if had_active_files && seen.is_empty() && !self.root.join(".lansync-root").exists() {
            tracing::warn!("Sync root marker is missing and 0 files found. Suspected unmounted drive, skipping deletions.");
            return Ok(report);
        }

        for (path, old) in known {
            if old.hash.is_some() && !seen.contains(&path) {
                // If an active partial download (.lansync-part-*.tmp) is present, do not record a tombstone
                if let Some(h) = old.hash {
                    if crate::transfer::partial_download_path(&self.root, &path, &h).is_some_and(|part| part.exists()) {
                        continue;
                    }
                    let _ = self.record_tombstone(&path, &h);
                }
                let mut vv = old.vv.clone();
                vv.increment(&self.device_id);
                let e = Entry { path, size: 0, mtime_ns: 0, hash: None, vv };
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
        let idx = Index::open(&t.path().join("index.db"), &root, "dev_test").unwrap();
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
        let e = Entry::simple("x.txt", 5, 9, Some([3; 32]));
        idx.record_file(&e).unwrap();
        assert_eq!(idx.get("x.txt").unwrap(), Some(e));
        idx.set_base("PEER", "x.txt", &[1; 32]).unwrap();
        idx.set_base("PEER", "x.txt", &[2; 32]).unwrap();
        assert_eq!(idx.bases("PEER").unwrap().get("x.txt"), Some(&[2; 32]));
        assert!(idx.bases("OTHER").unwrap().is_empty());
    }

    #[test]
    fn version_vector_increments_and_persists() {
        let (t, root, mut idx) = setup();
        fs::write(root.join("vv_test.txt"), b"v1").unwrap();
        idx.scan().unwrap();
        let e1 = idx.get("vv_test.txt").unwrap().unwrap();
        assert_eq!(e1.vv.get("dev_test"), 1);

        // Edit file
        fs::write(root.join("vv_test.txt"), b"v2_content").unwrap();
        idx.scan().unwrap();
        let e2 = idx.get("vv_test.txt").unwrap().unwrap();
        assert_eq!(e2.vv.get("dev_test"), 2);

        // Delete file
        fs::remove_file(root.join("vv_test.txt")).unwrap();
        idx.scan().unwrap();
        let e3 = idx.get("vv_test.txt").unwrap().unwrap();
        assert_eq!(e3.vv.get("dev_test"), 3);
        assert_eq!(e3.hash, None);

        // Reopen DB and verify persistence
        let idx_reopened = Index::open(&t.path().join("index.db"), &root, "dev_test").unwrap();
        let e_persisted = idx_reopened.get("vv_test.txt").unwrap().unwrap();
        assert_eq!(e_persisted.vv.get("dev_test"), 3);
    }

    #[test]
    fn tombstone_recorded_on_delete_and_cleared_on_recreate() {
        let (_t, root, mut idx) = setup();
        fs::write(root.join("test.pdf"), b"pdf-content").unwrap();
        idx.scan().unwrap();
        let expected_hash = *blake3::hash(b"pdf-content").as_bytes();
        assert!(idx.tombstones().unwrap().is_empty());

        // Delete file
        fs::remove_file(root.join("test.pdf")).unwrap();
        idx.scan().unwrap();

        // Tombstone must be recorded with original file's hash!
        let tombstones = idx.tombstones().unwrap();
        assert_eq!(tombstones.get("test.pdf"), Some(&expected_hash));

        // Re-creating file clears the tombstone
        fs::write(root.join("test.pdf"), b"new-pdf-content").unwrap();
        idx.scan().unwrap();
        assert!(idx.tombstones().unwrap().is_empty());
    }

    #[test]
    fn unmounted_root_prevents_mass_deletion() {
        let (_t, root, mut idx) = setup();
        fs::write(root.join("important1.doc"), b"data1").unwrap();
        fs::write(root.join("important2.doc"), b"data2").unwrap();
        idx.scan().unwrap();
        assert_eq!(idx.entries().unwrap().iter().filter(|e| e.hash.is_some()).count(), 2);

        // Simulate external drive unmount by removing root directory or emptying it without marker
        fs::remove_file(root.join(".lansync-root")).unwrap();
        fs::remove_file(root.join("important1.doc")).unwrap();
        fs::remove_file(root.join("important2.doc")).unwrap();

        let report = idx.scan().unwrap();
        // Mass deletion must be halted!
        assert!(report.changed.is_empty(), "Unmounted root must not report deletions!");
        assert_eq!(idx.entries().unwrap().iter().filter(|e| e.hash.is_some()).count(), 2);
    }

    #[test]
    fn test_custom_lansyncignore_is_respected() {
        let (_t, root, mut idx) = setup();
        fs::write(root.join(".lansyncignore"), b"*.log\nbuild/\nsecret_key.pem\n").unwrap();
        fs::write(root.join("app.log"), b"log").unwrap();
        fs::create_dir_all(root.join("build")).unwrap();
        fs::write(root.join("build/output.bin"), b"binary").unwrap();
        fs::write(root.join("secret_key.pem"), b"secret").unwrap();
        fs::write(root.join("allowed.txt"), b"allowed").unwrap();

        let report = idx.scan().unwrap();
        let changed_paths = paths(&report);

        assert_eq!(changed_paths, vec!["allowed.txt"]);
    }
}
