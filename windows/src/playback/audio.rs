//! Remote audio selection, decode, and system-output ownership.

use std::time::{Duration, Instant};

use tokio::{
    sync::{mpsc, watch},
    task::JoinHandle,
};

use super::sync::{self, AudioAnchor, AudioClockReader, AudioClockWriter};
use super::{
    AudioStats, ViewAudioPhase, ViewAudioSnapshot, ViewEvent, callback_consumed_nonzero,
    output_diagnostics, pcm_duration_us, pcm_has_nonzero_f32,
};

const REPORT_INTERVAL: Duration = Duration::from_secs(1);

fn remote_audio_decode_config() -> moq_audio::decode::Options {
    let mut options = moq_audio::decode::Options::new();
    options.output.format = moq_audio::Format::F32;
    options.max_age = super::AV_LIVE_EDGE_BUDGET;
    options
}

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

struct VolumeListener {
    task: JoinHandle<()>,
}

impl VolumeListener {
    fn spawn(control: moq_audio::playback::Control, mut volume: watch::Receiver<u8>) -> Self {
        control.set_volume(volume_scalar(*volume.borrow_and_update()));
        let task = tokio::spawn(async move {
            while volume.changed().await.is_ok() {
                control.set_volume(volume_scalar(*volume.borrow_and_update()));
            }
        });
        Self { task }
    }
}

impl Drop for VolumeListener {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn volume_scalar(percent: u8) -> f32 {
    f32::from(percent.min(100)) / 100.0
}

impl Playback {
    async fn open(
        broadcast: &moq_tokio::moq_net::broadcast::Consumer,
        name: &str,
        config: &hang::catalog::AudioConfig,
    ) -> anyhow::Result<Self> {
        let decode = remote_audio_decode_config();
        let consumer = moq_audio::decode::Consumer::new(broadcast, config, name, decode).await?;
        let engine = moq_audio::playback::Engine::open(Default::default()).await?;
        let sink = engine.sink(moq_audio::playback::Input {
            format: moq_audio::Format::F32,
            sample_rate: consumer.sample_rate(),
            layout: consumer.layout(),
            ..Default::default()
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
            channels: Some(self.consumer.layout().channels()),
            last_error: None,
        }
    }
}

pub(super) struct Task {
    task: JoinHandle<()>,
    pub(super) clock: AudioClockReader,
}

impl Task {
    pub(super) fn spawn(
        broadcast: &moq_tokio::moq_net::broadcast::Consumer,
        selection: &Selection,
        events: Events,
        volume: watch::Receiver<u8>,
    ) -> Self {
        let broadcast = broadcast.clone();
        let selection = selection.clone();
        let (writer, clock) = sync::audio_clock();
        let task = tokio::spawn(async move {
            run(broadcast, selection, events, writer, volume).await;
        });
        Self { task, clock }
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn run(
    broadcast: moq_tokio::moq_net::broadcast::Consumer,
    selection: Selection,
    events: Events,
    clock: AudioClockWriter,
    volume: watch::Receiver<u8>,
) {
    let started_at = Instant::now();
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
        view_generation = events.generation,
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
                view_generation = events.generation,
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
                })
                .await;
            return;
        }
    };
    let _volume_listener = VolumeListener::spawn(playback.sink.control(), volume);
    tracing::info!(
        broadcast = ?events.path,
        view_generation = events.generation,
        track = ?name,
        codec = %codec,
        decoded_sample_rate = playback.consumer.sample_rate(),
        decoded_channels = playback.consumer.layout().channels(),
        live_edge_budget_ms = super::AV_LIVE_EDGE_BUDGET.as_millis() as u64,
        elapsed_ms = elapsed_ms(started_at),
        output_device = "system-default",
        "remote audio pipeline opened; output callback has not been observed"
    );
    output_diagnostics::spawn("pipeline_open", events.generation, elapsed_ms(started_at));

    let mut stats = AudioStats::default();
    let mut reports = tokio::time::interval(REPORT_INTERVAL);
    reports.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    reports.tick().await;
    let mut callback_nonzero_observed = false;
    let mut timeline = sync::AudioTimeline::default();
    let mut output_trust = sync::AudioOutputTrust::default();

    loop {
        tokio::select! {
            decoded = playback.read() => {
                let frame = match decoded {
                    Ok(Some(frame)) => frame,
                    Ok(None) => {
                        tracing::debug!(
                            broadcast = ?events.path,
                            view_generation = events.generation,
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
                            view_generation = events.generation,
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
                    playback.consumer.layout().channels(),
                    playback.consumer.sample_rate(),
                );
                let nonzero_pcm = pcm_has_nonzero_f32(&frame.data);
                let (first_frame, first_nonzero_pcm) =
                    stats.decoded(timestamp_us, bytes, duration_us, nonzero_pcm);
                if first_frame {
                    tracing::info!(
                        broadcast = ?events.path,
                        view_generation = events.generation,
                        track = ?name,
                        frame_pts_us = %timestamp_us,
                        pcm_bytes = bytes,
                        pcm_duration_us = %duration_us,
                        nonzero_pcm,
                        elapsed_ms = elapsed_ms(started_at),
                        "decoded first remote PCM frame"
                    );
                    events
                        .send(playback.snapshot(ViewAudioPhase::Decoded, &codec))
                        .await;
                }
                if first_nonzero_pcm {
                    tracing::info!(
                        broadcast = ?events.path,
                        view_generation = events.generation,
                        track = ?name,
                        frame_pts_us = %timestamp_us,
                        pcm_bytes = bytes,
                        elapsed_ms = elapsed_ms(started_at),
                        "decoded first nonzero remote PCM frame"
                    );
                }

                let pts = Duration::from_micros(timestamp_us.min(u128::from(u64::MAX)) as u64);
                let duration = Duration::from_micros(duration_us.min(u128::from(u64::MAX)) as u64);
                if clock.observe_source(&mut timeline, pts, duration) {
                    tracing::warn!(
                        view_generation = events.generation,
                        "audio PTS discontinuity; video uses latest frames until the audio task is replaced"
                    );
                }

                let buffered_before = playback.sink.buffered();
                let write = match playback.sink.write(&frame.data) {
                    Ok(write) => write,
                    Err(error) => {
                        stats.write_failed();
                        tracing::warn!(
                            broadcast = ?events.path,
                            view_generation = events.generation,
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
                };
                let output_trusted = output_trust.observe(
                    buffered_before,
                    write.accepted_sample_frames,
                    write.dropped_sample_frames,
                );
                if !output_trusted {
                    clock.update(None);
                }
                if write.accepted_sample_frames == 0 {
                    continue;
                }
                let now = Instant::now();
                let anchor = if timeline.is_continuous() && output_trusted {
                    AudioAnchor::new(
                        sync::accepted_pcm_end(
                            pts,
                            write.accepted_sample_frames,
                            playback.consumer.sample_rate(),
                        ),
                        playback.sink.buffered(),
                        now,
                    )
                } else {
                    None
                };
                clock.update(anchor);
                if stats.wrote(write.accepted_sample_frames) {
                    let elapsed_ms = elapsed_ms(started_at);
                    tracing::info!(
                        broadcast = ?events.path,
                        view_generation = events.generation,
                        track = ?name,
                        frame_pts_us = %timestamp_us,
                        buffered_us = %playback.sink.buffered().as_micros(),
                        accepted_sample_frames = write.accepted_sample_frames,
                        dropped_sample_frames = write.dropped_sample_frames,
                        elapsed_ms,
                        "first remote PCM sink write returned successfully; output callback has not been observed"
                    );
                    output_diagnostics::spawn("first_sink_write", events.generation, elapsed_ms);
                    events
                        .send(playback.snapshot(ViewAudioPhase::Writing, &codec))
                        .await;
                }
            }
            _ = reports.tick() => {
                let peak = log_interval(&events, &name, &codec, &playback, &mut stats);
                if !callback_nonzero_observed && callback_consumed_nonzero(peak) {
                    callback_nonzero_observed = true;
                    let elapsed_ms = elapsed_ms(started_at);
                    tracing::info!(
                        broadcast = ?events.path,
                        view_generation = events.generation,
                        track = ?name,
                        peak,
                        elapsed_ms,
                        "audio output callback consumed nonzero PCM; audible output is not proven"
                    );
                    output_diagnostics::spawn(
                        "first_nonzero_callback",
                        events.generation,
                        elapsed_ms,
                    );
                    events
                        .send(playback.snapshot(ViewAudioPhase::CallbackConsumed, &codec))
                        .await;
                }
            }
        }
    }
}

fn elapsed_ms(started_at: Instant) -> u64 {
    started_at
        .elapsed()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn log_interval(
    events: &Events,
    track: &str,
    codec: &str,
    playback: &Playback,
    stats: &mut AudioStats,
) -> f32 {
    let report = stats.take_report();
    let buffered_us = playback.sink.buffered().as_micros();
    let peak = playback.sink.peak();
    tracing::info!(
        broadcast = ?events.path,
        view_generation = events.generation,
        track,
        codec,
        decoded_frames = report.decoded_frames,
        nonzero_pcm_frames = report.nonzero_pcm_frames,
        decoded_bytes = report.decoded_bytes,
        sink_writes = report.sink_writes,
        sink_write_errors = report.sink_write_errors,
        first_pts_us = ?report.first_pts_us,
        last_pts_us = ?report.last_pts_us,
        pts_gaps = report.pts_gaps,
        max_pts_gap_us = %report.max_pts_gap_us,
        pts_regressions = report.pts_regressions,
        buffered_us = %buffered_us,
        peak,
        callback_consumed_nonzero_pcm = callback_consumed_nonzero(peak),
        "remote audio playback interval"
    );
    peak
}

#[derive(Clone)]
pub(super) struct Events {
    generation: u64,
    path: String,
    sender: mpsc::Sender<ViewEvent>,
}

impl Events {
    pub(super) fn new(generation: u64, path: &str, sender: &mpsc::Sender<ViewEvent>) -> Self {
        Self {
            generation,
            path: path.to_owned(),
            sender: sender.clone(),
        }
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playback_volume_percent_maps_to_the_sink_gain_range() {
        assert_eq!(volume_scalar(0), 0.0);
        assert_eq!(volume_scalar(40), 0.4);
        assert_eq!(volume_scalar(100), 1.0);
        assert_eq!(volume_scalar(u8::MAX), 1.0);
    }

    #[test]
    fn remote_audio_decode_config_uses_f32_and_80ms_live_edge_budget() {
        let config = remote_audio_decode_config();

        assert_eq!(config.output.format, moq_audio::Format::F32);
        assert_eq!(config.max_age, Duration::from_millis(80));
    }
}
