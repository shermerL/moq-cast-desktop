//! Nearby discovery and direct-only MoQ session ownership.

pub(crate) mod discovery;
pub(crate) mod security;
pub(crate) mod session;

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use moq_tokio::mdns;
use thiserror::Error;

use self::{
    discovery::{PeerRecord, PeerRegistry, PeerUpdate},
    session::{
        SessionEvent, SessionFoundation, SessionSubject, TransportDirection, TransportPhase,
    },
};
use crate::remote;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DialRole {
    Active,
    Passive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PeerSession {
    Waiting,
    Connecting,
    Connected,
    Rejected,
    Failed,
    Disconnected,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PeerStatus {
    pub(crate) id: String,
    pub(crate) ordinal: u64,
    pub(crate) discovered: bool,
    pub(crate) role: DialRole,
    pub(crate) session: PeerSession,
    pub(crate) transport_generation: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Event {
    Peer(PeerStatus),
    PeerRemoved(String),
    InboundCount(usize),
    InboundRejected,
    Screen(remote::Update),
    DiscoveryStopped,
    ListenerStopped,
}

#[derive(Debug, Error)]
pub(crate) enum StartError {
    #[error("direct listener could not start")]
    Listener(#[from] session::StartError),
    #[error("nearby discovery could not start")]
    Discovery(#[from] mdns::Error),
}

pub(crate) struct Services {
    discovery: mdns::Discovery,
    discovery_active: bool,
    sessions_active: bool,
    remote_active: bool,
    registry: PeerRegistry,
    sessions: SessionFoundation,
    remote: remote::Directory,
    peers: BTreeMap<String, PeerStatus>,
    inbound: BTreeSet<u64>,
    pending: VecDeque<Event>,
    next_ordinal: u64,
}

impl Services {
    pub(crate) async fn start() -> Result<Self, StartError> {
        let _ = moq_tokio::rustls::crypto::aws_lc_rs::default_provider().install_default();
        let bound = SessionFoundation::bind("[::]:0".parse().expect("valid listener bind"))?;
        let advertisement = bound.advertisement().clone();
        let discovery = mdns::Config::new(advertisement.addr.port())
            .with_fingerprint(advertisement.fingerprint)
            .advertise()
            .await?;
        let registry = PeerRegistry::new(discovery.id());
        let sessions = bound.start(discovery.credential().to_owned()).await?;
        let remote = remote::Directory::start(sessions.receive_origin(), discovery.id().to_owned());

        Ok(Self {
            discovery,
            discovery_active: true,
            sessions_active: true,
            remote_active: true,
            registry,
            sessions,
            remote,
            peers: BTreeMap::new(),
            inbound: BTreeSet::new(),
            pending: VecDeque::new(),
            next_ordinal: 0,
        })
    }

    pub(crate) async fn recv(&mut self) -> Option<Event> {
        loop {
            if let Some(event) = self.pending.pop_front() {
                return Some(event);
            }

            let input = tokio::select! {
                event = self.discovery.recv(), if self.discovery_active => Input::Discovery(event),
                event = self.sessions.recv(), if self.sessions_active => Input::Session(event),
                event = self.remote.recv(), if self.remote_active => Input::Remote(event),
                else => return None,
            };
            self.handle_input(input).await;
        }
    }

    pub(crate) async fn shutdown(self) {
        self.remote.stop().await;
        self.sessions.shutdown().await;
        drop(self.discovery);
    }

    pub(crate) fn remote_broadcast(
        &self,
        path: &str,
    ) -> Option<moq_tokio::moq_net::broadcast::Consumer> {
        self.remote.broadcast(path)
    }

    pub(crate) fn local_peer_id(&self) -> &str {
        self.discovery.id()
    }

    pub(crate) fn publish_origin(&self) -> moq_tokio::moq_net::origin::Producer {
        self.sessions.publish_origin()
    }

    async fn handle_input(&mut self, input: Input) {
        match input {
            Input::Discovery(Some(mdns::Event::Found(peer))) => self.found(peer).await,
            Input::Discovery(Some(mdns::Event::Lost(id))) => self.lost(&id).await,
            Input::Discovery(Some(_)) => {}
            Input::Discovery(None) => {
                self.discovery_active = false;
                self.pending.push_back(Event::DiscoveryStopped);
            }
            Input::Session(Some(SessionEvent::Transport(update))) => {
                self.transport(update).await;
            }
            Input::Session(Some(SessionEvent::ListenerStopped)) => {
                self.pending.push_back(Event::ListenerStopped);
            }
            Input::Session(None) => self.sessions_active = false,
            Input::Remote(Some(update)) => self.pending.push_back(Event::Screen(update)),
            Input::Remote(None) => self.remote_active = false,
        }
    }

    async fn found(&mut self, peer: mdns::Peer) {
        let should_dial = self.discovery.should_dial(&peer.id);
        let found = PeerRecord::from_mdns(peer);
        let id = found.id.clone();
        let update = self.registry.found(found);
        if !update.changed() {
            return;
        }
        let record = self
            .registry
            .get(&id)
            .expect("a changed peer remains registered")
            .clone();

        let role = if should_dial {
            DialRole::Active
        } else {
            DialRole::Passive
        };
        let ordinal = match self.peers.get(&record.id) {
            Some(peer) => peer.ordinal,
            None => {
                self.next_ordinal = self.next_ordinal.saturating_add(1);
                self.next_ordinal
            }
        };
        let previous = self.peers.get(&record.id).cloned();
        let mut status = PeerStatus {
            id: record.id.clone(),
            ordinal,
            discovered: true,
            role,
            session: previous
                .as_ref()
                .map_or(PeerSession::Waiting, |peer| peer.session),
            transport_generation: previous.and_then(|peer| peer.transport_generation),
        };

        if role == DialRole::Passive {
            status.session = PeerSession::Waiting;
            status.transport_generation = None;
        } else if should_connect(update, status.session) {
            match self.sessions.connect(&record).await {
                Ok(transport) => {
                    status.session = PeerSession::Connecting;
                    status.transport_generation = Some(transport.state.generation());
                }
                Err(_) => {
                    status.session = PeerSession::Failed;
                    status.transport_generation = None;
                }
            }
        }

        self.peers.insert(record.id, status.clone());
        self.pending.push_back(Event::Peer(status));
    }

    async fn lost(&mut self, id: &str) {
        if !self.registry.lost(id) {
            return;
        }
        let Some(mut status) = self.peers.remove(id) else {
            return;
        };
        status.discovered = false;
        if retain_after_lost(&status) {
            self.peers.insert(id.to_owned(), status.clone());
            self.pending.push_back(Event::Peer(status));
        } else {
            let _ = self.sessions.disconnect(id).await;
            self.pending.push_back(Event::PeerRemoved(id.to_owned()));
        }
    }

    async fn transport(&mut self, update: session::TransportUpdate) {
        match update.subject {
            SessionSubject::Peer(id) => {
                debug_assert_eq!(update.state.direction(), TransportDirection::Outbound);
                let Some(status) = self.peers.get_mut(&id) else {
                    return;
                };
                if status.transport_generation != Some(update.state.generation()) {
                    return;
                }
                status.session = map_phase(update.state.phase());
                let status = status.clone();
                self.pending.push_back(Event::Peer(status.clone()));
                if !status.discovered && status.session != PeerSession::Connected {
                    self.peers.remove(&id);
                    self.pending.push_back(Event::PeerRemoved(id));
                }
            }
            SessionSubject::Inbound(id) => {
                debug_assert_eq!(update.state.direction(), TransportDirection::Inbound);
                let previous_count = self.inbound.len();
                match update.state.phase() {
                    TransportPhase::Connected => {
                        self.inbound.insert(id);
                    }
                    TransportPhase::Rejected => {
                        self.inbound.remove(&id);
                        self.pending.push_back(Event::InboundRejected);
                    }
                    TransportPhase::Failed | TransportPhase::Disconnected => {
                        self.inbound.remove(&id);
                    }
                    TransportPhase::Connecting => {}
                }
                if self.inbound.len() != previous_count {
                    self.pending
                        .push_back(Event::InboundCount(self.inbound.len()));
                }
            }
        }
    }
}

enum Input {
    Discovery(Option<mdns::Event>),
    Session(Option<SessionEvent>),
    Remote(Option<remote::Update>),
}

fn should_connect(update: PeerUpdate, phase: PeerSession) -> bool {
    match update {
        PeerUpdate::IdentityReplaced => true,
        PeerUpdate::Added | PeerUpdate::CandidatesMerged => matches!(
            phase,
            PeerSession::Waiting
                | PeerSession::Rejected
                | PeerSession::Failed
                | PeerSession::Disconnected
        ),
        PeerUpdate::Unchanged => false,
    }
}

fn retain_after_lost(peer: &PeerStatus) -> bool {
    peer.role == DialRole::Active && peer.session == PeerSession::Connected
}

fn map_phase(phase: TransportPhase) -> PeerSession {
    match phase {
        TransportPhase::Connecting => PeerSession::Connecting,
        TransportPhase::Connected => PeerSession::Connected,
        TransportPhase::Rejected => PeerSession::Rejected,
        TransportPhase::Failed => PeerSession::Failed,
        TransportPhase::Disconnected => PeerSession::Disconnected,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{DialRole, PeerSession, PeerStatus, retain_after_lost, should_connect};
    use crate::network::discovery::PeerUpdate;

    fn status(role: DialRole, session: PeerSession) -> PeerStatus {
        PeerStatus {
            id: "peer".to_owned(),
            ordinal: 1,
            discovered: false,
            role,
            session,
            transport_generation: Some(3),
        }
    }

    #[test]
    fn lost_keeps_only_a_healthy_precise_outbound_session() {
        assert!(retain_after_lost(&status(
            DialRole::Active,
            PeerSession::Connected
        )));
        assert!(!retain_after_lost(&status(
            DialRole::Active,
            PeerSession::Connecting
        )));
        assert!(!retain_after_lost(&status(
            DialRole::Passive,
            PeerSession::Connected
        )));
    }

    #[test]
    fn merged_candidates_do_not_replace_a_healthy_session() {
        assert!(!should_connect(
            PeerUpdate::CandidatesMerged,
            PeerSession::Connected
        ));
        assert!(should_connect(
            PeerUpdate::CandidatesMerged,
            PeerSession::Failed
        ));
        assert!(should_connect(
            PeerUpdate::IdentityReplaced,
            PeerSession::Connected
        ));
        assert!(!should_connect(PeerUpdate::Added, PeerSession::Connected));
    }

    #[tokio::test]
    #[ignore = "requires the macOS local network and mDNS sockets"]
    async fn two_local_services_discover_and_open_one_direct_session() {
        let mut first = super::Services::start().await.expect("first services");
        let mut second = super::Services::start().await.expect("second services");
        let mut connected = false;
        let mut inbound = false;

        tokio::time::timeout(Duration::from_secs(15), async {
            while !(connected && inbound) {
                let event = tokio::select! {
                    event = first.recv() => event,
                    event = second.recv() => event,
                }
                .expect("service event");
                match event {
                    super::Event::Peer(peer) => {
                        connected |= peer.session == PeerSession::Connected;
                    }
                    super::Event::InboundCount(count) => inbound |= count > 0,
                    _ => {}
                }
            }
        })
        .await
        .expect("services discovered and connected");

        first.shutdown().await;
        second.shutdown().await;
    }
}
