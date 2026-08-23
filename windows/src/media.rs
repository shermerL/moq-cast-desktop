//! Single-publication screen media lifecycle and Windows capture pipeline.

use moq_native::moq_net;

use crate::{
    audio::{AudioSnapshot, StatusUpdate as AudioStatusUpdate},
    screen_path,
};

pub(crate) const COMPATIBLE_MAX_SCREEN_EDGE: u32 = 1920;

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
            Self::Compatible => return Err(PublicationFailure::CompatibleDisplayTooLarge),
            Self::NativeQhdHardware if (info.width, info.height) == (QHD_WIDTH, QHD_HEIGHT) => {
                EncoderRequirement::HardwareOnly
            }
            Self::NativeQhdHardware => {
                return Err(PublicationFailure::NativeQhdDisplayRequired);
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
    #[cfg(any(target_os = "windows", test))]
    CompatibleDisplayTooLarge,
    #[cfg(any(target_os = "windows", test))]
    NativeQhdDisplayRequired,
    #[cfg(any(target_os = "windows", test))]
    NativeQhdUnavailable,
    Unexpected,
}

impl PublicationFailure {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::CaptureUnavailable => "Windows could not open a capturable display.",
            #[cfg(any(target_os = "windows", test))]
            Self::CompatibleDisplayTooLarge => {
                "Compatible mode supports native displays with a longest edge up to 1920 pixels."
            }
            #[cfg(any(target_os = "windows", test))]
            Self::NativeQhdDisplayRequired => {
                "Native QHD mode currently requires a landscape 2560x1440 display."
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
fn classify_publication_failure(
    policy: VideoEncodingPolicy,
    no_encoder: bool,
) -> PublicationFailure {
    match (policy, no_encoder) {
        (VideoEncodingPolicy::NativeQhdHardware, true) => PublicationFailure::NativeQhdUnavailable,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MediaSnapshot {
    pub(crate) generation: u64,
    pub(crate) phase: MediaPhase,
    pub(crate) audio: AudioSnapshot,
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
            video_encoding: VideoEncodingPolicy::default(),
            path: None,
            width: None,
            height: None,
            last_error: None,
        }
    }
}

impl MediaSnapshot {
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
}

pub(crate) struct ReadyPublication {
    #[cfg(target_os = "windows")]
    publication: Publication,
    #[cfg(target_os = "windows")]
    source: moq_video::capture::Source,
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
            capture.framerate = Some(30);

            let mut encode = moq_video::encode::Options::default();
            encode.codec = moq_video::encode::Codec::H264;
            encode.kind = self.plan.encoder.kind();

            tracing::info!(
                video_policy = self.plan.policy.name(),
                source_width = self.plan.info.width,
                source_height = self.plan.info.height,
                encoder_kind = self.plan.encoder.name(),
                codec = "H.264",
                "screen publication requested"
            );

            let clock = moq_mux::Clock::new();
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
                classify_publication_failure(
                    self.plan.policy,
                    matches!(error, moq_video::Error::NoEncoder(_)),
                )
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
    pub(crate) fn prepare(
        origin: &moq_net::origin::Producer,
        local_peer_id: &str,
    ) -> anyhow::Result<Self> {
        #[cfg(target_os = "windows")]
        {
            let path = screen_path::for_peer(local_peer_id);
            let mut broadcast = origin
                .create_broadcast(path, moq_net::broadcast::Route::new().with_announce(true))?;
            let catalog = moq_mux::catalog::Producer::new(&mut broadcast)?;
            Ok(Self { broadcast, catalog })
        }

        #[cfg(not(target_os = "windows"))]
        {
            let _ = (origin, local_peer_id);
            anyhow::bail!("Windows screen capture is unavailable on this host")
        }
    }

    pub(crate) async fn configure(
        self,
        policy: VideoEncodingPolicy,
    ) -> Result<ReadyPublication, PublicationFailure> {
        #[cfg(target_os = "windows")]
        {
            let displays = moq_video::capture::displays().await.map_err(|error| {
                tracing::warn!(%error, "could not enumerate Windows displays");
                PublicationFailure::CaptureUnavailable
            })?;
            let display = displays
                .into_iter()
                .next()
                .ok_or(PublicationFailure::CaptureUnavailable)?;
            let info = PublicationInfo {
                width: display.width,
                height: display.height,
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
                source: display.source(),
                plan,
            })
        }

        #[cfg(not(target_os = "windows"))]
        {
            let _ = self;
            let _ = policy;
            Err(PublicationFailure::CaptureUnavailable)
        }
    }
}

#[cfg(target_os = "windows")]
impl Drop for Publication {
    fn drop(&mut self) {
        self.broadcast.finish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
                Err(PublicationFailure::NativeQhdDisplayRequired)
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
            classify_publication_failure(VideoEncodingPolicy::NativeQhdHardware, true),
            PublicationFailure::NativeQhdUnavailable
        );
        assert_eq!(
            classify_publication_failure(VideoEncodingPolicy::NativeQhdHardware, false),
            PublicationFailure::Unexpected
        );
        assert_eq!(
            classify_publication_failure(VideoEncodingPolicy::Compatible, true),
            PublicationFailure::Unexpected
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
