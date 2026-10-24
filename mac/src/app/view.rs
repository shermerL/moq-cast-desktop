//! Pure presentation mapping for the M2 Nearby interface.

use std::collections::BTreeMap;

use crate::{
    network::PeerSession,
    remote::{ScreenAvailability, ScreenView as RemoteScreen},
    runtime::PeerSnapshot,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PresenceView {
    Nearby,
    NotNearby,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ConnectionView {
    Waiting,
    ConnectingSecurely,
    Connected,
    Rejected,
    Failed,
    Disconnected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PeerPresentation {
    pub(super) presence: PresenceView,
    pub(super) connection: ConnectionView,
}

impl PeerPresentation {
    pub(super) fn facts(self, available: bool) -> moqcast_lan_state::Peer {
        use moqcast_lan_state::Connection;
        let connection = match self.connection {
            ConnectionView::Waiting => Connection::Unknown,
            ConnectionView::ConnectingSecurely => Connection::Connecting,
            ConnectionView::Connected => Connection::Connected,
            ConnectionView::Rejected => Connection::Rejected,
            ConnectionView::Failed => Connection::Failed,
            ConnectionView::Disconnected => Connection::Disconnected,
        };
        moqcast_lan_state::Peer {
            discovered: self.presence == PresenceView::Nearby,
            connection,
            available,
        }
    }
}

impl From<&PeerSnapshot> for PeerPresentation {
    fn from(peer: &PeerSnapshot) -> Self {
        let presence = if peer.discovered {
            PresenceView::Nearby
        } else {
            PresenceView::NotNearby
        };
        let connection = match peer.session {
            PeerSession::Waiting => ConnectionView::Waiting,
            PeerSession::Connecting => ConnectionView::ConnectingSecurely,
            PeerSession::Connected => ConnectionView::Connected,
            PeerSession::Rejected => ConnectionView::Rejected,
            PeerSession::Failed => ConnectionView::Failed,
            PeerSession::Disconnected => ConnectionView::Disconnected,
        };
        Self {
            presence,
            connection,
        }
    }
}

pub(super) fn screen_availability(
    peer_id: &str,
    screens: &BTreeMap<String, RemoteScreen>,
) -> ScreenAvailability {
    screens
        .get(&crate::contract::screen_path(peer_id))
        .filter(|screen| screen.peer_id == peer_id)
        .map_or(ScreenAvailability::Unavailable, |screen| {
            screen.availability
        })
}
