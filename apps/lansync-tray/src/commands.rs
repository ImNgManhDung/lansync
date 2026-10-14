use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use tauri::State;

use crate::state::{ActivityItem, AppState};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusResponse {
    pub device_id: String,
    pub device_name: String,
    pub sync_dir: String,
    pub listen_port: u16,
    pub activities: Vec<ActivityItem>,
}

#[tauri::command]
pub async fn get_status(state: State<'_, Arc<AppState>>) -> Result<StatusResponse, String> {
    let activities = state.activities.lock().await.clone();
    Ok(StatusResponse {
        device_id: state.engine.local_device_id().to_string(),
        device_name: state.device_name.clone(),
        sync_dir: state.sync_dir.to_string_lossy().to_string(),
        listen_port: state.listen_port,
        activities,
    })
}

#[tauri::command]
pub async fn open_sync_folder(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let path = &state.sync_dir;
    open::that(path).map_err(|e| format!("Failed to open folder: {}", e))
}

#[tauri::command]
pub async fn pair_device(addr: String, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let socket_addr: SocketAddr = addr
        .parse()
        .map_err(|e| format!("Invalid address (example: 192.168.1.10:42424): {}", e))?;

    state
        .engine
        .pair_with(socket_addr)
        .await
        .map_err(|e| format!("Failed to connect for pairing: {}", e))?;

    state
        .add_activity(ActivityItem::new(
            "Connecting for Pairing",
            format!("Sent request to {}", addr),
            "peer",
        ))
        .await;

    Ok(())
}

#[tauri::command]
pub async fn approve_pairing(peer_id: String, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    state
        .engine
        .approve_pairing(&peer_id)
        .await
        .map_err(|e| format!("Failed to approve pairing: {}", e))?;

    state
        .add_activity(ActivityItem::new(
            "Pairing Approved",
            format!("Device: {}", peer_id),
            "peer",
        ))
        .await;

    Ok(())
}

#[tauri::command]
pub async fn reject_pairing(peer_id: String, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    state
        .engine
        .reject_pairing(&peer_id)
        .await
        .map_err(|e| format!("Failed to reject pairing: {}", e))?;

    state
        .add_activity(ActivityItem::new(
            "Pairing Rejected",
            format!("Device: {}", peer_id),
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
