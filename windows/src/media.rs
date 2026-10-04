//! Single-publication screen media lifecycle and Windows capture pipeline.

use moq_tokio::moq_net;

use crate::{
    audio::{AudioSnapshot, StatusUpdate as AudioStatusUpdate},
    screen_path,
};

pub(crate) const COMPATIBLE_MAX_SCREEN_EDGE: u32 = 1920;
#[cfg(any(target_os = "windows", test))]
const WINDOW_ENUMERATION_WARNING: &str =
    "Windows could not enumerate capturable windows. Any listed displays remain available.";

#[cfg(any(target_os = "windows", test))]
const QHD_WIDTH: u32 = 2560;
#[cfg(any(target_os = "windows", test))]
const QHD_HEIGHT: u32 = 1440;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum VideoEncodingPolicy {
    #[default]
    Compatible,
    NativeQhdHardware,
}

impl VideoEncodingPolicy {
    #[cfg(any(target_os = "windows", test))]
    fn resolve(self, info: PublicationInfo) -> Result<VideoEncodingPlan, PublicationFailure> {
        if info.width == 0 || info.height == 0 {
            return Err(PublicationFailure::CaptureUnavailable);
        }
        let encoder = match self {
            Self::Compatible if info.width.max(info.height) <= COMPATIBLE_MAX_SCREEN_EDGE => {
                EncoderRequirement::Auto
            }
            Self::Compatible => return Err(PublicationFailure::CompatibleSourceTooLarge),
            Self::NativeQhdHardware if (info.width, info.height) == (QHD_WIDTH, QHD_HEIGHT) => {
                EncoderRequirement::HardwareOnly
            }
            Self::NativeQhdHardware => {
                return Err(PublicationFailure::NativeQhdSourceRequired);
            }
        };
        Ok(VideoEncodingPlan {
            policy: self,
            encoder,
            info,
        })
    }

    #[cfg(target_os = "windows")]
    fn name(self) -> &'static str {
        match self {
            Self::Compatible => "compatible",
            Self::NativeQhdHardware => "native-qhd-hardware",
        }
    }
}

#[cfg(any(target_os = "windows", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EncoderRequirement {
    Auto,
    HardwareOnly,
}

#[cfg(any(target_os = "windows", test))]
impl EncoderRequirement {
    #[cfg(target_os = "windows")]
    fn name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::HardwareOnly => "hardware-only",
        }
    }

    #[cfg(target_os = "windows")]
    fn kind(self) -> moq_video::encode::Kind {
        match self {
            Self::Auto => moq_video::encode::Kind::Auto,
            Self::HardwareOnly => moq_video::encode::Kind::Hardware,
        }
    }
}

#[cfg(any(target_os = "windows", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct VideoEncodingPlan {
    policy: VideoEncodingPolicy,
    encoder: EncoderRequirement,
    info: PublicationInfo,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PublicationFailure {
    CaptureUnavailable,
    NoCaptureSourcesAvailable,
    CaptureSourceSelectionRequired,
    CaptureSourceUnavailable,
    #[cfg(any(target_os = "windows", test))]
    CompatibleSourceTooLarge,
    #[cfg(any(target_os = "windows", test))]
    NativeQhdSourceRequired,
    #[cfg(any(target_os = "windows", test))]
    NativeQhdUnavailable,
    Unexpected,
}

impl PublicationFailure {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::CaptureUnavailable => "Windows could not open the selected capture source.",
            Self::NoCaptureSourcesAvailable => {
                "No capturable Windows displays or windows are available."
            }
            Self::CaptureSourceSelectionRequired => {
                "Choose an available display or window before starting screen sharing."
            }
            Self::CaptureSourceUnavailable => {
                "The selected capture source is unavailable. Restore a minimized window or choose a source again."
            }
            #[cfg(any(target_os = "windows", test))]
            Self::CompatibleSourceTooLarge => {
                "Compatible mode supports native sources with a longest edge up to 1920 pixels."
            }
            #[cfg(any(target_os = "windows", test))]
            Self::NativeQhdSourceRequired => {
                "Native QHD mode currently requires a landscape 2560x1440 capture source."
            }
            #[cfg(any(target_os = "windows", test))]
            Self::NativeQhdUnavailable => {
                "No hardware H.264 encoder could be opened for native QHD sharing."
            }
            Self::Unexpected => "Screen publication ended unexpectedly.",
        }
    }
}

#[cfg(any(target_os = "windows", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PublicationErrorKind {
    NoEncoder,
    SourceUnavailable,
    Other,
}

#[cfg(any(target_os = "windows", test))]
fn classify_publication_failure(
    policy: VideoEncodingPolicy,
    error: PublicationErrorKind,
) -> PublicationFailure {
    match (policy, error) {
        (VideoEncodingPolicy::NativeQhdHardware, PublicationErrorKind::NoEncoder) => {
            PublicationFailure::NativeQhdUnavailable
        }
        (_, PublicationErrorKind::SourceUnavailable) => {
            PublicationFailure::CaptureSourceUnavailable
        }
        _ => PublicationFailure::Unexpected,
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum MediaPhase {
    #[default]
    Idle,
    Preparing,
    Sharing,
    Stopping,
    Failed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum CaptureSourceCatalogPhase {
    #[default]
    Loading,
    Ready,
    Empty,
    Failed,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub(crate) enum CaptureSourceKind {
    Display,
    Window,
}

impl CaptureSourceKind {
    #[cfg(target_os = "windows")]
    fn name(self) -> &'static str {
        match self {
            Self::Display => "display",
            Self::Window => "window",
        }
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub(crate) struct CaptureSourceChoice {
    pub(crate) kind: CaptureSourceKind,
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) application: Option<String>,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CaptureSourceCatalogSnapshot {
    pub(crate) phase: CaptureSourceCatalogPhase,
    pub(crate) choices: Vec<CaptureSourceChoice>,
    pub(crate) selected: Option<CaptureSourceChoice>,
    pub(crate) warning: Option<&'static str>,
    pub(crate) last_error: Option<&'static str>,
    selection_invalidated: bool,
}

impl Default for CaptureSourceCatalogSnapshot {
    fn default() -> Self {
        Self {
            phase: CaptureSourceCatalogPhase::Loading,
            choices: Vec::new(),
            selected: None,
            warning: None,
            last_error: None,
            selection_invalidated: false,
        }
    }
}

impl CaptureSourceCatalogSnapshot {
    fn begin_refresh(&mut self) {
        self.phase = CaptureSourceCatalogPhase::Loading;
        self.warning = None;
        self.last_error = None;
    }

    fn refreshed(&mut self, choices: Vec<CaptureSourceChoice>, warning: Option<&'static str>) {
        let previous = self.selected.take();
        self.phase = if choices.is_empty() {
            CaptureSourceCatalogPhase::Empty
        } else {
            CaptureSourceCatalogPhase::Ready
        };
        self.choices = choices;
        self.warning = warning;
        self.last_error = None;

        match previous {
            Some(selected) if selected.kind == CaptureSourceKind::Window => {
                if let Some(current) = self
                    .choices
                    .iter()
                    .find(|current| current.kind == selected.kind && current.id == selected.id)
                {
                    self.selected = Some(current.clone());
                } else {
                    self.selection_invalidated = true;
                    self.last_error = Some(
                        "The selected capture source is no longer available. Choose a source again.",
                    );
                }
            }
            Some(selected) if self.choices.contains(&selected) => {
                self.selected = Some(selected);
            }
            Some(_) => {
                self.selection_invalidated = true;
                self.last_error = Some(
                    "The selected capture source is no longer available. Choose a source again.",
                );
            }
            None if !self.selection_invalidated => {
                self.selected = self.choices.first().cloned();
            }
            None => {}
        }
    }

    fn failed(&mut self) {
        self.phase = CaptureSourceCatalogPhase::Failed;
        self.warning = None;
        self.last_error = Some("Windows could not enumerate capturable displays and windows.");
    }

    fn select(&mut self, choice: &CaptureSourceChoice) -> bool {
        if self.phase != CaptureSourceCatalogPhase::Ready {
            return false;
        }
        let Some(selected) = self.choices.iter().find(|display| *display == choice) else {
            return false;
        };
        self.selected = Some(selected.clone());
        self.selection_invalidated = false;
        self.last_error = None;
        true
    }

    fn invalidate_selection(&mut self) {
        self.selected = None;
        self.selection_invalidated = true;
        self.last_error =
            Some("The selected capture source is no longer available. Choose a source again.");
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CaptureSourceEnumeration {
    choices: Vec<CaptureSourceChoice>,
    warning: Option<&'static str>,
}

#[cfg(any(target_os = "windows", test))]
fn combine_capture_sources(
    mut displays: Vec<CaptureSourceChoice>,
    windows: Result<Vec<CaptureSourceChoice>, PublicationFailure>,
) -> CaptureSourceEnumeration {
    let warning = match windows {
        Ok(mut windows) => {
            displays.append(&mut windows);
            None
        }
        Err(_) => Some(WINDOW_ENUMERATION_WARNING),
    };
    CaptureSourceEnumeration {
        choices: displays,
        warning,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MediaSnapshot {
    pub(crate) generation: u64,
    pub(crate) phase: MediaPhase,
    pub(crate) audio: AudioSnapshot,
    pub(crate) capture_sources: CaptureSourceCatalogSnapshot,
    pub(crate) video_encoding: VideoEncodingPolicy,
    pub(crate) path: Option<String>,
    pub(crate) width: Option<u32>,
    pub(crate) height: Option<u32>,
    pub(crate) last_error: Option<&'static str>,
}

impl Default for MediaSnapshot {
    fn default() -> Self {
        Self {
            generation: 0,
            phase: MediaPhase::Idle,
            audio: AudioSnapshot::default(),
            capture_sources: CaptureSourceCatalogSnapshot::default(),
            video_encoding: VideoEncodingPolicy::default(),
            path: None,
            width: None,
            height: None,
            last_error: None,
        }
    }
}

impl MediaSnapshot {
    pub(crate) fn begin_capture_source_refresh(&mut self) -> bool {
        if !matches!(self.phase, MediaPhase::Idle | MediaPhase::Failed) {
            return false;
        }
        self.capture_sources.begin_refresh();
        true
    }

    #[cfg(test)]
    pub(crate) fn capture_sources_refreshed(&mut self, choices: Vec<CaptureSourceChoice>) {
        self.capture_sources.refreshed(choices, None);
    }

    pub(crate) fn capture_source_enumeration_refreshed(
        &mut self,
        enumeration: CaptureSourceEnumeration,
    ) {
        self.capture_sources
            .refreshed(enumeration.choices, enumeration.warning);
    }

    pub(crate) fn capture_source_refresh_failed(&mut self) {
        self.capture_sources.failed();
    }

    pub(crate) fn select_capture_source(&mut self, choice: &CaptureSourceChoice) -> bool {
        if !matches!(self.phase, MediaPhase::Idle | MediaPhase::Failed) {
            return false;
        }
        let selected = self.capture_sources.select(choice);
        if selected {
            self.last_error = None;
        }
        selected
    }

    pub(crate) fn reject_start(&mut self, failure: PublicationFailure) -> bool {
        if !matches!(self.phase, MediaPhase::Idle | MediaPhase::Failed) {
            return false;
        }
        self.phase = MediaPhase::Failed;
        self.last_error = Some(failure.message());
        true
    }

    pub(crate) fn set_video_encoding_policy(&mut self, policy: VideoEncodingPolicy) -> bool {
        if !matches!(self.phase, MediaPhase::Idle | MediaPhase::Failed) {
            return false;
        }
        self.video_encoding = policy;
        self.last_error = None;
        true
    }

    pub(crate) fn begin(&mut self, local_peer_id: &str) -> Option<u64> {
        if matches!(
            self.phase,
            MediaPhase::Preparing | MediaPhase::Sharing | MediaPhase::Stopping
        ) {
            return None;
        }
        self.generation = self.generation.saturating_add(1);
        self.phase = MediaPhase::Preparing;
        self.path = Some(screen_path::for_peer(local_peer_id));
        self.width = None;
        self.height = None;
        self.last_error = None;
        self.audio.begin(self.generation);
        Some(self.generation)
    }

    pub(crate) fn started(&mut self, generation: u64, info: PublicationInfo) -> bool {
        if generation != self.generation || self.phase != MediaPhase::Preparing {
            return false;
        }
        self.phase = MediaPhase::Sharing;
        self.width = Some(info.width);
        self.height = Some(info.height);
        true
    }

    pub(crate) fn begin_stop(&mut self) -> Option<u64> {
        if !matches!(self.phase, MediaPhase::Preparing | MediaPhase::Sharing) {
            return None;
        }
        self.phase = MediaPhase::Stopping;
        self.audio.begin_stop(self.generation);
        Some(self.generation)
    }

    pub(crate) fn stopped(&mut self, generation: u64) -> bool {
        if generation != self.generation || self.phase != MediaPhase::Stopping {
            return false;
        }
        self.phase = MediaPhase::Idle;
        self.path = None;
        self.width = None;
        self.height = None;
        self.audio.ended(generation);
        true
    }

    pub(crate) fn ended(
        &mut self,
        generation: u64,
        result: Result<(), PublicationFailure>,
    ) -> bool {
        if generation != self.generation
            || !matches!(self.phase, MediaPhase::Preparing | MediaPhase::Sharing)
        {
            return false;
        }
        self.audio.ended(generation);
        match result {
            Ok(()) => {
                self.phase = MediaPhase::Idle;
                self.path = None;
                self.width = None;
                self.height = None;
            }
            Err(failure) => {
                if failure == PublicationFailure::CaptureSourceUnavailable {
                    self.capture_sources.invalidate_selection();
                }
                self.phase = MediaPhase::Failed;
                self.last_error = Some(failure.message());
            }
        }
        true
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PublicationInfo {
    pub(crate) width: u32,
    pub(crate) height: u32,
}

pub(crate) struct Publication {
    #[cfg(target_os = "windows")]
    broadcast: moq_net::broadcast::Producer,
    #[cfg(target_os = "windows")]
    catalog: moq_mux::catalog::Producer,
    #[cfg(target_os = "windows")]
    clock: moq_mux::Clock,
}

pub(crate) struct ReadyPublication {
    #[cfg(target_os = "windows")]
    publication: Publication,
    #[cfg(target_os = "windows")]
    source: moq_video::capture::Source,
    #[cfg(target_os = "windows")]
    source_kind: CaptureSourceKind,
    #[cfg(target_os = "windows")]
    plan: VideoEncodingPlan,
    #[cfg(not(target_os = "windows"))]
    info: PublicationInfo,
}

impl ReadyPublication {
    pub(crate) fn info(&self) -> PublicationInfo {
        #[cfg(target_os = "windows")]
        {
            self.plan.info
        }
        #[cfg(not(target_os = "windows"))]
        {
            self.info
        }
    }

    pub(crate) async fn run(
        self,
        generation: u64,
        audio_updates: tokio::sync::watch::Sender<Option<AudioStatusUpdate>>,
    ) -> Result<(), PublicationFailure> {
        #[cfg(target_os = "windows")]
        {
            let mut capture = moq_video::capture::Config::default();
            capture.source = self.source;
            capture.framerate = Some(moq_video::Rate::new(30, 1).expect("valid frame rate"));

            let mut encode = moq_video::encode::Options::default();
            encode.codec = moq_video::encode::Codec::H264;
            encode.kind = self.plan.encoder.kind();

            tracing::info!(
                video_policy = self.plan.policy.name(),
                capture_source_kind = self.source_kind.name(),
                source_width = self.plan.info.width,
                source_height = self.plan.info.height,
                encoder_kind = self.plan.encoder.name(),
                codec = "H.264",
                "screen publication requested"
            );

            let clock = self.publication.clock;
            let audio = crate::audio::publish(
                self.publication.broadcast.clone(),
                self.publication.catalog.clone(),
                clock,
                generation,
                audio_updates,
            );
            let video = moq_video::encode::publish_capture(
                self.publication.broadcast.clone(),
                self.publication.catalog.clone(),
                capture,
                encode,
                clock,
            );
            tokio::pin!(audio);
            tokio::pin!(video);

            let result = tokio::select! {
                result = &mut video => result,
                () = &mut audio => video.await,
            };
            result.map_err(|error| {
                tracing::warn!(
                    video_policy = self.plan.policy.name(),
                    source_width = self.plan.info.width,
                    source_height = self.plan.info.height,
                    encoder_kind = self.plan.encoder.name(),
                    %error,
                    "screen publication failed"
                );
                let error = match error {
                    moq_video::Error::NoEncoder(_) => PublicationErrorKind::NoEncoder,
                    moq_video::Error::SourceUnavailable(_) => {
                        PublicationErrorKind::SourceUnavailable
                    }
                    _ => PublicationErrorKind::Other,
                };
                classify_publication_failure(self.plan.policy, error)
            })
        }

        #[cfg(not(target_os = "windows"))]
        {
            let _ = (generation, audio_updates);
            Err(PublicationFailure::CaptureUnavailable)
        }
    }
}

impl Publication {
    pub(crate) async fn enumerate_capture_sources()
    -> Result<CaptureSourceEnumeration, PublicationFailure> {
        #[cfg(target_os = "windows")]
        {
            let displays = moq_video::capture::displays().await.map_err(|error| {
                tracing::warn!(%error, "could not enumerate Windows displays");
                PublicationFailure::CaptureUnavailable
            })?;
            let displays = displays
                .into_iter()
                .map(|display| CaptureSourceChoice {
                    kind: CaptureSourceKind::Display,
                    id: display.id,
                    name: display.name,
                    application: None,
                    width: display.width,
                    height: display.height,
                })
                .collect();
            let windows = moq_video::capture::windows()
                .await
                .map(|windows| {
                    windows
                        .into_iter()
                        .map(|window| CaptureSourceChoice {
                            kind: CaptureSourceKind::Window,
                            id: window.id,
                            name: window.title,
                            application: Some(window.app),
                            width: window.width,
                            height: window.height,
                        })
                        .collect()
                })
                .map_err(|error| {
                    tracing::warn!(%error, "could not enumerate Windows windows");
                    PublicationFailure::CaptureUnavailable
                });
            Ok(combine_capture_sources(displays, windows))
        }

        #[cfg(not(target_os = "windows"))]
        {
            Err(PublicationFailure::CaptureUnavailable)
        }
    }

    pub(crate) fn prepare(
        origin: &moq_net::origin::Producer,
        local_peer_id: &str,
    ) -> anyhow::Result<Self> {
        #[cfg(target_os = "windows")]
        {
            let path = screen_path::for_peer(local_peer_id);
            let mut broadcast = origin.create_broadcast(path)?;
            let clock = moq_mux::Clock::new();
            let catalog = moq_mux::catalog::Producer::new(
                &mut broadcast,
                moq_mux::catalog::Config::default().with_clock(clock),
            )?;
            broadcast.announce(moq_net::origin::Route::default())?;
            Ok(Self {
                broadcast,
                catalog,
                clock,
            })
        }

        #[cfg(not(target_os = "windows"))]
        {
            let _ = (origin, local_peer_id);
            anyhow::bail!("Windows screen capture is unavailable on this host")
        }
    }

    pub(crate) async fn configure(
        self,
        selected: &CaptureSourceChoice,
        policy: VideoEncodingPolicy,
    ) -> Result<ReadyPublication, PublicationFailure> {
        #[cfg(target_os = "windows")]
        {
            let (source, info) = match selected.kind {
                CaptureSourceKind::Display => {
                    let display = moq_video::capture::displays()
                        .await
                        .map_err(|error| {
                            tracing::warn!(%error, "could not enumerate Windows displays");
                            PublicationFailure::CaptureUnavailable
                        })?
                        .into_iter()
                        .find(|display| {
                            display.id == selected.id
                                && display.name == selected.name
                                && display.width == selected.width
                                && display.height == selected.height
                        })
                        .ok_or(PublicationFailure::CaptureSourceUnavailable)?;
                    let info = PublicationInfo {
                        width: display.width,
                        height: display.height,
                    };
                    (display.source(), info)
                }
                CaptureSourceKind::Window => {
                    let window = moq_video::capture::windows()
                        .await
                        .map_err(|error| {
                            tracing::warn!(%error, "could not enumerate Windows windows");
                            PublicationFailure::CaptureUnavailable
                        })?
                        .into_iter()
                        .find(|window| window.id == selected.id)
                        .ok_or(PublicationFailure::CaptureSourceUnavailable)?;
                    let info = PublicationInfo {
                        width: window.width,
                        height: window.height,
                    };
                    (window.source(), info)
                }
            };
            let plan = policy.resolve(info).inspect_err(|error| {
                tracing::warn!(
                    video_policy = policy.name(),
                    source_width = info.width,
                    source_height = info.height,
                    reason = error.message(),
                    "screen source does not satisfy the requested encoding policy"
                );
            })?;
            Ok(ReadyPublication {
                publication: self,
                source,
                source_kind: selected.kind,
                plan,
            })
        }

        #[cfg(not(target_os = "windows"))]
        {
            let _ = self;
            let _ = selected;
            let _ = policy;
            Err(PublicationFailure::CaptureUnavailable)
        }
    }
}

#[cfg(target_os = "windows")]
impl Drop for Publication {
    fn drop(&mut self) {
        self.broadcast.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(id: &str, title: &str, app: &str, width: u32, height: u32) -> CaptureSourceChoice {
        CaptureSourceChoice {
            kind: CaptureSourceKind::Window,
            id: id.to_owned(),
            name: title.to_owned(),
            application: Some(app.to_owned()),
            width,
            height,
        }
    }

    fn display(id: &str, name: &str, width: u32, height: u32) -> CaptureSourceChoice {
        CaptureSourceChoice {
            kind: CaptureSourceKind::Display,
            id: id.to_owned(),
            name: name.to_owned(),
            application: None,
            width,
            height,
        }
    }

    #[test]
    fn first_display_refresh_selects_once_but_never_replaces_a_missing_choice() {
        let first = display("display:0", "Display 1", 1920, 1080);
        let second = display("display:1", "Display 2", 1280, 720);
        let mut media = MediaSnapshot::default();

        assert!(media.begin_capture_source_refresh());
        media.capture_sources_refreshed(vec![first.clone(), second.clone()]);
        assert_eq!(media.capture_sources.selected, Some(first.clone()));

        assert!(media.select_capture_source(&second));
        assert!(media.begin_capture_source_refresh());
        media.capture_sources_refreshed(vec![first.clone()]);
        assert!(media.capture_sources.selected.is_none());
        assert!(media.capture_sources.last_error.is_some());

        assert!(media.begin_capture_source_refresh());
        media.capture_sources_refreshed(vec![first]);
        assert!(media.capture_sources.selected.is_none());
    }

    #[test]
    fn refresh_preserves_an_exact_window_and_invalidates_a_closed_one() {
        let editor = window("window:7", "Notes", "Editor", 1280, 720);
        let terminal = window("window:8", "Build", "Terminal", 960, 720);
        let mut media = MediaSnapshot::default();

        media.capture_sources_refreshed(vec![editor.clone(), terminal]);
        assert!(media.select_capture_source(&editor));

        assert!(media.begin_capture_source_refresh());
        let renamed = window("window:7", "Notes (saved)", "Editor", 1024, 768);
        media.capture_sources_refreshed(vec![renamed.clone()]);
        assert_eq!(media.capture_sources.selected, Some(renamed));

        assert!(media.begin_capture_source_refresh());
        media.capture_sources_refreshed(Vec::new());
        assert!(media.capture_sources.selected.is_none());
        assert_eq!(
            media.capture_sources.last_error,
            Some("The selected capture source is no longer available. Choose a source again.")
        );
    }

    #[test]
    fn source_kind_is_part_of_the_selection_identity() {
        let display = CaptureSourceChoice {
            kind: CaptureSourceKind::Display,
            id: "source:1".to_owned(),
            name: "Screen".to_owned(),
            application: None,
            width: 1920,
            height: 1080,
        };
        let window = CaptureSourceChoice {
            kind: CaptureSourceKind::Window,
            application: Some("App".to_owned()),
            ..display.clone()
        };
        let mut media = MediaSnapshot::default();

        media.capture_sources_refreshed(vec![display.clone(), window.clone()]);
        assert!(media.select_capture_source(&window));
        assert_eq!(media.capture_sources.selected, Some(window));
        assert_ne!(display, media.capture_sources.selected.unwrap());
    }

    #[test]
    fn display_refresh_requires_an_exact_current_descriptor() {
        let selected = display("display:0", "Display 1", 1920, 1080);
        let mut media = MediaSnapshot::default();
        media.capture_sources_refreshed(vec![selected.clone()]);
        assert_eq!(media.capture_sources.selected, Some(selected.clone()));

        media.begin_capture_source_refresh();
        media.capture_sources_refreshed(vec![display("display:0", "Display 2", 2560, 1440)]);

        assert!(media.capture_sources.selected.is_none());
        let refreshed = display("display:0", "Display 2", 2560, 1440);
        assert!(!media.select_capture_source(&selected));
        assert!(media.select_capture_source(&refreshed));
        assert_eq!(media.capture_sources.selected, Some(refreshed));
    }

    #[test]
    fn display_selection_is_locked_for_the_entire_publication_lifecycle() {
        let first = display("display:0", "Display 1", 1920, 1080);
        let second = display("display:1", "Display 2", 1280, 720);
        let mut media = MediaSnapshot::default();
        media.capture_sources_refreshed(vec![first.clone(), second.clone()]);
        let generation = media.begin("peer-a").expect("begin");

        assert!(!media.select_capture_source(&second));
        assert!(!media.begin_capture_source_refresh());
        assert_eq!(media.capture_sources.selected, Some(first));

        assert!(media.started(
            generation,
            PublicationInfo {
                width: 1920,
                height: 1080,
            }
        ));
        assert!(!media.select_capture_source(&second));
        assert!(!media.begin_capture_source_refresh());

        assert_eq!(media.begin_stop(), Some(generation));
        assert!(!media.select_capture_source(&second));
        assert!(!media.begin_capture_source_refresh());
        assert!(media.stopped(generation));
        assert!(media.select_capture_source(&second));
        assert_eq!(media.capture_sources.selected, Some(second));
    }

    #[test]
    fn display_failed_or_empty_refresh_preserves_explicit_retry_semantics() {
        let mut media = MediaSnapshot::default();
        media.capture_sources_refreshed(Vec::new());
        assert_eq!(
            media.capture_sources.phase,
            CaptureSourceCatalogPhase::Empty
        );
        assert!(media.capture_sources.selected.is_none());

        media.begin_capture_source_refresh();
        media.capture_source_refresh_failed();
        assert_eq!(
            media.capture_sources.phase,
            CaptureSourceCatalogPhase::Failed
        );
        assert!(media.capture_sources.last_error.is_some());

        media.begin_capture_source_refresh();
        let available = display("display:0", "Display 1", 1920, 1080);
        media.capture_sources_refreshed(vec![available.clone()]);
        assert_eq!(media.capture_sources.selected, Some(available));
    }

    #[test]
    fn media_lifecycle_is_single_generation_and_stop_is_explicit() {
        let mut media = MediaSnapshot::default();
        let generation = media.begin("peer-a").expect("begin");
        assert_eq!(media.path.as_deref(), Some("moqcast.screen/peer-a"));
        assert!(media.begin("peer-a").is_none());
        assert!(media.started(
            generation,
            PublicationInfo {
                width: 1920,
                height: 1080,
            }
        ));
        assert_eq!(media.phase, MediaPhase::Sharing);
        assert_eq!(media.audio.phase, crate::audio::AudioPhase::Preparing);
        assert_eq!(media.begin_stop(), Some(generation));
        assert_eq!(media.audio.phase, crate::audio::AudioPhase::Stopping);
        assert!(media.stopped(generation));
        assert_eq!(media.phase, MediaPhase::Idle);
        assert_eq!(media.audio.phase, crate::audio::AudioPhase::Idle);
    }

    #[test]
    fn old_publication_generation_cannot_replace_current_state() {
        let mut media = MediaSnapshot::default();
        let old = media.begin("peer-a").expect("old");
        media.phase = MediaPhase::Failed;
        let current = media.begin("peer-a").expect("current");
        assert!(current > old);
        assert!(!media.ended(old, Err(PublicationFailure::Unexpected)));
        assert_eq!(media.phase, MediaPhase::Preparing);
    }

    #[test]
    fn source_unavailable_invalidates_only_the_current_publication_selection() {
        let selected = window("window:7", "Notes", "Editor", 1280, 720);
        let mut media = MediaSnapshot::default();
        media.capture_sources_refreshed(vec![selected.clone()]);

        let old = media.begin("peer-a").expect("old");
        media.phase = MediaPhase::Failed;
        let current = media.begin("peer-a").expect("current");

        assert!(!media.ended(old, Err(PublicationFailure::CaptureSourceUnavailable)));
        assert_eq!(media.capture_sources.selected, Some(selected.clone()));

        assert!(media.started(
            current,
            PublicationInfo {
                width: selected.width,
                height: selected.height,
            }
        ));
        assert!(media.ended(current, Err(PublicationFailure::CaptureSourceUnavailable)));
        assert!(media.capture_sources.selected.is_none());

        assert!(media.begin_capture_source_refresh());
        media.capture_sources_refreshed(vec![selected.clone()]);
        assert!(media.capture_sources.selected.is_none());
        assert!(media.select_capture_source(&selected));
    }

    #[test]
    fn failed_window_enumeration_keeps_displays_and_drops_window_selection() {
        let display = display("display:0", "Display 1", 1920, 1080);
        let window = window("window:7", "Notes", "Editor", 1280, 720);
        let partial = combine_capture_sources(
            vec![display.clone()],
            Err(PublicationFailure::CaptureUnavailable),
        );

        let mut display_selected = MediaSnapshot::default();
        display_selected.capture_sources_refreshed(vec![display.clone(), window.clone()]);
        assert!(display_selected.select_capture_source(&display));
        display_selected.capture_source_enumeration_refreshed(partial.clone());
        assert_eq!(
            display_selected.capture_sources.phase,
            CaptureSourceCatalogPhase::Ready
        );
        assert_eq!(
            display_selected.capture_sources.choices,
            vec![display.clone()]
        );
        assert_eq!(
            display_selected.capture_sources.selected,
            Some(display.clone())
        );
        assert_eq!(
            display_selected.capture_sources.warning,
            Some(WINDOW_ENUMERATION_WARNING)
        );

        let mut window_selected = MediaSnapshot::default();
        window_selected.capture_sources_refreshed(vec![display.clone(), window.clone()]);
        assert!(window_selected.select_capture_source(&window));
        window_selected.capture_source_enumeration_refreshed(partial);
        assert_eq!(window_selected.capture_sources.choices, vec![display]);
        assert!(window_selected.capture_sources.selected.is_none());
        assert_eq!(
            window_selected.capture_sources.warning,
            Some(WINDOW_ENUMERATION_WARNING)
        );
    }

    #[test]
    fn late_publication_end_cannot_override_an_explicit_stop() {
        let mut media = MediaSnapshot::default();
        let generation = media.begin("peer-a").expect("begin");
        assert!(media.started(
            generation,
            PublicationInfo {
                width: 1920,
                height: 1080,
            }
        ));
        assert_eq!(media.begin_stop(), Some(generation));
        assert!(media.stopped(generation));
        assert!(!media.ended(generation, Err(PublicationFailure::Unexpected)));
        assert_eq!(media.phase, MediaPhase::Idle);
    }

    #[test]
    fn compatible_policy_preserves_native_sizes_up_to_the_existing_limit() {
        assert_eq!(
            VideoEncodingPolicy::default(),
            VideoEncodingPolicy::Compatible
        );
        for info in [
            PublicationInfo {
                width: 1280,
                height: 720,
            },
            PublicationInfo {
                width: 1920,
                height: 1080,
            },
            PublicationInfo {
                width: 1080,
                height: 1920,
            },
        ] {
            let plan = VideoEncodingPolicy::Compatible
                .resolve(info)
                .expect("compatible native size");
            assert_eq!(plan.info, info);
            assert_eq!(plan.encoder, EncoderRequirement::Auto);
        }
    }

    #[test]
    fn native_qhd_is_exact_landscape_and_hardware_only() {
        let info = PublicationInfo {
            width: 2560,
            height: 1440,
        };
        let plan = VideoEncodingPolicy::NativeQhdHardware
            .resolve(info)
            .expect("native landscape QHD");
        assert_eq!(plan.info, info);
        assert_eq!(plan.encoder, EncoderRequirement::HardwareOnly);
    }

    #[test]
    fn policies_reject_mismatched_empty_and_oversized_sources() {
        let qhd = VideoEncodingPolicy::NativeQhdHardware;
        for info in [
            PublicationInfo {
                width: 1920,
                height: 1080,
            },
            PublicationInfo {
                width: 1440,
                height: 2560,
            },
            PublicationInfo {
                width: 3840,
                height: 2160,
            },
            PublicationInfo {
                width: 3440,
                height: 1440,
            },
        ] {
            assert_eq!(
                qhd.resolve(info),
                Err(PublicationFailure::NativeQhdSourceRequired)
            );
        }
        assert!(
            VideoEncodingPolicy::Compatible
                .resolve(PublicationInfo {
                    width: 2560,
                    height: 1440,
                })
                .is_err()
        );
        assert!(
            VideoEncodingPolicy::Compatible
                .resolve(PublicationInfo {
                    width: 0,
                    height: 1080,
                })
                .is_err()
        );
    }

    #[test]
    fn native_qhd_runtime_failure_is_explicit_and_does_not_claim_fallback() {
        let message = PublicationFailure::NativeQhdUnavailable.message();
        assert!(message.contains("hardware H.264"));
        assert!(!message.contains("OpenH264"));
        assert_eq!(
            classify_publication_failure(
                VideoEncodingPolicy::NativeQhdHardware,
                PublicationErrorKind::NoEncoder,
            ),
            PublicationFailure::NativeQhdUnavailable
        );
        assert_eq!(
            classify_publication_failure(
                VideoEncodingPolicy::NativeQhdHardware,
                PublicationErrorKind::Other,
            ),
            PublicationFailure::Unexpected
        );
        assert_eq!(
            classify_publication_failure(
                VideoEncodingPolicy::Compatible,
                PublicationErrorKind::NoEncoder,
            ),
            PublicationFailure::Unexpected
        );
        assert_eq!(
            classify_publication_failure(
                VideoEncodingPolicy::Compatible,
                PublicationErrorKind::SourceUnavailable,
            ),
            PublicationFailure::CaptureSourceUnavailable
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn native_qhd_maps_to_the_moq_video_hardware_kind() {
        let plan = VideoEncodingPolicy::NativeQhdHardware
            .resolve(PublicationInfo {
                width: QHD_WIDTH,
                height: QHD_HEIGHT,
            })
            .expect("native QHD plan");
        assert_eq!(plan.encoder.kind(), moq_video::encode::Kind::Hardware);
    }
}
