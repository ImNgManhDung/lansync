use egui::{vec2, Rect, RichText, Sense, Ui};
use tokio::sync::mpsc::UnboundedSender;

use crate::application::commands::AppCommand;
use crate::application::state::AppStateSnapshot;
use crate::domain::models::{format_bytes, ConflictResolution};
use crate::presentation::theme::{
    card_frame, compact_button, danger_button, hairline_divider, paint_nav_icon, primary_button,
    secondary_button, split_row, status_pill, NavIcon, Palette,
};

pub fn render_alerts_section(
    ui: &mut Ui,
    snap: &AppStateSnapshot,
    p: &Palette,
    tx_cmd: &UnboundedSender<AppCommand>,
) {
    if !snap.conflicts.is_empty() {
        card_frame(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            split_row(
                ui,
                |ui| {
                    status_pill(
                        ui,
                        p.warning_bg,
                        p.warning_dot,
                        &format!("Xung đột file ({})", snap.conflicts.len()),
                    );
                    ui.label(
                        RichText::new("Hai thiết bị cùng sửa đổi file khi ngoại tuyến")
                            .color(p.text_secondary)
                            .size(12.5),
                    );
                },
                |_ui| {},
            );

            for c in &snap.conflicts {
                hairline_divider(ui, p);
                split_row(
                    ui,
                    |ui| {
                        ui.label(RichText::new(&c.original_path).color(p.text_primary).strong());
                        ui.label(
                            RichText::new(format!("{} · {}", format_bytes(c.size), c.modified_at))
                                .color(p.text_muted)
                                .size(12.0),
                        );
                    },
                    |ui| {
                        if secondary_button(ui, p, "Giữ cả hai").clicked() {
                            let _ = tx_cmd.send(AppCommand::ResolveConflict {
                                conflict_path: c.conflict_path.clone(),
                                resolution: ConflictResolution::KeepBoth,
                            });
                        }
                        if secondary_button(ui, p, "Giữ bản máy kia").clicked() {
                            let _ = tx_cmd.send(AppCommand::ResolveConflict {
                                conflict_path: c.conflict_path.clone(),
                                resolution: ConflictResolution::KeepTheirs,
                            });
                        }
                        if primary_button(ui, p, "Giữ bản máy này").clicked() {
                            let _ = tx_cmd.send(AppCommand::ResolveConflict {
                                conflict_path: c.conflict_path.clone(),
                                resolution: ConflictResolution::KeepMine,
                            });
                        }
                    },
                );
            }
        });
        ui.add_space(14.0);
    }

    if !snap.pending_deletions.is_empty() {
        card_frame(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            split_row(
                ui,
                |ui| {
                    status_pill(
                        ui,
                        p.danger_bg,
                        p.danger_dot,
                        &format!("Yêu cầu xoá ({})", snap.pending_deletions.len()),
                    );
                    ui.label(
                        RichText::new("Thiết bị ghép đôi đã xoá các file dưới đây")
                            .color(p.text_secondary)
                            .size(12.5),
                    );
                },
                |_ui| {},
            );

            for item in &snap.pending_deletions {
                hairline_divider(ui, p);
                let peer_label = snap
                    .peers
                    .iter()
                    .find(|d| d.device_id == item.peer_id)
                    .map(|d| d.name.as_str())
                    .unwrap_or("thiết bị ghép đôi");
                split_row(
                    ui,
                    |ui| {
                        ui.label(RichText::new(&item.path).color(p.text_primary).strong());
                        ui.label(
                            RichText::new(format!("từ {}", peer_label))
                                .color(p.text_muted)
                                .size(12.0),
                        );
                    },
                    |ui| {
                        if secondary_button(ui, p, "Giữ lại").clicked() {
                            let _ = tx_cmd.send(AppCommand::ResolvePendingDeletion {
                                path: item.path.clone(),
                                accept: false,
                            });
                        }
                        if danger_button(ui, p, "Xoá vào Thùng rác").clicked() {
                            let _ = tx_cmd.send(AppCommand::ResolvePendingDeletion {
                                path: item.path.clone(),
                                accept: true,
                            });
                        }
                    },
                );
            }
        });
        ui.add_space(14.0);
    }
}

pub fn render_sync_tab(
    ui: &mut Ui,
    snap: &AppStateSnapshot,
    p: &Palette,
    tx_cmd: &UnboundedSender<AppCommand>,
) {
    card_frame(p).show(ui, |ui| {
        ui.set_width(ui.available_width());

        // Row 1: Workspace Folder
        split_row(
            ui,
            |ui| {
                let (icon_rect, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                paint_nav_icon(ui, icon_rect, NavIcon::Folder, p.text_secondary);
                ui.label(
                    RichText::new("Thư mục đồng bộ")
                        .color(p.text_primary)
                        .strong()
                        .size(13.5),
                );
                ui.monospace(
                    RichText::new(&snap.sync_dir)
                        .color(p.text_secondary)
                        .size(12.5),
                );
            },
            |ui| {
                if secondary_button(ui, p, "Mở Finder").clicked() {
                    let _ = tx_cmd.send(AppCommand::OpenSyncFolder);
                }
                if secondary_button(ui, p, "Đổi thư mục").clicked() {
                    let _ = tx_cmd.send(AppCommand::PickSyncFolder);
                }
            },
        );

        hairline_divider(ui, p);

        // Row 2: Local Node Endpoint
        let full_lan_addr = format!(
            "{}:{}",
            snap.local_ip.as_deref().unwrap_or("127.0.0.1"),
            snap.listen_port
        );

        split_row(
            ui,
            |ui| {
                let (icon_rect, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                paint_nav_icon(
                    ui,
                    Rect::from_center_size(icon_rect.center(), vec2(16.0, 16.0)),
                    NavIcon::Devices,
                    p.text_secondary,
                );
                ui.label(
                    RichText::new(&snap.device_name)
                        .color(p.text_primary)
                        .strong()
                        .size(13.0),
                );
                ui.label(RichText::new("·").color(p.text_muted));
                ui.monospace(
                    RichText::new(&full_lan_addr)
                        .color(p.accent_blue)
                        .strong()
                        .size(12.5),
                );
            },
            |ui| {
                if compact_button(ui, p, "Sao chép IP").clicked() {
                    let _ = tx_cmd.send(AppCommand::CopyText(full_lan_addr.clone()));
                }
            },
        );
    });
}
