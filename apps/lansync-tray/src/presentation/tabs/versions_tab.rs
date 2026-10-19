use egui::{RichText, Ui};
use tokio::sync::mpsc::UnboundedSender;

use crate::application::commands::AppCommand;
use crate::application::state::AppStateSnapshot;
use crate::domain::models::format_bytes;
use crate::presentation::theme::{
    hairline_divider, primary_button, secondary_button, split_row, Palette,
};

pub fn render_versions_tab(
    ui: &mut Ui,
    snap: &AppStateSnapshot,
    p: &Palette,
    tx_cmd: &UnboundedSender<AppCommand>,
) {
    if snap.versioned_files.is_empty() {
        hairline_divider(ui, p);
        ui.add_space(6.0);
        ui.label(
            RichText::new(
                "Chưa có phiên bản lưu trữ nào. Khi một file bị ghi đè bởi thiết bị khác, bản gốc sẽ tự động lưu tại đây.",
            )
            .color(p.text_secondary)
            .size(12.5),
        );
        ui.add_space(4.0);
        return;
    }

    for rel_path in &snap.versioned_files {
        hairline_divider(ui, p);
        let is_selected = snap.selected_version_file.as_deref() == Some(rel_path);

        split_row(
            ui,
            |ui| {
                ui.monospace(
                    RichText::new(rel_path)
                        .color(p.text_primary)
                        .strong()
                        .size(13.0),
                );
            },
            |ui| {
                let label = if is_selected {
                    "Ẩn bản lưu"
                } else {
                    "Xem bản lưu"
                };
                if secondary_button(ui, p, label).clicked() {
                    let next = if is_selected {
                        None
                    } else {
                        Some(rel_path.clone())
                    };
                    let _ = tx_cmd.send(AppCommand::SelectVersionFile(next));
                }
            },
        );

        if is_selected {
            for v in &snap.selected_file_versions {
                ui.add_space(4.0);
                split_row(
                    ui,
                    |ui| {
                        ui.add_space(16.0);
                        ui.label(
                            RichText::new(format!(
                                "↳ Bản lưu lúc {} ({})",
                                v.formatted_time,
                                format_bytes(v.size)
                            ))
                            .color(p.text_secondary)
                            .size(12.0),
                        );
                    },
                    |ui| {
                        if primary_button(ui, p, "Khôi phục").clicked() {
                            let _ = tx_cmd.send(AppCommand::RestoreFileVersion {
                                rel_path: v.rel_path.clone(),
                                timestamp_micros: v.timestamp_micros,
                            });
                        }
                    },
                );
            }
        }
    }
}
