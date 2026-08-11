//! Serialized command processing for runtime-owned resources.

use std::time::Duration;

use moq_native::moq_net;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use crate::app::{AppSnapshot, DiscoveredPeer, PeerState, UserCommand};
use crate::network::discovery::{PeerRecord, PeerRegistry};
use crate::network::{peer, server, service};
use crate::publish::session::Publication;

const EVENT_CAPACITY: usize = 64;
const DISCOVERY_RETRY_LIMIT: u8 = 5;
const DISCOVERY_RETRY_BASE_DELAY: Duration = Duration::from_millis(250);
const DISCOVERY_RETRY_MAX_DELAY: Duration = Duration::from_secs(4);

struct DiscoveryRetry {
    attempt: u8,
    delay: Duration,
}

#[derive(Default)]
struct DiscoveryRetryBudget {
    attempts: u8,
}

impl DiscoveryRetryBudget {
    fn next(&mut self, salt: u64) -> Option<DiscoveryRetry> {
        if self.attempts >= DISCOVERY_RETRY_LIMIT {
            return None;
        }

        self.attempts += 1;
        let exponent = u32::from(self.attempts.saturating_sub(1));
        let nominal = DISCOVERY_RETRY_BASE_DELAY
            .saturating_mul(1_u32.checked_shl(exponent).unwrap_or(u32::MAX))
            .min(DISCOVERY_RETRY_MAX_DELAY);
        let spread = nominal / 5;
        let range_ms = spread.as_millis().saturating_mul(2).saturating_add(1);
        let seed = salt
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .wrapping_add(u64::from(self.attempts));
        let jitter_ms = u128::from(seed) % range_ms;
        let delay = nominal
            .saturating_sub(spread)
            .saturating_add(Duration::from_millis(jitter_ms as u64))
            .min(DISCOVERY_RETRY_MAX_DELAY);

        Some(DiscoveryRetry {
            attempt: self.attempts,
            delay,
        })
    }

    fn reset(&mut self) {
        self.attempts = 0;
    }
}

enum Input {
    Command(Option<UserCommand>),
    Service(Option<service::Event>),
    Operation(Option<OperationEvent>),
}

enum LoopAction {
    Changed,
    Unchanged,
    Shutdown,
}

enum OperationEvent {
    ServicesStarted {
        generation: u64,
        result: Result<Box<service::Services>, String>,
    },
    RestartDiscovery {
        generation: u64,
    },
    PeerReady {
        generation: u64,
        peer_id: String,
        result: Result<PeerSession, String>,
    },
    PeerClosed {
        generation: u64,
        error: Option<String>,
    },
    PublishEnded {
        generation: u64,
        result: Result<(), String>,
    },
}

#[derive(Clone)]
enum PeerSession {
    Outbound(moq_native::Connection),
    Inbound(moq_net::Session),
}

impl PeerSession {
    fn close(&self) {
        match self {
            Self::Outbound(connection) => connection.close(),
            Self::Inbound(session) => session.abort(moq_net::Error::Cancel),
        }
    }

    async fn closed_error(&self) -> Option<String> {
        match self {
            Self::Outbound(connection) => connection
                .closed()
                .await
                .err()
                .map(|error| error.to_string()),
            Self::Inbound(session) => Some(session.closed().await.to_string()),
        }
    }

    fn send_bandwidth(&self) -> Option<moq_net::bandwidth::Consumer> {
        match self {
            Self::Outbound(connection) => Some(connection.send_bandwidth()),
            Self::Inbound(session) => session.send_bandwidth(),
        }
    }
}

#[derive(Default)]
struct DiscoveryResources {
    services: Option<service::Services>,
    start: Option<JoinHandle<()>>,
    retry: Option<JoinHandle<()>>,
    retry_budget: DiscoveryRetryBudget,
    generation: u64,
    peers: Option<PeerRegistry>,
}

impl DiscoveryResources {
    fn advance(&mut self) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.generation
    }

    fn cancel_current(&mut self) -> u64 {
        self.advance();
        if let Some(task) = self.start.take() {
            task.abort();
        }
        if let Some(task) = self.retry.take() {
            task.abort();
        }
        self.services = None;
        self.peers = None;
        self.generation
    }

    fn stop(&mut self) {
        self.cancel_current();
        self.retry_budget.reset();
    }
}

#[derive(Default)]
struct PeerResources {
    session: Option<PeerSession>,
    generation: u64,
}

impl PeerResources {
    fn advance(&mut self) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.generation
    }

    fn close(&mut self) {
        self.advance();
        if let Some(session) = self.session.take() {
            session.close();
        }
    }
}

#[derive(Default)]
struct PublishResources {
    task: Option<JoinHandle<()>>,
    generation: u64,
}

impl PublishResources {
    fn advance(&mut self) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.generation
    }

    async fn stop(&mut self) {
        self.advance();
        if let Some(task) = self.task.take() {
            task.abort();
            let _ = task.await;
        }
    }
}

struct Supervisor {
    state: AppSnapshot,
    origin: moq_net::origin::Producer,
    discovery: DiscoveryResources,
    peer: PeerResources,
    publish: PublishResources,
    service_tx: mpsc::Sender<service::Event>,
    service_rx: mpsc::Receiver<service::Event>,
    operation_tx: mpsc::Sender<OperationEvent>,
    operation_rx: mpsc::Receiver<OperationEvent>,
}

impl Supervisor {
    fn new() -> Self {
        let (service_tx, service_rx) = mpsc::channel(EVENT_CAPACITY);
        let (operation_tx, operation_rx) = mpsc::channel(EVENT_CAPACITY);
        Self {
            state: AppSnapshot::default(),
            origin: moq_net::Origin::random().produce(),
            discovery: DiscoveryResources::default(),
            peer: PeerResources::default(),
            publish: PublishResources::default(),
            service_tx,
            service_rx,
            operation_tx,
            operation_rx,
        }
    }

    async fn run(
        mut self,
        mut commands: mpsc::Receiver<UserCommand>,
        snapshots: watch::Sender<AppSnapshot>,
    ) {
        loop {
            let input = tokio::select! {
                command = commands.recv() => Input::Command(command),
                event = self.service_rx.recv() => Input::Service(event),
                event = self.operation_rx.recv() => Input::Operation(event),
            };
            let action = match input {
                Input::Command(Some(command)) => self.handle_command(command).await,
                Input::Service(Some(event)) => self.handle_service_event(event).await,
                Input::Operation(Some(event)) => self.handle_operation_event(event).await,
                Input::Command(None) | Input::Service(None) | Input::Operation(None) => {
                    LoopAction::Shutdown
                }
            };
            match action {
                LoopAction::Changed => {
                    snapshots.send_replace(self.state.clone());
                }
                LoopAction::Unchanged => {}
                LoopAction::Shutdown => break,
            }
        }

        self.publish.stop().await;
        self.peer.close();
        self.discovery.stop();
        tracing::info!("desktop runtime stopped");
    }

    async fn handle_command(&mut self, command: UserCommand) -> LoopAction {
        match command {
            UserCommand::StartDiscovery => self.start_discovery(),
            UserCommand::StopDiscovery => {
                self.discovery.stop();
                self.state.stop_discovery();
                LoopAction::Changed
            }
            UserCommand::ConnectPeer { peer_id } => self.connect(peer_id),
            UserCommand::Disconnect => self.disconnect().await,
            UserCommand::StartScreenShare => self.start_publish(),
            UserCommand::StopScreenShare => self.stop_publish().await,
            UserCommand::Shutdown => LoopAction::Shutdown,
        }
    }

    fn start_discovery(&mut self) -> LoopAction {
        if self.state.discovery.is_active() {
            self.state.last_error = Some("LAN discovery is already active.".into());
            return LoopAction::Changed;
        }

        self.state.start_discovery();
        self.state.peers.clear();
        self.discovery.stop();
        self.launch_services();
        LoopAction::Changed
    }

    fn launch_services(&mut self) {
        let generation = self.discovery.generation;
        let operation_events = self.operation_tx.clone();
        self.discovery.start = Some(tokio::spawn(async move {
            let result = service::Services::start()
                .await
                .map(Box::new)
                .map_err(|error| error.to_string());
            let _ = operation_events
                .send(OperationEvent::ServicesStarted { generation, result })
                .await;
        }));
    }

    fn connect(&mut self, peer_id: String) -> LoopAction {
        let Some(record) = self
            .discovery
            .peers
            .as_ref()
            .and_then(|peers| peers.get(&peer_id))
            .cloned()
        else {
            self.state.last_error = Some("The selected peer is no longer available.".into());
            return LoopAction::Changed;
        };
        if let Err(error) = self.state.begin_connect(peer_id.clone()) {
            self.state.last_error = Some(error.to_string());
            return LoopAction::Changed;
        }

        let generation = self.peer.advance();
        let origin = self.origin.clone();
        let events = self.operation_tx.clone();
        tokio::spawn(async move {
            let result = match peer::dial(&record, origin) {
                Ok(connection) => connection
                    .established()
                    .await
                    .map(PeerSession::Outbound)
                    .map_err(|error| error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            let _ = events
                .send(OperationEvent::PeerReady {
                    generation,
                    peer_id,
                    result,
                })
                .await;
        });
        LoopAction::Changed
    }

    async fn disconnect(&mut self) -> LoopAction {
        if let Err(error) = self.state.begin_disconnect() {
            self.state.last_error = Some(error.to_string());
            return LoopAction::Changed;
        }
        self.publish.stop().await;
        self.peer.close();
        self.state
            .finish_disconnect()
            .expect("disconnect was just started");
        LoopAction::Changed
    }

    fn start_publish(&mut self) -> LoopAction {
        if let Err(error) = self.state.begin_publish() {
            self.state.last_error = Some(error.to_string());
            return LoopAction::Changed;
        }

        let bandwidth = self
            .peer
            .session
            .as_ref()
            .and_then(PeerSession::send_bandwidth);
        let publication = match Publication::prepare(&self.origin, bandwidth) {
            Ok(publication) => publication,
            Err(error) => {
                self.state
                    .fail_publish(error.to_string())
                    .expect("publication preparation was active");
                return LoopAction::Changed;
            }
        };

        let generation = self.publish.advance();
        let events = self.operation_tx.clone();
        self.publish.task = Some(tokio::spawn(async move {
            let result = publication.run().await.map_err(|error| error.to_string());
            let _ = events
                .send(OperationEvent::PublishEnded { generation, result })
                .await;
        }));
        self.state
            .finish_publish()
            .expect("publication preparation was active");
        LoopAction::Changed
    }

    async fn stop_publish(&mut self) -> LoopAction {
        if let Err(error) = self.state.begin_stop_publish() {
            self.state.last_error = Some(error.to_string());
            return LoopAction::Changed;
        }
        self.publish.stop().await;
        self.state
            .finish_stop_publish()
            .expect("publication was stopping");
        LoopAction::Changed
    }

    async fn handle_service_event(&mut self, event: service::Event) -> LoopAction {
        if event.generation != self.discovery.generation {
            return LoopAction::Unchanged;
        }
        match event.kind {
            service::EventKind::Found(peer) => {
                let Some(peers) = self.discovery.peers.as_mut() else {
                    return LoopAction::Unchanged;
                };
                if peers.found(PeerRecord::from_mdns(peer)) {
                    self.project_peers();
                    LoopAction::Changed
                } else {
                    LoopAction::Unchanged
                }
            }
            service::EventKind::Lost(id) => {
                let Some(peers) = self.discovery.peers.as_mut() else {
                    return LoopAction::Unchanged;
                };
                if peers.lost(&id) {
                    self.project_peers();
                    LoopAction::Changed
                } else {
                    LoopAction::Unchanged
                }
            }
            service::EventKind::InitialScanFinished => {
                let stabilized = self.discovery.retry_budget.attempts > 0;
                self.discovery.retry_budget.reset();
                let previous = self.state.discovery.clone();
                self.state.finish_initial_scan();
                if stabilized {
                    tracing::info!(stage = "discovery", "LAN discovery recovery stabilized");
                }
                if self.state.discovery == previous {
                    LoopAction::Unchanged
                } else {
                    LoopAction::Changed
                }
            }
            service::EventKind::DiscoveryStopped => {
                self.recover_discovery("LAN discovery stopped unexpectedly.")
            }
            service::EventKind::ListenerStopped => {
                self.recover_discovery("The LAN peer listener stopped unexpectedly.")
            }
            service::EventKind::Inbound(request) => self.accept_inbound(request).await,
        }
    }

    async fn accept_inbound(&mut self, request: moq_native::Request) -> LoopAction {
        let Some(credential) = self
            .discovery
            .services
            .as_ref()
            .map(|services| services.credential.clone())
        else {
            tracing::warn!(
                stage = "discovery",
                "rejecting inbound peer while LAN services are unavailable"
            );
            request.close(503).await.ok();
            return LoopAction::Unchanged;
        };
        if !server::authorized_request(&request, &credential) {
            let peer_id = server::incoming_peer_id(&request);
            tracing::warn!(
                stage = "auth",
                peer_id,
                "rejecting unauthorized inbound peer"
            );
            request.close(403).await.ok();
            return LoopAction::Unchanged;
        }
        if !matches!(
            self.state.peer,
            PeerState::Disconnected | PeerState::Failed { .. }
        ) {
            let peer_id = server::incoming_peer_id(&request);
            tracing::info!(
                stage = "transport",
                peer_id,
                "rejecting a second peer session"
            );
            request.close(409).await.ok();
            return LoopAction::Unchanged;
        }

        let peer_id = server::incoming_peer_id(&request);
        self.state
            .begin_connect(peer_id.clone())
            .expect("peer state was checked");
        let generation = self.peer.advance();
        let origin = self.origin.clone();
        let events = self.operation_tx.clone();
        tokio::spawn(async move {
            let result = server::accept(request, &credential, origin)
                .await
                .map(PeerSession::Inbound)
                .map_err(|error| error.to_string());
            let _ = events
                .send(OperationEvent::PeerReady {
                    generation,
                    peer_id,
                    result,
                })
                .await;
        });
        LoopAction::Changed
    }

    async fn handle_operation_event(&mut self, event: OperationEvent) -> LoopAction {
        match event {
            OperationEvent::ServicesStarted { generation, result } => {
                if generation != self.discovery.generation || !self.state.discovery.is_active() {
                    return LoopAction::Unchanged;
                }
                self.discovery.start = None;
                match result {
                    Ok(mut services) => {
                        let recovered = self.discovery.retry_budget.attempts > 0;
                        self.discovery.peers = Some(PeerRegistry::new(services.local_id.clone()));
                        services.activate(generation, self.service_tx.clone());
                        self.discovery.services = Some(*services);
                        if recovered {
                            tracing::info!(stage = "discovery", "LAN discovery restarted");
                        }
                        LoopAction::Unchanged
                    }
                    Err(error) => self.recover_discovery(error),
                }
            }
            OperationEvent::RestartDiscovery { generation } => {
                if generation != self.discovery.generation || !self.state.discovery.is_active() {
                    return LoopAction::Unchanged;
                }
                self.discovery.retry = None;
                self.launch_services();
                LoopAction::Unchanged
            }
            OperationEvent::PeerReady {
                generation,
                peer_id,
                result,
            } => {
                if generation != self.peer.generation || !self.is_connecting(&peer_id) {
                    if let Ok(session) = result {
                        session.close();
                    }
                    return LoopAction::Unchanged;
                }
                match result {
                    Ok(session) => {
                        self.state.finish_connect().expect("peer is connecting");
                        watch_peer(generation, session.clone(), self.operation_tx.clone());
                        self.peer.session = Some(session);
                    }
                    Err(error) => {
                        tracing::warn!(
                            stage = "transport",
                            peer_id,
                            error,
                            "peer connection failed"
                        );
                        self.state.fail_connect(error).expect("peer is connecting");
                    }
                }
                LoopAction::Changed
            }
            OperationEvent::PeerClosed { generation, error } => {
                if generation != self.peer.generation {
                    return LoopAction::Unchanged;
                }
                self.publish.stop().await;
                self.state.disconnect();
                if let Some(error) = &error {
                    tracing::warn!(stage = "transport", error, "peer session closed");
                } else {
                    tracing::info!(stage = "transport", "peer session closed");
                }
                self.state.last_error = error;
                self.peer.session = None;
                LoopAction::Changed
            }
            OperationEvent::PublishEnded { generation, result } => {
                if generation != self.publish.generation {
                    return LoopAction::Unchanged;
                }
                self.publish.task = None;
                match result {
                    Ok(()) => {
                        self.state.end_publish().expect("current publication ended");
                    }
                    Err(error) => {
                        tracing::warn!(stage = "publish", error, "screen publication ended");
                        self.state
                            .fail_publish(error)
                            .expect("current publication failed");
                    }
                }
                LoopAction::Changed
            }
        }
    }

    fn is_connecting(&self, expected: &str) -> bool {
        matches!(&self.state.peer, PeerState::Connecting { peer_id } if peer_id == expected)
    }

    fn recover_discovery(&mut self, error: impl Into<String>) -> LoopAction {
        let error = error.into();
        let generation = self.discovery.cancel_current();
        self.state.start_discovery();
        self.state.peers.clear();

        let Some(retry) = self.discovery.retry_budget.next(generation) else {
            tracing::error!(
                stage = "discovery",
                error,
                attempts = DISCOVERY_RETRY_LIMIT,
                "LAN discovery recovery exhausted"
            );
            self.state.fail_discovery(format!(
                "{error} Automatic recovery stopped after {DISCOVERY_RETRY_LIMIT} attempts."
            ));
            return LoopAction::Changed;
        };

        tracing::warn!(
            stage = "discovery",
            error,
            attempt = retry.attempt,
            delay_ms = retry.delay.as_millis() as u64,
            "scheduling LAN discovery recovery"
        );
        let events = self.operation_tx.clone();
        self.discovery.retry = Some(tokio::spawn(async move {
            tokio::time::sleep(retry.delay).await;
            let _ = events
                .send(OperationEvent::RestartDiscovery { generation })
                .await;
        }));
        LoopAction::Changed
    }

    fn project_peers(&mut self) {
        let peers = self
            .discovery
            .peers
            .as_ref()
            .expect("peer registry exists while discovery events are handled")
            .values()
            .map(|peer| DiscoveredPeer {
                id: peer.id.clone(),
                name: peer.id.clone(),
                endpoints: peer.endpoint_labels(),
                fingerprint_pinned: peer.fingerprint.is_some(),
            })
            .collect();
        self.state.replace_peers(peers);
    }
}

pub(super) async fn run(
    commands: mpsc::Receiver<UserCommand>,
    snapshots: watch::Sender<AppSnapshot>,
) {
    Supervisor::new().run(commands, snapshots).await;
}

fn watch_peer(generation: u64, session: PeerSession, events: mpsc::Sender<OperationEvent>) {
    tokio::spawn(async move {
        let error = session.closed_error().await;
        let _ = events
            .send(OperationEvent::PeerClosed { generation, error })
            .await;
    });
}

#[cfg(test)]
mod tests {
    use crate::app::{DiscoveryState, UserCommand};

    use crate::network::service;

    use super::{DISCOVERY_RETRY_LIMIT, DiscoveryRetryBudget, Supervisor};

    #[test]
    fn discovery_retry_budget_is_bounded_and_resets_after_success() {
        let mut retries = DiscoveryRetryBudget::default();

        for attempt in 1..=DISCOVERY_RETRY_LIMIT {
            let retry = retries
                .next(7)
                .expect("retry budget should allow the configured attempt");
            assert_eq!(retry.attempt, attempt);
        }
        assert!(retries.next(7).is_none());

        retries.reset();
        assert_eq!(retries.next(7).expect("reset restores budget").attempt, 1);
    }

    #[test]
    fn discovery_retry_delay_is_jittered_exponential_and_capped() {
        let mut retries = DiscoveryRetryBudget::default();
        let mut previous = std::time::Duration::ZERO;

        while let Some(retry) = retries.next(11) {
            assert!(retry.delay >= previous);
            assert!(retry.delay <= super::DISCOVERY_RETRY_MAX_DELAY);
            previous = retry.delay;
        }
    }

    #[tokio::test]
    async fn stopping_discovery_cancels_a_scheduled_recovery() {
        let mut supervisor = Supervisor::new();
        supervisor.state.start_discovery();
        supervisor.recover_discovery("network changed");

        assert!(supervisor.discovery.retry.is_some());
        assert_eq!(supervisor.state.discovery, DiscoveryState::Scanning);
        assert!(supervisor.state.last_error.is_none());

        supervisor.handle_command(UserCommand::StopDiscovery).await;

        assert!(supervisor.discovery.retry.is_none());
        assert_eq!(supervisor.discovery.retry_budget.attempts, 0);
        assert_eq!(supervisor.state.discovery, DiscoveryState::Idle);
    }

    #[tokio::test]
    async fn exhausted_discovery_recovery_becomes_a_terminal_error() {
        let mut supervisor = Supervisor::new();
        supervisor.state.start_discovery();

        for _ in 0..=DISCOVERY_RETRY_LIMIT {
            supervisor.recover_discovery("network unavailable");
        }

        assert!(supervisor.discovery.retry.is_none());
        assert_eq!(supervisor.state.discovery, DiscoveryState::Error);
        assert!(
            supervisor
                .state
                .last_error
                .as_deref()
                .is_some_and(|error| error.contains("stopped after 5 attempts"))
        );
    }

    #[tokio::test]
    async fn a_stable_discovery_generation_resets_the_recovery_budget() {
        let mut supervisor = Supervisor::new();
        supervisor.state.start_discovery();
        supervisor.recover_discovery("network changed");
        supervisor
            .discovery
            .retry
            .take()
            .expect("recovery is scheduled")
            .abort();

        supervisor
            .handle_service_event(service::Event {
                generation: supervisor.discovery.generation,
                kind: service::EventKind::InitialScanFinished,
            })
            .await;

        assert_eq!(supervisor.discovery.retry_budget.attempts, 0);
        assert_eq!(supervisor.state.discovery, DiscoveryState::Empty);
    }
}
