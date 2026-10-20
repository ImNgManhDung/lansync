use anyhow::Result;
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::broadcast;

use lansync_tray::application::commands::AppCommand;
use lansync_tray::application::state::AppStore;
use lansync_tray::application::use_cases::AppUseCaseInteractor;
use lansync_tray::domain::models::{
    derive_original_from_conflict, format_bytes, ConflictEntity, ConflictResolution, DeletePolicy,
    PeerEntity, PendingDeletionEntity, VersionSnapshotEntity,
};
use lansync_tray::domain::ports::{BackendEvent, SyncBackendPort, SystemServicePort};

struct MockBackend {
    paused: AtomicBool,
    tx: broadcast::Sender<BackendEvent>,
}

impl MockBackend {
    fn new() -> Self {
        let (tx, _) = broadcast::channel(16);
        Self {
            paused: AtomicBool::new(false),
            tx,
        }
    }
}

#[async_trait]
impl SyncBackendPort for MockBackend {
    fn local_device_id(&self) -> String {
        "MOCK_DEV_12345678".into()
    }
    fn device_name(&self) -> String {
        "MockMac".into()
    }
    fn sync_dir(&self) -> PathBuf {
        PathBuf::from("/tmp/mock-sync")
    }
    async fn set_sync_dir(&self, _new_dir: PathBuf) -> Result<()> {
        Ok(())
    }
    fn listen_port(&self) -> u16 {
        42424
    }
    fn subscribe_events(&self) -> broadcast::Receiver<BackendEvent> {
        self.tx.subscribe()
    }
    fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }
    async fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::SeqCst);
    }
    fn bandwidth_limit_kbps(&self) -> u32 {
        0
    }
    fn set_bandwidth_limit_kbps(&self, _kbps: u32) {}
    async fn get_delete_policy(&self) -> DeletePolicy {
        DeletePolicy::Never
    }
    async fn set_delete_policy(&self, _policy: DeletePolicy) -> Result<()> {
        Ok(())
    }
    async fn set_peer_delete_policy(
        &self,
        _peer_id: &str,
        _policy: Option<DeletePolicy>,
    ) -> Result<()> {
        Ok(())
    }
    async fn trigger_manual_sync(&self) {}
    async fn rescan_peers(&self) {}
    async fn pair_with_address(&self, _addr: &str) -> Result<()> {
        Ok(())
    }
    async fn approve_pairing(&self, _peer_id: &str) -> Result<()> {
        Ok(())
    }
    async fn reject_pairing(&self, _peer_id: &str) -> Result<()> {
        Ok(())
    }
    async fn unpair_peer(&self, _peer_id: &str) -> Result<()> {
        Ok(())
    }
    async fn list_combined_peers(&self) -> Vec<PeerEntity> {
        vec![]
    }
    async fn list_conflicts(&self) -> Vec<ConflictEntity> {
        vec![]
    }
    async fn resolve_conflict(&self, _p: &str, _r: ConflictResolution) -> Result<()> {
        Ok(())
    }
    async fn list_pending_deletions(&self) -> Vec<PendingDeletionEntity> {
        vec![]
    }
    async fn resolve_pending_deletion(&self, _p: &str, _a: bool) -> Result<()> {
        Ok(())
    }
    async fn list_versioned_files(&self) -> Vec<String> {
        vec![]
    }
    async fn list_file_versions(&self, _r: &str) -> Result<Vec<VersionSnapshotEntity>> {
        Ok(vec![])
    }
    async fn restore_file_version(&self, _r: &str, _t: u64) -> Result<()> {
        Ok(())
    }
}

struct MockSystem;

#[async_trait]
impl SystemServicePort for MockSystem {
    fn local_lan_ip(&self) -> Option<String> {
        Some("192.168.1.99".into())
    }
    fn open_folder(&self, _path: &Path) -> Result<()> {
        Ok(())
    }
    async fn pick_folder(&self) -> Option<PathBuf> {
        None
    }
    fn copy_to_clipboard(&self, _text: &str) -> Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn test_use_case_interactor_pause_and_pairing_event() {
    let store = AppStore::default();
    let show_flag = Arc::new(AtomicBool::new(false));
    let interactor = AppUseCaseInteractor::new(
        Arc::new(MockBackend::new()),
        Arc::new(MockSystem),
        store.clone(),
        show_flag.clone(),
    );

    interactor.initialize_state().await;
    let snap = store.snapshot();
    assert_eq!(snap.device_name, "MockMac");
    assert_eq!(snap.local_ip.as_deref(), Some("192.168.1.99"));
    assert!(!snap.is_paused);

    interactor.handle_command(AppCommand::SetPaused(true)).await;
    assert!(store.snapshot().is_paused);

    interactor
        .handle_backend_event(BackendEvent::PairingRequested {
            peer_id: "PEER_999".into(),
            device_name: "WinPC".into(),
            code: "654321".into(),
        })
        .await;

    let snap2 = store.snapshot();
    assert_eq!(snap2.pending_pairing.unwrap().code, "654321");
    assert!(show_flag.load(Ordering::SeqCst));
}

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
    for policy in [
        DeletePolicy::Never,
        DeletePolicy::Ask,
        DeletePolicy::MirrorTrash,
    ] {
        assert_eq!(DeletePolicy::from_code(policy.as_code()), Some(policy));
    }
    assert_eq!(DeletePolicy::from_code("unknown"), None);
}
