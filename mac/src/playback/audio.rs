//! Remote Opus selection, decode, and default-output ownership.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{OnceCell, mpsc};
use tokio::task::JoinHandle;

use super::{AudioPhase, AudioSnapshot};

pub(super) const LIVE_EDGE_BUDGET: Duration = Duration::from_millis(100);

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Identity {
    track: String,
    broadcast: Option<moq_tokio::moq_net::PathRelativeOwned>,
    codec: hang::catalog::AudioCodec,
    description: Option<Vec<u8>>,
    container: hang::catalog::Container,
    sample_rate: u32,
    channel_count: u32,
}

#[derive(Clone, Debug)]
pub(super) enum Selection {
    NotPublished,
    Unsupported,
    Playable {
        name: String,
        config: Box<hang::catalog::AudioConfig>,
        identity: Identity,
    },
}

impl PartialEq for Selection {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::NotPublished, Self::NotPublished) | (Self::Unsupported, Self::Unsupported) => {
                true
            }
            (
                Self::Playable {
                    identity: current, ..
                },
                Self::Playable { identity: next, .. },
            ) => current == next,
            _ => false,
        }
    }
}

impl Selection {
    pub(super) fn from_catalog(mut audio: hang::catalog::Audio, preferred: Option<&str>) -> Self {
        if audio.renditions.is_empty() {
            return Self::NotPublished;
        }
        let selected = preferred
            .and_then(|name| {
                audio
                    .renditions
                    .remove(name)
                    .filter(supported_audio)
                    .map(|config| (name.to_owned(), config))
            })
            .or_else(|| {
                audio
                    .renditions
                    .into_iter()
                    .find(|(_, config)| supported_audio(config))
            });
        selected.map_or(Self::Unsupported, |(name, config)| {
            let identity = Identity {
                track: name.clone(),
                broadcast: config.broadcast.clone(),
                codec: config.codec.clone(),
                description: config.description.as_deref().map(<[u8]>::to_vec),
                container: config.container.clone(),
                sample_rate: config.sample_rate,
                channel_count: config.channel_count,
            };
            Self::Playable {
                name,
                config: Box::new(config),
                identity,
            }
        })
    }

    pub(super) fn name(&self) -> Option<&str> {
        match self {
            Self::Playable { name, .. } => Some(name),
            Self::NotPublished | Self::Unsupported => None,
        }
    }
}

fn supported_audio(config: &hang::catalog::AudioConfig) -> bool {
    config.broadcast.is_none() && matches!(&config.codec, hang::catalog::AudioCodec::Opus)
}

fn decode_config() -> moq_audio::decode::Config {
    let mut config = moq_audio::decode::Config::new();
    config.format = moq_audio::Format::F32;
    config.max_age = LIVE_EDGE_BUDGET;
    config
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OwnerTeardownReason {
    Stop,
    Replacement,
    Withdraw,
}

impl OwnerTeardownReason {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Stop => "stop",
            Self::Replacement => "replacement",
            Self::Withdraw => "withdraw",
        }
    }
}

pub(super) fn transition_teardown_reason(
    current: &Selection,
    next: &Selection,
) -> OwnerTeardownReason {
    match (current, next) {
        (Selection::Playable { .. }, Selection::NotPublished | Selection::Unsupported) => {
            OwnerTeardownReason::Withdraw
        }
        _ => OwnerTeardownReason::Replacement,
    }
}

pub(super) struct Update {
    pub(super) generation: u64,
    pub(super) snapshot: AudioSnapshot,
}

struct Playback {
    consumer: moq_audio::decode::Consumer,
    sink: moq_audio::playback::Sink,
}

impl Playback {
    fn snapshot(&self, phase: AudioPhase, track: &str, codec: &str) -> AudioSnapshot {
        AudioSnapshot {
            phase,
            track: Some(track.to_owned()),
            codec: Some(codec.to_owned()),
            sample_rate: Some(self.consumer.sample_rate()),
            channels: Some(self.consumer.channels()),
            last_error: None,
        }
    }
}

pub(super) struct Task {
    handle: Option<JoinHandle<()>>,
    generation: u64,
    path: String,
    track: Option<String>,
}

impl Task {
    pub(super) fn spawn(
        generation: u64,
        path: &str,
        broadcast: &moq_tokio::moq_net::broadcast::Consumer,
        selection: &Selection,
        updates: &mpsc::Sender<Update>,
        engine: &Arc<OnceCell<moq_audio::playback::Engine>>,
    ) -> Self {
        let broadcast = broadcast.clone();
        let selection = selection.clone();
        let track = selection.name().map(str::to_owned);
        let events = Events {
            generation,
            path: path.to_owned(),
            sender: updates.clone(),
        };
        let engine = engine.clone();
        let handle = tokio::spawn(async move {
            run(broadcast, selection, events, engine).await;
        });
        Self {
            handle: Some(handle),
            generation,
            path: path.to_owned(),
            track,
        }
    }

    pub(super) async fn stop(&mut self, reason: OwnerTeardownReason) {
        if let Some(handle) = self.handle.take() {
            let running = !handle.is_finished();
            handle.abort();
            let _ = handle.await;
            if running {
                tracing::info!(
                    broadcast = %self.path,
                    track = ?self.track,
                    audio_generation = self.generation,
                    teardown_reason = reason.as_str(),
                    "remote audio generation stopped by playback owner"
                );
            }
        }
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        if let Some(handle) = &self.handle {
            handle.abort();
        }
    }
}

async fn run(
    broadcast: moq_tokio::moq_net::broadcast::Consumer,
    selection: Selection,
    events: Events,
    engine: Arc<OnceCell<moq_audio::playback::Engine>>,
) {
    events.send(AudioSnapshot::pending()).await;

    let (name, config) = match selection {
        Selection::NotPublished => {
            tracing::debug!(
                broadcast = %events.path,
                audio_generation = events.generation,
                "remote screen has no audio track"
            );
            events.send(AudioSnapshot::no_audio()).await;
            return;
        }
        Selection::Unsupported => {
            tracing::warn!(
                broadcast = %events.path,
                audio_generation = events.generation,
                "remote screen has no supported local Opus rendition; video continues"
            );
            events
                .send(AudioSnapshot::failed(
                    "Remote screen has no supported Opus audio rendition.",
                ))
                .await;
            return;
        }
        Selection::Playable { name, config, .. } => (name, config),
    };

    let codec = config.codec.to_string();
    events
        .send(AudioSnapshot {
            phase: AudioPhase::TrackSelected,
            track: Some(name.clone()),
            codec: Some(codec.clone()),
            sample_rate: Some(config.sample_rate),
            channels: Some(config.channel_count),
            last_error: None,
        })
        .await;

    let consumer =
        match moq_audio::decode::Consumer::new(&broadcast, &config, &name, decode_config()).await {
            Ok(consumer) => consumer,
            Err(error) => {
                tracing::warn!(
                    broadcast = %events.path,
                    track = %name,
                    audio_generation = events.generation,
                    error = %error,
                    "could not open remote Opus decoder; video continues"
                );
                events
                    .send(AudioSnapshot {
                        phase: AudioPhase::Failed,
                        track: Some(name),
                        codec: Some(codec),
                        sample_rate: Some(config.sample_rate),
                        channels: Some(config.channel_count),
                        last_error: Some("Remote Opus audio could not be decoded.".into()),
                    })
                    .await;
                return;
            }
        };
    let output = match engine
        .get_or_try_init(|| moq_audio::playback::Engine::open(Default::default()))
        .await
    {
        Ok(output) => output,
        Err(error) => {
            tracing::warn!(
                broadcast = %events.path,
                track = %name,
                audio_generation = events.generation,
                error = %error,
                "could not open the default audio output; video continues"
            );
            events
                .send(AudioSnapshot {
                    phase: AudioPhase::Failed,
                    track: Some(name),
                    codec: Some(codec),
                    sample_rate: Some(consumer.sample_rate()),
                    channels: Some(consumer.channels()),
                    last_error: Some(
                        "Remote audio could not start on the default output device.".into(),
                    ),
                })
                .await;
            return;
        }
    };
    let sink = match output.sink(moq_audio::playback::Input {
        format: moq_audio::Format::F32,
        sample_rate: consumer.sample_rate(),
        channels: consumer.channels(),
    }) {
        Ok(sink) => sink,
        Err(error) => {
            tracing::warn!(
                broadcast = %events.path,
                track = %name,
                audio_generation = events.generation,
                error = %error,
                "could not configure the default audio output; video continues"
            );
            events
                .send(AudioSnapshot {
                    phase: AudioPhase::Failed,
                    track: Some(name),
                    codec: Some(codec),
                    sample_rate: Some(consumer.sample_rate()),
                    channels: Some(consumer.channels()),
                    last_error: Some(
                        "Remote audio could not use the default output device.".into(),
                    ),
                })
                .await;
            return;
        }
    };
    let mut playback = Playback { consumer, sink };

    tracing::info!(
        broadcast = %events.path,
        track = %name,
        codec = %codec,
        sample_rate = playback.consumer.sample_rate(),
        channels = playback.consumer.channels(),
        live_edge_budget_ms = LIVE_EDGE_BUDGET.as_millis() as u64,
        audio_generation = events.generation,
        "remote audio decoder and output sink opened with a live-edge freshness budget"
    );

    let mut decoded = false;
    let mut submitted = false;
    loop {
        let frame = match playback.consumer.read().await {
            Ok(Some(frame)) => frame,
            Ok(None) => {
                events
                    .send(failed_snapshot(
                        &playback,
                        &name,
                        &codec,
                        "Remote audio track ended.",
                    ))
                    .await;
                tracing::info!(
                    broadcast = %events.path,
                    track = %name,
                    audio_generation = events.generation,
                    teardown_reason = "ended",
                    "remote audio generation ended; video continues"
                );
                return;
            }
            Err(error) => {
                tracing::warn!(
                    broadcast = %events.path,
                    track = %name,
                    audio_generation = events.generation,
                    error = %error,
                    teardown_reason = "decode_error",
                    "remote audio decode failed; video continues"
                );
                events
                    .send(failed_snapshot(
                        &playback,
                        &name,
                        &codec,
                        "Remote audio decode failed; video is continuing.",
                    ))
                    .await;
                return;
            }
        };

        if !decoded {
            decoded = true;
            tracing::info!(
                broadcast = %events.path,
                track = %name,
                audio_generation = events.generation,
                pcm_bytes = frame.data.len(),
                timestamp_us = frame.timestamp.as_micros() as u64,
                "first remote PCM frame decoded"
            );
            events
                .send(playback.snapshot(AudioPhase::PcmDecoded, &name, &codec))
                .await;
        }
        if let Err(error) = playback.sink.write(&frame.data) {
            tracing::warn!(
                broadcast = %events.path,
                track = %name,
                audio_generation = events.generation,
                error = %error,
                teardown_reason = "sink_error",
                "remote PCM submission failed; video continues"
            );
            events
                .send(failed_snapshot(
                    &playback,
                    &name,
                    &codec,
                    "Remote PCM submission failed; video is continuing.",
                ))
                .await;
            return;
        }
        if !submitted {
            submitted = true;
            events
                .send(playback.snapshot(AudioPhase::PcmSubmitted, &name, &codec))
                .await;
        }
    }
}

fn failed_snapshot(playback: &Playback, track: &str, codec: &str, error: &str) -> AudioSnapshot {
    let mut snapshot = playback.snapshot(AudioPhase::Failed, track, codec);
    snapshot.last_error = Some(error.to_owned());
    snapshot
}

struct Events {
    generation: u64,
    path: String,
    sender: mpsc::Sender<Update>,
}

impl Events {
    async fn send(&self, snapshot: AudioSnapshot) {
        let _ = self
            .sender
            .send(Update {
                generation: self.generation,
                snapshot,
            })
            .await;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    #[test]
    fn empty_catalog_reports_no_audio() {
        assert_eq!(
            Selection::from_catalog(hang::catalog::Audio::default(), None),
            Selection::NotPublished
        );
    }

    #[test]
    fn selects_only_local_opus_renditions() {
        let mut audio = hang::catalog::Audio::default();
        audio.renditions.insert(
            "audio".into(),
            hang::catalog::AudioConfig::new(hang::catalog::AudioCodec::Opus, 48_000, 2),
        );
        assert!(matches!(
            Selection::from_catalog(audio, None),
            Selection::Playable { name, .. } if name == "audio"
        ));

        let mut pcm = hang::catalog::Audio::default();
        pcm.renditions.insert(
            "audio".into(),
            hang::catalog::AudioConfig::new(hang::catalog::AudioCodec::Pcm, 48_000, 2),
        );
        assert_eq!(Selection::from_catalog(pcm, None), Selection::Unsupported);
    }

    #[test]
    fn decode_policy_uses_f32_and_live_edge_freshness_budget() {
        let config = decode_config();

        assert_eq!(config.format, moq_audio::Format::F32);
        assert_eq!(config.max_age, Duration::from_millis(100));
    }

    #[test]
    fn playable_audio_withdrawal_has_a_distinct_teardown_reason() {
        let mut audio = hang::catalog::Audio::default();
        audio.renditions.insert(
            "audio".into(),
            hang::catalog::AudioConfig::new(hang::catalog::AudioCodec::Opus, 48_000, 2),
        );
        let current = Selection::from_catalog(audio, None);

        assert_eq!(
            transition_teardown_reason(&current, &Selection::NotPublished),
            OwnerTeardownReason::Withdraw
        );
    }

    #[tokio::test]
    async fn stop_aborts_and_awaits_audio_task() {
        struct Teardown(Arc<AtomicBool>);
        impl Drop for Teardown {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }

        let stopped = Arc::new(AtomicBool::new(false));
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let task_stopped = stopped.clone();
        let handle = tokio::spawn(async move {
            let _teardown = Teardown(task_stopped);
            let _ = started_tx.send(());
            std::future::pending::<()>().await;
        });
        started_rx.await.expect("test task started");
        let mut task = Task {
            handle: Some(handle),
            generation: 7,
            path: "moqcast.screen/peer".into(),
            track: Some("audio".into()),
        };

        task.stop(OwnerTeardownReason::Stop).await;

        assert!(stopped.load(Ordering::Acquire));
        assert!(task.handle.is_none());
    }
}
