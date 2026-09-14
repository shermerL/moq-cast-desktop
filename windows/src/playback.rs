//! Remote screen playback lifecycle and decoded-frame delivery.

#[cfg(target_os = "windows")]
mod audio;
#[cfg(any(target_os = "windows", test))]
mod output_diagnostics;
#[cfg(any(target_os = "windows", test))]
mod sync;

#[cfg(any(target_os = "windows", test))]
use std::future::Future;
use std::sync::Arc;

use tokio::sync::{mpsc, watch};

#[cfg(any(target_os = "windows", test))]
const AV_LIVE_EDGE_BUDGET: std::time::Duration = std::time::Duration::from_millis(80);
#[cfg(any(target_os = "windows", test))]
const VIDEO_EVENT_CAPACITY: usize = 1;

#[cfg(any(target_os = "windows", test))]
fn video_max_age(has_playable_audio: bool) -> std::time::Duration {
    if has_playable_audio {
        AV_LIVE_EDGE_BUDGET
    } else {
        std::time::Duration::ZERO
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ViewPhase {
    #[default]
    Idle,
    Preparing,
    Viewing,
    Stopping,
    Failed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) enum ViewAudioPhase {
    #[default]
    Idle,
    Pending,
    TrackSelected,
    Decoded,
    NotPublished,
    Writing,
    CallbackConsumed,
    Failed,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ViewAudioSnapshot {
    pub(crate) phase: ViewAudioPhase,
    pub(crate) codec: Option<String>,
    pub(crate) sample_rate: Option<u32>,
    pub(crate) channels: Option<u32>,
    pub(crate) last_error: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ViewSnapshot {
    pub(crate) generation: u64,
    pub(crate) phase: ViewPhase,
    pub(crate) path: Option<String>,
    pub(crate) decoder: Option<String>,
    pub(crate) width: Option<u32>,
    pub(crate) height: Option<u32>,
    pub(crate) audio: ViewAudioSnapshot,
    pub(crate) last_error: Option<String>,
}

impl ViewSnapshot {
    pub(crate) fn begin(&mut self, path: &str) -> Option<u64> {
        if matches!(
            self.phase,
            ViewPhase::Preparing | ViewPhase::Viewing | ViewPhase::Stopping
        ) {
            return None;
        }
        self.generation = self.generation.saturating_add(1);
        self.phase = ViewPhase::Preparing;
        self.path = Some(path.to_owned());
        self.decoder = None;
        self.width = None;
        self.height = None;
        self.audio = ViewAudioSnapshot {
            phase: ViewAudioPhase::Pending,
            ..ViewAudioSnapshot::default()
        };
        self.last_error = None;
        Some(self.generation)
    }

    pub(crate) fn decoder_ready(
        &mut self,
        generation: u64,
        path: &str,
        decoder: String,
        width: u32,
        height: u32,
    ) -> bool {
        if generation != self.generation
            || self.path.as_deref() != Some(path)
            || !matches!(self.phase, ViewPhase::Preparing | ViewPhase::Viewing)
        {
            return false;
        }
        self.phase = ViewPhase::Viewing;
        self.decoder = Some(decoder);
        self.width = Some(width);
        self.height = Some(height);
        true
    }

    pub(crate) fn audio_changed(
        &mut self,
        generation: u64,
        path: &str,
        audio: ViewAudioSnapshot,
    ) -> bool {
        if generation != self.generation
            || self.path.as_deref() != Some(path)
            || !matches!(self.phase, ViewPhase::Preparing | ViewPhase::Viewing)
        {
            return false;
        }
        self.audio = audio;
        true
    }

    pub(crate) fn begin_stop(&mut self) -> Option<u64> {
        if !matches!(self.phase, ViewPhase::Preparing | ViewPhase::Viewing) {
            return None;
        }
        self.phase = ViewPhase::Stopping;
        Some(self.generation)
    }

    pub(crate) fn stopped(&mut self, generation: u64) -> bool {
        if generation != self.generation || self.phase != ViewPhase::Stopping {
            return false;
        }
        self.reset(ViewPhase::Idle, None);
        true
    }

    pub(crate) fn ended(&mut self, generation: u64, result: Result<(), String>) -> bool {
        if generation != self.generation
            || !matches!(self.phase, ViewPhase::Preparing | ViewPhase::Viewing)
        {
            return false;
        }
        match result {
            Ok(()) => self.reset(ViewPhase::Idle, None),
            Err(error) => self.reset(ViewPhase::Failed, Some(error)),
        }
        true
    }

    fn reset(&mut self, phase: ViewPhase, error: Option<String>) {
        self.phase = phase;
        self.path = None;
        self.decoder = None;
        self.width = None;
        self.height = None;
        self.audio = ViewAudioSnapshot::default();
        self.last_error = error;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct PlaybackFrameIdentity {
    pub(crate) view_generation: u64,
    pub(crate) decoder_generation: u64,
    pub(crate) sequence: u64,
}

#[derive(Clone)]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) struct PlaybackFrame {
    pub(crate) identity: PlaybackFrameIdentity,
    pub(crate) timestamp_us: u128,
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) display_width: u32,
    pub(crate) display_height: u32,
    pub(crate) rgba: Vec<u8>,
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) enum ViewEvent {
    DecoderReady {
        generation: u64,
        path: String,
        decoder: String,
        width: u32,
        height: u32,
    },
    AudioChanged {
        generation: u64,
        path: String,
        audio: ViewAudioSnapshot,
    },
    Ended {
        generation: u64,
        result: Result<(), String>,
    },
}

#[cfg(any(target_os = "windows", test))]
#[derive(Debug)]
struct VideoUpdate<T> {
    generation: u64,
    event: T,
}

#[cfg(any(target_os = "windows", test))]
fn accept_video_update<T>(generation: u64, update: VideoUpdate<T>) -> Option<T> {
    (update.generation == generation).then_some(update.event)
}

#[cfg(any(target_os = "windows", test))]
struct VideoTask {
    handle: Option<tokio::task::JoinHandle<()>>,
}

#[cfg(any(target_os = "windows", test))]
trait VideoReader: Send + 'static {
    type Frame: Send + 'static;

    fn read(&mut self) -> impl Future<Output = Result<Option<Self::Frame>, String>> + Send;
}

#[cfg(any(target_os = "windows", test))]
impl VideoTask {
    fn spawn<R>(
        generation: u64,
        mut reader: R,
        updates: &mpsc::Sender<VideoUpdate<VideoEvent<R::Frame>>>,
    ) -> Self
    where
        R: VideoReader,
    {
        let updates = updates.clone();
        let handle = tokio::spawn(async move {
            loop {
                let event = match reader.read().await {
                    Ok(Some(frame)) => VideoEvent::Frame(frame),
                    Ok(None) => VideoEvent::Ended,
                    Err(error) => VideoEvent::Failed(error),
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
        Self {
            handle: Some(handle),
        }
    }

    async fn stop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.abort();
            let _ = handle.await;
        }
    }
}

#[cfg(any(target_os = "windows", test))]
impl Drop for VideoTask {
    fn drop(&mut self) {
        if let Some(handle) = &self.handle {
            handle.abort();
        }
    }
}

#[cfg(any(target_os = "windows", test))]
#[derive(Debug)]
enum VideoEvent<T> {
    Frame(T),
    Ended,
    Failed(String),
}

#[cfg(target_os = "windows")]
impl VideoReader for moq_video::decode::Consumer {
    type Frame = moq_video::Frame;

    async fn read(&mut self) -> Result<Option<Self::Frame>, String> {
        moq_video::decode::Consumer::read(self)
            .await
            .map_err(|error| error.to_string())
    }
}

#[cfg(any(target_os = "windows", test))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct AudioStatsReport {
    decoded_frames: u64,
    nonzero_pcm_frames: u64,
    decoded_bytes: u64,
    sink_writes: u64,
    sink_write_errors: u64,
    first_pts_us: Option<u128>,
    last_pts_us: Option<u128>,
    pts_gaps: u64,
    max_pts_gap_us: u128,
    pts_regressions: u64,
}

#[cfg(any(target_os = "windows", test))]
#[derive(Default)]
struct AudioStats {
    report: AudioStatsReport,
    total_decoded_frames: u64,
    total_nonzero_pcm_frames: u64,
    total_sink_writes: u64,
    previous_pts_us: Option<u128>,
    previous_end_us: Option<u128>,
}

#[cfg(any(target_os = "windows", test))]
impl AudioStats {
    fn decoded(
        &mut self,
        pts_us: u128,
        bytes: usize,
        duration_us: u128,
        nonzero_pcm: bool,
    ) -> (bool, bool) {
        let first_frame = self.total_decoded_frames == 0;
        let first_nonzero_pcm = nonzero_pcm && self.total_nonzero_pcm_frames == 0;
        self.total_decoded_frames = self.total_decoded_frames.saturating_add(1);
        self.report.decoded_frames = self.report.decoded_frames.saturating_add(1);
        self.report.decoded_bytes = self.report.decoded_bytes.saturating_add(bytes as u64);
        if nonzero_pcm {
            self.total_nonzero_pcm_frames = self.total_nonzero_pcm_frames.saturating_add(1);
            self.report.nonzero_pcm_frames = self.report.nonzero_pcm_frames.saturating_add(1);
        }
        self.report.first_pts_us.get_or_insert(pts_us);
        self.report.last_pts_us = Some(pts_us);
        if self
            .previous_pts_us
            .is_some_and(|previous| pts_us < previous)
        {
            self.report.pts_regressions = self.report.pts_regressions.saturating_add(1);
        }
        if let Some(previous_end_us) = self.previous_end_us
            && pts_us > previous_end_us
        {
            let gap_us = pts_us - previous_end_us;
            self.report.pts_gaps = self.report.pts_gaps.saturating_add(1);
            self.report.max_pts_gap_us = self.report.max_pts_gap_us.max(gap_us);
        }
        self.previous_pts_us = Some(pts_us);
        self.previous_end_us = Some(pts_us.saturating_add(duration_us));
        (first_frame, first_nonzero_pcm)
    }

    fn wrote(&mut self) -> bool {
        let first = self.total_sink_writes == 0;
        self.total_sink_writes = self.total_sink_writes.saturating_add(1);
        self.report.sink_writes = self.report.sink_writes.saturating_add(1);
        first
    }

    fn write_failed(&mut self) {
        self.report.sink_write_errors = self.report.sink_write_errors.saturating_add(1);
    }

    fn take_report(&mut self) -> AudioStatsReport {
        std::mem::take(&mut self.report)
    }
}

#[cfg(any(target_os = "windows", test))]
fn pcm_duration_us(bytes: usize, channels: u32, sample_rate: u32) -> u128 {
    if channels == 0 || sample_rate == 0 {
        return 0;
    }
    let frames = bytes / std::mem::size_of::<f32>() / channels as usize;
    (frames as u128 * 1_000_000) / sample_rate as u128
}

#[cfg(any(target_os = "windows", test))]
fn pcm_has_nonzero_f32(data: &[u8]) -> bool {
    data.chunks_exact(std::mem::size_of::<f32>())
        .any(|bytes| f32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]).abs() > 0.0)
}

#[cfg(any(target_os = "windows", test))]
fn callback_consumed_nonzero(peak: f32) -> bool {
    peak.is_finite() && peak > 0.0
}

#[cfg(target_os = "windows")]
#[derive(Clone, PartialEq)]
struct Selection {
    name: String,
    config: hang::catalog::VideoConfig,
    display: Option<(u32, u32)>,
    quarter_turns: u8,
    flip: bool,
    audio: audio::Selection,
}

#[cfg(target_os = "windows")]
impl Selection {
    fn from_catalog(catalog: moq_mux::catalog::hang::Catalog) -> anyhow::Result<Self> {
        let audio = audio::Selection::from_catalog(catalog.audio);
        let (name, config) = catalog
            .video
            .renditions
            .into_iter()
            .find(|(_, config)| matches!(&config.codec, hang::catalog::VideoCodec::H264(_)))
            .ok_or_else(|| anyhow::anyhow!("remote screen has no H.264 rendition"))?;
        anyhow::ensure!(
            config.broadcast.is_none(),
            "external rendition broadcasts are not supported"
        );
        let rotation = catalog.video.rotation.unwrap_or(0.0);
        anyhow::ensure!(rotation.is_finite(), "remote screen rotation is invalid");
        let quarter_turns = ((rotation.rem_euclid(360.0) / 90.0).round() as u8) % 4;
        let display = catalog.video.display.and_then(|display| {
            (display.width > 0 && display.height > 0).then_some((display.width, display.height))
        });
        Ok(Self {
            name,
            config,
            display,
            quarter_turns,
            flip: catalog.video.flip.unwrap_or(false),
            audio,
        })
    }

    async fn decoder(
        &self,
        broadcast: &moq_tokio::moq_net::broadcast::Consumer,
        max_age: std::time::Duration,
    ) -> anyhow::Result<moq_video::decode::Consumer> {
        let mut config = moq_video::decode::Config::new();
        config.max_age = max_age;
        moq_video::decode::Consumer::new(broadcast, &self.config, self.name.clone(), config)
            .await
            .map_err(Into::into)
    }
}

#[cfg(target_os = "windows")]
struct PendingFrame {
    decoded: moq_video::Frame,
    identity: PlaybackFrameIdentity,
    display: Option<(u32, u32)>,
    quarter_turns: u8,
    flip: bool,
}

#[cfg(any(target_os = "windows", test))]
async fn wait_for_cancel(cancel: &mut watch::Receiver<bool>) {
    if *cancel.borrow() {
        return;
    }
    let _ = cancel.wait_for(|cancelled| *cancelled).await;
}

#[cfg(any(target_os = "windows", test))]
async fn send_unless_cancelled<T: Send>(
    cancel: &mut watch::Receiver<bool>,
    events: &mpsc::Sender<T>,
    event: T,
) -> bool {
    tokio::select! {
        biased;
        _ = wait_for_cancel(cancel) => false,
        result = events.send(event) => result.is_ok(),
    }
}

#[cfg(target_os = "windows")]
pub(crate) async fn run(
    generation: u64,
    path: String,
    broadcast: moq_tokio::moq_net::broadcast::Consumer,
    events: mpsc::Sender<ViewEvent>,
    frames: watch::Sender<Option<Arc<PlaybackFrame>>>,
    mut cancel: watch::Receiver<bool>,
    volume: watch::Receiver<u8>,
) {
    use moq_mux::catalog::Stream;

    let result = async {
        let mut catalog = tokio::select! {
            biased;
            _ = wait_for_cancel(&mut cancel) => return Ok(()),
            catalog = moq_mux::catalog::Consumer::<()>::new(
                &broadcast,
                moq_mux::catalog::CatalogFormat::Hang,
            ) => catalog?,
        };
        let first = tokio::select! {
            biased;
            _ = wait_for_cancel(&mut cancel) => return Ok(()),
            first = catalog.next() => first?
                .ok_or_else(|| anyhow::anyhow!("remote screen catalog ended"))?,
        };
        let mut selection = Selection::from_catalog(first)?;
        let initial_audio = matches!(selection.audio, audio::Selection::Playable { .. });
        let video_max_age = video_max_age(initial_audio);
        let decoder = tokio::select! {
            biased;
            _ = wait_for_cancel(&mut cancel) => return Ok(()),
            decoder = selection.decoder(&broadcast, video_max_age) => decoder?,
        };
        let mut decoder_name = decoder.name().to_owned();
        let (video_updates_tx, mut video_updates_rx) = mpsc::channel(VIDEO_EVENT_CAPACITY);
        let audio_events = audio::Events::new(generation, &path, &events);
        let mut audio_task = audio::Task::spawn(
            &broadcast,
            &selection.audio,
            audio_events.clone(),
            volume.clone(),
        );
        let mut scheduler = sync::VideoScheduler::<PendingFrame>::default();
        let mut audio_sync_allowed = initial_audio;
        let mut clock_source = None;
        let mut last_delta_us = None;
        let mut reports = tokio::time::interval(std::time::Duration::from_secs(1));
        reports.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        reports.tick().await;
        let mut decoder_generation = 1_u64;
        let mut video_task = Some(VideoTask::spawn(
            decoder_generation,
            decoder,
            &video_updates_tx,
        ));
        let mut sequence = 0_u64;
        let mut decoder_ready = false;
        let mut last_timestamp_us = None;
        let mut view_high_water_timestamp_us = None;
        tracing::info!(
            view_generation = generation,
            decoder_generation,
            decoder = %decoder_name,
            track = %selection.name,
            "remote video decoder opened"
        );

        let loop_result: anyhow::Result<()> = async {
            loop {
            let anchor = if audio_sync_allowed {
                audio_task.clock.anchor()
            } else {
                None
            };
            let advance = scheduler.advance(anchor, std::time::Instant::now());
            if clock_source != Some(advance.audio_master) {
                clock_source = Some(advance.audio_master);
                tracing::info!(
                    view_generation = generation,
                    audio_master = advance.audio_master,
                    "video presentation clock changed; audio position is an estimate"
                );
            }
            if let Some(pending) = advance.frame {
                last_delta_us = advance.delta_us;
                let frame = tokio::task::spawn_blocking(move || {
                    PlaybackFrame::from_video(
                        pending.decoded,
                        pending.identity,
                        pending.display,
                        pending.quarter_turns,
                        pending.flip,
                    )
                })
                .await??;
                let width = frame.display_width;
                let height = frame.display_height;
                frames.send_replace(Some(Arc::new(frame)));
                if !decoder_ready {
                    decoder_ready = true;
                    let sent = send_unless_cancelled(
                        &mut cancel,
                        &events,
                        ViewEvent::DecoderReady {
                            generation,
                            path: path.clone(),
                            decoder: decoder_name.clone(),
                            width,
                            height,
                        },
                    )
                    .await;
                    if !sent {
                        break Ok(());
                    }
                }
            }
            tokio::select! {
                biased;
                _ = wait_for_cancel(&mut cancel) => break Ok(()),
                _ = async {
                    match advance.deadline {
                        Some(deadline) => tokio::time::sleep_until(deadline.into()).await,
                        None => std::future::pending().await,
                    }
                } => {}
                _ = audio_task.clock.changed() => {
                    if audio_task.clock.anchor().is_none() {
                        scheduler.reset();
                    }
                }
                _ = reports.tick() => {
                    let stats = scheduler.take_stats();
                    tracing::info!(
                        view_generation = generation,
                        decoder_generation,
                        audio_master = clock_source.unwrap_or(false),
                        estimated_av_delta_us = ?last_delta_us,
                        selected_for_ui = stats.selected,
                        late_drops = stats.late,
                        superseded = stats.superseded,
                        capacity_drops = stats.capacity,
                        expired_drops = stats.expired,
                        nonmonotonic_drops = stats.nonmonotonic,
                        resets = stats.resets,
                        peak_queue = stats.peak_queue,
                        "video presentation interval"
                    );
                }
                update = catalog.next() => {
                    let Some(update) = update? else {
                        anyhow::bail!("remote screen catalog ended");
                    };
                    let next = Selection::from_catalog(update)?;
                    if next == selection {
                        continue;
                    }
                    let video_changed = next.name != selection.name
                        || next.config != selection.config
                        || next.display != selection.display
                        || next.quarter_turns != selection.quarter_turns
                        || next.flip != selection.flip;
                    tracing::info!(
                        view_generation = generation,
                        decoder_generation,
                        video_changed,
                        old_track = %selection.name,
                        new_track = %next.name,
                        old_display = ?selection.display,
                        new_display = ?next.display,
                        old_quarter_turns = selection.quarter_turns,
                        new_quarter_turns = next.quarter_turns,
                        old_flip = selection.flip,
                        new_flip = next.flip,
                        "remote screen catalog changed"
                    );
                    if next.audio != selection.audio {
                        scheduler.reset();
                        drop(std::mem::replace(
                            &mut audio_task,
                            audio::Task::spawn(
                                &broadcast,
                                &next.audio,
                                audio_events.clone(),
                                volume.clone(),
                            ),
                        ));
                    }
                    if !video_changed {
                        selection = next;
                        continue;
                    }
                    if let Some(task) = &mut video_task {
                        task.stop().await;
                    }
                    video_task = None;
                    let next_decoder = tokio::select! {
                        biased;
                        _ = wait_for_cancel(&mut cancel) => None,
                        decoder = next.decoder(&broadcast, video_max_age) => Some(decoder?),
                    };
                    let Some(next_decoder) = next_decoder else {
                        break Ok(());
                    };
                    selection = next;
                    decoder_name = next_decoder.name().to_owned();
                    scheduler.reset();
                    audio_sync_allowed = initial_audio;
                    decoder_generation = decoder_generation.saturating_add(1);
                    sequence = 0;
                    decoder_ready = false;
                    last_timestamp_us = None;
                    tracing::info!(
                        view_generation = generation,
                        decoder_generation,
                        decoder = %decoder_name,
                        track = %selection.name,
                        "remote video decoder rebuilt after catalog change"
                    );
                    video_task = Some(VideoTask::spawn(
                        decoder_generation,
                        next_decoder,
                        &video_updates_tx,
                    ));
                }
                update = video_updates_rx.recv() => {
                    let Some(update) = update else {
                        anyhow::bail!("remote video decoder task ended without a terminal update");
                    };
                    let Some(update) = accept_video_update(decoder_generation, update) else {
                        continue;
                    };
                    let decoded = match update {
                        VideoEvent::Frame(frame) => frame,
                        VideoEvent::Ended => anyhow::bail!("remote screen video track ended"),
                        VideoEvent::Failed(error) => anyhow::bail!(error),
                    };
                    sequence = sequence.saturating_add(1);
                    let timestamp_us = decoded.timestamp.as_micros();
                    if let Some(previous_timestamp_us) = last_timestamp_us
                        && timestamp_us < previous_timestamp_us
                    {
                        scheduler.reset();
                        audio_sync_allowed = false;
                        tracing::warn!(
                            view_generation = generation,
                            decoder_generation,
                            sequence,
                            previous_pts_us = %previous_timestamp_us,
                            frame_pts_us = %timestamp_us,
                            decoder = %decoder_name,
                            "decoded video PTS regressed; mux discontinuity is not exposed by moq-video"
                        );
                    }
                    let behind_high_water_us = view_high_water_timestamp_us
                        .map(|high_water: u128| high_water.saturating_sub(timestamp_us))
                        .unwrap_or(0);
                    if behind_high_water_us > 0
                        && (sequence == 1 || sequence.is_multiple_of(30))
                    {
                        tracing::warn!(
                            view_generation = generation,
                            decoder_generation,
                            sequence,
                            frame_pts_us = %timestamp_us,
                            view_high_water_pts_us = %view_high_water_timestamp_us.unwrap_or_default(),
                            behind_high_water_us = %behind_high_water_us,
                            decoder = %decoder_name,
                            "decoded video remains behind the view PTS high-water mark"
                        );
                    }
                    last_timestamp_us = Some(timestamp_us);
                    view_high_water_timestamp_us = Some(
                        view_high_water_timestamp_us
                            .map_or(timestamp_us, |high_water| high_water.max(timestamp_us)),
                    );
                    if sequence == 1 || sequence.is_multiple_of(300) {
                        tracing::info!(
                            view_generation = generation,
                            decoder_generation,
                            sequence,
                            frame_pts_us = %timestamp_us,
                            decoder = %decoder_name,
                            "decoded remote video frame"
                        );
                    }
                    let identity = PlaybackFrameIdentity {
                        view_generation: generation,
                        decoder_generation,
                        sequence,
                    };
                    scheduler.push(
                        std::time::Duration::from_micros(timestamp_us.min(u128::from(u64::MAX)) as u64),
                        PendingFrame {
                            decoded,
                            identity,
                            display: selection.display,
                            quarter_turns: selection.quarter_turns,
                            flip: selection.flip,
                        },
                        std::time::Instant::now(),
                    );
                }
            }
            }
        }
        .await;
        if let Some(mut task) = video_task.take() {
            task.stop().await;
        }
        loop_result
    }
    .await
    .map_err(|error: anyhow::Error| error.to_string());

    let _ = send_unless_cancelled(
        &mut cancel,
        &events,
        ViewEvent::Ended { generation, result },
    )
    .await;
}

#[cfg(not(target_os = "windows"))]
pub(crate) async fn run(
    generation: u64,
    _path: String,
    _broadcast: moq_tokio::moq_net::broadcast::Consumer,
    events: mpsc::Sender<ViewEvent>,
    _frames: watch::Sender<Option<Arc<PlaybackFrame>>>,
    _cancel: watch::Receiver<bool>,
    _volume: watch::Receiver<u8>,
) {
    let _ = events
        .send(ViewEvent::Ended {
            generation,
            result: Err("Windows playback is unavailable on this host".to_owned()),
        })
        .await;
}

#[cfg(target_os = "windows")]
impl PlaybackFrame {
    fn from_video(
        frame: moq_video::Frame,
        identity: PlaybackFrameIdentity,
        display: Option<(u32, u32)>,
        quarter_turns: u8,
        flip: bool,
    ) -> anyhow::Result<Self> {
        let width = frame.surface.width() as usize;
        let height = frame.surface.height() as usize;
        let timestamp_us = frame.timestamp.as_micros();
        anyhow::ensure!(
            width > 0 && height > 0 && width.is_multiple_of(2) && height.is_multiple_of(2),
            "remote I420 frame dimensions must be non-zero and even"
        );
        let i420 = frame.surface.into_i420()?;
        let pixels = width
            .checked_mul(height)
            .ok_or_else(|| anyhow::anyhow!("remote frame dimensions overflow"))?;
        anyhow::ensure!(
            i420.len() == pixels * 3 / 2,
            "remote I420 frame has an invalid length"
        );
        let mut rgba = Vec::with_capacity(pixels * 4);
        let u_offset = pixels;
        let v_offset = pixels + pixels / 4;
        for y in 0..height {
            for x in 0..width {
                let luma = i32::from(i420[y * width + x]) - 16;
                let chroma = (y / 2) * (width / 2) + x / 2;
                let u = i32::from(i420[u_offset + chroma]) - 128;
                let v = i32::from(i420[v_offset + chroma]) - 128;
                let red = (298 * luma + 409 * v + 128) >> 8;
                let green = (298 * luma - 100 * u - 208 * v + 128) >> 8;
                let blue = (298 * luma + 516 * u + 128) >> 8;
                rgba.extend_from_slice(&[
                    red.clamp(0, 255) as u8,
                    green.clamp(0, 255) as u8,
                    blue.clamp(0, 255) as u8,
                    255,
                ]);
            }
        }
        let (rgba, width, height) = orient_rgba(rgba, width, height, quarter_turns, flip);
        let (display_width, display_height) = display.unwrap_or((width as u32, height as u32));
        Ok(Self {
            identity,
            timestamp_us,
            width,
            height,
            display_width,
            display_height,
            rgba,
        })
    }
}

#[cfg(any(target_os = "windows", test))]
fn orient_rgba(
    source: Vec<u8>,
    width: usize,
    height: usize,
    quarter_turns: u8,
    flip: bool,
) -> (Vec<u8>, usize, usize) {
    let quarter_turns = quarter_turns % 4;
    if quarter_turns == 0 && !flip {
        return (source, width, height);
    }
    let (output_width, output_height) = if quarter_turns.is_multiple_of(2) {
        (width, height)
    } else {
        (height, width)
    };
    let mut output = vec![0; source.len()];
    for y in 0..height {
        for x in 0..width {
            let (mut output_x, output_y) = match quarter_turns {
                0 => (x, y),
                1 => (height - 1 - y, x),
                2 => (width - 1 - x, height - 1 - y),
                3 => (y, width - 1 - x),
                _ => unreachable!(),
            };
            if flip {
                output_x = output_width - 1 - output_x;
            }
            let source_offset = (y * width + x) * 4;
            let output_offset = (output_y * output_width + output_x) * 4;
            output[output_offset..output_offset + 4]
                .copy_from_slice(&source[source_offset..source_offset + 4]);
        }
    }
    (output, output_width, output_height)
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use super::*;

    struct InFlightRead {
        completed: bool,
        canceled: Arc<AtomicBool>,
    }

    impl Drop for InFlightRead {
        fn drop(&mut self) {
            if !self.completed {
                self.canceled.store(true, Ordering::SeqCst);
            }
        }
    }

    struct ControlledReader {
        next: u8,
        last: u8,
        started: mpsc::Sender<u8>,
        release: mpsc::Receiver<()>,
        canceled: Arc<AtomicBool>,
    }

    impl VideoReader for ControlledReader {
        type Frame = u8;

        async fn read(&mut self) -> Result<Option<Self::Frame>, String> {
            if self.next > self.last {
                return Ok(None);
            }
            let event = self.next;
            self.next = self.next.saturating_add(1);
            let mut read = InFlightRead {
                completed: false,
                canceled: self.canceled.clone(),
            };
            self.started
                .send(event)
                .await
                .map_err(|_| "read observer closed".to_owned())?;
            self.release
                .recv()
                .await
                .ok_or_else(|| "read release closed".to_owned())?;
            read.completed = true;
            Ok(Some(event))
        }
    }

    #[test]
    fn audio_video_share_80ms_and_video_only_skips_stale_groups() {
        assert_eq!(video_max_age(true), std::time::Duration::from_millis(80));
        assert_eq!(video_max_age(false), std::time::Duration::ZERO);
    }

    #[tokio::test]
    async fn control_events_do_not_cancel_continuous_owned_reads() {
        let canceled = Arc::new(AtomicBool::new(false));
        let observed = canceled.clone();
        let (started_tx, mut started_rx) = mpsc::channel(1);
        let (release_tx, release_rx) = mpsc::channel(1);
        let (updates_tx, mut updates_rx) = mpsc::channel(1);
        let mut task = VideoTask::spawn(
            7,
            ControlledReader {
                next: 41,
                last: 42,
                started: started_tx,
                release: release_rx,
                canceled: observed,
            },
            &updates_tx,
        );

        let (control_tx, mut control_rx) = mpsc::channel(1);
        for expected in [41_u8, 42_u8] {
            assert_eq!(started_rx.recv().await, Some(expected));
            control_tx.send(()).await.expect("control event queued");
            tokio::select! {
                event = control_rx.recv() => assert_eq!(event, Some(())),
                update = updates_rx.recv() => panic!("read completed before release: {update:?}"),
            }
            assert!(!canceled.load(Ordering::SeqCst));

            release_tx.send(()).await.expect("release owned read");
            let update = tokio::time::timeout(std::time::Duration::from_secs(1), updates_rx.recv())
                .await
                .expect("owned read completion timed out")
                .expect("owned read update channel closed");
            assert!(matches!(
                accept_video_update(7, update),
                Some(VideoEvent::Frame(event)) if event == expected
            ));
            assert!(!canceled.load(Ordering::SeqCst));
        }

        task.stop().await;
    }

    #[tokio::test]
    async fn stopping_an_owned_read_awaits_teardown_and_replacement_is_generation_scoped() {
        let canceled = Arc::new(AtomicBool::new(false));
        let observed = canceled.clone();
        let (started_tx, mut started_rx) = mpsc::channel(1);
        let (_release_tx, release_rx) = mpsc::channel(1);
        let (old_updates_tx, _old_updates_rx) = mpsc::channel(1);
        let mut task = VideoTask::spawn(
            7,
            ControlledReader {
                next: 7,
                last: 7,
                started: started_tx,
                release: release_rx,
                canceled: observed,
            },
            &old_updates_tx,
        );
        assert_eq!(started_rx.recv().await, Some(7));

        task.stop().await;
        assert!(canceled.load(Ordering::SeqCst));

        let (updates_tx, mut updates_rx) = mpsc::channel(2);
        updates_tx
            .send(VideoUpdate {
                generation: 7,
                event: VideoEvent::Frame(7_u8),
            })
            .await
            .expect("queue stale completion");
        let (replacement_started_tx, mut replacement_started_rx) = mpsc::channel(1);
        let (replacement_release_tx, replacement_release_rx) = mpsc::channel(1);
        let replacement_canceled = Arc::new(AtomicBool::new(false));
        let mut replacement = VideoTask::spawn(
            8,
            ControlledReader {
                next: 8,
                last: 8,
                started: replacement_started_tx,
                release: replacement_release_rx,
                canceled: replacement_canceled.clone(),
            },
            &updates_tx,
        );
        assert_eq!(replacement_started_rx.recv().await, Some(8));
        replacement_release_tx
            .send(())
            .await
            .expect("release replacement read");

        let stale = updates_rx.recv().await.expect("stale completion queued");
        assert!(accept_video_update(8, stale).is_none());
        let current = updates_rx
            .recv()
            .await
            .expect("replacement completion queued");
        assert!(matches!(
            accept_video_update(8, current),
            Some(VideoEvent::Frame(8))
        ));
        assert!(!replacement_canceled.load(Ordering::SeqCst));

        replacement.stop().await;
    }

    #[tokio::test]
    async fn cancellation_releases_a_terminal_notification_blocked_by_a_full_queue() {
        let (events, _events_rx) = mpsc::channel(1);
        events.send(1_u8).await.expect("fill event queue");
        let (cancel, mut canceled) = watch::channel(false);
        let mut send = Box::pin(send_unless_cancelled(&mut canceled, &events, 2_u8));
        std::future::poll_fn(|context| {
            assert!(send.as_mut().poll(context).is_pending());
            std::task::Poll::Ready(())
        })
        .await;

        cancel.send_replace(true);
        let sent = tokio::time::timeout(std::time::Duration::from_secs(1), send)
            .await
            .expect("terminal notification remained blocked");

        assert!(!sent);
    }

    #[test]
    fn view_becomes_active_only_after_a_current_decoder_frame() {
        let mut view = ViewSnapshot::default();
        let generation = view.begin("moqcast.screen/peer-a").expect("begin");
        assert_eq!(view.phase, ViewPhase::Preparing);
        assert!(!view.decoder_ready(
            generation + 1,
            "moqcast.screen/peer-a",
            "mediafoundation".to_owned(),
            1920,
            1080,
        ));
        assert!(view.decoder_ready(
            generation,
            "moqcast.screen/peer-a",
            "mediafoundation".to_owned(),
            1920,
            1080,
        ));
        assert_eq!(view.phase, ViewPhase::Viewing);
    }

    #[test]
    fn stale_view_and_decoder_frames_have_distinct_identities() {
        let first = PlaybackFrameIdentity {
            view_generation: 1,
            decoder_generation: 1,
            sequence: 7,
        };
        let replaced_decoder = PlaybackFrameIdentity {
            decoder_generation: 2,
            ..first
        };
        let next_view = PlaybackFrameIdentity {
            view_generation: 2,
            ..first
        };
        assert_ne!(first, replaced_decoder);
        assert_ne!(first, next_view);
        assert!(first < replaced_decoder);
        assert!(first < next_view);
    }

    #[test]
    fn stopping_and_failures_ignore_old_generations() {
        let mut view = ViewSnapshot::default();
        let old = view.begin("moqcast.screen/peer-a").expect("old");
        view.phase = ViewPhase::Failed;
        let current = view.begin("moqcast.screen/peer-a").expect("current");
        assert!(current > old);
        assert!(!view.ended(old, Err("late".to_owned())));
        assert_eq!(view.phase, ViewPhase::Preparing);
        assert_eq!(view.begin_stop(), Some(current));
        assert!(view.stopped(current));
        assert!(!view.ended(current, Err("late".to_owned())));
        assert_eq!(view.phase, ViewPhase::Idle);
        assert_eq!(view.audio.phase, ViewAudioPhase::Idle);
    }

    #[test]
    fn audio_state_is_generation_scoped_and_does_not_end_video() {
        let mut view = ViewSnapshot::default();
        let generation = view.begin("moqcast.screen/peer-a").expect("begin");
        assert!(view.decoder_ready(
            generation,
            "moqcast.screen/peer-a",
            "mediafoundation".to_owned(),
            1920,
            1080,
        ));
        assert!(!view.audio_changed(
            generation + 1,
            "moqcast.screen/peer-a",
            ViewAudioSnapshot {
                phase: ViewAudioPhase::Failed,
                ..ViewAudioSnapshot::default()
            },
        ));
        assert!(view.audio_changed(
            generation,
            "moqcast.screen/peer-a",
            ViewAudioSnapshot {
                phase: ViewAudioPhase::CallbackConsumed,
                codec: Some("opus".to_owned()),
                sample_rate: Some(48_000),
                channels: Some(2),
                last_error: None,
            },
        ));
        assert_eq!(view.phase, ViewPhase::Viewing);
        assert_eq!(view.audio.phase, ViewAudioPhase::CallbackConsumed);
    }

    #[test]
    fn audio_stats_distinguish_pcm_writes_and_callback_evidence() {
        let mut stats = AudioStats::default();
        let silence = vec![0_u8; 480 * 2 * std::mem::size_of::<f32>()];
        let mut nonzero = silence.clone();
        nonzero[..4].copy_from_slice(&0.25_f32.to_ne_bytes());

        assert_eq!(
            stats.decoded(10_000, silence.len(), 10_000, pcm_has_nonzero_f32(&silence)),
            (true, false)
        );
        assert!(stats.wrote());
        assert_eq!(
            stats.decoded(21_000, nonzero.len(), 10_000, pcm_has_nonzero_f32(&nonzero),),
            (false, true)
        );
        assert!(!stats.wrote());
        stats.write_failed();

        let report = stats.take_report();
        assert_eq!(report.decoded_frames, 2);
        assert_eq!(report.nonzero_pcm_frames, 1);
        assert_eq!(report.sink_writes, 2);
        assert_eq!(report.sink_write_errors, 1);
        assert_eq!(report.first_pts_us, Some(10_000));
        assert_eq!(report.last_pts_us, Some(21_000));
        assert_eq!(report.pts_gaps, 1);
        assert_eq!(report.max_pts_gap_us, 1_000);
        assert!(!callback_consumed_nonzero(0.0));
        assert!(callback_consumed_nonzero(0.25));
        assert!(!callback_consumed_nonzero(f32::NAN));

        assert_eq!(
            stats.decoded(20_000, nonzero.len(), 10_000, true),
            (false, false)
        );
        assert!(!stats.wrote());
        let next = stats.take_report();
        assert_eq!(next.pts_regressions, 1);
        assert_eq!(next.decoded_frames, 1);
    }

    #[test]
    fn rotation_and_flip_transform_pixels_without_changing_identity() {
        let pixels = vec![1, 0, 0, 255, 2, 0, 0, 255, 3, 0, 0, 255, 4, 0, 0, 255];
        let (rotated, width, height) = orient_rgba(pixels.clone(), 2, 2, 1, false);
        assert_eq!((width, height), (2, 2));
        assert_eq!(rotated[0], 3);
        assert_eq!(rotated[4], 1);
        assert_eq!(rotated[8], 4);
        assert_eq!(rotated[12], 2);

        let (flipped, _, _) = orient_rgba(pixels, 2, 2, 0, true);
        assert_eq!(flipped[0], 2);
        assert_eq!(flipped[4], 1);
        assert_eq!(flipped[8], 4);
        assert_eq!(flipped[12], 3);
    }
}
