//! Serialized command processing for runtime-owned resources.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use moq_tokio::moq_net;
use moqcast_ui::DEFAULT_PLAYER_VOLUME_PERCENT;
use tokio::sync::{mpsc, watch};
use tokio::task::{JoinHandle, JoinSet};

use crate::app::MediaState;
use crate::app::{
    AppSnapshot, DialRole, DiscoveredPeer, RemoteAudioSnapshot, RemoteVideoSnapshot,
    TransportState, UserCommand,
};
use crate::network::discovery::{PeerRegistry, PeerUpdate};
use crate::network::{peer, server, service};
use crate::publish::session::{Failure as PublishFailure, Options as PublishOptions, Publication};
use crate::publish::source::{self, CaptureSource, SourceCatalog};

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
    CaptureSourcesLoaded(Result<SourceCatalog, String>),
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
        result: Result<(), PublishFailure>,
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
    ViewVideoChanged {
        generation: u64,
        path: String,
        video: RemoteVideoSnapshot,
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
            Self::Outbound(connection) => connection.abort(moq_net::Error::Cancel),
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
    retired: Vec<JoinHandle<()>>,
    retiring_services: JoinSet<Result<(), String>>,
}

impl DiscoveryResources {
    fn advance(&mut self) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.generation
    }

    fn cancel_current(&mut self) -> u64 {
        self.advance();
        for task in [self.start.take(), self.retry.take()].into_iter().flatten() {
            task.abort();
            self.retired.push(task);
        }
        if let Some(services) = self.services.take() {
            self.retiring_services.spawn(services.shutdown());
        }
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
    retired: Vec<JoinHandle<()>>,
    session: Option<PeerSession>,
    connecting: Option<moq_tokio::Connection>,
    attempt: Option<(crate::network::discovery::PeerRecord, tokio::time::Instant)>,
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
        self.attempt = None;
        if let Some(task) = self.pending.take() {
            task.abort();
            self.retired.push(task);
        }
        if let Some(connection) = self.connecting.take() {
            connection.abort(moq_net::Error::Cancel);
            self.retired.push(tokio::spawn(async move {
                let _ = connection.closed().await;
            }));
        }
        if let Some(session) = self.session.take() {
            session.close();
            self.retired.push(tokio::spawn(async move {
                let _ = session.closed_error().await;
            }));
        }
    }

    fn reset(&mut self) {
        self.close();
        self.retry_budget.reset();
    }
}

#[derive(Default)]
struct MeshResources {
    retired: Vec<JoinHandle<()>>,
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

    fn remove(&mut self, key: &SessionKey) {
        let resources = match key {
            SessionKey::Outbound(peer_id) => self.outbound.remove(peer_id),
            SessionKey::Inbound(id) => self.inbound.remove(id),
        };
        if let Some(mut resources) = resources {
            resources.close();
            self.retired.append(&mut resources.retired);
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
            self.retired.append(&mut resources.retired);
        }
        self.inbound.clear();
        count
    }

    fn close_all(&mut self) {
        for peer in self.outbound.values_mut().chain(self.inbound.values_mut()) {
            peer.close();
            self.retired.append(&mut peer.retired);
        }
        self.outbound.clear();
        self.inbound.clear();
    }
}

#[derive(Default)]
struct PublishResources {
    task: Option<JoinHandle<Result<(), String>>>,
    cancel: Option<watch::Sender<bool>>,
    generation: u64,
}

struct PublishCompletion {
    generation: u64,
    events: mpsc::Sender<OperationEvent>,
    cancelled: watch::Receiver<bool>,
}

impl PublishCompletion {
    async fn report(mut self, result: Result<(), PublishFailure>) -> Result<(), String> {
        // Stop joins this task while the operation queue is not being drained.
        let stopping = *self.cancelled.borrow();
        if !stopping {
            tokio::select! {
                biased;
                _ = self.cancelled.changed() => {},
                _ = self.events.send(OperationEvent::PublishEnded {
                    generation: self.generation,
                    result: result.clone(),
                }) => {},
            }
        }
        result.map_err(|error| error.to_string())
    }
}

#[derive(Default)]
struct ViewResources {
    task: Option<JoinHandle<()>>,
    cancel: Option<watch::Sender<bool>>,
    volume: Option<watch::Sender<u8>>,
    generation: u64,
}

impl ViewResources {
    fn advance(&mut self) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.generation
    }

    fn set_volume(&self, generation: u64, percent: u8) -> bool {
        if self.generation != generation {
            return false;
        }
        let Some(volume) = &self.volume else {
            return false;
        };
        volume.send_replace(percent.min(100));
        true
    }

    async fn stop(&mut self) -> Result<(), String> {
        self.advance();
        self.volume = None;
        if let Some(cancel) = self.cancel.take() {
            cancel.send_replace(true);
        }
        if let Some(task) = self.task.take() {
            task.await
                .map_err(|error| format!("playback shutdown task failed: {error}"))?;
        }
        Ok(())
    }
}

impl PublishResources {
    fn advance(&mut self) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.generation
    }

    async fn stop(&mut self) -> Result<(), String> {
        self.advance();
        if let Some(cancel) = self.cancel.take() {
            cancel.send_replace(true);
        }
        if let Some(task) = self.task.take() {
            return task
                .await
                .map_err(|error| format!("publication shutdown task failed: {error}"))?;
        }
        Ok(())
    }
}

struct Origins {
    publish: moq_net::origin::Producer,
    receive: moq_net::origin::Producer,
    drivers: JoinSet<()>,
    announcements: Option<JoinHandle<()>>,
}

impl Origins {
    fn new(events: mpsc::Sender<OperationEvent>) -> Self {
        let mut drivers = JoinSet::new();
        let (publish, publish_driver) = moq_net::origin::Producer::new(Default::default());
        let (receive, receive_driver) = moq_net::origin::Producer::new(Default::default());
        drivers.spawn(async move {
            let _ = moq_net::time::run(publish_driver).await;
        });
        drivers.spawn(async move {
            let _ = moq_net::time::run(receive_driver).await;
        });
        let announcements = watch_announcements(receive.clone(), events);
        Self {
            publish,
            receive,
            drivers,
            announcements: Some(announcements),
        }
    }

    async fn shutdown(self) -> Result<(), String> {
        let mut errors = crate::cleanup::Errors::default();
        if let Some(task) = self.announcements {
            task.abort();
            errors.join("announcement watcher", task.await, true);
        }
        drop(self.publish);
        drop(self.receive);
        let mut drivers = self.drivers;
        while let Some(result) = drivers.join_next().await {
            errors.join("origin driver", result, false);
        }
        errors.finish()
    }
}

struct Supervisor {
    cleanup: crate::cleanup::Errors,
    state: AppSnapshot,
    origins: Option<Origins>,
    session_watchers: JoinSet<()>,
    discovery: DiscoveryResources,
    mesh: MeshResources,
    remote_screens: HashMap<String, moq_net::broadcast::Consumer>,
    publish: PublishResources,
    view: ViewResources,
    source_refresh: Option<JoinHandle<()>>,
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
        Self {
            cleanup: Default::default(),
            state: AppSnapshot::default(),
            origins: None,
            session_watchers: JoinSet::new(),
            discovery: DiscoveryResources::default(),
            mesh: MeshResources::default(),
            remote_screens: HashMap::new(),
            publish: PublishResources::default(),
            view: ViewResources::default(),
            source_refresh: None,
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
                Input::Command(Some(command)) => {
                    if matches!(command, UserCommand::StopDiscovery) {
                        self.state.lan_stopping = true;
                        snapshots.send_replace(Arc::new(self.state.clone()));
                    }
                    self.handle_command(command).await
                }
                Input::Service(Some(event)) => self.handle_service_event(event).await,
                Input::Operation(Some(event)) => self.handle_operation_event(event).await,
                Input::Command(None) | Input::Service(None) | Input::Operation(None) => {
                    LoopAction::Shutdown
                }
            };
            while let Some(result) = self.session_watchers.try_join_next() {
                self.cleanup.join("session observer", result, false);
            }
            reap_retired(&mut self.discovery.retired, &mut self.cleanup).await;
            while let Some(result) = self.discovery.retiring_services.try_join_next() {
                match result {
                    Ok(result) => self.cleanup.result("retired services", result),
                    Err(error) => self.cleanup.join("retired services", Err(error), false),
                }
            }
            reap_retired(&mut self.mesh.retired, &mut self.cleanup).await;
            for peer in self
                .mesh
                .outbound
                .values_mut()
                .chain(self.mesh.inbound.values_mut())
            {
                reap_retired(&mut peer.retired, &mut self.cleanup).await;
            }
            match action {
                LoopAction::Changed => {
                    snapshots.send_replace(Arc::new(self.state.clone()));
                }
                LoopAction::Unchanged => {}
                LoopAction::Shutdown => break,
            }
        }

        if let Some(task) = self.source_refresh.take() {
            task.abort();
            let _ = task.await;
        }
        self.stop_lan().await;
        tracing::info!(stage = "runtime", "desktop runtime stopped");
    }

    async fn handle_command(&mut self, command: UserCommand) -> LoopAction {
        match command {
            UserCommand::StartDiscovery => self.start_discovery(),
            UserCommand::StopDiscovery => {
                self.stop_lan().await;
                LoopAction::Changed
            }
            UserCommand::RetryDiscovery => self.restart_discovery(),
            UserCommand::RefreshCaptureSources => self.refresh_sources(),
            UserCommand::StartScreenShare {
                system_audio,
                source,
            } => self.start_publish(system_audio, source),
            UserCommand::StopScreenShare => self.stop_publish().await,
            UserCommand::StartWatching { path } => self.start_view(path),
            UserCommand::SetPlaybackVolume {
                generation,
                percent,
            } => {
                if self.view.set_volume(generation, percent) {
                    tracing::debug!(
                        view_generation = generation,
                        volume_percent = percent.min(100),
                        "remote playback volume changed"
                    );
                } else {
                    tracing::debug!(
                        view_generation = generation,
                        volume_percent = percent.min(100),
                        "ignored stale remote playback volume change"
                    );
                }
                LoopAction::Unchanged
            }
            UserCommand::StopWatching => self.stop_view().await,
            UserCommand::Shutdown => LoopAction::Shutdown,
        }
    }

    async fn stop_lan(&mut self) {
        // The command loop cannot admit new work until this shutdown completes.
        // Invalidate discovery before joining tasks so queued results stay stale.
        let generation = self.discovery.advance();
        self.state.lan_stopping = true;
        self.state.local_peer_id = None;
        tracing::info!(
            stage = "lan_shutdown",
            generation,
            outbound = self.mesh.outbound.len(),
            inbound = self.mesh.inbound.len(),
            publishing = self.publish.task.is_some(),
            viewing = self.view.task.is_some(),
            "stopping LAN resources"
        );
        for task in [self.discovery.start.take(), self.discovery.retry.take()]
            .into_iter()
            .flatten()
        {
            task.abort();
            self.cleanup.join("cancelled LAN task", task.await, true);
        }
        if let Some(services) = self.discovery.services.take() {
            self.cleanup
                .result("discovery services", services.shutdown().await);
        }
        for task in self.discovery.retired.drain(..) {
            self.cleanup.join("cancelled LAN task", task.await, true);
        }
        while let Some(result) = self.discovery.retiring_services.join_next().await {
            match result {
                Ok(result) => self.cleanup.result("retired services", result),
                Err(error) => self.cleanup.join("retired services", Err(error), false),
            }
        }
        self.discovery.peers = None;
        self.discovery.retry_budget.reset();
        tracing::debug!(
            stage = "lan_shutdown",
            generation,
            step = "view",
            "discovery tasks stopped; stopping viewer"
        );
        let view_result = self.view.stop().await;
        tracing::debug!(
            stage = "lan_shutdown",
            generation,
            step = "publish",
            "viewer stopped; stopping publisher"
        );
        let publish_result = self.publish.stop().await;
        tracing::debug!(
            stage = "lan_shutdown",
            generation,
            step = "sessions",
            "closing connection owners"
        );
        let mut pending = Vec::new();
        for resources in self
            .mesh
            .outbound
            .values_mut()
            .chain(self.mesh.inbound.values_mut())
        {
            if let Some(task) = resources.pending.take() {
                task.abort();
                pending.push(task);
            }
            resources.close();
        }
        for task in pending {
            self.cleanup.join("cancelled LAN task", task.await, true);
        }
        self.mesh.close_all();
        for task in self.mesh.retired.drain(..) {
            self.cleanup.join("cancelled LAN task", task.await, true);
        }
        self.session_watchers.abort_all();
        while let Some(result) = self.session_watchers.join_next().await {
            self.cleanup.join("session observer", result, true);
        }

        // Stop all event producers before draining results which may own connections.
        if let Some(task) = self
            .origins
            .as_mut()
            .and_then(|origins| origins.announcements.take())
        {
            task.abort();
            self.cleanup.join("cancelled LAN task", task.await, true);
        }
        while let Ok(event) = self.operation_rx.try_recv() {
            match event {
                OperationEvent::ServicesStarted {
                    result: Ok(services),
                    ..
                } => self
                    .cleanup
                    .result("late services", services.shutdown().await),
                OperationEvent::SessionReady {
                    result: Ok(session),
                    ..
                } => {
                    session.close();
                    let _ = session.closed_error().await;
                }
                OperationEvent::CaptureSourcesLoaded(result) => {
                    self.source_refresh = None;
                    self.state.sources = result.unwrap_or_else(SourceCatalog::Failed);
                }
                _ => {}
            }
        }
        while let Ok(event) = self.service_rx.try_recv() {
            if let service::EventKind::Inbound(request) = event.kind {
                let _ = request.reject(moq_tokio::server::Reject::App(503)).await;
            }
        }
        self.remote_screens.clear();
        tracing::debug!(
            stage = "lan_shutdown",
            generation,
            step = "origins",
            "event producers joined; stopping route drivers"
        );
        if let Some(origins) = self.origins.take() {
            self.cleanup.result("origins", origins.shutdown().await);
        }
        self.playback_tx.send_replace(None);
        self.state.stop_discovery();
        self.state.lan_stopping = false;
        self.state.peers.clear();
        self.state.remote_screens.clear();
        self.state.inbound_session_count = 0;
        self.state.local_peer_id = None;
        self.state.media = MediaState::Idle;
        self.state.remote_audio = RemoteAudioSnapshot::default();
        self.state.remote_video = RemoteVideoSnapshot::default();
        self.cleanup.result("viewer", view_result);
        self.cleanup.result("publisher", publish_result);
        if let Err(error) = std::mem::take(&mut self.cleanup).finish() {
            self.state.discovery = crate::app::DiscoveryState::Error;
            tracing::warn!(stage = "lan_shutdown", generation, %error, "LAN cleanup reported a failure");
            self.state.last_error = Some(format!("LAN shutdown reported an error: {error}"));
        } else {
            self.state.last_error = None;
            tracing::info!(
                stage = "lan_shutdown",
                generation,
                mdns_completion = "drop_requested",
                "application LAN owners stopped; mDNS daemon completion is not observable"
            );
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
        self.state.local_peer_id = None;
        self.origins
            .get_or_insert_with(|| Origins::new(self.operation_tx.clone()));
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
        self.start_outbound(
            peer_id,
            reset,
            tokio::time::Instant::now() + peer::RECONNECT_BUDGET,
        );
    }

    async fn refresh_outbound(&mut self, peer_id: &str) {
        let Some(record) = self
            .discovery
            .peers
            .as_ref()
            .and_then(|peers| peers.get(peer_id))
        else {
            return;
        };
        let Some(resources) = self.mesh.outbound.get_mut(peer_id) else {
            return;
        };
        let Some((previous, deadline)) = resources.attempt.as_ref() else {
            return;
        };
        if resources.session.is_some()
            || resources
                .connecting
                .as_ref()
                .is_none_or(|connection| connection.connected())
            || previous.has_node
            || record.has_node
            || previous.credential != record.credential
            || previous.fingerprint != record.fingerprint
            || !record
                .addrs
                .iter()
                .any(|addr| !previous.addrs.contains(addr))
            || tokio::time::Instant::now() >= *deadline
        {
            return;
        }
        let deadline = *deadline;
        tracing::debug!(
            stage = "transport",
            generation = resources.generation,
            peer_id,
            previous_addresses = ?previous.addrs,
            addresses = ?record.addrs,
            previous_address_count = previous.addrs.len(),
            address_count = record.addrs.len(),
            "refreshing pending peer addresses within original connection budget"
        );
        // Retire and join the old owner before opening its replacement. Its queued
        // completion is ignored by the generation advanced here.
        resources.close();
        for task in resources.retired.drain(..) {
            self.cleanup
                .join("refreshed outbound owner", task.await, true);
        }
        self.start_outbound(peer_id, false, deadline);
    }

    fn start_outbound(&mut self, peer_id: &str, reset: bool, deadline: tokio::time::Instant) {
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
        let origins = self
            .origins
            .as_ref()
            .expect("LAN origins exist while connecting");
        let publish_origin = origins.publish.clone();
        let receive_origin = origins.receive.clone();
        let events = self.operation_tx.clone();
        // An expired refresh never creates a fresh retry budget or a new socket.
        tracing::debug!(
            stage = "transport",
            generation,
            peer_id,
            addresses = ?record.addrs,
            address_count = record.addrs.len(),
            ipv4_count = record.addrs.iter().filter(|addr| addr.is_ipv4()).count(),
            ipv6_count = record.addrs.iter().filter(|addr| addr.is_ipv6()).count(),
            remaining_ms = deadline
                .saturating_duration_since(tokio::time::Instant::now())
                .as_millis() as u64,
            "starting outbound candidate set"
        );
        let dial = if tokio::time::Instant::now() >= deadline {
            Err("peer connection setup budget exhausted".to_owned())
        } else {
            peer::dial(&record, &publish_origin, receive_origin).map_err(|error| error.to_string())
        };
        peer_resources.connecting = dial.as_ref().ok().cloned();
        peer_resources.attempt = Some((record, deadline));
        peer_resources.pending = Some(tokio::spawn(async move {
            let result = match dial {
                Ok(connection) => {
                    match tokio::time::timeout_at(deadline, connection.clone().established()).await
                    {
                        Ok(result) => result
                            .map(PeerSession::Outbound)
                            .map_err(|error| error.to_string()),
                        Err(_) => {
                            connection.abort(moq_net::Error::Cancel);
                            Err("peer connection setup budget exhausted".to_owned())
                        }
                    }
                }
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

    fn refresh_sources(&mut self) -> LoopAction {
        if self.source_refresh.is_some() || self.state.media != MediaState::Idle {
            return LoopAction::Unchanged;
        }
        self.state.sources = SourceCatalog::Loading;
        let events = self.operation_tx.clone();
        self.source_refresh = Some(tokio::spawn(async move {
            let result = source::enumerate().await;
            let _ = events
                .send(OperationEvent::CaptureSourcesLoaded(result))
                .await;
        }));
        LoopAction::Changed
    }

    fn start_publish(&mut self, system_audio: bool, source: CaptureSource) -> LoopAction {
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

        let publication = match Publication::prepare(
            &self
                .origins
                .as_ref()
                .expect("LAN origins exist while publishing")
                .publish,
            &local_peer_id,
            PublishOptions {
                system_audio,
                source,
            },
        ) {
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
        let (cancel, cancelled) = watch::channel(false);
        self.publish.cancel = Some(cancel);
        self.publish.task = Some(tokio::spawn(async move {
            let result = publication
                .run(cancelled.clone())
                .await
                .map_err(PublishFailure::from);
            PublishCompletion {
                generation,
                events,
                cancelled,
            }
            .report(result)
            .await
        }));
        self.state
            .finish_publish()
            .expect("publication preparation was active");
        LoopAction::Changed
    }

    async fn stop_publish(&mut self) -> LoopAction {
        if self.publish.task.is_none() && self.state.media == MediaState::Idle {
            return LoopAction::Unchanged;
        }
        if let Err(error) = self.state.begin_stop_publish() {
            self.state.last_error = Some(error.to_string());
            return LoopAction::Changed;
        }
        let result = self.publish.stop().await;
        self.state
            .finish_stop_publish()
            .expect("publication was stopping");
        if let Err(error) = result {
            tracing::warn!(
                stage = "publish",
                error,
                "screen sharing ended with an error"
            );
            self.state.last_error = Some(format!("Screen sharing stopped with an error: {error}"));
        }
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
        self.state.view_generation = generation;
        let events = self.operation_tx.clone();
        let frames = self.playback_tx.clone();
        let (cancel, cancelled) = watch::channel(false);
        let (volume, volume_rx) = watch::channel(DEFAULT_PLAYER_VOLUME_PERCENT);
        self.view.cancel = Some(cancel);
        self.view.volume = Some(volume);
        self.view.task = Some(tokio::spawn(run_view(
            generation, path, broadcast, cancelled, events, frames, volume_rx,
        )));
        LoopAction::Changed
    }

    async fn stop_view(&mut self) -> LoopAction {
        if let Err(error) = self.state.begin_stop_view() {
            self.state.last_error = Some(error.to_string());
            return LoopAction::Changed;
        }
        let result = self.view.stop().await;
        self.playback_tx.send_replace(None);
        self.state
            .finish_stop_view()
            .expect("remote playback was stopping");
        if let Err(error) = result {
            self.state.last_error = Some(error);
        }
        LoopAction::Changed
    }

    async fn handle_service_event(&mut self, event: service::Event) -> LoopAction {
        if event.generation != self.discovery.generation {
            tracing::debug!(
                stage = "discovery",
                generation = event.generation,
                current_generation = self.discovery.generation,
                "ignored stale LAN event"
            );
            return LoopAction::Unchanged;
        }
        match event.kind {
            service::EventKind::Found { peer, should_dial } => {
                tracing::debug!(
                    stage = "discovery",
                    generation = event.generation,
                    peer_id = %peer.id,
                    addresses = ?peer.addrs,
                    address_count = peer.addrs.len(),
                    should_dial,
                    "Nearby discovery delivered Found"
                );
                let record = peer;
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
                    let reset = reset_outbound_for(update, active);
                    if !reset && active && update == PeerUpdate::CandidatesMerged {
                        self.refresh_outbound(&peer_id).await;
                    } else if reset || update == PeerUpdate::Added {
                        self.ensure_outbound(&peer_id, reset);
                    }
                } else {
                    self.accept_inbound_role(&peer_id);
                }
                LoopAction::Changed
            }
            service::EventKind::Lost(id) => {
                tracing::debug!(
                    stage = "discovery",
                    generation = event.generation,
                    peer_id = %id,
                    "Nearby discovery delivered Lost"
                );
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

    async fn accept_inbound(&mut self, request: moq_tokio::server::Request) -> LoopAction {
        tracing::debug!(
            stage = "transport",
            generation = self.discovery.generation,
            "inbound request reached application authorization"
        );
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
            request
                .reject(moq_tokio::server::Reject::App(503))
                .await
                .ok();
            return LoopAction::Unchanged;
        };
        if !server::authorized_request(&request, &credential) {
            tracing::warn!(stage = "auth", "rejecting unauthorized inbound peer");
            request
                .reject(moq_tokio::server::Reject::Forbidden)
                .await
                .ok();
            return LoopAction::Unchanged;
        }

        let key = self.mesh.next_inbound();
        let SessionKey::Inbound(id) = key else {
            unreachable!()
        };
        tracing::debug!(
            stage = "transport",
            inbound_id = id,
            "inbound credential accepted"
        );
        let resources = self.mesh.inbound.entry(id).or_default();
        let generation = resources.advance();
        let origins = self
            .origins
            .as_ref()
            .expect("LAN origins exist while connecting");
        let publish_origin = origins.publish.clone();
        let receive_origin = origins.receive.clone();
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
            OperationEvent::CaptureSourcesLoaded(result) => {
                self.source_refresh = None;
                self.state.sources = match result {
                    Ok(catalog) => catalog,
                    Err(error) => SourceCatalog::Failed(error),
                };
                LoopAction::Changed
            }
            OperationEvent::ServicesStarted { generation, result } => {
                if generation != self.discovery.generation || !self.state.discovery.is_active() {
                    if let Ok(services) = result {
                        self.cleanup
                            .result("stale services", services.shutdown().await);
                    }
                    return LoopAction::Unchanged;
                }
                if let Some(task) = self.discovery.start.take() {
                    self.discovery.retired.push(task);
                }
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
                self.publish.cancel = None;
                match result {
                    Ok(()) => {
                        self.state.end_publish().expect("current publication ended");
                    }
                    Err(error) => {
                        tracing::warn!(stage = "publish", %error, "screen publication ended");
                        if let PublishFailure::Source(message) = &error {
                            self.state.sources = SourceCatalog::Failed(message.clone());
                        }
                        self.state
                            .fail_publish(error.to_string())
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
            OperationEvent::ViewVideoChanged {
                generation,
                path,
                video,
            } => {
                if generation != self.view.generation {
                    return LoopAction::Unchanged;
                }
                if self.state.set_remote_video(&path, video) {
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
                self.mesh.retired.push(tokio::spawn(async move {
                    let _ = session.closed_error().await;
                }));
            }
            return LoopAction::Unchanged;
        };
        resources.attempt = None;
        if let Some(task) = resources.pending.take() {
            resources.retired.push(task);
        }
        if let Some(connection) = resources.connecting.take()
            && result.is_err()
        {
            connection.abort(moq_net::Error::Cancel);
            resources.retired.push(tokio::spawn(async move {
                let _ = connection.closed().await;
            }));
        }

        match result {
            Ok(session) => {
                let observation = moqcast_lan_state::bandwidth::Observation::default();
                if let SessionKey::Outbound(peer_id) = &key
                    && let Some(peer) = self.state.peers.get_mut(peer_id)
                {
                    peer.bandwidth = observation.view();
                }
                self.session_watchers.spawn(watch_session(
                    key.clone(),
                    generation,
                    session.clone(),
                    self.operation_tx.clone(),
                    observation,
                ));
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
            // Keep the generation tombstone until Off; rediscovery may reuse this ID.
            if let Some(resources) = self.mesh.outbound.get_mut(peer_id) {
                resources.close();
            }
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
        // Only a current ServicesStarted result may admit new media again.
        self.state.local_peer_id = None;
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
        // Keep the attempt generation across role changes so old completions
        // cannot match a later outbound attempt for the same advertised ID.
        if let Some(resources) = self.mesh.outbound.get_mut(peer_id) {
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

async fn watch_session(
    key: SessionKey,
    generation: u64,
    session: PeerSession,
    events: mpsc::Sender<OperationEvent>,
    observation: moqcast_lan_state::bandwidth::Observation,
) {
    let error = match (&key, &session) {
        (SessionKey::Outbound(peer), PeerSession::Outbound(connection)) => {
            crate::bandwidth::outbound(connection, observation, peer, generation)
                .await
                .err()
                .map(|error| error.to_string())
        }
        (SessionKey::Inbound(id), PeerSession::Inbound(session)) => {
            Some(crate::bandwidth::inbound(session, *id).await.to_string())
        }
        _ => session.closed_error().await,
    };
    let _ = events
        .send(OperationEvent::SessionClosed {
            key,
            generation,
            error,
        })
        .await;
}

fn watch_announcements(
    receive_origin: moq_net::origin::Producer,
    events: mpsc::Sender<OperationEvent>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut announcements = receive_origin.consume().announced();
        while let Some(update) = announcements.next().await {
            let (update, active) = match update {
                moq_net::announce::Event::Start(update)
                | moq_net::announce::Event::Update(update) => (update, true),
                moq_net::announce::Event::End(update) => (update, false),
            };
            let path = update.prefix.to_string();
            let broadcast = if active {
                match receive_origin
                    .consume()
                    .request_broadcast(path.as_str(), update.route.epoch.clone())
                    .await
                {
                    Ok(broadcast) => Some(broadcast),
                    Err(error) => {
                        tracing::warn!(%error, "remote screen announcement could not resolve");
                        continue;
                    }
                }
            } else {
                None
            };
            if events
                .send(OperationEvent::ScreenAnnouncement { path, broadcast })
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
    volume: watch::Receiver<u8>,
) {
    let (playback_tx, mut playback_rx) = mpsc::channel(8);
    let playback = super::playback::run(
        generation,
        path.clone(),
        broadcast,
        cancelled.clone(),
        playback_tx,
        frames,
        volume,
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
                    super::playback::Event::Video(video) => {
                        if cancelling {
                            continue;
                        }
                        let operation = OperationEvent::ViewVideoChanged {
                            generation,
                            path: path.clone(),
                            video,
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
    _volume: watch::Receiver<u8>,
) {
    let _ = events
        .send(OperationEvent::ViewEnded {
            generation,
            result: Err("remote screen playback is available only on Linux".into()),
        })
        .await;
}

async fn reap_retired(tasks: &mut Vec<JoinHandle<()>>, errors: &mut crate::cleanup::Errors) {
    let mut index = 0;
    while index < tasks.len() {
        if tasks[index].is_finished() {
            errors.join("retired LAN task", tasks.swap_remove(index).await, true);
        } else {
            index += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PeerResources, PeerSession};
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use tokio::sync::watch;

    use crate::app::{
        DialRole, DiscoveredPeer, DiscoveryState, MediaState, RemoteAudioPhase,
        RemoteAudioSnapshot, TransportState, UserCommand,
    };
    use crate::network::service;
    use crate::publish::session::Failure as PublishFailure;
    use crate::publish::source::{CaptureSource, SourceCatalog};

    use super::{
        DISCOVERY_RETRY_LIMIT, DiscoveryRetryBudget, OperationEvent, PEER_RETRY_LIMIT,
        PeerRetryBudget, PublishCompletion, PublishResources, SessionKey, Supervisor,
        ViewResources, reset_outbound_for,
    };

    fn supervisor() -> Supervisor {
        let (frames, _) = watch::channel(None);
        let mut supervisor = Supervisor::new(frames);
        supervisor.origins = Some(super::Origins::new(supervisor.operation_tx.clone()));
        supervisor
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
    async fn shutdown_waits_for_connection_still_being_established() {
        use std::{
            future::Future,
            task::{Context, Poll, Waker},
        };
        let _ = moq_tokio::rustls::crypto::aws_lc_rs::default_provider().install_default();
        let (mut supervisor, mut advert) = discovered_outbound();
        advert.fingerprint = Some("00".repeat(32));
        advert.addrs = vec!["127.0.0.1:9".parse().unwrap()];
        advert.urls = vec!["moqt://127.0.0.1:9".parse().unwrap()];
        supervisor
            .discovery
            .peers
            .as_mut()
            .unwrap()
            .found(advert.clone());
        supervisor.ensure_outbound(&advert.id, true);
        let connection = supervisor.mesh.outbound[&advert.id]
            .connecting
            .as_ref()
            .unwrap()
            .clone();
        supervisor.stop_lan().await;
        let mut closed = std::pin::pin!(connection.closed());
        assert!(matches!(
            closed
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(_)
        ));
    }

    #[tokio::test]
    async fn completed_retired_failure_survives_reaping_until_off() {
        let mut supervisor = supervisor();
        let task = tokio::spawn(async {
            panic!("reaped task panic");
        });
        while !task.is_finished() {
            tokio::task::yield_now().await;
        }
        supervisor.mesh.retired.push(task);
        super::reap_retired(&mut supervisor.mesh.retired, &mut supervisor.cleanup).await;
        assert!(supervisor.mesh.retired.is_empty());
        supervisor.stop_lan().await;
        assert!(
            supervisor
                .state
                .last_error
                .as_deref()
                .unwrap()
                .contains("reaped task panic")
        );
        // Consumed failures must not poison the next independent lifecycle.
        supervisor.stop_lan().await;
        assert!(supervisor.state.last_error.is_none());
    }

    #[tokio::test]
    async fn shutdown_reports_origin_failure_after_joining_other_drivers() {
        let mut supervisor = supervisor();
        let completed = Arc::new(AtomicBool::new(false));
        let done = completed.clone();
        let origins = supervisor.origins.as_mut().unwrap();
        origins.drivers.spawn(async {
            panic!("origin cleanup panic");
        });
        origins.drivers.spawn(async move {
            done.store(true, Ordering::SeqCst);
        });
        supervisor.stop_lan().await;
        assert!(completed.load(Ordering::SeqCst));
        assert!(
            supervisor
                .state
                .last_error
                .as_deref()
                .unwrap()
                .contains("origin cleanup panic")
        );
    }

    #[tokio::test]
    async fn removed_inbound_preserves_retired_cleanup_errors() {
        let (frames, _) = watch::channel(None);
        let mut supervisor = Supervisor::new(frames);
        let task = tokio::spawn(async {
            panic!("retired inbound panic");
        });
        while !task.is_finished() {
            tokio::task::yield_now().await;
        }
        let mut peer = PeerResources::default();
        peer.retired.push(task);
        supervisor.mesh.inbound.insert(7, peer);
        supervisor.mesh.remove(&SessionKey::Inbound(7));
        supervisor.stop_lan().await;
        assert_eq!(
            supervisor.state.discovery,
            crate::app::DiscoveryState::Error
        );
    }

    #[tokio::test]
    async fn shutdown_waits_for_active_and_late_connection_loops() {
        use std::{
            future::Future,
            task::{Context, Poll, Waker},
        };
        let _ = moq_tokio::rustls::crypto::aws_lc_rs::default_provider().install_default();
        for late in [false, true] {
            let (frames, _) = watch::channel(None);
            let mut supervisor = Supervisor::new(frames);
            let client = moq_tokio::connect::Config::default()
                .init(Default::default())
                .unwrap();
            let connection = client.connect("moqt://127.0.0.1:9".parse::<url::Url>().unwrap());
            let observed = connection.clone();
            if late {
                supervisor
                    .operation_tx
                    .try_send(OperationEvent::SessionReady {
                        key: SessionKey::Outbound("test".into()),
                        generation: 1,
                        result: Ok(PeerSession::Outbound(connection)),
                    })
                    .unwrap();
            } else {
                supervisor.mesh.outbound.insert(
                    "test".into(),
                    PeerResources {
                        session: Some(PeerSession::Outbound(connection)),
                        ..Default::default()
                    },
                );
            }
            supervisor.stop_lan().await;
            let mut closed = std::pin::pin!(observed.closed());
            assert!(matches!(
                closed
                    .as_mut()
                    .poll(&mut Context::from_waker(Waker::noop())),
                Poll::Ready(_)
            ));
        }
    }

    #[tokio::test]
    async fn shutdown_reports_retired_task_failures() {
        let (frames, _) = watch::channel(None);
        let mut supervisor = Supervisor::new(frames);
        let task = tokio::spawn(async {
            panic!("retired dial panic");
        });
        while !task.is_finished() {
            tokio::task::yield_now().await;
        }
        supervisor.mesh.retired.push(task);
        supervisor.stop_lan().await;
        assert_eq!(
            supervisor.state.discovery,
            crate::app::DiscoveryState::Error
        );
        assert!(
            supervisor
                .state
                .last_error
                .as_deref()
                .unwrap()
                .contains("panic")
        );
    }

    #[tokio::test]
    async fn nearby_audit_viewer_panic_is_not_successful_off() {
        let mut supervisor = supervisor();
        supervisor.state.start_discovery();
        supervisor.state.local_peer_id = Some("local".into());
        supervisor.view.task = Some(tokio::spawn(async { panic!("injected viewer failure") }));
        supervisor.stop_lan().await;
        assert_eq!(supervisor.state.discovery, DiscoveryState::Error);
        assert!(
            supervisor
                .state
                .last_error
                .as_deref()
                .unwrap()
                .contains("playback")
        );
        assert!(!supervisor.state.access().publish());
        assert!(
            supervisor.origins.is_none(),
            "a failed viewer must not skip remaining cleanup"
        );
    }

    #[tokio::test]
    async fn nearby_audit_stop_joins_tasks_cancelled_before_off() {
        struct Released(Arc<AtomicBool>);
        impl Drop for Released {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        for discovery in [true, false] {
            let (frames, _) = watch::channel(None);
            let mut supervisor = Supervisor::new(frames);
            let released = Arc::new(AtomicBool::new(false));
            let guard = Released(released.clone());
            let (ready, started) = tokio::sync::oneshot::channel();
            let task = tokio::spawn(async move {
                let _guard = guard;
                ready.send(()).unwrap();
                std::future::pending::<()>().await;
            });
            started.await.unwrap();
            if discovery {
                supervisor.discovery.start = Some(task);
                supervisor.discovery.cancel_current();
            } else {
                supervisor.mesh.inbound.entry(1).or_default().pending = Some(task);
                supervisor.close_inbound("test listener restart");
            }
            supervisor.stop_lan().await;
            assert!(
                released.load(Ordering::SeqCst),
                "Off returned before prior task teardown: discovery={discovery}"
            );
        }
    }

    #[tokio::test]
    async fn nearby_audit_old_outbound_close_cannot_match_after_role_change() {
        let (mut supervisor, advert) = discovered_outbound();
        let old = supervisor
            .mesh
            .outbound
            .entry(advert.id.clone())
            .or_default()
            .advance();
        supervisor.accept_inbound_role(&advert.id);
        supervisor.ensure_outbound(&advert.id, false);
        supervisor.session_closed(SessionKey::Outbound(advert.id.clone()), old, None);
        assert_eq!(
            supervisor.state.peers[&advert.id].transport,
            TransportState::Connecting
        );
        supervisor.stop_lan().await;
    }

    #[tokio::test]
    async fn nearby_contract_stop_joins_media_and_pending_work() {
        struct Released(Arc<AtomicBool>);
        impl Drop for Released {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        for publishing in [false, true] {
            let mut supervisor = supervisor();
            supervisor.state.start_discovery();
            supervisor.state.local_peer_id = Some("local".into());
            supervisor.state.upsert_peer(peer("remote"));
            supervisor.state.set_inbound_session_count(1);
            supervisor.state.media = if publishing {
                MediaState::Publishing
            } else {
                MediaState::Viewing {
                    path: "moqcast.screen/remote".into(),
                }
            };
            let (cancel, mut cancelled) = watch::channel(false);
            let released = Arc::new(AtomicBool::new(false));
            let guard = Released(released.clone());
            let (ready, started) = tokio::sync::oneshot::channel();
            let task = tokio::spawn(async move {
                let _guard = guard;
                ready.send(()).unwrap();
                while !*cancelled.borrow_and_update() {
                    if cancelled.changed().await.is_err() {
                        break;
                    }
                }
            });
            started.await.unwrap();
            if publishing {
                supervisor.publish.cancel = Some(cancel);
                supervisor.publish.task = Some(tokio::spawn(async move {
                    task.await.unwrap();
                    Ok(())
                }));
            } else {
                supervisor.view.cancel = Some(cancel);
                supervisor.view.task = Some(task);
            }
            let pending_released = Arc::new(AtomicBool::new(false));
            let guard = Released(pending_released.clone());
            supervisor
                .mesh
                .outbound
                .entry("remote".into())
                .or_default()
                .pending = Some(tokio::spawn(async move {
                let _guard = guard;
                std::future::pending::<()>().await;
            }));
            let mut extra_released = Vec::new();
            for slot in [
                &mut supervisor.discovery.start,
                &mut supervisor.discovery.retry,
            ] {
                let released = Arc::new(AtomicBool::new(false));
                let guard = Released(released.clone());
                *slot = Some(tokio::spawn(async move {
                    let _guard = guard;
                    std::future::pending::<()>().await;
                }));
                extra_released.push(released);
            }
            let observer_released = Arc::new(AtomicBool::new(false));
            let guard = Released(observer_released.clone());
            supervisor.session_watchers.spawn(async move {
                let _guard = guard;
                std::future::pending::<()>().await;
            });
            supervisor.handle_command(UserCommand::StopDiscovery).await;
            assert!(
                extra_released
                    .iter()
                    .all(|flag| flag.load(Ordering::Acquire))
            );
            assert!(observer_released.load(Ordering::Acquire));
            assert!(
                released.load(Ordering::Acquire),
                "media owner not released before Off"
            );
            assert!(
                pending_released.load(Ordering::Acquire),
                "pending owner not joined"
            );
            assert!(supervisor.mesh.outbound.is_empty());
            assert!(supervisor.mesh.inbound.is_empty());
            assert!(supervisor.state.peers.is_empty());
            assert!(supervisor.state.local_peer_id.is_none());
            assert_eq!(supervisor.state.inbound_session_count, 0);
            assert_eq!(supervisor.state.media, MediaState::Idle);
            assert_eq!(supervisor.state.discovery, DiscoveryState::Idle);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn nearby_contract_stop_does_not_wait_on_a_full_operation_queue() {
        let mut supervisor = supervisor();
        supervisor.state.start_discovery();
        supervisor.state.media = MediaState::Publishing;
        let generation = supervisor.publish.advance();
        let (cancel, cancelled) = watch::channel(false);
        supervisor.publish.cancel = Some(cancel);
        for _ in 0..super::EVENT_CAPACITY {
            supervisor
                .operation_tx
                .try_send(OperationEvent::RetryOutbound {
                    peer_id: "old".into(),
                    generation: 0,
                })
                .unwrap();
        }
        let completion = PublishCompletion {
            generation,
            events: supervisor.operation_tx.clone(),
            cancelled,
        };
        supervisor.publish.task = Some(tokio::spawn(completion.report(Ok(()))));
        // Virtual-time deadline detects a deadlock without sleeping or relaxing production timeouts.
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            supervisor.handle_command(UserCommand::StopDiscovery),
        )
        .await
        .expect("stop must not block on its event queue");
        assert!(supervisor.publish.task.is_none());
        assert_eq!(supervisor.state.media, MediaState::Idle);
    }

    #[tokio::test]
    async fn nearby_contract_close_error_is_not_reported_as_off() {
        let mut supervisor = supervisor();
        supervisor.state.start_discovery();
        supervisor.state.media = MediaState::Publishing;
        supervisor.publish.task = Some(tokio::spawn(async { Err("close failed".to_owned()) }));
        supervisor.handle_command(UserCommand::StopDiscovery).await;
        assert_eq!(supervisor.state.discovery, DiscoveryState::Error);
        assert!(
            supervisor
                .state
                .last_error
                .as_deref()
                .unwrap()
                .contains("close failed")
        );
    }

    #[tokio::test]
    async fn nearby_contract_stop_drains_old_events_before_reopening() {
        let mut supervisor = supervisor();
        supervisor.state.start_discovery();
        let old = supervisor.discovery.generation;
        let broadcast = super::moq_net::broadcast::Producer::new(Default::default());
        supervisor
            .operation_tx
            .try_send(OperationEvent::ScreenAnnouncement {
                path: "moqcast.screen/old".into(),
                broadcast: Some(broadcast.consume()),
            })
            .unwrap();
        supervisor
            .service_tx
            .try_send(service::Event {
                generation: old,
                kind: service::EventKind::InitialScanFinished,
            })
            .unwrap();
        supervisor.handle_command(UserCommand::StopDiscovery).await;
        assert!(supervisor.operation_rx.try_recv().is_err());
        assert!(supervisor.service_rx.try_recv().is_err());
        assert!(supervisor.origins.is_none());
        assert!(supervisor.session_watchers.is_empty());
        // Create the next run without opening real sockets.
        supervisor.state.start_discovery();
        supervisor.origins = Some(super::Origins::new(supervisor.operation_tx.clone()));
        supervisor.state.upsert_peer(peer("new"));
        let action = supervisor
            .handle_service_event(service::Event {
                generation: old,
                kind: service::EventKind::Lost("new".into()),
            })
            .await;
        assert!(matches!(action, super::LoopAction::Unchanged));
        assert!(supervisor.state.peers.contains_key("new"));
        assert!(supervisor.remote_screens.is_empty());
        supervisor.handle_command(UserCommand::StopDiscovery).await;
    }

    #[tokio::test]
    async fn rejected_source_invalidates_catalog_before_retry() {
        let mut supervisor = supervisor();
        let selected = CaptureSource::Display {
            id: "x11:0".into(),
            name: "DP-1".into(),
            width: 1920,
            height: 1080,
        };
        supervisor.state.sources = SourceCatalog::Ready(vec![selected.clone()]);
        supervisor.state.media = MediaState::Publishing;
        supervisor.publish.generation = 7;
        supervisor
            .handle_operation_event(OperationEvent::PublishEnded {
                generation: 6,
                result: Err(PublishFailure::Source("stale failure".into())),
            })
            .await;
        assert!(matches!(supervisor.state.sources, SourceCatalog::Ready(_)));
        supervisor
            .handle_operation_event(OperationEvent::PublishEnded {
                generation: 7,
                result: Err(PublishFailure::Other("encoder failed".into())),
            })
            .await;
        assert!(matches!(supervisor.state.sources, SourceCatalog::Ready(_)));
        supervisor.state.media = MediaState::Publishing;
        supervisor
            .handle_operation_event(OperationEvent::PublishEnded {
                generation: 7,
                result: Err(PublishFailure::Source(
                    "The selected source is unavailable. Refresh and select again.".into(),
                )),
            })
            .await;
        assert!(matches!(supervisor.state.sources, SourceCatalog::Failed(_)));
        assert_eq!(supervisor.state.sources.selected(Some(&selected)), None);
        assert_eq!(supervisor.state.media, MediaState::Idle);
    }

    #[tokio::test]
    async fn source_refresh_is_rejected_while_media_is_active() {
        let (playback, _) = watch::channel(None);
        let mut supervisor = Supervisor::new(playback);
        supervisor.state.media = MediaState::Publishing;
        assert!(matches!(
            supervisor.refresh_sources(),
            super::LoopAction::Unchanged
        ));
        assert!(supervisor.source_refresh.is_none());
        assert_eq!(supervisor.state.sources, super::SourceCatalog::Unloaded);
        if let Some(task) = supervisor
            .origins
            .as_ref()
            .and_then(|origins| origins.announcements.as_ref())
        {
            task.abort();
        }
    }

    #[tokio::test]
    async fn failed_source_refresh_cannot_leave_a_stale_catalog_usable() {
        let (playback, _) = watch::channel(None);
        let mut supervisor = Supervisor::new(playback);
        supervisor.state.sources = super::SourceCatalog::Loading;
        supervisor
            .handle_operation_event(OperationEvent::CaptureSourcesLoaded(Err(
                "display unavailable".into(),
            )))
            .await;
        assert_eq!(
            supervisor.state.sources,
            super::SourceCatalog::Failed("display unavailable".into())
        );
        if let Some(task) = supervisor
            .origins
            .as_ref()
            .and_then(|origins| origins.announcements.as_ref())
        {
            task.abort();
        }
    }

    #[tokio::test]
    async fn stopping_publish_waits_for_cleanup_and_returns_its_failure() {
        let (cancel, mut cancelled) = watch::channel(false);
        let (ack, acknowledged) = tokio::sync::oneshot::channel();
        let (started, stopping) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            cancelled.changed().await.unwrap();
            started.send(()).unwrap();
            acknowledged.await.unwrap();
            Err("portal close failed".to_owned())
        });
        let mut resources = PublishResources {
            task: Some(task),
            cancel: Some(cancel),
            generation: 4,
        };
        let stop = tokio::spawn(async move {
            let result = resources.stop().await;
            (resources, result)
        });
        stopping.await.unwrap();
        assert!(!stop.is_finished());
        ack.send(()).unwrap();
        let (mut resources, result) = stop.await.unwrap();
        assert_eq!(result.unwrap_err(), "portal close failed");
        assert_eq!(resources.generation, 5);
        assert!(resources.task.is_none());
        assert!(resources.cancel.is_none());
        resources.stop().await.unwrap();
    }

    #[tokio::test]
    async fn publish_completion_cannot_block_stop_on_a_full_event_queue() {
        let (events, _receiver) = tokio::sync::mpsc::channel(1);
        events
            .send(OperationEvent::PublishEnded {
                generation: 1,
                result: Ok(()),
            })
            .await
            .unwrap();
        let (cancel, cancelled) = watch::channel(false);
        let task = tokio::spawn(
            PublishCompletion {
                generation: 2,
                events,
                cancelled,
            }
            .report(Err(PublishFailure::Other("close failed".to_owned()))),
        );
        tokio::task::yield_now().await;
        assert!(!task.is_finished());
        cancel.send_replace(true);
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.unwrap_err(), "close failed");
    }

    #[tokio::test]
    async fn failed_publish_stop_is_visible_and_old_completion_is_ignored() {
        let mut supervisor = supervisor();
        supervisor.state.media = MediaState::Publishing;
        supervisor.publish.generation = 7;
        supervisor.publish.task = Some(tokio::spawn(async { Err("close failed".to_owned()) }));
        supervisor.stop_publish().await;
        assert_eq!(supervisor.state.media, MediaState::Idle);
        let error = supervisor.state.last_error.clone().unwrap();
        assert!(error.contains("Screen sharing stopped with an error"));
        supervisor
            .handle_operation_event(OperationEvent::PublishEnded {
                generation: 7,
                result: Ok(()),
            })
            .await;
        supervisor.stop_publish().await;
        assert_eq!(supervisor.state.last_error.as_deref(), Some(error.as_str()));
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
            volume: None,
            generation: 4,
        };

        resources.stop().await.unwrap();

        assert_eq!(resources.generation, 5);
        assert!(resources.task.is_none());
        assert!(resources.cancel.is_none());
        assert!(stopped.load(Ordering::Acquire));
    }

    #[test]
    fn stale_volume_generation_cannot_update_a_new_view() {
        let (volume, mut current) = watch::channel(super::DEFAULT_PLAYER_VOLUME_PERCENT);
        let resources = ViewResources {
            volume: Some(volume),
            generation: 8,
            ..ViewResources::default()
        };

        assert!(!resources.set_volume(7, 25));
        assert_eq!(
            *current.borrow_and_update(),
            super::DEFAULT_PLAYER_VOLUME_PERCENT
        );
        assert!(resources.set_volume(8, 25));
        assert_eq!(*current.borrow_and_update(), 25);
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

    fn discovered_outbound() -> (Supervisor, crate::network::discovery::PeerRecord) {
        let mut supervisor = supervisor();
        supervisor.state.start_discovery();
        let record = peer_record("peer-b");
        let mut peers = crate::network::discovery::PeerRegistry::new("local");
        peers.found(record.clone());
        supervisor.discovery.peers = Some(peers);
        supervisor.project_peer(&record.id, true);
        (supervisor, record)
    }

    async fn rediscover(supervisor: &mut Supervisor, peer: crate::network::discovery::PeerRecord) {
        supervisor
            .handle_service_event(service::Event {
                generation: supervisor.discovery.generation,
                kind: service::EventKind::Found {
                    peer,
                    should_dial: true,
                },
            })
            .await;
    }

    #[tokio::test]
    async fn late_reachable_candidate_connects_and_a_completed_handshake_is_not_replaced() {
        let _ = moq_tokio::rustls::crypto::aws_lc_rs::default_provider().install_default();
        let server = crate::network::server::build().unwrap();
        let addr: std::net::SocketAddr =
            format!("127.0.0.1:{}", server.local_addr().unwrap().port())
                .parse()
                .unwrap();
        let fingerprint = server
            .certificates()
            .fingerprints()
            .into_iter()
            .next()
            .unwrap();
        let accept = tokio::spawn(async move {
            let publish = moq_tokio::origin::spawn();
            let receive = moq_tokio::origin::spawn();
            let mut listener = server.listen().await.unwrap();
            let request = listener.accept().await.unwrap();
            let session = crate::network::server::accept(request, "proof", &publish, receive)
                .await
                .unwrap();
            (session, listener)
        });
        let silent = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let (mut supervisor, mut advert) = discovered_outbound();
        advert.fingerprint = Some(fingerprint);
        advert.addrs = vec![silent.local_addr().unwrap()];
        advert.urls = vec![format!("moqt://{}", advert.addrs[0]).parse().unwrap()];
        supervisor
            .discovery
            .peers
            .as_mut()
            .unwrap()
            .found(advert.clone());
        supervisor.ensure_outbound(&advert.id, true);
        let deadline = supervisor.mesh.outbound[&advert.id]
            .attempt
            .as_ref()
            .unwrap()
            .1;
        tokio::time::timeout_at(deadline, silent.recv(&mut [0; 2048]))
            .await
            .unwrap()
            .unwrap();
        advert.addrs.push(addr);
        rediscover(&mut supervisor, advert.clone()).await;
        let generation = supervisor.mesh.outbound[&advert.id].generation;
        let connection = supervisor.mesh.outbound[&advert.id]
            .connecting
            .as_ref()
            .unwrap()
            .clone();
        let _established = tokio::time::timeout_at(deadline, connection.clone().established())
            .await
            .unwrap()
            .unwrap();
        let (session, listener) = accept.await.unwrap();
        // The observer's SessionReady is still queued, but the transport already won.
        advert.addrs.push("127.0.0.1:9".parse().unwrap());
        rediscover(&mut supervisor, advert.clone()).await;
        assert_eq!(supervisor.mesh.outbound[&advert.id].generation, generation);
        assert_eq!(
            supervisor.mesh.outbound[&advert.id]
                .attempt
                .as_ref()
                .unwrap()
                .1,
            deadline
        );
        assert!(connection.connected());
        supervisor.stop_lan().await;
        session.abort(moq_tokio::moq_net::Error::Cancel);
        listener.close().await;
    }

    #[tokio::test]
    async fn late_candidates_refresh_the_actual_pending_dial() {
        let _ = moq_tokio::rustls::crypto::aws_lc_rs::default_provider().install_default();
        let (mut supervisor, mut advert) = discovered_outbound();
        let silent = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let second = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        advert.fingerprint = Some("00".repeat(32));
        advert.addrs = vec![silent.local_addr().unwrap()];
        advert.urls = vec![format!("moqt://{}", advert.addrs[0]).parse().unwrap()];
        supervisor
            .discovery
            .peers
            .as_mut()
            .unwrap()
            .found(advert.clone());
        supervisor.ensure_outbound(&advert.id, true);
        let before = supervisor.mesh.outbound[&advert.id].generation;
        let deadline = supervisor.mesh.outbound[&advert.id]
            .attempt
            .as_ref()
            .unwrap()
            .1;
        let old = supervisor.mesh.outbound[&advert.id]
            .connecting
            .as_ref()
            .unwrap()
            .clone();
        tokio::time::timeout(
            std::time::Duration::from_secs(3),
            silent.recv(&mut [0; 2048]),
        )
        .await
        .unwrap()
        .unwrap();
        advert.addrs.push(second.local_addr().unwrap());
        rediscover(&mut supervisor, advert.clone()).await;
        assert!(
            supervisor.mesh.outbound[&advert.id].generation > before,
            "new candidates stayed in registry instead of reaching the pending dial"
        );
        let refreshed = supervisor.mesh.outbound[&advert.id].generation;
        assert_eq!(
            supervisor.mesh.outbound[&advert.id]
                .attempt
                .as_ref()
                .unwrap()
                .1,
            deadline
        );
        tokio::time::timeout_at(deadline, second.recv(&mut [0; 2048]))
            .await
            .unwrap()
            .unwrap();
        supervisor.session_ready(
            SessionKey::Outbound(advert.id.clone()),
            before,
            Err("stale failure".into()),
        );
        assert_eq!(supervisor.mesh.outbound[&advert.id].generation, refreshed);
        assert_eq!(
            supervisor.state.peers[&advert.id].transport,
            TransportState::Connecting
        );
        // Repeated/reordered candidates neither restart nor renew the attempt.
        advert.addrs.reverse();
        rediscover(&mut supervisor, advert.clone()).await;
        assert_eq!(supervisor.mesh.outbound[&advert.id].generation, refreshed);
        assert_eq!(
            supervisor.mesh.outbound[&advert.id]
                .attempt
                .as_ref()
                .unwrap()
                .1,
            deadline
        );
        let expired = tokio::time::Instant::now() - std::time::Duration::from_secs(1);
        supervisor
            .mesh
            .outbound
            .get_mut(&advert.id)
            .unwrap()
            .attempt
            .as_mut()
            .unwrap()
            .1 = expired;
        advert.addrs.push("127.0.0.1:9".parse().unwrap());
        rediscover(&mut supervisor, advert.clone()).await;
        assert_eq!(supervisor.mesh.outbound[&advert.id].generation, refreshed);
        let current = supervisor.mesh.outbound[&advert.id]
            .connecting
            .as_ref()
            .unwrap()
            .clone();
        supervisor.stop_lan().await;
        for connection in [&old, &current] {
            use std::{
                future::Future,
                task::{Context, Poll, Waker},
            };
            let mut closed = std::pin::pin!(connection.closed());
            assert!(matches!(
                closed
                    .as_mut()
                    .poll(&mut Context::from_waker(Waker::noop())),
                Poll::Ready(_)
            ));
        }
        let _ = old.closed().await;
    }

    #[tokio::test]
    async fn rediscovery_preserves_pending_dial_and_merges_new_addresses() {
        let (mut supervisor, mut advert) = discovered_outbound();
        let task = tokio::spawn(std::future::pending::<()>());
        let dial = task.abort_handle();
        let resources = supervisor
            .mesh
            .outbound
            .entry(advert.id.clone())
            .or_default();
        resources.generation = 7;
        resources.pending = Some(task);
        supervisor
            .state
            .set_transport(&advert.id, TransportState::Connecting);

        rediscover(&mut supervisor, advert.clone()).await;
        advert.addrs.push("192.0.2.2:4443".parse().unwrap());
        advert.urls.push("moqt://192.0.2.2:4443".parse().unwrap());
        rediscover(&mut supervisor, advert.clone()).await;
        tokio::task::yield_now().await;

        let resources = &supervisor.mesh.outbound[&advert.id];
        assert_eq!(resources.generation, 7);
        assert_eq!(resources.pending.as_ref().unwrap().id(), dial.id());
        assert!(!dial.is_finished());
        assert_eq!(
            supervisor.state.peers[&advert.id].transport,
            TransportState::Connecting
        );
        assert_eq!(
            supervisor
                .discovery
                .peers
                .as_ref()
                .unwrap()
                .get(&advert.id)
                .unwrap()
                .addrs
                .len(),
            2
        );
        supervisor
            .mesh
            .outbound
            .get_mut(&advert.id)
            .unwrap()
            .close();
    }

    #[tokio::test(start_paused = true)]
    async fn rediscovery_preserves_backoff_and_retry_still_starts() {
        let (mut supervisor, advert) = discovered_outbound();
        supervisor
            .mesh
            .outbound
            .entry(advert.id.clone())
            .or_default();
        supervisor
            .state
            .set_transport(&advert.id, TransportState::Failed);
        supervisor.schedule_outbound_retry(&advert.id);
        let resources = &supervisor.mesh.outbound[&advert.id];
        let generation = resources.generation;
        let timer = resources.pending.as_ref().unwrap().id();

        rediscover(&mut supervisor, advert.clone()).await;
        let resources = &supervisor.mesh.outbound[&advert.id];
        assert_eq!(resources.generation, generation);
        assert_eq!(resources.pending.as_ref().unwrap().id(), timer);
        assert_eq!(resources.retry_budget.attempts, 1);
        assert_eq!(
            supervisor.state.peers[&advert.id].transport,
            TransportState::Failed
        );

        tokio::task::yield_now().await;
        tokio::time::advance(super::PEER_RETRY_MAX_DELAY).await;
        let event = supervisor.operation_rx.recv().await.unwrap();
        assert!(
            matches!(&event, OperationEvent::RetryOutbound { peer_id, generation: current }
            if peer_id == &advert.id && *current == generation)
        );
        supervisor.handle_operation_event(event).await;

        let resources = &supervisor.mesh.outbound[&advert.id];
        assert_eq!(resources.generation, generation + 1);
        assert_ne!(resources.pending.as_ref().unwrap().id(), timer);
        assert_eq!(resources.retry_budget.attempts, 1);
        assert_eq!(
            supervisor.state.peers[&advert.id].transport,
            TransportState::Connecting
        );
        supervisor
            .mesh
            .outbound
            .get_mut(&advert.id)
            .unwrap()
            .close();
    }

    #[tokio::test]
    async fn rediscovery_replaces_rotated_credentials_and_ignores_old_dial_result() {
        for lost in [false, true] {
            let (mut supervisor, mut advert) = discovered_outbound();
            let task = tokio::spawn(std::future::pending::<()>());
            let dial = task.abort_handle();
            let resources = supervisor
                .mesh
                .outbound
                .entry(advert.id.clone())
                .or_default();
            resources.generation = 7;
            resources.pending = Some(task);
            if lost {
                supervisor
                    .handle_service_event(service::Event {
                        generation: supervisor.discovery.generation,
                        kind: service::EventKind::Lost(advert.id.clone()),
                    })
                    .await;
            }
            advert.credential = "replacement-proof".into();

            rediscover(&mut supervisor, advert.clone()).await;
            let resources = &supervisor.mesh.outbound[&advert.id];
            let replacement = resources.pending.as_ref().unwrap().id();
            assert_eq!(resources.generation, 8);
            assert_ne!(replacement, dial.id());
            supervisor.session_ready(
                SessionKey::Outbound(advert.id.clone()),
                7,
                Err("old dial".into()),
            );
            assert_eq!(
                supervisor.mesh.outbound[&advert.id]
                    .pending
                    .as_ref()
                    .unwrap()
                    .id(),
                replacement
            );
            assert_eq!(
                supervisor.state.peers[&advert.id].transport,
                TransportState::Connecting
            );
            supervisor
                .mesh
                .outbound
                .get_mut(&advert.id)
                .unwrap()
                .close();
            tokio::task::yield_now().await;
            assert!(dial.is_finished());
        }
    }

    #[tokio::test]
    async fn cleanup_regression_recovery_blocks_new_media_with_old_identity() {
        for exhausted in [false, true] {
            let mut supervisor = supervisor();
            supervisor.state.start_discovery();
            supervisor.state.local_peer_id = Some("previous-service".into());
            assert!(supervisor.state.access().publish());
            if exhausted {
                for _ in 0..DISCOVERY_RETRY_LIMIT {
                    supervisor.recover_discovery("listener lost");
                }
            }
            supervisor.recover_discovery("listener lost");
            assert!(supervisor.discovery.services.is_none());
            assert_eq!(
                supervisor.state.begin_publish(),
                Err(crate::app::StateError::LanNotReady)
            );
            assert_eq!(supervisor.state.media, MediaState::Idle);
            assert!(supervisor.publish.task.is_none());
            supervisor.stop_lan().await;
        }
    }

    #[tokio::test]
    async fn cleanup_regression_restart_revokes_admission_before_launch() {
        let mut supervisor = supervisor();
        supervisor.state.start_discovery();
        supervisor.state.local_peer_id = Some("previous-service".into());
        let old = supervisor.discovery.generation;
        supervisor.restart_discovery();
        // Cancel before yielding so this lifecycle test never starts real mDNS.
        let task = supervisor.discovery.start.take().unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(!supervisor.state.access().ready);
        let current = supervisor.discovery.generation;
        supervisor
            .handle_operation_event(OperationEvent::ServicesStarted {
                generation: old,
                result: Err("late old startup failure".into()),
            })
            .await;
        assert_eq!(supervisor.discovery.generation, current);
        assert!(!supervisor.state.access().ready);
        assert!(supervisor.discovery.retry.is_none());
        supervisor.stop_lan().await;
    }

    #[tokio::test]
    async fn cleanup_regression_recovery_does_not_stop_existing_publication() {
        let mut supervisor = supervisor();
        supervisor.state.start_discovery();
        supervisor.state.local_peer_id = Some("previous-service".into());
        supervisor.state.begin_publish().unwrap();
        supervisor.state.finish_publish().unwrap();
        let task = tokio::spawn(std::future::pending());
        let abort = task.abort_handle();
        supervisor.publish.task = Some(task);
        supervisor.recover_discovery("listener lost");
        assert_eq!(supervisor.state.media, MediaState::Publishing);
        assert!(!abort.is_finished());
        assert!(!supervisor.state.access().ready);
        abort.abort();
        supervisor.stop_lan().await;
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

        let observation = moqcast_lan_state::bandwidth::Observation::default();
        observation.update(
            0,
            moqcast_lan_state::bandwidth::Sample {
                send_bps: Some(123),
                recv_bps: Some(456),
            },
        );
        supervisor.state.peers.get_mut("peer-b").unwrap().bandwidth = observation.view();

        let action = supervisor.session_closed(SessionKey::Outbound("peer-b".into()), 8, None);

        assert!(matches!(action, super::LoopAction::Unchanged));
        assert_eq!(
            supervisor.state.peers["peer-b"].bandwidth.sample().send_bps,
            Some(123)
        );
        assert!(supervisor.mesh.outbound.contains_key("peer-b"));
        assert_eq!(
            supervisor.state.peers["peer-b"].transport,
            TransportState::Connected
        );
        supervisor.session_closed(SessionKey::Outbound("peer-b".into()), 9, None);
        assert_eq!(
            supervisor.state.peers["peer-b"].bandwidth.sample(),
            Default::default()
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

        let retired = &supervisor.mesh.outbound["peer-a"];
        assert!(retired.session.is_none());
        assert!(retired.pending.is_none());
        assert_eq!(retired.generation, 5);
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
