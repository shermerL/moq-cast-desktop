//! Nearby peer discovery page.

use eframe::egui::{self, RichText};

use super::super::{
    AppSnapshot, DiscoveryState, Locale, MUTED, MediaState, PeerDiscoveryState, ScreenAvailability,
    TEAL, TransportState, UserCommand, heading, section_frame,
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

    if snapshot.inbound_session_count > 0 {
        ui.label(
            RichText::new(format!(
                "{}: {}",
                locale.inbound_sessions(),
                snapshot.inbound_session_count
            ))
            .size(12.0)
            .color(TEAL),
        );
    }

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

        for (peer_id, peer) in &snapshot.peers {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(&peer.name).size(16.0).strong());
                    if !peer.endpoints.is_empty() {
                        ui.label(
                            RichText::new(peer.endpoints.join(", "))
                                .size(12.0)
                                .color(MUTED),
                        );
                    }
                    let discovery = match peer.discovery {
                        PeerDiscoveryState::Found => locale.discovery_found(),
                        PeerDiscoveryState::Lost => locale.discovery_lost(),
                    };
                    let transport = match peer.transport {
                        TransportState::Waiting => locale.transport_waiting(),
                        TransportState::Connecting => locale.transport_connecting(),
                        TransportState::Connected => locale.transport_connected(),
                        TransportState::Failed => locale.transport_failed(),
                    };
                    let screen = if peer.screen == ScreenAvailability::Available {
                        locale.screen_available()
                    } else {
                        locale.screen_unavailable()
                    };
                    ui.label(
                        RichText::new(format!("{discovery} · {transport} · {screen}"))
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
                    let enabled = peer.screen == ScreenAvailability::Available
                        && snapshot.has_mesh_session()
                        && snapshot.media == MediaState::Idle;
                    if ui
                        .add_enabled(enabled, egui::Button::new(locale.watch()))
                        .clicked()
                    {
                        command = Some(UserCommand::StartWatching {
                            path: crate::screen_path::for_peer(peer_id),
                        });
                    }
                });
            });
            ui.separator();
        }
    });

    command
}
