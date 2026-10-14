use std::sync::Arc;
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, PhysicalPosition};
use tracing::info;

use crate::state::AppState;

pub fn setup_tray(app: &AppHandle, state: Arc<AppState>) -> anyhow::Result<()> {
    let open_folder_item = MenuItemBuilder::with_id("open_folder", "Open LanSync Folder").build(app)?;
    let toggle_window_item = MenuItemBuilder::with_id("toggle_window", "Show / Hide Control Panel").build(app)?;
    let quit_item = MenuItemBuilder::with_id("quit", "Quit LanSync").build(app)?;

    let menu = MenuBuilder::new(app)
        .item(&open_folder_item)
        .item(&toggle_window_item)
        .separator()
        .item(&quit_item)
        .build()?;

    let mut builder = TrayIconBuilder::with_id("main-tray")
        .tooltip("LanSync - Local Network Sync")
        .menu(&menu)
        .show_menu_on_left_click(false);

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }

    let state_for_menu = state.clone();
    builder
        .on_menu_event(move |app_handle, event| match event.id().as_ref() {
            "open_folder" => {
                let _ = open::that(&state_for_menu.sync_dir);
            }
            "toggle_window" => {
                toggle_popover(app_handle);
            }
            "quit" => {
                app_handle.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                toggle_popover(app);
            }
        })
        .build(app)?;

    info!("System Tray initialized successfully.");
    Ok(())
}

pub fn toggle_popover(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let is_visible = window.is_visible().unwrap_or(false);
        if is_visible {
            let _ = window.hide();
        } else {
            // Position the window near the tray
            if let Ok(Some(monitor)) = window.current_monitor() {
                let size = window.outer_size().unwrap_or_default();
                let screen_size = monitor.size();
                #[cfg(target_os = "macos")]
                {
                    // macOS: menu bar at the top, position at top-right
                    let x = (screen_size.width as i32) - (size.width as i32) - 30;
                    let y = 35;
                    let _ = window.set_position(tauri::Position::Physical(PhysicalPosition::new(x, y)));
                }
                #[cfg(target_os = "windows")]
                {
                    // Windows: taskbar usually at bottom-right
                    let x = (screen_size.width as i32) - (size.width as i32) - 20;
                    let y = (screen_size.height as i32) - (size.height as i32) - 60;
                    let _ = window.set_position(tauri::Position::Physical(PhysicalPosition::new(x, y)));
                }
            }
            let _ = window.show();
            let _ = window.set_focus();
        }
    }
}
