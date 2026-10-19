use chrono::Local;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingPairingEntity {
    pub peer_id: String,
    pub device_name: String,
    pub code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityEntity {
    pub id: String,
    pub timestamp: String,
    pub title: String,
    pub details: String,
    pub kind: String,
}

impl ActivityEntity {
    pub fn new(title: impl Into<String>, details: impl Into<String>, kind: impl Into<String>) -> Self {
        Self {
            id: format!("{:x}", rand::random::<u64>()),
            timestamp: Local::now().format("%H:%M:%S").to_string(),
            title: title.into(),
            details: details.into(),
            kind: kind.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerEntity {
    pub device_id: String,
    pub name: String,
    pub is_paired: bool,
    pub is_connected: bool,
    pub status: String,
    pub address: Option<String>,
    pub delete_policy: Option<DeletePolicy>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConflictEntity {
    pub conflict_path: String,
    pub original_path: String,
    pub size: u64,
    pub modified_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingDeletionEntity {
    pub peer_id: String,
    pub path: String,
    pub timestamp: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionSnapshotEntity {
    pub rel_path: String,
    pub timestamp_micros: u64,
    pub formatted_time: String,
    pub size: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeletePolicy {
    Never,
    Ask,
    MirrorTrash,
}

impl DeletePolicy {
    #[cfg(test)]
    pub fn as_code(&self) -> &'static str {
        match self {
            DeletePolicy::Never => "never",
            DeletePolicy::Ask => "ask",
            DeletePolicy::MirrorTrash => "mirror_trash",
        }
    }

    #[cfg(test)]
    pub fn from_code(s: &str) -> Option<Self> {
        match s {
            "never" => Some(DeletePolicy::Never),
            "ask" => Some(DeletePolicy::Ask),
            "mirror_trash" => Some(DeletePolicy::MirrorTrash),
            _ => None,
        }
    }

    pub fn label_vi(&self) -> &'static str {
        match self {
            DeletePolicy::Never => "Không xoá chéo (An toàn nhất)",
            DeletePolicy::Ask => "Hỏi tôi trước khi xoá",
            DeletePolicy::MirrorTrash => "Chuyển vào Thùng rác (Trash)",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConflictResolution {
    KeepMine,
    KeepTheirs,
    KeepBoth,
}

/// Tách đường dẫn file gốc từ tên file xung đột:
/// `report (conflict-MacBook 2026-10-14 0930).pdf` -> `report.pdf`
pub fn derive_original_from_conflict(conflict_rel: &str) -> String {
    if let Some(idx) = conflict_rel.find(" (conflict-") {
        let (prefix, rest) = conflict_rel.split_at(idx);
        if let Some(close_idx) = rest.find(')') {
            let ext = &rest[close_idx + 1..];
            return format!("{}{}", prefix, ext);
        }
    }
    conflict_rel.to_string()
}

/// Định dạng dung lượng byte sang chuỗi dễ đọc (B, KB, MB, GB)
pub fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;

    let b = bytes as f64;
    if b >= GB {
        format!("{:.2} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{} B", bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_original_from_conflict() {
        assert_eq!(
            derive_original_from_conflict("docs/report (conflict-WinPC 2026-10-14 1200).docx"),
            "docs/report.docx"
        );
        assert_eq!(
            derive_original_from_conflict("README (conflict-Mac 1).md"),
            "README.md"
        );
        assert_eq!(derive_original_from_conflict("normal.txt"), "normal.txt");
    }

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(2048), "2.0 KB");
        assert_eq!(format_bytes(5 * 1024 * 1024), "5.0 MB");
    }

    #[test]
    fn test_delete_policy_roundtrip() {
        for policy in [DeletePolicy::Never, DeletePolicy::Ask, DeletePolicy::MirrorTrash] {
            assert_eq!(DeletePolicy::from_code(policy.as_code()), Some(policy));
        }
        assert_eq!(DeletePolicy::from_code("unknown"), None);
    }
}
