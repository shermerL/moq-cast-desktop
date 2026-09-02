//! Remote screen playback page.

use eframe::egui;
use moqcast_ui::{StatePanelKind, StatePanelSpec, primary_button, state_panel};

use super::super::player::{LivePlayer, PlayerAction, PlayerMode};
use super::super::{AppSnapshot, Locale, MediaState, UserCommand};

pub(in crate::app) enum WatchAction {
    Command(UserCommand),
    OpenNearby,
}

pub(in crate::app) fn show(
    ui: &mut egui::Ui,
    locale: Locale,
    snapshot: &AppSnapshot,
    playback: Option<&egui::TextureHandle>,
    player: &mut LivePlayer,
) -> Option<WatchAction> {
    match &snapshot.media {
        MediaState::PreparingView { path } => player
            .show(
                ui,
                locale,
                PlayerMode::Preparing {
                    device: device_name(snapshot, path, locale),
                    audio: &snapshot.remote_audio,
                },
                playback,
            )
            .map(|PlayerAction::StopWatching| WatchAction::Command(UserCommand::StopWatching)),
        MediaState::Viewing { path } | MediaState::StoppingView { path } => player
            .show(
                ui,
                locale,
                PlayerMode::Viewing {
                    device: device_name(snapshot, path, locale),
                    stopping: matches!(snapshot.media, MediaState::StoppingView { .. }),
                    audio: &snapshot.remote_audio,
                },
                playback,
            )
            .map(|PlayerAction::StopWatching| WatchAction::Command(UserCommand::StopWatching)),
        _ => {
            let mut open_nearby = false;
            state_panel(
                ui,
                StatePanelSpec::new(
                    StatePanelKind::Empty,
                    locale.choose_watch_screen(),
                    locale.choose_watch_screen_hint(),
                ),
                |ui| {
                    open_nearby = primary_button(ui, locale.open_nearby(), true).clicked();
                },
            );
            open_nearby.then_some(WatchAction::OpenNearby)
        }
    }
}

fn device_name<'a>(snapshot: &'a AppSnapshot, path: &str, locale: Locale) -> &'a str {
    snapshot
        .peers
        .iter()
        .find(|(peer_id, _)| crate::screen_path::for_peer(peer_id) == path)
        .map(|(_, peer)| peer.name.as_str())
        .unwrap_or_else(|| locale.nearby_device())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{
        DialRole, PeerDiscoveryState, PeerSnapshot, ScreenAvailability, TransportState,
    };

    #[test]
    fn player_device_name_uses_public_device_copy_instead_of_the_screen_path() {
        let mut snapshot = AppSnapshot::default();
        snapshot.peers.insert(
            "peer-a".to_owned(),
            PeerSnapshot {
                name: "Office display".to_owned(),
                endpoints: Vec::new(),
                fingerprint_pinned: true,
                dial_role: DialRole::Outbound,
                discovery: PeerDiscoveryState::Found,
                transport: TransportState::Connected,
                screen: ScreenAvailability::Available,
            },
        );

        assert_eq!(
            device_name(
                &snapshot,
                &crate::screen_path::for_peer("peer-a"),
                Locale::English
            ),
            "Office display"
        );
        assert_eq!(
            device_name(&snapshot, "unknown", Locale::English),
            "Nearby device"
        );
    }
}
