//! Nearby peer discovery page.

use eframe::egui::{self, RichText};

use super::super::components::{self, BadgeTone, primary_button, secondary_button, status_badge};
use super::super::theme::{MUTED, TEXT};
use super::super::{
    AppSnapshot, DialRole, DiscoveryState, Locale, MediaState, PeerDiscoveryState,
    ScreenAvailability, TransportState, UserCommand,
};

pub(in crate::app) fn show(
    ui: &mut egui::Ui,
    locale: Locale,
    snapshot: &AppSnapshot,
) -> Option<UserCommand> {
    let discovery_active = snapshot.discovery.is_active();
    let mut command = None;

    ui.horizontal_wrapped(|ui| {
        let (status, tone) = match snapshot.discovery {
            DiscoveryState::Idle => (locale.discovery_idle(), BadgeTone::Neutral),
            DiscoveryState::Scanning => (locale.scanning(), BadgeTone::Info),
            DiscoveryState::Ready => (locale.discovery_ready(), BadgeTone::Success),
            DiscoveryState::Empty => (locale.no_devices(), BadgeTone::Neutral),
            DiscoveryState::Error => (locale.discovery_error(), BadgeTone::Error),
        };
        status_badge(ui, status, tone);
        let response = if discovery_active {
            secondary_button(ui, locale.stop_scan(), true)
        } else {
            primary_button(ui, locale.start_scan(), true)
        };
        if response.clicked() {
            command = Some(if discovery_active {
                UserCommand::StopDiscovery
            } else {
                UserCommand::StartDiscovery
            });
        }
    });

    if snapshot.inbound_session_count > 0 {
        ui.add_space(12.0);
        components::muted_surface().show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                status_badge(
                    ui,
                    &format!(
                        "{}: {}",
                        locale.inbound_sessions(),
                        snapshot.inbound_session_count
                    ),
                    BadgeTone::Info,
                );
                ui.label(
                    RichText::new(locale.mesh_status_hint())
                        .size(12.0)
                        .color(MUTED),
                );
            });
        });
    }

    ui.add_space(16.0);
    if snapshot.peers.is_empty() {
        components::empty_state(
            ui,
            if snapshot.discovery == DiscoveryState::Scanning {
                locale.scanning()
            } else {
                locale.no_devices()
            },
            locale.no_devices_hint(),
            snapshot.discovery == DiscoveryState::Scanning,
        );
        return command;
    }

    components::surface().show(ui, |ui| {
        components::section_title(ui, locale.devices(), Some(locale.mesh_status_hint()));
        ui.add_space(10.0);

        for (index, (peer_id, peer)) in snapshot.peers.iter().enumerate() {
            if index > 0 {
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(8.0);
            }

            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(&peer.name).size(16.0).strong().color(TEXT));
                if peer.fingerprint_pinned {
                    status_badge(ui, locale.fingerprint_pinning(), BadgeTone::Neutral);
                }
            });
            if !peer.endpoints.is_empty() {
                ui.add(
                    egui::Label::new(
                        RichText::new(peer.endpoints.join("  ·  "))
                            .monospace()
                            .size(11.0)
                            .color(MUTED),
                    )
                    .wrap(),
                );
            }
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                status_badge(
                    ui,
                    match peer.discovery {
                        PeerDiscoveryState::Found => locale.discovery_found(),
                        PeerDiscoveryState::Lost => locale.discovery_lost(),
                    },
                    match peer.discovery {
                        PeerDiscoveryState::Found => BadgeTone::Success,
                        PeerDiscoveryState::Lost => BadgeTone::Neutral,
                    },
                );

                let (transport, tone) = match (&peer.dial_role, &peer.transport) {
                    (DialRole::Inbound, _) => (locale.transport_inbound_role(), BadgeTone::Info),
                    (DialRole::Outbound, TransportState::Waiting) => {
                        (locale.transport_waiting(), BadgeTone::Neutral)
                    }
                    (DialRole::Outbound, TransportState::Connecting) => {
                        (locale.transport_connecting(), BadgeTone::Info)
                    }
                    (DialRole::Outbound, TransportState::Connected) => {
                        (locale.transport_connected(), BadgeTone::Success)
                    }
                    (DialRole::Outbound, TransportState::Failed) => {
                        (locale.transport_failed(), BadgeTone::Error)
                    }
                };
                status_badge(ui, transport, tone);

                status_badge(
                    ui,
                    if peer.screen == ScreenAvailability::Available {
                        locale.screen_available()
                    } else {
                        locale.screen_unavailable()
                    },
                    if peer.screen == ScreenAvailability::Available {
                        BadgeTone::Success
                    } else {
                        BadgeTone::Neutral
                    },
                );

                let enabled = peer.screen == ScreenAvailability::Available
                    && snapshot.has_mesh_session()
                    && snapshot.media == MediaState::Idle;
                if primary_button(ui, locale.watch(), enabled).clicked() {
                    command = Some(UserCommand::StartWatching {
                        path: crate::screen_path::for_peer(peer_id),
                    });
                }
            });
        }
    });

    command
}
