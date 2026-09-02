//! Nearby device workspace.

use std::collections::BTreeMap;

use eframe::egui;
use moqcast_ui::{
    DetailRowSpec, DeviceBadgeSpec, DeviceListItemSpec, DeviceListSpec, SettingRowSpec, Size,
    Spacing, detail_row, device_list, section_header, setting_row,
};

use super::super::components::{self, BadgeTone, primary_button, secondary_button, status_badge};
use super::super::{
    AppSnapshot, DeviceWorkspaceLayout, DialRole, DiscoveryState, Locale, MediaState,
    PeerDiscoveryState, PeerSnapshot, ScreenAvailability, TransportState, UserCommand,
};

const WORKSPACE_GAP: f32 = Spacing::LG;

fn device_list_viewport_height(available_height: f32) -> f32 {
    available_height.max(Size::DEVICE_ROW)
}

fn split_workspace_widths(total_width: f32) -> (f32, f32) {
    let detail_width = Size::NEARBY_LIST.min((total_width - WORKSPACE_GAP).max(1.0));
    let list_width = (total_width - detail_width - WORKSPACE_GAP).max(1.0);
    (list_width, detail_width)
}

pub(in crate::app) fn show(
    ui: &mut egui::Ui,
    locale: Locale,
    snapshot: &AppSnapshot,
    local_device_name: &str,
    selected_peer: &mut Option<String>,
    layout: DeviceWorkspaceLayout,
) -> Option<UserCommand> {
    let mut command = workspace_toolbar(ui, locale, snapshot, local_device_name);

    ui.add_space(Spacing::LG);
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
    local_device_name: &str,
) -> Option<UserCommand> {
    let discovery_active = snapshot.discovery.is_active();
    let mut command = None;
    let (status, _) = match snapshot.discovery {
        DiscoveryState::Idle => (locale.discovery_idle(), BadgeTone::Neutral),
        DiscoveryState::Scanning => (locale.scanning(), BadgeTone::Info),
        DiscoveryState::Ready => (locale.discovery_ready(), BadgeTone::Success),
        DiscoveryState::Empty => (locale.no_devices(), BadgeTone::Neutral),
        DiscoveryState::Error => (locale.discovery_error(), BadgeTone::Error),
    };
    let description = local_device_description(
        locale,
        local_device_name,
        status,
        snapshot.local_peer_id.as_deref(),
    );
    setting_row(
        ui,
        SettingRowSpec::new(device_id_label(locale)).description(&description),
        |ui| {
            let label = if discovery_active {
                locale.stop_scan()
            } else {
                locale.start_scan()
            };
            if secondary_button(ui, label, true).clicked() {
                command = Some(if discovery_active {
                    UserCommand::StopDiscovery
                } else {
                    UserCommand::StartDiscovery
                });
            }
        },
    );
    command
}

fn show_split_workspace(
    ui: &mut egui::Ui,
    locale: Locale,
    snapshot: &AppSnapshot,
    selected_peer: &mut Option<String>,
) -> Option<UserCommand> {
    let total_width = ui.available_width();
    let workspace_height = device_list_viewport_height(ui.available_height());
    let (list_width, detail_width) = split_workspace_widths(total_width);
    let mut command = None;
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(list_width, workspace_height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| show_device_list(ui, locale, snapshot, selected_peer, workspace_height),
        );
        ui.add_space(WORKSPACE_GAP);
        ui.allocate_ui_with_layout(
            egui::vec2(detail_width, workspace_height),
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
    let workspace_height = device_list_viewport_height(ui.available_height());
    show_device_list(ui, locale, snapshot, selected_peer, workspace_height);
    ui.add_space(Spacing::LG);
    let mut command = None;
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), 1.0),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            command = show_device_detail(ui, locale, snapshot, selected_peer.as_deref());
        },
    );
    command
}

fn show_device_list(
    ui: &mut egui::Ui,
    locale: Locale,
    snapshot: &AppSnapshot,
    selected_peer: &mut Option<String>,
    viewport_height: f32,
) {
    let subtitles = snapshot
        .peers
        .iter()
        .map(|(peer_id, peer)| peer_list_subtitle(locale, peer, peer_id))
        .collect::<Vec<_>>();
    let items = snapshot
        .peers
        .iter()
        .zip(&subtitles)
        .map(|((peer_id, peer), subtitle)| {
            DeviceListItemSpec::new(peer_id.clone(), &peer.name)
                .subtitle(subtitle)
                .badge(DeviceBadgeSpec::new(
                    peer_row_summary(locale, peer),
                    peer_badge_tone(peer),
                ))
                .selected(selected_peer.as_deref() == Some(peer_id.as_str()))
                .enabled(peer_can_be_selected(peer))
        })
        .collect::<Vec<_>>();
    if let Some(peer_id) = device_list(
        ui,
        DeviceListSpec::new(egui::Id::new("linux-nearby-peers"), &items)
            .viewport_height(viewport_height),
    ) {
        *selected_peer = Some(peer_id);
    }
}

fn show_device_detail(
    ui: &mut egui::Ui,
    locale: Locale,
    snapshot: &AppSnapshot,
    selected_peer: Option<&str>,
) -> Option<UserCommand> {
    let mut command = None;
    let Some((peer_id, peer)) =
        selected_peer.and_then(|peer_id| snapshot.peers.get(peer_id).map(|peer| (peer_id, peer)))
    else {
        section_header(ui, locale.device_details(), Some(locale.select_device()));
        return None;
    };

    section_header(ui, &peer.name, Some(peer_row_summary(locale, peer)));
    detail_value(
        ui,
        remote_device_id_label(locale),
        peer_id,
        BadgeTone::Neutral,
    );
    ui.separator();
    detail_value(
        ui,
        locale.discovery_status(),
        match peer.discovery {
            PeerDiscoveryState::Found => locale.discovery_found(),
            PeerDiscoveryState::Lost => locale.discovery_lost(),
        },
        match peer.discovery {
            PeerDiscoveryState::Found => BadgeTone::Success,
            PeerDiscoveryState::Lost => BadgeTone::Neutral,
        },
    );
    ui.separator();
    detail_value(
        ui,
        locale.connection_status(),
        transport_label(locale, peer),
        transport_tone(peer),
    );
    ui.separator();
    detail_value(
        ui,
        locale.shared_screen_status(),
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
    if peer.screen == ScreenAvailability::Available {
        ui.add_space(Spacing::LG);
        let watch_enabled = snapshot.has_mesh_session() && snapshot.media == MediaState::Idle;
        if primary_button(ui, locale.watch(), watch_enabled).clicked() {
            command = Some(UserCommand::StartWatching {
                path: crate::screen_path::for_peer(peer_id),
            });
        }
    }
    command
}

fn detail_value(ui: &mut egui::Ui, label: &str, value: &str, tone: BadgeTone) {
    detail_row(ui, DetailRowSpec::new(label), |ui| {
        status_badge(ui, value, tone);
    });
}

fn transport_label(locale: Locale, peer: &PeerSnapshot) -> &'static str {
    match (&peer.dial_role, &peer.transport) {
        (DialRole::Inbound, _) => locale.transport_inbound_role(),
        (DialRole::Outbound, TransportState::Waiting) => locale.transport_waiting(),
        (DialRole::Outbound, TransportState::Connecting) => locale.transport_connecting(),
        (DialRole::Outbound, TransportState::Connected) => locale.transport_connected(),
        (DialRole::Outbound, TransportState::Failed) => locale.transport_failed(),
    }
}

fn transport_tone(peer: &PeerSnapshot) -> BadgeTone {
    match (&peer.dial_role, &peer.transport) {
        (DialRole::Inbound, _) | (DialRole::Outbound, TransportState::Connecting) => {
            BadgeTone::Info
        }
        (DialRole::Outbound, TransportState::Connected) => BadgeTone::Success,
        (DialRole::Outbound, TransportState::Failed) => BadgeTone::Error,
        (DialRole::Outbound, TransportState::Waiting) => BadgeTone::Neutral,
    }
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

fn device_id_label(locale: Locale) -> &'static str {
    match locale {
        Locale::Chinese => "本机 ID",
        Locale::English => "This device ID",
    }
}

fn remote_device_id_label(locale: Locale) -> &'static str {
    match locale {
        Locale::Chinese => "设备 ID",
        Locale::English => "Device ID",
    }
}

fn local_device_description(
    locale: Locale,
    device_name: &str,
    status: &str,
    peer_id: Option<&str>,
) -> String {
    let identity = match (locale, peer_id) {
        (Locale::Chinese, Some(peer_id)) => format!("本次运行：{peer_id}"),
        (Locale::English, Some(peer_id)) => format!("Current run: {peer_id}"),
        (Locale::Chinese, None) => "本次运行尚未生成 ID".to_owned(),
        (Locale::English, None) => "No ID for the current run yet".to_owned(),
    };
    format!("{device_name} · {status} · {identity}")
}

fn peer_list_subtitle(locale: Locale, peer: &PeerSnapshot, peer_id: &str) -> String {
    format!(
        "{} · {}: {peer_id}",
        peer_row_summary(locale, peer),
        remote_device_id_label(locale)
    )
}

fn peer_badge_tone(peer: &PeerSnapshot) -> moqcast_ui::BadgeTone {
    if peer.discovery == PeerDiscoveryState::Lost {
        return moqcast_ui::BadgeTone::Neutral;
    }
    if peer.screen == ScreenAvailability::Available {
        return moqcast_ui::BadgeTone::Info;
    }
    match (&peer.dial_role, &peer.transport) {
        (DialRole::Outbound, TransportState::Failed) => moqcast_ui::BadgeTone::Danger,
        (DialRole::Inbound, _)
        | (DialRole::Outbound, TransportState::Connecting | TransportState::Connected) => {
            moqcast_ui::BadgeTone::Info
        }
        (DialRole::Outbound, TransportState::Waiting) => moqcast_ui::BadgeTone::Neutral,
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

    #[test]
    fn compact_detail_rows_keep_the_shared_fixed_height() {
        egui::__run_test_ui(|ui| {
            ui.set_width(420.0);
            let (rect, ()) = detail_row(ui, DetailRowSpec::new("Connection"), |ui| {
                ui.label("Connected");
            });
            assert_eq!(rect.width(), 420.0);
            assert_eq!(rect.height(), Size::DETAIL_ROW);
        });
    }

    #[test]
    fn split_workspace_keeps_the_detail_column_compact() {
        assert_eq!(
            split_workspace_widths(944.0),
            (944.0 - Size::NEARBY_LIST - WORKSPACE_GAP, Size::NEARBY_LIST)
        );
    }

    #[test]
    fn device_list_uses_remaining_height_with_a_row_sized_floor() {
        assert_eq!(device_list_viewport_height(420.0), 420.0);
        assert_eq!(device_list_viewport_height(24.0), Size::DEVICE_ROW);
    }

    #[test]
    fn identity_copy_exposes_only_current_run_and_remote_peer_ids() {
        let peer = peer(PeerDiscoveryState::Found, ScreenAvailability::Available);
        let local = local_device_description(
            Locale::English,
            "Workstation",
            "Nearby is on",
            Some("local-peer"),
        );
        let remote = peer_list_subtitle(Locale::English, &peer, "remote-peer");

        assert_eq!(device_id_label(Locale::English), "This device ID");
        assert_eq!(remote_device_id_label(Locale::English), "Device ID");
        assert!(local.contains("Current run: local-peer"));
        assert!(remote.contains("Device ID: remote-peer"));
        assert!(!local.contains("192.0.2.1"));
        assert!(!remote.contains("192.0.2.1"));
    }
}
