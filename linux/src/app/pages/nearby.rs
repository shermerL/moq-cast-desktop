//! Nearby peer discovery page.

use eframe::egui::{self, RichText};

use super::super::{
    AppSnapshot, DiscoveryState, Locale, MUTED, TEAL, UserCommand, heading, section_frame,
};

pub(in crate::app) fn show(
    ui: &mut egui::Ui,
    locale: Locale,
    snapshot: &AppSnapshot,
) -> Option<UserCommand> {
    heading(ui, locale.nearby(), locale.nearby_description());

    let discovery_active = snapshot.discovery.is_active();
    let mut command = if ui
        .button(if discovery_active {
            locale.stop_scan()
        } else {
            locale.start_scan()
        })
        .clicked()
    {
        Some(if discovery_active {
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
                if snapshot.discovery == DiscoveryState::Scanning {
                    ui.spinner();
                    ui.label(RichText::new(locale.scanning()).size(15.0));
                } else {
                    ui.label(RichText::new(locale.no_devices()).size(18.0).strong());
                }
                ui.label(
                    RichText::new(locale.no_devices_hint())
                        .size(13.0)
                        .color(MUTED),
                );
            });
            return;
        }

        for peer in &snapshot.peers {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(&peer.name).size(16.0).strong());
                    ui.label(
                        RichText::new(peer.endpoints.join(", "))
                            .size(12.0)
                            .color(MUTED),
                    );
                    if peer.fingerprint_pinned {
                        ui.label(
                            RichText::new(locale.fingerprint_pinning())
                                .size(12.0)
                                .color(TEAL),
                        );
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    match &snapshot.peer {
                        super::super::PeerState::Connecting { peer_id } if peer_id == &peer.id => {
                            ui.add_enabled(false, egui::Button::new(locale.connecting()));
                        }
                        super::super::PeerState::Connected { peer_id } if peer_id == &peer.id => {
                            if ui.button(locale.disconnect()).clicked() {
                                command = Some(UserCommand::Disconnect);
                            }
                        }
                        state => {
                            let available = matches!(
                                state,
                                super::super::PeerState::Disconnected
                                    | super::super::PeerState::Failed { .. }
                            );
                            if ui
                                .add_enabled(available, egui::Button::new(locale.connect()))
                                .clicked()
                            {
                                command = Some(UserCommand::ConnectPeer {
                                    peer_id: peer.id.clone(),
                                });
                            }
                        }
                    }
                });
            });
            ui.separator();
        }
    });

    command
}
