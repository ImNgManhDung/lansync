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

    pub fn forget_missing_file(&mut self, path: &str) -> Result<()> {
        self.conn.execute("DELETE FROM files WHERE path = ?1", [path])?;
        self.conn.execute("DELETE FROM peer_base WHERE path = ?1", [path])?;
        self.conn.execute("DELETE FROM tombstones WHERE path = ?1", [path])?;
        Ok(())
    }

    /// Ensures `.lansync-root` exists if the root directory is present, and removes stale DB records
    /// (`files`, `peer_base`, `tombstones`) for any file that does not currently exist on disk.
    pub fn clear_missing_files_for_resync(&mut self) -> Result<usize> {
        if self.root.exists() && self.root.is_dir() {
            let marker = self.root.join(".lansync-root");
            if !marker.exists() {
                let _ = std::fs::write(&marker, b"lansync-root-marker\n");
            }
        }
        let all_entries = self.entries()?;
        let mut cleared = 0;
        for entry in all_entries {
            let fs_path = crate::paths::to_fs_path(&self.root, &entry.path);
            if !fs_path.exists() {
                self.forget_missing_file(&entry.path)?;
                cleared += 1;
            }
        }
        let tombs = self.tombstones()?;
        for path in tombs.keys() {
            let fs_path = crate::paths::to_fs_path(&self.root, path);
            if !fs_path.exists() {
                self.forget_missing_file(path)?;
                cleared += 1;
            }
        }
        Ok(cleared)
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

