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
    media::{MediaSnapshot, Publication},
    registry::{PeerRegistry, PeerSummary, RegistryChange, sanitize_identity},
    session::{
        SessionFoundation, SessionSubject, TransportDirection, TransportPhase, TransportUpdate,
    },
};

const COMMAND_CAPACITY: usize = 32;
const MEDIA_EVENT_CAPACITY: usize = 4;

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
    pub(crate) media: MediaSnapshot,
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
            media: MediaSnapshot::default(),
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

    fn should_auto_connect(&self, peer: &str) -> bool {
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

    fn begin_auto_connect(&mut self, peer: &str) -> Option<u64> {
        if !self.should_auto_connect(peer) {
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
    ShareScreen,
    StopSharing,
    Shutdown,
}

enum MediaEvent {
    Ended { generation: u64, failed: bool },
}

#[derive(Default)]
struct PublicationOwner {
    generation: u64,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl PublicationOwner {
    fn start(
        &mut self,
        generation: u64,
        publication: crate::media::ReadyPublication,
        events: mpsc::Sender<MediaEvent>,
    ) {
        self.generation = generation;
        self.task = Some(tokio::spawn(async move {
            let result = publication.run().await;
            if let Err(error) = &result {
                tracing::warn!(stage = "publish", %error, "screen publication ended");
            }
            let _ = events
                .send(MediaEvent::Ended {
                    generation,
                    failed: result.is_err(),
                })
                .await;
        }));
    }

    fn finished(&mut self, generation: u64) {
        if self.generation == generation {
            self.task = None;
        }
    }

    async fn stop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
            let _ = task.await;
        }
    }
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
    let (media_events, mut media_recv) = mpsc::channel(MEDIA_EVENT_CAPACITY);
    let mut publication = PublicationOwner::default();
    let _ = snapshots.send(snapshot.clone());

    loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else { break };
                if handle_command(
                    command,
                    &mut snapshot,
                    &raw_peers,
                    &mut sessions,
                    &mut publication,
                    &media_events,
                ).await {
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
                        raw_peers.insert(key.clone(), peer);
                        snapshot.apply_registry(change);
                        if should_dial {
                            auto_connect(&key, &mut snapshot, &raw_peers, &mut sessions).await;
                        } else if let Some(update) = sessions.disconnect(&raw_id).await {
                            apply_session_update(&mut snapshot, update);
                        }
                    }
                    mdns::Event::Lost(raw_id) => {
                        let key = sanitize_identity(&raw_id);
                        raw_peers.remove(&key);
                        snapshot.apply_registry(registry.lost(&raw_id));
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
            event = media_recv.recv() => {
                let Some(MediaEvent::Ended { generation, failed }) = event else {
                    continue;
                };
                publication.finished(generation);
                snapshot.media.ended(generation, failed);
                let _ = snapshots.send(snapshot.clone());
            }
        }
    }

    publication.stop().await;
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
    _peers: &BTreeMap<String, mdns::Peer>,
    sessions: &mut SessionFoundation,
    publication: &mut PublicationOwner,
    media_events: &mpsc::Sender<MediaEvent>,
) -> bool {
    match command {
        RuntimeCommand::ShareScreen => {
            start_publication(snapshot, sessions, publication, media_events).await;
            false
        }
        RuntimeCommand::StopSharing => {
            let Some(generation) = snapshot.media.begin_stop() else {
                return false;
            };
            publication.stop().await;
            snapshot.media.stopped(generation);
            false
        }
        RuntimeCommand::Shutdown => {
            if let Some(generation) = snapshot.media.begin_stop() {
                publication.stop().await;
                snapshot.media.stopped(generation);
            }
            snapshot.shutdown();
            true
        }
    }
}

async fn start_publication(
    snapshot: &mut RuntimeSnapshot,
    sessions: &SessionFoundation,
    publication: &mut PublicationOwner,
    media_events: &mpsc::Sender<MediaEvent>,
) {
    let Some(local_id) = snapshot.local_id.clone() else {
        snapshot.last_error = Some("LAN services must be ready before sharing.");
        return;
    };
    let Some(generation) = snapshot.media.begin(&local_id) else {
        return;
    };
    let prepared = match Publication::prepare(sessions.origin(), &local_id) {
        Ok(prepared) => prepared,
        Err(_) => {
            snapshot.media.ended(generation, true);
            return;
        }
    };
    let ready = match prepared.configure().await {
        Ok(ready) => ready,
        Err(error) => {
            tracing::warn!(stage = "capture", %error, "screen capture preparation failed");
            snapshot.media.ended(generation, true);
            return;
        }
    };
    let info = ready.info();
    if snapshot.media.started(generation, info) {
        publication.start(generation, ready, media_events.clone());
    }
}

async fn auto_connect(
    key: &str,
    snapshot: &mut RuntimeSnapshot,
    peers: &BTreeMap<String, mdns::Peer>,
    sessions: &mut SessionFoundation,
) {
    let Some(generation) = snapshot.begin_auto_connect(key) else {
        return;
    };
    let Some(peer) = peers.get(key) else {
        return;
    };
    match sessions.connect(peer).await {
        Ok(update) => apply_session_update(snapshot, update),
        Err(_) => snapshot.apply_transport(
            key,
            TransportView {
                generation,
                direction: Some(TransportDirectionView::Outbound),
                phase: TransportPhaseView::Failed,
            },
        ),
    }
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
        assert!(snapshot.should_auto_connect("peer"));
        snapshot.begin_auto_connect("peer");
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
        assert_eq!(
            snapshot.peers["peer"].transport.phase,
            TransportPhaseView::Connecting
        );
    }

    #[test]
    fn deterministic_role_gates_automatic_connect() {
        let mut snapshot = RuntimeSnapshot::default();
        snapshot.apply_registry(RegistryChange::Added(peer("peer", "moqt://one:4443")));
        assert!(snapshot.should_auto_connect("peer"));
        snapshot.begin_auto_connect("peer");
        assert!(!snapshot.should_auto_connect("peer"));
        snapshot.apply_transport(
            "peer",
            TransportView {
                generation: 1,
                direction: Some(TransportDirectionView::Outbound),
                phase: TransportPhaseView::Connected,
            },
        );
        assert!(!snapshot.should_auto_connect("peer"));
    }

    #[test]
    fn inbound_role_cannot_issue_outbound_connect() {
        let mut snapshot = RuntimeSnapshot::default();
        let mut inbound = peer("peer", "moqt://one:4443");
        inbound.should_dial = false;
        snapshot.apply_registry(RegistryChange::Added(inbound));
        assert!(!snapshot.should_auto_connect("peer"));
    }

    #[test]
    fn old_generation_cannot_override_new_state() {
        let mut snapshot = RuntimeSnapshot::default();
        snapshot.apply_registry(RegistryChange::Added(peer("peer", "moqt://one:4443")));
        let first = snapshot
            .begin_auto_connect("peer")
            .expect("first generation");
        snapshot.apply_transport(
            "peer",
            TransportView {
                generation: first,
                direction: Some(TransportDirectionView::Outbound),
                phase: TransportPhaseView::Disconnected,
            },
        );
        snapshot.begin_auto_connect("peer");
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
        assert_eq!(snapshot.media.phase, crate::media::MediaPhase::Idle);
    }

    #[test]
    fn stopping_media_does_not_change_transport_state() {
        let mut snapshot = RuntimeSnapshot::default();
        snapshot.apply_registry(RegistryChange::Added(peer("peer", "moqt://one:4443")));
        let generation = snapshot.begin_auto_connect("peer").expect("connect");
        snapshot.apply_transport(
            "peer",
            TransportView {
                generation,
                direction: Some(TransportDirectionView::Outbound),
                phase: TransportPhaseView::Connected,
            },
        );
        let media_generation = snapshot.media.begin("local").expect("share");
        snapshot.media.started(
            media_generation,
            crate::media::PublicationInfo {
                width: 1920,
                height: 1080,
            },
        );
        snapshot.media.begin_stop();
        snapshot.media.stopped(media_generation);

        assert_eq!(snapshot.media.phase, crate::media::MediaPhase::Idle);
        assert_eq!(
            snapshot.peers["peer"].transport.phase,
            TransportPhaseView::Connected
        );
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
