//! Remote audio selection, decode, and default-output ownership.

use std::sync::Arc;
use std::time::Instant;

use tokio::{
    sync::{OnceCell, mpsc},
    task::JoinHandle,
};

use crate::app::{RemoteAudioPhase, RemoteAudioSnapshot};

use crate::runtime::playback_audio_config::{
    REMOTE_AUDIO_LIVE_EDGE_BUDGET, remote_audio_decode_config,
};
pub(super) use crate::runtime::playback_audio_continuity::OwnerTeardownReason;
use crate::runtime::playback_audio_continuity::{
    FrameTiming, TeardownControl, TeardownReason, Tracker as ContinuityTracker, pacing_delay,
};
use crate::runtime::playback_sync::{self, AudioLease, MediaClock};

#[derive(Clone, Debug, PartialEq)]
struct Identity {
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

fn supported_audio(config: &hang::catalog::AudioConfig) -> bool {
    config.broadcast.is_none()
        && matches!(
            &config.codec,
            hang::catalog::AudioCodec::Opus | hang::catalog::AudioCodec::Pcm
        )
}

pub(super) struct Update {
    pub(super) generation: u64,
    pub(super) snapshot: RemoteAudioSnapshot,
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
        engine: &OnceCell<moq_audio::playback::Engine>,
    ) -> anyhow::Result<Self> {
        let decode = remote_audio_decode_config();
        let consumer = moq_audio::decode::Consumer::new(broadcast, config, name, decode).await?;
        let engine = engine
            .get_or_try_init(|| moq_audio::playback::Engine::open(Default::default()))
            .await?;
        let sink = engine.sink(moq_audio::playback::Input {
            format: moq_audio::Format::F32,
            sample_rate: consumer.sample_rate(),
            channels: consumer.channels(),
        })?;
        Ok(Self { consumer, sink })
    }

    fn snapshot(&self, phase: RemoteAudioPhase, track: &str, codec: &str) -> RemoteAudioSnapshot {
        RemoteAudioSnapshot {
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
    teardown: Arc<TeardownControl>,
}

impl Task {
    pub(super) fn spawn(
        generation: u64,
        path: &str,
        broadcast: &moq_tokio::moq_net::broadcast::Consumer,
        selection: &Selection,
        updates: &mpsc::Sender<Update>,
        engine: &Arc<OnceCell<moq_audio::playback::Engine>>,
        clock: &Arc<MediaClock>,
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
        let clock = clock.audio(generation);
        let teardown = Arc::new(TeardownControl::default());
        let task_teardown = teardown.clone();
        let handle = tokio::spawn(async move {
            run(broadcast, selection, events, engine, clock, task_teardown).await;
        });
        Self {
            handle: Some(handle),
            generation,
            path: path.to_owned(),
            track,
            teardown,
        }
    }

    pub(super) async fn stop(&mut self, reason: OwnerTeardownReason) {
        if let Some(handle) = self.handle.take() {
            self.teardown.set_owner_reason(reason);
            handle.abort();
            if handle.await.is_err_and(|error| error.is_cancelled())
                && !self.teardown.summary_emitted()
            {
                tracing::info!(
                    broadcast = %self.path,
                    track = ?self.track,
                    audio_generation = self.generation,
                    teardown_reason = TeardownReason::from(reason).as_str(),
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
    clock: AudioLease,
    teardown: Arc<TeardownControl>,
) {
    events
        .send(RemoteAudioSnapshot {
            phase: RemoteAudioPhase::Pending,
            ..RemoteAudioSnapshot::default()
        })
        .await;

    let (name, config) = match selection {
        Selection::NotPublished => {
            tracing::debug!(
                broadcast = %events.path,
                audio_generation = events.generation,
                "remote screen has no audio track"
            );
            events
                .send(RemoteAudioSnapshot {
                    phase: RemoteAudioPhase::NoAudio,
                    ..RemoteAudioSnapshot::default()
                })
                .await;
            return;
        }
        Selection::Unsupported => {
            tracing::warn!(
                broadcast = %events.path,
                audio_generation = events.generation,
                "remote screen has no supported local audio rendition; video continues"
            );
            events
                .send(RemoteAudioSnapshot {
                    phase: RemoteAudioPhase::Failed,
                    last_error: Some(
                        "Remote screen has no supported local audio rendition.".into(),
                    ),
                    ..RemoteAudioSnapshot::default()
                })
                .await;
            return;
        }
        Selection::Playable { name, config, .. } => (name, config),
    };

    let codec = config.codec.to_string();
    let selected_at = Instant::now();
    let mut continuity = ContinuityTracker::new(
        events.generation,
        &events.path,
        &name,
        &codec,
        selected_at,
        teardown,
    );
    events
        .send(RemoteAudioSnapshot {
            phase: RemoteAudioPhase::TrackSelected,
            track: Some(name.clone()),
            codec: Some(codec.clone()),
            sample_rate: Some(config.sample_rate),
            channels: Some(config.channel_count),
            last_error: None,
        })
        .await;

    let mut playback = match Playback::open(&broadcast, &name, &config, &engine).await {
        Ok(playback) => playback,
        Err(error) => {
            tracing::warn!(
                broadcast = %events.path,
                track = %name,
                audio_generation = events.generation,
                error = %error,
                "could not start remote audio; video continues"
            );
            events
                .send(RemoteAudioSnapshot {
                    phase: RemoteAudioPhase::Failed,
                    track: Some(name),
                    codec: Some(codec),
                    sample_rate: Some(config.sample_rate),
                    channels: Some(config.channel_count),
                    last_error: Some(
                        "Remote audio could not start on the default output device.".into(),
                    ),
                })
                .await;
            continuity.finish(TeardownReason::StartError);
            return;
        }
    };

    tracing::info!(
        broadcast = %events.path,
        track = %name,
        codec = %codec,
        sample_rate = playback.consumer.sample_rate(),
        channels = playback.consumer.channels(),
        live_edge_budget_ms = REMOTE_AUDIO_LIVE_EDGE_BUDGET.as_millis() as u64,
        audio_generation = events.generation,
        "remote audio decoder and output sink opened with a live-edge budget"
    );

    let mut decoded = false;
    let mut submitted = false;
    let sample_rate = playback.consumer.sample_rate();
    let channels = playback.consumer.channels();
    let stride = channels as usize * size_of::<f32>();
    // A PCM frame may exceed the sink capacity, so pace one aligned second at a time.
    let chunk = (sample_rate as usize * stride).max(stride);
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
                continuity.finish(TeardownReason::Ended);
                return;
            }
            Err(error) => {
                tracing::warn!(
                    broadcast = %events.path,
                    track = %name,
                    audio_generation = events.generation,
                    error = %error,
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
                continuity.finish(TeardownReason::DecodeError);
                return;
            }
        };

        let timing = FrameTiming::new(
            playback_sync::timestamp(frame.timestamp),
            frame.data.len(),
            sample_rate,
            channels,
        );
        let _ = continuity.observe_frame(timing, Instant::now);

        if !decoded {
            decoded = true;
            events
                .send(playback.snapshot(RemoteAudioPhase::PcmDecoded, &name, &codec))
                .await;
        }
        let end = timing.end();
        for part in frame.data.chunks(chunk) {
            let buffered = playback.sink.buffered();
            continuity.observe_buffered(buffered);
            if let Some(delay) = pacing_delay(buffered) {
                continuity.observe_pacing_delay_requested(delay);
                tokio::time::sleep(delay).await;
            }
            if let Err(error) = playback.sink.write(part) {
                let _ = continuity.observe_write(
                    part.len(),
                    sample_rate,
                    channels,
                    false,
                    Instant::now,
                );
                tracing::warn!(
                    broadcast = %events.path,
                    track = %name,
                    audio_generation = events.generation,
                    error = %error,
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
                continuity.finish(TeardownReason::SinkError);
                return;
            }
            // Success only means Sink::write returned Ok; output readiness is not exposed.
            let _ = continuity.observe_write(part.len(), sample_rate, channels, true, Instant::now);
        }
        let buffered = playback.sink.buffered();
        continuity.observe_buffered(buffered);
        let frame_end_now = Instant::now();
        clock.anchor(end, buffered, frame_end_now);
        if !submitted {
            submitted = true;
            events
                .send(playback.snapshot(RemoteAudioPhase::PcmSubmitted, &name, &codec))
                .await;
        }
        continuity.maybe_log_summary(frame_end_now);
    }
}

fn failed_snapshot(
    playback: &Playback,
    track: &str,
    codec: &str,
    error: &str,
) -> RemoteAudioSnapshot {
    let mut snapshot = playback.snapshot(RemoteAudioPhase::Failed, track, codec);
    snapshot.last_error = Some(error.to_owned());
    snapshot
}

struct Events {
    generation: u64,
    path: String,
    sender: mpsc::Sender<Update>,
}

impl Events {
    async fn send(&self, snapshot: RemoteAudioSnapshot) {
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
    use std::sync::Arc;
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
    fn selects_local_opus_and_pcm_renditions() {
        for codec in [
            hang::catalog::AudioCodec::Opus,
            hang::catalog::AudioCodec::Pcm,
        ] {
            let mut audio = hang::catalog::Audio::default();
            audio.renditions.insert(
                "audio".into(),
                hang::catalog::AudioConfig::new(codec, 48_000, 2),
            );

            assert!(matches!(
                Selection::from_catalog(audio, None),
                Selection::Playable { name, .. } if name == "audio"
            ));
        }
    }

    #[test]
    fn rejects_unsupported_renditions() {
        let mut audio = hang::catalog::Audio::default();
        audio.renditions.insert(
            "audio".into(),
            hang::catalog::AudioConfig::new(
                hang::catalog::AudioCodec::Unknown("aac-test".into()),
                48_000,
                2,
            ),
        );

        assert_eq!(Selection::from_catalog(audio, None), Selection::Unsupported);
    }

    #[test]
    fn rejects_supported_codec_in_an_external_broadcast() {
        let mut config =
            hang::catalog::AudioConfig::new(hang::catalog::AudioCodec::Opus, 48_000, 2);
        config.broadcast = Some(moq_tokio::moq_net::PathRelative::new("./audio").into_owned());
        let mut audio = hang::catalog::Audio::default();
        audio.renditions.insert("audio".into(), config);

        assert_eq!(Selection::from_catalog(audio, None), Selection::Unsupported);
    }

    #[test]
    fn audio_estimates_do_not_change_playback_identity() {
        let mut audio = hang::catalog::Audio::default();
        audio.renditions.insert(
            "audio".into(),
            hang::catalog::AudioConfig::new(hang::catalog::AudioCodec::Opus, 48_000, 2),
        );
        let current = Selection::from_catalog(audio.clone(), None);

        let config = audio.renditions.get_mut("audio").expect("audio config");
        config.bitrate = Some(128_000);
        config.jitter = Some(std::time::Duration::from_millis(40));
        let updated = Selection::from_catalog(audio, Some("audio"));

        assert_eq!(current, updated);
    }

    #[test]
    fn track_add_replace_and_withdraw_change_selection_once() {
        let playable = |name: &str, sample_rate| {
            let mut audio = hang::catalog::Audio::default();
            audio.renditions.insert(
                name.to_owned(),
                hang::catalog::AudioConfig::new(hang::catalog::AudioCodec::Opus, sample_rate, 2),
            );
            Selection::from_catalog(audio, None)
        };

        let first = playable("audio-a", 48_000);
        assert_ne!(Selection::NotPublished, first);
        assert_eq!(first, first.clone());
        assert_ne!(first, playable("audio-b", 48_000));
        assert_ne!(first, playable("audio-a", 44_100));
        assert_ne!(first, Selection::NotPublished);
    }

    #[test]
    fn playable_audio_replacement_and_withdraw_have_distinct_teardown_reasons() {
        let playable = |name: &str| {
            let mut audio = hang::catalog::Audio::default();
            audio.renditions.insert(
                name.to_owned(),
                hang::catalog::AudioConfig::new(hang::catalog::AudioCodec::Opus, 48_000, 2),
            );
            Selection::from_catalog(audio, None)
        };

        let current = playable("audio-a");
        assert_eq!(
            transition_teardown_reason(&current, &playable("audio-b")),
            OwnerTeardownReason::Replacement
        );
        assert_eq!(
            transition_teardown_reason(&current, &Selection::NotPublished),
            OwnerTeardownReason::Withdraw
        );
        assert_eq!(
            transition_teardown_reason(&current, &Selection::Unsupported),
            OwnerTeardownReason::Withdraw
        );
    }

    #[tokio::test]
    async fn stop_aborts_and_awaits_task_teardown() {
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
            path: "screen".into(),
            track: Some("audio".into()),
            teardown: Arc::new(TeardownControl::default()),
        };

        task.stop(OwnerTeardownReason::Stop).await;

        assert!(stopped.load(Ordering::Acquire));
        assert!(task.handle.is_none());
        assert!(!task.teardown.summary_emitted());
    }
}
