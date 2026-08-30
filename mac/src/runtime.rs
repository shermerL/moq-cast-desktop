//! Generation-guarded application lifecycle owned outside the UI thread.

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use thiserror::Error;
use tokio::sync::{mpsc, watch};

use crate::network::{self, Event as NetworkEvent, PeerSession};
use crate::playback::{self, Event as PlaybackEvent, Frame as PlaybackFrame};
use crate::publication::{self, Event as PublicationEvent, Selection as ShareSelection};
use crate::remote::{ScreenAvailability, ScreenView};

const COMMAND_CAPACITY: usize = 32;
const PLAYBACK_EVENT_CAPACITY: usize = 8;

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
    Available,
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
    PreparingWatch,
    Watching,
    PreparingShare,
    Sharing,
    Stopping,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MediaOwner {
    Watch,
    Share,
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
    pub(crate) remote_screens: BTreeMap<String, ScreenView>,
    pub(crate) inbound_sessions: usize,
    pub(crate) nearby_issue: Option<NearbyIssue>,
    pub(crate) media_peer: Option<String>,
    pub(crate) media_path: Option<String>,
    pub(crate) media_owner: Option<MediaOwner>,
    pub(crate) share_selection: Option<ShareSelection>,
    pub(crate) media_decoder: Option<String>,
    pub(crate) media_width: Option<u32>,
    pub(crate) media_height: Option<u32>,
    pub(crate) media_error: Option<String>,
}

impl Default for AppSnapshot {
    fn default() -> Self {
        Self {
            runtime: Lifecycle::new(RuntimePhase::Starting),
            discovery: Lifecycle::new(DiscoveryPhase::Starting),
            session: Lifecycle::new(SessionPhase::Starting),
            media: Lifecycle::new(MediaPhase::Idle),
            capture: Lifecycle::new(CapabilityPhase::Available),
            decoder: Lifecycle::new(CapabilityPhase::Available),
            local_device_name: local_device_name(),
            peers: BTreeMap::new(),
            remote_screens: BTreeMap::new(),
            inbound_sessions: 0,
            nearby_issue: None,
            media_peer: None,
            media_path: None,
            media_owner: None,
            share_selection: None,
            media_decoder: None,
            media_width: None,
            media_height: None,
            media_error: None,
        }
    }
}

impl AppSnapshot {
    pub(crate) fn can_watch(&self, peer: &str, path: &str) -> bool {
        self.media.phase() == MediaPhase::Idle
            && self.peers.contains_key(peer)
            && path == crate::contract::screen_path(peer)
            && self.remote_screens.get(path).is_some_and(|screen| {
                screen.peer_id == peer && screen.availability == ScreenAvailability::Available
            })
    }

    fn begin_watch(&mut self, peer: &str, path: &str) -> Option<Generation> {
        if self.media.phase() != MediaPhase::Idle {
            return None;
        }
        let generation = self.media.begin(MediaPhase::PreparingWatch);
        self.media_owner = Some(MediaOwner::Watch);
        self.media_peer = Some(peer.to_owned());
        self.media_path = Some(path.to_owned());
        self.media_decoder = None;
        self.media_width = None;
        self.media_height = None;
        self.media_error = None;
        Some(generation)
    }

    fn playback_started(
        &mut self,
        generation: Generation,
        decoder: String,
        width: u32,
        height: u32,
    ) -> bool {
        if self.media_owner != Some(MediaOwner::Watch)
            || !matches!(
                self.media.phase(),
                MediaPhase::PreparingWatch | MediaPhase::Watching
            )
            || !self.media.apply(generation, MediaPhase::Watching)
        {
            return false;
        }
        self.media_decoder = Some(decoder);
        self.media_width = Some(width);
        self.media_height = Some(height);
        true
    }

    fn playback_ended(&mut self, generation: Generation, result: Result<(), String>) -> bool {
        if self.media_owner != Some(MediaOwner::Watch)
            || self.media.generation() != generation
            || !matches!(
                self.media.phase(),
                MediaPhase::PreparingWatch | MediaPhase::Watching
            )
        {
            return false;
        }
        match result {
            Ok(()) => self.clear_media(MediaPhase::Idle, None),
            Err(error) => {
                self.media.apply(generation, MediaPhase::Failed);
                self.media_decoder = None;
                self.media_width = None;
                self.media_height = None;
                self.media_error = Some(error);
            }
        }
        true
    }

    fn begin_stop_watch(&mut self) -> Option<Generation> {
        if self.media_owner != Some(MediaOwner::Watch)
            || !matches!(
                self.media.phase(),
                MediaPhase::PreparingWatch | MediaPhase::Watching | MediaPhase::Failed
            )
        {
            return None;
        }
        Some(self.media.begin(MediaPhase::Stopping))
    }

    fn finish_stop_media(&mut self, generation: Generation) -> bool {
        if !self.media.apply(generation, MediaPhase::Idle) {
            return false;
        }
        self.clear_media_fields(None);
        true
    }

    fn select_share_source(&mut self, selection: ShareSelection) -> bool {
        if !matches!(self.media.phase(), MediaPhase::Idle | MediaPhase::Failed) {
            return false;
        }
        self.share_selection = Some(selection);
        true
    }

    fn begin_share(&mut self) -> Option<(Generation, ShareSelection)> {
        if self.media.phase() != MediaPhase::Idle {
            return None;
        }
        let selection = self.share_selection.clone()?;
        let generation = self.media.begin(MediaPhase::PreparingShare);
        self.media_owner = Some(MediaOwner::Share);
        self.media_peer = None;
        self.media_path = None;
        self.media_decoder = None;
        self.media_width = None;
        self.media_height = None;
        self.media_error = None;
        Some((generation, selection))
    }

    fn publication_announced(&mut self, generation: Generation, path: String) -> bool {
        if self.media_owner != Some(MediaOwner::Share)
            || self.media.phase() != MediaPhase::PreparingShare
            || !self.media.apply(generation, MediaPhase::Sharing)
        {
            return false;
        }
        self.media_path = Some(path);
        true
    }

    fn publication_ended(
        &mut self,
        generation: Generation,
        result: Result<(), publication::Failure>,
    ) -> bool {
        if self.media_owner != Some(MediaOwner::Share)
            || self.media.generation() != generation
            || !matches!(
                self.media.phase(),
                MediaPhase::PreparingShare | MediaPhase::Sharing
            )
        {
            return false;
        }
        match result {
            Ok(()) => self.clear_media(MediaPhase::Idle, None),
            Err(error) => {
                let (message, source_unavailable) = error.into_parts();
                self.media.apply(generation, MediaPhase::Failed);
                self.media_path = None;
                self.media_error = Some(message);
                if source_unavailable {
                    self.share_selection = None;
                }
            }
        }
        true
    }

    fn begin_stop_share(&mut self) -> Option<Generation> {
        if self.media_owner != Some(MediaOwner::Share)
            || !matches!(
                self.media.phase(),
                MediaPhase::PreparingShare | MediaPhase::Sharing | MediaPhase::Failed
            )
        {
            return None;
        }
        Some(self.media.begin(MediaPhase::Stopping))
    }

    fn clear_media(&mut self, phase: MediaPhase, error: Option<String>) {
        self.media.begin(phase);
        self.clear_media_fields(error);
    }

    fn clear_media_fields(&mut self, error: Option<String>) {
        self.media_owner = None;
        self.media_peer = None;
        self.media_path = None;
        self.media_decoder = None;
        self.media_width = None;
        self.media_height = None;
        self.media_error = error;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RuntimeCommand {
    WatchScreen { peer: String, path: String },
    StopWatching,
    SelectShareSource(ShareSelection),
    StartSharing,
    StopSharing,
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
    frames: watch::Receiver<Option<Arc<PlaybackFrame>>>,
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
        let (frames_tx, frames) = watch::channel(None);
        let wake = Arc::new(wake);
        let owner = thread::Builder::new()
            .name("moqcast-macos-runtime".to_owned())
            .spawn(move || runtime.block_on(run(command_rx, snapshot_tx, frames_tx, start(), wake)))
            .map_err(RuntimeStartError::OwnerThread)?;

        Ok(Self {
            commands,
            snapshot,
            frames,
            owner: Some(owner),
        })
    }

    pub(crate) fn snapshot(&self) -> Arc<AppSnapshot> {
        self.snapshot.borrow().clone()
    }

    pub(crate) fn playback_frame(&self) -> Option<Arc<PlaybackFrame>> {
        self.frames.borrow().clone()
    }

    pub(crate) fn watch_screen(&self, peer: String, path: String) -> bool {
        self.commands
            .try_send(RuntimeCommand::WatchScreen { peer, path })
            .is_ok()
    }

    pub(crate) fn stop_watching(&self) -> bool {
        self.commands.try_send(RuntimeCommand::StopWatching).is_ok()
    }

    pub(crate) fn select_share_source(&self, selection: ShareSelection) -> bool {
        self.commands
            .try_send(RuntimeCommand::SelectShareSource(selection))
            .is_ok()
    }

    pub(crate) fn start_sharing(&self) -> bool {
        self.commands.try_send(RuntimeCommand::StartSharing).is_ok()
    }

    pub(crate) fn stop_sharing(&self) -> bool {
        self.commands.try_send(RuntimeCommand::StopSharing).is_ok()
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
    frames: watch::Sender<Option<Arc<PlaybackFrame>>>,
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
                loop {
                    if matches!(commands.recv().await, Some(RuntimeCommand::Shutdown) | None) {
                        break;
                    }
                }
                stop_snapshot(&mut snapshot, &snapshot_tx, &wake);
                return;
            }
        },
        command = commands.recv() => {
            if matches!(command, Some(RuntimeCommand::Shutdown) | None) {
                stop_snapshot(&mut snapshot, &snapshot_tx, &wake);
                return;
            }
            unreachable!("media commands are unavailable before services start");
        }
    };

    let (playback_events_tx, mut playback_events) = mpsc::channel(PLAYBACK_EVENT_CAPACITY);
    let mut playback = playback::Owner::default();
    let mut publication = publication::Owner::default();
    let mut initial_scan = Box::pin(tokio::time::sleep(Duration::from_secs(3)));
    let mut scan_finished = false;
    loop {
        let input = if scan_finished {
            tokio::select! {
                command = commands.recv() => RuntimeInput::Command(command),
                event = services.recv() => RuntimeInput::Network(event),
                event = playback_events.recv() => RuntimeInput::Playback(event),
                event = publication.recv() => RuntimeInput::Publication(event),
            }
        } else {
            tokio::select! {
                command = commands.recv() => RuntimeInput::Command(command),
                event = services.recv() => RuntimeInput::Network(event),
                event = playback_events.recv() => RuntimeInput::Playback(event),
                event = publication.recv() => RuntimeInput::Publication(event),
                () = &mut initial_scan => RuntimeInput::InitialScanFinished,
            }
        };

        let previous = snapshot.clone();
        let network_exhausted = match input {
            RuntimeInput::Command(Some(RuntimeCommand::Shutdown) | None) => break,
            RuntimeInput::Command(Some(RuntimeCommand::WatchScreen { peer, path })) => {
                start_watch(
                    &mut snapshot,
                    StartWatch {
                        services: &services,
                        playback: &mut playback,
                        events: &playback_events_tx,
                        frames: &frames,
                        wake: &wake,
                        peer_id: peer,
                        path,
                    },
                );
                false
            }
            RuntimeInput::Command(Some(RuntimeCommand::StopWatching)) => {
                stop_watch(&mut snapshot, &mut playback, &frames).await;
                false
            }
            RuntimeInput::Command(Some(RuntimeCommand::SelectShareSource(selection))) => {
                snapshot.select_share_source(selection);
                false
            }
            RuntimeInput::Command(Some(RuntimeCommand::StartSharing)) => {
                start_share(&mut snapshot, &services, &mut publication);
                false
            }
            RuntimeInput::Command(Some(RuntimeCommand::StopSharing)) => {
                stop_share(&mut snapshot, &mut publication);
                false
            }
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
            RuntimeInput::Playback(Some(event)) => {
                apply_playback_event(&mut snapshot, &frames, event);
                false
            }
            RuntimeInput::Playback(None) => false,
            RuntimeInput::Publication(event) => {
                apply_publication_event(&mut snapshot, event);
                false
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

    playback.stop().await;
    publication.stop();
    frames.send_replace(None);
    services.shutdown().await;
    stop_snapshot(&mut snapshot, &snapshot_tx, &wake);
}

enum RuntimeInput {
    Command(Option<RuntimeCommand>),
    Network(Option<NetworkEvent>),
    Playback(Option<PlaybackEvent>),
    Publication(PublicationEvent),
    InitialScanFinished,
}

struct StartWatch<'a> {
    services: &'a network::Services,
    playback: &'a mut playback::Owner,
    events: &'a mpsc::Sender<PlaybackEvent>,
    frames: &'a watch::Sender<Option<Arc<PlaybackFrame>>>,
    wake: &'a Arc<dyn Fn() + Send + Sync>,
    peer_id: String,
    path: String,
}

fn start_watch(snapshot: &mut AppSnapshot, start: StartWatch<'_>) {
    if !snapshot.can_watch(&start.peer_id, &start.path) {
        return;
    }
    let Some(broadcast) = start.services.remote_broadcast(&start.path) else {
        return;
    };
    let Some(generation) = snapshot.begin_watch(&start.peer_id, &start.path) else {
        return;
    };
    start.frames.send_replace(None);
    start.playback.start(
        generation.value(),
        broadcast,
        start.events.clone(),
        start.frames.clone(),
        start.wake.clone(),
    );
}

async fn stop_watch(
    snapshot: &mut AppSnapshot,
    playback: &mut playback::Owner,
    frames: &watch::Sender<Option<Arc<PlaybackFrame>>>,
) {
    let Some(generation) = snapshot.begin_stop_watch() else {
        return;
    };
    playback.stop().await;
    frames.send_replace(None);
    snapshot.finish_stop_media(generation);
}

fn start_share(
    snapshot: &mut AppSnapshot,
    services: &network::Services,
    publication: &mut publication::Owner,
) {
    let Some((generation, selection)) = snapshot.begin_share() else {
        return;
    };
    publication.start(
        generation.value(),
        services.publish_origin(),
        services.local_peer_id().to_owned(),
        selection,
    );
}

fn stop_share(snapshot: &mut AppSnapshot, publication: &mut publication::Owner) {
    let Some(generation) = snapshot.begin_stop_share() else {
        return;
    };
    publication.stop();
    snapshot.finish_stop_media(generation);
}

fn apply_publication_event(snapshot: &mut AppSnapshot, event: PublicationEvent) {
    match event {
        PublicationEvent::Announced { generation, path } => {
            let generation = Generation(generation);
            if snapshot.publication_announced(generation, path) {
                tracing::info!(
                    publish_generation = generation.value(),
                    "screen publication announced"
                );
            }
        }
        PublicationEvent::Ended { generation, result } => {
            let generation = Generation(generation);
            let failed = result.is_err();
            let error = result
                .as_ref()
                .err()
                .map(|error| error.message().to_owned());
            if snapshot.publication_ended(generation, result) {
                if let Some(error) = error {
                    tracing::warn!(
                        publish_generation = generation.value(),
                        %error,
                        "screen sharing ended"
                    );
                }
            } else if failed {
                tracing::debug!(
                    publish_generation = generation.value(),
                    "ignored stale publication failure"
                );
            }
        }
    }
}

fn apply_playback_event(
    snapshot: &mut AppSnapshot,
    frames: &watch::Sender<Option<Arc<PlaybackFrame>>>,
    event: PlaybackEvent,
) {
    match event {
        PlaybackEvent::Started {
            generation,
            decoder,
            width,
            height,
        } => {
            let generation = Generation(generation);
            if snapshot.playback_started(generation, decoder.clone(), width, height) {
                tracing::info!(
                    view_generation = generation.value(),
                    decoder,
                    width,
                    height,
                    "remote screen playback started"
                );
            }
        }
        PlaybackEvent::Ended { generation, result } => {
            let generation = Generation(generation);
            let failed = result.is_err();
            if snapshot.playback_ended(generation, result.clone()) {
                frames.send_replace(None);
                if let Err(error) = result {
                    tracing::warn!(
                        view_generation = generation.value(),
                        %error,
                        "remote screen playback ended"
                    );
                }
            } else if failed {
                tracing::debug!(
                    view_generation = generation.value(),
                    "ignored stale playback failure"
                );
            }
        }
    }
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
        NetworkEvent::Screen(update) => {
            snapshot.remote_screens.insert(update.path, update.view);
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
    snapshot.clear_media(MediaPhase::Idle, None);
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
                let (frames, _) = watch::channel(None);
                let owner = run(
                    command_rx,
                    snapshot_tx,
                    frames,
                    async { Err(NearbyIssue::LocalNetworkUnavailable) },
                    Arc::new(|| {}),
                );
                let observe = async move {
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
                    assert_eq!(snapshot.capture.phase(), CapabilityPhase::Available);
                    assert_eq!(snapshot.decoder.phase(), CapabilityPhase::Available);

                    commands
                        .send(RuntimeCommand::Shutdown)
                        .await
                        .expect("shutdown command");
                };
                tokio::join!(owner, observe);
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

    #[test]
    fn screen_availability_does_not_override_presence_or_session() {
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
        apply_network_event(
            &mut snapshot,
            discovery,
            session,
            NetworkEvent::Screen(crate::remote::Update {
                path: crate::contract::screen_path("internal-peer"),
                view: ScreenView {
                    peer_id: "internal-peer".to_owned(),
                    availability: ScreenAvailability::Available,
                },
            }),
        );

        let peer = snapshot.peers.get("internal-peer").expect("retained peer");
        assert!(!peer.discovered);
        assert_eq!(peer.session, PeerSession::Connected);
        assert_eq!(
            snapshot.remote_screens["moqcast.screen/internal-peer"].availability,
            ScreenAvailability::Available
        );
    }

    #[test]
    fn waiting_peer_with_available_screen_can_watch() {
        let mut snapshot = AppSnapshot::default();
        snapshot.peers.insert(
            "passive-peer".to_owned(),
            PeerSnapshot {
                ordinal: 1,
                discovered: true,
                session: PeerSession::Waiting,
            },
        );
        let path = crate::contract::screen_path("passive-peer");
        snapshot.remote_screens.insert(
            path.clone(),
            ScreenView {
                peer_id: "passive-peer".to_owned(),
                availability: ScreenAvailability::Available,
            },
        );

        assert!(snapshot.can_watch("passive-peer", &path));
        assert_eq!(snapshot.peers["passive-peer"].session, PeerSession::Waiting);
    }

    #[test]
    fn peer_without_available_screen_cannot_watch() {
        let mut snapshot = AppSnapshot::default();
        snapshot.peers.insert(
            "passive-peer".to_owned(),
            PeerSnapshot {
                ordinal: 1,
                discovered: true,
                session: PeerSession::Waiting,
            },
        );
        let path = crate::contract::screen_path("passive-peer");

        assert!(!snapshot.can_watch("passive-peer", &path));

        snapshot.remote_screens.insert(
            path.clone(),
            ScreenView {
                peer_id: "passive-peer".to_owned(),
                availability: ScreenAvailability::Unavailable,
            },
        );
        assert!(!snapshot.can_watch("passive-peer", &path));
    }

    #[test]
    fn playback_generation_keeps_stale_decoder_events_out() {
        let mut snapshot = AppSnapshot::default();
        let first = snapshot
            .begin_watch("peer", "moqcast.screen/peer")
            .expect("first view");
        let stopping = snapshot.begin_stop_watch().expect("stop first view");
        assert!(snapshot.finish_stop_media(stopping));
        let second = snapshot
            .begin_watch("peer", "moqcast.screen/peer")
            .expect("second view");

        assert!(!snapshot.playback_started(first, "stale".to_owned(), 640, 360));
        assert!(snapshot.playback_started(second, "videotoolbox".to_owned(), 640, 360));
        assert_eq!(snapshot.media.phase(), MediaPhase::Watching);
        assert_eq!(snapshot.media_decoder.as_deref(), Some("videotoolbox"));
    }

    #[test]
    fn failed_watch_requires_stop_before_retry() {
        let mut snapshot = AppSnapshot::default();
        let generation = snapshot
            .begin_watch("peer", "moqcast.screen/peer")
            .expect("first view");
        assert!(snapshot.playback_ended(generation, Err("decoder failed".to_owned())));

        assert!(
            snapshot
                .begin_watch("peer", "moqcast.screen/peer")
                .is_none()
        );
        let stopping = snapshot.begin_stop_watch().expect("stop failed view");
        assert!(snapshot.finish_stop_media(stopping));
        assert!(
            snapshot
                .begin_watch("peer", "moqcast.screen/peer")
                .is_some()
        );
    }

    #[test]
    fn stopping_watch_does_not_change_a_healthy_session() {
        let mut snapshot = AppSnapshot::default();
        snapshot.peers.insert(
            "peer".to_owned(),
            PeerSnapshot {
                ordinal: 1,
                discovered: true,
                session: PeerSession::Connected,
            },
        );
        snapshot
            .begin_watch("peer", "moqcast.screen/peer")
            .expect("view");
        let stopping = snapshot.begin_stop_watch().expect("stop");
        assert!(snapshot.finish_stop_media(stopping));

        assert_eq!(snapshot.media.phase(), MediaPhase::Idle);
        assert_eq!(snapshot.peers["peer"].session, PeerSession::Connected);
    }

    fn display_selection() -> ShareSelection {
        ShareSelection::Display {
            display_id: 7,
            label: "Display 7".to_owned(),
        }
    }

    #[test]
    fn watch_and_share_have_one_media_owner() {
        let mut snapshot = AppSnapshot::default();
        assert!(snapshot.select_share_source(display_selection()));
        let (share, _) = snapshot.begin_share().expect("share starts");
        assert_eq!(snapshot.media_owner, Some(MediaOwner::Share));
        assert!(
            snapshot
                .begin_watch("peer", "moqcast.screen/peer")
                .is_none()
        );
        assert!(!snapshot.playback_started(share, "stale".to_owned(), 640, 360));
    }

    #[test]
    fn stale_publication_events_cannot_override_a_new_media_generation() {
        let mut snapshot = AppSnapshot::default();
        assert!(snapshot.select_share_source(display_selection()));
        let (first, _) = snapshot.begin_share().expect("first share");
        let stopping = snapshot.begin_stop_share().expect("stop first share");
        assert!(snapshot.finish_stop_media(stopping));
        let (second, _) = snapshot.begin_share().expect("second share");

        assert!(!snapshot.publication_announced(first, "moqcast.screen/stale".to_owned()));
        assert!(
            !snapshot
                .publication_ended(first, Err(publication::Failure::pipeline("stale failure")),)
        );
        assert_eq!(snapshot.media.phase(), MediaPhase::PreparingShare);
        assert!(snapshot.media_error.is_none());
        assert!(snapshot.publication_announced(second, "moqcast.screen/current".to_owned()));
        assert_eq!(snapshot.media.phase(), MediaPhase::Sharing);
        assert_eq!(
            snapshot.media_path.as_deref(),
            Some("moqcast.screen/current")
        );
    }

    #[test]
    fn stopping_share_does_not_change_a_healthy_session() {
        let mut snapshot = AppSnapshot::default();
        let session_generation = snapshot.session.begin(SessionPhase::Listening);
        snapshot.peers.insert(
            "peer".to_owned(),
            PeerSnapshot {
                ordinal: 1,
                discovered: true,
                session: PeerSession::Connected,
            },
        );
        assert!(snapshot.select_share_source(display_selection()));
        snapshot.begin_share().expect("share");
        let stopping = snapshot.begin_stop_share().expect("stop share");
        assert!(snapshot.finish_stop_media(stopping));

        assert_eq!(snapshot.media.phase(), MediaPhase::Idle);
        assert_eq!(snapshot.media_owner, None);
        assert_eq!(snapshot.session.phase(), SessionPhase::Listening);
        assert_eq!(snapshot.session.generation(), session_generation);
        assert_eq!(snapshot.peers["peer"].session, PeerSession::Connected);
        assert!(snapshot.share_selection.is_some());
    }

    #[test]
    fn unavailable_share_source_requires_a_new_picker_selection() {
        let mut snapshot = AppSnapshot::default();
        assert!(snapshot.select_share_source(display_selection()));
        let (generation, _) = snapshot.begin_share().expect("share");

        assert!(
            snapshot
                .publication_ended(generation, Err(publication::Failure::source_unavailable()),)
        );
        assert_eq!(snapshot.media.phase(), MediaPhase::Failed);
        assert!(snapshot.share_selection.is_none());
    }

    #[test]
    fn active_media_rejects_source_replacement() {
        let mut snapshot = AppSnapshot::default();
        assert!(snapshot.select_share_source(display_selection()));
        snapshot.begin_share().expect("share");

        assert!(!snapshot.select_share_source(ShareSelection::Window {
            window_id: 9,
            label: "Window".to_owned(),
        }));
        assert_eq!(
            snapshot.share_selection.as_ref().map(ShareSelection::label),
            Some("Display 7")
        );
    }
}
