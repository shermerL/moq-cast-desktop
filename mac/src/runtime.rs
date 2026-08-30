//! Generation-guarded application lifecycle owned outside the UI thread.

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use thiserror::Error;
use tokio::sync::{mpsc, watch};

use crate::network::{self, Event as NetworkEvent, PeerSession};

const COMMAND_CAPACITY: usize = 32;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Generation(u64);

impl Generation {
    fn next(self) -> Self {
        Self(
            self.0
                .checked_add(1)
                .expect("lifecycle generation counter exhausted"),
        )
    }

    #[cfg(test)]
    pub(crate) fn value(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Lifecycle<P> {
    generation: Generation,
    phase: P,
}

impl<P: Copy> Lifecycle<P> {
    pub(crate) fn new(phase: P) -> Self {
        Self {
            generation: Generation::default(),
            phase,
        }
    }

    pub(crate) fn begin(&mut self, phase: P) -> Generation {
        self.generation = self.generation.next();
        self.phase = phase;
        self.generation
    }

    pub(crate) fn apply(&mut self, generation: Generation, phase: P) -> bool {
        if self.generation != generation {
            return false;
        }
        self.phase = phase;
        true
    }

    pub(crate) fn generation(&self) -> Generation {
        self.generation
    }

    pub(crate) fn phase(&self) -> P {
        self.phase
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum RuntimePhase {
    #[default]
    Starting,
    Ready,
    Stopped,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum CapabilityPhase {
    #[default]
    Unavailable,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum DiscoveryPhase {
    #[default]
    Starting,
    Scanning,
    Ready,
    Empty,
    Failed,
    Stopped,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum SessionPhase {
    #[default]
    Starting,
    Listening,
    Failed,
    Stopped,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum MediaPhase {
    #[default]
    Idle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NearbyIssue {
    LocalNetworkUnavailable,
    DirectConnectionsUnavailable,
    DiscoveryStopped,
    ListenerStopped,
    DeviceRejected,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PeerSnapshot {
    pub(crate) ordinal: u64,
    pub(crate) discovered: bool,
    pub(crate) session: PeerSession,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AppSnapshot {
    pub(crate) runtime: Lifecycle<RuntimePhase>,
    pub(crate) discovery: Lifecycle<DiscoveryPhase>,
    pub(crate) session: Lifecycle<SessionPhase>,
    pub(crate) media: Lifecycle<MediaPhase>,
    pub(crate) capture: Lifecycle<CapabilityPhase>,
    pub(crate) decoder: Lifecycle<CapabilityPhase>,
    pub(crate) local_device_name: Option<String>,
    pub(crate) peers: BTreeMap<String, PeerSnapshot>,
    pub(crate) inbound_sessions: usize,
    pub(crate) nearby_issue: Option<NearbyIssue>,
}

impl Default for AppSnapshot {
    fn default() -> Self {
        Self {
            runtime: Lifecycle::new(RuntimePhase::Starting),
            discovery: Lifecycle::new(DiscoveryPhase::Starting),
            session: Lifecycle::new(SessionPhase::Starting),
            media: Lifecycle::new(MediaPhase::Idle),
            capture: Lifecycle::new(CapabilityPhase::Unavailable),
            decoder: Lifecycle::new(CapabilityPhase::Unavailable),
            local_device_name: local_device_name(),
            peers: BTreeMap::new(),
            inbound_sessions: 0,
            nearby_issue: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RuntimeCommand {
    Shutdown,
}

#[derive(Debug, Error)]
pub(crate) enum RuntimeStartError {
    #[error("failed to create the async runtime: {0}")]
    AsyncRuntime(#[source] std::io::Error),
    #[error("failed to create the runtime owner thread: {0}")]
    OwnerThread(#[source] std::io::Error),
}

pub(crate) struct RuntimeOwner {
    commands: mpsc::Sender<RuntimeCommand>,
    snapshot: watch::Receiver<Arc<AppSnapshot>>,
    owner: Option<thread::JoinHandle<()>>,
}

impl RuntimeOwner {
    pub(crate) fn start(
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self, RuntimeStartError> {
        Self::start_with(
            || async {
                network::Services::start()
                    .await
                    .map_err(|error| match error {
                        network::StartError::Discovery(_) => NearbyIssue::LocalNetworkUnavailable,
                        network::StartError::Listener(_) => {
                            NearbyIssue::DirectConnectionsUnavailable
                        }
                    })
            },
            wake,
        )
    }

    fn start_with<F, Fut, W>(start: F, wake: W) -> Result<Self, RuntimeStartError>
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<network::Services, NearbyIssue>> + Send + 'static,
        W: Fn() + Send + Sync + 'static,
    {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("moqcast-macos-async")
            .enable_all()
            .build()
            .map_err(RuntimeStartError::AsyncRuntime)?;
        let (commands, command_rx) = mpsc::channel(COMMAND_CAPACITY);
        let (snapshot_tx, snapshot) = watch::channel(Arc::new(AppSnapshot::default()));
        let wake = Arc::new(wake);
        let owner = thread::Builder::new()
            .name("moqcast-macos-runtime".to_owned())
            .spawn(move || runtime.block_on(run(command_rx, snapshot_tx, start(), wake)))
            .map_err(RuntimeStartError::OwnerThread)?;

        Ok(Self {
            commands,
            snapshot,
            owner: Some(owner),
        })
    }

    pub(crate) fn snapshot(&self) -> Arc<AppSnapshot> {
        self.snapshot.borrow().clone()
    }

    fn shutdown(&mut self) {
        let Some(owner) = self.owner.take() else {
            return;
        };
        let _ = self.commands.blocking_send(RuntimeCommand::Shutdown);
        if owner.join().is_err() {
            tracing::error!(stage = "shutdown", "runtime owner thread panicked");
        }
    }
}

impl Drop for RuntimeOwner {
    fn drop(&mut self) {
        self.shutdown();
    }
}

async fn run(
    mut commands: mpsc::Receiver<RuntimeCommand>,
    snapshot_tx: watch::Sender<Arc<AppSnapshot>>,
    start: impl Future<Output = Result<network::Services, NearbyIssue>>,
    wake: Arc<dyn Fn() + Send + Sync>,
) {
    let mut snapshot = AppSnapshot::default();
    let runtime_generation = snapshot.runtime.begin(RuntimePhase::Starting);
    assert!(
        snapshot
            .runtime
            .apply(runtime_generation, RuntimePhase::Ready)
    );
    let discovery_generation = snapshot.discovery.begin(DiscoveryPhase::Starting);
    let session_generation = snapshot.session.begin(SessionPhase::Starting);
    publish_snapshot(&snapshot_tx, &snapshot, &wake);
    tracing::info!(stage = "runtime", "macOS runtime owner ready");

    tokio::pin!(start);
    let mut services = tokio::select! {
        result = &mut start => match result {
            Ok(services) => {
                snapshot.discovery.apply(discovery_generation, DiscoveryPhase::Scanning);
                snapshot.session.apply(session_generation, SessionPhase::Listening);
                snapshot.nearby_issue = None;
                publish_snapshot(&snapshot_tx, &snapshot, &wake);
                services
            }
            Err(issue) => {
                snapshot.discovery.apply(discovery_generation, DiscoveryPhase::Failed);
                snapshot.session.apply(session_generation, SessionPhase::Failed);
                snapshot.nearby_issue = Some(issue);
                publish_snapshot(&snapshot_tx, &snapshot, &wake);
                let _ = commands.recv().await;
                stop_snapshot(&mut snapshot, &snapshot_tx, &wake);
                return;
            }
        },
        command = commands.recv() => {
            if matches!(command, Some(RuntimeCommand::Shutdown) | None) {
                stop_snapshot(&mut snapshot, &snapshot_tx, &wake);
                return;
            }
            unreachable!("shutdown is the only runtime command");
        }
    };

    let mut initial_scan = Box::pin(tokio::time::sleep(Duration::from_secs(3)));
    let mut scan_finished = false;
    loop {
        let input = if scan_finished {
            tokio::select! {
                command = commands.recv() => RuntimeInput::Command(command),
                event = services.recv() => RuntimeInput::Network(event),
            }
        } else {
            tokio::select! {
                command = commands.recv() => RuntimeInput::Command(command),
                event = services.recv() => RuntimeInput::Network(event),
                () = &mut initial_scan => RuntimeInput::InitialScanFinished,
            }
        };

        let previous = snapshot.clone();
        let network_exhausted = match input {
            RuntimeInput::Command(Some(RuntimeCommand::Shutdown) | None) => break,
            RuntimeInput::Network(Some(event)) => {
                apply_network_event(
                    &mut snapshot,
                    discovery_generation,
                    session_generation,
                    event,
                );
                refresh_discovery_result(&mut snapshot, discovery_generation, scan_finished);
                false
            }
            RuntimeInput::Network(None) => {
                snapshot
                    .discovery
                    .apply(discovery_generation, DiscoveryPhase::Failed);
                snapshot
                    .session
                    .apply(session_generation, SessionPhase::Failed);
                snapshot.nearby_issue = Some(NearbyIssue::ListenerStopped);
                true
            }
            RuntimeInput::InitialScanFinished => {
                scan_finished = true;
                let phase = if snapshot.peers.values().any(|peer| peer.discovered) {
                    DiscoveryPhase::Ready
                } else {
                    DiscoveryPhase::Empty
                };
                snapshot.discovery.apply(discovery_generation, phase);
                false
            }
        };
        if snapshot != previous {
            publish_snapshot(&snapshot_tx, &snapshot, &wake);
        }
        if network_exhausted {
            break;
        }
    }

    services.shutdown().await;
    stop_snapshot(&mut snapshot, &snapshot_tx, &wake);
}

enum RuntimeInput {
    Command(Option<RuntimeCommand>),
    Network(Option<NetworkEvent>),
    InitialScanFinished,
}

fn apply_network_event(
    snapshot: &mut AppSnapshot,
    discovery_generation: Generation,
    session_generation: Generation,
    event: NetworkEvent,
) {
    if snapshot.discovery.generation() != discovery_generation
        || snapshot.session.generation() != session_generation
    {
        return;
    }

    match event {
        NetworkEvent::Peer(peer) => {
            let view = PeerSnapshot {
                ordinal: peer.ordinal,
                discovered: peer.discovered,
                session: peer.session,
            };
            snapshot.peers.insert(peer.id, view);
            if peer.session == PeerSession::Connected
                && snapshot.nearby_issue == Some(NearbyIssue::DeviceRejected)
            {
                snapshot.nearby_issue = None;
            }
            if peer.discovered {
                snapshot
                    .discovery
                    .apply(discovery_generation, DiscoveryPhase::Ready);
            }
        }
        NetworkEvent::PeerRemoved(id) => {
            snapshot.peers.remove(&id);
        }
        NetworkEvent::InboundCount(count) => snapshot.inbound_sessions = count,
        NetworkEvent::InboundRejected => {
            snapshot.nearby_issue = Some(NearbyIssue::DeviceRejected);
        }
        NetworkEvent::DiscoveryStopped => {
            snapshot
                .discovery
                .apply(discovery_generation, DiscoveryPhase::Failed);
            snapshot.nearby_issue = Some(NearbyIssue::DiscoveryStopped);
        }
        NetworkEvent::ListenerStopped => {
            snapshot
                .session
                .apply(session_generation, SessionPhase::Failed);
            snapshot.nearby_issue = Some(NearbyIssue::ListenerStopped);
        }
    }
}

fn refresh_discovery_result(
    snapshot: &mut AppSnapshot,
    generation: Generation,
    initial_scan_finished: bool,
) {
    if !initial_scan_finished
        || matches!(
            snapshot.discovery.phase(),
            DiscoveryPhase::Failed | DiscoveryPhase::Stopped
        )
    {
        return;
    }
    let phase = if snapshot.peers.values().any(|peer| peer.discovered) {
        DiscoveryPhase::Ready
    } else {
        DiscoveryPhase::Empty
    };
    snapshot.discovery.apply(generation, phase);
}

fn stop_snapshot(
    snapshot: &mut AppSnapshot,
    snapshot_tx: &watch::Sender<Arc<AppSnapshot>>,
    wake: &Arc<dyn Fn() + Send + Sync>,
) {
    snapshot.runtime.begin(RuntimePhase::Stopped);
    snapshot.discovery.begin(DiscoveryPhase::Stopped);
    snapshot.session.begin(SessionPhase::Stopped);
    publish_snapshot(snapshot_tx, snapshot, wake);
    tracing::info!(stage = "shutdown", "macOS runtime owner stopped");
}

fn publish_snapshot(
    snapshot_tx: &watch::Sender<Arc<AppSnapshot>>,
    snapshot: &AppSnapshot,
    wake: &Arc<dyn Fn() + Send + Sync>,
) {
    snapshot_tx.send_replace(Arc::new(snapshot.clone()));
    wake();
}

fn local_device_name() -> Option<String> {
    let value = std::env::var("HOSTNAME").ok()?;
    let name = value
        .chars()
        .filter(|character| !character.is_control())
        .collect::<String>()
        .trim()
        .trim_end_matches(".local")
        .chars()
        .take(80)
        .collect::<String>();
    (!name.is_empty()).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::DialRole;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Phase {
        Idle,
        Starting,
        Ready,
        Failed,
    }

    #[test]
    fn beginning_a_new_operation_advances_generation() {
        let mut lifecycle = Lifecycle::new(Phase::Idle);
        let first = lifecycle.begin(Phase::Starting);
        let second = lifecycle.begin(Phase::Starting);

        assert_ne!(first, second);
    }

    #[test]
    fn stale_events_cannot_override_the_current_phase() {
        let mut lifecycle = Lifecycle::new(Phase::Idle);
        let stale = lifecycle.begin(Phase::Starting);
        let current = lifecycle.begin(Phase::Starting);

        assert!(!lifecycle.apply(stale, Phase::Failed));
        assert!(lifecycle.apply(current, Phase::Ready));
        assert_eq!(lifecycle.phase(), Phase::Ready);
    }

    #[test]
    fn failed_network_start_is_truthful_and_still_shuts_down() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("test runtime")
            .block_on(async {
                let (commands, command_rx) = mpsc::channel(COMMAND_CAPACITY);
                let (snapshot_tx, mut snapshot_rx) =
                    watch::channel(Arc::new(AppSnapshot::default()));
                let owner = tokio::spawn(run(
                    command_rx,
                    snapshot_tx,
                    async { Err(NearbyIssue::LocalNetworkUnavailable) },
                    Arc::new(|| {}),
                ));

                loop {
                    snapshot_rx.changed().await.expect("startup snapshot");
                    if snapshot_rx.borrow().discovery.phase() == DiscoveryPhase::Failed {
                        break;
                    }
                }
                let snapshot = snapshot_rx.borrow().clone();
                assert_eq!(snapshot.runtime.phase(), RuntimePhase::Ready);
                assert_eq!(snapshot.runtime.generation().value(), 1);
                assert_eq!(snapshot.discovery.phase(), DiscoveryPhase::Failed);
                assert_eq!(snapshot.session.phase(), SessionPhase::Failed);
                assert_eq!(snapshot.media.phase(), MediaPhase::Idle);
                assert_eq!(snapshot.capture.phase(), CapabilityPhase::Unavailable);
                assert_eq!(snapshot.decoder.phase(), CapabilityPhase::Unavailable);

                commands
                    .send(RuntimeCommand::Shutdown)
                    .await
                    .expect("shutdown command");
                owner.await.expect("runtime owner task");
            });
    }

    #[test]
    fn shutdown_joins_the_owner_and_publishes_stopped() {
        let mut runtime = RuntimeOwner::start_with(
            || async { Err(NearbyIssue::LocalNetworkUnavailable) },
            || {},
        )
        .expect("runtime starts");
        runtime.shutdown();

        assert_eq!(runtime.snapshot().runtime.phase(), RuntimePhase::Stopped);
        assert_eq!(
            runtime.snapshot().discovery.phase(),
            DiscoveryPhase::Stopped
        );
        assert_eq!(runtime.snapshot().session.phase(), SessionPhase::Stopped);
    }

    #[test]
    fn stale_network_generation_cannot_mutate_snapshot() {
        let mut snapshot = AppSnapshot::default();
        let stale_discovery = snapshot.discovery.begin(DiscoveryPhase::Scanning);
        let stale_session = snapshot.session.begin(SessionPhase::Listening);
        snapshot.discovery.begin(DiscoveryPhase::Starting);
        snapshot.session.begin(SessionPhase::Starting);

        apply_network_event(
            &mut snapshot,
            stale_discovery,
            stale_session,
            NetworkEvent::InboundCount(4),
        );

        assert_eq!(snapshot.inbound_sessions, 0);
    }

    #[test]
    fn lost_presence_does_not_override_a_healthy_session() {
        let mut snapshot = AppSnapshot::default();
        let discovery = snapshot.discovery.begin(DiscoveryPhase::Scanning);
        let session = snapshot.session.begin(SessionPhase::Listening);
        apply_network_event(
            &mut snapshot,
            discovery,
            session,
            NetworkEvent::Peer(network::PeerStatus {
                id: "internal-peer".to_owned(),
                ordinal: 1,
                discovered: false,
                role: DialRole::Active,
                session: PeerSession::Connected,
                transport_generation: Some(3),
            }),
        );

        let peer = snapshot.peers.get("internal-peer").expect("retained peer");
        assert!(!peer.discovered);
        assert_eq!(peer.session, PeerSession::Connected);
    }
}
