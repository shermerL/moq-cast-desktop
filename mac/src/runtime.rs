//! Generation-guarded application lifecycle owned outside the UI thread.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
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
    Suspended,
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ShareAudioPhase {
    #[default]
    Off,
    Preparing,
    Included,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NearbyIssue {
    LocalNetworkUnavailable,
    DirectConnectionsUnavailable,
    DiscoveryStopped,
    ListenerStopped,
    ServicesStopped,
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
    pub(crate) local_peer_id: Option<String>,
    pub(crate) peers: BTreeMap<String, PeerSnapshot>,
    pub(crate) remote_screens: BTreeMap<String, ScreenView>,
    pub(crate) inbound_sessions: usize,
    pub(crate) nearby_issue: Option<NearbyIssue>,
    pub(crate) media_peer: Option<String>,
    pub(crate) media_path: Option<String>,
    pub(crate) media_owner: Option<MediaOwner>,
    pub(crate) share_selection: Option<ShareSelection>,
    pub(crate) share_system_audio: bool,
    pub(crate) share_audio: ShareAudioPhase,
    pub(crate) share_audio_error: Option<String>,
    pub(crate) watch_audio: playback::AudioSnapshot,
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
            local_peer_id: None,
            peers: BTreeMap::new(),
            remote_screens: BTreeMap::new(),
            inbound_sessions: 0,
            nearby_issue: None,
            media_peer: None,
            media_path: None,
            media_owner: None,
            share_selection: None,
            share_system_audio: false,
            share_audio: ShareAudioPhase::Off,
            share_audio_error: None,
            watch_audio: playback::AudioSnapshot::default(),
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
        self.watch_audio = playback::AudioSnapshot::default();
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
                self.watch_audio = playback::AudioSnapshot::default();
            }
        }
        true
    }

    fn playback_audio_changed(
        &mut self,
        generation: Generation,
        audio: playback::AudioSnapshot,
    ) -> bool {
        if self.media_owner != Some(MediaOwner::Watch)
            || self.media.generation() != generation
            || !matches!(
                self.media.phase(),
                MediaPhase::PreparingWatch | MediaPhase::Watching
            )
        {
            return false;
        }
        self.watch_audio = audio;
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
        let reset_share_audio = self.media_owner == Some(MediaOwner::Share);
        if !self.media.apply(generation, MediaPhase::Idle) {
            return false;
        }
        self.clear_media_fields(None);
        if reset_share_audio {
            self.reset_share_audio();
        }
        true
    }

    fn select_share_source(&mut self, selection: ShareSelection) -> bool {
        if !matches!(self.media.phase(), MediaPhase::Idle | MediaPhase::Failed) {
            return false;
        }
        self.share_selection = Some(selection);
        self.share_system_audio = false;
        self.share_audio = ShareAudioPhase::Off;
        self.share_audio_error = None;
        true
    }

    fn set_share_system_audio(&mut self, enabled: bool) -> bool {
        if self.media.phase() != MediaPhase::Idle
            || (enabled
                && !self
                    .share_selection
                    .as_ref()
                    .is_some_and(ShareSelection::supports_system_audio))
        {
            return false;
        }
        self.share_system_audio = enabled;
        self.share_audio = ShareAudioPhase::Off;
        self.share_audio_error = None;
        true
    }

    fn begin_share(&mut self) -> Option<ShareStart> {
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
        self.share_audio = if self.share_system_audio {
            ShareAudioPhase::Preparing
        } else {
            ShareAudioPhase::Off
        };
        self.share_audio_error = None;
        Some(ShareStart {
            generation,
            selection,
            system_audio: self.share_system_audio,
        })
    }

    fn publication_announced(
        &mut self,
        generation: Generation,
        path: String,
        audio: publication::AudioStatus,
    ) -> bool {
        if self.media_owner != Some(MediaOwner::Share)
            || self.media.phase() != MediaPhase::PreparingShare
            || !self.media.apply(generation, MediaPhase::Sharing)
        {
            return false;
        }
        self.media_path = Some(path);
        match audio {
            publication::AudioStatus::Off => {
                self.share_audio = ShareAudioPhase::Off;
                self.share_audio_error = None;
            }
            publication::AudioStatus::Included => {
                self.share_audio = ShareAudioPhase::Included;
                self.share_audio_error = None;
            }
            publication::AudioStatus::Unavailable(message) => {
                self.share_audio = ShareAudioPhase::Failed;
                self.share_audio_error = Some(message);
            }
        }
        true
    }

    fn publication_audio_failed(&mut self, generation: Generation, message: String) -> bool {
        if self.media_owner != Some(MediaOwner::Share)
            || self.media.generation() != generation
            || !matches!(
                self.media.phase(),
                MediaPhase::PreparingShare | MediaPhase::Sharing
            )
        {
            return false;
        }
        self.share_audio = ShareAudioPhase::Failed;
        self.share_audio_error = Some(message);
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
                self.share_audio = ShareAudioPhase::Off;
                self.share_audio_error = None;
                if source_unavailable {
                    self.share_selection = None;
                    self.reset_share_audio();
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
        let reset_share_audio = self.media_owner == Some(MediaOwner::Share);
        self.media.begin(phase);
        self.clear_media_fields(error);
        if reset_share_audio {
            self.reset_share_audio();
        }
    }

    fn clear_media_fields(&mut self, error: Option<String>) {
        self.media_owner = None;
        self.media_peer = None;
        self.media_path = None;
        self.media_decoder = None;
        self.media_width = None;
        self.media_height = None;
        self.media_error = error;
        self.watch_audio = playback::AudioSnapshot::default();
    }

    fn reset_share_audio(&mut self) {
        self.share_system_audio = false;
        self.share_audio = ShareAudioPhase::Off;
        self.share_audio_error = None;
    }
}

struct ShareStart {
    generation: Generation,
    selection: ShareSelection,
    system_audio: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RuntimeCommand {
    WatchScreen { peer: String, path: String },
    StopWatching,
    SelectShareSource(ShareSelection),
    SetShareSystemAudio(bool),
    StartSharing,
    StopSharing,
    RestartNetwork,
    Shutdown,
}

type NetworkStartFuture =
    Pin<Box<dyn Future<Output = Result<network::Services, NearbyIssue>> + Send>>;
type NetworkStart = Arc<dyn Fn() -> NetworkStartFuture + Send + Sync>;

#[derive(Clone)]
pub(crate) struct SystemLifecycle {
    events: mpsc::UnboundedSender<SystemEvent>,
}

impl SystemLifecycle {
    pub(crate) fn suspend(&self) -> bool {
        self.events.send(SystemEvent::Suspend).is_ok()
    }

    pub(crate) fn resume(&self) -> bool {
        self.events.send(SystemEvent::Resume).is_ok()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SystemEvent {
    Suspend,
    Resume,
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
    system_events: mpsc::UnboundedSender<SystemEvent>,
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
        F: Fn() -> Fut + Send + Sync + 'static,
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
        let (system_events, system_event_rx) = mpsc::unbounded_channel();
        let (snapshot_tx, snapshot) = watch::channel(Arc::new(AppSnapshot::default()));
        let (frames_tx, frames) = watch::channel(None);
        let wake = Arc::new(wake);
        let start: NetworkStart = Arc::new(move || Box::pin(start()));
        let owner = thread::Builder::new()
            .name("moqcast-macos-runtime".to_owned())
            .spawn(move || {
                runtime.block_on(run(
                    command_rx,
                    system_event_rx,
                    snapshot_tx,
                    frames_tx,
                    start,
                    wake,
                ))
            })
            .map_err(RuntimeStartError::OwnerThread)?;

        Ok(Self {
            commands,
            system_events,
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

    pub(crate) fn set_share_system_audio(&self, enabled: bool) -> bool {
        self.commands
            .try_send(RuntimeCommand::SetShareSystemAudio(enabled))
            .is_ok()
    }

    pub(crate) fn start_sharing(&self) -> bool {
        self.commands.try_send(RuntimeCommand::StartSharing).is_ok()
    }

    pub(crate) fn stop_sharing(&self) -> bool {
        self.commands.try_send(RuntimeCommand::StopSharing).is_ok()
    }

    pub(crate) fn restart_network(&self) -> bool {
        self.commands
            .try_send(RuntimeCommand::RestartNetwork)
            .is_ok()
    }

    pub(crate) fn system_lifecycle(&self) -> SystemLifecycle {
        SystemLifecycle {
            events: self.system_events.clone(),
        }
    }

    pub(crate) fn shutdown(&mut self) {
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
    mut system_events: mpsc::UnboundedReceiver<SystemEvent>,
    snapshot_tx: watch::Sender<Arc<AppSnapshot>>,
    frames: watch::Sender<Option<Arc<PlaybackFrame>>>,
    start: NetworkStart,
    wake: Arc<dyn Fn() + Send + Sync>,
) {
    let mut snapshot = AppSnapshot::default();
    let runtime_generation = snapshot.runtime.begin(RuntimePhase::Starting);
    assert!(
        snapshot
            .runtime
            .apply(runtime_generation, RuntimePhase::Ready)
    );
    publish_snapshot(&snapshot_tx, &snapshot, &wake);
    tracing::info!(stage = "runtime", "macOS runtime owner ready");

    let (playback_events_tx, mut playback_events) = mpsc::channel(PLAYBACK_EVENT_CAPACITY);
    let mut playback = playback::Owner::default();
    let mut publication = publication::Owner::default();
    let mut suspended = false;

    'runtime: loop {
        if suspended {
            match wait_while_suspended(&mut commands, &mut system_events).await {
                SuspendedAction::Resume => {
                    suspended = false;
                    snapshot.runtime.begin(RuntimePhase::Ready);
                    tracing::info!("resuming Nearby network services after system wake");
                }
                SuspendedAction::Shutdown => {
                    stop_snapshot(&mut snapshot, &snapshot_tx, &wake);
                    return;
                }
            }
        }
        let generations = begin_network_start(&mut snapshot);
        tracing::info!(
            discovery_generation = generations.discovery.value(),
            session_generation = generations.session.value(),
            "starting Nearby network services"
        );
        publish_snapshot(&snapshot_tx, &snapshot, &wake);
        let mut start_attempt = start();
        let mut services = loop {
            tokio::select! {
                result = &mut start_attempt => match result {
                    Ok(services) => break services,
                    Err(issue) => {
                        tracing::warn!(?issue, "Nearby network services failed to start");
                        mark_network_failed(&mut snapshot, generations, issue);
                        publish_snapshot(&snapshot_tx, &snapshot, &wake);
                        match wait_for_network_restart(&mut commands, &mut system_events).await {
                            RecoveryAction::Restart => continue 'runtime,
                            RecoveryAction::Suspend => {
                                teardown_media(&mut snapshot, &mut playback, &mut publication, &frames).await;
                                suspend_snapshot(&mut snapshot, &snapshot_tx, &wake);
                                suspended = true;
                                continue 'runtime;
                            }
                            RecoveryAction::Shutdown => {
                                teardown_media(&mut snapshot, &mut playback, &mut publication, &frames).await;
                                stop_snapshot(&mut snapshot, &snapshot_tx, &wake);
                                return;
                            }
                            RecoveryAction::Failed => unreachable!(),
                        }
                    }
                },
                command = commands.recv() => match command {
                    Some(RuntimeCommand::Shutdown) | None => {
                        teardown_media(&mut snapshot, &mut playback, &mut publication, &frames).await;
                        stop_snapshot(&mut snapshot, &snapshot_tx, &wake);
                        return;
                    }
                    Some(RuntimeCommand::RestartNetwork) => continue 'runtime,
                    Some(_) => tracing::debug!(
                        "ignored media command while Nearby network services are starting"
                    ),
                },
                Some(event) = system_events.recv() => match event {
                    SystemEvent::Suspend => {
                        teardown_media(&mut snapshot, &mut playback, &mut publication, &frames).await;
                        drop(start_attempt);
                        suspend_snapshot(&mut snapshot, &snapshot_tx, &wake);
                        suspended = true;
                        continue 'runtime;
                    }
                    SystemEvent::Resume => {}
                },
            }
        };

        snapshot.local_peer_id = Some(services.local_peer_id().to_owned());
        snapshot
            .discovery
            .apply(generations.discovery, DiscoveryPhase::Scanning);
        snapshot
            .session
            .apply(generations.session, SessionPhase::Listening);
        snapshot.nearby_issue = None;
        publish_snapshot(&snapshot_tx, &snapshot, &wake);

        let action = ServiceRun {
            commands: &mut commands,
            system_events: &mut system_events,
            snapshot: &mut snapshot,
            services: &mut services,
            generations,
            playback: &mut playback,
            playback_events_tx: &playback_events_tx,
            playback_events: &mut playback_events,
            publication: &mut publication,
            frames: &frames,
            snapshot_tx: &snapshot_tx,
            wake: &wake,
        }
        .run()
        .await;
        teardown_media(&mut snapshot, &mut playback, &mut publication, &frames).await;
        services.shutdown().await;

        match action {
            RecoveryAction::Shutdown => {
                stop_snapshot(&mut snapshot, &snapshot_tx, &wake);
                return;
            }
            RecoveryAction::Restart => {
                tracing::info!("restarting Nearby network services");
                continue;
            }
            RecoveryAction::Suspend => {
                suspend_snapshot(&mut snapshot, &snapshot_tx, &wake);
                suspended = true;
                continue;
            }
            RecoveryAction::Failed => {
                tracing::warn!("Nearby network services ended; waiting for explicit restart");
                mark_network_failed(&mut snapshot, generations, NearbyIssue::ServicesStopped);
                publish_snapshot(&snapshot_tx, &snapshot, &wake);
                match wait_for_network_restart(&mut commands, &mut system_events).await {
                    RecoveryAction::Restart => continue,
                    RecoveryAction::Suspend => {
                        suspend_snapshot(&mut snapshot, &snapshot_tx, &wake);
                        suspended = true;
                        continue;
                    }
                    RecoveryAction::Shutdown => {
                        stop_snapshot(&mut snapshot, &snapshot_tx, &wake);
                        return;
                    }
                    RecoveryAction::Failed => unreachable!(),
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
struct NetworkGenerations {
    discovery: Generation,
    session: Generation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecoveryAction {
    Restart,
    Suspend,
    Shutdown,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SuspendedAction {
    Resume,
    Shutdown,
}

fn begin_network_start(snapshot: &mut AppSnapshot) -> NetworkGenerations {
    snapshot.local_peer_id = None;
    snapshot.peers.clear();
    snapshot.remote_screens.clear();
    snapshot.inbound_sessions = 0;
    snapshot.nearby_issue = None;
    NetworkGenerations {
        discovery: snapshot.discovery.begin(DiscoveryPhase::Starting),
        session: snapshot.session.begin(SessionPhase::Starting),
    }
}

fn mark_network_failed(
    snapshot: &mut AppSnapshot,
    generations: NetworkGenerations,
    issue: NearbyIssue,
) -> bool {
    if snapshot.discovery.generation() != generations.discovery
        || snapshot.session.generation() != generations.session
    {
        return false;
    }
    snapshot.peers.clear();
    snapshot.remote_screens.clear();
    snapshot.inbound_sessions = 0;
    snapshot.local_peer_id = None;
    snapshot
        .discovery
        .apply(generations.discovery, DiscoveryPhase::Failed);
    snapshot
        .session
        .apply(generations.session, SessionPhase::Failed);
    snapshot.nearby_issue = Some(issue);
    true
}

async fn wait_for_network_restart(
    commands: &mut mpsc::Receiver<RuntimeCommand>,
    system_events: &mut mpsc::UnboundedReceiver<SystemEvent>,
) -> RecoveryAction {
    loop {
        tokio::select! {
            command = commands.recv() => match command {
                Some(RuntimeCommand::RestartNetwork) => return RecoveryAction::Restart,
                Some(RuntimeCommand::Shutdown) | None => return RecoveryAction::Shutdown,
                Some(_) => {}
            },
            Some(event) = system_events.recv() => match event {
                SystemEvent::Suspend => return RecoveryAction::Suspend,
                SystemEvent::Resume => {}
            },
        }
    }
}

async fn wait_while_suspended(
    commands: &mut mpsc::Receiver<RuntimeCommand>,
    system_events: &mut mpsc::UnboundedReceiver<SystemEvent>,
) -> SuspendedAction {
    loop {
        tokio::select! {
            command = commands.recv() => match command {
                Some(RuntimeCommand::Shutdown) | None => return SuspendedAction::Shutdown,
                Some(RuntimeCommand::RestartNetwork) => {}
                Some(_) => tracing::debug!("ignored media command while the runtime is suspended"),
            },
            Some(event) = system_events.recv() => match event {
                SystemEvent::Resume => return SuspendedAction::Resume,
                SystemEvent::Suspend => {}
            },
        }
    }
}

struct ServiceRun<'a> {
    commands: &'a mut mpsc::Receiver<RuntimeCommand>,
    system_events: &'a mut mpsc::UnboundedReceiver<SystemEvent>,
    snapshot: &'a mut AppSnapshot,
    services: &'a mut network::Services,
    generations: NetworkGenerations,
    playback: &'a mut playback::Owner,
    playback_events_tx: &'a mpsc::Sender<PlaybackEvent>,
    playback_events: &'a mut mpsc::Receiver<PlaybackEvent>,
    publication: &'a mut publication::Owner,
    frames: &'a watch::Sender<Option<Arc<PlaybackFrame>>>,
    snapshot_tx: &'a watch::Sender<Arc<AppSnapshot>>,
    wake: &'a Arc<dyn Fn() + Send + Sync>,
}

impl ServiceRun<'_> {
    async fn run(&mut self) -> RecoveryAction {
        let mut initial_scan = Box::pin(tokio::time::sleep(Duration::from_secs(3)));
        let mut scan_finished = false;
        loop {
            let input = if scan_finished {
                tokio::select! {
                    command = self.commands.recv() => RuntimeInput::Command(command),
                    Some(event) = self.system_events.recv() => RuntimeInput::System(event),
                    event = self.services.recv() => RuntimeInput::Network(event),
                    event = self.playback_events.recv() => RuntimeInput::Playback(event),
                    event = self.publication.recv() => RuntimeInput::Publication(event),
                }
            } else {
                tokio::select! {
                    command = self.commands.recv() => RuntimeInput::Command(command),
                    Some(event) = self.system_events.recv() => RuntimeInput::System(event),
                    event = self.services.recv() => RuntimeInput::Network(event),
                    event = self.playback_events.recv() => RuntimeInput::Playback(event),
                    event = self.publication.recv() => RuntimeInput::Publication(event),
                    () = &mut initial_scan => RuntimeInput::InitialScanFinished,
                }
            };

            let previous = self.snapshot.clone();
            let action = match input {
                RuntimeInput::Command(Some(RuntimeCommand::Shutdown) | None) => {
                    Some(RecoveryAction::Shutdown)
                }
                RuntimeInput::Command(Some(RuntimeCommand::RestartNetwork)) => {
                    Some(RecoveryAction::Restart)
                }
                RuntimeInput::Command(Some(RuntimeCommand::WatchScreen { peer, path })) => {
                    start_watch(
                        self.snapshot,
                        StartWatch {
                            services: self.services,
                            playback: self.playback,
                            events: self.playback_events_tx,
                            frames: self.frames,
                            wake: self.wake,
                            peer_id: peer,
                            path,
                        },
                    );
                    None
                }
                RuntimeInput::Command(Some(RuntimeCommand::StopWatching)) => {
                    stop_watch(self.snapshot, self.playback, self.frames).await;
                    None
                }
                RuntimeInput::Command(Some(RuntimeCommand::SelectShareSource(selection))) => {
                    self.snapshot.select_share_source(selection);
                    None
                }
                RuntimeInput::Command(Some(RuntimeCommand::SetShareSystemAudio(enabled))) => {
                    self.snapshot.set_share_system_audio(enabled);
                    None
                }
                RuntimeInput::Command(Some(RuntimeCommand::StartSharing)) => {
                    start_share(self.snapshot, self.services, self.publication);
                    None
                }
                RuntimeInput::Command(Some(RuntimeCommand::StopSharing)) => {
                    stop_share(self.snapshot, self.publication);
                    None
                }
                RuntimeInput::System(SystemEvent::Suspend) => Some(RecoveryAction::Suspend),
                RuntimeInput::System(SystemEvent::Resume) => None,
                RuntimeInput::Network(Some(event)) => {
                    apply_network_event(
                        self.snapshot,
                        self.generations.discovery,
                        self.generations.session,
                        event,
                    );
                    refresh_discovery_result(
                        self.snapshot,
                        self.generations.discovery,
                        scan_finished,
                    );
                    None
                }
                RuntimeInput::Network(None) => Some(RecoveryAction::Failed),
                RuntimeInput::Playback(Some(event)) => {
                    apply_playback_event(self.snapshot, self.frames, event);
                    None
                }
                RuntimeInput::Playback(None) => None,
                RuntimeInput::Publication(event) => {
                    apply_publication_event(self.snapshot, event);
                    None
                }
                RuntimeInput::InitialScanFinished => {
                    scan_finished = true;
                    let phase = if self.snapshot.peers.values().any(|peer| peer.discovered) {
                        DiscoveryPhase::Ready
                    } else {
                        DiscoveryPhase::Empty
                    };
                    self.snapshot
                        .discovery
                        .apply(self.generations.discovery, phase);
                    None
                }
            };
            if *self.snapshot != previous {
                publish_snapshot(self.snapshot_tx, self.snapshot, self.wake);
            }
            if let Some(action) = action {
                return action;
            }
        }
    }
}

async fn teardown_media(
    snapshot: &mut AppSnapshot,
    playback: &mut playback::Owner,
    publication: &mut publication::Owner,
    frames: &watch::Sender<Option<Arc<PlaybackFrame>>>,
) {
    snapshot.clear_media(MediaPhase::Idle, None);
    publication.stop();
    playback.stop().await;
    frames.send_replace(None);
}

enum RuntimeInput {
    Command(Option<RuntimeCommand>),
    System(SystemEvent),
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
        start.path,
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
    let Some(start) = snapshot.begin_share() else {
        return;
    };
    publication.start(
        start.generation.value(),
        services.publish_origin(),
        services.local_peer_id().to_owned(),
        start.selection,
        start.system_audio,
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
        PublicationEvent::Announced {
            generation,
            path,
            audio,
        } => {
            let generation = Generation(generation);
            if snapshot.publication_announced(generation, path, audio) {
                tracing::info!(
                    publish_generation = generation.value(),
                    "screen publication announced"
                );
            }
        }
        PublicationEvent::AudioFailed {
            generation,
            message,
        } => {
            let generation = Generation(generation);
            if snapshot.publication_audio_failed(generation, message.clone()) {
                tracing::warn!(
                    publish_generation = generation.value(),
                    %message,
                    "system audio unavailable; video publication continues"
                );
            } else {
                tracing::debug!(
                    publish_generation = generation.value(),
                    "ignored stale system audio failure"
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
        PlaybackEvent::Audio {
            generation,
            snapshot: audio,
        } => {
            let generation = Generation(generation);
            if snapshot.playback_audio_changed(generation, audio.clone()) {
                match audio.phase {
                    playback::AudioPhase::NoAudio => tracing::debug!(
                        view_generation = generation.value(),
                        "remote screen has no audio; video playback continues"
                    ),
                    playback::AudioPhase::Failed => tracing::warn!(
                        view_generation = generation.value(),
                        error = audio
                            .last_error
                            .as_deref()
                            .unwrap_or("remote audio unavailable"),
                        "remote audio unavailable; video playback continues"
                    ),
                    _ => {}
                }
            } else {
                tracing::debug!(
                    view_generation = generation.value(),
                    "ignored stale remote audio state"
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
            if has_available_screen(snapshot, &id) {
                if let Some(peer) = snapshot.peers.get_mut(&id) {
                    peer.discovered = false;
                }
            } else {
                snapshot.peers.remove(&id);
            }
        }
        NetworkEvent::InboundCount(count) => snapshot.inbound_sessions = count,
        NetworkEvent::InboundRejected => {
            snapshot.nearby_issue = Some(NearbyIssue::DeviceRejected);
        }
        NetworkEvent::Screen(update) => {
            let peer_id = update.view.peer_id.clone();
            let available = update.view.availability == ScreenAvailability::Available;
            snapshot.remote_screens.insert(update.path, update.view);
            if !available
                && snapshot.peers.get(&peer_id).is_some_and(|peer| {
                    !peer.discovered && !peer.session.is_active() && snapshot.inbound_sessions == 0
                })
            {
                snapshot.peers.remove(&peer_id);
            }
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

fn has_available_screen(snapshot: &AppSnapshot, peer_id: &str) -> bool {
    snapshot.remote_screens.values().any(|screen| {
        screen.peer_id == peer_id && screen.availability == ScreenAvailability::Available
    })
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
    snapshot.local_peer_id = None;
    snapshot.runtime.begin(RuntimePhase::Stopped);
    snapshot.discovery.begin(DiscoveryPhase::Stopped);
    snapshot.session.begin(SessionPhase::Stopped);
    snapshot.clear_media(MediaPhase::Idle, None);
    publish_snapshot(snapshot_tx, snapshot, wake);
    tracing::info!(stage = "shutdown", "macOS runtime owner stopped");
}

fn suspend_snapshot(
    snapshot: &mut AppSnapshot,
    snapshot_tx: &watch::Sender<Arc<AppSnapshot>>,
    wake: &Arc<dyn Fn() + Send + Sync>,
) {
    snapshot.local_peer_id = None;
    snapshot.runtime.begin(RuntimePhase::Suspended);
    snapshot.discovery.begin(DiscoveryPhase::Stopped);
    snapshot.session.begin(SessionPhase::Stopped);
    snapshot.peers.clear();
    snapshot.remote_screens.clear();
    snapshot.inbound_sessions = 0;
    snapshot.nearby_issue = None;
    publish_snapshot(snapshot_tx, snapshot, wake);
    tracing::info!(stage = "suspend", "macOS runtime owner suspended");
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
                let (_system_events, system_event_rx) = mpsc::unbounded_channel();
                let (snapshot_tx, mut snapshot_rx) =
                    watch::channel(Arc::new(AppSnapshot::default()));
                let (frames, _) = watch::channel(None);
                let start: NetworkStart =
                    Arc::new(|| Box::pin(async { Err(NearbyIssue::LocalNetworkUnavailable) }));
                let owner = run(
                    command_rx,
                    system_event_rx,
                    snapshot_tx,
                    frames,
                    start,
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
    fn failed_network_owner_restarts_only_after_an_explicit_command() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("test runtime")
            .block_on(async {
                let attempts = Arc::new(AtomicUsize::new(0));
                let start: NetworkStart = {
                    let attempts = attempts.clone();
                    Arc::new(move || {
                        attempts.fetch_add(1, Ordering::SeqCst);
                        Box::pin(async { Err(NearbyIssue::LocalNetworkUnavailable) })
                    })
                };
                let (commands, command_rx) = mpsc::channel(COMMAND_CAPACITY);
                let (_system_events, system_event_rx) = mpsc::unbounded_channel();
                let (snapshot_tx, mut snapshot_rx) =
                    watch::channel(Arc::new(AppSnapshot::default()));
                let (frames, _) = watch::channel(None);
                let owner = run(
                    command_rx,
                    system_event_rx,
                    snapshot_tx,
                    frames,
                    start,
                    Arc::new(|| {}),
                );
                let observe = async move {
                    loop {
                        snapshot_rx.changed().await.expect("first failure");
                        if snapshot_rx.borrow().discovery.phase() == DiscoveryPhase::Failed {
                            break;
                        }
                    }
                    let first_generation = snapshot_rx.borrow().discovery.generation();
                    assert_eq!(attempts.load(Ordering::SeqCst), 1);

                    commands
                        .send(RuntimeCommand::RestartNetwork)
                        .await
                        .expect("restart command");
                    loop {
                        snapshot_rx.changed().await.expect("second failure");
                        let snapshot = snapshot_rx.borrow();
                        if snapshot.discovery.phase() == DiscoveryPhase::Failed
                            && snapshot.discovery.generation() != first_generation
                        {
                            break;
                        }
                    }
                    assert_eq!(attempts.load(Ordering::SeqCst), 2);
                    commands
                        .send(RuntimeCommand::Shutdown)
                        .await
                        .expect("shutdown command");
                };
                tokio::join!(owner, observe);
            });
    }

    #[test]
    fn shutdown_waits_for_the_current_start_owner_to_drop() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::mpsc as std_mpsc;

        struct DropSignal(Arc<AtomicBool>);

        impl Drop for DropSignal {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }

        let (entered_tx, entered_rx) = std_mpsc::channel();
        let dropped = Arc::new(AtomicBool::new(false));
        let mut runtime = RuntimeOwner::start_with(
            {
                let dropped = dropped.clone();
                move || {
                    let entered_tx = entered_tx.clone();
                    let dropped = dropped.clone();
                    async move {
                        let _drop = DropSignal(dropped);
                        entered_tx.send(()).expect("start entered");
                        std::future::pending::<Result<network::Services, NearbyIssue>>().await
                    }
                }
            },
            || {},
        )
        .expect("runtime starts");
        entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("start future was polled");

        runtime.shutdown();

        assert!(dropped.load(Ordering::SeqCst));
        assert_eq!(runtime.snapshot().runtime.phase(), RuntimePhase::Stopped);
    }

    #[test]
    fn restart_drops_a_pending_start_and_advances_network_generation() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use std::sync::mpsc as std_mpsc;

        struct DropSignal(Arc<AtomicBool>);

        impl Drop for DropSignal {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }

        let attempts = Arc::new(AtomicUsize::new(0));
        let first_dropped = Arc::new(AtomicBool::new(false));
        let (first_entered_tx, first_entered_rx) = std_mpsc::channel();
        let (second_started_tx, second_started_rx) = std_mpsc::channel();
        let (wake_tx, wake_rx) = std_mpsc::channel();
        let mut runtime = RuntimeOwner::start_with(
            {
                let attempts = attempts.clone();
                let first_dropped = first_dropped.clone();
                move || {
                    let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                    let first_dropped = first_dropped.clone();
                    let first_entered_tx = first_entered_tx.clone();
                    let second_started_tx = second_started_tx.clone();
                    async move {
                        if attempt == 0 {
                            let _drop = DropSignal(first_dropped);
                            first_entered_tx.send(()).expect("first start entered");
                            std::future::pending::<()>().await;
                            unreachable!();
                        }
                        second_started_tx.send(()).expect("second start invoked");
                        Err(NearbyIssue::LocalNetworkUnavailable)
                    }
                }
            },
            move || {
                let _ = wake_tx.send(());
            },
        )
        .expect("runtime starts");
        first_entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("first start future was polled");
        let first_generation = runtime.snapshot().discovery.generation();

        assert!(runtime.restart_network());
        second_started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("restart invoked the factory again");
        while runtime.snapshot().discovery.phase() != DiscoveryPhase::Failed {
            wake_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("restart snapshot");
        }

        assert!(first_dropped.load(Ordering::SeqCst));
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        assert_ne!(runtime.snapshot().discovery.generation(), first_generation);
        runtime.shutdown();
    }

    #[test]
    fn suspend_resume_is_idempotent_while_network_start_is_pending() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use std::sync::mpsc as std_mpsc;

        struct DropSignal(Arc<AtomicBool>);

        impl Drop for DropSignal {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }

        let attempts = Arc::new(AtomicUsize::new(0));
        let first_dropped = Arc::new(AtomicBool::new(false));
        let (first_entered_tx, first_entered_rx) = std_mpsc::channel();
        let (second_started_tx, second_started_rx) = std_mpsc::channel();
        let (wake_tx, wake_rx) = std_mpsc::channel();
        let mut runtime = RuntimeOwner::start_with(
            {
                let attempts = attempts.clone();
                let first_dropped = first_dropped.clone();
                move || {
                    let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                    let first_dropped = first_dropped.clone();
                    let first_entered_tx = first_entered_tx.clone();
                    let second_started_tx = second_started_tx.clone();
                    async move {
                        if attempt == 0 {
                            let _drop = DropSignal(first_dropped);
                            first_entered_tx.send(()).expect("first start entered");
                            std::future::pending::<()>().await;
                            unreachable!();
                        }
                        second_started_tx.send(()).expect("second start invoked");
                        Err(NearbyIssue::LocalNetworkUnavailable)
                    }
                }
            },
            move || {
                let _ = wake_tx.send(());
            },
        )
        .expect("runtime starts");
        let lifecycle = runtime.system_lifecycle();
        first_entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("first start future was polled");
        let first_generation = runtime.snapshot().discovery.generation();

        assert!(lifecycle.suspend());
        while runtime.snapshot().runtime.phase() != RuntimePhase::Suspended {
            wake_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("suspended snapshot");
        }
        assert!(first_dropped.load(Ordering::SeqCst));
        assert_eq!(attempts.load(Ordering::SeqCst), 1);

        assert!(lifecycle.suspend());
        assert!(lifecycle.resume());
        second_started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("resume invoked the factory once");
        while runtime.snapshot().discovery.phase() != DiscoveryPhase::Failed {
            wake_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("resumed snapshot");
        }

        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        assert_ne!(runtime.snapshot().discovery.generation(), first_generation);
        runtime.shutdown();
    }

    #[test]
    fn system_lifecycle_does_not_wait_for_the_bounded_command_queue() {
        let (commands, _command_rx) = mpsc::channel(1);
        commands
            .try_send(RuntimeCommand::RestartNetwork)
            .expect("command queue filled");
        let (events, mut event_rx) = mpsc::unbounded_channel();
        let lifecycle = SystemLifecycle { events };

        assert!(lifecycle.suspend());
        assert!(lifecycle.resume());
        assert_eq!(event_rx.try_recv(), Ok(SystemEvent::Suspend));
        assert_eq!(event_rx.try_recv(), Ok(SystemEvent::Resume));

        drop(event_rx);
        assert!(!lifecycle.suspend());
    }

    #[test]
    fn stale_network_generation_cannot_mutate_snapshot() {
        let mut snapshot = AppSnapshot {
            local_peer_id: Some("previous-run".to_owned()),
            ..AppSnapshot::default()
        };
        let stale = begin_network_start(&mut snapshot);
        assert!(snapshot.local_peer_id.is_none());
        apply_network_event(
            &mut snapshot,
            stale.discovery,
            stale.session,
            NetworkEvent::InboundCount(4),
        );
        assert_eq!(snapshot.inbound_sessions, 4);

        let current = begin_network_start(&mut snapshot);
        snapshot.inbound_sessions = 2;
        assert!(!mark_network_failed(
            &mut snapshot,
            stale,
            NearbyIssue::ServicesStopped,
        ));

        apply_network_event(
            &mut snapshot,
            stale.discovery,
            stale.session,
            NetworkEvent::InboundCount(9),
        );

        assert_eq!(snapshot.inbound_sessions, 2);
        assert_eq!(snapshot.discovery.phase(), DiscoveryPhase::Starting);
        assert_eq!(snapshot.session.phase(), SessionPhase::Starting);
        assert_eq!(snapshot.discovery.generation(), current.discovery);
        assert_eq!(snapshot.session.generation(), current.session);
    }

    #[test]
    fn failed_network_start_clears_the_current_run_peer_id() {
        let mut snapshot = AppSnapshot::default();
        let generation = begin_network_start(&mut snapshot);
        snapshot.local_peer_id = Some("failed-run".to_owned());

        assert!(mark_network_failed(
            &mut snapshot,
            generation,
            NearbyIssue::ServicesStopped,
        ));
        assert!(snapshot.local_peer_id.is_none());
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
    fn peer_removal_keeps_an_available_screen_until_withdrawal() {
        let mut snapshot = AppSnapshot::default();
        let discovery = snapshot.discovery.begin(DiscoveryPhase::Scanning);
        let session = snapshot.session.begin(SessionPhase::Listening);
        let path = crate::contract::screen_path("passive-peer");
        apply_network_event(
            &mut snapshot,
            discovery,
            session,
            NetworkEvent::Peer(network::PeerStatus {
                id: "passive-peer".to_owned(),
                ordinal: 1,
                discovered: true,
                role: DialRole::Passive,
                session: PeerSession::Waiting,
                transport_generation: None,
            }),
        );
        apply_network_event(
            &mut snapshot,
            discovery,
            session,
            NetworkEvent::Screen(crate::remote::Update {
                path: path.clone(),
                view: ScreenView {
                    peer_id: "passive-peer".to_owned(),
                    availability: ScreenAvailability::Available,
                },
            }),
        );

        apply_network_event(
            &mut snapshot,
            discovery,
            session,
            NetworkEvent::PeerRemoved("passive-peer".to_owned()),
        );
        assert!(snapshot.can_watch("passive-peer", &path));
        assert!(!snapshot.peers["passive-peer"].discovered);

        apply_network_event(
            &mut snapshot,
            discovery,
            session,
            NetworkEvent::Screen(crate::remote::Update {
                path,
                view: ScreenView {
                    peer_id: "passive-peer".to_owned(),
                    availability: ScreenAvailability::Withdrawn,
                },
            }),
        );
        assert!(!snapshot.peers.contains_key("passive-peer"));
    }

    #[test]
    fn route_withdrawal_keeps_a_passive_peer_with_an_inbound_session() {
        let mut snapshot = AppSnapshot::default();
        let discovery = snapshot.discovery.begin(DiscoveryPhase::Scanning);
        let session = snapshot.session.begin(SessionPhase::Listening);
        snapshot.inbound_sessions = 1;
        snapshot.peers.insert(
            "passive-peer".to_owned(),
            PeerSnapshot {
                ordinal: 1,
                discovered: false,
                session: PeerSession::Waiting,
            },
        );

        apply_network_event(
            &mut snapshot,
            discovery,
            session,
            NetworkEvent::Screen(crate::remote::Update {
                path: crate::contract::screen_path("passive-peer"),
                view: ScreenView {
                    peer_id: "passive-peer".to_owned(),
                    availability: ScreenAvailability::Withdrawn,
                },
            }),
        );

        assert!(snapshot.peers.contains_key("passive-peer"));

        apply_network_event(
            &mut snapshot,
            discovery,
            session,
            NetworkEvent::InboundCount(0),
        );
        apply_network_event(
            &mut snapshot,
            discovery,
            session,
            NetworkEvent::PeerRemoved("passive-peer".to_owned()),
        );
        assert!(!snapshot.peers.contains_key("passive-peer"));
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
    fn remote_audio_failure_does_not_end_healthy_video_playback() {
        let mut snapshot = AppSnapshot::default();
        let generation = snapshot
            .begin_watch("peer", "moqcast.screen/peer")
            .expect("view");
        assert!(snapshot.playback_started(generation, "videotoolbox".to_owned(), 640, 360,));

        assert!(snapshot.playback_audio_changed(
            generation,
            playback::AudioSnapshot {
                phase: playback::AudioPhase::Failed,
                last_error: Some("default output unavailable".to_owned()),
                ..playback::AudioSnapshot::default()
            },
        ));

        assert_eq!(snapshot.media.phase(), MediaPhase::Watching);
        assert_eq!(snapshot.media_decoder.as_deref(), Some("videotoolbox"));
        assert_eq!(snapshot.watch_audio.phase, playback::AudioPhase::Failed);
    }

    #[test]
    fn stale_remote_audio_cannot_override_a_new_watch() {
        let mut snapshot = AppSnapshot::default();
        let first = snapshot
            .begin_watch("peer", "moqcast.screen/peer")
            .expect("first view");
        let stopping = snapshot.begin_stop_watch().expect("stop first view");
        assert!(snapshot.finish_stop_media(stopping));
        let second = snapshot
            .begin_watch("peer", "moqcast.screen/peer")
            .expect("second view");

        assert!(!snapshot.playback_audio_changed(
            first,
            playback::AudioSnapshot {
                phase: playback::AudioPhase::NoAudio,
                ..playback::AudioSnapshot::default()
            },
        ));
        assert!(snapshot.playback_audio_changed(
            second,
            playback::AudioSnapshot {
                phase: playback::AudioPhase::Pending,
                ..playback::AudioSnapshot::default()
            },
        ));
        assert_eq!(snapshot.watch_audio.phase, playback::AudioPhase::Pending);
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
        assert!(snapshot.select_share_source(display_selection()));
        assert!(snapshot.set_share_system_audio(true));
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
        snapshot.watch_audio = playback::AudioSnapshot {
            phase: playback::AudioPhase::PcmSubmitted,
            track: Some("audio".to_owned()),
            codec: Some("opus".to_owned()),
            sample_rate: Some(48_000),
            channels: Some(2),
            last_error: None,
        };
        let stopping = snapshot.begin_stop_watch().expect("stop");
        assert!(snapshot.finish_stop_media(stopping));

        assert_eq!(snapshot.media.phase(), MediaPhase::Idle);
        assert_eq!(snapshot.peers["peer"].session, PeerSession::Connected);
        assert!(snapshot.share_system_audio);
        assert_eq!(snapshot.share_audio, ShareAudioPhase::Off);
        assert_eq!(snapshot.watch_audio, playback::AudioSnapshot::default());
    }

    fn display_selection() -> ShareSelection {
        ShareSelection::Display {
            display_id: 7,
            primary: true,
            label: "Display 7".to_owned(),
        }
    }

    fn secondary_display_selection() -> ShareSelection {
        ShareSelection::Display {
            display_id: 8,
            primary: false,
            label: "Display 8".to_owned(),
        }
    }

    #[test]
    fn system_audio_is_opt_in_and_requires_the_primary_display() {
        let mut snapshot = AppSnapshot::default();
        assert!(snapshot.select_share_source(display_selection()));
        assert!(!snapshot.share_system_audio);
        assert!(snapshot.set_share_system_audio(true));
        assert!(snapshot.share_system_audio);

        assert!(snapshot.select_share_source(secondary_display_selection()));
        assert!(!snapshot.share_system_audio);
        assert!(!snapshot.set_share_system_audio(true));

        assert!(snapshot.select_share_source(ShareSelection::Window {
            window_id: 9,
            label: "Window".to_owned(),
        }));
        assert!(!snapshot.set_share_system_audio(true));
    }

    #[test]
    fn current_audio_failure_keeps_video_sharing_and_stale_failure_is_ignored() {
        let mut snapshot = AppSnapshot::default();
        assert!(snapshot.select_share_source(display_selection()));
        assert!(snapshot.set_share_system_audio(true));
        let start = snapshot.begin_share().expect("share starts");
        assert!(snapshot.publication_announced(
            start.generation,
            "moqcast.screen/current".to_owned(),
            publication::AudioStatus::Included,
        ));

        assert!(!snapshot.publication_audio_failed(
            Generation(start.generation.value() - 1),
            "stale".to_owned(),
        ));
        assert!(snapshot.publication_audio_failed(
            start.generation,
            "System audio is unavailable. Video sharing continues.".to_owned(),
        ));
        assert_eq!(snapshot.media.phase(), MediaPhase::Sharing);
        assert_eq!(
            snapshot.media_path.as_deref(),
            Some("moqcast.screen/current")
        );
        assert_eq!(snapshot.share_audio, ShareAudioPhase::Failed);
    }

    #[test]
    fn unavailable_audio_keeps_video_sharing_and_reports_the_reason() {
        let mut snapshot = AppSnapshot::default();
        assert!(snapshot.select_share_source(display_selection()));
        assert!(snapshot.set_share_system_audio(true));
        let start = snapshot.begin_share().expect("share starts");
        assert!(snapshot.publication_announced(
            start.generation,
            "moqcast.screen/current".to_owned(),
            publication::AudioStatus::Unavailable("main display required".to_owned()),
        ));

        assert_eq!(snapshot.media.phase(), MediaPhase::Sharing);
        assert_eq!(snapshot.share_audio, ShareAudioPhase::Failed);
        assert_eq!(
            snapshot.share_audio_error.as_deref(),
            Some("main display required")
        );
    }

    #[test]
    fn publication_failure_ends_the_audio_running_state() {
        let mut snapshot = AppSnapshot::default();
        assert!(snapshot.select_share_source(display_selection()));
        assert!(snapshot.set_share_system_audio(true));
        let start = snapshot.begin_share().expect("share starts");
        assert!(snapshot.publication_announced(
            start.generation,
            "moqcast.screen/current".to_owned(),
            publication::AudioStatus::Included,
        ));
        assert!(snapshot.publication_ended(
            start.generation,
            Err(publication::Failure::pipeline("video failed")),
        ));

        assert_eq!(snapshot.media.phase(), MediaPhase::Failed);
        assert!(snapshot.share_system_audio);
        assert_eq!(snapshot.share_audio, ShareAudioPhase::Off);
        assert!(snapshot.share_audio_error.is_none());
    }

    #[test]
    fn watch_and_share_have_one_media_owner() {
        let mut snapshot = AppSnapshot::default();
        assert!(snapshot.select_share_source(display_selection()));
        let share = snapshot.begin_share().expect("share starts").generation;
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
        let first = snapshot.begin_share().expect("first share").generation;
        let stopping = snapshot.begin_stop_share().expect("stop first share");
        assert!(snapshot.finish_stop_media(stopping));
        let second = snapshot.begin_share().expect("second share").generation;

        assert!(!snapshot.publication_announced(
            first,
            "moqcast.screen/stale".to_owned(),
            publication::AudioStatus::Off,
        ));
        assert!(
            !snapshot
                .publication_ended(first, Err(publication::Failure::pipeline("stale failure")),)
        );
        assert_eq!(snapshot.media.phase(), MediaPhase::PreparingShare);
        assert!(snapshot.media_error.is_none());
        assert!(snapshot.publication_announced(
            second,
            "moqcast.screen/current".to_owned(),
            publication::AudioStatus::Off,
        ));
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
        assert!(snapshot.set_share_system_audio(true));
        let start = snapshot.begin_share().expect("share");
        assert!(snapshot.publication_announced(
            start.generation,
            "moqcast.screen/current".to_owned(),
            publication::AudioStatus::Included,
        ));
        let stopping = snapshot.begin_stop_share().expect("stop share");
        assert!(snapshot.finish_stop_media(stopping));

        assert_eq!(snapshot.media.phase(), MediaPhase::Idle);
        assert_eq!(snapshot.media_owner, None);
        assert_eq!(snapshot.session.phase(), SessionPhase::Listening);
        assert_eq!(snapshot.session.generation(), session_generation);
        assert_eq!(snapshot.peers["peer"].session, PeerSession::Connected);
        assert!(snapshot.share_selection.is_some());
        assert!(!snapshot.share_system_audio);
        assert_eq!(snapshot.share_audio, ShareAudioPhase::Off);
        assert!(snapshot.share_audio_error.is_none());
    }

    #[test]
    fn unavailable_share_source_requires_a_new_picker_selection() {
        let mut snapshot = AppSnapshot::default();
        assert!(snapshot.select_share_source(display_selection()));
        let generation = snapshot.begin_share().expect("share").generation;

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
