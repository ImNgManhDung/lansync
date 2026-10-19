use std::sync::{Arc, RwLock};

use crate::domain::models::{
    ActivityEntity, ConflictEntity, DeletePolicy, PeerEntity, PendingDeletionEntity,
    PendingPairingEntity, VersionSnapshotEntity,
};

#[derive(Debug, Clone)]
pub struct AppStateSnapshot {
    pub device_id: String,
    pub device_name: String,
    pub sync_dir: String,
    pub listen_port: u16,
    pub local_ip: Option<String>,
    pub is_paused: bool,
    pub is_syncing: bool,
    pub active_download: Option<(String, u64)>,
    pub delete_policy: DeletePolicy,
    pub bandwidth_limit_kbps: u32,
    pub pending_pairing: Option<PendingPairingEntity>,
    pub peers: Vec<PeerEntity>,
    pub conflicts: Vec<ConflictEntity>,
    pub pending_deletions: Vec<PendingDeletionEntity>,
    pub versioned_files: Vec<String>,
    pub selected_version_file: Option<String>,
    pub selected_file_versions: Vec<VersionSnapshotEntity>,
    pub activities: Vec<ActivityEntity>,
    pub last_toast: Option<String>,
}

impl Default for AppStateSnapshot {
    fn default() -> Self {
        Self {
            device_id: String::new(),
            device_name: String::new(),
            sync_dir: String::new(),
            listen_port: 42424,
            local_ip: None,
            is_paused: false,
            is_syncing: false,
            active_download: None,
            delete_policy: DeletePolicy::Never,
            bandwidth_limit_kbps: 0,
            pending_pairing: None,
            peers: Vec::new(),
            conflicts: Vec::new(),
            pending_deletions: Vec::new(),
            versioned_files: Vec::new(),
            selected_version_file: None,
            selected_file_versions: Vec::new(),
            activities: Vec::new(),
            last_toast: None,
        }
    }
}

#[derive(Clone, Default)]
pub struct AppStore {
    inner: Arc<RwLock<AppStateSnapshot>>,
}

impl AppStore {
    pub fn snapshot(&self) -> AppStateSnapshot {
        self.inner.read().expect("AppStore poisoned").clone()
    }

    pub fn update<F>(&self, f: F)
    where
        F: FnOnce(&mut AppStateSnapshot),
    {
        if let Ok(mut guard) = self.inner.write() {
            f(&mut guard);
        }
    }

    pub fn push_activity(&self, item: ActivityEntity) {
        self.update(|s| {
            s.activities.insert(0, item);
            if s.activities.len() > 50 {
                s.activities.truncate(50);
            }
        });
    }
}
