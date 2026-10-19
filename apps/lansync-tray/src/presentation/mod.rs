pub mod tabs;
pub mod theme;

use egui::{vec2, Context, CornerRadius, Frame, Margin, RichText, Stroke, ViewportCommand};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;

use crate::application::commands::AppCommand;
use crate::application::state::AppStore;
use crate::domain::models::format_bytes;
use crate::infrastructure::tray_adapter::SystemTrayAdapter;
use crate::presentation::tabs::{
    activity_tab::render_activity_tab,
    devices_tab::render_devices_tab,
    settings_tab::render_settings_tab,
    sync_tab::{render_alerts_section, render_sync_tab},
    versions_tab::render_versions_tab,
};
use crate::presentation::theme::{
    apply_theme, card_frame, init_fonts, primary_button, prompt_bar_frame, secondary_button,
    sidebar_nav_item, split_row, status_pill, NavIcon, Palette, ThemeMode,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceView {
    Overview,
    Devices,
    Activity,
    Versions,
    Settings,
}

pub struct LanSyncGuiApp {
    store: AppStore,
    tx_cmd: UnboundedSender<AppCommand>,
    tray: Option<SystemTrayAdapter>,
    logo_tex: Option<egui::TextureHandle>,
    active_view: WorkspaceView,
    theme_mode: ThemeMode,
    manual_ip_input: String,
    confirming_unpair_peer: Option<String>,
    repaint_signal: Arc<AtomicBool>,
    show_window_flag: Arc<AtomicBool>,
    quit_flag: Arc<AtomicBool>,
}

impl LanSyncGuiApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        store: AppStore,
        tx_cmd: UnboundedSender<AppCommand>,
        tray: Option<SystemTrayAdapter>,
        repaint_signal: Arc<AtomicBool>,
        show_window_flag: Arc<AtomicBool>,
        quit_flag: Arc<AtomicBool>,
    ) -> Self {
        init_fonts(&cc.egui_ctx);
        // Default to Antigravity / ChatGPT / Claude Code Obsidian Dark Mode
        let default_mode = ThemeMode::Dark;
        apply_theme(&cc.egui_ctx, default_mode);

        let logo_tex = image::load_from_memory(include_bytes!("../../icons/128x128.png"))
            .ok()
            .map(|img| {
                let mut rgba = img.into_rgba8();
                let (width, height) = rgba.dimensions();
                let w = width as f32;
                let h = height as f32;
                let radius = w.min(h) * 0.24;
                for y in 0..height {
                    for x in 0..width {
                        let px = x as f32 + 0.5;
                        let py = y as f32 + 0.5;
                        let qx = (px - w * 0.5).abs() - (w * 0.5 - radius);
                        let qy = (py - h * 0.5).abs() - (h * 0.5 - radius);
                        let dist = qx.max(0.0).hypot(qy.max(0.0)) + qx.min(qy).min(0.0) - radius;
                        let coverage = (0.5 - dist).clamp(0.0, 1.0);
                        let pixel = rgba.get_pixel_mut(x, y);
                        pixel[3] = ((pixel[3] as f32) * coverage).round() as u8;
                    }
                }
                let size = [width as usize, height as usize];
                let color_image =
                    egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_flat_samples().as_slice());
                cc.egui_ctx
                    .load_texture("app_logo_icon", color_image, egui::TextureOptions::LINEAR)
            });

        Self {
            store,
            tx_cmd,
            tray,
            logo_tex,
            active_view: WorkspaceView::Overview,
            theme_mode: default_mode,
            manual_ip_input: String::new(),
            confirming_unpair_peer: None,
            repaint_signal,
            show_window_flag,
            quit_flag,
        }
    }
}

impl eframe::App for LanSyncGuiApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        if let Some(ref tray) = self.tray {
            tray.poll_events();
        }

        if self.quit_flag.load(Ordering::SeqCst) {
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return;
        }

        if self.show_window_flag.swap(false, Ordering::SeqCst) {
            ctx.send_viewport_cmd(ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(ViewportCommand::Focus);
        }

        if ctx.input(|i| i.viewport().close_requested()) && !self.quit_flag.load(Ordering::SeqCst) {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        }

        if self.repaint_signal.swap(false, Ordering::SeqCst) {
            ctx.request_repaint();
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(500));

        let snap = self.store.snapshot();
        let p = Palette::for_mode(self.theme_mode);

        // 1. ANTIGRAVITY / CLAUDE / CHATGPT MINIMALIST LEFT SIDEBAR
        egui::SidePanel::left("antigravity_sidebar")
            .resizable(false)
            .exact_width(216.0)
            .frame(
                Frame::new()
                    .fill(p.bg_sidebar)
                    .inner_margin(Margin::symmetric(12, 16))
                    .stroke(Stroke::new(1.0_f32, p.border_subtle)),
            )
            .show(ctx, |ui| {
                // Brand Header Row with Rounded Logo Icon
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    if let Some(ref tex) = self.logo_tex {
                        ui.add(
                            egui::Image::from_texture((tex.id(), vec2(24.0, 24.0)))
                                .corner_radius(CornerRadius::same(6)),
                        );
                    }

                    ui.label(
                        RichText::new("LanSync")
                            .color(p.text_primary)
                            .strong()
                            .size(15.5),
                    );
                });

                ui.add_space(16.0);

                let nav_items = [
                    (
                        WorkspaceView::Overview,
                        NavIcon::Overview,
                        "Tổng quan",
                        None,
                    ),
                    (
                        WorkspaceView::Devices,
                        NavIcon::Devices,
                        "Thiết bị mạng LAN",
                        if snap.peers.is_empty() {
                            None
                        } else {
                            Some(snap.peers.len())
                        },
                    ),
                    (
                        WorkspaceView::Activity,
                        NavIcon::Activity,
                        "Luồng hoạt động",
                        if snap.activities.is_empty() {
                            None
                        } else {
                            Some(snap.activities.len())
                        },
                    ),
                    (
                        WorkspaceView::Versions,
                        NavIcon::Versions,
                        "Bản lưu dự phòng",
                        if snap.versioned_files.is_empty() {
                            None
                        } else {
                            Some(snap.versioned_files.len())
                        },
                    ),
                    (
                        WorkspaceView::Settings,
                        NavIcon::Settings,
                        "Cấu hình hệ thống",
                        None,
                    ),
                ];

                for (view, icon, label, badge) in nav_items {
                    if sidebar_nav_item(ui, &p, icon, label, badge, self.active_view == view) {
                        self.active_view = view;
                    }
                    ui.add_space(2.0);
                }

                // Bottom Sidebar Actions (Folder & Theme Switcher)
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    let theme_label = match self.theme_mode {
                        ThemeMode::Dark => "Giao diện: Obsidian (Tối)",
                        ThemeMode::Light => "Giao diện: Paper (Sáng)",
                    };
                    if sidebar_nav_item(ui, &p, NavIcon::SunMoon, theme_label, None, false) {
                        self.theme_mode = match self.theme_mode {
                            ThemeMode::Dark => ThemeMode::Light,
                            ThemeMode::Light => ThemeMode::Dark,
                        };
                        apply_theme(ctx, self.theme_mode);
                    }

                    ui.add_space(2.0);

                    if sidebar_nav_item(ui, &p, NavIcon::Folder, "Mở thư mục đồng bộ", None, false)
                    {
                        let _ = self.tx_cmd.send(AppCommand::OpenSyncFolder);
                    }
                });
            });

        // 2. BOTTOM FLOATING COMMAND / QUICK-CONNECT BAR (ChatGPT / Antigravity Prompt Box Style)
        egui::TopBottomPanel::bottom("antigravity_prompt_bar")
            .frame(
                Frame::new()
                    .fill(p.bg_canvas)
                    .inner_margin(Margin::symmetric(24, 14))
                    .stroke(Stroke::NONE),
            )
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.set_max_width(720.0);
                    prompt_bar_frame(&p).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let has_ip_text = !self.manual_ip_input.trim().is_empty();
                        let mut submit_enter = false;
                        let mut submit_btn = false;
                        split_row(
                            ui,
                            |ui| {
                                ui.monospace(
                                    RichText::new(">")
                                        .color(p.text_muted)
                                        .strong()
                                        .size(14.0),
                                );
                                let edit = egui::TextEdit::singleline(&mut self.manual_ip_input)
                                    .frame(false)
                                    .hint_text(
                                        "Nhập IP:Port để kết nối thiết bị (vd: 192.168.1.50:42424)...",
                                    )
                                    .desired_width(360.0);
                                let resp = ui.add(edit);
                                if resp.lost_focus()
                                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                                {
                                    submit_enter = true;
                                }
                            },
                            |ui| {
                                if has_ip_text {
                                    if primary_button(ui, &p, "Kết nối IP").clicked() {
                                        submit_btn = true;
                                    }
                                } else {
                                    let pause_label = if snap.is_paused {
                                        "Tiếp tục"
                                    } else {
                                        "Tạm dừng"
                                    };
                                    if secondary_button(ui, &p, pause_label).clicked() {
                                        let _ =
                                            self.tx_cmd.send(AppCommand::SetPaused(!snap.is_paused));
                                    }
                                    if primary_button(ui, &p, "Đồng bộ ngay").clicked() {
                                        let _ = self.tx_cmd.send(AppCommand::TriggerManualSync);
                                    }
                                }
                            },
                        );

                        if (submit_enter || submit_btn) && !self.manual_ip_input.trim().is_empty() {
                            let addr = self.manual_ip_input.trim().to_string();
                            let _ = self.tx_cmd.send(AppCommand::PairWithAddress(addr));
                            self.manual_ip_input.clear();
                        }
                    });
                });
            });

        // 3. MAIN CANVAS WITH CENTERED 720PX COLUMN (Antigravity / Claude / ChatGPT Layout)
        egui::CentralPanel::default()
            .frame(Frame::new().fill(p.bg_canvas).inner_margin(Margin::symmetric(24, 20)))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.vertical_centered(|ui| {
                            ui.set_max_width(720.0);

                            // Top Context Header inside the 720px column
                            split_row(
                                ui,
                                |ui| {
                                    let title = match self.active_view {
                                        WorkspaceView::Overview => "Tổng quan đồng bộ",
                                        WorkspaceView::Devices => "Thiết bị mạng LAN",
                                        WorkspaceView::Activity => "Luồng hoạt động",
                                        WorkspaceView::Versions => "Khôi phục phiên bản",
                                        WorkspaceView::Settings => "Cấu hình hệ thống",
                                    };
                                    ui.label(
                                        RichText::new(title)
                                            .color(p.text_primary)
                                            .strong()
                                            .size(18.0),
                                    );
                                },
                                |ui| {
                                    ui.monospace(
                                        RichText::new(format!(
                                            "{} · :{}",
                                            snap.device_name, snap.listen_port
                                        ))
                                        .color(p.text_muted)
                                        .size(12.0),
                                    );
                                },
                            );

                            ui.add_space(16.0);

                            // Global 6-digit Pairing Verification Card (if pending)
                            if let Some(ref pairing) = snap.pending_pairing {
                                Frame::new()
                                    .fill(p.warning_bg)
                                    .stroke(Stroke::new(1.0_f32, p.warning_dot))
                                    .corner_radius(CornerRadius::same(12))
                                    .inner_margin(Margin::symmetric(18, 14))
                                    .show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        split_row(
                                            ui,
                                            |ui| {
                                                ui.label(
                                                    RichText::new(format!(
                                                        "Ghép đôi với {} — Mã xác thực:",
                                                        pairing.device_name
                                                    ))
                                                    .color(p.warning_dot)
                                                    .strong()
                                                    .size(13.5),
                                                );
                                                ui.monospace(
                                                    RichText::new(&pairing.code)
                                                        .color(p.text_primary)
                                                        .strong()
                                                        .size(16.0),
                                                );
                                            },
                                            |ui| {
                                                if secondary_button(ui, &p, "Từ chối").clicked()
                                                {
                                                    let _ = self.tx_cmd.send(
                                                        AppCommand::RejectPairing(Some(
                                                            pairing.peer_id.clone(),
                                                        )),
                                                    );
                                                }
                                                if primary_button(ui, &p, "Chấp nhận").clicked()
                                                {
                                                    let _ = self.tx_cmd.send(
                                                        AppCommand::ApprovePairing(Some(
                                                            pairing.peer_id.clone(),
                                                        )),
                                                    );
                                                }
                                            },
                                        );
                                    });
                                ui.add_space(14.0);
                            }

                            // Conflict & Deletion Alerts
                            render_alerts_section(ui, &snap, &p, &self.tx_cmd);

                            // Active Download Progress Card
                            if let Some((ref filename, size)) = snap.active_download {
                                card_frame(&p).show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    split_row(
                                        ui,
                                        |ui| {
                                            status_pill(
                                                ui,
                                                p.secondary_bg,
                                                p.accent_blue,
                                                "Đang tải",
                                            );
                                            ui.label(
                                                RichText::new(filename)
                                                    .color(p.text_primary)
                                                    .strong()
                                                    .size(13.5),
                                            );
                                        },
                                        |ui| {
                                            ui.monospace(
                                                RichText::new(format_bytes(size))
                                                    .color(p.text_secondary)
                                                    .size(12.0),
                                            );
                                        },
                                    );
                                    ui.add_space(8.0);
                                    ui.add(egui::ProgressBar::new(0.55).animate(true));
                                });
                                ui.add_space(14.0);
                            }

                            match self.active_view {
                                WorkspaceView::Overview => {
                                    // 1. Folder & Local Node Identity Card
                                    render_sync_tab(ui, &snap, &p, &self.tx_cmd);
                                    ui.add_space(14.0);

                                    // 2. LAN Devices Card
                                    render_devices_tab(
                                        ui,
                                        &snap,
                                        &p,
                                        &mut self.manual_ip_input,
                                        &mut self.confirming_unpair_peer,
                                        &self.tx_cmd,
                                    );
                                    ui.add_space(14.0);

                                    // 3. Recent Activity Stream Card (Antigravity Timeline Style)
                                    card_frame(&p).show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        split_row(
                                            ui,
                                            |ui| {
                                                ui.label(
                                                    RichText::new("Luồng hoạt động gần đây")
                                                        .color(p.text_primary)
                                                        .strong()
                                                        .size(14.0),
                                                );
                                            },
                                            |ui| {
                                                if !snap.versioned_files.is_empty()
                                                    && secondary_button(
                                                        ui,
                                                        &p,
                                                        &format!(
                                                            "Bản lưu dự phòng ({})",
                                                            snap.versioned_files.len()
                                                        ),
                                                    )
                                                    .clicked()
                                                {
                                                    self.active_view = WorkspaceView::Versions;
                                                }
                                            },
                                        );
                                        render_activity_tab(ui, &snap, &p);
                                    });
                                }
                                WorkspaceView::Devices => {
                                    render_devices_tab(
                                        ui,
                                        &snap,
                                        &p,
                                        &mut self.manual_ip_input,
                                        &mut self.confirming_unpair_peer,
                                        &self.tx_cmd,
                                    );
                                }
                                WorkspaceView::Activity => {
                                    card_frame(&p).show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        ui.label(
                                            RichText::new("Nhật ký đồng bộ trực tiếp")
                                                .color(p.text_primary)
                                                .strong()
                                                .size(14.0),
                                        );
                                        render_activity_tab(ui, &snap, &p);
                                    });
                                }
                                WorkspaceView::Versions => {
                                    card_frame(&p).show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        ui.label(
                                            RichText::new("Khôi phục phiên bản file (.lansync-versions)")
                                                .color(p.text_primary)
                                                .strong()
                                                .size(14.0),
                                        );
                                        render_versions_tab(ui, &snap, &p, &self.tx_cmd);
                                    });
                                }
                                WorkspaceView::Settings => {
                                    render_settings_tab(ui, &snap, &p, &self.tx_cmd);
                                }
                            }
                        });
                    });
            });
    }
}
