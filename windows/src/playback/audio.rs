//! Remote audio selection, decode, and system-output ownership.

use std::time::Duration;

use tokio::{sync::mpsc, task::JoinHandle};

use super::{AudioStats, ViewAudioPhase, ViewAudioSnapshot, ViewEvent, pcm_duration_us};

const REPORT_INTERVAL: Duration = Duration::from_secs(1);

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

    async fn read(&mut self) -> anyhow::Result<Option<moq_audio::Frame>> {
        self.consumer.read().await.map_err(Into::into)
    }

    fn snapshot(&self, phase: ViewAudioPhase, codec: &str) -> ViewAudioSnapshot {
        ViewAudioSnapshot {
            phase,
            codec: Some(codec.to_owned()),
            sample_rate: Some(self.consumer.sample_rate()),
            channels: Some(self.consumer.channels()),
            ..ViewAudioSnapshot::default()
        }
    }
}

pub(super) struct Task {
    handle: Option<JoinHandle<()>>,
    view_generation: u64,
    audio_generation: u64,
    path: String,
}

impl Task {
    pub(super) fn spawn(
        view_generation: u64,
        audio_generation: u64,
        path: &str,
        broadcast: &moq_tokio::moq_net::broadcast::Consumer,
        selection: &Selection,
        events: &mpsc::Sender<ViewEvent>,
    ) -> Self {
        let broadcast = broadcast.clone();
        let selection = selection.clone();
        let events = Events {
            view_generation,
            audio_generation,
            path: path.to_owned(),
            sender: events.clone(),
        };
        Self {
            handle: Some(tokio::spawn(async move {
                run(broadcast, selection, events).await;
            })),
            view_generation,
            audio_generation,
            path: path.to_owned(),
        }
    }

    pub(super) async fn stop(mut self, reason: &'static str) {
        if let Some(handle) = self.handle.take() {
            handle.abort();
            let _ = handle.await;
        }
        tracing::info!(
            view_generation = self.view_generation,
            audio_generation = self.audio_generation,
            broadcast = ?self.path,
            reason,
            "remote audio task teardown completed"
        );
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.abort();
            tracing::warn!(
                view_generation = self.view_generation,
                audio_generation = self.audio_generation,
                broadcast = ?self.path,
                "remote audio task dropped without awaited teardown"
            );
        }
    }
}

async fn run(
    broadcast: moq_tokio::moq_net::broadcast::Consumer,
    selection: Selection,
    events: Events,
) {
    let (name, config) = match selection {
        Selection::NotPublished => {
            tracing::debug!(
                broadcast = ?events.path,
                view_generation = events.view_generation,
                audio_generation = events.audio_generation,
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
                view_generation = events.view_generation,
                audio_generation = events.audio_generation,
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

    let codec = config.codec.to_string();
    events
        .send(ViewAudioSnapshot {
            phase: ViewAudioPhase::TrackSelected,
            codec: Some(codec.clone()),
            sample_rate: Some(config.sample_rate),
            channels: Some(config.channel_count),
            ..ViewAudioSnapshot::default()
        })
        .await;
    tracing::info!(
        broadcast = ?events.path,
        view_generation = events.view_generation,
        audio_generation = events.audio_generation,
        track = ?name,
        codec = %codec,
        catalog_sample_rate = config.sample_rate,
        catalog_channels = config.channel_count,
        container = ?config.container,
        output_device = "system-default",
        "remote audio track selected"
    );

    let mut playback = match Playback::open(&broadcast, &name, &config).await {
        Ok(playback) => playback,
        Err(error) => {
            tracing::warn!(
                broadcast = ?events.path,
                view_generation = events.view_generation,
                audio_generation = events.audio_generation,
                track = ?name,
                error = %error,
                "could not start remote audio; video continues"
            );
            events
                .send(ViewAudioSnapshot {
                    phase: ViewAudioPhase::Failed,
                    codec: Some(config.codec.to_string()),
                    sample_rate: Some(config.sample_rate),
                    channels: Some(config.channel_count),
                    last_error: Some(
                        "Remote audio could not start on the default output device.".to_owned(),
                    ),
                    ..ViewAudioSnapshot::default()
                })
                .await;
            return;
        }
    };
    tracing::info!(
        broadcast = ?events.path,
        view_generation = events.view_generation,
        audio_generation = events.audio_generation,
        track = ?name,
        codec = %codec,
        decoded_sample_rate = playback.consumer.sample_rate(),
        decoded_channels = playback.consumer.channels(),
        output_device = "system-default",
        "remote audio pipeline opened; callback readiness is not exposed"
    );

    let mut stats = AudioStats::default();
    let mut reports = tokio::time::interval(REPORT_INTERVAL);
    reports.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    reports.tick().await;

    loop {
        tokio::select! {
            decoded = playback.read() => {
                let frame = match decoded {
                    Ok(Some(frame)) => frame,
                    Ok(None) => {
                        tracing::debug!(
                            broadcast = ?events.path,
                            view_generation = events.view_generation,
                            audio_generation = events.audio_generation,
                            track = ?name,
                            "remote audio track ended"
                        );
                        let mut audio = playback.snapshot(ViewAudioPhase::Failed, &codec);
                        audio.last_error = Some("Remote audio track ended.".to_owned());
                        events.send(audio).await;
                        return;
                    }
                    Err(error) => {
                        tracing::warn!(
                            broadcast = ?events.path,
                            view_generation = events.view_generation,
                            audio_generation = events.audio_generation,
                            track = ?name,
                            error = %error,
                            "remote audio decode failed; video continues"
                        );
                        let mut audio = playback.snapshot(ViewAudioPhase::Failed, &codec);
                        audio.last_error =
                            Some("Remote audio playback failed; video is continuing.".to_owned());
                        events.send(audio).await;
                        return;
                    }
                };

                let timestamp_us = frame.timestamp.as_micros();
                let bytes = frame.data.len();
                let duration_us = pcm_duration_us(
                    bytes,
                    playback.consumer.channels(),
                    playback.consumer.sample_rate(),
                );
                if stats.decoded(timestamp_us, bytes, duration_us) {
                    tracing::info!(
                        broadcast = ?events.path,
                        view_generation = events.view_generation,
                        audio_generation = events.audio_generation,
                        track = ?name,
                        frame_pts_us = %timestamp_us,
                        pcm_bytes = bytes,
                        pcm_duration_us = %duration_us,
                        "decoded first remote PCM frame"
                    );
                    events
                        .send(playback.snapshot(ViewAudioPhase::Decoded, &codec))
                        .await;
                }

                if let Err(error) = playback.sink.write(&frame.data) {
                    stats.write_failed();
                    tracing::warn!(
                        broadcast = ?events.path,
                        view_generation = events.view_generation,
                        audio_generation = events.audio_generation,
                        track = ?name,
                        frame_pts_us = %timestamp_us,
                        error = %error,
                        "remote PCM sink write failed; video continues"
                    );
                    log_interval(&events, &name, &codec, &playback, &mut stats);
                    let mut audio = playback.snapshot(ViewAudioPhase::Failed, &codec);
                    audio.last_error =
                        Some("Remote audio playback failed; video is continuing.".to_owned());
                    events.send(audio).await;
                    return;
                }
                if stats.wrote() {
                    tracing::info!(
                        broadcast = ?events.path,
                        view_generation = events.view_generation,
                        audio_generation = events.audio_generation,
                        track = ?name,
                        frame_pts_us = %timestamp_us,
                        buffered_us = %playback.sink.buffered().as_micros(),
                        "first remote PCM sink write returned successfully; callback readiness is not exposed"
                    );
                    events
                        .send(playback.snapshot(ViewAudioPhase::Writing, &codec))
                        .await;
                }
            }
            _ = reports.tick() => {
                log_interval(&events, &name, &codec, &playback, &mut stats);
            }
        }
    }
}

fn log_interval(
    events: &Events,
    track: &str,
    codec: &str,
    playback: &Playback,
    stats: &mut AudioStats,
) {
    let report = stats.take_report();
    tracing::info!(
        broadcast = ?events.path,
        view_generation = events.view_generation,
        audio_generation = events.audio_generation,
        track,
        codec,
        decoded_frames = report.frames,
        decoded_bytes = report.bytes,
        sink_writes = report.writes,
        sink_write_errors = report.write_errors,
        pts_gaps = report.pts_gaps,
        pts_gap_us = %report.pts_gap_us,
        max_pts_gap_us = %report.max_pts_gap_us,
        pts_regressions = report.pts_regressions,
        buffered_us = %playback.sink.buffered().as_micros(),
        peak = playback.sink.peak(),
        "remote audio playback interval"
    );
}

struct Events {
    view_generation: u64,
    audio_generation: u64,
    path: String,
    sender: mpsc::Sender<ViewEvent>,
}

impl Events {
    async fn send(&self, audio: ViewAudioSnapshot) {
        let _ = self
            .sender
            .send(ViewEvent::AudioChanged {
                generation: self.view_generation,
                path: self.path.clone(),
                audio_generation: self.audio_generation,
                audio,
            })
            .await;
    }
}
