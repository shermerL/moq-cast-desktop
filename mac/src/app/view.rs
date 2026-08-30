//! Pure presentation mapping for the M2 Nearby interface.

use std::collections::BTreeMap;

use crate::{
    network::PeerSession,
    remote::{ScreenAvailability, ScreenView as RemoteScreen},
    runtime::PeerSnapshot,
};

pub(super) const CONTENT_BREAKPOINT: f32 = 920.0;
pub(super) const NAVIGATION_BREAKPOINT: f32 = 760.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ContentLayout {
    SingleColumn,
    ListDetail,
}

impl ContentLayout {
    pub(super) fn for_width(width: f32) -> Self {
        if width >= CONTENT_BREAKPOINT {
            Self::ListDetail
        } else {
            Self::SingleColumn
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NavigationLayout {
    TwoRows,
    OneRow,
}

impl NavigationLayout {
    pub(super) fn for_width(width: f32) -> Self {
        if width >= NAVIGATION_BREAKPOINT {
            Self::OneRow
        } else {
            Self::TwoRows
        }
    }
}

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

pub(super) fn selected_peer(
    current: Option<&str>,
    peers: &BTreeMap<String, PeerSnapshot>,
) -> Option<String> {
    current
        .filter(|id| peers.contains_key(*id))
        .map(str::to_owned)
        .or_else(|| peers.keys().next().cloned())
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
