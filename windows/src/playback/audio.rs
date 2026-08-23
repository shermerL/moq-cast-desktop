//! Remote audio selection, decode, and system-output ownership.

use tokio::{sync::mpsc, task::JoinHandle};

use super::{ViewAudioPhase, ViewAudioSnapshot, ViewEvent};

#[derive(Clone, PartialEq)]
pub(super) enum Selection {
    NotPublished,
    Unsupported,
    Playable {
        name: String,
        config: Box<hang::catalog::AudioConfig>,
    },
}

impl Selection {
    pub(super) fn from_catalog(audio: hang::catalog::Audio) -> Self {
        if audio.renditions.is_empty() {
            return Self::NotPublished;
        }
        audio
            .renditions
            .into_iter()
            .find(|(_, config)| {
                config.broadcast.is_none()
                    && matches!(
                        &config.codec,
                        hang::catalog::AudioCodec::Opus | hang::catalog::AudioCodec::Pcm
                    )
            })
            .map_or(Self::Unsupported, |(name, config)| Self::Playable {
                name,
                config: Box::new(config),
            })
    }
}

struct Playback {
    consumer: moq_audio::decode::Consumer,
    sink: moq_audio::playback::Sink,
}

impl Playback {
    async fn open(
        broadcast: &moq_tokio::moq_net::broadcast::Consumer,
        name: &str,
        config: &hang::catalog::AudioConfig,
    ) -> anyhow::Result<Self> {
        let mut decode = moq_audio::decode::Config::new();
        decode.format = moq_audio::Format::F32;
        let consumer = moq_audio::decode::Consumer::new(broadcast, config, name, decode).await?;
        let engine = moq_audio::playback::Engine::open(Default::default()).await?;
        let sink = engine.sink(moq_audio::playback::Input {
            format: moq_audio::Format::F32,
            sample_rate: consumer.sample_rate(),
            channels: consumer.channels(),
        })?;
        Ok(Self { consumer, sink })
    }

    async fn pump(&mut self) -> anyhow::Result<bool> {
        let Some(frame) = self.consumer.read().await? else {
            return Ok(false);
        };
        self.sink.write(&frame.data)?;
        Ok(true)
    }

    fn snapshot(&self, codec: &str) -> ViewAudioSnapshot {
        ViewAudioSnapshot {
            phase: ViewAudioPhase::Playing,
            codec: Some(codec.to_owned()),
            sample_rate: Some(self.consumer.sample_rate()),
            channels: Some(self.consumer.channels()),
            last_error: None,
        }
    }
}

pub(super) struct Task(JoinHandle<()>);

impl Task {
    pub(super) fn spawn(
        generation: u64,
        path: &str,
        broadcast: &moq_tokio::moq_net::broadcast::Consumer,
        selection: &Selection,
        events: &mpsc::Sender<ViewEvent>,
    ) -> Self {
        let broadcast = broadcast.clone();
        let selection = selection.clone();
        let events = Events {
            generation,
            path: path.to_owned(),
            sender: events.clone(),
        };
        Self(tokio::spawn(async move {
            run(broadcast, selection, events).await;
        }))
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn run(
    broadcast: moq_tokio::moq_net::broadcast::Consumer,
    selection: Selection,
    events: Events,
) {
    events
        .send(ViewAudioSnapshot {
            phase: ViewAudioPhase::Pending,
            ..ViewAudioSnapshot::default()
        })
        .await;

    let (name, config) = match selection {
        Selection::NotPublished => {
            tracing::debug!(
                broadcast = ?events.path,
                view_generation = events.generation,
                "remote screen has no audio track"
            );
            events
                .send(ViewAudioSnapshot {
                    phase: ViewAudioPhase::NotPublished,
                    ..ViewAudioSnapshot::default()
                })
                .await;
            return;
        }
        Selection::Unsupported => {
            tracing::warn!(
                broadcast = ?events.path,
                view_generation = events.generation,
                "remote screen has no supported audio track; video continues"
            );
            events
                .send(ViewAudioSnapshot {
                    phase: ViewAudioPhase::Failed,
                    last_error: Some("Remote screen has no supported audio rendition.".to_owned()),
                    ..ViewAudioSnapshot::default()
                })
                .await;
            return;
        }
        Selection::Playable { name, config } => (name, config),
    };

    let mut playback = match Playback::open(&broadcast, &name, &config).await {
        Ok(playback) => playback,
        Err(error) => {
            tracing::warn!(
                broadcast = ?events.path,
                track = ?name,
                error = %error,
                "could not start remote audio; video continues"
            );
            events
                .send(ViewAudioSnapshot {
                    phase: ViewAudioPhase::Failed,
                    codec: Some(config.codec.to_string()),
                    last_error: Some(
                        "Remote audio could not start on the default output device.".to_owned(),
                    ),
                    ..ViewAudioSnapshot::default()
                })
                .await;
            return;
        }
    };
    let codec = config.codec.to_string();
    let audio = playback.snapshot(&codec);
    tracing::info!(
        broadcast = ?events.path,
        track = ?name,
        codec = %codec,
        sample_rate = audio.sample_rate.unwrap_or_default(),
        channels = audio.channels.unwrap_or_default(),
        "playing remote audio"
    );
    events.send(audio).await;

    loop {
        match playback.pump().await {
            Ok(true) => {}
            Ok(false) => {
                tracing::debug!(
                    broadcast = ?events.path,
                    track = ?name,
                    "remote audio track ended"
                );
                let mut audio = playback.snapshot(&codec);
                audio.phase = ViewAudioPhase::Failed;
                audio.last_error = Some("Remote audio track ended.".to_owned());
                events.send(audio).await;
                return;
            }
            Err(error) => {
                tracing::warn!(
                    broadcast = ?events.path,
                    track = ?name,
                    error = %error,
                    "remote audio playback failed; video continues"
                );
                let mut audio = playback.snapshot(&codec);
                audio.phase = ViewAudioPhase::Failed;
                audio.last_error =
                    Some("Remote audio playback failed; video is continuing.".to_owned());
                events.send(audio).await;
                return;
            }
        }
    }
}

struct Events {
    generation: u64,
    path: String,
    sender: mpsc::Sender<ViewEvent>,
}

impl Events {
    async fn send(&self, audio: ViewAudioSnapshot) {
        let _ = self
            .sender
            .send(ViewEvent::AudioChanged {
                generation: self.generation,
                path: self.path.clone(),
                audio,
            })
            .await;
    }
}
