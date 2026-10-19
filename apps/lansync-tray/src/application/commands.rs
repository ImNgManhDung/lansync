use crate::domain::models::{ConflictResolution, DeletePolicy};

#[derive(Debug, Clone)]
pub enum AppCommand {
    OpenSyncFolder,
    PickSyncFolder,
    TriggerManualSync,
    SetPaused(bool),
    CopyText(String),
    PairWithAddress(String),
    ApprovePairing(Option<String>),
    RejectPairing(Option<String>),
    UnpairDevice(String),
    RescanPeers,
    SetDeletePolicy(DeletePolicy),
    SetPeerDeletePolicy {
        peer_id: String,
        policy: Option<DeletePolicy>,
    },
    SetBandwidthLimit(u32),
    ResolveConflict {
        conflict_path: String,
        resolution: ConflictResolution,
    },
    ResolvePendingDeletion {
        path: String,
        accept: bool,
    },
    SelectVersionFile(Option<String>),
    RestoreFileVersion {
        rel_path: String,
        timestamp_micros: u64,
    },
}
