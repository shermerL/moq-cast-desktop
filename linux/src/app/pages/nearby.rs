//! Nearby peer discovery page.

use eframe::egui::{self, Color32, RichText};

use super::super::{AppSnapshot, DiscoveryState, Locale, UserCommand, heading, section_frame};

pub(in crate::app) fn show(
    ui: &mut egui::Ui,
    locale: Locale,
    snapshot: &AppSnapshot,
) -> Option<UserCommand> {
    heading(ui, locale.nearby(), locale.nearby_description());

    let scanning = snapshot.discovery == DiscoveryState::Scanning;
    let mut command = if ui
        .button(if scanning {
            locale.stop_scan()
        } else {
            locale.start_scan()
        })
        .clicked()
    {
        Some(if scanning {
            UserCommand::StopDiscovery
        } else {
            UserCommand::StartDiscovery
        })
    } else {
        None
    };

    ui.add_space(14.0);
    section_frame().show(ui, |ui| {
        ui.set_min_height(260.0);
        if snapshot.peers.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(64.0);
                if scanning {
                    ui.spinner();
                    ui.label(RichText::new(locale.scanning()).size(15.0));
                } else {
                    ui.label(RichText::new(locale.no_devices()).size(18.0).strong());
                }
                ui.label(
                    RichText::new(locale.no_devices_hint())
                        .size(13.0)
                        .color(Color32::from_rgb(99, 116, 112)),
                );
            });
            return;
        }

        for peer in &snapshot.peers {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(&peer.name).size(16.0).strong());
                    ui.label(
                        RichText::new(&peer.id)
                            .size(12.0)
                            .color(Color32::from_rgb(99, 116, 112)),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(locale.connect()).clicked() {
                        command = Some(UserCommand::ConnectPeer {
                            peer_id: peer.id.clone(),
                        });
                    }
                });
            });
            ui.separator();
        }
    });

    command
}
