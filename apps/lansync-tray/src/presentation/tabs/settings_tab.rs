use egui::{RichText, Ui};
use tokio::sync::mpsc::UnboundedSender;

use crate::application::commands::AppCommand;
use crate::application::state::AppStateSnapshot;
use crate::domain::models::DeletePolicy;
use crate::presentation::theme::{
    card_frame, hairline_divider, secondary_button, split_row, status_pill, Palette,
};

pub fn render_settings_tab(
    ui: &mut Ui,
    snap: &AppStateSnapshot,
    p: &Palette,
    tx_cmd: &UnboundedSender<AppCommand>,
) {
    card_frame(p).show(ui, |ui| {
        ui.set_width(ui.available_width());

        // 1. Sync Folder
        split_row(
            ui,
            |ui| {
                ui.label(
                    RichText::new("Thư mục đồng bộ")
                        .color(p.text_primary)
                        .strong()
                        .size(13.5),
                );
                ui.monospace(
                    RichText::new(&snap.sync_dir)
                        .color(p.text_secondary)
                        .size(12.0),
                );
            },
            |ui| {
                if secondary_button(ui, p, "Chọn lại...").clicked() {
                    let _ = tx_cmd.send(AppCommand::PickSyncFolder);
                }
            },
        );

        hairline_divider(ui, p);

        // 2. Delete Policy
        split_row(
            ui,
            |ui| {
                ui.label(
                    RichText::new("Chế độ xử lý xoá file")
                        .color(p.text_primary)
                        .strong()
                        .size(13.5),
                );
                ui.label(
                    RichText::new("· Quy tắc khi thiết bị khác xoá file")
                        .color(p.text_muted)
                        .size(12.0),
                );
            },
            |ui| {
                egui::ComboBox::from_id_salt("settings_delete_policy")
                    .selected_text(snap.delete_policy.label_vi())
                    .show_ui(ui, |ui| {
                        for policy in [
                            DeletePolicy::Never,
                            DeletePolicy::Ask,
                            DeletePolicy::MirrorTrash,
                        ] {
                            if ui
                                .selectable_label(snap.delete_policy == policy, policy.label_vi())
                                .clicked()
                            {
                                let _ = tx_cmd.send(AppCommand::SetDeletePolicy(policy));
                            }
                        }
                    });
            },
        );

        hairline_divider(ui, p);

        // 3. Bandwidth Limit
        let options: &[(u32, &str)] = &[
            (0, "Không giới hạn"),
            (1024, "1 MB/s"),
            (5120, "5 MB/s"),
            (10240, "10 MB/s"),
            (20480, "20 MB/s"),
        ];
        let current_text = options
            .iter()
            .find(|(k, _)| *k == snap.bandwidth_limit_kbps)
            .map(|(_, label)| *label)
            .unwrap_or("Không giới hạn");

        split_row(
            ui,
            |ui| {
                ui.label(
                    RichText::new("Giới hạn tốc độ mạng LAN")
                        .color(p.text_primary)
                        .strong()
                        .size(13.5),
                );
                ui.label(
                    RichText::new("· Kiểm soát băng thông truyền tải tối đa")
                        .color(p.text_muted)
                        .size(12.0),
                );
            },
            |ui| {
                egui::ComboBox::from_id_salt("settings_bandwidth_limit")
                    .selected_text(current_text)
                    .show_ui(ui, |ui| {
                        for &(kbps, label) in options {
                            if ui
                                .selectable_label(snap.bandwidth_limit_kbps == kbps, label)
                                .clicked()
                            {
                                let _ = tx_cmd.send(AppCommand::SetBandwidthLimit(kbps));
                            }
                        }
                    });
            },
        );

        hairline_divider(ui, p);

        // 4. Security Architecture
        split_row(
            ui,
            |ui| {
                ui.label(
                    RichText::new("Bảo mật kênh truyền")
                        .color(p.text_primary)
                        .strong()
                        .size(13.5),
                );
                ui.label(
                    RichText::new("· Mutual TLS 1.3 · Ed25519 · BLAKE3")
                        .color(p.text_muted)
                        .size(12.0),
                );
            },
            |ui| {
                status_pill(ui, p.online_bg, p.online_dot, "Đang bật");
            },
        );
    });
}
