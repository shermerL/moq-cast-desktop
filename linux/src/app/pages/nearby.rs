//! Device actions derived from the runtime's independent discovery and media state.
use super::super::{AppSnapshot, Locale, MediaState, PeerSnapshot, UserCommand};
use eframe::egui;
use moqcast_ui::v2::{self, Palette, StatusTone};

pub(in crate::app) enum NearbyAction {
    Details(String),
    Watch(UserCommand),
    ReturnToPlayer,
}
fn owns_playback(snapshot: &AppSnapshot, id: &str) -> bool {
    match &snapshot.media {
        MediaState::PreparingView { path }
        | MediaState::Viewing { path }
        | MediaState::StoppingView { path } => *path == crate::screen_path::for_peer(id),
        _ => false,
    }
}
fn can_watch(snapshot: &AppSnapshot, id: &str) -> bool {
    snapshot.can_watch(&crate::screen_path::for_peer(id))
}
pub(in crate::app) fn peer_status(
    snapshot: &AppSnapshot,
    id: &str,
    peer: &PeerSnapshot,
    locale: Locale,
) -> (StatusTone, &'static str) {
    if owns_playback(snapshot, id) {
        return match snapshot.media {
            MediaState::PreparingView { .. } => (StatusTone::Busy, locale.preparing_view()),
            MediaState::StoppingView { .. } => (StatusTone::Busy, locale.stopping_view()),
            _ => (StatusTone::Active, locale.viewing_screen()),
        };
    }
    use moqcast_lan_state::Status;
    let status = peer.facts().status();
    let tone = match status {
        Status::Available | Status::Connected => StatusTone::Active,
        Status::Connecting => StatusTone::Busy,
        Status::Failed | Status::Rejected => StatusTone::Error,
        _ => StatusTone::Neutral,
    };
    (tone, status.label(locale == Locale::Chinese))
}

pub(in crate::app) fn show(
    ui: &mut egui::Ui,
    locale: Locale,
    snapshot: &AppSnapshot,
    compact: bool,
    stopping: bool,
) -> Option<NearbyAction> {
    let p = Palette::for_ui(ui);
    let mut result = None;
    ui.label(
        egui::RichText::new(locale.nearby())
            .size(16.0)
            .variation(b"wght", 600.0),
    );
    ui.add_space(12.0);
    egui::ScrollArea::vertical()
        .id_salt("v2-nearby-list")
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if !snapshot.peers.values().any(|peer| peer.facts().visible()) {
                ui.add_space(32.0);
                ui.vertical_centered(|ui| {
                    ui.label(egui::RichText::new(locale.no_devices()).color(p.muted));
                });
            }
            for (id, peer) in snapshot
                .peers
                .iter()
                .filter(|(_, peer)| peer.facts().visible())
            {
                let current = owns_playback(snapshot, id);
                let (tone, status) = peer_status(snapshot, id, peer, locale);
                let response = v2::nearby_row(
                    ui,
                    v2::NearbyRow {
                        id: egui::Id::new(("v2-peer", id)),
                        name: &peer.name,
                        subtitle: &format!("{}: {id}", locale.text("设备 ID", "Device ID")),
                        status,
                        tone,
                        current: current && matches!(snapshot.media, MediaState::Viewing { .. }),
                        watch_label: if current {
                            locale.text("返回播放器", "Return")
                        } else {
                            locale.watch()
                        },
                        watch_enabled: current || (!stopping && can_watch(snapshot, id)),
                        details_label: locale.device_details(),
                        compact,
                    },
                );
                if response.details {
                    result = Some(NearbyAction::Details(id.clone()));
                }
                if response.watch {
                    result = Some(if current {
                        NearbyAction::ReturnToPlayer
                    } else {
                        NearbyAction::Watch(UserCommand::StartWatching {
                            path: crate::screen_path::for_peer(id),
                        })
                    });
                }
                ui.add_space(8.0);
            }
        });
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{
        DialRole, DiscoveryState, PeerDiscoveryState, ScreenAvailability, TransportState,
    };

    fn peer() -> PeerSnapshot {
        PeerSnapshot {
            bandwidth: Default::default(),
            name: "Computer".into(),
            endpoints: vec![],
            fingerprint_pinned: true,
            dial_role: DialRole::Outbound,
            discovery: PeerDiscoveryState::Lost,
            transport: TransportState::Connected,
            screen: ScreenAvailability::Available,
        }
    }

    #[test]
    fn discovery_loss_keeps_valid_route_watchable_until_shutdown() {
        let p = peer();
        let mut s = AppSnapshot {
            local_peer_id: Some("local".into()),
            discovery: DiscoveryState::Ready,
            ..Default::default()
        };
        s.peers.insert("peer".into(), p.clone());
        s.update_remote_screen(crate::screen_path::for_peer("peer"), true);
        assert!(can_watch(&s, "peer"));
        assert_eq!(peer_status(&s, "peer", &p, Locale::English).1, "Available");
        s.lan_stopping = true;
        assert!(!can_watch(&s, "peer"));
        s.lan_stopping = false;
        s.media = MediaState::Publishing;
        assert!(!can_watch(&s, "peer"));
    }

    #[test]
    fn unrelated_inbound_does_not_enable_watch_without_a_route() {
        let mut s = AppSnapshot {
            local_peer_id: Some("local".into()),
            discovery: DiscoveryState::Ready,
            inbound_session_count: 1,
            ..Default::default()
        };
        s.peers.insert("peer".into(), peer());
        assert!(!can_watch(&s, "peer"));
    }

    #[test]
    fn preparing_is_not_reported_as_playing_and_return_does_not_restart() {
        let p = peer();
        let s = AppSnapshot {
            media: MediaState::PreparingView {
                path: crate::screen_path::for_peer("peer"),
            },
            ..Default::default()
        };
        assert!(owns_playback(&s, "peer"));
        assert!(!owns_playback(&s, "other"));
        assert_eq!(
            peer_status(&s, "peer", &p, Locale::English).1,
            Locale::English.preparing_view()
        );
        assert!(!can_watch(&s, "peer"));
    }
}
