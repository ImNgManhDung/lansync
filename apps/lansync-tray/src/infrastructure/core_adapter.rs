use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chrono::{Local, TimeZone};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::broadcast;

use lansync_core::config::DeleteMode;
use lansync_core::engine::{SyncEngine, SyncEvent};

use crate::domain::models::{
    derive_original_from_conflict, ConflictEntity, ConflictResolution, DeletePolicy, PeerEntity,
    PendingDeletionEntity, VersionSnapshotEntity,
};
use crate::domain::ports::{BackendEvent, SyncBackendPort};

pub struct LanSyncCoreAdapter {
    engine: Arc<SyncEngine>,
    sync_dir: std::sync::RwLock<PathBuf>,
    device_name: String,
    listen_port: u16,
    events_tx: broadcast::Sender<BackendEvent>,
}

impl LanSyncCoreAdapter {
    pub fn new(
        engine: Arc<SyncEngine>,
        sync_dir: PathBuf,
        device_name: String,
        listen_port: u16,
    ) -> Self {
        let (events_tx, _) = broadcast::channel(128);
        let mut core_rx = engine.subscribe_events();
        let tx_clone = events_tx.clone();

        tokio::spawn(async move {
            while let Ok(ev) = core_rx.recv().await {
                if let Some(mapped) = map_core_event(ev) {
                    let _ = tx_clone.send(mapped);
                }
            }
        });

        Self {
            engine,
            sync_dir: std::sync::RwLock::new(sync_dir),
            device_name,
            listen_port,
            events_tx,
        }
    }
}

fn map_core_event(ev: SyncEvent) -> Option<BackendEvent> {
    match ev {
        SyncEvent::PeerDiscovered(p) => Some(BackendEvent::PeerDiscovered {
            device_id: p.device_id,
            device_name: p.device_name,
        }),
        SyncEvent::PairingRequested {
            peer_id,
            device_name,
            code,
        } => Some(BackendEvent::PairingRequested {
            peer_id,
            device_name,
            code,
        }),
        SyncEvent::PairingAccepted { peer_id } => Some(BackendEvent::PairingAccepted { peer_id }),
        SyncEvent::PairingRejected { .. } => Some(BackendEvent::PairingRejected),
        SyncEvent::PeerConnected { device_name, .. } => {
            Some(BackendEvent::PeerConnected { device_name })
        }
        SyncEvent::PeerDisconnected { peer_id } => {
            Some(BackendEvent::PeerDisconnected { peer_id })
        }
        SyncEvent::PeerStatusChanged { .. } => Some(BackendEvent::PeerStatusChanged),
        SyncEvent::SyncStarted { .. } => Some(BackendEvent::SyncStarted),
        SyncEvent::FileDownloading { rel_path, size } => {
            Some(BackendEvent::FileDownloading { rel_path, size })
        }
        SyncEvent::FileDownloaded { rel_path } => {
            Some(BackendEvent::FileDownloaded { rel_path })
        }
        SyncEvent::FileTrashed { rel_path, .. } => {
            Some(BackendEvent::FileTrashed { rel_path })
        }
        SyncEvent::PendingDeletion { peer_id, rel_path } => {
            Some(BackendEvent::PendingDeletion { peer_id, rel_path })
        }
        SyncEvent::ConflictCreated { original, conflict } => {
            Some(BackendEvent::ConflictCreated { original, conflict })
        }
        SyncEvent::SyncCompleted { .. } => Some(BackendEvent::SyncCompleted),
        SyncEvent::Error(msg) => Some(BackendEvent::Error(msg)),
    }
}

fn to_domain_policy(m: DeleteMode) -> DeletePolicy {
    match m {
        DeleteMode::Never => DeletePolicy::Never,
        DeleteMode::Ask => DeletePolicy::Ask,
        DeleteMode::MirrorTrash => DeletePolicy::MirrorTrash,
    }
}

fn to_core_mode(p: DeletePolicy) -> DeleteMode {
    match p {
        DeletePolicy::Never => DeleteMode::Never,
        DeletePolicy::Ask => DeleteMode::Ask,
        DeletePolicy::MirrorTrash => DeleteMode::MirrorTrash,
    }
}

#[async_trait]
impl SyncBackendPort for LanSyncCoreAdapter {
    fn local_device_id(&self) -> String {
        self.engine.local_device_id().to_string()
    }

    fn device_name(&self) -> String {
        self.device_name.clone()
    }

    fn sync_dir(&self) -> PathBuf {
        self.sync_dir.read().expect("sync_dir poisoned").clone()
    }

    async fn set_sync_dir(&self, new_dir: PathBuf) -> Result<()> {
        self.engine.set_sync_dir(new_dir.clone()).await?;
        if let Ok(mut guard) = self.sync_dir.write() {
            *guard = new_dir;
        }
        Ok(())
    }

    fn listen_port(&self) -> u16 {
        self.listen_port
    }

    fn subscribe_events(&self) -> broadcast::Receiver<BackendEvent> {
        self.events_tx.subscribe()
    }

    fn is_paused(&self) -> bool {
        self.engine.is_paused()
    }

    async fn set_paused(&self, paused: bool) {
        self.engine.set_paused(paused).await;
    }

    fn bandwidth_limit_kbps(&self) -> u32 {
        self.engine.get_bandwidth_limit_kbps()
    }

    fn set_bandwidth_limit_kbps(&self, kbps: u32) {
        self.engine.set_bandwidth_limit_kbps(kbps);
    }

    async fn get_delete_policy(&self) -> DeletePolicy {
        to_domain_policy(self.engine.get_delete_mode().await)
    }

    async fn set_delete_policy(&self, policy: DeletePolicy) -> Result<()> {
        self.engine.set_delete_mode(to_core_mode(policy)).await
    }

    async fn set_peer_delete_policy(
        &self,
        peer_id: &str,
        policy: Option<DeletePolicy>,
    ) -> Result<()> {
        self.engine
            .set_peer_delete_mode(peer_id, policy.map(to_core_mode))
            .await
    }

    async fn trigger_manual_sync(&self) {
        self.engine.trigger_sync_all().await;
    }

    async fn rescan_peers(&self) {
        self.engine.rescan_peers().await;
    }

    async fn pair_with_address(&self, addr: &str) -> Result<()> {
        let clean_addr = addr.trim();
        let socket_addr: SocketAddr = if clean_addr.contains(':') {
            clean_addr.parse()
        } else {
            format!("{}:{}", clean_addr, self.listen_port).parse()
        }
        .map_err(|e| anyhow!("Địa chỉ không hợp lệ ({}): {}", clean_addr, e))?;

        self.engine.pair_with(socket_addr).await
    }

    async fn approve_pairing(&self, peer_id: &str) -> Result<()> {
        self.engine.approve_pairing(peer_id).await
    }

    async fn reject_pairing(&self, peer_id: &str) -> Result<()> {
        self.engine.reject_pairing(peer_id).await
    }

    async fn unpair_peer(&self, peer_id: &str) -> Result<()> {
        self.engine.unpair_peer(peer_id).await.map(|_| ())
    }

    async fn list_combined_peers(&self) -> Vec<PeerEntity> {
        let paired_list = self.engine.list_peers().await;
        let connected = self.engine.connected_peer_ids().await;
        let discovered_list = self.engine.list_discovered_peers().await;

        let mut map: HashMap<String, PeerEntity> = HashMap::new();

        for p in paired_list {
            let is_connected = connected.contains(&p.device_id);
            let status = self.engine.get_peer_status(&p.device_id).await.to_string();
            map.insert(
                p.device_id.clone(),
                PeerEntity {
                    device_id: p.device_id,
                    name: p.name,
                    is_paired: true,
                    is_connected,
                    status,
                    address: p.last_addr,
                    delete_policy: p.delete_mode.map(to_domain_policy),
                },
            );
        }

        for dp in discovered_list {
            let best_addr = dp
                .addresses
                .iter()
                .find(|a| a.is_ipv4())
                .or_else(|| dp.addresses.first())
                .map(|a| a.to_string());

            if let Some(existing) = map.get_mut(&dp.device_id) {
                if best_addr.is_some() {
                    existing.address = best_addr;
                }
                if !dp.device_name.is_empty() {
                    existing.name = dp.device_name;
                }
            } else {
                let is_connected = connected.contains(&dp.device_id);
                let status = self.engine.get_peer_status(&dp.device_id).await.to_string();
                map.insert(
                    dp.device_id.clone(),
                    PeerEntity {
                        device_id: dp.device_id,
                        name: dp.device_name,
                        is_paired: false,
                        is_connected,
                        status,
                        address: best_addr,
                        delete_policy: None,
                    },
                );
            }
        }

        let mut out: Vec<PeerEntity> = map.into_values().collect();
        out.sort_by(|a, b| {
            b.is_paired
                .cmp(&a.is_paired)
                .then_with(|| b.is_connected.cmp(&a.is_connected))
                .then_with(|| a.name.cmp(&b.name))
        });
        out
    }

    async fn list_conflicts(&self) -> Vec<ConflictEntity> {
        let sync_dir = self.sync_dir();
        let mut conflicts = Vec::new();

        for entry in walkdir::WalkDir::new(&sync_dir)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let Some(rel) = lansync_core::paths::normalize_rel(&sync_dir, entry.path()) else {
                continue;
            };
            if rel.contains(" (conflict-") {
                let meta = entry.metadata().ok();
                let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
                let modified_at = meta
                    .and_then(|m| m.modified().ok())
                    .map(|t| {
                        let dt: chrono::DateTime<Local> = t.into();
                        dt.format("%Y-%m-%d %H:%M").to_string()
                    })
                    .unwrap_or_else(|| "Unknown".to_string());

                let original_path = derive_original_from_conflict(&rel);
                conflicts.push(ConflictEntity {
                    conflict_path: rel,
                    original_path,
                    size,
                    modified_at,
                });
            }
        }

        conflicts
    }

    async fn resolve_conflict(
        &self,
        conflict_path: &str,
        resolution: ConflictResolution,
    ) -> Result<()> {
        let sync_dir = self.sync_dir();
        let original_rel = derive_original_from_conflict(conflict_path);
        let conflict_full = sync_dir.join(conflict_path);
        let original_full = sync_dir.join(&original_rel);

        match resolution {
            ConflictResolution::KeepMine => {
                if conflict_full.exists() {
                    let _ = tokio::fs::rename(&conflict_full, &original_full).await;
                }
            }
            ConflictResolution::KeepTheirs => {
                if conflict_full.exists() {
                    let _ = tokio::fs::remove_file(&conflict_full).await;
                }
            }
            ConflictResolution::KeepBoth => {}
        }
        Ok(())
    }

    async fn list_pending_deletions(&self) -> Vec<PendingDeletionEntity> {
        self.engine
            .list_pending_deletions()
            .await
            .into_iter()
            .map(|d| PendingDeletionEntity {
                peer_id: d.peer_id,
                path: d.path,
                timestamp: d.timestamp,
            })
            .collect()
    }

    async fn resolve_pending_deletion(&self, path: &str, accept: bool) -> Result<()> {
        self.engine.resolve_pending_deletion(path, accept).await
    }

    async fn list_versioned_files(&self) -> Vec<String> {
        let sync_dir = self.sync_dir();
        lansync_core::versioning::list_all_versioned_files(&sync_dir)
    }

    async fn list_file_versions(&self, rel_path: &str) -> Result<Vec<VersionSnapshotEntity>> {
        let sync_dir = self.sync_dir();
        let items = lansync_core::versioning::list_versions(&sync_dir, rel_path).await?;
        Ok(items
            .into_iter()
            .map(|v| {
                let secs = (v.timestamp_micros / 1_000_000) as i64;
                let formatted_time = Local
                    .timestamp_opt(secs, 0)
                    .single()
                    .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                    .unwrap_or_else(|| v.timestamp_micros.to_string());
                VersionSnapshotEntity {
                    rel_path: rel_path.to_string(),
                    timestamp_micros: v.timestamp_micros,
                    formatted_time,
                    size: v.size,
                }
            })
            .collect())
    }

    async fn restore_file_version(&self, rel_path: &str, timestamp_micros: u64) -> Result<()> {
        let sync_dir = self.sync_dir();
        lansync_core::versioning::restore_version_by_timestamp(
            &sync_dir,
            rel_path,
            timestamp_micros,
        )
        .await
    }
}
