//! One Linux capture source published as the MoQCast screen broadcast.

use super::source::CaptureSource;

use moq_tokio::moq_net;
use tokio::sync::watch;

#[cfg(target_os = "linux")]
use super::audio;
#[cfg(target_os = "linux")]
use crate::screen_path;

#[derive(Clone, Debug, thiserror::Error)]
pub(crate) enum Failure {
    #[error("{0}")]
    Source(String),
    #[error("{0}")]
    Other(String),
}

impl From<anyhow::Error> for Failure {
    fn from(error: anyhow::Error) -> Self {
        let unavailable = error.is::<super::source::Unavailable>();
        #[cfg(target_os = "linux")]
        let unavailable = unavailable
            || matches!(
                error.downcast_ref::<moq_video::Error>(),
                Some(moq_video::Error::SourceUnavailable(_))
            );
        if unavailable {
            Self::Source(error.to_string())
        } else {
            Self::Other(error.to_string())
        }
    }
}

pub(crate) struct Options {
    pub(crate) system_audio: bool,
    pub(crate) source: CaptureSource,
}

/// A prepared screen publication whose future owns capture and encoding.
pub(crate) struct Publication {
    #[cfg(target_os = "linux")]
    broadcast: moq_net::broadcast::Producer,
    #[cfg(target_os = "linux")]
    catalog: moq_mux::catalog::Producer,
    #[cfg(target_os = "linux")]
    clock: moq_mux::Clock,
    #[cfg(target_os = "linux")]
    system_audio: bool,
    #[cfg(target_os = "linux")]
    source: CaptureSource,
}

impl Publication {
    /// Create the announced broadcast before opening the selected source.
    pub(crate) fn prepare(
        origin: &moq_net::origin::Producer,
        local_peer_id: &str,
        options: Options,
    ) -> anyhow::Result<Self> {
        #[cfg(target_os = "linux")]
        {
            let path = screen_path::for_peer(local_peer_id);
            let mut broadcast = origin.create_broadcast(&path)?;
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
                system_audio: options.system_audio,
                source: options.source,
            })
        }

        #[cfg(not(target_os = "linux"))]
        {
            let _ = (origin, local_peer_id, options.system_audio, options.source);
            anyhow::bail!("screen sharing is available only on Linux")
        }
    }

    /// Validate the selected source, open capture, and publish H.264.
    pub(crate) async fn run(self, mut cancelled: watch::Receiver<bool>) -> anyhow::Result<()> {
        #[cfg(target_os = "linux")]
        {
            if *cancelled.borrow() {
                return Ok(());
            }
            let source = tokio::select! {
                biased;
                _ = cancelled.changed() => return Ok(()),
                source = super::source::prepare(self.source.clone()) => source?,
            };
            let mut capture = moq_video::capture::Config::default();
            capture.source = source;
            capture.framerate = Some(moq_video::Rate::new(30, 1).expect("valid frame rate"));
            let cleanup = moq_video::capture::cleanup::Owner::default();
            capture.cleanup = Some(cleanup.handle());

            let mut encode = moq_video::encode::Options::default();
            encode.codec = moq_video::encode::Codec::H264;
            encode.kind = moq_video::encode::Kind::Auto;
            encode.max_size = Some(moq_video::Size::new(1920, 1080));
            let clock = self.clock;
            let result = {
                let media = async {
                    let mut options = moq_video::encode::Capture::default();
                    options.capture = capture;
                    options.encode = encode;
                    let video = moq_video::encode::publish_capture(
                        self.broadcast.clone(),
                        self.catalog.clone(),
                        options,
                    );
                    if self.system_audio {
                        let audio =
                            audio::publish(self.broadcast.clone(), self.catalog.clone(), clock);
                        tokio::try_join!(
                            async { video.await.map_err(anyhow::Error::from) },
                            audio
                        )?;
                        Ok(())
                    } else {
                        video.await.map_err(anyhow::Error::from)
                    }
                };
                let stopped = *cancelled.borrow();
                if stopped {
                    Ok(())
                } else {
                    tokio::select! {
                        biased;
                        _ = cancelled.changed() => Ok(()),
                        result = media => result,
                    }
                }
            };
            // Drop the media future before awaiting the parent-owned portal close.
            let closed = cleanup.finish().await;
            match (result, closed) {
                (Err(error), Err(close)) => Err(anyhow::anyhow!("{error}; cleanup: {close}")),
                (Err(error), Ok(())) => Err(error),
                (Ok(()), Err(close)) => Err(anyhow::anyhow!(close)),
                (Ok(()), Ok(())) => Ok(()),
            }
        }

        #[cfg(not(target_os = "linux"))]
        {
            let _ = &mut cancelled;
            unreachable!("non-Linux publication cannot be prepared")
        }
    }
}

#[cfg(target_os = "linux")]
impl Drop for Publication {
    fn drop(&mut self) {
        self.broadcast.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_rejection_remains_distinct_from_other_publication_errors() {
        let source = anyhow::Error::new(super::super::source::Unavailable("select again".into()));
        assert!(
            matches!(Failure::from(source), Failure::Source(message) if message == "select again")
        );
        assert!(matches!(
            Failure::from(anyhow::anyhow!("encoder failed")),
            Failure::Other(_)
        ));
    }
}
