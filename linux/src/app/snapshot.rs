//! Immutable UI snapshots and guarded lifecycle transitions.

use std::collections::BTreeMap;

use thiserror::Error;

use crate::screen_path;

/// Local discovery lifecycle.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum DiscoveryState {
    /// Discovery is not running.
    #[default]
    Idle,
    /// The runtime is browsing for LAN peers.
    Scanning,
    /// Discovery is active and one or more peers are available.
    Ready,
    /// Discovery is active but no peers are currently available.
    Empty,
    /// Discovery could not remain active.
    Error,
}

impl DiscoveryState {
    /// Whether an mDNS browser and listener should still be running.
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Scanning | Self::Ready | Self::Empty)
    }
}

/// Whether a peer is currently present in mDNS.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum PeerDiscoveryState {
    /// The peer is currently advertised.
    Found,
    /// The most recent advertisement was withdrawn or discovery restarted.
    #[default]
    Lost,
}

/// Transport lifecycle for one discovered peer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum TransportState {
    /// No exact outbound session is currently available.
    #[default]
    Waiting,
    /// The deterministic outbound session is being established.
    Connecting,
    /// The peer has an established outbound session.
    Connected,
    /// The most recent outbound connection attempt ended.
    Failed,
}

/// Deterministic connection direction for one discovered peer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DialRole {
    /// This desktop owns the exact outbound session and can report its state.
    Outbound,
    /// The remote peer is expected to dial; any inbound session remains unattributed.
    Inbound,
}

/// Availability of a peer's screen broadcast.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ScreenAvailability {
    /// No screen announcement has been observed.
    #[default]
    Unavailable,
    /// A remote screen broadcast is currently announced.
    Available,
    /// A previously announced screen broadcast was withdrawn.
    Withdrawn,
}

/// Screen media lifecycle. Publishing and viewing share this single state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum MediaState {
    /// No local publication or remote playback exists.
    #[default]
    Idle,
    /// Portal selection or encoder setup is in progress.
    PreparingPublish,
    /// Encoded local frames are being published.
    Publishing,
    /// Local capture and encoding are stopping.
    StoppingPublish,
    /// A remote screen subscription is being prepared.
    PreparingView { path: String },
    /// One remote screen is being viewed.
    Viewing { path: String },
    /// The remote screen subscription is stopping.
    StoppingView { path: String },
}

/// Discovery details used to update one peer row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredPeer {
    /// Stable peer identifier from discovery.
    pub id: String,
    /// Human-readable peer label.
    pub name: String,
    /// Resolved candidate endpoints in dial order.
    pub endpoints: Vec<String>,
    /// Whether outbound TLS will pin an advertised certificate fingerprint.
    pub fingerprint_pinned: bool,
    /// Deterministic dial direction derived from the active mDNS discovery.
    pub dial_role: DialRole,
}

/// Combined discovery, transport, and screen state for one known peer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeerSnapshot {
    /// Human-readable peer label.
    pub name: String,
    /// Resolved candidate endpoints in dial order.
    pub endpoints: Vec<String>,
    /// Whether outbound TLS will pin an advertised certificate fingerprint.
    pub fingerprint_pinned: bool,
    /// Which side owns dialing for this peer id pair.
    pub dial_role: DialRole,
    /// Current mDNS availability.
    pub discovery: PeerDiscoveryState,
    /// Exact outbound transport state, when this side owns dialing.
    pub transport: TransportState,
    /// Current remote screen availability.
    pub screen: ScreenAvailability,
}

/// One remote screen path retained across announcement withdrawal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteScreenSnapshot {
    /// Peer id encoded in the screen broadcast path.
    pub peer_id: String,
    /// Current announcement state.
    pub availability: ScreenAvailability,
}

/// The latest runtime state consumed by the UI.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppSnapshot {
    /// Current discovery lifecycle.
    pub discovery: DiscoveryState,
    /// Current local mDNS peer id, when services are active.
    pub local_peer_id: Option<String>,
    /// Known peers keyed by stable mDNS id.
    pub peers: BTreeMap<String, PeerSnapshot>,
    /// Authorized inbound sessions whose remote mDNS identity is unavailable.
    pub inbound_session_count: usize,
    /// Remote screen directory keyed by complete broadcast path.
    pub remote_screens: BTreeMap<String, RemoteScreenSnapshot>,
    /// Current screen media lifecycle.
    pub media: MediaState,
    /// Most recent user-facing runtime failure.
    pub last_error: Option<String>,
}

/// A rejected lifecycle transition.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum StateError {
    /// Screen media requires at least one established mesh session.
    #[error("screen media requires at least one mesh session")]
    MeshNotConnected,
    /// Another publication or playback already owns the screen media slot.
    #[error("screen media is already active")]
    MediaAlreadyActive,
    /// The selected remote screen is not currently available.
    #[error("the selected remote screen is not available")]
    ScreenNotAvailable,
    /// No active local publication can be stopped.
    #[error("screen publishing is not active")]
    PublishNotActive,
    /// The current local publication is already stopping.
    #[error("screen publishing is already stopping")]
    PublishAlreadyStopping,
    /// No active remote screen playback can be stopped.
    #[error("remote screen playback is not active")]
    ViewNotActive,
    /// A completion event arrived for the wrong lifecycle phase.
    #[error("the lifecycle completion does not match the current state")]
    UnexpectedCompletion,
}

impl AppSnapshot {
    /// Begin discovery without changing established mesh or media state.
    pub fn start_discovery(&mut self) {
        self.discovery = DiscoveryState::Scanning;
        self.last_error = None;
    }

    /// Stop discovery while retaining transport and historical device rows.
    pub fn stop_discovery(&mut self) {
        self.discovery = DiscoveryState::Idle;
        self.mark_all_peers_lost();
    }

    /// Mark the initial browse window complete when nothing was found.
    pub fn finish_initial_scan(&mut self) {
        if self.discovery == DiscoveryState::Scanning {
            self.discovery = if self.has_found_peers() {
                DiscoveryState::Ready
            } else {
                DiscoveryState::Empty
            };
        }
    }

    /// Insert or refresh one discovered peer without replacing transport state.
    pub fn upsert_peer(&mut self, peer: DiscoveredPeer) {
        let screen = self
            .remote_screens
            .get(&screen_path::for_peer(&peer.id))
            .map_or(ScreenAvailability::Unavailable, |screen| {
                screen.availability.clone()
            });
        self.peers
            .entry(peer.id)
            .and_modify(|current| {
                current.name.clone_from(&peer.name);
                current.endpoints.clone_from(&peer.endpoints);
                current.fingerprint_pinned = peer.fingerprint_pinned;
                current.dial_role.clone_from(&peer.dial_role);
                current.discovery = PeerDiscoveryState::Found;
                current.screen.clone_from(&screen);
            })
            .or_insert_with(|| PeerSnapshot {
                name: peer.name,
                endpoints: peer.endpoints,
                fingerprint_pinned: peer.fingerprint_pinned,
                dial_role: peer.dial_role,
                discovery: PeerDiscoveryState::Found,
                transport: TransportState::Waiting,
                screen,
            });
        if self.discovery.is_active() {
            self.discovery = DiscoveryState::Ready;
        }
    }

    /// Mark one peer absent from discovery without changing its transport.
    pub fn mark_peer_lost(&mut self, peer_id: &str) {
        if let Some(peer) = self.peers.get_mut(peer_id) {
            peer.discovery = PeerDiscoveryState::Lost;
            peer.endpoints.clear();
        }
        if self.discovery.is_active() && !self.has_found_peers() {
            self.discovery = DiscoveryState::Empty;
        }
    }

    /// Record that discovery stopped unexpectedly.
    pub fn fail_discovery(&mut self, error: impl Into<String>) {
        self.discovery = DiscoveryState::Error;
        self.mark_all_peers_lost();
        self.last_error = Some(error.into());
    }

    /// Update one exact outbound transport without affecting other peers.
    pub fn set_transport(&mut self, peer_id: &str, transport: TransportState) {
        if let Some(peer) = self.peers.get_mut(peer_id) {
            peer.transport = transport;
        }
    }

    /// Update the aggregate count of authorized inbound sessions.
    pub fn set_inbound_session_count(&mut self, count: usize) {
        self.inbound_session_count = count;
    }

    /// Apply one remote screen announcement or withdrawal.
    pub fn update_remote_screen(&mut self, path: String, available: bool) -> bool {
        let Some(peer_id) = screen_path::peer_id(&path).map(str::to_owned) else {
            return false;
        };
        if self.local_peer_id.as_deref() == Some(peer_id.as_str()) {
            return false;
        }

        let availability = if available {
            ScreenAvailability::Available
        } else {
            ScreenAvailability::Withdrawn
        };
        if self
            .remote_screens
            .get(&path)
            .is_some_and(|screen| screen.peer_id == peer_id && screen.availability == availability)
        {
            return false;
        }
        self.remote_screens.insert(
            path.clone(),
            RemoteScreenSnapshot {
                peer_id: peer_id.clone(),
                availability: availability.clone(),
            },
        );
        if let Some(peer) = self.peers.get_mut(&peer_id) {
            peer.screen = availability;
        }
        true
    }

    /// Whether any exact outbound or aggregate inbound mesh session exists.
    pub fn has_mesh_session(&self) -> bool {
        self.inbound_session_count > 0
            || self.peers.values().any(|peer| {
                peer.dial_role == DialRole::Outbound && peer.transport == TransportState::Connected
            })
    }

    /// Begin preparing a local screen publication.
    pub fn begin_publish(&mut self) -> Result<(), StateError> {
        if !self.has_mesh_session() {
            return Err(StateError::MeshNotConnected);
        }
        if self.media != MediaState::Idle {
            return Err(StateError::MediaAlreadyActive);
        }
        self.media = MediaState::PreparingPublish;
        self.last_error = None;
        Ok(())
    }

    /// Mark the prepared local publication as active.
    pub fn finish_publish(&mut self) -> Result<(), StateError> {
        if self.media != MediaState::PreparingPublish {
            return Err(StateError::UnexpectedCompletion);
        }
        self.media = MediaState::Publishing;
        Ok(())
    }

    /// Return a failed local publication to idle.
    pub fn fail_publish(&mut self, error: impl Into<String>) -> Result<(), StateError> {
        if !matches!(
            self.media,
            MediaState::PreparingPublish | MediaState::Publishing
        ) {
            return Err(StateError::UnexpectedCompletion);
        }
        self.media = MediaState::Idle;
        self.last_error = Some(error.into());
        Ok(())
    }

    /// Return a completed local publication to idle.
    pub fn end_publish(&mut self) -> Result<(), StateError> {
        if self.media != MediaState::Publishing {
            return Err(StateError::UnexpectedCompletion);
        }
        self.media = MediaState::Idle;
        Ok(())
    }

    /// Begin stopping an active local screen publication.
    pub fn begin_stop_publish(&mut self) -> Result<(), StateError> {
        match self.media {
            MediaState::Publishing => {
                self.media = MediaState::StoppingPublish;
                Ok(())
            }
            MediaState::StoppingPublish => Err(StateError::PublishAlreadyStopping),
            _ => Err(StateError::PublishNotActive),
        }
    }

    /// Finish stopping a local screen publication.
    pub fn finish_stop_publish(&mut self) -> Result<(), StateError> {
        if self.media != MediaState::StoppingPublish {
            return Err(StateError::UnexpectedCompletion);
        }
        self.media = MediaState::Idle;
        Ok(())
    }

    /// Reserve the media slot for one currently announced remote screen.
    pub fn begin_view(&mut self, path: &str) -> Result<(), StateError> {
        if !self.has_mesh_session() {
            return Err(StateError::MeshNotConnected);
        }
        if self.media != MediaState::Idle {
            return Err(StateError::MediaAlreadyActive);
        }
        let available = self
            .remote_screens
            .get(path)
            .is_some_and(|screen| screen.availability == ScreenAvailability::Available);
        if !available {
            return Err(StateError::ScreenNotAvailable);
        }
        self.media = MediaState::PreparingView {
            path: path.to_owned(),
        };
        self.last_error = None;
        Ok(())
    }

    /// Mark a prepared remote playback as active.
    pub fn finish_view(&mut self) -> Result<(), StateError> {
        let MediaState::PreparingView { path } = &self.media else {
            return Err(StateError::UnexpectedCompletion);
        };
        self.media = MediaState::Viewing { path: path.clone() };
        Ok(())
    }

    /// Return a failed remote playback to idle.
    pub fn fail_view(&mut self, error: impl Into<String>) -> Result<(), StateError> {
        if !matches!(
            self.media,
            MediaState::PreparingView { .. } | MediaState::Viewing { .. }
        ) {
            return Err(StateError::UnexpectedCompletion);
        }
        self.media = MediaState::Idle;
        self.last_error = Some(error.into());
        Ok(())
    }

    /// Return a completed remote playback to idle.
    pub fn end_view(&mut self) -> Result<(), StateError> {
        if !matches!(self.media, MediaState::Viewing { .. }) {
            return Err(StateError::UnexpectedCompletion);
        }
        self.media = MediaState::Idle;
        Ok(())
    }

    /// Begin stopping the current remote playback.
    pub fn begin_stop_view(&mut self) -> Result<(), StateError> {
        let path = match &self.media {
            MediaState::PreparingView { path } | MediaState::Viewing { path } => path.clone(),
            _ => return Err(StateError::ViewNotActive),
        };
        self.media = MediaState::StoppingView { path };
        Ok(())
    }

    /// Finish stopping the current remote playback.
    pub fn finish_stop_view(&mut self) -> Result<(), StateError> {
        if !matches!(self.media, MediaState::StoppingView { .. }) {
            return Err(StateError::UnexpectedCompletion);
        }
        self.media = MediaState::Idle;
        Ok(())
    }

    fn has_found_peers(&self) -> bool {
        self.peers
            .values()
            .any(|peer| peer.discovery == PeerDiscoveryState::Found)
    }

    fn mark_all_peers_lost(&mut self) {
        for peer in self.peers.values_mut() {
            peer.discovery = PeerDiscoveryState::Lost;
            peer.endpoints.clear();
        }
    }
}
