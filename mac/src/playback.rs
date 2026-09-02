//! Catalog-driven H.264 and Opus playback with decoded-frame delivery.

mod audio;
mod sync;

use std::sync::Arc;
use std::time::{Duration, Instant};

use moq_mux::catalog::Stream;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct FrameIdentity {
    pub(crate) view_generation: u64,
    pub(crate) decoder_generation: u64,
    pub(crate) sequence: u64,
}

#[derive(Clone)]
pub(crate) struct Frame {
    pub(crate) identity: FrameIdentity,
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) display_width: u32,
    pub(crate) display_height: u32,
    pub(crate) rgba: Vec<u8>,
}

pub(crate) enum Event {
    Started {
        generation: u64,
        decoder: String,
        width: u32,
        height: u32,
    },
    Audio {
        generation: u64,
        snapshot: AudioSnapshot,
    },
    Ended {
        generation: u64,
        result: Result<(), String>,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum AudioPhase {
    #[default]
    Idle,
    Pending,
    NoAudio,
    TrackSelected,
    PcmDecoded,
    PcmSubmitted,
    Failed,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct AudioSnapshot {
    pub(crate) phase: AudioPhase,
    pub(crate) track: Option<String>,
    pub(crate) codec: Option<String>,
    pub(crate) sample_rate: Option<u32>,
    pub(crate) channels: Option<u32>,
    pub(crate) last_error: Option<String>,
}

impl AudioSnapshot {
    fn pending() -> Self {
        Self {
            phase: AudioPhase::Pending,
            ..Self::default()
        }
    }

    fn no_audio() -> Self {
        Self {
            phase: AudioPhase::NoAudio,
            ..Self::default()
        }
    }

    fn failed(message: &str) -> Self {
        Self {
            phase: AudioPhase::Failed,
            last_error: Some(message.to_owned()),
            ..Self::default()
        }
    }
}

#[derive(Default)]
pub(crate) struct Owner {
    task: Option<JoinHandle<()>>,
    cancel: Option<watch::Sender<bool>>,
}

impl Owner {
    pub(crate) fn start(
        &mut self,
        generation: u64,
        path: String,
        broadcast: moq_tokio::moq_net::broadcast::Consumer,
        events: mpsc::Sender<Event>,
        frames: watch::Sender<Option<Arc<Frame>>>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) {
        debug_assert!(self.task.is_none());
        let (cancel, cancelled) = watch::channel(false);
        self.cancel = Some(cancel);
        self.task = Some(tokio::spawn(run(
            generation, path, broadcast, cancelled, events, frames, wake,
        )));
    }

    pub(crate) async fn stop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel.send_replace(true);
        }
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel.send_replace(true);
        }
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct VideoIdentity {
    track: String,
    broadcast: Option<moq_tokio::moq_net::PathRelativeOwned>,
    codec: hang::catalog::VideoCodec,
    description: Option<Vec<u8>>,
    container: hang::catalog::Container,
    coded_width: Option<u32>,
    coded_height: Option<u32>,
    display: Option<(u32, u32)>,
    quarter_turns: u8,
    flip: bool,
}

#[derive(Clone, Debug)]
struct VideoSelection {
    name: String,
    config: Box<hang::catalog::VideoConfig>,
    identity: VideoIdentity,
}

impl PartialEq for VideoSelection {
    fn eq(&self, other: &Self) -> bool {
        self.identity == other.identity
    }
}

impl VideoSelection {
    fn from_catalog(
        mut video: hang::catalog::Video,
        preferred: Option<&str>,
    ) -> anyhow::Result<Option<Self>> {
        let selected = preferred
            .and_then(|name| {
                video
                    .renditions
                    .remove(name)
                    .filter(supported_video)
                    .map(|config| (name.to_owned(), config))
            })
            .or_else(|| {
                video
                    .renditions
                    .into_iter()
                    .find(|(_, config)| supported_video(config))
            });
        let Some((name, config)) = selected else {
            return Ok(None);
        };
        let rotation = video.rotation.unwrap_or(0.0);
        anyhow::ensure!(rotation.is_finite(), "remote screen rotation is invalid");
        let quarter_turns = ((rotation.rem_euclid(360.0) / 90.0).round() as u8) % 4;
        let display = video.display.and_then(|display| {
            (display.width > 0 && display.height > 0).then_some((display.width, display.height))
        });
        let identity = VideoIdentity {
            track: name.clone(),
            broadcast: config.broadcast.clone(),
            codec: config.codec.clone(),
            description: config.description.as_deref().map(<[u8]>::to_vec),
            container: config.container.clone(),
            coded_width: config.coded_width,
            coded_height: config.coded_height,
            display,
            quarter_turns,
            flip: video.flip.unwrap_or(false),
        };
        Ok(Some(Self {
            name,
            config: Box::new(config),
            identity,
        }))
    }

    async fn decoder(
        &self,
        broadcast: &moq_tokio::moq_net::broadcast::Consumer,
    ) -> anyhow::Result<moq_video::decode::Consumer> {
        moq_video::decode::Consumer::new(
            broadcast,
            &self.config,
            self.name.clone(),
            moq_video::decode::Config::new(),
        )
        .await
        .map_err(Into::into)
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Selection {
    video: Option<VideoSelection>,
    audio: audio::Selection,
}

impl Selection {
    fn from_catalog(
        catalog: moq_mux::catalog::hang::Catalog<()>,
        current: Option<&Self>,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            video: VideoSelection::from_catalog(
                catalog.video,
                current.and_then(|selection| {
                    selection.video.as_ref().map(|video| video.name.as_str())
                }),
            )?,
            audio: audio::Selection::from_catalog(
                catalog.audio,
                current.and_then(|selection| selection.audio.name()),
            ),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MediaChanges {
    video: bool,
    audio: bool,
}

fn media_changes(current: &Selection, next: &Selection, video_reader_active: bool) -> MediaChanges {
    MediaChanges {
        video: next.video != current.video || (next.video.is_some() && !video_reader_active),
        audio: next.audio != current.audio,
    }
}

fn supported_video(config: &hang::catalog::VideoConfig) -> bool {
    config.broadcast.is_none() && matches!(&config.codec, hang::catalog::VideoCodec::H264(_))
}

struct FrameSequence {
    view_generation: u64,
    decoder_generation: u64,
    sequence: u64,
    started: bool,
}

impl FrameSequence {
    fn new(view_generation: u64) -> Self {
        Self {
            view_generation,
            decoder_generation: 0,
            sequence: 0,
            started: false,
        }
    }

    fn replace_decoder(&mut self) {
        self.decoder_generation = self.decoder_generation.wrapping_add(1);
        self.sequence = 0;
    }

    fn next(&mut self) -> (FrameIdentity, bool) {
        self.sequence = self.sequence.wrapping_add(1);
        let first_view_frame = !self.started;
        self.started = true;
        (
            FrameIdentity {
                view_generation: self.view_generation,
                decoder_generation: self.decoder_generation,
                sequence: self.sequence,
            },
            first_view_frame,
        )
    }
}

enum VideoEvent {
    Frame(moq_video::Frame),
    Ended,
    Failed(String),
}

enum VideoEventDisposition {
    Frame(moq_video::Frame),
    WaitForCatalog,
    FailOwner(String),
}

fn video_event_disposition(event: VideoEvent) -> VideoEventDisposition {
    match event {
        VideoEvent::Frame(frame) => VideoEventDisposition::Frame(frame),
        VideoEvent::Ended => VideoEventDisposition::WaitForCatalog,
        VideoEvent::Failed(error) => VideoEventDisposition::FailOwner(error),
    }
}

struct VideoUpdate {
    generation: u64,
    event: VideoEvent,
}

fn accept_audio_update(current_generation: u64, update: audio::Update) -> Option<AudioSnapshot> {
    (update.generation == current_generation).then_some(update.snapshot)
}

struct VideoTask {
    task: Option<JoinHandle<()>>,
}

impl VideoTask {
    fn spawn(
        generation: u64,
        mut decoder: moq_video::decode::Consumer,
        updates: &mpsc::Sender<VideoUpdate>,
    ) -> Self {
        let updates = updates.clone();
        let task = tokio::spawn(async move {
            loop {
                let event = match decoder.read().await {
                    Ok(Some(frame)) => VideoEvent::Frame(frame),
                    Ok(None) => VideoEvent::Ended,
                    Err(error) => VideoEvent::Failed(error.to_string()),
                };
                let terminal = !matches!(&event, VideoEvent::Frame(_));
                if updates
                    .send(VideoUpdate { generation, event })
                    .await
                    .is_err()
                    || terminal
                {
                    return;
                }
            }
        });
        Self { task: Some(task) }
    }

    async fn stop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
            let _ = task.await;
        }
    }
}

impl Drop for VideoTask {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

async fn wait_for_cancel(cancel: &mut watch::Receiver<bool>) {
    if *cancel.borrow_and_update() {
        return;
    }
    loop {
        if cancel.changed().await.is_err() || *cancel.borrow_and_update() {
            return;
        }
    }
}

async fn wait_for_deadline(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline.into()).await,
        None => std::future::pending().await,
    }
}

async fn run(
    generation: u64,
    path: String,
    broadcast: moq_tokio::moq_net::broadcast::Consumer,
    cancel: watch::Receiver<bool>,
    events: mpsc::Sender<Event>,
    frames: watch::Sender<Option<Arc<Frame>>>,
    wake: Arc<dyn Fn() + Send + Sync>,
) {
    let stopped = cancel.clone();
    let result = run_inner(
        generation, &path, broadcast, cancel, &events, &frames, &wake,
    )
    .await
    .map_err(|error| error.to_string());
    let event = Event::Ended { generation, result };
    if *stopped.borrow() {
        let _ = events.try_send(event);
    } else {
        let _ = events.send(event).await;
    }
}

async fn run_inner(
    generation: u64,
    path: &str,
    broadcast: moq_tokio::moq_net::broadcast::Consumer,
    mut cancel: watch::Receiver<bool>,
    events: &mpsc::Sender<Event>,
    frames: &watch::Sender<Option<Arc<Frame>>>,
    wake: &Arc<dyn Fn() + Send + Sync>,
) -> anyhow::Result<()> {
    let mut catalog = tokio::select! {
        biased;
        _ = wait_for_cancel(&mut cancel) => return Ok(()),
        result = moq_mux::catalog::Consumer::<()>::new(
            &broadcast,
            moq_mux::catalog::CatalogFormat::Hang,
        ) => result?,
    };
    let first = tokio::select! {
        biased;
        _ = wait_for_cancel(&mut cancel) => return Ok(()),
        result = catalog.next() => result?
            .ok_or_else(|| anyhow::anyhow!("remote screen catalog ended"))?,
    };
    let mut selection = Selection::from_catalog(first, None)?;
    let mut sequence = FrameSequence::new(generation);
    let (video_tx, mut video_rx) = mpsc::channel(1);
    let mut decoder_name = None;
    let mut video_task = None;
    let media_clock = Arc::new(sync::MediaClock::default());
    let mut video_scheduler = sync::VideoScheduler::default();
    let mut queue_drops = 0_u64;
    let mut due_skips = 0_u64;
    let mut fallback_reanchors = 0_u64;
    let mut last_video_pts: Option<Duration> = None;
    let mut video_discontinuity_reported = false;

    if let Some(video) = &selection.video {
        sequence.replace_decoder();
        let decoder = tokio::select! {
            biased;
            _ = wait_for_cancel(&mut cancel) => return Ok(()),
            result = video.decoder(&broadcast) => result?,
        };
        decoder_name = Some(decoder.name().to_owned());
        tracing::info!(
            broadcast = path,
            view_generation = generation,
            decoder_generation = sequence.decoder_generation,
            decoder = decoder.name(),
            track = %video.name,
            "remote video decoder opened"
        );
        video_task = Some(VideoTask::spawn(
            sequence.decoder_generation,
            decoder,
            &video_tx,
        ));
    } else {
        tracing::debug!(
            view_generation = generation,
            "waiting for a playable remote video rendition"
        );
    }

    let (audio_tx, mut audio_rx) = mpsc::channel(8);
    let audio_engine = Arc::new(tokio::sync::OnceCell::new());
    let mut audio_generation = 1_u64;
    let mut audio_task = audio::Task::spawn(
        audio_generation,
        path,
        &broadcast,
        &selection.audio,
        &audio_tx,
        &audio_engine,
        &media_clock,
    );

    let result = async {
        loop {
            let advance = video_scheduler.advance(media_clock.audio_anchor(), Instant::now());
            if let Some(source) = advance.source_changed {
                tracing::info!(
                    broadcast = path,
                    view_generation = generation,
                    clock_source = ?source,
                    "remote video playback clock source changed"
                );
            }
            if advance.skipped_due > 0 {
                due_skips = due_skips.saturating_add(advance.skipped_due as u64);
                tracing::debug!(
                    broadcast = path,
                    view_generation = generation,
                    skipped_due = advance.skipped_due,
                    skipped_due_total = due_skips,
                    "remote video scheduler selected the latest due frame"
                );
            }
            if let Some(decoded) = advance.due {
                let selected = selection
                    .video
                    .as_ref()
                    .expect("scheduled video frame has a selection");
                let (identity, first_view_frame) = sequence.next();
                let display = selected.identity.display;
                let quarter_turns = selected.identity.quarter_turns;
                let flip = selected.identity.flip;
                let frame = tokio::task::spawn_blocking(move || {
                    Frame::from_video(decoded, identity, display, quarter_turns, flip)
                })
                .await??;
                let width = frame.display_width;
                let height = frame.display_height;
                frames.send_replace(Some(Arc::new(frame)));
                wake();
                if first_view_frame {
                    events
                        .send(Event::Started {
                            generation,
                            decoder: decoder_name
                                .clone()
                                .expect("scheduled video frame has a decoder name"),
                            width,
                            height,
                        })
                        .await
                        .map_err(|_| anyhow::anyhow!("playback event receiver closed"))?;
                }
            }

            tokio::select! {
                biased;
                _ = wait_for_cancel(&mut cancel) => {
                    tracing::debug!(
                        view_generation = generation,
                        "remote playback cancellation received"
                    );
                    break Ok(());
                }
                update = catalog.next() => {
                    let Some(update) = update? else {
                        anyhow::bail!("remote screen catalog ended");
                    };
                    let next = Selection::from_catalog(update, Some(&selection))?;
                    let changes = media_changes(&selection, &next, video_task.is_some());
                    if !changes.video && !changes.audio {
                        selection = next;
                        continue;
                    }

                    tracing::info!(
                        view_generation = generation,
                        video_changed = changes.video,
                        audio_changed = changes.audio,
                        old_video_track = %selection
                            .video
                            .as_ref()
                            .map(|video| video.name.as_str())
                            .unwrap_or("<none>"),
                        new_video_track = %next
                            .video
                            .as_ref()
                            .map(|video| video.name.as_str())
                            .unwrap_or("<none>"),
                        "remote screen catalog changed"
                    );

                    if changes.audio {
                        let reason = audio::transition_teardown_reason(
                            &selection.audio,
                            &next.audio,
                        );
                        video_scheduler.reset_fallback();
                        audio_task.stop(reason).await;
                        audio_generation = audio_generation.wrapping_add(1);
                        audio_task = audio::Task::spawn(
                            audio_generation,
                            path,
                            &broadcast,
                            &next.audio,
                            &audio_tx,
                            &audio_engine,
                            &media_clock,
                        );
                    }
                    if changes.video {
                        if let Some(mut task) = video_task.take() {
                            tracing::info!(
                                broadcast = path,
                                view_generation = generation,
                                decoder_generation = sequence.decoder_generation,
                                teardown_reason = "replacement",
                                queue_drops,
                                due_skips,
                                fallback_reanchors,
                                "remote video generation stopped by playback owner"
                            );
                            task.stop().await;
                        }
                        sequence.replace_decoder();
                        video_scheduler.reset();
                        queue_drops = 0;
                        due_skips = 0;
                        fallback_reanchors = 0;
                        last_video_pts = None;
                        video_discontinuity_reported = false;
                        decoder_name = None;
                        if let Some(video) = &next.video {
                            let decoder = tokio::select! {
                                biased;
                                _ = wait_for_cancel(&mut cancel) => break Ok(()),
                                result = video.decoder(&broadcast) => result?,
                            };
                            decoder_name = Some(decoder.name().to_owned());
                            tracing::info!(
                                broadcast = path,
                                view_generation = generation,
                                decoder_generation = sequence.decoder_generation,
                                decoder = decoder.name(),
                                track = %video.name,
                                "remote video decoder rebuilt after catalog change"
                            );
                            video_task = Some(VideoTask::spawn(
                                sequence.decoder_generation,
                                decoder,
                                &video_tx,
                            ));
                        } else {
                            tracing::debug!(
                                view_generation = generation,
                                decoder_generation = sequence.decoder_generation,
                                "remote video rendition withdrawn; waiting for catalog replacement"
                            );
                        }
                    }
                    selection = next;
                }
                update = audio_rx.recv() => {
                    let Some(update) = update else {
                        anyhow::bail!("remote audio event channel closed");
                    };
                    if let Some(snapshot) = accept_audio_update(audio_generation, update) {
                        events
                            .send(Event::Audio {
                                generation,
                                snapshot,
                            })
                            .await
                            .map_err(|_| anyhow::anyhow!("playback event receiver closed"))?;
                    }
                }
                update = video_rx.recv(), if video_task.is_some() => {
                    let Some(update) = update else {
                        anyhow::bail!("remote video reader stopped");
                    };
                    if update.generation != sequence.decoder_generation {
                        continue;
                    }
                    match video_event_disposition(update.event) {
                        VideoEventDisposition::Frame(decoded) => {
                            let pts = sync::timestamp(decoded.timestamp);
                            if let Some(previous) = last_video_pts
                                && !video_discontinuity_reported
                            {
                                if pts < previous {
                                    video_discontinuity_reported = true;
                                    tracing::warn!(
                                        broadcast = path,
                                        view_generation = generation,
                                        decoder_generation = sequence.decoder_generation,
                                        previous_pts_us = previous.as_micros() as u64,
                                        actual_pts_us = pts.as_micros() as u64,
                                        "remote video PTS regressed"
                                    );
                                } else if pts.saturating_sub(previous) > Duration::from_secs(1) {
                                    video_discontinuity_reported = true;
                                    tracing::warn!(
                                        broadcast = path,
                                        view_generation = generation,
                                        decoder_generation = sequence.decoder_generation,
                                        gap_us = pts.saturating_sub(previous).as_micros() as u64,
                                        "remote video PTS gap detected"
                                    );
                                }
                            }
                            last_video_pts = Some(pts);
                            let pushed = video_scheduler.push(pts, decoded, Instant::now());
                            if pushed.fallback_reanchored {
                                fallback_reanchors = fallback_reanchors.saturating_add(1);
                                if fallback_reanchors == 1 {
                                    tracing::info!(
                                        broadcast = path,
                                        view_generation = generation,
                                        decoder_generation = sequence.decoder_generation,
                                        frame_pts_us = pts.as_micros() as u64,
                                        latency_ceiling_ms = sync::VIDEO_FALLBACK_LATENCY_CEILING
                                            .as_millis() as u64,
                                        "remote video fallback clock re-anchored to the live edge"
                                    );
                                }
                            }
                            if pushed.dropped.is_some() {
                                queue_drops = queue_drops.saturating_add(1);
                                if queue_drops == 1 {
                                    tracing::warn!(
                                        broadcast = path,
                                        view_generation = generation,
                                        decoder_generation = sequence.decoder_generation,
                                        queue_capacity = sync::MAX_VIDEO_FRAMES,
                                        rejected_incoming = !pushed.accepted,
                                        "remote decoded video frame dropped by the bounded scheduler"
                                    );
                                }
                            }
                        }
                        VideoEventDisposition::WaitForCatalog => {
                            if let Some(mut task) = video_task.take() {
                                task.stop().await;
                            }
                            video_scheduler.reset();
                            decoder_name = None;
                            tracing::info!(
                                broadcast = path,
                                view_generation = generation,
                                decoder_generation = sequence.decoder_generation,
                                teardown_reason = "ended",
                                queue_drops,
                                due_skips,
                                fallback_reanchors,
                                "remote video track ended; waiting for catalog replacement"
                            );
                        }
                        VideoEventDisposition::FailOwner(error) => anyhow::bail!(error),
                    }
                }
                _ = media_clock.changed() => {}
                _ = wait_for_deadline(advance.deadline) => {}
            }
        }
    }
    .await;

    if let Some(mut task) = video_task {
        tracing::info!(
            broadcast = path,
            view_generation = generation,
            decoder_generation = sequence.decoder_generation,
            teardown_reason = "stop",
            queue_drops,
            due_skips,
            fallback_reanchors,
            "remote video generation stopped by playback owner"
        );
        task.stop().await;
    }
    audio_task.stop(audio::OwnerTeardownReason::Stop).await;
    result
}

impl Frame {
    fn from_video(
        frame: moq_video::Frame,
        identity: FrameIdentity,
        display: Option<(u32, u32)>,
        quarter_turns: u8,
        flip: bool,
    ) -> anyhow::Result<Self> {
        let width = frame.surface.width() as usize;
        let height = frame.surface.height() as usize;
        anyhow::ensure!(
            width > 0 && height > 0 && width.is_multiple_of(2) && height.is_multiple_of(2),
            "remote I420 frame dimensions must be non-zero and even"
        );
        let i420 = frame.surface.into_i420()?;
        let pixels = width
            .checked_mul(height)
            .ok_or_else(|| anyhow::anyhow!("remote frame dimensions overflow"))?;
        let expected = pixels
            .checked_mul(3)
            .map(|length| length / 2)
            .ok_or_else(|| anyhow::anyhow!("remote I420 frame length overflow"))?;
        anyhow::ensure!(
            i420.len() == expected,
            "remote I420 frame has an invalid length"
        );
        let rgba_length = pixels
            .checked_mul(4)
            .ok_or_else(|| anyhow::anyhow!("remote RGBA frame length overflow"))?;
        let (rgba, width, height) =
            convert_i420(&i420, width, height, rgba_length, quarter_turns, flip);
        let (display_width, display_height) = display.unwrap_or((width as u32, height as u32));
        Ok(Self {
            identity,
            width,
            height,
            display_width,
            display_height,
            rgba,
        })
    }
}

fn convert_i420(
    source: &[u8],
    width: usize,
    height: usize,
    rgba_length: usize,
    quarter_turns: u8,
    flip: bool,
) -> (Vec<u8>, usize, usize) {
    let quarter_turns = quarter_turns % 4;
    let (output_width, output_height) = if quarter_turns.is_multiple_of(2) {
        (width, height)
    } else {
        (height, width)
    };
    let mut output = vec![0; rgba_length];
    let pixels = width * height;
    let u_offset = pixels;
    let v_offset = pixels + pixels / 4;
    for output_y in 0..output_height {
        for output_x in 0..output_width {
            let (mut source_x, source_y) = match quarter_turns {
                0 => (output_x, output_y),
                1 => (output_y, height - 1 - output_x),
                2 => (width - 1 - output_x, height - 1 - output_y),
                3 => (width - 1 - output_y, output_x),
                _ => unreachable!(),
            };
            if flip {
                source_x = width - 1 - source_x;
            }
            let luma = i32::from(source[source_y * width + source_x]) - 16;
            let chroma = (source_y / 2) * (width / 2) + source_x / 2;
            let u = i32::from(source[u_offset + chroma]) - 128;
            let v = i32::from(source[v_offset + chroma]) - 128;
            let red = (298 * luma + 409 * v + 128) >> 8;
            let green = (298 * luma - 100 * u - 208 * v + 128) >> 8;
            let blue = (298 * luma + 516 * u + 128) >> 8;
            let output_offset = (output_y * output_width + output_x) * 4;
            output[output_offset..output_offset + 4].copy_from_slice(&[
                red.clamp(0, 255) as u8,
                green.clamp(0, 255) as u8,
                blue.clamp(0, 255) as u8,
                255,
            ]);
        }
    }
    (output, output_width, output_height)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h264() -> hang::catalog::VideoConfig {
        hang::catalog::VideoConfig::new(hang::catalog::H264 {
            inline: true,
            profile: 0x42,
            constraints: 0xc0,
            level: 0x1f,
        })
    }

    #[test]
    fn replacing_decoder_advances_only_decoder_generation() {
        let mut sequence = FrameSequence::new(7);
        sequence.replace_decoder();
        let (first, first_view_frame) = sequence.next();
        sequence.replace_decoder();
        let (replacement, replacement_first_view_frame) = sequence.next();

        assert_eq!(first.view_generation, 7);
        assert_eq!(replacement.view_generation, 7);
        assert!(replacement.decoder_generation > first.decoder_generation);
        assert_eq!(replacement.sequence, 1);
        assert!(first_view_frame);
        assert!(!replacement_first_view_frame);
    }

    #[test]
    fn conversion_rotates_and_flips_without_changing_pixels() {
        let source = vec![16, 64, 128, 235, 128, 128];
        let (plain, _, _) = convert_i420(&source, 2, 2, 16, 0, false);
        let (rotated, width, height) = convert_i420(&source, 2, 2, 16, 1, false);
        assert_eq!((width, height), (2, 2));
        assert_eq!(&rotated[0..4], &plain[8..12]);
        assert_eq!(&rotated[4..8], &plain[0..4]);
        assert_eq!(&rotated[8..12], &plain[12..16]);
        assert_eq!(&rotated[12..16], &plain[4..8]);

        let (flipped, _, _) = convert_i420(&source, 2, 2, 16, 0, true);
        assert_eq!(&flipped[0..4], &plain[4..8]);
        assert_eq!(&flipped[4..8], &plain[0..4]);
    }

    #[test]
    fn video_only_catalog_does_not_wait_for_audio() {
        let mut catalog = moq_mux::catalog::hang::Catalog::<()>::default();
        catalog.video.renditions.insert("screen".into(), h264());

        let selection = Selection::from_catalog(catalog, None).expect("valid catalog");

        assert!(selection.video.is_some());
        assert_eq!(selection.audio, audio::Selection::NotPublished);
    }

    #[test]
    fn audio_only_catalog_selects_opus_independently() {
        let mut catalog = moq_mux::catalog::hang::Catalog::<()>::default();
        catalog.audio.renditions.insert(
            "audio".into(),
            hang::catalog::AudioConfig::new(hang::catalog::AudioCodec::Opus, 48_000, 2),
        );

        let selection = Selection::from_catalog(catalog, None).expect("valid catalog");

        assert!(selection.video.is_none());
        assert!(matches!(
            selection.audio,
            audio::Selection::Playable { name, .. } if name == "audio"
        ));
    }

    #[test]
    fn audio_only_catalog_change_does_not_replace_video() {
        let mut catalog = moq_mux::catalog::hang::Catalog::<()>::default();
        catalog.video.renditions.insert("screen".into(), h264());
        let current = Selection::from_catalog(catalog.clone(), None).expect("valid catalog");
        catalog.audio.renditions.insert(
            "audio".into(),
            hang::catalog::AudioConfig::new(hang::catalog::AudioCodec::Opus, 48_000, 2),
        );
        let next = Selection::from_catalog(catalog, Some(&current)).expect("valid catalog");

        assert_eq!(
            media_changes(&current, &next, true),
            MediaChanges {
                video: false,
                audio: true,
            }
        );
    }

    #[test]
    fn stale_audio_generation_update_is_ignored() {
        let stale = audio::Update {
            generation: 4,
            snapshot: AudioSnapshot::failed("stale"),
        };
        let current = audio::Update {
            generation: 5,
            snapshot: AudioSnapshot::no_audio(),
        };

        assert!(accept_audio_update(5, stale).is_none());
        assert_eq!(
            accept_audio_update(5, current).map(|snapshot| snapshot.phase),
            Some(AudioPhase::NoAudio)
        );
    }

    #[test]
    fn video_track_end_waits_for_catalog_replacement() {
        assert!(matches!(
            video_event_disposition(VideoEvent::Ended),
            VideoEventDisposition::WaitForCatalog
        ));
        assert!(matches!(
            video_event_disposition(VideoEvent::Failed("decode".into())),
            VideoEventDisposition::FailOwner(error) if error == "decode"
        ));
    }
}
