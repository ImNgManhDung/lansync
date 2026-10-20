use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::application::commands::AppCommand;
use crate::application::state::{AppStateSnapshot, AppStore};
use crate::domain::models::{ActivityEntity, PendingPairingEntity};
use crate::domain::ports::{BackendEvent, SyncBackendPort, SystemServicePort};

pub struct AppUseCaseInteractor {
    backend: Arc<dyn SyncBackendPort>,
    system: Arc<dyn SystemServicePort>,
    store: AppStore,
    request_show_window: Arc<AtomicBool>,
}

impl AppUseCaseInteractor {
    pub fn new(
        backend: Arc<dyn SyncBackendPort>,
        system: Arc<dyn SystemServicePort>,
        store: AppStore,
        request_show_window: Arc<AtomicBool>,
    ) -> Arc<Self> {
        Arc::new(Self {
            backend,
            system,
            store,
            request_show_window,
        })
    }

    pub async fn initialize_state(&self) {
        let device_id = self.backend.local_device_id();
        let device_name = self.backend.device_name();
        let sync_dir = self.backend.sync_dir().to_string_lossy().to_string();
        let listen_port = self.backend.listen_port();
        let local_ip = self.system.local_lan_ip();
        let is_paused = self.backend.is_paused();
        let bandwidth_limit_kbps = self.backend.bandwidth_limit_kbps();
        let delete_policy = self.backend.get_delete_policy().await;
        let peers = self.backend.list_combined_peers().await;
        let conflicts = self.backend.list_conflicts().await;
        let pending_deletions = self.backend.list_pending_deletions().await;
        let versioned_files = self.backend.list_versioned_files().await;

        self.store.update(|s| {
            *s = AppStateSnapshot {
                device_id,
                device_name,
                sync_dir,
                listen_port,
                local_ip,
                is_paused,
                is_syncing: false,
                active_download: None,
                delete_policy,
                bandwidth_limit_kbps,
                pending_pairing: None,
                peers,
                conflicts,
                pending_deletions,
                versioned_files,
                selected_version_file: None,
                selected_file_versions: Vec::new(),
                activities: Vec::new(),
                last_toast: None,
            };
        });
    }

    pub async fn refresh_snapshot(&self) {
        let local_ip = self.system.local_lan_ip();
        let is_paused = self.backend.is_paused();
        let bandwidth_limit_kbps = self.backend.bandwidth_limit_kbps();
        let delete_policy = self.backend.get_delete_policy().await;
        let peers = self.backend.list_combined_peers().await;
        let conflicts = self.backend.list_conflicts().await;
        let pending_deletions = self.backend.list_pending_deletions().await;
        let versioned_files = self.backend.list_versioned_files().await;

        let selected_file = self.store.snapshot().selected_version_file;
        let selected_versions = if let Some(ref rel) = selected_file {
            self.backend.list_file_versions(rel).await.unwrap_or_default()
        } else {
            Vec::new()
        };

        self.store.update(|s| {
            s.local_ip = local_ip;
            s.is_paused = is_paused;
            s.bandwidth_limit_kbps = bandwidth_limit_kbps;
            s.delete_policy = delete_policy;
            s.peers = peers;
            s.conflicts = conflicts;
            s.pending_deletions = pending_deletions;
            s.versioned_files = versioned_files;
            s.selected_file_versions = selected_versions;
        });
    }

    pub async fn handle_command(&self, cmd: AppCommand) {
        match cmd {
            AppCommand::OpenSyncFolder => {
                let path = self.backend.sync_dir();
                let _ = self.system.open_folder(&path);
            }
            AppCommand::PickSyncFolder => {
                if let Some(picked) = self.system.pick_folder().await {
                    let display = picked.to_string_lossy().to_string();
                    if let Err(e) = self.backend.set_sync_dir(picked).await {
                        self.store.push_activity(ActivityEntity::new(
                            "Lỗi đổi thư mục",
                            e.to_string(),
                            "error",
                        ));
                    } else {
                        self.store.update(|s| {
                            s.sync_dir = display.clone();
                        });
                        self.store.push_activity(ActivityEntity::new(
                            "Đã đổi thư mục đồng bộ",
                            display,
                            "system",
                        ));
                        self.refresh_snapshot().await;
                    }
                }
            }
            AppCommand::TriggerManualSync => {
                self.backend.trigger_manual_sync().await;
                self.store.push_activity(ActivityEntity::new(
                    "Đồng bộ thủ công",
                    "Đang quét và đồng bộ với các thiết bị đã kết nối",
                    "system",
                ));
                self.refresh_snapshot().await;
            }
            AppCommand::SetPaused(paused) => {
                self.backend.set_paused(paused).await;
                self.store.push_activity(ActivityEntity::new(
                    if paused { "Tạm dừng đồng bộ" } else { "Tiếp tục đồng bộ" },
                    if paused {
                        "Hoạt động đồng bộ đang tạm dừng"
                    } else {
                        "Đã tiếp tục đồng bộ tự động"
                    },
                    "system",
                ));
                self.refresh_snapshot().await;
            }
            AppCommand::CopyText(text) => {
                let _ = self.system.copy_to_clipboard(&text);
            }
            AppCommand::PairWithAddress(addr) => {
                match self.backend.pair_with_address(&addr).await {
                    Ok(()) => {
                        self.store.push_activity(ActivityEntity::new(
                            "Đang kết nối ghép đôi",
                            format!("Đã gửi yêu cầu tới {}", addr),
                            "peer",
                        ));
                    }
                    Err(e) => {
                        self.store.push_activity(ActivityEntity::new(
                            "Lỗi kết nối",
                            format!("{}: {}", addr, e),
                            "error",
                        ));
                    }
                }
                self.refresh_snapshot().await;
            }
            AppCommand::ApprovePairing(peer_id_opt) => {
                let target_id = peer_id_opt.or_else(|| {
                    self.store.snapshot().pending_pairing.map(|p| p.peer_id)
                });
                if let Some(peer_id) = target_id {
                    if self.backend.approve_pairing(&peer_id).await.is_ok() {
                        self.store.update(|s| s.pending_pairing = None);
                        self.store.push_activity(ActivityEntity::new(
                            "Đã chấp nhận ghép đôi",
                            format!("Thiết bị: {}", peer_id),
                            "peer",
                        ));
                    }
                }
                self.refresh_snapshot().await;
            }
            AppCommand::RejectPairing(peer_id_opt) => {
                let target_id = peer_id_opt.or_else(|| {
                    self.store.snapshot().pending_pairing.map(|p| p.peer_id)
                });
                if let Some(peer_id) = target_id {
                    if self.backend.reject_pairing(&peer_id).await.is_ok() {
                        self.store.update(|s| s.pending_pairing = None);
                        self.store.push_activity(ActivityEntity::new(
                            "Đã từ chối ghép đôi",
                            format!("Thiết bị: {}", peer_id),
                            "peer",
                        ));
                    }
                }
                self.refresh_snapshot().await;
            }
            AppCommand::UnpairDevice(peer_id) => {
                if self.backend.unpair_peer(&peer_id).await.is_ok() {
                    self.store.push_activity(ActivityEntity::new(
                        "Đã huỷ ghép nối",
                        format!("Thiết bị: {}", peer_id),
                        "peer",
                    ));
                }
                self.refresh_snapshot().await;
            }
            AppCommand::RescanPeers => {
                self.backend.rescan_peers().await;
                self.refresh_snapshot().await;
            }
            AppCommand::SetDeletePolicy(policy) => {
                let _ = self.backend.set_delete_policy(policy).await;
                self.refresh_snapshot().await;
            }
            AppCommand::SetPeerDeletePolicy { peer_id, policy } => {
                let _ = self.backend.set_peer_delete_policy(&peer_id, policy).await;
                self.refresh_snapshot().await;
            }
            AppCommand::SetBandwidthLimit(kbps) => {
                self.backend.set_bandwidth_limit_kbps(kbps);
                self.refresh_snapshot().await;
            }
            AppCommand::ResolveConflict { conflict_path, resolution } => {
                if self.backend.resolve_conflict(&conflict_path, resolution).await.is_ok() {
                    self.store.push_activity(ActivityEntity::new(
                        "Đã xử lý xung đột",
                        conflict_path,
                        "conflict",
                    ));
                }
                self.refresh_snapshot().await;
            }
            AppCommand::ResolvePendingDeletion { path, accept } => {
                if self.backend.resolve_pending_deletion(&path, accept).await.is_ok() {
                    self.store.push_activity(ActivityEntity::new(
                        if accept { "Đã chuyển vào Thùng rác" } else { "Đã giữ lại file" },
                        path,
                        "file",
                    ));
                }
                self.refresh_snapshot().await;
            }
            AppCommand::SelectVersionFile(file_opt) => {
                let versions = if let Some(ref rel) = file_opt {
                    self.backend.list_file_versions(rel).await.unwrap_or_default()
                } else {
                    Vec::new()
                };
                self.store.update(|s| {
                    s.selected_version_file = file_opt;
                    s.selected_file_versions = versions;
                });
            }
            AppCommand::RestoreFileVersion { rel_path, timestamp_micros } => {
                if self.backend.restore_file_version(&rel_path, timestamp_micros).await.is_ok() {
                    self.store.push_activity(ActivityEntity::new(
                        "Đã khôi phục phiên bản",
                        format!("Khôi phục bản lưu của {}", rel_path),
                        "file",
                    ));
                }
                self.refresh_snapshot().await;
            }
        }
    }

    pub async fn handle_backend_event(&self, event: BackendEvent) {
        match event {
            BackendEvent::PeerDiscovered {
                device_id: _,
                device_name,
            } => {
                let already_logged = self
                    .store
                    .snapshot()
                    .activities
                    .first()
                    .map(|a| a.title == "Phát hiện thiết bị" && a.details == device_name)
                    .unwrap_or(false);
                if !already_logged {
                    self.store.push_activity(ActivityEntity::new(
                        "Phát hiện thiết bị",
                        device_name,
                        "peer",
                    ));
                }
                self.refresh_snapshot().await;
            }
            BackendEvent::PairingRequested { peer_id, device_name, code } => {
                self.store.update(|s| {
                    s.pending_pairing = Some(PendingPairingEntity {
                        peer_id: peer_id.clone(),
                        device_name: device_name.clone(),
                        code: code.clone(),
                    });
                });
                self.store.push_activity(ActivityEntity::new(
                    "Yêu cầu ghép đôi",
                    format!("Thiết bị: {}, Mã: {}", device_name, code),
                    "peer",
                ));
                self.request_show_window.store(true, Ordering::SeqCst);
            }
            BackendEvent::PairingAccepted { peer_id } => {
                self.store.update(|s| s.pending_pairing = None);
                self.store.push_activity(ActivityEntity::new(
                    "Ghép đôi thành công",
                    format!("ID: {}", peer_id),
                    "peer",
                ));
                self.refresh_snapshot().await;
            }
            BackendEvent::PairingRejected => {
                self.store.update(|s| s.pending_pairing = None);
                self.refresh_snapshot().await;
            }
            BackendEvent::PeerConnected { device_name } => {
                self.store.push_activity(ActivityEntity::new(
                    "Đã kết nối",
                    device_name,
                    "peer",
                ));
                self.refresh_snapshot().await;
            }
            BackendEvent::PeerDisconnected { peer_id } => {
                self.store.push_activity(ActivityEntity::new(
                    "Ngắt kết nối",
                    format!("ID: {}", peer_id),
                    "peer",
                ));
                self.refresh_snapshot().await;
            }
            BackendEvent::PeerStatusChanged => {
                self.refresh_snapshot().await;
            }
            BackendEvent::SyncStarted => {
                self.store.update(|s| s.is_syncing = true);
            }
            BackendEvent::FileDownloading { rel_path, size } => {
                self.store.update(|s| {
                    s.is_syncing = true;
                    s.active_download = Some((rel_path.clone(), size));
                });
                self.store.push_activity(ActivityEntity::new(
                    "Đang tải file",
                    format!("{} ({} B)", rel_path, size),
                    "file",
                ));
            }
            BackendEvent::FileDownloaded { rel_path } => {
                self.store.update(|s| {
                    s.is_syncing = false;
                    s.active_download = None;
                });
                self.store.push_activity(ActivityEntity::new(
                    "Tải xong file",
                    rel_path,
                    "file",
                ));
                self.refresh_snapshot().await;
            }
            BackendEvent::FileTrashed { rel_path } => {
                self.store.push_activity(ActivityEntity::new(
                    "Đã chuyển vào Thùng rác",
                    rel_path,
                    "file",
                ));
                self.refresh_snapshot().await;
            }
            BackendEvent::PendingDeletion { peer_id, rel_path } => {
                self.store.push_activity(ActivityEntity::new(
                    "Yêu cầu xoá file",
                    format!("Thiết bị {} xoá {}", peer_id, rel_path),
                    "file",
                ));
                self.refresh_snapshot().await;
            }
            BackendEvent::ConflictCreated { original, conflict } => {
                self.store.push_activity(ActivityEntity::new(
                    "Xung đột file",
                    format!("{} -> {}", original, conflict),
                    "conflict",
                ));
                self.refresh_snapshot().await;
            }
            BackendEvent::SyncCompleted => {
                self.store.update(|s| {
                    s.is_syncing = false;
                    s.active_download = None;
                });
                self.refresh_snapshot().await;
            }
            BackendEvent::Error(err) => {
                self.store.push_activity(ActivityEntity::new(
                    "Lỗi hệ thống",
                    err,
                    "error",
                ));
            }
        }
    }

    pub fn spawn_worker(
        self: Arc<Self>,
        mut rx_cmd: mpsc::UnboundedReceiver<AppCommand>,
        repaint_signal: Arc<AtomicBool>,
    ) {
        let mut rx_events = self.backend.subscribe_events();
        tokio::spawn(async move {
            self.initialize_state().await;
            repaint_signal.store(true, Ordering::SeqCst);

            let mut ticker = tokio::time::interval(Duration::from_secs(3));
            ticker.tick().await;

            loop {
                tokio::select! {
                    Some(cmd) = rx_cmd.recv() => {
                        self.handle_command(cmd).await;
                        repaint_signal.store(true, Ordering::SeqCst);
                    }
                    Ok(ev) = rx_events.recv() => {
                        self.handle_backend_event(ev).await;
                        repaint_signal.store(true, Ordering::SeqCst);
                    }
                    _ = ticker.tick() => {
                        self.refresh_snapshot().await;
                        repaint_signal.store(true, Ordering::SeqCst);
                    }
                }
            }
        });
    }
}

