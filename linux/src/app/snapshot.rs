//! Immutable UI snapshots and guarded lifecycle transitions.

use thiserror::Error;

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

/// Peer transport lifecycle.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum PeerState {
    /// No peer session exists.
    #[default]
    Disconnected,
    /// A session is being established.
    Connecting { peer_id: String },
    /// One peer session is available.
    Connected { peer_id: String },
    /// The selected peer is closing.
    Disconnecting { peer_id: String },
    /// The most recent connection attempt failed.
    Failed { peer_id: String },
}

/// Screen publication lifecycle.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum PublishState {
    /// No screen publication exists.
    #[default]
    Idle,
    /// Portal selection or encoder setup is in progress.
    Preparing,
    /// Encoded frames are being published.
    Publishing,
    /// Capture and encoding are stopping.
    Stopping,
}

/// A nearby peer displayed by the UI.
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
}

/// The latest runtime state consumed by the UI.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppSnapshot {
    /// Current discovery lifecycle.
    pub discovery: DiscoveryState,
    /// Discovered peers, merged by stable identity.
    pub peers: Vec<DiscoveredPeer>,
    /// Current selected-peer lifecycle.
    pub peer: PeerState,
    /// Current screen-publish lifecycle.
    pub publish: PublishState,
    /// Most recent user-facing runtime failure.
    pub last_error: Option<String>,
}

/// A rejected lifecycle transition.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum StateError {
    /// A peer operation is already active.
    #[error("a peer operation is already active")]
    PeerBusy,
    /// Screen publishing requires a connected peer.
    #[error("screen publishing requires a connected peer")]
    PeerNotConnected,
    /// A publication is already preparing or active.
    #[error("screen publishing is already active")]
    PublishAlreadyActive,
    /// No active publication can be stopped.
    #[error("screen publishing is not active")]
    PublishNotActive,
    /// The current publication is already stopping.
    #[error("screen publishing is already stopping")]
    PublishAlreadyStopping,
    /// A completion event arrived for the wrong lifecycle phase.
    #[error("the lifecycle completion does not match the current state")]
    UnexpectedCompletion,
}

impl AppSnapshot {
    /// Begin discovery without changing peer or publication state.
    pub fn start_discovery(&mut self) {
        self.discovery = DiscoveryState::Scanning;
        self.last_error = None;
    }

    /// Stop discovery and clear nearby peers without disconnecting the selected peer.
    pub fn stop_discovery(&mut self) {
        self.discovery = DiscoveryState::Idle;
        self.peers.clear();
    }

    /// Mark the initial browse window complete when nothing was found.
    pub fn finish_initial_scan(&mut self) {
        if self.discovery == DiscoveryState::Scanning && self.peers.is_empty() {
            self.discovery = DiscoveryState::Empty;
        }
    }

    /// Replace the visible peer projection without changing the transport state.
    pub fn replace_peers(&mut self, mut peers: Vec<DiscoveredPeer>) {
        peers.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
        self.peers = peers;
        if self.discovery.is_active() {
            self.discovery = if self.peers.is_empty() {
                DiscoveryState::Empty
            } else {
                DiscoveryState::Ready
            };
        }
    }

    /// Record that discovery stopped unexpectedly.
    pub fn fail_discovery(&mut self, error: impl Into<String>) {
        self.discovery = DiscoveryState::Error;
        self.peers.clear();
        self.last_error = Some(error.into());
    }

    /// Enter the connecting phase for one peer.
    pub fn begin_connect(&mut self, peer_id: impl Into<String>) -> Result<(), StateError> {
        if !matches!(
            self.peer,
            PeerState::Disconnected | PeerState::Failed { .. }
        ) {
            return Err(StateError::PeerBusy);
        }
        self.peer = PeerState::Connecting {
            peer_id: peer_id.into(),
        };
        self.last_error = None;
        Ok(())
    }

    /// Mark the current connecting peer as connected.
    pub fn finish_connect(&mut self) -> Result<(), StateError> {
        let PeerState::Connecting { peer_id } = &self.peer else {
            return Err(StateError::UnexpectedCompletion);
        };
        self.peer = PeerState::Connected {
            peer_id: peer_id.clone(),
        };
        Ok(())
    }

    /// Record a terminal failure for the current connection attempt.
    pub fn fail_connect(&mut self, error: impl Into<String>) -> Result<(), StateError> {
        let PeerState::Connecting { peer_id } = &self.peer else {
            return Err(StateError::UnexpectedCompletion);
        };
        self.peer = PeerState::Failed {
            peer_id: peer_id.clone(),
        };
        self.last_error = Some(error.into());
        Ok(())
    }

    /// Begin disconnecting the current session.
    pub fn begin_disconnect(&mut self) -> Result<(), StateError> {
        let PeerState::Connected { peer_id } = &self.peer else {
            return Err(StateError::PeerNotConnected);
        };
        self.peer = PeerState::Disconnecting {
            peer_id: peer_id.clone(),
        };
        Ok(())
    }

    /// Finish releasing the current session.
    pub fn finish_disconnect(&mut self) -> Result<(), StateError> {
        if !matches!(self.peer, PeerState::Disconnecting { .. }) {
            return Err(StateError::UnexpectedCompletion);
        }
        self.disconnect();
        Ok(())
    }

    /// Begin preparing a screen publication.
    pub fn begin_publish(&mut self) -> Result<(), StateError> {
        if !matches!(self.peer, PeerState::Connected { .. }) {
            return Err(StateError::PeerNotConnected);
        }
        if self.publish != PublishState::Idle {
            return Err(StateError::PublishAlreadyActive);
        }
        self.publish = PublishState::Preparing;
        self.last_error = None;
        Ok(())
    }

    /// Mark the prepared publication as active.
    pub fn finish_publish(&mut self) -> Result<(), StateError> {
        if self.publish != PublishState::Preparing {
            return Err(StateError::UnexpectedCompletion);
        }
        self.publish = PublishState::Publishing;
        Ok(())
    }

    /// Return a failed preparation or active publication to idle.
    pub fn fail_publish(&mut self, error: impl Into<String>) -> Result<(), StateError> {
        if self.publish == PublishState::Idle {
            return Err(StateError::UnexpectedCompletion);
        }
        self.publish = PublishState::Idle;
        self.last_error = Some(error.into());
        Ok(())
    }

    /// Return a completed publication to idle without changing the peer.
    pub fn end_publish(&mut self) -> Result<(), StateError> {
        if self.publish == PublishState::Idle {
            return Err(StateError::UnexpectedCompletion);
        }
        self.publish = PublishState::Idle;
        Ok(())
    }

    /// Begin stopping an active screen publication.
    pub fn begin_stop_publish(&mut self) -> Result<(), StateError> {
        match self.publish {
            PublishState::Publishing => {
                self.publish = PublishState::Stopping;
                Ok(())
            }
            PublishState::Stopping => Err(StateError::PublishAlreadyStopping),
            PublishState::Idle | PublishState::Preparing => Err(StateError::PublishNotActive),
        }
    }

    /// Finish stopping a screen publication.
    pub fn finish_stop_publish(&mut self) -> Result<(), StateError> {
        if self.publish != PublishState::Stopping {
            return Err(StateError::UnexpectedCompletion);
        }
        self.publish = PublishState::Idle;
        Ok(())
    }

    /// Reset media before dropping the selected peer.
    pub fn disconnect(&mut self) {
        self.publish = PublishState::Idle;
        self.peer = PeerState::Disconnected;
        self.last_error = None;
    }
}
