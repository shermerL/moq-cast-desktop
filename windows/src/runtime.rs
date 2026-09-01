//! Background ownership and UI-safe snapshots for the Windows desktop shell.

use std::{
    collections::BTreeMap,
    net::SocketAddr,
    path::PathBuf,
    sync::Arc,
    thread::{self, JoinHandle},
};

use moq_tokio::{mdns, moq_net};
use thiserror::Error;
use tokio::sync::{mpsc, watch};
use url::Url;

use crate::{
    audio::StatusUpdate as AudioStatusUpdate,
    media::{MediaSnapshot, Publication, PublicationFailure, VideoEncodingPolicy},
    playback::{PlaybackFrame, ViewEvent, ViewPhase, ViewSnapshot},
    registry::{PeerRegistry, PeerSummary, RegistryChange, sanitize_identity},
    remote::{Directory as RemoteDirectory, RemoteScreenView, ScreenAvailability},
    session::{
        SessionFoundation, SessionSubject, TransportDirection, TransportPhase, TransportUpdate,
    },
};

const COMMAND_CAPACITY: usize = 32;
const MEDIA_EVENT_CAPACITY: usize = 4;
const VIEW_EVENT_CAPACITY: usize = 8;
const MAX_DEVICE_NAME_CHARS: usize = 64;
const SHORT_SESSION_ID_CHARS: usize = 8;

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
    Stopping,
    Failed,
    Stopped,
}

impl DiscoveryState {
    pub(crate) fn is_active(self) -> bool {
        matches!(self, Self::Starting | Self::Ready | Self::Empty)
    }
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
    pub(crate) screen: ScreenAvailability,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RuntimeSnapshot {
    pub(crate) discovery: DiscoveryState,
    pub(crate) media: MediaSnapshot,
    pub(crate) view: ViewSnapshot,
    pub(crate) remote_screens: BTreeMap<String, RemoteScreenView>,
    pub(crate) peers: BTreeMap<String, PeerView>,
    pub(crate) inbound_sessions: usize,
    pub(crate) listener: Option<String>,
    pub(crate) local_id: Option<String>,
    pub(crate) local_device_name: String,
    pub(crate) lan_session_id: Option<String>,
    pub(crate) version: &'static str,
    pub(crate) last_error: Option<&'static str>,
    pub(crate) stopping: bool,
}

impl Default for RuntimeSnapshot {
    fn default() -> Self {
        Self {
            discovery: DiscoveryState::Starting,
            media: MediaSnapshot::default(),
            view: ViewSnapshot::default(),
            remote_screens: BTreeMap::new(),
            peers: BTreeMap::new(),
            inbound_sessions: 0,
            listener: None,
            local_id: None,
            local_device_name: local_device_name(),
            lan_session_id: None,
            version: env!("CARGO_PKG_VERSION"),
            last_error: None,
            stopping: false,
        }
    }
}

impl RuntimeSnapshot {
    fn begin_scan(&mut self) {
        self.discovery = DiscoveryState::Starting;
        self.last_error = None;
    }

    fn started_scan(&mut self, listener: String, local_id: String) {
        self.discovery = DiscoveryState::Empty;
        self.listener = Some(listener);
        self.lan_session_id = Some(short_lan_session_id(&local_id));
        self.local_id = Some(local_id);
        self.last_error = None;
    }

    fn begin_stop_scan(&mut self) -> bool {
        if !self.discovery.is_active() {
            return false;
        }
        self.discovery = DiscoveryState::Stopping;
        self.last_error = None;
        true
    }

    fn stopped_scan(&mut self) {
        self.clear_lan_state();
        self.discovery = DiscoveryState::Stopped;
        self.last_error = None;
    }

    fn failed_scan(&mut self, message: &'static str) {
        self.clear_lan_state();
        self.discovery = DiscoveryState::Failed;
        self.last_error = Some(message);
    }

    fn clear_lan_state(&mut self) {
        self.peers.clear();
        self.remote_screens.clear();
        self.inbound_sessions = 0;
        self.listener = None;
        self.local_id = None;
        self.lan_session_id = None;
    }

    fn apply_registry(&mut self, change: RegistryChange) {
        match change {
            RegistryChange::Added(peer) | RegistryChange::Updated(peer) => self.upsert(peer),
            RegistryChange::Removed { id } => {
                if self.has_exact_outbound_connection(&id) {
                    let peer = self
                        .peers
                        .get_mut(&id)
                        .expect("connected peer remains in the snapshot");
                    peer.present = false;
                    peer.candidates.clear();
                } else {
                    self.remove_peer(&id);
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

    pub(crate) fn present_peer_count(&self) -> usize {
        self.peers.values().filter(|peer| peer.present).count()
    }

    fn has_exact_outbound_connection(&self, peer: &str) -> bool {
        self.peers.get(peer).is_some_and(|peer| {
            peer.transport.direction == Some(TransportDirectionView::Outbound)
                && peer.transport.phase == TransportPhaseView::Connected
        })
    }

    fn remove_peer(&mut self, peer: &str) {
        self.peers.remove(peer);
        self.remote_screens
            .retain(|_, screen| screen.peer_id != peer);
    }

    fn upsert(&mut self, summary: PeerSummary) {
        let screen = self
            .remote_screens
            .get(&crate::screen_path::for_peer(&summary.id))
            .map_or(ScreenAvailability::Unavailable, |screen| {
                screen.availability
            });
        self.peers
            .entry(summary.id.clone())
            .and_modify(|peer| {
                peer.candidates.clone_from(&summary.candidates);
                peer.should_dial = summary.should_dial;
                peer.authenticated_discovery = summary.authenticated_discovery;
                peer.tls_pinned = summary.tls_pinned;
                peer.present = true;
                peer.screen = screen;
            })
            .or_insert(PeerView {
                id: summary.id,
                candidates: summary.candidates,
                should_dial: summary.should_dial,
                authenticated_discovery: summary.authenticated_discovery,
                tls_pinned: summary.tls_pinned,
                present: true,
                transport: TransportView::default(),
                screen,
            });
    }

    fn update_remote_screen(&mut self, update: crate::remote::Update) -> bool {
        let path = update.path;
        let peer_id = update.view.peer_id.clone();
        let availability = update.view.availability;
        if self
            .remote_screens
            .get(&path)
            .is_some_and(|screen| screen.peer_id == peer_id && screen.availability == availability)
        {
            return false;
        }
        self.remote_screens.insert(path, update.view);
        if let Some(peer) = self.peers.get_mut(&peer_id) {
            peer.screen = availability;
        }
        true
    }

    pub(crate) fn has_mesh_session(&self) -> bool {
        self.inbound_sessions > 0
            || self
                .peers
                .values()
                .any(|peer| peer.transport.phase == TransportPhaseView::Connected)
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
        if !(current.present
            || current.transport.direction == Some(TransportDirectionView::Outbound)
                && current.transport.phase == TransportPhaseView::Connected)
        {
            self.remove_peer(peer);
        }
    }

    fn shutdown(&mut self) {
        self.stopping = true;
        self.stopped_scan();
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RuntimeCommand {
    StartScan,
    StopScan,
    SetVideoEncodingPolicy(VideoEncodingPolicy),
    ShareScreen,
    StopSharing,
    WatchScreen { path: String },
    StopWatching,
    Shutdown,
}

enum MediaEvent {
    Ended {
        generation: u64,
        result: Result<(), PublicationFailure>,
    },
}

#[derive(Default)]
struct PublicationOwner {
    generation: u64,
    task: Option<tokio::task::JoinHandle<()>>,
}

#[derive(Default)]
struct ViewOwner {
    generation: u64,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl ViewOwner {
    fn start(
        &mut self,
        generation: u64,
        path: String,
        broadcast: moq_net::broadcast::Consumer,
        events: mpsc::Sender<ViewEvent>,
        frames: watch::Sender<Option<Arc<PlaybackFrame>>>,
    ) {
        self.generation = generation;
        self.task = Some(tokio::spawn(crate::playback::run(
            generation, path, broadcast, events, frames,
        )));
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

impl PublicationOwner {
    fn start(
        &mut self,
        generation: u64,
        publication: crate::media::ReadyPublication,
        events: mpsc::Sender<MediaEvent>,
        audio_updates: watch::Sender<Option<AudioStatusUpdate>>,
    ) {
        self.generation = generation;
        self.task = Some(tokio::spawn(async move {
            let result = publication.run(generation, audio_updates).await;
            let _ = events.send(MediaEvent::Ended { generation, result }).await;
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
    playback: watch::Receiver<Option<Arc<PlaybackFrame>>>,
    thread: Option<JoinHandle<()>>,
}

impl RuntimeOwner {
    pub(crate) fn start(config: RuntimeConfig) -> Result<Self, RuntimeStartError> {
        let (commands, command_rx) = mpsc::channel(COMMAND_CAPACITY);
        let (snapshot_tx, snapshots) = watch::channel(RuntimeSnapshot::default());
        let (playback_tx, playback) = watch::channel(None);
        let thread = thread::Builder::new()
            .name("moqcast-windows-runtime".to_owned())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build();
                match runtime {
                    Ok(runtime) => {
                        runtime.block_on(run(config, command_rx, snapshot_tx, playback_tx))
                    }
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
            playback,
            thread: Some(thread),
        })
    }

    pub(crate) fn snapshot(&mut self) -> RuntimeSnapshot {
        self.snapshots.borrow_and_update().clone()
    }

    pub(crate) fn playback_frame(&mut self) -> Option<Arc<PlaybackFrame>> {
        self.playback.borrow_and_update().clone()
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

#[derive(Default)]
struct ServiceLifecycle {
    generation: u64,
    active: bool,
}

impl ServiceLifecycle {
    fn begin_start(&mut self) -> Option<u64> {
        if self.active {
            return None;
        }
        self.generation = self.generation.wrapping_add(1);
        self.active = true;
        Some(self.generation)
    }

    fn fail(&mut self, generation: u64) {
        if self.generation == generation {
            self.active = false;
        }
    }

    fn stop(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.active = false;
    }

    fn accepts(&self, generation: u64) -> bool {
        self.active && self.generation == generation
    }
}

struct LanServices {
    generation: u64,
    discovery: mdns::Discovery,
    registry: PeerRegistry,
    sessions: SessionFoundation,
    remote: RemoteDirectory,
    raw_peers: BTreeMap<String, mdns::Peer>,
}

impl LanServices {
    async fn shutdown(self) {
        self.remote.stop().await;
        self.sessions.shutdown().await;
        drop(self.discovery);
    }
}

#[derive(Default)]
struct ServiceOwner {
    lifecycle: ServiceLifecycle,
    services: Option<LanServices>,
}

impl ServiceOwner {
    async fn start(
        &mut self,
        config: &RuntimeConfig,
        snapshot: &mut RuntimeSnapshot,
        snapshots: &watch::Sender<RuntimeSnapshot>,
    ) {
        let Some(generation) = self.lifecycle.begin_start() else {
            return;
        };
        snapshot.begin_scan();
        let _ = snapshots.send(snapshot.clone());
        match start_services(config, generation).await {
            Ok(services) if self.lifecycle.accepts(generation) => {
                let listener = services.sessions.advertisement().addr.to_string();
                let local_id = sanitize_identity(services.discovery.id());
                snapshot.started_scan(listener, local_id);
                self.services = Some(services);
            }
            Ok(services) => services.shutdown().await,
            Err(message) => {
                self.lifecycle.fail(generation);
                snapshot.failed_scan(message);
            }
        }
    }

    async fn stop(&mut self, snapshot: &mut RuntimeSnapshot) {
        self.lifecycle.stop();
        if let Some(services) = self.services.take() {
            services.shutdown().await;
        }
        snapshot.stopped_scan();
    }

    async fn fail(&mut self, snapshot: &mut RuntimeSnapshot, message: &'static str) {
        let generation = self.lifecycle.generation;
        self.lifecycle.fail(generation);
        if let Some(services) = self.services.take() {
            services.shutdown().await;
        }
        snapshot.failed_scan(message);
    }
}

enum RuntimeInput {
    Command(Option<RuntimeCommand>),
    Discovery {
        generation: u64,
        event: Option<mdns::Event>,
    },
    Session {
        generation: u64,
        update: Option<TransportUpdate>,
    },
    Remote {
        generation: u64,
        update: Option<crate::remote::Update>,
    },
    Media(Option<MediaEvent>),
    View(Option<ViewEvent>),
    Audio(Result<(), watch::error::RecvError>),
}

async fn run(
    config: RuntimeConfig,
    mut commands: mpsc::Receiver<RuntimeCommand>,
    snapshots: watch::Sender<RuntimeSnapshot>,
    playback: watch::Sender<Option<Arc<PlaybackFrame>>>,
) {
    let mut snapshot = RuntimeSnapshot::default();
    let (media_events, mut media_recv) = mpsc::channel(MEDIA_EVENT_CAPACITY);
    let (audio_updates, mut audio_recv) = watch::channel(None::<AudioStatusUpdate>);
    let (view_events, mut view_recv) = mpsc::channel(VIEW_EVENT_CAPACITY);
    let mut publication = PublicationOwner::default();
    let mut view = ViewOwner::default();
    let mut service_owner = ServiceOwner::default();
    service_owner
        .start(&config, &mut snapshot, &snapshots)
        .await;
    let _ = snapshots.send(snapshot.clone());

    loop {
        let input = if let Some(services) = service_owner.services.as_mut() {
            let generation = services.generation;
            tokio::select! {
                command = commands.recv() => RuntimeInput::Command(command),
                event = services.discovery.recv() => RuntimeInput::Discovery { generation, event },
                update = services.sessions.recv() => RuntimeInput::Session { generation, update },
                update = services.remote.recv() => RuntimeInput::Remote { generation, update },
                event = media_recv.recv() => RuntimeInput::Media(event),
                event = view_recv.recv() => RuntimeInput::View(event),
                changed = audio_recv.changed() => RuntimeInput::Audio(changed),
            }
        } else {
            tokio::select! {
                command = commands.recv() => RuntimeInput::Command(command),
                event = media_recv.recv() => RuntimeInput::Media(event),
                event = view_recv.recv() => RuntimeInput::View(event),
                changed = audio_recv.changed() => RuntimeInput::Audio(changed),
            }
        };

        match input {
            RuntimeInput::Command(command) => {
                let Some(command) = command else { break };
                if handle_command(
                    command,
                    RuntimeContext {
                        config: &config,
                        snapshots: &snapshots,
                        snapshot: &mut snapshot,
                        services: &mut service_owner,
                        publication: &mut publication,
                        media_events: &media_events,
                        view: &mut view,
                        view_events: &view_events,
                        playback: &playback,
                        audio_updates: &audio_updates,
                    },
                )
                .await
                {
                    break;
                }
            }
            RuntimeInput::Discovery { generation, event } => {
                if !service_owner.lifecycle.accepts(generation) {
                    continue;
                }
                let Some(event) = event else {
                    stop_active_media(&mut snapshot, &mut publication, &mut view, &playback).await;
                    service_owner
                        .fail(&mut snapshot, "LAN discovery stopped unexpectedly.")
                        .await;
                    let _ = snapshots.send(snapshot.clone());
                    continue;
                };
                let services = service_owner
                    .services
                    .as_mut()
                    .expect("current discovery generation owns services");
                match event {
                    mdns::Event::Found(peer) => {
                        let should_dial = services.discovery.should_dial(&peer.id);
                        let raw_id = peer.id.clone();
                        let key = sanitize_identity(&raw_id);
                        let change = services.registry.found(&peer, should_dial);
                        services.raw_peers.insert(key.clone(), peer);
                        snapshot.apply_registry(change);
                        if should_dial {
                            auto_connect(
                                &key,
                                &mut snapshot,
                                &services.raw_peers,
                                &mut services.sessions,
                            )
                            .await;
                        } else if let Some(update) = services.sessions.disconnect(&raw_id).await {
                            apply_session_update(&mut snapshot, update);
                        }
                    }
                    mdns::Event::Lost(raw_id) => {
                        let key = sanitize_identity(&raw_id);
                        let keep_session = snapshot.has_exact_outbound_connection(&key);
                        services.raw_peers.remove(&key);
                        snapshot.apply_registry(services.registry.lost(&raw_id));
                        if !keep_session
                            && let Some(update) = services.sessions.disconnect(&raw_id).await
                        {
                            apply_session_update(&mut snapshot, update);
                        }
                    }
                    _ => {}
                }
            }
            RuntimeInput::Session { generation, update } => {
                if !service_owner.lifecycle.accepts(generation) {
                    continue;
                }
                let Some(update) = update else {
                    stop_active_media(&mut snapshot, &mut publication, &mut view, &playback).await;
                    service_owner
                        .fail(
                            &mut snapshot,
                            "The direct session listener stopped unexpectedly.",
                        )
                        .await;
                    let _ = snapshots.send(snapshot.clone());
                    continue;
                };
                apply_session_update(&mut snapshot, update);
            }
            RuntimeInput::Media(event) => {
                let Some(MediaEvent::Ended { generation, result }) = event else {
                    continue;
                };
                publication.finished(generation);
                snapshot.media.ended(generation, result);
            }
            RuntimeInput::Remote { generation, update } => {
                if !service_owner.lifecycle.accepts(generation) {
                    continue;
                }
                let Some(update) = update else {
                    stop_active_media(&mut snapshot, &mut publication, &mut view, &playback).await;
                    service_owner
                        .fail(
                            &mut snapshot,
                            "The remote screen directory stopped unexpectedly.",
                        )
                        .await;
                    let _ = snapshots.send(snapshot.clone());
                    continue;
                };
                snapshot.update_remote_screen(update);
            }
            RuntimeInput::View(event) => {
                let Some(event) = event else {
                    continue;
                };
                match event {
                    ViewEvent::DecoderReady {
                        generation,
                        path,
                        decoder,
                        width,
                        height,
                    } => {
                        tracing::info!(
                            view_generation = generation,
                            path = %path,
                            decoder = %decoder,
                            width,
                            height,
                            "remote video decoder produced its first frame"
                        );
                        snapshot
                            .view
                            .decoder_ready(generation, &path, decoder, width, height);
                    }
                    ViewEvent::AudioChanged {
                        generation,
                        path,
                        audio,
                    } => {
                        snapshot.view.audio_changed(generation, &path, audio);
                    }
                    ViewEvent::Ended { generation, result } => {
                        tracing::info!(
                            view_generation = generation,
                            failed = result.is_err(),
                            "remote screen subscription ended"
                        );
                        view.finished(generation);
                        playback.send_replace(None);
                        snapshot.view.ended(generation, result);
                    }
                }
            }
            RuntimeInput::Audio(changed) => {
                if changed.is_err() {
                    continue;
                }
                let Some(update) = *audio_recv.borrow_and_update() else {
                    continue;
                };
                snapshot.media.audio.apply(update);
            }
        }
        let _ = snapshots.send(snapshot.clone());
    }

    stop_active_media(&mut snapshot, &mut publication, &mut view, &playback).await;
    service_owner.stop(&mut snapshot).await;
    snapshot.shutdown();
    let _ = snapshots.send(snapshot);
}

async fn start_services(
    config: &RuntimeConfig,
    generation: u64,
) -> Result<LanServices, &'static str> {
    let bound = match SessionFoundation::bind(config.bind) {
        Ok(bound) => bound,
        Err(_) => return Err("The direct session listener could not bind."),
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
            Err(_) => return Err("The LAN discovery secret could not be loaded."),
        };
        discovery_config = discovery_config.with_secret(secret);
    }
    let discovery = match discovery_config.advertise().await {
        Ok(discovery) => discovery,
        Err(_) => return Err("LAN discovery could not start."),
    };
    let registry = PeerRegistry::new(discovery.id(), authenticated);
    let sessions = match bound.start(discovery.credential().to_owned()).await {
        Ok(sessions) => sessions,
        Err(_) => return Err("The direct session listener could not start."),
    };
    let local_id = sanitize_identity(discovery.id());
    let remote = RemoteDirectory::start(sessions.receive_origin().clone(), local_id);
    Ok(LanServices {
        generation,
        discovery,
        registry,
        sessions,
        remote,
        raw_peers: BTreeMap::new(),
    })
}

fn sanitize_device_name(raw: &str) -> String {
    let normalized = raw
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    let bounded = normalized
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_DEVICE_NAME_CHARS)
        .collect::<String>();
    if bounded.is_empty() {
        "Windows device".to_owned()
    } else {
        bounded
    }
}

fn short_lan_session_id(id: &str) -> String {
    id.chars().take(SHORT_SESSION_ID_CHARS).collect()
}

#[cfg(target_os = "windows")]
fn local_device_name() -> String {
    use windows::{
        Win32::System::SystemInformation::{ComputerNameDnsHostname, GetComputerNameExW},
        core::PWSTR,
    };

    let mut buffer = [0_u16; 256];
    let mut length = buffer.len() as u32;
    let result = unsafe {
        GetComputerNameExW(
            ComputerNameDnsHostname,
            Some(PWSTR(buffer.as_mut_ptr())),
            &mut length,
        )
    };
    if result.is_ok() {
        sanitize_device_name(&String::from_utf16_lossy(&buffer[..length as usize]))
    } else {
        "Windows device".to_owned()
    }
}

#[cfg(not(target_os = "windows"))]
fn local_device_name() -> String {
    sanitize_device_name(&std::env::var("HOSTNAME").unwrap_or_default())
}

struct RuntimeContext<'a> {
    config: &'a RuntimeConfig,
    snapshots: &'a watch::Sender<RuntimeSnapshot>,
    snapshot: &'a mut RuntimeSnapshot,
    services: &'a mut ServiceOwner,
    publication: &'a mut PublicationOwner,
    media_events: &'a mpsc::Sender<MediaEvent>,
    view: &'a mut ViewOwner,
    view_events: &'a mpsc::Sender<ViewEvent>,
    playback: &'a watch::Sender<Option<Arc<PlaybackFrame>>>,
    audio_updates: &'a watch::Sender<Option<AudioStatusUpdate>>,
}

async fn handle_command(command: RuntimeCommand, context: RuntimeContext<'_>) -> bool {
    let RuntimeContext {
        config,
        snapshots,
        snapshot,
        services,
        publication,
        media_events,
        view,
        view_events,
        playback,
        audio_updates,
    } = context;
    match command {
        RuntimeCommand::StartScan => {
            services.start(config, snapshot, snapshots).await;
            false
        }
        RuntimeCommand::StopScan => {
            if !snapshot.begin_stop_scan() {
                return false;
            }
            let _ = snapshots.send(snapshot.clone());
            stop_active_media(snapshot, publication, view, playback).await;
            services.stop(snapshot).await;
            false
        }
        RuntimeCommand::SetVideoEncodingPolicy(policy) => {
            snapshot.media.set_video_encoding_policy(policy);
            false
        }
        RuntimeCommand::ShareScreen => {
            if let Some(active) = services.services.as_ref() {
                start_publication(
                    snapshot,
                    &active.sessions,
                    publication,
                    media_events,
                    audio_updates,
                )
                .await;
            } else {
                snapshot.last_error = Some("Start LAN discovery before sharing.");
            }
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
        RuntimeCommand::WatchScreen { path } => {
            if let Some(active) = services.services.as_ref() {
                start_view(path, snapshot, &active.remote, view, view_events, playback);
            } else {
                snapshot.last_error = Some("Start LAN discovery before watching.");
            }
            false
        }
        RuntimeCommand::StopWatching => {
            let Some(generation) = snapshot.view.begin_stop() else {
                return false;
            };
            view.stop().await;
            playback.send_replace(None);
            snapshot.view.stopped(generation);
            false
        }
        RuntimeCommand::Shutdown => true,
    }
}

async fn stop_active_media(
    snapshot: &mut RuntimeSnapshot,
    publication: &mut PublicationOwner,
    view: &mut ViewOwner,
    playback: &watch::Sender<Option<Arc<PlaybackFrame>>>,
) {
    if let Some(generation) = snapshot.view.begin_stop() {
        view.stop().await;
        playback.send_replace(None);
        snapshot.view.stopped(generation);
    } else {
        view.stop().await;
        playback.send_replace(None);
    }
    if let Some(generation) = snapshot.media.begin_stop() {
        publication.stop().await;
        snapshot.media.stopped(generation);
    } else {
        publication.stop().await;
    }
}

async fn start_publication(
    snapshot: &mut RuntimeSnapshot,
    sessions: &SessionFoundation,
    publication: &mut PublicationOwner,
    media_events: &mpsc::Sender<MediaEvent>,
    audio_updates: &watch::Sender<Option<AudioStatusUpdate>>,
) {
    if !matches!(snapshot.view.phase, ViewPhase::Idle | ViewPhase::Failed) {
        snapshot.last_error = Some("Stop watching before sharing the local screen.");
        return;
    }
    let Some(local_id) = snapshot.local_id.clone() else {
        snapshot.last_error = Some("LAN services must be ready before sharing.");
        return;
    };
    let Some(generation) = snapshot.media.begin(&local_id) else {
        return;
    };
    let policy = snapshot.media.video_encoding;
    let prepared = match Publication::prepare(sessions.publish_origin(), &local_id) {
        Ok(prepared) => prepared,
        Err(error) => {
            tracing::warn!(stage = "publish", %error, "screen publication preparation failed");
            snapshot
                .media
                .ended(generation, Err(PublicationFailure::Unexpected));
            return;
        }
    };
    let ready = match prepared.configure(policy).await {
        Ok(ready) => ready,
        Err(error) => {
            snapshot.media.ended(generation, Err(error));
            return;
        }
    };
    let info = ready.info();
    if snapshot.media.started(generation, info) {
        tracing::info!(
            generation,
            video_policy = ?policy,
            source_width = info.width,
            source_height = info.height,
            "screen publication configured"
        );
        publication.start(
            generation,
            ready,
            media_events.clone(),
            audio_updates.clone(),
        );
    }
}

fn start_view(
    path: String,
    snapshot: &mut RuntimeSnapshot,
    remote: &RemoteDirectory,
    view: &mut ViewOwner,
    events: &mpsc::Sender<ViewEvent>,
    playback: &watch::Sender<Option<Arc<PlaybackFrame>>>,
) {
    if !snapshot.has_mesh_session() {
        snapshot.last_error = Some("A direct peer session is required before watching.");
        return;
    }
    if !matches!(
        snapshot.media.phase,
        crate::media::MediaPhase::Idle | crate::media::MediaPhase::Failed
    ) {
        snapshot.last_error = Some("Stop sharing before watching a remote screen.");
        return;
    }
    let available = snapshot
        .remote_screens
        .get(&path)
        .is_some_and(|screen| screen.availability == ScreenAvailability::Available);
    if !available {
        snapshot.last_error = Some("The selected remote screen is no longer available.");
        return;
    }
    let Some(broadcast) = remote.broadcast(&path) else {
        snapshot.last_error = Some("The selected remote screen is no longer available.");
        return;
    };
    let Some(generation) = snapshot.view.begin(&path) else {
        snapshot.last_error = Some("Another remote screen is already active.");
        return;
    };
    tracing::info!(
        view_generation = generation,
        path = %path,
        "remote screen subscription starting"
    );
    snapshot.last_error = None;
    playback.send_replace(None);
    view.start(
        generation,
        path,
        broadcast,
        events.clone(),
        playback.clone(),
    );
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
            let peer = sanitize_identity(&raw_id);
            let generation = state.generation();
            let direction = match state.direction() {
                TransportDirection::Inbound => TransportDirectionView::Inbound,
                TransportDirection::Outbound => TransportDirectionView::Outbound,
            };
            let phase = match state.phase() {
                TransportPhase::Connecting => TransportPhaseView::Connecting,
                TransportPhase::Connected => TransportPhaseView::Connected,
                TransportPhase::Rejected => TransportPhaseView::Rejected,
                TransportPhase::Failed => TransportPhaseView::Failed,
                TransportPhase::Disconnected => TransportPhaseView::Disconnected,
            };
            tracing::info!(
                peer = %peer,
                generation,
                direction = ?direction,
                phase = ?phase,
                "peer transport state changed"
            );
            snapshot.apply_transport(
                &peer,
                TransportView {
                    generation,
                    direction: Some(direction),
                    phase,
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
        assert!(!snapshot.peers.contains_key("peer"));
        assert_eq!(snapshot.discovery, DiscoveryState::Empty);
    }

    #[test]
    fn lost_keeps_only_an_exact_outbound_connected_peer_until_disconnect() {
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
        assert!(snapshot.update_remote_screen(crate::remote::Update {
            path: "moqcast.screen/peer".to_owned(),
            view: RemoteScreenView {
                peer_id: "peer".to_owned(),
                availability: ScreenAvailability::Available,
            },
        }));

        snapshot.apply_registry(RegistryChange::Removed {
            id: "peer".to_owned(),
        });
        assert!(!snapshot.peers["peer"].present);
        assert!(snapshot.remote_screens.contains_key("moqcast.screen/peer"));

        snapshot.apply_transport(
            "peer",
            TransportView {
                generation,
                direction: Some(TransportDirectionView::Outbound),
                phase: TransportPhaseView::Disconnected,
            },
        );
        assert!(!snapshot.peers.contains_key("peer"));
        assert!(!snapshot.remote_screens.contains_key("moqcast.screen/peer"));
    }

    #[test]
    fn lost_does_not_keep_an_inbound_connected_peer() {
        let mut snapshot = RuntimeSnapshot::default();
        snapshot.apply_registry(RegistryChange::Added(peer("peer", "moqt://one:4443")));
        snapshot.apply_transport(
            "peer",
            TransportView {
                generation: 1,
                direction: Some(TransportDirectionView::Inbound),
                phase: TransportPhaseView::Connected,
            },
        );

        snapshot.apply_registry(RegistryChange::Removed {
            id: "peer".to_owned(),
        });
        assert!(!snapshot.peers.contains_key("peer"));
    }

    #[test]
    fn peer_count_only_includes_present_devices() {
        let mut snapshot = RuntimeSnapshot::default();
        snapshot.apply_registry(RegistryChange::Added(peer("connected", "moqt://one:4443")));
        let generation = snapshot.begin_auto_connect("connected").expect("connect");
        snapshot.apply_transport(
            "connected",
            TransportView {
                generation,
                direction: Some(TransportDirectionView::Outbound),
                phase: TransportPhaseView::Connected,
            },
        );
        snapshot.apply_registry(RegistryChange::Added(peer("present", "moqt://two:4443")));
        snapshot.apply_registry(RegistryChange::Removed {
            id: "connected".to_owned(),
        });

        assert_eq!(snapshot.present_peer_count(), 1);
        assert_eq!(snapshot.peers.len(), 2);
    }

    #[test]
    fn local_device_name_is_sanitized_and_bounded() {
        assert_eq!(sanitize_device_name("  Desk\0top\n  "), "Desk top");
        assert_eq!(sanitize_device_name("\u{0000}\n"), "Windows device");
        assert_eq!(sanitize_device_name(&"a".repeat(80)).chars().count(), 64);
    }

    #[test]
    fn lan_session_id_is_short_and_not_a_stable_device_label() {
        assert_eq!(short_lan_session_id("0123456789abcdef"), "01234567");
        assert_eq!(short_lan_session_id("tiny"), "tiny");
    }

    #[test]
    fn service_generation_supports_start_stop_start() {
        let mut lifecycle = ServiceLifecycle::default();
        let first = lifecycle.begin_start().expect("first start");
        assert!(lifecycle.accepts(first));

        lifecycle.stop();
        assert!(!lifecycle.accepts(first));
        let second = lifecycle.begin_start().expect("second start");

        assert!(second > first);
        assert!(lifecycle.accepts(second));
    }

    #[test]
    fn failed_service_generation_can_retry_without_accepting_stale_events() {
        let mut lifecycle = ServiceLifecycle::default();
        let failed = lifecycle.begin_start().expect("failed start");
        lifecycle.fail(failed);
        assert!(!lifecycle.accepts(failed));

        let retry = lifecycle.begin_start().expect("retry start");
        assert!(lifecycle.accepts(retry));
        assert!(!lifecycle.accepts(failed));
    }

    #[test]
    fn stale_service_generation_cannot_pollute_a_new_snapshot() {
        let mut lifecycle = ServiceLifecycle::default();
        let stale = lifecycle.begin_start().expect("first start");
        lifecycle.stop();
        let current = lifecycle.begin_start().expect("second start");
        let mut snapshot = RuntimeSnapshot::default();

        if lifecycle.accepts(stale) {
            snapshot.apply_registry(RegistryChange::Added(peer("stale", "moqt://old:4443")));
        }
        if lifecycle.accepts(current) {
            snapshot.apply_registry(RegistryChange::Added(peer("current", "moqt://new:4443")));
        }

        assert!(!snapshot.peers.contains_key("stale"));
        assert!(snapshot.peers.contains_key("current"));
    }

    #[test]
    fn stopping_scan_clears_lan_state_but_keeps_media_policy() {
        let mut snapshot = RuntimeSnapshot::default();
        snapshot.started_scan("[::]:4443".to_owned(), "local-session".to_owned());
        snapshot.apply_registry(RegistryChange::Added(peer("peer", "moqt://one:4443")));
        snapshot
            .media
            .set_video_encoding_policy(VideoEncodingPolicy::NativeQhdHardware);

        assert!(snapshot.begin_stop_scan());
        assert_eq!(snapshot.discovery, DiscoveryState::Stopping);
        assert!(!snapshot.begin_stop_scan());
        snapshot.stopped_scan();

        assert_eq!(snapshot.discovery, DiscoveryState::Stopped);
        assert!(snapshot.peers.is_empty());
        assert!(snapshot.remote_screens.is_empty());
        assert!(snapshot.local_id.is_none());
        assert!(snapshot.lan_session_id.is_none());
        assert_eq!(
            snapshot.media.video_encoding,
            VideoEncodingPolicy::NativeQhdHardware
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
    fn audio_failure_does_not_end_video_publication() {
        let mut snapshot = RuntimeSnapshot::default();
        let generation = snapshot.media.begin("local").expect("share");
        snapshot.media.started(
            generation,
            crate::media::PublicationInfo {
                width: 1920,
                height: 1080,
            },
        );

        assert!(snapshot.media.audio.apply(AudioStatusUpdate {
            generation,
            event: crate::audio::AudioEvent::Failed(crate::audio::AudioIssue::CaptureBackend),
        }));
        assert_eq!(snapshot.media.phase, crate::media::MediaPhase::Sharing);
        assert_eq!(snapshot.media.audio.phase, crate::audio::AudioPhase::Failed);
    }

    #[test]
    fn video_encoding_policy_is_locked_while_sharing_and_editable_after_stop() {
        let mut snapshot = RuntimeSnapshot::default();
        assert!(
            snapshot
                .media
                .set_video_encoding_policy(crate::media::VideoEncodingPolicy::NativeQhdHardware)
        );

        let generation = snapshot.media.begin("local").expect("share");
        assert!(
            !snapshot
                .media
                .set_video_encoding_policy(crate::media::VideoEncodingPolicy::Compatible)
        );
        assert_eq!(
            snapshot.media.video_encoding,
            crate::media::VideoEncodingPolicy::NativeQhdHardware
        );

        snapshot.media.begin_stop();
        snapshot.media.stopped(generation);
        assert!(
            snapshot
                .media
                .set_video_encoding_policy(crate::media::VideoEncodingPolicy::Compatible)
        );
        assert_eq!(
            snapshot.media.video_encoding,
            crate::media::VideoEncodingPolicy::Compatible
        );
    }

    #[test]
    fn canonical_announcements_update_the_matching_peer_without_starting_view() {
        let mut snapshot = RuntimeSnapshot {
            local_id: Some("local".to_owned()),
            ..RuntimeSnapshot::default()
        };
        snapshot.apply_registry(RegistryChange::Added(peer("peer", "moqt://one:4443")));

        let update = |path: &str, peer_id: &str, availability| crate::remote::Update {
            path: path.to_owned(),
            view: RemoteScreenView {
                peer_id: peer_id.to_owned(),
                availability,
            },
        };

        assert!(snapshot.update_remote_screen(update(
            "moqcast.screen/peer",
            "peer",
            ScreenAvailability::Available,
        )));
        assert_eq!(snapshot.peers["peer"].screen, ScreenAvailability::Available);
        assert_eq!(snapshot.view.phase, ViewPhase::Idle);

        assert!(snapshot.update_remote_screen(update(
            "moqcast.screen/peer",
            "peer",
            ScreenAvailability::Withdrawn,
        )));
        assert_eq!(snapshot.peers["peer"].screen, ScreenAvailability::Withdrawn);
        assert_eq!(snapshot.view.phase, ViewPhase::Idle);
    }

    #[test]
    fn dropping_runtime_owner_sends_shutdown_and_joins_thread() {
        let (commands, mut command_rx) = mpsc::channel(1);
        let (_snapshot_tx, snapshots) = watch::channel(RuntimeSnapshot::default());
        let (_playback_tx, playback) = watch::channel(None);
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
            playback,
            thread: Some(thread),
        };

        drop(owner);

        assert!(stopped.load(Ordering::SeqCst));
    }
}
