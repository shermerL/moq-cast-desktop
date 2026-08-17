//! Nearby device workspace.

use std::collections::BTreeMap;

use eframe::egui::{self, RichText};

use super::super::components::{
    self, BadgeTone, primary_button, secondary_button, status_badge, status_line,
};
use super::super::theme::{BRAND_DARK, BRAND_SOFT, MUTED, TEXT};
use super::super::{
    AppSnapshot, DeviceWorkspaceLayout, DialRole, DiscoveryState, Locale, MediaState,
    PeerDiscoveryState, PeerSnapshot, ScreenAvailability, TransportState, UserCommand,
};

pub(in crate::app) fn show(
    ui: &mut egui::Ui,
    locale: Locale,
    snapshot: &AppSnapshot,
    selected_peer: &mut Option<String>,
    layout: DeviceWorkspaceLayout,
) -> Option<UserCommand> {
    let mut command = workspace_toolbar(ui, locale, snapshot);

    if snapshot.inbound_session_count > 0 {
        ui.add_space(12.0);
        ui.horizontal_wrapped(|ui| {
            status_line(
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
    }

    ui.add_space(16.0);
    if snapshot.peers.is_empty() {
        *selected_peer = None;
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

    *selected_peer = reconcile_peer_selection(selected_peer.as_deref(), &snapshot.peers);
    let page_command = match layout {
        DeviceWorkspaceLayout::Split => show_split_workspace(ui, locale, snapshot, selected_peer),
        DeviceWorkspaceLayout::Single => show_single_workspace(ui, locale, snapshot, selected_peer),
    };
    command = command.or(page_command);
    command
}

fn workspace_toolbar(
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
        status_line(ui, status, tone);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let share_enabled = snapshot.has_mesh_session() && snapshot.media == MediaState::Idle;
            if primary_button(ui, locale.share_local_screen(), share_enabled).clicked() {
                command = Some(UserCommand::StartScreenShare);
            }
            let scan_label = if discovery_active {
                locale.stop_scan()
            } else {
                locale.start_scan()
            };
            if secondary_button(ui, scan_label, true).clicked() {
                command = Some(if discovery_active {
                    UserCommand::StopDiscovery
                } else {
                    UserCommand::StartDiscovery
                });
            }
        });
    });
    command
}

fn show_split_workspace(
    ui: &mut egui::Ui,
    locale: Locale,
    snapshot: &AppSnapshot,
    selected_peer: &mut Option<String>,
) -> Option<UserCommand> {
    let total_width = ui.available_width();
    let list_width = total_width.mul_add(0.34, 0.0).clamp(304.0, 344.0);
    let detail_width = (total_width - list_width - 16.0).max(1.0);
    let mut command = None;
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(list_width, 1.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| show_device_list(ui, locale, snapshot, selected_peer),
        );
        ui.add_space(8.0);
        ui.allocate_ui_with_layout(
            egui::vec2(detail_width, 1.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                command = show_device_detail(ui, locale, snapshot, selected_peer.as_deref());
            },
        );
    });
    command
}

fn show_single_workspace(
    ui: &mut egui::Ui,
    locale: Locale,
    snapshot: &AppSnapshot,
    selected_peer: &mut Option<String>,
) -> Option<UserCommand> {
    show_device_list(ui, locale, snapshot, selected_peer);
    ui.add_space(16.0);
    show_device_detail(ui, locale, snapshot, selected_peer.as_deref())
}

fn show_device_list(
    ui: &mut egui::Ui,
    locale: Locale,
    snapshot: &AppSnapshot,
    selected_peer: &mut Option<String>,
) {
    components::surface().show(ui, |ui| {
        ui.set_width(ui.available_width());
        components::section_title(ui, locale.devices(), None);
        ui.add_space(8.0);
        for (index, (peer_id, peer)) in snapshot.peers.iter().enumerate() {
            if index > 0 {
                ui.separator();
            }
            let selectable = peer_can_be_selected(peer);
            let selected = selected_peer.as_deref() == Some(peer_id.as_str());
            let summary = peer_row_summary(locale, peer);
            let response = ui
                .scope(|ui| {
                    ui.visuals_mut().selection.bg_fill = BRAND_SOFT;
                    ui.visuals_mut().selection.stroke = egui::Stroke::new(1.0, BRAND_DARK);
                    ui.add_enabled(
                        selectable,
                        egui::Button::new(
                            RichText::new(format!("{}\n{summary}", peer.name))
                                .size(13.0)
                                .color(if selected {
                                    BRAND_DARK
                                } else if selectable {
                                    TEXT
                                } else {
                                    MUTED
                                }),
                        )
                        .selected(selected)
                        .wrap()
                        .min_size(egui::vec2(ui.available_width(), 56.0)),
                    )
                })
                .inner;
            if response.clicked() {
                *selected_peer = Some(peer_id.clone());
            }
        }
    });
}

fn show_device_detail(
    ui: &mut egui::Ui,
    locale: Locale,
    snapshot: &AppSnapshot,
    selected_peer: Option<&str>,
) -> Option<UserCommand> {
    let mut command = None;
    components::surface().show(ui, |ui| {
        ui.set_width(ui.available_width());
        components::section_title(ui, locale.device_details(), None);
        ui.add_space(10.0);
        let Some((peer_id, peer)) = selected_peer
            .and_then(|peer_id| snapshot.peers.get(peer_id).map(|peer| (peer_id, peer)))
        else {
            ui.label(
                RichText::new(locale.select_device())
                    .size(13.0)
                    .color(MUTED),
            );
            return;
        };

        ui.label(RichText::new(&peer.name).size(18.0).strong().color(TEXT));
        ui.label(
            RichText::new(format!("{}: {peer_id}", locale.peer_identifier()))
                .monospace()
                .size(11.0)
                .color(MUTED),
        );
        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            peer_statuses(ui, locale, peer);
        });
        if !peer.endpoints.is_empty() {
            ui.add_space(12.0);
            ui.label(
                RichText::new(locale.network_endpoints())
                    .size(12.0)
                    .strong()
                    .color(TEXT),
            );
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
        if peer.fingerprint_pinned {
            ui.add_space(10.0);
            status_badge(ui, locale.fingerprint_pinning(), BadgeTone::Neutral);
        }
        ui.add_space(18.0);
        if peer.screen == ScreenAvailability::Available {
            let watch_enabled = snapshot.has_mesh_session() && snapshot.media == MediaState::Idle;
            if secondary_button(ui, locale.watch(), watch_enabled).clicked() {
                command = Some(UserCommand::StartWatching {
                    path: crate::screen_path::for_peer(peer_id),
                });
            }
        }
    });
    command
}

fn peer_statuses(ui: &mut egui::Ui, locale: Locale, peer: &PeerSnapshot) {
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
}

fn peer_row_summary(locale: Locale, peer: &PeerSnapshot) -> &'static str {
    if peer.discovery == PeerDiscoveryState::Lost {
        return locale.discovery_lost();
    }
    if peer.screen == ScreenAvailability::Available {
        return locale.screen_available();
    }
    match (&peer.dial_role, &peer.transport) {
        (DialRole::Inbound, _) => locale.transport_inbound_role(),
        (DialRole::Outbound, TransportState::Connecting) => locale.transport_connecting(),
        (DialRole::Outbound, TransportState::Connected) => locale.transport_connected(),
        (DialRole::Outbound, TransportState::Failed) => locale.transport_failed(),
        (DialRole::Outbound, TransportState::Waiting) => locale.transport_waiting(),
    }
}

fn peer_can_be_selected(peer: &PeerSnapshot) -> bool {
    peer.discovery == PeerDiscoveryState::Found && peer.screen != ScreenAvailability::Withdrawn
}

fn reconcile_peer_selection(
    selected_peer: Option<&str>,
    peers: &BTreeMap<String, PeerSnapshot>,
) -> Option<String> {
    selected_peer
        .and_then(|peer_id| {
            peers
                .get(peer_id)
                .filter(|peer| peer_can_be_selected(peer))
                .map(|_| peer_id.to_owned())
        })
        .or_else(|| {
            peers
                .iter()
                .find(|(_, peer)| peer_can_be_selected(peer))
                .map(|(peer_id, _)| peer_id.clone())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(discovery: PeerDiscoveryState, screen: ScreenAvailability) -> PeerSnapshot {
        PeerSnapshot {
            name: "Peer".to_owned(),
            endpoints: vec!["192.0.2.1:4443".to_owned()],
            fingerprint_pinned: true,
            dial_role: DialRole::Outbound,
            discovery,
            transport: TransportState::Connected,
            screen,
        }
    }

    #[test]
    fn selection_falls_back_when_the_selected_peer_is_lost() {
        let peers = BTreeMap::from([
            (
                "peer-a".to_owned(),
                peer(PeerDiscoveryState::Lost, ScreenAvailability::Available),
            ),
            (
                "peer-b".to_owned(),
                peer(PeerDiscoveryState::Found, ScreenAvailability::Available),
            ),
        ]);

        assert_eq!(
            reconcile_peer_selection(Some("peer-a"), &peers).as_deref(),
            Some("peer-b")
        );
    }

    #[test]
    fn selection_falls_back_when_the_selected_screen_is_withdrawn() {
        let peers = BTreeMap::from([
            (
                "peer-a".to_owned(),
                peer(PeerDiscoveryState::Found, ScreenAvailability::Withdrawn),
            ),
            (
                "peer-b".to_owned(),
                peer(PeerDiscoveryState::Found, ScreenAvailability::Unavailable),
            ),
        ]);

        assert_eq!(
            reconcile_peer_selection(Some("peer-a"), &peers).as_deref(),
            Some("peer-b")
        );
    }

    #[test]
    fn selection_clears_when_no_selectable_peer_remains() {
        let peers = BTreeMap::from([
            (
                "peer-a".to_owned(),
                peer(PeerDiscoveryState::Lost, ScreenAvailability::Available),
            ),
            (
                "peer-b".to_owned(),
                peer(PeerDiscoveryState::Found, ScreenAvailability::Withdrawn),
            ),
        ]);

        assert_eq!(reconcile_peer_selection(Some("peer-a"), &peers), None);
    }
}
