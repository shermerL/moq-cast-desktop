//! Catalog-driven H.264 playback and decoded-frame delivery.

use std::sync::Arc;

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
    Ended {
        generation: u64,
        result: Result<(), String>,
    },
}

#[derive(Default)]
pub(crate) struct Owner {
    task: Option<JoinHandle<()>>,
}

impl Owner {
    pub(crate) fn start(
        &mut self,
        generation: u64,
        broadcast: moq_tokio::moq_net::broadcast::Consumer,
        events: mpsc::Sender<Event>,
        frames: watch::Sender<Option<Arc<Frame>>>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) {
        debug_assert!(self.task.is_none());
        self.task = Some(tokio::spawn(run(
            generation, broadcast, events, frames, wake,
        )));
    }

    pub(crate) async fn stop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
            let _ = task.await;
        }
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
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
struct Selection {
    name: String,
    config: Box<hang::catalog::VideoConfig>,
    identity: VideoIdentity,
}

impl PartialEq for Selection {
    fn eq(&self, other: &Self) -> bool {
        self.identity == other.identity
    }
}

impl Selection {
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

struct VideoUpdate {
    generation: u64,
    event: VideoEvent,
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

async fn run(
    generation: u64,
    broadcast: moq_tokio::moq_net::broadcast::Consumer,
    events: mpsc::Sender<Event>,
    frames: watch::Sender<Option<Arc<Frame>>>,
    wake: Arc<dyn Fn() + Send + Sync>,
) {
    let result = run_inner(generation, broadcast, &events, &frames, &wake)
        .await
        .map_err(|error| error.to_string());
    let _ = events.send(Event::Ended { generation, result }).await;
}

async fn run_inner(
    generation: u64,
    broadcast: moq_tokio::moq_net::broadcast::Consumer,
    events: &mpsc::Sender<Event>,
    frames: &watch::Sender<Option<Arc<Frame>>>,
    wake: &Arc<dyn Fn() + Send + Sync>,
) -> anyhow::Result<()> {
    let mut catalog =
        moq_mux::catalog::Consumer::<()>::new(&broadcast, moq_mux::catalog::CatalogFormat::Hang)
            .await?;
    let first = catalog
        .next()
        .await?
        .ok_or_else(|| anyhow::anyhow!("remote screen catalog ended"))?;
    let mut selection = Selection::from_catalog(first.video, None)?;
    let mut sequence = FrameSequence::new(generation);
    let (video_tx, mut video_rx) = mpsc::channel(1);
    let mut decoder_name = None;
    let mut video_task = None;

    if let Some(video) = &selection {
        sequence.replace_decoder();
        let decoder = video.decoder(&broadcast).await?;
        decoder_name = Some(decoder.name().to_owned());
        tracing::info!(
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
    }

    loop {
        tokio::select! {
            biased;
            update = catalog.next() => {
                let Some(update) = update? else {
                    anyhow::bail!("remote screen catalog ended");
                };
                let preferred = selection.as_ref().map(|video| video.name.as_str());
                let next = Selection::from_catalog(update.video, preferred)?;
                if next == selection && !(next.is_some() && video_task.is_none()) {
                    continue;
                }
                if let Some(mut task) = video_task.take() {
                    task.stop().await;
                }
                sequence.replace_decoder();
                decoder_name = None;
                if let Some(video) = &next {
                    let decoder = video.decoder(&broadcast).await?;
                    decoder_name = Some(decoder.name().to_owned());
                    tracing::info!(
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
                }
                selection = next;
            }
            update = video_rx.recv(), if video_task.is_some() => {
                let Some(update) = update else {
                    anyhow::bail!("remote video reader stopped");
                };
                if update.generation != sequence.decoder_generation {
                    continue;
                }
                match update.event {
                    VideoEvent::Frame(decoded) => {
                        let selected = selection.as_ref().expect("active decoder has a selection");
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
                                    decoder: decoder_name.clone().expect("active decoder has a name"),
                                    width,
                                    height,
                                })
                                .await
                                .map_err(|_| anyhow::anyhow!("playback event receiver closed"))?;
                        }
                    }
                    VideoEvent::Ended => anyhow::bail!("remote screen video track ended"),
                    VideoEvent::Failed(error) => anyhow::bail!(error),
                }
            }
        }
    }
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
}
