//! One Linux display capture published as the MoQCast screen broadcast.

use moq_native::moq_net;

#[cfg(target_os = "linux")]
const SCREEN_BROADCAST: &str = "moqcast.screen";

/// A prepared screen publication whose future owns capture and encoding.
pub(crate) struct Publication {
    #[cfg(target_os = "linux")]
    broadcast: moq_net::broadcast::Producer,
    #[cfg(target_os = "linux")]
    catalog: moq_mux::catalog::Producer,
    #[cfg(target_os = "linux")]
    bandwidth: Option<moq_net::bandwidth::Consumer>,
}

impl Publication {
    /// Create the announced broadcast before opening the system picker.
    pub(crate) fn prepare(
        origin: &moq_net::origin::Producer,
        bandwidth: Option<moq_net::bandwidth::Consumer>,
    ) -> anyhow::Result<Self> {
        #[cfg(target_os = "linux")]
        {
            let mut broadcast = origin.create_broadcast(
                SCREEN_BROADCAST,
                moq_net::broadcast::Route::new().with_announce(true),
            )?;
            let catalog = moq_mux::catalog::Producer::new(&mut broadcast)?;
            Ok(Self {
                broadcast,
                catalog,
                bandwidth,
            })
        }

        #[cfg(not(target_os = "linux"))]
        {
            let _ = (origin, bandwidth);
            anyhow::bail!("screen sharing is available only on Linux")
        }
    }

    /// Open the portal picker, capture the selected display, and publish H.264.
    pub(crate) async fn run(self) -> anyhow::Result<()> {
        #[cfg(target_os = "linux")]
        {
            let mut capture = moq_video::capture::Config::default();
            capture.source = moq_video::capture::Source::Display(None);
            capture.framerate = Some(30);

            let mut encode = moq_video::encode::Options::default();
            encode.codec = moq_video::encode::Codec::H264;
            encode.h264_profile = moq_video::encode::H264Profile::Baseline;
            encode.kind = moq_video::encode::Kind::Auto;
            encode.max_size = Some(moq_video::Size::new(1920, 1080));
            encode.bandwidth = self.bandwidth.clone();
            let result = moq_video::encode::publish_capture(
                self.broadcast.clone(),
                self.catalog.clone(),
                capture,
                encode,
                moq_mux::Clock::new(),
            )
            .await;
            result.map_err(Into::into)
        }

        #[cfg(not(target_os = "linux"))]
        unreachable!("non-Linux publication cannot be prepared")
    }
}

#[cfg(target_os = "linux")]
impl Drop for Publication {
    fn drop(&mut self) {
        self.broadcast.finish();
    }
}
