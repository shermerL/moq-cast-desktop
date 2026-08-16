//! Single-publication screen media lifecycle and Windows capture pipeline.

use moq_native::moq_net;

use crate::screen_path;

pub(crate) const MAX_SCREEN_EDGE: u32 = 1920;

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
            path: None,
            width: None,
            height: None,
            last_error: None,
        }
    }
}

impl MediaSnapshot {
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
        true
    }

    pub(crate) fn ended(&mut self, generation: u64, failed: bool) -> bool {
        if generation != self.generation
            || !matches!(self.phase, MediaPhase::Preparing | MediaPhase::Sharing)
        {
            return false;
        }
        if failed {
            self.phase = MediaPhase::Failed;
            self.last_error = Some("Screen publication ended unexpectedly.");
        } else {
            self.phase = MediaPhase::Idle;
            self.path = None;
            self.width = None;
            self.height = None;
        }
        true
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PublicationInfo {
    pub(crate) width: u32,
    pub(crate) height: u32,
}

impl PublicationInfo {
    #[cfg(any(target_os = "windows", test))]
    fn validate(self) -> anyhow::Result<Self> {
        if self.width == 0 || self.height == 0 {
            anyhow::bail!("the selected display has no usable pixels");
        }
        if self.width.max(self.height) > MAX_SCREEN_EDGE {
            anyhow::bail!(
                "the selected display exceeds the current {MAX_SCREEN_EDGE}-pixel edge limit"
            );
        }
        Ok(self)
    }
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
    info: PublicationInfo,
}

impl ReadyPublication {
    pub(crate) fn info(&self) -> PublicationInfo {
        self.info
    }

    pub(crate) async fn run(self) -> anyhow::Result<()> {
        #[cfg(target_os = "windows")]
        {
            let mut capture = moq_video::capture::Config::default();
            capture.source = self.source;
            capture.framerate = Some(30);

            let mut encode = moq_video::encode::Options::default();
            encode.codec = moq_video::encode::Codec::H264;
            encode.kind = moq_video::encode::Kind::Auto;

            moq_video::encode::publish_capture(
                self.publication.broadcast.clone(),
                self.publication.catalog.clone(),
                capture,
                encode,
                moq_mux::Clock::new(),
            )
            .await
            .map_err(Into::into)
        }

        #[cfg(not(target_os = "windows"))]
        anyhow::bail!("Windows screen capture is unavailable on this host")
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

    pub(crate) async fn configure(self) -> anyhow::Result<ReadyPublication> {
        #[cfg(target_os = "windows")]
        {
            let display = moq_video::capture::displays()
                .await?
                .into_iter()
                .next()
                .ok_or_else(|| anyhow::anyhow!("Windows reported no capturable display"))?;
            let info = PublicationInfo {
                width: display.width,
                height: display.height,
            }
            .validate()?;
            Ok(ReadyPublication {
                publication: self,
                source: display.source(),
                info,
            })
        }

        #[cfg(not(target_os = "windows"))]
        {
            let _ = self;
            anyhow::bail!("Windows screen capture is unavailable on this host")
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
        assert_eq!(media.begin_stop(), Some(generation));
        assert!(media.stopped(generation));
        assert_eq!(media.phase, MediaPhase::Idle);
    }

    #[test]
    fn old_publication_generation_cannot_replace_current_state() {
        let mut media = MediaSnapshot::default();
        let old = media.begin("peer-a").expect("old");
        media.phase = MediaPhase::Failed;
        let current = media.begin("peer-a").expect("current");
        assert!(current > old);
        assert!(!media.ended(old, true));
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
        assert!(!media.ended(generation, true));
        assert_eq!(media.phase, MediaPhase::Idle);
    }

    #[test]
    fn output_size_rejects_oversized_or_empty_displays() {
        assert!(
            PublicationInfo {
                width: 1920,
                height: 1080,
            }
            .validate()
            .is_ok()
        );
        assert!(
            PublicationInfo {
                width: 1080,
                height: 1920,
            }
            .validate()
            .is_ok()
        );
        assert!(
            PublicationInfo {
                width: 2560,
                height: 1440,
            }
            .validate()
            .is_err()
        );
        assert!(
            PublicationInfo {
                width: 0,
                height: 1080,
            }
            .validate()
            .is_err()
        );
    }
}
