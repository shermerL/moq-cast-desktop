//! Remote screen playback lifecycle and decoded-frame delivery.

#[cfg(target_os = "windows")]
mod audio;

use std::sync::Arc;

use tokio::sync::{mpsc, watch};

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
    NotPublished,
    Playing,
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
        broadcast: &moq_native::moq_net::broadcast::Consumer,
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

#[cfg(target_os = "windows")]
pub(crate) async fn run(
    generation: u64,
    path: String,
    broadcast: moq_native::moq_net::broadcast::Consumer,
    events: mpsc::Sender<ViewEvent>,
    frames: watch::Sender<Option<Arc<PlaybackFrame>>>,
) {
    use moq_mux::catalog::Stream;

    let result = async {
        let mut catalog = moq_mux::catalog::Consumer::<()>::new(
            &broadcast,
            moq_mux::catalog::CatalogFormat::Hang,
        )
        .await?;
        let first = catalog
            .next()
            .await?
            .ok_or_else(|| anyhow::anyhow!("remote screen catalog ended"))?;
        let mut selection = Selection::from_catalog(first)?;
        let mut decoder = selection.decoder(&broadcast).await?;
        let mut audio_task =
            audio::Task::spawn(generation, &path, &broadcast, &selection.audio, &events);
        let mut decoder_generation = 1_u64;
        let mut sequence = 0_u64;
        let mut decoder_ready = false;
        let mut last_timestamp_us = None;
        tracing::info!(
            view_generation = generation,
            decoder_generation,
            decoder = decoder.name(),
            track = %selection.name,
            "remote video decoder opened"
        );

        loop {
            tokio::select! {
                biased;
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
                        drop(std::mem::replace(
                            &mut audio_task,
                            audio::Task::spawn(
                                generation,
                                &path,
                                &broadcast,
                                &next.audio,
                                &events,
                            ),
                        ));
                    }
                    if !video_changed {
                        selection = next;
                        continue;
                    }
                    let next_decoder = next.decoder(&broadcast).await?;
                    selection = next;
                    decoder = next_decoder;
                    decoder_generation = decoder_generation.saturating_add(1);
                    sequence = 0;
                    decoder_ready = false;
                    last_timestamp_us = None;
                    tracing::info!(
                        view_generation = generation,
                        decoder_generation,
                        decoder = decoder.name(),
                        track = %selection.name,
                        "remote video decoder rebuilt after catalog change"
                    );
                }
                decoded = decoder.read() => {
                    let Some(decoded) = decoded? else {
                        anyhow::bail!("remote screen video track ended");
                    };
                    sequence = sequence.saturating_add(1);
                    let timestamp_us = decoded.timestamp.as_micros();
                    if let Some(previous_timestamp_us) = last_timestamp_us
                        && timestamp_us < previous_timestamp_us
                    {
                        tracing::warn!(
                            view_generation = generation,
                            decoder_generation,
                            sequence,
                            previous_pts_us = %previous_timestamp_us,
                            frame_pts_us = %timestamp_us,
                            decoder = decoder.name(),
                            "decoded video PTS regressed; mux discontinuity is not exposed by moq-video"
                        );
                    }
                    last_timestamp_us = Some(timestamp_us);
                    if sequence == 1 || sequence.is_multiple_of(300) {
                        tracing::info!(
                            view_generation = generation,
                            decoder_generation,
                            sequence,
                            frame_pts_us = %timestamp_us,
                            decoder = decoder.name(),
                            "decoded remote video frame"
                        );
                    }
                    let identity = PlaybackFrameIdentity {
                        view_generation: generation,
                        decoder_generation,
                        sequence,
                    };
                    let display = selection.display;
                    let quarter_turns = selection.quarter_turns;
                    let flip = selection.flip;
                    let frame = tokio::task::spawn_blocking(move || {
                        PlaybackFrame::from_video(decoded, identity, display, quarter_turns, flip)
                    })
                    .await??;
                    let width = frame.display_width;
                    let height = frame.display_height;
                    frames.send_replace(Some(Arc::new(frame)));
                    if !decoder_ready {
                        decoder_ready = true;
                        let _ = events
                            .send(ViewEvent::DecoderReady {
                                generation,
                                path: path.clone(),
                                decoder: decoder.name().to_owned(),
                                width,
                                height,
                            })
                            .await;
                        }
                    }
            }
        }
    }
    .await
    .map_err(|error: anyhow::Error| error.to_string());

    let _ = events.send(ViewEvent::Ended { generation, result }).await;
}

#[cfg(not(target_os = "windows"))]
pub(crate) async fn run(
    generation: u64,
    _path: String,
    _broadcast: moq_native::moq_net::broadcast::Consumer,
    events: mpsc::Sender<ViewEvent>,
    _frames: watch::Sender<Option<Arc<PlaybackFrame>>>,
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
    use super::*;

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
                phase: ViewAudioPhase::Playing,
                codec: Some("opus".to_owned()),
                sample_rate: Some(48_000),
                channels: Some(2),
                last_error: None,
            },
        ));
        assert_eq!(view.phase, ViewPhase::Viewing);
        assert_eq!(view.audio.phase, ViewAudioPhase::Playing);
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
