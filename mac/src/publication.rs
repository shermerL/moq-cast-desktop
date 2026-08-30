//! ScreenCaptureKit source selection and H.264 screen publication ownership.

use std::future::{Future, pending};
use std::pin::Pin;

use moq_tokio::moq_net;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Selection {
    Display { display_id: u32, label: String },
    Window { window_id: u32, label: String },
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

pub(crate) enum Event {
    Announced {
        generation: u64,
        path: String,
    },
    Ended {
        generation: u64,
        result: Result<(), Failure>,
    },
}

type Operation = Pin<Box<dyn Future<Output = Result<Publication, Failure>>>>;
type Running = Pin<Box<dyn Future<Output = Result<(), Failure>>>>;

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
    ) {
        self.stop();
        self.stage = Stage::Preparing {
            generation,
            operation: Box::pin(prepare(origin, local_peer_id, selection)),
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
                            self.stage = Stage::Running {
                                generation,
                                operation: Box::pin(publication.run()),
                            };
                            return Event::Announced { generation, path };
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
                } => {
                    let generation = *generation;
                    let result = operation.as_mut().await;
                    self.stage = Stage::Idle;
                    return Event::Ended { generation, result };
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
}

impl Publication {
    async fn run(self) -> Result<(), Failure> {
        let mut capture = moq_video::capture::Config::default();
        capture.source = self.source.clone();
        capture.framerate = Some(30);
        capture.cursor = true;

        let mut encode = moq_video::encode::Options::default();
        encode.codec = moq_video::encode::Codec::H264;
        encode.kind = moq_video::encode::Kind::Auto;

        tracing::info!(codec = "H.264", "screen publication requested");
        moq_video::encode::publish_capture(
            self.broadcast.clone(),
            self.catalog.clone(),
            capture,
            encode,
            moq_mux::Clock::new(),
        )
        .await
        .map_err(|error| {
            tracing::warn!(%error, "screen publication ended");
            Failure::pipeline("Screen sharing stopped because capture or encoding failed.")
        })
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
) -> Result<Publication, Failure> {
    let source_kind = selection.kind();
    let source = resolve(selection).await?;
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
    })
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
}
