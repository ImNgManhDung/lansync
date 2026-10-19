use anyhow::Result;
use async_trait::async_trait;
use std::path::PathBuf;
use tokio::sync::broadcast;

use crate::domain::models::{
    ConflictEntity, ConflictResolution, DeletePolicy, PeerEntity, PendingDeletionEntity,
    VersionSnapshotEntity,
};

#[derive(Debug, Clone)]
pub enum BackendEvent {
    PeerDiscovered {
        device_id: String,
        device_name: String,
    },
    PairingRequested {
        peer_id: String,
        device_name: String,
        code: String,
    },
    PairingAccepted {
        peer_id: String,
    },
    PairingRejected,
    PeerConnected {
        device_name: String,
    },
    PeerDisconnected {
        peer_id: String,
    },
    PeerStatusChanged,
    SyncStarted,
    FileDownloading {
        rel_path: String,
        size: u64,
    },
    FileDownloaded {
        rel_path: String,
    },
    FileTrashed {
        rel_path: String,
    },
    PendingDeletion {
        peer_id: String,
        rel_path: String,
    },
    ConflictCreated {
        original: String,
        conflict: String,
    },
    SyncCompleted,
    Error(String),
}

#[async_trait]
pub trait SyncBackendPort: Send + Sync {
    fn local_device_id(&self) -> String;
    fn device_name(&self) -> String;
    fn sync_dir(&self) -> PathBuf;
    fn listen_port(&self) -> u16;
    fn subscribe_events(&self) -> broadcast::Receiver<BackendEvent>;

    fn is_paused(&self) -> bool;
    async fn set_paused(&self, paused: bool);

    fn bandwidth_limit_kbps(&self) -> u32;
    fn set_bandwidth_limit_kbps(&self, kbps: u32);

    async fn get_delete_policy(&self) -> DeletePolicy;
    async fn set_delete_policy(&self, policy: DeletePolicy) -> Result<()>;
    async fn set_peer_delete_policy(&self, peer_id: &str, policy: Option<DeletePolicy>) -> Result<()>;

    async fn trigger_manual_sync(&self);
    async fn rescan_peers(&self);
    async fn pair_with_address(&self, addr: &str) -> Result<()>;
    async fn approve_pairing(&self, peer_id: &str) -> Result<()>;
    async fn reject_pairing(&self, peer_id: &str) -> Result<()>;
    async fn unpair_peer(&self, peer_id: &str) -> Result<()>;

    async fn list_combined_peers(&self) -> Vec<PeerEntity>;
    async fn list_conflicts(&self) -> Vec<ConflictEntity>;
    async fn resolve_conflict(&self, conflict_path: &str, resolution: ConflictResolution) -> Result<()>;

    async fn list_pending_deletions(&self) -> Vec<PendingDeletionEntity>;
    async fn resolve_pending_deletion(&self, path: &str, accept: bool) -> Result<()>;

    async fn list_versioned_files(&self) -> Vec<String>;
    async fn list_file_versions(&self, rel_path: &str) -> Result<Vec<VersionSnapshotEntity>>;
    async fn restore_file_version(&self, rel_path: &str, timestamp_micros: u64) -> Result<()>;
}

#[async_trait]
pub trait SystemServicePort: Send + Sync {
    fn local_lan_ip(&self) -> Option<String>;
    fn open_folder(&self, path: &std::path::Path) -> Result<()>;
    async fn pick_folder(&self) -> Option<PathBuf>;
    fn copy_to_clipboard(&self, text: &str) -> Result<()>;
}
