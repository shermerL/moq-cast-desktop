//! ScreenCaptureKit source selection and H.264 screen publication ownership.

use std::future::{Future, pending};
use std::pin::Pin;

use moq_tokio::moq_net;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Selection {
    Display {
        display_id: u32,
        primary: bool,
        label: String,
    },
    Window {
        window_id: u32,
        label: String,
    },
}

impl Selection {
    pub(crate) fn label(&self) -> &str {
        match self {
            Self::Display { label, .. } | Self::Window { label, .. } => label,
        }
    }

    pub(crate) fn kind(&self) -> SourceKind {
        match self {
            Self::Display { .. } => SourceKind::Display,
            Self::Window { .. } => SourceKind::Window,
        }
    }

    pub(crate) fn supports_system_audio(&self) -> bool {
        matches!(self, Self::Display { primary: true, .. })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SourceKind {
    Display,
    Window,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Failure {
    message: String,
    source_unavailable: bool,
}

impl Failure {
    pub(crate) fn pipeline(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            source_unavailable: false,
        }
    }

    pub(crate) fn source_unavailable() -> Self {
        Self {
            message: "The selected screen is no longer available. Choose it again.".to_owned(),
            source_unavailable: true,
        }
    }

    pub(crate) fn into_parts(self) -> (String, bool) {
        (self.message, self.source_unavailable)
    }

    pub(crate) fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AudioStatus {
    Off,
    Included,
    Unavailable(String),
}

pub(crate) enum Event {
    Announced {
        generation: u64,
        path: String,
        audio: AudioStatus,
    },
    AudioFailed {
        generation: u64,
        message: String,
    },
    Ended {
        generation: u64,
        result: Result<(), Failure>,
    },
}

type Operation = Pin<Box<dyn Future<Output = Result<Publication, Failure>>>>;
type Running = Pin<Box<dyn Future<Output = Result<(), Failure>>>>;
type AudioRunning = Pin<Box<dyn Future<Output = Result<(), String>>>>;

#[derive(Default)]
enum Stage {
    #[default]
    Idle,
    Preparing {
        generation: u64,
        operation: Operation,
    },
    Running {
        generation: u64,
        operation: Running,
        audio_events: Option<tokio::sync::mpsc::UnboundedReceiver<String>>,
    },
}

#[derive(Default)]
pub(crate) struct Owner {
    stage: Stage,
}

impl Owner {
    pub(crate) fn start(
        &mut self,
        generation: u64,
        origin: moq_net::origin::Producer,
        local_peer_id: String,
        selection: Selection,
        system_audio: bool,
    ) {
        self.stop();
        self.stage = Stage::Preparing {
            generation,
            operation: Box::pin(prepare(origin, local_peer_id, selection, system_audio)),
        };
    }

    pub(crate) fn stop(&mut self) {
        self.stage = Stage::Idle;
    }

    pub(crate) async fn recv(&mut self) -> Event {
        loop {
            match &mut self.stage {
                Stage::Idle => pending().await,
                Stage::Preparing {
                    generation,
                    operation,
                } => {
                    let generation = *generation;
                    match operation.as_mut().await {
                        Ok(publication) => {
                            let path = publication.path.clone();
                            let audio = publication.audio_status();
                            let (audio_events_tx, audio_events) =
                                tokio::sync::mpsc::unbounded_channel();
                            self.stage = Stage::Running {
                                generation,
                                operation: Box::pin(publication.run(audio_events_tx)),
                                audio_events: Some(audio_events),
                            };
                            return Event::Announced {
                                generation,
                                path,
                                audio,
                            };
                        }
                        Err(error) => {
                            self.stage = Stage::Idle;
                            return Event::Ended {
                                generation,
                                result: Err(error),
                            };
                        }
                    }
                }
                Stage::Running {
                    generation,
                    operation,
                    audio_events,
                } => {
                    let generation = *generation;
                    if let Some(events) = audio_events.as_mut() {
                        tokio::select! {
                            result = operation.as_mut() => {
                                self.stage = Stage::Idle;
                                return Event::Ended { generation, result };
                            }
                            message = events.recv() => match message {
                                Some(message) => return Event::AudioFailed { generation, message },
                                None => *audio_events = None,
                            }
                        }
                    } else {
                        let result = operation.as_mut().await;
                        self.stage = Stage::Idle;
                        return Event::Ended { generation, result };
                    }
                }
            }
        }
    }
}

struct Publication {
    path: String,
    broadcast: moq_net::broadcast::Producer,
    catalog: moq_mux::catalog::Producer,
    source: moq_video::capture::Source,
    audio: AudioPlan,
}

impl Publication {
    fn audio_status(&self) -> AudioStatus {
        match &self.audio {
            AudioPlan::Off => AudioStatus::Off,
            AudioPlan::Capture => AudioStatus::Included,
            AudioPlan::Unavailable(message) => AudioStatus::Unavailable(message.clone()),
        }
    }

    async fn run(
        self,
        audio_events: tokio::sync::mpsc::UnboundedSender<String>,
    ) -> Result<(), Failure> {
        let mut capture = moq_video::capture::Config::default();
        capture.source = self.source.clone();
        capture.framerate = Some(30);
        capture.cursor = true;

        let mut encode = moq_video::encode::Options::default();
        encode.codec = moq_video::encode::Codec::H264;
        encode.kind = moq_video::encode::Kind::Auto;

        let clock = moq_mux::Clock::new();
        let video_broadcast = self.broadcast.clone();
        let video_catalog = self.catalog.clone();
        let video: Running = Box::pin(async move {
            tracing::info!(codec = "H.264", "screen publication requested");
            moq_video::encode::publish_capture(
                video_broadcast,
                video_catalog,
                capture,
                encode,
                clock,
            )
            .await
            .map_err(|error| {
                tracing::warn!(%error, "screen publication ended");
                Failure::pipeline("Screen sharing stopped because capture or encoding failed.")
            })
        });

        let audio = match self.audio {
            AudioPlan::Capture => {
                let mut capture = moq_audio::capture::Config::default();
                capture.source = moq_audio::capture::Source::System;
                capture.sample_rate = Some(48_000);
                capture.channels = Some(2);

                let mut encode = moq_audio::encode::Options::default();
                encode.codec = moq_audio::encode::Codec::Opus;
                encode.sample_rate = Some(48_000);
                encode.channels = Some(2);

                let audio_broadcast = self.broadcast.clone();
                let audio_catalog = self.catalog.clone();
                Some(Box::pin(async move {
                    tracing::info!(codec = "Opus", "system audio publication requested");
                    moq_audio::encode::publish_capture(
                        audio_broadcast,
                        audio_catalog,
                        capture,
                        encode,
                        clock,
                    )
                    .await
                    .map_err(|error| {
                        tracing::warn!(%error, "system audio publication ended");
                        "System audio is unavailable. Video sharing continues.".to_owned()
                    })
                }) as AudioRunning)
            }
            AudioPlan::Off | AudioPlan::Unavailable(_) => None,
        };

        run_tracks(video, audio, audio_events).await
    }
}

enum AudioPlan {
    Off,
    Capture,
    Unavailable(String),
}

async fn run_tracks(
    mut video: Running,
    audio: Option<AudioRunning>,
    audio_events: tokio::sync::mpsc::UnboundedSender<String>,
) -> Result<(), Failure> {
    let Some(mut audio) = audio else {
        return video.await;
    };
    tokio::select! {
        result = &mut video => result,
        result = &mut audio => {
            let message = match result {
                Ok(()) => "System audio stopped. Video sharing continues.".to_owned(),
                Err(message) => message,
            };
            let _ = audio_events.send(message);
            video.await
        }
    }
}

impl Drop for Publication {
    fn drop(&mut self) {
        self.broadcast.finish();
    }
}

async fn prepare(
    origin: moq_net::origin::Producer,
    local_peer_id: String,
    selection: Selection,
    system_audio: bool,
) -> Result<Publication, Failure> {
    let source_kind = selection.kind();
    let supports_system_audio = selection.supports_system_audio();
    let source = resolve(selection).await?;
    let audio = audio_plan(system_audio, supports_system_audio, &source);
    let path = canonical_path(&local_peer_id)?;
    let mut broadcast = origin
        .create_broadcast(&path, moq_net::broadcast::Route::new().with_announce(true))
        .map_err(|error| {
            tracing::warn!(%error, "could not create screen broadcast");
            Failure::pipeline("Screen sharing could not create its local broadcast.")
        })?;
    let catalog = moq_mux::catalog::Producer::new(&mut broadcast).map_err(|error| {
        tracing::warn!(%error, "could not create screen catalog");
        Failure::pipeline("Screen sharing could not create its media catalog.")
    })?;
    tracing::info!(?source_kind, "screen source prepared");
    Ok(Publication {
        path,
        broadcast,
        catalog,
        source,
        audio,
    })
}

fn audio_plan(
    requested: bool,
    supported_selection: bool,
    source: &moq_video::capture::Source,
) -> AudioPlan {
    if !requested {
        AudioPlan::Off
    } else if supported_selection
        && matches!(source, moq_video::capture::Source::Display(Some(index)) if index == "0")
    {
        AudioPlan::Capture
    } else {
        AudioPlan::Unavailable(
            "System audio is available only when sharing the main display.".to_owned(),
        )
    }
}

async fn resolve(selection: Selection) -> Result<moq_video::capture::Source, Failure> {
    match selection {
        Selection::Display { display_id, .. } => {
            let displays = moq_video::capture::displays().await.map_err(|error| {
                tracing::warn!(%error, "could not enumerate displays");
                Failure::source_unavailable()
            })?;
            resolve_display(display_id, &displays).ok_or_else(Failure::source_unavailable)
        }
        Selection::Window { window_id, .. } => {
            let windows = moq_video::capture::windows().await.map_err(|error| {
                tracing::warn!(%error, "could not enumerate windows");
                Failure::source_unavailable()
            })?;
            windows
                .into_iter()
                .find(|window| window.id == window_id.to_string())
                .map(|window| window.source())
                .ok_or_else(Failure::source_unavailable)
        }
    }
}

fn resolve_display(
    display_id: u32,
    displays: &[moq_video::capture::Display],
) -> Option<moq_video::capture::Source> {
    let name = format!("Display {display_id}");
    displays
        .iter()
        .find(|display| display.name == name)
        .map(moq_video::capture::Display::source)
}

fn canonical_path(local_peer_id: &str) -> Result<String, Failure> {
    let path = crate::contract::screen_path(local_peer_id);
    (crate::contract::screen_peer_id(&path) == Some(local_peer_id))
        .then_some(path)
        .ok_or_else(|| Failure::pipeline("Screen sharing could not create a canonical local path."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_publication_path_is_canonical() {
        assert_eq!(
            canonical_path("local-peer").expect("canonical path"),
            "moqcast.screen/local-peer"
        );
        assert!(canonical_path("not/a/segment").is_err());
    }

    #[test]
    fn picker_display_id_resolves_to_the_current_capture_index() {
        let displays = vec![
            moq_video::capture::Display {
                id: "0".to_owned(),
                name: "Display 42".to_owned(),
                width: 1728,
                height: 1117,
            },
            moq_video::capture::Display {
                id: "1".to_owned(),
                name: "Display 84".to_owned(),
                width: 1920,
                height: 1080,
            },
        ];

        assert_eq!(
            resolve_display(84, &displays),
            Some(moq_video::capture::Source::Display(Some("1".to_owned())))
        );
        assert_eq!(resolve_display(7, &displays), None);
    }

    #[test]
    fn only_the_primary_display_supports_system_audio() {
        let primary = Selection::Display {
            display_id: 42,
            primary: true,
            label: "Display 42".to_owned(),
        };
        let secondary = Selection::Display {
            display_id: 84,
            primary: false,
            label: "Display 84".to_owned(),
        };
        let window = Selection::Window {
            window_id: 7,
            label: "Window".to_owned(),
        };

        assert!(primary.supports_system_audio());
        assert!(!secondary.supports_system_audio());
        assert!(!window.supports_system_audio());
    }

    #[test]
    fn system_audio_capture_requires_the_first_resolved_display() {
        assert!(matches!(
            audio_plan(
                true,
                true,
                &moq_video::capture::Source::Display(Some("0".to_owned())),
            ),
            AudioPlan::Capture
        ));
        assert!(matches!(
            audio_plan(
                true,
                true,
                &moq_video::capture::Source::Display(Some("1".to_owned())),
            ),
            AudioPlan::Unavailable(_)
        ));
        assert!(matches!(
            audio_plan(
                true,
                false,
                &moq_video::capture::Source::Display(Some("0".to_owned())),
            ),
            AudioPlan::Unavailable(_)
        ));
    }

    #[tokio::test]
    async fn audio_failure_does_not_end_video_publication() {
        let (video_tx, video_rx) = tokio::sync::oneshot::channel();
        let (audio_events_tx, mut audio_events) = tokio::sync::mpsc::unbounded_channel();
        let video: Running =
            Box::pin(async move { video_rx.await.expect("video completion signal") });
        let audio: AudioRunning = Box::pin(async { Err("audio unavailable".to_owned()) });
        let operation = run_tracks(video, Some(audio), audio_events_tx);
        tokio::pin!(operation);

        let message = tokio::select! {
            result = &mut operation => panic!("video ended after audio failure: {result:?}"),
            message = audio_events.recv() => message,
        };
        assert_eq!(message.as_deref(), Some("audio unavailable"));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), &mut operation)
                .await
                .is_err(),
            "audio failure must not end video publication"
        );

        video_tx.send(Ok(())).expect("finish video");
        assert!(operation.await.is_ok());
    }
}
