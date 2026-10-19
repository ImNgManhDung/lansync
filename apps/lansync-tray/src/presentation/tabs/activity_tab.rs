use egui::{vec2, RichText, Sense, Ui};

use crate::application::state::AppStateSnapshot;
use crate::presentation::theme::{hairline_divider, split_row, Palette};

pub fn render_activity_tab(ui: &mut Ui, snap: &AppStateSnapshot, p: &Palette) {
    if snap.activities.is_empty() {
        hairline_divider(ui, p);
        ui.add_space(6.0);
        ui.label(
            RichText::new("Chưa có sự kiện nào. Các hoạt động truyền tải file và kết nối sẽ hiển thị tại đây.")
                .color(p.text_secondary)
                .size(12.5),
        );
        ui.add_space(4.0);
        return;
    }

    for item in &snap.activities {
        hairline_divider(ui, p);

        let dot_color = match item.kind.as_str() {
            "error" => p.danger_dot,
            "conflict" => p.warning_dot,
            "file" => p.accent_blue,
            "peer" => p.online_dot,
            _ => p.text_muted,
        };

        split_row(
            ui,
            |ui| {
                // Antigravity timeline step dot
                let (rect, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                ui.painter().circle_filled(rect.center(), 3.5, dot_color);

                ui.label(
                    RichText::new(&item.title)
                        .color(p.text_primary)
                        .strong()
                        .size(13.0),
                );
                if !item.details.is_empty() {
                    ui.label(
                        RichText::new(&item.details)
                            .color(p.text_secondary)
                            .size(12.5),
                    );
                }
            },
            |ui| {
                ui.monospace(
                    RichText::new(&item.timestamp)
                        .color(p.text_muted)
                        .size(11.5),
                );
            },
        );
    }
}
