//! File versioning before overwriting, keeping previous snapshots in `.lansync-versions/`.
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct VersionItem {
    pub timestamp_micros: u64,
    pub path: PathBuf,
    pub size: u64,
}

/// Backs up the current file to `.lansync-versions/` before it gets overwritten by remote sync.
/// Keeps at most `max_versions` snapshots (oldest versions are automatically pruned).
pub async fn archive_version(root: &Path, rel_path: &str, max_versions: usize) -> Result<Option<PathBuf>> {
    let current_file = crate::paths::to_fs_path(root, rel_path);
    if !current_file.is_file() {
        return Ok(None);
    }

    let version_dir = root.join(".lansync-versions").join(rel_path);
    tokio::fs::create_dir_all(&version_dir).await?;

    let now_micros = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0);

    let backup_path = version_dir.join(format!("{now_micros}.bak"));
    tokio::fs::copy(&current_file, &backup_path).await?;

    // Prune older versions exceeding max_versions limit
    prune_old_versions(&version_dir, max_versions).await?;

    Ok(Some(backup_path))
}

/// Lists all unique relative file paths that have at least one archived version snapshot.
pub fn list_all_versioned_files(root: &Path) -> Vec<String> {
    let base_versions_dir = root.join(".lansync-versions");
    if !base_versions_dir.is_dir() {
        return Vec::new();
    }

    let mut set = std::collections::BTreeSet::new();
    for entry in walkdir::WalkDir::new(&base_versions_dir).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let p = entry.path();
        if p.extension().and_then(|ext| ext.to_str()) != Some("bak") {
            continue;
        }
        let Some(parent) = p.parent() else {
            continue;
        };
        let Ok(rel) = parent.strip_prefix(&base_versions_dir) else {
            continue;
        };
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if !rel_str.is_empty() {
            set.insert(rel_str);
        }
    }

    set.into_iter().collect()
}

/// Lists all archived versions for a given relative path, sorted from newest to oldest.
pub async fn list_versions(root: &Path, rel_path: &str) -> Result<Vec<VersionItem>> {
    let version_dir = root.join(".lansync-versions").join(rel_path);
    if !version_dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut items = Vec::new();
    let mut reader = tokio::fs::read_dir(&version_dir).await?;
    while let Some(entry) = reader.next_entry().await? {
        let path = entry.path();
        let stem_ts = path
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| s.parse::<u64>().ok());

        if let Some(ts) = stem_ts.filter(|_| path.is_file()) {
            let meta = entry.metadata().await?;
            items.push(VersionItem {
                timestamp_micros: ts,
                path,
                size: meta.len(),
            });
        }
    }

    items.sort_by_key(|a| std::cmp::Reverse(a.timestamp_micros));
    Ok(items)
}

/// Restores a previously archived snapshot back to its active file location.
pub async fn restore_version(root: &Path, rel_path: &str, snapshot_path: &Path) -> Result<()> {
    let target_file = crate::paths::to_fs_path(root, rel_path);
    if let Some(parent) = target_file.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::copy(snapshot_path, &target_file).await?;
    Ok(())
}

/// Restores a snapshot matching the specified timestamp back to the active file.
pub async fn restore_version_by_timestamp(root: &Path, rel_path: &str, timestamp_micros: u64) -> Result<()> {
    let version_dir = root.join(".lansync-versions").join(rel_path);
    let snapshot_path = version_dir.join(format!("{timestamp_micros}.bak"));
    restore_version(root, rel_path, &snapshot_path).await
}

async fn prune_old_versions(dir: &Path, max_versions: usize) -> Result<()> {
    if max_versions == 0 {
        return Ok(());
    }
    let mut entries = Vec::new();
    let mut reader = tokio::fs::read_dir(dir).await?;
    while let Some(entry) = reader.next_entry().await? {
        let path = entry.path();
        let stem_ts = path
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| s.parse::<u64>().ok());

        if let Some(ts) = stem_ts.filter(|_| path.is_file()) {
            entries.push((ts, path));
        }
    }

    // Sort newest to oldest
    entries.sort_by_key(|a| std::cmp::Reverse(a.0));

    if entries.len() > max_versions {
        for (_, old_path) in &entries[max_versions..] {
            let _ = tokio::fs::remove_file(old_path).await;
        }
    }

    Ok(())
}

