// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{info, warn};

use lansync_core::config::Config;
use lansync_core::engine::SyncEngine;

use lansync_tray::application::state::AppStore;
use lansync_tray::application::use_cases::AppUseCaseInteractor;
use lansync_tray::infrastructure::core_adapter::LanSyncCoreAdapter;
use lansync_tray::infrastructure::os_adapter::OsSystemAdapter;
use lansync_tray::infrastructure::tray_adapter::SystemTrayAdapter;
use lansync_tray::presentation::LanSyncGuiApp;

fn load_window_icon() -> Option<Arc<egui::IconData>> {
    let png_bytes = include_bytes!("../icons/128x128.png");
    let mut img = image::load_from_memory(png_bytes).ok()?.into_rgba8();
    let (width, height) = img.dimensions();
    let w = width as f32;
    let h = height as f32;
    let radius = w.min(h) * 0.225;

    for y in 0..height {
        for x in 0..width {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let qx = (px - w * 0.5).abs() - (w * 0.5 - radius);
            let qy = (py - h * 0.5).abs() - (h * 0.5 - radius);
            let dist = qx.max(0.0).hypot(qy.max(0.0)) + qx.min(qy).min(0.0) - radius;
            let coverage = (0.5 - dist).clamp(0.0, 1.0);
            let pixel = img.get_pixel_mut(x, y);
            pixel[3] = ((pixel[3] as f32) * coverage).round() as u8;
        }
    }

    Some(Arc::new(egui::IconData {
        rgba: img.into_raw(),
        width,
        height,
    }))
}

fn main() -> eframe::Result<()> {
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);

    let runtime = tokio::runtime::Runtime::new().expect("Failed to initialize tokio runtime");
    let _rt_guard = runtime.enter();

    // Determine data directory and config path
    let data_dir = Config::default_data_dir();
    let config_path = data_dir.join("config.json");
    let default_sync_dir = directories::UserDirs::new()
        .and_then(|u| u.document_dir().map(|d| d.join("LanSync")))
        .unwrap_or_else(|| PathBuf::from("./LanSync"));

    let config = Config::load_or_default(&config_path, default_sync_dir.clone())
        .unwrap_or_else(|_| Config::new(default_sync_dir.clone(), 42424, "LanSync Device".into()));

    let sync_dir = config.sync_dir.clone();
    let device_name = config.device_name.clone();

    // Start Core SyncEngine (fallback to an OS-assigned free port if 42424 is already in use)
    let engine = runtime
        .block_on(async {
            match SyncEngine::start(config.clone(), data_dir.clone()).await {
                Ok(eng) => Ok(eng),
                Err(e) => {
                    warn!("Port {} busy ({}), falling back to dynamic port...", config.listen_port, e);
                    let mut fallback_cfg = config;
                    fallback_cfg.listen_port = 0;
                    SyncEngine::start(fallback_cfg, data_dir.clone()).await
                }
            }
        })
        .expect("Failed to start LanSync Core Engine");

    let listen_port = engine.local_addr().port();

    // Clean Architecture Composition Root:
    // 1. Infrastructure Adapters
    let backend_adapter = Arc::new(LanSyncCoreAdapter::new(
        engine.clone(),
        sync_dir.clone(),
        device_name,
        listen_port,
    ));
    let os_adapter = Arc::new(OsSystemAdapter::new());

    // 2. Application Store & UseCase Interactor
    let store = AppStore::default();
    let repaint_signal = Arc::new(AtomicBool::new(false));
    let show_window_flag = Arc::new(AtomicBool::new(false));
    let quit_flag = Arc::new(AtomicBool::new(false));

    let interactor = AppUseCaseInteractor::new(
        backend_adapter,
        os_adapter,
        store.clone(),
        show_window_flag.clone(),
    );

    let (tx_cmd, rx_cmd) = mpsc::unbounded_channel();
    interactor.spawn_worker(rx_cmd, repaint_signal.clone());

    // 3. Native System Tray Adapter
    let tray_adapter = match SystemTrayAdapter::new(
        sync_dir,
        show_window_flag.clone(),
        quit_flag.clone(),
    ) {
        Ok(t) => Some(t),
        Err(e) => {
            warn!("Could not initialize System Tray icon: {}", e);
            None
        }
    };

    // 4. Presentation Layer (egui / eframe Desktop Window)
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("LanSync - Đồng bộ Mạng LAN")
        .with_inner_size([980.0, 640.0])
        .with_min_inner_size([820.0, 520.0]);

    if let Some(icon) = load_window_icon() {
        viewport = viewport.with_icon(icon);
    }

    let native_options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    info!("Starting LanSync Native GUI (egui/eframe)...");
    let res = eframe::run_native(
        "LanSync",
        native_options,
        Box::new(move |cc| {
            Ok(Box::new(LanSyncGuiApp::new(
                cc,
                store,
                tx_cmd,
                tray_adapter,
                repaint_signal,
                show_window_flag,
                quit_flag,
            )))
        }),
    );

    let _ = runtime.block_on(async { engine.shutdown().await });
    res
}
