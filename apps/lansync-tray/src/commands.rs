use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use tauri::State;

use crate::state::{ActivityItem, AppState, PendingPairingInfo};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusResponse {
    pub device_id: String,
    pub device_name: String,
    pub sync_dir: String,
    pub listen_port: u16,
    pub local_ip: Option<String>,
    pub activities: Vec<ActivityItem>,
    pub pending_pairing: Option<PendingPairingInfo>,
}

fn get_local_ip() -> Option<String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    Some(socket.local_addr().ok()?.ip().to_string())
}

#[tauri::command]
pub async fn get_status(state: State<'_, Arc<AppState>>) -> Result<StatusResponse, String> {
    let activities = state.activities.lock().await.clone();
    let pending_pairing = state.pending_pairing.lock().await.clone();
    Ok(StatusResponse {
        device_id: state.engine.local_device_id().to_string(),
        device_name: state.device_name.clone(),
        sync_dir: state.sync_dir.to_string_lossy().to_string(),
        listen_port: state.listen_port,
        local_ip: get_local_ip(),
        activities,
        pending_pairing,
    })
}

#[tauri::command]
pub async fn open_sync_folder(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let path = &state.sync_dir;
    open::that(path).map_err(|e| format!("Failed to open folder: {}", e))
}

#[tauri::command]
pub async fn pair_device(addr: String, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let clean_addr = addr.trim();
    let socket_addr: SocketAddr = if clean_addr.contains(':') {
        clean_addr.parse()
    } else {
        format!("{}:{}", clean_addr, state.listen_port).parse()
    }
    .map_err(|e| format!("Invalid address (example: 192.168.1.10:42424 or 192.168.1.10): {}", e))?;

    state
        .engine
        .pair_with(socket_addr)
        .await
        .map_err(|e| format!("Failed to connect for pairing: {}", e))?;

    state
        .add_activity(ActivityItem::new(
            "Connecting for Pairing",
            format!("Sent request to {}", socket_addr),
            "peer",
        ))
        .await;

    Ok(())
}

#[tauri::command]
pub async fn approve_pairing(peer_id: Option<String>, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let target_id = if let Some(id) = peer_id.filter(|s| !s.is_empty()) {
        id
    } else if let Some(ref p) = *state.pending_pairing.lock().await {
        p.peer_id.clone()
    } else {
        return Err("No pending pairing request found".into());
    };

    state
        .engine
        .approve_pairing(&target_id)
        .await
        .map_err(|e| format!("Failed to approve pairing: {}", e))?;

    *state.pending_pairing.lock().await = None;

    state
        .add_activity(ActivityItem::new(
            "Pairing Approved",
            format!("Device: {}", target_id),
            "peer",
        ))
        .await;

    Ok(())
}

#[tauri::command]
pub async fn reject_pairing(peer_id: Option<String>, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let target_id = if let Some(id) = peer_id.filter(|s| !s.is_empty()) {
        id
    } else if let Some(ref p) = *state.pending_pairing.lock().await {
        p.peer_id.clone()
    } else {
        return Err("No pending pairing request found".into());
    };

    state
        .engine
        .reject_pairing(&target_id)
        .await
        .map_err(|e| format!("Failed to reject pairing: {}", e))?;

    *state.pending_pairing.lock().await = None;

    state
        .add_activity(ActivityItem::new(
            "Pairing Rejected",
            format!("Device: {}", target_id),
            "peer",
        ))
        .await;

    Ok(())
}

#[tauri::command]
pub async fn pick_sync_folder() -> Result<Option<String>, String> {
    let folder = rfd::AsyncFileDialog::new()
        .set_title("Select LanSync folder")
        .pick_folder()
        .await;

    Ok(folder.map(|f| f.path().to_string_lossy().to_string()))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerItem {
    pub device_id: String,
    pub name: String,
    pub cert_fingerprint: String,
    pub last_addr: Option<String>,
    pub is_connected: bool,
    pub delete_mode: Option<String>,
}

#[tauri::command]
pub async fn list_peers(state: State<'_, Arc<AppState>>) -> Result<Vec<PeerItem>, String> {
    let peers = state.engine.list_peers().await;
    let connected = state.engine.connected_peer_ids().await;
    Ok(peers
        .into_iter()
        .map(|p| {
            let is_connected = connected.contains(&p.device_id);
            let delete_mode = p.delete_mode.map(format_delete_mode);
            PeerItem {
                device_id: p.device_id,
                name: p.name,
                cert_fingerprint: p.cert_fingerprint,
                last_addr: p.last_addr,
                is_connected,
                delete_mode,
            }
        })
        .collect())
}

#[tauri::command]
pub async fn unpair_device(peer_id: String, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    state
        .engine
        .unpair_peer(&peer_id)
        .await
        .map_err(|e| format!("Failed to unpair device: {}", e))?;

    state
        .add_activity(ActivityItem::new(
            "Device Unpaired",
            format!("Unpaired device: {}", peer_id),
            "peer",
        ))
        .await;

    Ok(())
}

#[tauri::command]
pub async fn list_discovered_peers(
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<lansync_core::discovery::DiscoveredPeer>, String> {
    Ok(state.engine.list_discovered_peers().await)
}

#[tauri::command]
pub async fn rescan_peers(
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<lansync_core::discovery::DiscoveredPeer>, String> {
    state.engine.rescan_peers().await;
    Ok(state.engine.list_discovered_peers().await)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictItem {
    pub conflict_path: String,
    pub original_path: String,
    pub size: u64,
    pub modified_at: String,
}

#[tauri::command]
pub async fn list_conflicts(state: State<'_, Arc<AppState>>) -> Result<Vec<ConflictItem>, String> {
    let sync_dir = state.sync_dir.clone();
    let mut conflicts = Vec::new();

    for entry in walkdir::WalkDir::new(&sync_dir).into_iter().filter_map(|e| e.ok()) {
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
                    let dt: chrono::DateTime<chrono::Local> = t.into();
                    dt.format("%Y-%m-%d %H:%M").to_string()
                })
                .unwrap_or_else(|| "Unknown".to_string());

            let original_path = derive_original_from_conflict(&rel);

            conflicts.push(ConflictItem {
                conflict_path: rel,
                original_path,
                size,
                modified_at,
            });
        }
    }

    Ok(conflicts)
}

fn derive_original_from_conflict(conflict_rel: &str) -> String {
    if let Some(idx) = conflict_rel.find(" (conflict-") {
        let (prefix, rest) = conflict_rel.split_at(idx);
        if let Some(close_idx) = rest.find(')') {
            let ext = &rest[close_idx + 1..];
            return format!("{}{}", prefix, ext);
        }
    }
    conflict_rel.to_string()
}

#[tauri::command]
pub async fn resolve_conflict(
    conflict_path: String,
    resolution: String,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    let sync_dir = state.sync_dir.clone();
    let original_rel = derive_original_from_conflict(&conflict_path);
    let conflict_full = sync_dir.join(&conflict_path);
    let original_full = sync_dir.join(&original_rel);

    match resolution.as_str() {
        "keep_mine" => {
            if conflict_full.exists() {
                let _ = tokio::fs::rename(&conflict_full, &original_full).await;
            }
        }
        "keep_theirs" => {
            if conflict_full.exists() {
                let _ = tokio::fs::remove_file(&conflict_full).await;
            }
        }
        "keep_both" => {}
        other => return Err(format!("Unknown resolution option: {}", other)),
    }

    state
        .add_activity(ActivityItem::new(
            "Conflict Resolved",
            format!("Resolved {} ({})", original_rel, resolution),
            "conflict",
        ))
        .await;

    Ok(())
}

fn format_delete_mode(mode: lansync_core::config::DeleteMode) -> String {
    match mode {
        lansync_core::config::DeleteMode::Never => "never".to_string(),
        lansync_core::config::DeleteMode::Ask => "ask".to_string(),
        lansync_core::config::DeleteMode::MirrorTrash => "mirror_trash".to_string(),
    }
}

fn parse_delete_mode(mode: &str) -> Result<lansync_core::config::DeleteMode, String> {
    match mode {
        "never" => Ok(lansync_core::config::DeleteMode::Never),
        "ask" => Ok(lansync_core::config::DeleteMode::Ask),
        "mirror_trash" => Ok(lansync_core::config::DeleteMode::MirrorTrash),
        other => Err(format!("Unknown delete mode: {}", other)),
    }
}

#[tauri::command]
pub async fn get_delete_mode(state: State<'_, Arc<AppState>>) -> Result<String, String> {
    let mode = state.engine.get_delete_mode().await;
    Ok(format_delete_mode(mode))
}

#[tauri::command]
pub async fn set_delete_mode(mode: String, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let parsed = parse_delete_mode(&mode)?;
    state.engine.set_delete_mode(parsed).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_peer_delete_mode(
    peer_id: String,
    mode: Option<String>,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    let parsed = match mode {
        Some(m) if !m.is_empty() && m != "default" => Some(parse_delete_mode(&m)?),
        _ => None,
    };
    state.engine.set_peer_delete_mode(&peer_id, parsed).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_pending_deletions(
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<lansync_core::engine::PendingDeletion>, String> {
    Ok(state.engine.list_pending_deletions().await)
}

#[tauri::command]
pub async fn resolve_pending_deletion(
    path: String,
    accept: bool,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    state
        .engine
        .resolve_pending_deletion(&path, accept)
        .await
        .map_err(|e| e.to_string())?;

    state
        .add_activity(ActivityItem::new(
            if accept { "Moved to Trash" } else { "Deletion Ignored" },
            path,
            "file",
        ))
        .await;

    Ok(())
}

#[tauri::command]
pub async fn list_versioned_files(state: State<'_, Arc<AppState>>) -> Result<Vec<String>, String> {
    Ok(lansync_core::versioning::list_all_versioned_files(&state.sync_dir))
}

#[tauri::command]
pub async fn list_file_versions(
    rel_path: String,
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<lansync_core::versioning::VersionItem>, String> {
    lansync_core::versioning::list_versions(&state.sync_dir, &rel_path)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn restore_file_version(
    rel_path: String,
    timestamp_micros: u64,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    lansync_core::versioning::restore_version_by_timestamp(&state.sync_dir, &rel_path, timestamp_micros)
        .await
        .map_err(|e| e.to_string())?;

    state
        .add_activity(ActivityItem::new(
            "Version Restored",
            format!("Restored snapshot of {}", rel_path),
            "file",
        ))
        .await;

    Ok(())
}

#[tauri::command]
pub async fn is_sync_paused(state: State<'_, Arc<AppState>>) -> Result<bool, String> {
    Ok(state.engine.is_paused())
}

#[tauri::command]
pub async fn set_sync_paused(paused: bool, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    state.engine.set_paused(paused).await;
    state
        .add_activity(ActivityItem::new(
            if paused { "Sync Paused" } else { "Sync Resumed" },
            if paused { "Sync activity temporarily paused" } else { "Sync activity resumed" },
            "system",
        ))
        .await;
    Ok(())
}

#[tauri::command]
pub async fn get_bandwidth_limit(state: State<'_, Arc<AppState>>) -> Result<u32, String> {
    Ok(state.engine.get_bandwidth_limit_kbps())
}

#[tauri::command]
pub async fn set_bandwidth_limit(limit_kbps: u32, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    state.engine.set_bandwidth_limit_kbps(limit_kbps);
    Ok(())
}

#[tauri::command]
pub async fn trigger_manual_sync(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    state.engine.trigger_sync_all().await;
    state
        .add_activity(ActivityItem::new(
            "Manual Sync Triggered",
            "Scanning and syncing with connected peers",
            "system",
        ))
        .await;
    Ok(())
}

