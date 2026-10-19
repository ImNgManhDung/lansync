use egui::{vec2, RichText, Sense, Ui};
use tokio::sync::mpsc::UnboundedSender;

use crate::application::commands::AppCommand;
use crate::application::state::AppStateSnapshot;
use crate::domain::models::DeletePolicy;
use crate::presentation::theme::{
    card_frame, danger_button, hairline_divider, paint_nav_icon, primary_button, secondary_button,
    split_row, status_pill, NavIcon, Palette,
};

pub fn render_devices_tab(
    ui: &mut Ui,
    snap: &AppStateSnapshot,
    p: &Palette,
    _manual_ip_input: &mut String,
    confirming_unpair_peer: &mut Option<String>,
    tx_cmd: &UnboundedSender<AppCommand>,
) {
    card_frame(p).show(ui, |ui| {
        ui.set_width(ui.available_width());

        // Card Header Row
        split_row(
            ui,
            |ui| {
                ui.label(
                    RichText::new("Thiết bị trong mạng LAN")
                        .color(p.text_primary)
                        .strong()
                        .size(14.0),
                );
            },
            |ui| {
                if secondary_button(ui, p, "Quét lại").clicked() {
                    let _ = tx_cmd.send(AppCommand::RescanPeers);
                }
            },
        );

        if snap.peers.is_empty() {
            hairline_divider(ui, p);
            ui.add_space(8.0);
            ui.label(
                RichText::new(
                    "Chưa phát hiện thiết bị nào khác trên cùng mạng LAN. Nhập địa chỉ IP:Port ở thanh lệnh bên dưới để kết nối trực tiếp.",
                )
                .color(p.text_secondary)
                .size(12.5),
            );
            ui.add_space(4.0);
        } else {
            for peer in &snap.peers {
                hairline_divider(ui, p);

                let addr_text = peer.address.as_deref().unwrap_or("Chưa rõ IP");
                let is_online = peer.is_connected || peer.status == "connected";

                split_row(
                    ui,
                    |ui| {
                        let (icon_rect, _) =
                            ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                        paint_nav_icon(
                            ui,
                            icon_rect,
                            NavIcon::Devices,
                            if is_online {
                                p.online_dot
                            } else {
                                p.text_muted
                            },
                        );

                        ui.label(
                            RichText::new(&peer.name)
                                .color(p.text_primary)
                                .strong()
                                .size(13.5),
                        );

                        if is_online {
                            status_pill(ui, p.online_bg, p.online_dot, "Đã kết nối");
                        } else if peer.status == "connecting" {
                            status_pill(ui, p.secondary_bg, p.accent_blue, "Đang kết nối");
                        } else if peer.status == "backoff" {
                            status_pill(ui, p.warning_bg, p.warning_dot, "Thử lại");
                        } else if peer.is_paired {
                            status_pill(ui, p.secondary_bg, p.text_muted, "Ngoại tuyến");
                        } else {
                            status_pill(ui, p.secondary_bg, p.text_secondary, "Chưa ghép");
                        }

                        ui.monospace(
                            RichText::new(addr_text)
                                .color(p.text_muted)
                                .size(12.0),
                        );
                    },
                    |ui| {
                        if peer.is_paired {
                            let is_confirming =
                                confirming_unpair_peer.as_deref() == Some(&peer.device_id);
                            if is_confirming {
                                if secondary_button(ui, p, "Đóng").clicked() {
                                    *confirming_unpair_peer = None;
                                }
                                if danger_button(ui, p, "Xác nhận huỷ").clicked() {
                                    let _ = tx_cmd
                                        .send(AppCommand::UnpairDevice(peer.device_id.clone()));
                                    *confirming_unpair_peer = None;
                                }
                            } else {
                                if secondary_button(ui, p, "Huỷ ghép").clicked() {
                                    *confirming_unpair_peer = Some(peer.device_id.clone());
                                }

                                let current_label = match peer.delete_policy {
                                    None => "Xoá: Mặc định",
                                    Some(DeletePolicy::Never) => "Xoá: Không xoá",
                                    Some(DeletePolicy::Ask) => "Xoá: Hỏi trước",
                                    Some(DeletePolicy::MirrorTrash) => "Xoá: Thùng rác",
                                };
                                egui::ComboBox::from_id_salt(format!(
                                    "peer_del_{}",
                                    peer.device_id
                                ))
                                .selected_text(current_label)
                                .show_ui(ui, |ui| {
                                    if ui
                                        .selectable_label(
                                            peer.delete_policy.is_none(),
                                            "Theo cài đặt chung",
                                        )
                                        .clicked()
                                    {
                                        let _ = tx_cmd.send(AppCommand::SetPeerDeletePolicy {
                                            peer_id: peer.device_id.clone(),
                                            policy: None,
                                        });
                                    }
                                    if ui
                                        .selectable_label(
                                            peer.delete_policy == Some(DeletePolicy::Never),
                                            "Không xoá chéo",
                                        )
                                        .clicked()
                                    {
                                        let _ = tx_cmd.send(AppCommand::SetPeerDeletePolicy {
                                            peer_id: peer.device_id.clone(),
                                            policy: Some(DeletePolicy::Never),
                                        });
                                    }
                                    if ui
                                        .selectable_label(
                                            peer.delete_policy == Some(DeletePolicy::Ask),
                                            "Hỏi trước khi xoá",
                                        )
                                        .clicked()
                                    {
                                        let _ = tx_cmd.send(AppCommand::SetPeerDeletePolicy {
                                            peer_id: peer.device_id.clone(),
                                            policy: Some(DeletePolicy::Ask),
                                        });
                                    }
                                    if ui
                                        .selectable_label(
                                            peer.delete_policy == Some(DeletePolicy::MirrorTrash),
                                            "Chuyển vào Thùng rác",
                                        )
                                        .clicked()
                                    {
                                        let _ = tx_cmd.send(AppCommand::SetPeerDeletePolicy {
                                            peer_id: peer.device_id.clone(),
                                            policy: Some(DeletePolicy::MirrorTrash),
                                        });
                                    }
                                });
                            }
                        } else if let Some(ref addr) = peer.address {
                            if primary_button(ui, p, "Ghép đôi").clicked() {
                                let _ = tx_cmd.send(AppCommand::PairWithAddress(addr.clone()));
                            }
                        }
                    },
                );
            }
        }
    });
}
