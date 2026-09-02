//! Serialized command processing for runtime-owned resources.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use moq_tokio::moq_net;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

#[cfg(target_os = "linux")]
use crate::app::MediaState;
use crate::app::{
    AppSnapshot, DialRole, DiscoveredPeer, RemoteAudioSnapshot, TransportState, UserCommand,
};
use crate::network::discovery::{PeerRecord, PeerRegistry, PeerUpdate};
use crate::network::{peer, server, service};
use crate::publish::session::Publication;

use super::PlaybackFrame;

const EVENT_CAPACITY: usize = 64;
const DISCOVERY_RETRY_LIMIT: u8 = 5;
const DISCOVERY_RETRY_BASE_DELAY: Duration = Duration::from_millis(250);
const DISCOVERY_RETRY_MAX_DELAY: Duration = Duration::from_secs(4);
const PEER_RETRY_LIMIT: u8 = 5;
const PEER_RETRY_BASE_DELAY: Duration = Duration::from_millis(500);
const PEER_RETRY_MAX_DELAY: Duration = Duration::from_secs(4);

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
        next_retry(
            &mut self.attempts,
            DISCOVERY_RETRY_LIMIT,
            DISCOVERY_RETRY_BASE_DELAY,
            DISCOVERY_RETRY_MAX_DELAY,
            salt,
        )
    }

    fn reset(&mut self) {
        self.attempts = 0;
    }
}

#[derive(Default)]
struct PeerRetryBudget {
    attempts: u8,
}

impl PeerRetryBudget {
    fn next(&mut self, salt: u64) -> Option<DiscoveryRetry> {
        next_retry(
            &mut self.attempts,
            PEER_RETRY_LIMIT,
            PEER_RETRY_BASE_DELAY,
            PEER_RETRY_MAX_DELAY,
            salt,
        )
    }

    fn reset(&mut self) {
        self.attempts = 0;
    }
}

fn next_retry(
    attempts: &mut u8,
    limit: u8,
    base_delay: Duration,
    max_delay: Duration,
    salt: u64,
) -> Option<DiscoveryRetry> {
    if *attempts >= limit {
        return None;
    }

    *attempts += 1;
    let exponent = u32::from(attempts.saturating_sub(1));
    let nominal = base_delay
        .saturating_mul(1_u32.checked_shl(exponent).unwrap_or(u32::MAX))
        .min(max_delay);
    let spread = nominal / 5;
    let range_ms = spread.as_millis().saturating_mul(2).saturating_add(1);
    let seed = salt
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(u64::from(*attempts));
    let jitter_ms = u128::from(seed) % range_ms;
    let delay = nominal
        .saturating_sub(spread)
        .saturating_add(Duration::from_millis(jitter_ms as u64))
        .min(max_delay);

    Some(DiscoveryRetry {
        attempt: *attempts,
        delay,
    })
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

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum SessionKey {
    Outbound(String),
    Inbound(u64),
}

enum OperationEvent {
    ServicesStarted {
        generation: u64,
        result: Result<Box<service::Services>, String>,
    },
    RestartDiscovery {
        generation: u64,
    },
    RetryOutbound {
        peer_id: String,
        generation: u64,
    },
    SessionReady {
        key: SessionKey,
        generation: u64,
        result: Result<PeerSession, String>,
    },
    SessionClosed {
        key: SessionKey,
        generation: u64,
        error: Option<String>,
    },
    ScreenAnnouncement {
        path: String,
        broadcast: Option<moq_net::broadcast::Consumer>,
    },
    PublishEnded {
        generation: u64,
        result: Result<(), String>,
    },
    #[cfg(target_os = "linux")]
    ViewStarted {
        generation: u64,
        path: String,
    },
    ViewAudioChanged {
        generation: u64,
        path: String,
        audio: RemoteAudioSnapshot,
    },
    ViewEnded {
        generation: u64,
        result: Result<(), String>,
    },
}

#[derive(Clone)]
enum PeerSession {
    Outbound(moq_tokio::Connection),
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
    pending: Option<JoinHandle<()>>,
    retry_budget: PeerRetryBudget,
    generation: u64,
}

impl PeerResources {
    fn advance(&mut self) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.generation
    }

    fn close(&mut self) {
        self.advance();
        if let Some(task) = self.pending.take() {
            task.abort();
        }
        if let Some(session) = self.session.take() {
            session.close();
        }
    }

    fn reset(&mut self) {
        self.close();
        self.retry_budget.reset();
    }
}

#[derive(Default)]
struct MeshResources {
    outbound: HashMap<String, PeerResources>,
    inbound: HashMap<u64, PeerResources>,
    next_inbound_id: u64,
}

impl MeshResources {
    fn get_mut(&mut self, key: &SessionKey) -> Option<&mut PeerResources> {
        match key {
            SessionKey::Outbound(peer_id) => self.outbound.get_mut(peer_id),
            SessionKey::Inbound(id) => self.inbound.get_mut(id),
        }
    }

    fn remove(&mut self, key: &SessionKey) -> Option<PeerResources> {
        match key {
            SessionKey::Outbound(peer_id) => self.outbound.remove(peer_id),
            SessionKey::Inbound(id) => self.inbound.remove(id),
        }
    }

    fn next_inbound(&mut self) -> SessionKey {
        self.next_inbound_id = self.next_inbound_id.wrapping_add(1);
        SessionKey::Inbound(self.next_inbound_id)
    }

    fn connected_inbound_count(&self) -> usize {
        self.inbound
            .values()
            .filter(|peer| peer.session.is_some())
            .count()
    }

    fn close_inbound(&mut self) -> usize {
        let count = self.connected_inbound_count();
        for resources in self.inbound.values_mut() {
            resources.close();
        }
        self.inbound.clear();
        count
    }

    fn close_all(&mut self) {
        for peer in self.outbound.values_mut().chain(self.inbound.values_mut()) {
            peer.close();
        }
        self.outbound.clear();
        self.inbound.clear();
    }
}

#[derive(Default)]
struct TaskResources {
    task: Option<JoinHandle<()>>,
    generation: u64,
}

#[derive(Default)]
struct ViewResources {
    task: Option<JoinHandle<()>>,
    cancel: Option<watch::Sender<bool>>,
    generation: u64,
}

impl ViewResources {
    fn advance(&mut self) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.generation
    }

    async fn stop(&mut self) {
        self.advance();
        if let Some(cancel) = self.cancel.take() {
            cancel.send_replace(true);
        }
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl TaskResources {
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
    publish_origin: moq_net::origin::Producer,
    publish_origin_driver: JoinHandle<()>,
    receive_origin: moq_net::origin::Producer,
    receive_origin_driver: JoinHandle<()>,
    discovery: DiscoveryResources,
    mesh: MeshResources,
    remote_screens: HashMap<String, moq_net::broadcast::Consumer>,
    publish: TaskResources,
    view: ViewResources,
    announcements: JoinHandle<()>,
    playback_tx: watch::Sender<Option<Arc<PlaybackFrame>>>,
    service_tx: mpsc::Sender<service::Event>,
    service_rx: mpsc::Receiver<service::Event>,
    operation_tx: mpsc::Sender<OperationEvent>,
    operation_rx: mpsc::Receiver<OperationEvent>,
}

impl Supervisor {
    fn new(playback_tx: watch::Sender<Option<Arc<PlaybackFrame>>>) -> Self {
        let (service_tx, service_rx) = mpsc::channel(EVENT_CAPACITY);
        let (operation_tx, operation_rx) = mpsc::channel(EVENT_CAPACITY);
        let (publish_origin, publish_origin_driver) =
            moq_net::origin::Producer::new(moq_net::origin::Info::new(moq_net::Origin::random()));
        let publish_origin_driver = tokio::spawn(publish_origin_driver);
        let (receive_origin, receive_origin_driver) =
            moq_net::origin::Producer::new(moq_net::origin::Info::new(moq_net::Origin::random()));
        let receive_origin_driver = tokio::spawn(receive_origin_driver);
        let announcements = watch_announcements(receive_origin.clone(), operation_tx.clone());
        Self {
            state: AppSnapshot::default(),
            publish_origin,
            publish_origin_driver,
            receive_origin,
            receive_origin_driver,
            discovery: DiscoveryResources::default(),
            mesh: MeshResources::default(),
            remote_screens: HashMap::new(),
            publish: TaskResources::default(),
            view: ViewResources::default(),
            announcements,
            playback_tx,
            service_tx,
            service_rx,
            operation_tx,
            operation_rx,
        }
    }

    async fn run(
        mut self,
        mut commands: mpsc::Receiver<UserCommand>,
        snapshots: watch::Sender<Arc<AppSnapshot>>,
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
                    snapshots.send_replace(Arc::new(self.state.clone()));
                }
                LoopAction::Unchanged => {}
                LoopAction::Shutdown => break,
            }
        }

        self.view.stop().await;
        self.publish.stop().await;
        self.mesh.close_all();
        self.discovery.stop();
        self.announcements.abort();
        let _ = self.announcements.await;
        self.remote_screens.clear();
        self.playback_tx.send_replace(None);
        drop(self.publish_origin);
        drop(self.receive_origin);
        let _ = self.publish_origin_driver.await;
        let _ = self.receive_origin_driver.await;
        tracing::info!(stage = "runtime", "desktop runtime stopped");
    }

    async fn handle_command(&mut self, command: UserCommand) -> LoopAction {
        match command {
            UserCommand::StartDiscovery => self.start_discovery(),
            UserCommand::StopDiscovery => {
                self.close_inbound("LAN services stopped by user");
                self.discovery.stop();
                self.state.stop_discovery();
                LoopAction::Changed
            }
            UserCommand::RetryDiscovery => self.restart_discovery(),
            UserCommand::StartScreenShare { system_audio } => self.start_publish(system_audio),
            UserCommand::StopScreenShare => self.stop_publish().await,
            UserCommand::StartWatching { path } => self.start_view(path),
            UserCommand::StopWatching => self.stop_view().await,
            UserCommand::Shutdown => LoopAction::Shutdown,
        }
    }

    fn start_discovery(&mut self) -> LoopAction {
        if self.state.discovery.is_active() {
            self.state.last_error = Some("LAN discovery is already active.".into());
            return LoopAction::Changed;
        }

        self.close_inbound("LAN services restarted by user");
        self.state.start_discovery();
        self.discovery.stop();
        self.launch_services();
        LoopAction::Changed
    }

    fn restart_discovery(&mut self) -> LoopAction {
        self.close_inbound("LAN services restarted after user retry");
        self.discovery.stop();
        self.state.stop_discovery();
        self.state.start_discovery();
        self.launch_services();
        tracing::info!(stage = "discovery", "LAN discovery restarted by user");
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

    fn ensure_outbound(&mut self, peer_id: &str, reset: bool) {
        let Some(record) = self
            .discovery
            .peers
            .as_ref()
            .and_then(|peers| peers.get(peer_id))
            .cloned()
        else {
            return;
        };
        if !reset
            && self
                .mesh
                .outbound
                .get(peer_id)
                .is_some_and(|peer| peer.session.is_some())
        {
            return;
        }

        let peer_resources = self.mesh.outbound.entry(peer_id.to_owned()).or_default();
        if reset {
            peer_resources.reset();
        } else {
            peer_resources.close();
        }
        let generation = peer_resources.generation;
        self.state
            .set_transport(peer_id, TransportState::Connecting);
        let key = SessionKey::Outbound(peer_id.to_owned());
        let publish_origin = self.publish_origin.clone();
        let receive_origin = self.receive_origin.clone();
        let events = self.operation_tx.clone();
        peer_resources.pending = Some(tokio::spawn(async move {
            let result = match peer::dial(&record, &publish_origin, receive_origin) {
                Ok(connection) => connection
                    .established()
                    .await
                    .map(PeerSession::Outbound)
                    .map_err(|error| error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            let _ = events
                .send(OperationEvent::SessionReady {
                    key,
                    generation,
                    result,
                })
                .await;
        }));
    }

    fn start_publish(&mut self, system_audio: bool) -> LoopAction {
        if let Err(error) = self.state.begin_publish() {
            self.state.last_error = Some(error.to_string());
            return LoopAction::Changed;
        }
        let Some(local_peer_id) = self.state.local_peer_id.clone() else {
            self.state
                .fail_publish("LAN services are not ready.")
                .expect("publication preparation was active");
            return LoopAction::Changed;
        };

        let publication =
            match Publication::prepare(&self.publish_origin, &local_peer_id, None, system_audio) {
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

    fn start_view(&mut self, path: String) -> LoopAction {
        if let Err(error) = self.state.begin_view(&path) {
            self.state.last_error = Some(error.to_string());
            return LoopAction::Changed;
        }
        let Some(broadcast) = self.remote_screens.get(&path).cloned() else {
            self.state
                .fail_view("The selected screen is no longer available.")
                .expect("remote playback preparation was active");
            return LoopAction::Changed;
        };

        let generation = self.view.advance();
        let events = self.operation_tx.clone();
        let frames = self.playback_tx.clone();
        let (cancel, cancelled) = watch::channel(false);
        self.view.cancel = Some(cancel);
        self.view.task = Some(tokio::spawn(run_view(
            generation, path, broadcast, cancelled, events, frames,
        )));
        LoopAction::Changed
    }

    async fn stop_view(&mut self) -> LoopAction {
        if let Err(error) = self.state.begin_stop_view() {
            self.state.last_error = Some(error.to_string());
            return LoopAction::Changed;
        }
        self.view.stop().await;
        self.playback_tx.send_replace(None);
        self.state
            .finish_stop_view()
            .expect("remote playback was stopping");
        LoopAction::Changed
    }

    async fn handle_service_event(&mut self, event: service::Event) -> LoopAction {
        if event.generation != self.discovery.generation {
            return LoopAction::Unchanged;
        }
        match event.kind {
            service::EventKind::Found { peer, should_dial } => {
                let record = PeerRecord::from_mdns(peer);
                let peer_id = record.id.clone();
                let Some(peers) = self.discovery.peers.as_mut() else {
                    return LoopAction::Unchanged;
                };
                let update = peers.found(record);
                self.project_peer(&peer_id, should_dial);
                if should_dial {
                    let active = self.mesh.outbound.get(&peer_id).is_some_and(|resources| {
                        resources.session.is_some() || resources.pending.is_some()
                    });
                    self.ensure_outbound(&peer_id, reset_outbound_for(update, active));
                } else {
                    self.accept_inbound_role(&peer_id);
                }
                LoopAction::Changed
            }
            service::EventKind::Lost(id) => {
                let Some(peers) = self.discovery.peers.as_mut() else {
                    return LoopAction::Unchanged;
                };
                if peers.lost(&id) {
                    self.state.mark_peer_lost(&id);
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

    async fn accept_inbound(&mut self, request: moq_tokio::Request) -> LoopAction {
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
            tracing::warn!(stage = "auth", "rejecting unauthorized inbound peer");
            request.close(403).await.ok();
            return LoopAction::Unchanged;
        }

        let key = self.mesh.next_inbound();
        let SessionKey::Inbound(id) = key else {
            unreachable!()
        };
        let resources = self.mesh.inbound.entry(id).or_default();
        let generation = resources.advance();
        let publish_origin = self.publish_origin.clone();
        let receive_origin = self.receive_origin.clone();
        let events = self.operation_tx.clone();
        resources.pending = Some(tokio::spawn(async move {
            let result = server::accept(request, &credential, &publish_origin, receive_origin)
                .await
                .map(PeerSession::Inbound)
                .map_err(|error| error.to_string());
            let _ = events
                .send(OperationEvent::SessionReady {
                    key: SessionKey::Inbound(id),
                    generation,
                    result,
                })
                .await;
        }));
        LoopAction::Unchanged
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
                        self.state.local_peer_id = Some(services.local_id.clone());
                        self.discovery.peers = Some(PeerRegistry::new(services.local_id.clone()));
                        services.activate(generation, self.service_tx.clone());
                        self.discovery.services = Some(*services);
                        if recovered {
                            tracing::info!(stage = "discovery", "LAN discovery restarted");
                        }
                        LoopAction::Changed
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
            OperationEvent::RetryOutbound {
                peer_id,
                generation,
            } => {
                if self
                    .mesh
                    .outbound
                    .get(&peer_id)
                    .is_none_or(|resources| resources.generation != generation)
                {
                    return LoopAction::Unchanged;
                }
                self.ensure_outbound(&peer_id, false);
                LoopAction::Changed
            }
            OperationEvent::SessionReady {
                key,
                generation,
                result,
            } => self.session_ready(key, generation, result),
            OperationEvent::SessionClosed {
                key,
                generation,
                error,
            } => self.session_closed(key, generation, error),
            OperationEvent::ScreenAnnouncement { path, broadcast } => {
                let available = broadcast.is_some();
                let Some(peer_id) = crate::screen_path::peer_id(&path) else {
                    return LoopAction::Unchanged;
                };
                if self.state.local_peer_id.as_deref() == Some(peer_id) {
                    return LoopAction::Unchanged;
                }
                let changed = self.state.update_remote_screen(path.clone(), available);
                if let Some(broadcast) = broadcast {
                    self.remote_screens.insert(path.clone(), broadcast);
                } else {
                    self.remote_screens.remove(&path);
                }
                if changed {
                    LoopAction::Changed
                } else {
                    LoopAction::Unchanged
                }
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
            #[cfg(target_os = "linux")]
            OperationEvent::ViewStarted { generation, path } => {
                if generation != self.view.generation
                    || !matches!(&self.state.media, MediaState::PreparingView { path: current } if current == &path)
                {
                    return LoopAction::Unchanged;
                }
                self.state
                    .finish_view()
                    .expect("remote playback was preparing");
                LoopAction::Changed
            }
            OperationEvent::ViewAudioChanged {
                generation,
                path,
                audio,
            } => {
                if generation != self.view.generation {
                    return LoopAction::Unchanged;
                }
                if self.state.set_remote_audio(&path, audio) {
                    LoopAction::Changed
                } else {
                    LoopAction::Unchanged
                }
            }
            OperationEvent::ViewEnded { generation, result } => {
                if generation != self.view.generation {
                    return LoopAction::Unchanged;
                }
                self.view.task = None;
                self.view.cancel = None;
                self.playback_tx.send_replace(None);
                match result {
                    Ok(()) => self.state.end_view().expect("remote playback ended"),
                    Err(error) => {
                        tracing::warn!(stage = "playback", error, "remote screen playback ended");
                        self.state.fail_view(error).expect("remote playback failed");
                    }
                }
                LoopAction::Changed
            }
        }
    }

    fn session_ready(
        &mut self,
        key: SessionKey,
        generation: u64,
        result: Result<PeerSession, String>,
    ) -> LoopAction {
        let Some(resources) = self
            .mesh
            .get_mut(&key)
            .filter(|resources| generation == resources.generation)
        else {
            if let Ok(session) = result {
                session.close();
            }
            return LoopAction::Unchanged;
        };
        resources.pending = None;

        match result {
            Ok(session) => {
                watch_session(
                    key.clone(),
                    generation,
                    session.clone(),
                    self.operation_tx.clone(),
                );
                resources.session = Some(session);
                match key {
                    SessionKey::Outbound(peer_id) => {
                        resources.retry_budget.reset();
                        self.state
                            .set_transport(&peer_id, TransportState::Connected);
                        tracing::info!(stage = "transport", peer_id, "mesh peer connected");
                    }
                    SessionKey::Inbound(_) => {
                        self.state
                            .set_inbound_session_count(self.mesh.connected_inbound_count());
                        tracing::info!(
                            stage = "transport",
                            inbound_sessions = self.mesh.connected_inbound_count(),
                            "authorized inbound mesh session connected"
                        );
                    }
                }
            }
            Err(error) => match key {
                SessionKey::Outbound(peer_id) => {
                    self.state.set_transport(&peer_id, TransportState::Failed);
                    tracing::warn!(
                        stage = "transport",
                        peer_id,
                        error,
                        "mesh peer connection failed"
                    );
                    self.schedule_outbound_retry(&peer_id);
                }
                SessionKey::Inbound(_) => {
                    self.mesh.remove(&key);
                    self.state
                        .set_inbound_session_count(self.mesh.connected_inbound_count());
                    tracing::warn!(stage = "transport", error, "inbound mesh session failed");
                }
            },
        }
        LoopAction::Changed
    }

    fn session_closed(
        &mut self,
        key: SessionKey,
        generation: u64,
        error: Option<String>,
    ) -> LoopAction {
        if self
            .mesh
            .get_mut(&key)
            .is_none_or(|resources| generation != resources.generation)
        {
            return LoopAction::Unchanged;
        }
        match key {
            SessionKey::Outbound(peer_id) => {
                if let Some(resources) = self.mesh.outbound.get_mut(&peer_id) {
                    resources.session = None;
                }
                self.state.set_transport(&peer_id, TransportState::Failed);
                if let Some(error) = error {
                    tracing::warn!(
                        stage = "transport",
                        peer_id,
                        error,
                        "mesh peer session closed"
                    );
                } else {
                    tracing::info!(stage = "transport", peer_id, "mesh peer session closed");
                }
                self.schedule_outbound_retry(&peer_id);
            }
            SessionKey::Inbound(_) => {
                self.mesh.remove(&key);
                self.state
                    .set_inbound_session_count(self.mesh.connected_inbound_count());
                if let Some(error) = error {
                    tracing::warn!(stage = "transport", error, "inbound mesh session closed");
                } else {
                    tracing::info!(stage = "transport", "inbound mesh session closed");
                }
            }
        }
        LoopAction::Changed
    }

    fn schedule_outbound_retry(&mut self, peer_id: &str) {
        if self
            .discovery
            .peers
            .as_ref()
            .and_then(|peers| peers.get(peer_id))
            .is_none()
        {
            self.mesh.outbound.remove(peer_id);
            return;
        }

        let Some(resources) = self.mesh.outbound.get_mut(peer_id) else {
            return;
        };
        let Some(retry) = resources.retry_budget.next(resources.generation) else {
            tracing::warn!(
                stage = "transport",
                peer_id,
                attempts = PEER_RETRY_LIMIT,
                "mesh peer recovery stopped after retry budget"
            );
            return;
        };
        let generation = resources.advance();
        tracing::warn!(
            stage = "transport",
            peer_id,
            attempt = retry.attempt,
            delay_ms = retry.delay.as_millis() as u64,
            "scheduling mesh peer recovery"
        );
        let events = self.operation_tx.clone();
        let peer_id = peer_id.to_owned();
        resources.pending = Some(tokio::spawn(async move {
            tokio::time::sleep(retry.delay).await;
            let _ = events
                .send(OperationEvent::RetryOutbound {
                    peer_id,
                    generation,
                })
                .await;
        }));
    }

    fn recover_discovery(&mut self, error: impl Into<String>) -> LoopAction {
        let error = error.into();
        self.close_inbound("LAN listener is restarting");
        let generation = self.discovery.cancel_current();
        self.state.stop_discovery();
        self.state.start_discovery();

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

    fn close_inbound(&mut self, reason: &'static str) {
        let closed = self.mesh.close_inbound();
        self.state.set_inbound_session_count(0);
        if closed > 0 {
            tracing::info!(
                stage = "transport",
                inbound_sessions = closed,
                reason,
                "closed unattributed inbound mesh sessions"
            );
        }
    }

    fn accept_inbound_role(&mut self, peer_id: &str) {
        if let Some(mut resources) = self.mesh.outbound.remove(peer_id) {
            resources.close();
            self.state.set_transport(peer_id, TransportState::Waiting);
            tracing::info!(
                stage = "transport",
                peer_id,
                "closed outbound session after deterministic dial role changed"
            );
        }
    }

    fn project_peer(&mut self, peer_id: &str, should_dial: bool) {
        let peers = self
            .discovery
            .peers
            .as_ref()
            .expect("peer registry exists while discovery events are handled");
        let peer = peers
            .get(peer_id)
            .expect("found peer exists in the registry");
        self.state.upsert_peer(DiscoveredPeer {
            id: peer.id.clone(),
            name: peer.id.clone(),
            endpoints: peer.endpoint_labels(),
            fingerprint_pinned: peer.fingerprint.is_some(),
            dial_role: if should_dial {
                DialRole::Outbound
            } else {
                DialRole::Inbound
            },
        });
    }
}

fn reset_outbound_for(update: PeerUpdate, active: bool) -> bool {
    update == PeerUpdate::IdentityReplaced || !active
}

pub(super) async fn run(
    commands: mpsc::Receiver<UserCommand>,
    snapshots: watch::Sender<Arc<AppSnapshot>>,
    playback: watch::Sender<Option<Arc<PlaybackFrame>>>,
) {
    Supervisor::new(playback).run(commands, snapshots).await;
}

fn watch_session(
    key: SessionKey,
    generation: u64,
    session: PeerSession,
    events: mpsc::Sender<OperationEvent>,
) {
    tokio::spawn(async move {
        let error = session.closed_error().await;
        let _ = events
            .send(OperationEvent::SessionClosed {
                key,
                generation,
                error,
            })
            .await;
    });
}

fn watch_announcements(
    receive_origin: moq_net::origin::Producer,
    events: mpsc::Sender<OperationEvent>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut announcements = receive_origin.consume().announced();
        while let Some(update) = announcements.next().await {
            if events
                .send(OperationEvent::ScreenAnnouncement {
                    path: update.path.to_string(),
                    broadcast: update.broadcast,
                })
                .await
                .is_err()
            {
                break;
            }
        }
    })
}

#[cfg(target_os = "linux")]
async fn run_view(
    generation: u64,
    path: String,
    broadcast: moq_net::broadcast::Consumer,
    mut cancelled: watch::Receiver<bool>,
    events: mpsc::Sender<OperationEvent>,
    frames: watch::Sender<Option<Arc<PlaybackFrame>>>,
) {
    let (playback_tx, mut playback_rx) = mpsc::channel(8);
    let playback = super::playback::run(
        generation,
        path.clone(),
        broadcast,
        cancelled.clone(),
        playback_tx,
        frames,
    );
    tokio::pin!(playback);
    let mut cancelling = *cancelled.borrow_and_update();
    let result = loop {
        tokio::select! {
            biased;
            result = &mut playback => break result,
            changed = cancelled.changed(), if !cancelling => {
                if changed.is_err() || *cancelled.borrow_and_update() {
                    cancelling = true;
                }
            }
            event = playback_rx.recv() => {
                let Some(event) = event else {
                    break playback.as_mut().await;
                };
                match event {
                    super::playback::Event::Started { ack } => {
                        if cancelling {
                            let _ = ack.send(());
                            continue;
                        }
                        let operation = OperationEvent::ViewStarted {
                            generation,
                            path: path.clone(),
                        };
                        tokio::select! {
                            result = events.send(operation) => {
                                if result.is_err() {
                                    break Err(anyhow::anyhow!("runtime operation channel closed"));
                                }
                            }
                            changed = cancelled.changed() => {
                                if changed.is_err() || *cancelled.borrow_and_update() {
                                    cancelling = true;
                                }
                            }
                        }
                        let _ = ack.send(());
                    }
                    super::playback::Event::Audio(audio) => {
                        if cancelling {
                            continue;
                        }
                        let operation = OperationEvent::ViewAudioChanged {
                            generation,
                            path: path.clone(),
                            audio,
                        };
                        tokio::select! {
                            result = events.send(operation) => {
                                if result.is_err() {
                                    break Err(anyhow::anyhow!("runtime operation channel closed"));
                                }
                            }
                            changed = cancelled.changed() => {
                                if changed.is_err() || *cancelled.borrow_and_update() {
                                    cancelling = true;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    .map_err(|error| error.to_string());

    if !cancelling && !*cancelled.borrow() {
        tokio::select! {
            _ = events.send(OperationEvent::ViewEnded { generation, result }) => {}
            _ = cancelled.changed() => {}
        }
    }
}

#[cfg(not(target_os = "linux"))]
async fn run_view(
    generation: u64,
    _path: String,
    _broadcast: moq_net::broadcast::Consumer,
    _cancelled: watch::Receiver<bool>,
    events: mpsc::Sender<OperationEvent>,
    _frames: watch::Sender<Option<Arc<PlaybackFrame>>>,
) {
    let _ = events
        .send(OperationEvent::ViewEnded {
            generation,
            result: Err("remote screen playback is available only on Linux".into()),
        })
        .await;
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use tokio::sync::watch;

    use crate::app::{
        DialRole, DiscoveredPeer, DiscoveryState, MediaState, RemoteAudioPhase,
        RemoteAudioSnapshot, TransportState, UserCommand,
    };
    use crate::network::service;

    use super::{
        DISCOVERY_RETRY_LIMIT, DiscoveryRetryBudget, OperationEvent, PEER_RETRY_LIMIT,
        PeerRetryBudget, SessionKey, Supervisor, ViewResources, reset_outbound_for,
    };

    fn supervisor() -> Supervisor {
        let (frames, _) = watch::channel(None);
        Supervisor::new(frames)
    }

    fn peer(id: &str) -> DiscoveredPeer {
        DiscoveredPeer {
            id: id.into(),
            name: id.into(),
            endpoints: vec!["192.0.2.1:4443".into()],
            fingerprint_pinned: true,
            dial_role: DialRole::Outbound,
        }
    }

    fn peer_record(id: &str) -> crate::network::discovery::PeerRecord {
        crate::network::discovery::PeerRecord::for_test(
            id,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)), 4443),
            "proof",
        )
    }

    #[tokio::test]
    async fn stopping_view_signals_and_awaits_the_owner() {
        struct Teardown(Arc<AtomicBool>);
        impl Drop for Teardown {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }

        let stopped = Arc::new(AtomicBool::new(false));
        let (cancel, mut cancelled) = watch::channel(false);
        let task_stopped = stopped.clone();
        let task = tokio::spawn(async move {
            let _teardown = Teardown(task_stopped);
            while cancelled.changed().await.is_ok() {
                if *cancelled.borrow_and_update() {
                    return;
                }
            }
        });
        let mut resources = ViewResources {
            task: Some(task),
            cancel: Some(cancel),
            generation: 4,
        };

        resources.stop().await;

        assert_eq!(resources.generation, 5);
        assert!(resources.task.is_none());
        assert!(resources.cancel.is_none());
        assert!(stopped.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn stale_audio_generation_cannot_update_a_new_view() {
        let mut supervisor = supervisor();
        supervisor.view.generation = 8;
        supervisor.state.media = MediaState::Viewing {
            path: "moqcast.screen/new".into(),
        };

        let action = supervisor
            .handle_operation_event(OperationEvent::ViewAudioChanged {
                generation: 7,
                path: "moqcast.screen/new".into(),
                audio: RemoteAudioSnapshot {
                    phase: RemoteAudioPhase::PcmSubmitted,
                    ..RemoteAudioSnapshot::default()
                },
            })
            .await;

        assert!(matches!(action, super::LoopAction::Unchanged));
        assert_eq!(supervisor.state.remote_audio.phase, RemoteAudioPhase::Idle);
    }

    #[tokio::test]
    async fn audio_failure_keeps_video_viewing() {
        let mut supervisor = supervisor();
        supervisor.view.generation = 8;
        supervisor.state.media = MediaState::Viewing {
            path: "moqcast.screen/peer".into(),
        };

        let action = supervisor
            .handle_operation_event(OperationEvent::ViewAudioChanged {
                generation: 8,
                path: "moqcast.screen/peer".into(),
                audio: RemoteAudioSnapshot {
                    phase: RemoteAudioPhase::Failed,
                    last_error: Some("output unavailable".into()),
                    ..RemoteAudioSnapshot::default()
                },
            })
            .await;

        assert!(matches!(action, super::LoopAction::Changed));
        assert!(matches!(
            supervisor.state.media,
            MediaState::Viewing { ref path } if path == "moqcast.screen/peer"
        ));
        assert_eq!(
            supervisor.state.remote_audio.phase,
            RemoteAudioPhase::Failed
        );
        assert!(supervisor.state.last_error.is_none());
    }

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

    #[test]
    fn peer_retry_budget_is_bounded_and_resets_after_connection() {
        let mut retries = PeerRetryBudget::default();

        for attempt in 1..=PEER_RETRY_LIMIT {
            let retry = retries
                .next(17)
                .expect("peer retry budget should allow the configured attempt");
            assert_eq!(retry.attempt, attempt);
            assert!(retry.delay <= super::PEER_RETRY_MAX_DELAY);
        }
        assert!(retries.next(17).is_none());

        retries.reset();
        assert_eq!(retries.next(17).expect("reset restores budget").attempt, 1);
    }

    #[test]
    fn rediscovery_keeps_a_healthy_session_but_identity_rotation_replaces_it() {
        assert!(!reset_outbound_for(
            crate::network::discovery::PeerUpdate::Added,
            true
        ));
        assert!(reset_outbound_for(
            crate::network::discovery::PeerUpdate::Added,
            false
        ));
        assert!(reset_outbound_for(
            crate::network::discovery::PeerUpdate::IdentityReplaced,
            true
        ));
        assert!(reset_outbound_for(
            crate::network::discovery::PeerUpdate::Unchanged,
            false
        ));
        assert!(!reset_outbound_for(
            crate::network::discovery::PeerUpdate::Unchanged,
            true
        ));
    }

    #[tokio::test]
    async fn stopping_discovery_cancels_a_scheduled_recovery() {
        let mut supervisor = supervisor();
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
        let mut supervisor = supervisor();
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
        let mut supervisor = supervisor();
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

    #[tokio::test]
    async fn three_peer_rows_keep_independent_transport_state() {
        let mut supervisor = supervisor();
        supervisor.state.start_discovery();
        supervisor.state.upsert_peer(peer("peer-a"));
        supervisor.state.upsert_peer(peer("peer-b"));
        supervisor.state.upsert_peer(peer("peer-c"));
        supervisor
            .state
            .set_transport("peer-b", TransportState::Connected);
        supervisor
            .state
            .set_transport("peer-c", TransportState::Connected);

        supervisor.state.mark_peer_lost("peer-a");
        supervisor
            .state
            .set_transport("peer-b", TransportState::Failed);

        assert!(!supervisor.state.peers.contains_key("peer-a"));
        assert_eq!(
            supervisor.state.peers["peer-b"].transport,
            TransportState::Failed
        );
        assert_eq!(
            supervisor.state.peers["peer-c"].transport,
            TransportState::Connected
        );
    }

    #[tokio::test]
    async fn stale_session_close_does_not_remove_new_generation() {
        let mut supervisor = supervisor();
        supervisor.state.upsert_peer(peer("peer-b"));
        let resources = supervisor.mesh.outbound.entry("peer-b".into()).or_default();
        resources.generation = 9;
        supervisor
            .state
            .set_transport("peer-b", TransportState::Connected);

        let action = supervisor.session_closed(SessionKey::Outbound("peer-b".into()), 8, None);

        assert!(matches!(action, super::LoopAction::Unchanged));
        assert!(supervisor.mesh.outbound.contains_key("peer-b"));
        assert_eq!(
            supervisor.state.peers["peer-b"].transport,
            TransportState::Connected
        );
    }

    #[tokio::test]
    async fn listener_recovery_clears_unattributed_inbound_sessions() {
        let mut supervisor = supervisor();
        supervisor.state.start_discovery();
        supervisor.mesh.inbound.insert(1, Default::default());
        supervisor.mesh.inbound.insert(2, Default::default());
        supervisor.state.set_inbound_session_count(2);

        supervisor.recover_discovery("listener stopped");

        assert!(supervisor.mesh.inbound.is_empty());
        assert_eq!(supervisor.state.inbound_session_count, 0);
    }

    #[tokio::test]
    async fn one_outbound_close_schedules_only_its_retry() {
        let mut supervisor = supervisor();
        supervisor.state.start_discovery();
        supervisor.state.upsert_peer(peer("peer-a"));
        supervisor.state.upsert_peer(peer("peer-b"));
        assert_eq!(
            supervisor.state.peers["peer-a"].transport,
            TransportState::Waiting
        );
        supervisor
            .state
            .set_transport("peer-a", TransportState::Connecting);
        assert_eq!(
            supervisor.state.peers["peer-a"].transport,
            TransportState::Connecting
        );
        supervisor
            .state
            .set_transport("peer-a", TransportState::Connected);
        supervisor
            .state
            .set_transport("peer-b", TransportState::Connected);
        let mut registry = crate::network::discovery::PeerRegistry::new("local");
        registry.found(peer_record("peer-a"));
        registry.found(peer_record("peer-b"));
        supervisor.discovery.peers = Some(registry);
        supervisor
            .mesh
            .outbound
            .entry("peer-a".into())
            .or_default()
            .generation = 7;
        supervisor
            .mesh
            .outbound
            .entry("peer-b".into())
            .or_default()
            .generation = 3;

        supervisor.session_closed(SessionKey::Outbound("peer-a".into()), 7, None);

        assert_eq!(
            supervisor.state.peers["peer-a"].transport,
            TransportState::Failed
        );
        assert_eq!(
            supervisor.state.peers["peer-b"].transport,
            TransportState::Connected
        );
        assert_eq!(supervisor.mesh.outbound["peer-a"].retry_budget.attempts, 1);
        assert_eq!(supervisor.mesh.outbound["peer-b"].retry_budget.attempts, 0);
        supervisor.mesh.close_all();
    }

    #[tokio::test]
    async fn inbound_role_change_closes_only_the_old_outbound_resource() {
        let mut supervisor = supervisor();
        supervisor.state.upsert_peer(peer("peer-a"));
        supervisor.state.upsert_peer(peer("peer-b"));
        supervisor
            .state
            .set_transport("peer-a", TransportState::Connected);
        supervisor
            .state
            .set_transport("peer-b", TransportState::Connected);
        supervisor
            .mesh
            .outbound
            .entry("peer-a".into())
            .or_default()
            .generation = 4;
        supervisor
            .mesh
            .outbound
            .entry("peer-b".into())
            .or_default()
            .generation = 8;

        supervisor.accept_inbound_role("peer-a");

        assert!(!supervisor.mesh.outbound.contains_key("peer-a"));
        assert!(supervisor.mesh.outbound.contains_key("peer-b"));
        assert_eq!(
            supervisor.state.peers["peer-a"].transport,
            TransportState::Waiting
        );
        assert_eq!(
            supervisor.state.peers["peer-b"].transport,
            TransportState::Connected
        );
        supervisor.mesh.close_all();
    }
}
