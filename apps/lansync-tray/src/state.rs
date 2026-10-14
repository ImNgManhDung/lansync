use chrono::Local;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

use lansync_core::engine::SyncEngine;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingPairingInfo {
    pub peer_id: String,
    pub device_name: String,
    pub code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityItem {
    pub id: String,
    pub timestamp: String,
    pub title: String,
    pub details: String,
    pub kind: String, // "sync", "file", "conflict", "peer", "error"
}

impl ActivityItem {
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

pub struct AppState {
    pub engine: Arc<SyncEngine>,
    #[allow(dead_code)]
    pub data_dir: PathBuf,
    #[allow(dead_code)]
    pub config_path: PathBuf,
    pub sync_dir: PathBuf,
    pub device_name: String,
    pub listen_port: u16,
    pub activities: Arc<Mutex<Vec<ActivityItem>>>,
    pub pending_pairing: Arc<Mutex<Option<PendingPairingInfo>>>,
}

impl AppState {
    pub async fn add_activity(&self, item: ActivityItem) {
        let mut list = self.activities.lock().await;
        list.insert(0, item);
        if list.len() > 50 {
            list.truncate(50);
        }
    }
}
