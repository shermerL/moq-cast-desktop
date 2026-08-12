//! Background ownership and UI-safe snapshots for the Windows desktop shell.

use std::{
    collections::BTreeMap,
    net::SocketAddr,
    path::PathBuf,
    thread::{self, JoinHandle},
};

use moq_native::mdns;
use thiserror::Error;
use tokio::sync::{mpsc, watch};
use url::Url;

use crate::{
    registry::{PeerRegistry, PeerSummary, RegistryChange, sanitize_identity},
    session::{
        SessionFoundation, SessionSubject, TransportDirection, TransportPhase, TransportUpdate,
    },
};

const COMMAND_CAPACITY: usize = 32;

#[derive(Clone, Debug)]
pub(crate) struct RuntimeConfig {
    pub(crate) bind: SocketAddr,
    pub(crate) node: Option<Url>,
    pub(crate) secret_file: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum DiscoveryState {
    #[default]
    Starting,
    Ready,
    Empty,
    Failed,
    Stopped,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum MediaState {
    #[default]
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TransportDirectionView {
    Inbound,
    Outbound,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum TransportPhaseView {
    #[default]
    Waiting,
    Connecting,
    Connected,
    Rejected,
    Failed,
    Disconnected,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TransportView {
    pub(crate) generation: u64,
    pub(crate) direction: Option<TransportDirectionView>,
    pub(crate) phase: TransportPhaseView,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PeerView {
    pub(crate) id: String,
    pub(crate) candidates: Vec<String>,
    pub(crate) should_dial: bool,
    pub(crate) authenticated_discovery: bool,
    pub(crate) tls_pinned: bool,
    pub(crate) present: bool,
    pub(crate) transport: TransportView,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RuntimeSnapshot {
    pub(crate) discovery: DiscoveryState,
    pub(crate) media: MediaState,
    pub(crate) peers: BTreeMap<String, PeerView>,
    pub(crate) inbound_sessions: usize,
    pub(crate) listener: Option<String>,
    pub(crate) local_id: Option<String>,
    pub(crate) version: &'static str,
    pub(crate) last_error: Option<&'static str>,
    pub(crate) stopping: bool,
}

impl Default for RuntimeSnapshot {
    fn default() -> Self {
        Self {
            discovery: DiscoveryState::Starting,
            media: MediaState::Unavailable,
            peers: BTreeMap::new(),
            inbound_sessions: 0,
            listener: None,
            local_id: None,
            version: env!("CARGO_PKG_VERSION"),
            last_error: None,
            stopping: false,
        }
    }
}

impl RuntimeSnapshot {
    fn apply_registry(&mut self, change: RegistryChange) {
        match change {
            RegistryChange::Added(peer) | RegistryChange::Updated(peer) => self.upsert(peer),
            RegistryChange::Removed { id } => {
                if let Some(peer) = self.peers.get_mut(&id) {
                    peer.present = false;
                    peer.candidates.clear();
                }
            }
            RegistryChange::Unchanged | RegistryChange::IgnoredSelf => return,
        }
        self.discovery = if self.peers.values().any(|peer| peer.present) {
            DiscoveryState::Ready
        } else {
            DiscoveryState::Empty
        };
    }

    fn upsert(&mut self, summary: PeerSummary) {
        self.peers
            .entry(summary.id.clone())
            .and_modify(|peer| {
                peer.candidates.clone_from(&summary.candidates);
                peer.should_dial = summary.should_dial;
                peer.authenticated_discovery = summary.authenticated_discovery;
                peer.tls_pinned = summary.tls_pinned;
                peer.present = true;
            })
            .or_insert(PeerView {
                id: summary.id,
                candidates: summary.candidates,
                should_dial: summary.should_dial,
                authenticated_discovery: summary.authenticated_discovery,
                tls_pinned: summary.tls_pinned,
                present: true,
                transport: TransportView::default(),
            });
    }

    fn can_connect(&self, peer: &str) -> bool {
        self.peers.get(peer).is_some_and(|peer| {
            peer.present
                && peer.should_dial
                && matches!(
                    peer.transport.phase,
                    TransportPhaseView::Waiting
                        | TransportPhaseView::Failed
                        | TransportPhaseView::Disconnected
                )
        })
    }

    fn can_disconnect(&self, peer: &str) -> bool {
        self.peers.get(peer).is_some_and(|peer| {
            matches!(
                peer.transport.phase,
                TransportPhaseView::Connecting | TransportPhaseView::Connected
            )
        })
    }

    fn begin_connect(&mut self, peer: &str) -> Option<u64> {
        if !self.can_connect(peer) {
            return None;
        }
        let current = self.peers.get_mut(peer)?;
        let generation = current.transport.generation.saturating_add(1);
        current.transport = TransportView {
            generation,
            direction: Some(TransportDirectionView::Outbound),
            phase: TransportPhaseView::Connecting,
        };
        Some(generation)
    }

    fn begin_disconnect(&mut self, peer: &str) -> bool {
        if !self.can_disconnect(peer) {
            return false;
        }
        let current = self.peers.get_mut(peer).expect("checked peer");
        current.transport.generation = current.transport.generation.saturating_add(1);
        current.transport.phase = TransportPhaseView::Disconnected;
        true
    }

    fn apply_transport(&mut self, peer: &str, update: TransportView) {
        let Some(current) = self.peers.get_mut(peer) else {
            return;
        };
        if update.generation < current.transport.generation {
            return;
        }
        current.transport = update;
    }

    fn shutdown(&mut self) {
        self.stopping = true;
        self.discovery = DiscoveryState::Stopped;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RuntimeCommand {
    Connect(String),
    Disconnect(String),
    Shutdown,
}

#[derive(Debug, Error)]
pub(crate) enum RuntimeStartError {
    #[error("failed to start the Windows runtime owner thread")]
    Thread(#[from] std::io::Error),
}

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum CommandError {
    #[error("the runtime command queue is full")]
    Full,
    #[error("the runtime is no longer available")]
    Closed,
}

pub(crate) struct RuntimeOwner {
    commands: mpsc::Sender<RuntimeCommand>,
    snapshots: watch::Receiver<RuntimeSnapshot>,
    thread: Option<JoinHandle<()>>,
}

impl RuntimeOwner {
    pub(crate) fn start(config: RuntimeConfig) -> Result<Self, RuntimeStartError> {
        let (commands, command_rx) = mpsc::channel(COMMAND_CAPACITY);
        let (snapshot_tx, snapshots) = watch::channel(RuntimeSnapshot::default());
        let thread = thread::Builder::new()
            .name("moqcast-windows-runtime".to_owned())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build();
                match runtime {
                    Ok(runtime) => runtime.block_on(run(config, command_rx, snapshot_tx)),
                    Err(_) => {
                        let failed = RuntimeSnapshot {
                            discovery: DiscoveryState::Failed,
                            last_error: Some("The background runtime could not start."),
                            ..RuntimeSnapshot::default()
                        };
                        let _ = snapshot_tx.send(failed);
                    }
                }
            })?;
        Ok(Self {
            commands,
            snapshots,
            thread: Some(thread),
        })
    }

    pub(crate) fn snapshot(&mut self) -> RuntimeSnapshot {
        self.snapshots.borrow_and_update().clone()
    }

    pub(crate) fn try_send(&self, command: RuntimeCommand) -> Result<(), CommandError> {
        self.commands
            .try_send(command)
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => CommandError::Full,
                mpsc::error::TrySendError::Closed(_) => CommandError::Closed,
            })
    }

    fn close(&mut self) {
        if let Some(thread) = self.thread.take() {
            let _ = self.commands.blocking_send(RuntimeCommand::Shutdown);
            let _ = thread.join();
        }
    }
}

impl Drop for RuntimeOwner {
    fn drop(&mut self) {
        self.close();
    }
}

async fn run(
    config: RuntimeConfig,
    mut commands: mpsc::Receiver<RuntimeCommand>,
    snapshots: watch::Sender<RuntimeSnapshot>,
) {
    let mut snapshot = RuntimeSnapshot::default();
    let Some((mut discovery, mut registry, mut sessions)) =
        start_services(&config, &mut snapshot).await
    else {
        let _ = snapshots.send(snapshot);
        while let Some(command) = commands.recv().await {
            if command == RuntimeCommand::Shutdown {
                break;
            }
        }
        return;
    };
    let mut raw_peers = BTreeMap::<String, mdns::Peer>::new();
    let _ = snapshots.send(snapshot.clone());

    loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else { break };
                if handle_command(command, &mut snapshot, &raw_peers, &mut sessions).await {
                    let _ = snapshots.send(snapshot.clone());
                    break;
                }
                let _ = snapshots.send(snapshot.clone());
            }
            event = discovery.recv() => {
                let Some(event) = event else {
                    snapshot.discovery = DiscoveryState::Failed;
                    snapshot.last_error = Some("LAN discovery stopped unexpectedly.");
                    let _ = snapshots.send(snapshot.clone());
                    break;
                };
                match event {
                    mdns::Event::Found(peer) => {
                        let should_dial = discovery.should_dial(&peer.id);
                        let raw_id = peer.id.clone();
                        let key = sanitize_identity(&raw_id);
                        let change = registry.found(&peer, should_dial);
                        raw_peers.insert(key, peer);
                        snapshot.apply_registry(change);
                    }
                    mdns::Event::Lost(raw_id) => {
                        let key = sanitize_identity(&raw_id);
                        raw_peers.remove(&key);
                        snapshot.apply_registry(registry.lost(&raw_id));
                        if let Some(update) = sessions.disconnect(&raw_id).await {
                            apply_session_update(&mut snapshot, update);
                        }
                    }
                    _ => {}
                }
                let _ = snapshots.send(snapshot.clone());
            }
            update = sessions.recv() => {
                let Some(update) = update else {
                    snapshot.last_error = Some("The direct session listener stopped unexpectedly.");
                    let _ = snapshots.send(snapshot.clone());
                    break;
                };
                apply_session_update(&mut snapshot, update);
                let _ = snapshots.send(snapshot.clone());
            }
        }
    }

    snapshot.shutdown();
    let _ = snapshots.send(snapshot);
    sessions.shutdown().await;
}

async fn start_services(
    config: &RuntimeConfig,
    snapshot: &mut RuntimeSnapshot,
) -> Option<(mdns::Discovery, PeerRegistry, SessionFoundation)> {
    let bound = match SessionFoundation::bind(config.bind) {
        Ok(bound) => bound,
        Err(_) => return fail(snapshot, "The direct session listener could not bind."),
    };
    let advertisement = bound.advertisement().clone();
    let authenticated = config.secret_file.is_some();
    let mut discovery_config =
        mdns::Config::new(advertisement.addr.port()).with_fingerprint(advertisement.fingerprint);
    if let Some(node) = config.node.clone() {
        discovery_config = discovery_config.with_node(node);
    }
    if let Some(path) = &config.secret_file {
        let secret = match mdns::Secret::load(path.to_string_lossy().as_ref()) {
            Ok(secret) => secret,
            Err(_) => return fail(snapshot, "The LAN discovery secret could not be loaded."),
        };
        discovery_config = discovery_config.with_secret(secret);
    }
    let discovery = match discovery_config.advertise().await {
        Ok(discovery) => discovery,
        Err(_) => return fail(snapshot, "LAN discovery could not start."),
    };
    let registry = PeerRegistry::new(discovery.id(), authenticated);
    let sessions = match bound.start(discovery.credential().to_owned()).await {
        Ok(sessions) => sessions,
        Err(_) => return fail(snapshot, "The direct session listener could not start."),
    };
    snapshot.discovery = DiscoveryState::Empty;
    snapshot.listener = Some(sessions.advertisement().addr.to_string());
    snapshot.local_id = Some(sanitize_identity(discovery.id()));
    Some((discovery, registry, sessions))
}

fn fail<T>(snapshot: &mut RuntimeSnapshot, message: &'static str) -> Option<T> {
    snapshot.discovery = DiscoveryState::Failed;
    snapshot.last_error = Some(message);
    None
}

async fn handle_command(
    command: RuntimeCommand,
    snapshot: &mut RuntimeSnapshot,
    peers: &BTreeMap<String, mdns::Peer>,
    sessions: &mut SessionFoundation,
) -> bool {
    match command {
        RuntimeCommand::Connect(key) => {
            let Some(generation) = snapshot.begin_connect(&key) else {
                return false;
            };
            let Some(peer) = peers.get(&key) else {
                return false;
            };
            match sessions.connect(peer).await {
                Ok(update) => apply_session_update(snapshot, update),
                Err(_) => snapshot.apply_transport(
                    &key,
                    TransportView {
                        generation,
                        direction: Some(TransportDirectionView::Outbound),
                        phase: TransportPhaseView::Failed,
                    },
                ),
            }
        }
        RuntimeCommand::Disconnect(key) => {
            if snapshot.begin_disconnect(&key)
                && let Some(peer) = peers.get(&key)
                && let Some(update) = sessions.disconnect(&peer.id).await
            {
                apply_session_update(snapshot, update);
            }
        }
        RuntimeCommand::Shutdown => {
            snapshot.shutdown();
            return true;
        }
    }
    false
}

fn apply_session_update(snapshot: &mut RuntimeSnapshot, update: TransportUpdate) {
    match update.subject {
        SessionSubject::Peer(raw_id) => {
            let state = update.state;
            snapshot.apply_transport(
                &sanitize_identity(&raw_id),
                TransportView {
                    generation: state.generation(),
                    direction: Some(match state.direction() {
                        TransportDirection::Inbound => TransportDirectionView::Inbound,
                        TransportDirection::Outbound => TransportDirectionView::Outbound,
                    }),
                    phase: match state.phase() {
                        TransportPhase::Connecting => TransportPhaseView::Connecting,
                        TransportPhase::Connected => TransportPhaseView::Connected,
                        TransportPhase::Rejected => TransportPhaseView::Rejected,
                        TransportPhase::Failed => TransportPhaseView::Failed,
                        TransportPhase::Disconnected => TransportPhaseView::Disconnected,
                    },
                },
            );
        }
        SessionSubject::Inbound(_) => {
            snapshot.inbound_sessions = match update.state.phase() {
                TransportPhase::Connected => snapshot.inbound_sessions.saturating_add(1),
                TransportPhase::Disconnected => snapshot.inbound_sessions.saturating_sub(1),
                TransportPhase::Connecting | TransportPhase::Rejected | TransportPhase::Failed => {
                    snapshot.inbound_sessions
                }
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use super::*;

    fn peer(id: &str, candidate: &str) -> PeerSummary {
        PeerSummary {
            id: id.to_owned(),
            candidates: vec![candidate.to_owned()],
            should_dial: true,
            authenticated_discovery: true,
            tls_pinned: true,
        }
    }

    #[test]
    fn found_updated_and_lost_project_without_resetting_transport() {
        let mut snapshot = RuntimeSnapshot::default();
        snapshot.apply_registry(RegistryChange::Added(peer("peer", "moqt://one:4443")));
        assert_eq!(snapshot.discovery, DiscoveryState::Ready);
        assert!(snapshot.can_connect("peer"));
        snapshot.begin_connect("peer");
        snapshot.apply_registry(RegistryChange::Updated(peer("peer", "moqt://two:4443")));
        assert_eq!(snapshot.peers["peer"].candidates, vec!["moqt://two:4443"]);
        assert_eq!(
            snapshot.peers["peer"].transport.phase,
            TransportPhaseView::Connecting
        );
        snapshot.apply_registry(RegistryChange::Removed {
            id: "peer".to_owned(),
        });
        assert!(!snapshot.peers["peer"].present);
        assert_eq!(snapshot.discovery, DiscoveryState::Empty);
    }

    #[test]
    fn command_gate_separates_connect_and_disconnect() {
        let mut snapshot = RuntimeSnapshot::default();
        snapshot.apply_registry(RegistryChange::Added(peer("peer", "moqt://one:4443")));
        assert!(snapshot.can_connect("peer"));
        assert!(!snapshot.can_disconnect("peer"));
        snapshot.begin_connect("peer");
        assert!(!snapshot.can_connect("peer"));
        assert!(snapshot.can_disconnect("peer"));
        assert!(snapshot.begin_disconnect("peer"));
        assert!(snapshot.can_connect("peer"));
    }

    #[test]
    fn inbound_role_cannot_issue_outbound_connect() {
        let mut snapshot = RuntimeSnapshot::default();
        let mut inbound = peer("peer", "moqt://one:4443");
        inbound.should_dial = false;
        snapshot.apply_registry(RegistryChange::Added(inbound));
        assert!(!snapshot.can_connect("peer"));
    }

    #[test]
    fn old_generation_cannot_override_new_state() {
        let mut snapshot = RuntimeSnapshot::default();
        snapshot.apply_registry(RegistryChange::Added(peer("peer", "moqt://one:4443")));
        let first = snapshot.begin_connect("peer").expect("first generation");
        snapshot.begin_disconnect("peer");
        snapshot.begin_connect("peer");
        snapshot.apply_transport(
            "peer",
            TransportView {
                generation: first,
                direction: Some(TransportDirectionView::Outbound),
                phase: TransportPhaseView::Connected,
            },
        );
        assert_eq!(
            snapshot.peers["peer"].transport.phase,
            TransportPhaseView::Connecting
        );
    }

    #[test]
    fn shutdown_is_explicit_and_preserves_typed_media_state() {
        let mut snapshot = RuntimeSnapshot::default();
        snapshot.shutdown();
        assert!(snapshot.stopping);
        assert_eq!(snapshot.discovery, DiscoveryState::Stopped);
        assert_eq!(snapshot.media, MediaState::Unavailable);
    }

    #[test]
    fn dropping_runtime_owner_sends_shutdown_and_joins_thread() {
        let (commands, mut command_rx) = mpsc::channel(1);
        let (_snapshot_tx, snapshots) = watch::channel(RuntimeSnapshot::default());
        let stopped = Arc::new(AtomicBool::new(false));
        let observed = stopped.clone();
        let thread = thread::spawn(move || {
            if command_rx.blocking_recv() == Some(RuntimeCommand::Shutdown) {
                observed.store(true, Ordering::SeqCst);
            }
        });
        let owner = RuntimeOwner {
            commands,
            snapshots,
            thread: Some(thread),
        };

        drop(owner);

        assert!(stopped.load(Ordering::SeqCst));
    }
}
