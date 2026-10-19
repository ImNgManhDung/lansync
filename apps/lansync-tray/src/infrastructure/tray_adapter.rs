use anyhow::Result;
use muda::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

pub struct SystemTrayAdapter {
    _tray_icon: TrayIcon,
    open_folder_id: muda::MenuId,
    show_window_id: muda::MenuId,
    quit_id: muda::MenuId,
    sync_dir: PathBuf,
    show_window_flag: Arc<AtomicBool>,
    quit_flag: Arc<AtomicBool>,
}

impl SystemTrayAdapter {
    pub fn new(
        sync_dir: PathBuf,
        show_window_flag: Arc<AtomicBool>,
        quit_flag: Arc<AtomicBool>,
    ) -> Result<Self> {
        let open_folder_item = MenuItem::new("Mở thư mục LanSync", true, None);
        let show_window_item = MenuItem::new("Hiện / Ẩn bảng điều khiển", true, None);
        let quit_item = MenuItem::new("Thoát LanSync", true, None);

        let open_folder_id = open_folder_item.id().clone();
        let show_window_id = show_window_item.id().clone();
        let quit_id = quit_item.id().clone();

        let menu = Menu::new();
        menu.append(&open_folder_item)?;
        menu.append(&show_window_item)?;
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&quit_item)?;

        let icon = load_tray_icon()?;

        let tray_icon = TrayIconBuilder::new()
            .with_tooltip("LanSync - Đồng bộ mạng LAN")
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_icon(icon)
            .build()?;

        Ok(Self {
            _tray_icon: tray_icon,
            open_folder_id,
            show_window_id,
            quit_id,
            sync_dir,
            show_window_flag,
            quit_flag,
        })
    }

    pub fn poll_events(&self) {
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == self.open_folder_id {
                let _ = open::that(&self.sync_dir);
            } else if event.id == self.show_window_id {
                self.show_window_flag.store(true, Ordering::SeqCst);
            } else if event.id == self.quit_id {
                self.quit_flag.store(true, Ordering::SeqCst);
            }
        }

        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                self.show_window_flag.store(true, Ordering::SeqCst);
            }
        }
    }
}

fn load_tray_icon() -> Result<Icon> {
    let png_bytes = include_bytes!("../../icons/32x32.png");
    let img = image::load_from_memory(png_bytes)?.into_rgba8();
    let (width, height) = img.dimensions();
    let rgba = img.into_raw();
    Ok(Icon::from_rgba(rgba, width, height)?)
}
