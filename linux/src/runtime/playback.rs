//! Linux remote-screen playback ownership and catalog-driven media replacement.

mod audio;

use std::sync::Arc;
use std::time::Instant;

use moq_mux::catalog::Stream;
use moq_tokio::moq_net;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;

use crate::app::RemoteAudioSnapshot;

use super::playback_sync as sync;
use super::{PlaybackFrame, PlaybackFrameIdentity};

const AUDIO_EVENT_CAPACITY: usize = 8;
const VIDEO_EVENT_CAPACITY: usize = 1;

/// Events emitted by one playback owner to the runtime supervisor.
pub(super) enum Event {
    /// The first renderable video frame is available.
    Started { ack: oneshot::Sender<()> },
    /// Remote audio progressed without changing the video lifecycle.
    Audio(RemoteAudioSnapshot),
}

#[derive(Clone, Debug, PartialEq)]
struct VideoIdentity {
    track: String,
    broadcast: Option<moq_net::PathRelativeOwned>,
    codec: hang::catalog::VideoCodec,
    description: Option<Vec<u8>>,
    container: hang::catalog::Container,
    coded_width: Option<u32>,
    coded_height: Option<u32>,
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
    fn from_catalog(mut video: hang::catalog::Video, preferred: Option<&str>) -> Option<Self> {
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
        let (name, config) = selected?;
        let identity = VideoIdentity {
            track: name.clone(),
            broadcast: config.broadcast.clone(),
            codec: config.codec.clone(),
            description: config.description.as_deref().map(<[u8]>::to_vec),
            container: config.container.clone(),
            coded_width: config.coded_width,
            coded_height: config.coded_height,
        };
        Some(Self {
            name,
            config: Box::new(config),
            identity,
        })
    }

    async fn decoder(
        &self,
        broadcast: &moq_net::broadcast::Consumer,
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

#[derive(Clone, Debug, PartialEq)]
struct Selection {
    video: Option<VideoSelection>,
    audio: audio::Selection,
}

impl Selection {
    fn from_catalog(catalog: moq_mux::catalog::hang::Catalog<()>, current: Option<&Self>) -> Self {
        Self {
            video: VideoSelection::from_catalog(
                catalog.video,
                current.and_then(|selection| {
                    selection.video.as_ref().map(|video| video.name.as_str())
                }),
            ),
            audio: audio::Selection::from_catalog(
                catalog.audio,
                current.and_then(|selection| selection.audio.name()),
            ),
        }
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

    fn next(&mut self) -> (PlaybackFrameIdentity, bool) {
        self.sequence = self.sequence.wrapping_add(1);
        let first_view_frame = !self.started;
        self.started = true;
        (
            PlaybackFrameIdentity {
                view_generation: self.view_generation,
                decoder_generation: self.decoder_generation,
                sequence: self.sequence,
            },
            first_view_frame,
        )
    }
}

fn accept_audio_update(
    current_generation: u64,
    update: audio::Update,
) -> Option<RemoteAudioSnapshot> {
    (update.generation == current_generation).then_some(update.snapshot)
}

enum VideoEvent {
    Frame(moq_video::Frame),
    Ended,
    Failed(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VideoEventDisposition {
    Frame,
    WaitForCatalog,
    FailOwner,
}

fn video_event_disposition(event: &VideoEvent) -> VideoEventDisposition {
    match event {
        VideoEvent::Frame(_) => VideoEventDisposition::Frame,
        VideoEvent::Ended => VideoEventDisposition::WaitForCatalog,
        VideoEvent::Failed(_) => VideoEventDisposition::FailOwner,
    }
}

struct VideoUpdate {
    generation: u64,
    event: VideoEvent,
}

fn accept_video_update(current_generation: u64, update: VideoUpdate) -> Option<VideoEvent> {
    (update.generation == current_generation).then_some(update.event)
}

struct VideoTask {
    handle: Option<JoinHandle<()>>,
}

impl VideoTask {
    fn spawn(
        generation: u64,
        mut decoder: moq_video::decode::Consumer,
        updates: &mpsc::Sender<VideoUpdate>,
    ) -> Self {
        let updates = updates.clone();
        let handle = tokio::spawn(async move {
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

impl Drop for VideoTask {
    fn drop(&mut self) {
        if let Some(handle) = &self.handle {
            handle.abort();
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
        None => std::future::pending::<()>().await,
    }
}

/// Consume one screen broadcast until its catalog or video track ends.
pub(super) async fn run(
    view_generation: u64,
    path: String,
    broadcast: moq_net::broadcast::Consumer,
    mut cancel: watch::Receiver<bool>,
    events: mpsc::Sender<Event>,
    frames: watch::Sender<Option<Arc<PlaybackFrame>>>,
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
    let mut selection = Selection::from_catalog(first, None);
    let mut frames_sequence = FrameSequence::new(view_generation);
    let mut video_scheduler = sync::VideoScheduler::default();
    let (video_updates_tx, mut video_updates_rx) = mpsc::channel(VIDEO_EVENT_CAPACITY);
    let mut video_task = None;
    if let Some(video) = &selection.video {
        frames_sequence.replace_decoder();
        let decoder = tokio::select! {
            biased;
            _ = wait_for_cancel(&mut cancel) => return Ok(()),
            result = video.decoder(&broadcast) => result?,
        };
        tracing::info!(
            view_generation,
            decoder_generation = frames_sequence.decoder_generation,
            decoder = decoder.name(),
            track = %video.name,
            "remote video decoder opened"
        );
        video_task = Some(VideoTask::spawn(
            frames_sequence.decoder_generation,
            decoder,
            &video_updates_tx,
        ));
    } else {
        tracing::debug!(
            view_generation,
            "waiting for a playable remote video rendition"
        );
    }
    let (audio_updates_tx, mut audio_updates_rx) = mpsc::channel(AUDIO_EVENT_CAPACITY);
    let audio_engine = Arc::new(tokio::sync::OnceCell::new());
    let media_clock = Arc::new(sync::MediaClock::default());
    let mut audio_generation = 1_u64;
    let mut audio_task = audio::Task::spawn(
        audio_generation,
        &path,
        &broadcast,
        &selection.audio,
        &audio_updates_tx,
        &audio_engine,
        &media_clock,
    );

    let result = async {
        loop {
            let advance = video_scheduler.advance(media_clock.audio_anchor(), Instant::now());
            if let Some(decoded) = advance.due {
                let (identity, first_view_frame) = frames_sequence.next();
                let frame = tokio::task::spawn_blocking(move || {
                    PlaybackFrame::from_video(decoded, identity)
                })
                .await??;
                frames.send_replace(Some(Arc::new(frame)));
                if first_view_frame {
                    let (ack, ready) = oneshot::channel();
                    events
                        .send(Event::Started { ack })
                        .await
                        .map_err(|_| anyhow::anyhow!("playback event receiver closed"))?;
                    ready
                        .await
                        .map_err(|_| anyhow::anyhow!("playback start acknowledgement closed"))?;
                }
            }
            tokio::select! {
                biased;
                _ = wait_for_cancel(&mut cancel) => {
                    tracing::debug!(view_generation, "remote playback cancellation received");
                    break Ok(());
                }
                _ = wait_for_deadline(advance.deadline), if advance.deadline.is_some() => {}
                _ = media_clock.changed() => {}
                update = catalog.next() => {
                    let Some(update) = update? else {
                        anyhow::bail!("remote screen catalog ended");
                    };
                    let next = Selection::from_catalog(update, Some(&selection));
                    let changes = media_changes(&selection, &next, video_task.is_some());
                    if !changes.video && !changes.audio {
                        selection = next;
                        continue;
                    }

                    tracing::info!(
                        view_generation,
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
                        video_scheduler.reset_fallback();
                        audio_task.stop().await;
                        audio_generation = audio_generation.wrapping_add(1);
                        audio_task = audio::Task::spawn(
                            audio_generation,
                            &path,
                            &broadcast,
                            &next.audio,
                            &audio_updates_tx,
                            &audio_engine,
                            &media_clock,
                        );
                    }
                    if changes.video {
                        // The reader owns the non-cancel-safe read future and its
                        // decoder. Await its teardown before opening a replacement.
                        if let Some(mut task) = video_task.take() {
                            task.stop().await;
                        }
                        frames_sequence.replace_decoder();
                        video_scheduler.reset();
                        if let Some(video) = &next.video {
                            let decoder = tokio::select! {
                                biased;
                                _ = wait_for_cancel(&mut cancel) => break Ok(()),
                                result = video.decoder(&broadcast) => result?,
                            };
                            tracing::info!(
                                view_generation,
                                decoder_generation = frames_sequence.decoder_generation,
                                decoder = decoder.name(),
                                track = %video.name,
                                "remote video decoder rebuilt after catalog change"
                            );
                            video_task = Some(VideoTask::spawn(
                                frames_sequence.decoder_generation,
                                decoder,
                                &video_updates_tx,
                            ));
                        } else {
                            tracing::debug!(
                                view_generation,
                                decoder_generation = frames_sequence.decoder_generation,
                                "remote video rendition withdrawn; waiting for catalog replacement"
                            );
                        }
                    }
                    selection = next;
                }
                update = audio_updates_rx.recv() => {
                    let Some(update) = update else {
                        anyhow::bail!("remote audio event channel closed");
                    };
                    if let Some(snapshot) = accept_audio_update(audio_generation, update) {
                        events
                            .send(Event::Audio(snapshot))
                            .await
                            .map_err(|_| anyhow::anyhow!("playback event receiver closed"))?;
                    }
                }
                update = video_updates_rx.recv(), if video_scheduler.has_capacity() => {
                    let Some(update) = update else {
                        anyhow::bail!("remote video event channel closed");
                    };
                    let Some(event) = accept_video_update(
                        frames_sequence.decoder_generation,
                        update,
                    ) else {
                        continue;
                    };
                    match video_event_disposition(&event) {
                        VideoEventDisposition::Frame => {
                            let VideoEvent::Frame(decoded) = event else {
                                unreachable!("frame disposition carries a frame")
                            };
                            let _ =
                                video_scheduler.push(sync::timestamp(decoded.timestamp), decoded);
                        }
                        VideoEventDisposition::WaitForCatalog => {
                            if let Some(mut task) = video_task.take() {
                                task.stop().await;
                            }
                            frames_sequence.replace_decoder();
                            video_scheduler.reset();
                            tracing::debug!(
                                view_generation,
                                decoder_generation = frames_sequence.decoder_generation,
                                "remote video track ended; waiting for catalog replacement"
                            );
                        }
                        VideoEventDisposition::FailOwner => {
                            let VideoEvent::Failed(error) = event else {
                                unreachable!("failure disposition carries an error")
                            };
                            break Err(anyhow::anyhow!(error));
                        }
                    }
                }
            }
        }
    }
    .await;

    if let Some(mut task) = video_task {
        task.stop().await;
    }
    audio_task.stop().await;
    drop(audio_engine);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selection() -> Selection {
        let mut catalog = moq_mux::catalog::hang::Catalog::<()>::default();
        catalog.video.renditions.insert("screen".into(), h264(0x1f));
        Selection::from_catalog(catalog, None)
    }

    fn h264(level: u8) -> hang::catalog::VideoConfig {
        hang::catalog::VideoConfig::new(hang::catalog::H264 {
            inline: true,
            profile: 0x42,
            constraints: 0xc0,
            level,
        })
    }

    #[test]
    fn identical_catalog_selection_does_not_replace_media() {
        let selection = selection();

        assert_eq!(
            media_changes(&selection, &selection.clone(), true),
            MediaChanges {
                video: false,
                audio: false,
            }
        );
    }

    #[test]
    fn current_supported_video_is_retained_when_another_rendition_appears() {
        let mut initial = hang::catalog::Video::default();
        initial.renditions.insert("screen-b".into(), h264(0x1f));
        let current = VideoSelection::from_catalog(initial, None).expect("initial selection");

        let mut updated = hang::catalog::Video::default();
        updated.renditions.insert("screen-a".into(), h264(0x1e));
        updated
            .renditions
            .insert("screen-b".into(), (*current.config).clone());
        let retained = VideoSelection::from_catalog(updated, Some(&current.name))
            .expect("preferred selection");

        assert_eq!(retained, current);
    }

    #[test]
    fn video_estimates_do_not_change_decoder_identity() {
        let mut current = hang::catalog::Video::default();
        current.renditions.insert("screen".into(), h264(0x1f));
        let selected = VideoSelection::from_catalog(current.clone(), None).expect("selection");

        let config = current.renditions.get_mut("screen").expect("video config");
        config.bitrate = Some(8_000_000);
        config.framerate = Some(60.0);
        config.jitter = Some(std::time::Duration::from_millis(40));
        let updated = VideoSelection::from_catalog(current, Some("screen")).expect("selection");

        assert_eq!(selected, updated);
    }

    #[test]
    fn audio_only_catalog_change_does_not_replace_video() {
        let mut catalog = moq_mux::catalog::hang::Catalog::<()>::default();
        catalog.video.renditions.insert("screen".into(), h264(0x1f));
        let current = Selection::from_catalog(catalog.clone(), None);
        catalog.audio.renditions.insert(
            "audio".into(),
            hang::catalog::AudioConfig::new(hang::catalog::AudioCodec::Opus, 48_000, 2),
        );
        let next = Selection::from_catalog(catalog, Some(&current));

        assert_eq!(
            media_changes(&current, &next, true),
            MediaChanges {
                video: false,
                audio: true,
            }
        );
    }

    #[test]
    fn video_withdrawal_waits_for_a_catalog_replacement() {
        let current = selection();
        let withdrawn = Selection {
            video: None,
            audio: current.audio.clone(),
        };
        let replacement = selection();

        assert!(media_changes(&current, &withdrawn, true).video);
        assert!(media_changes(&withdrawn, &replacement, false).video);
    }

    #[test]
    fn ended_video_reader_keeps_the_catalog_owner_alive() {
        let current = selection();

        assert_eq!(
            video_event_disposition(&VideoEvent::Ended),
            VideoEventDisposition::WaitForCatalog
        );
        assert!(media_changes(&current, &current.clone(), false).video);
    }

    #[test]
    fn stale_video_generation_event_is_ignored() {
        let stale = VideoUpdate {
            generation: 4,
            event: VideoEvent::Ended,
        };
        let current = VideoUpdate {
            generation: 5,
            event: VideoEvent::Ended,
        };

        assert!(accept_video_update(5, stale).is_none());
        assert!(matches!(
            accept_video_update(5, current),
            Some(VideoEvent::Ended)
        ));
    }

    #[test]
    fn video_decoder_generation_isolates_equal_sequences() {
        let mut frames = FrameSequence::new(7);
        let (before, _) = frames.next();
        frames.replace_decoder();
        let (after, _) = frames.next();

        assert_eq!(before.sequence, after.sequence);
        assert_ne!(before, after);
    }

    #[test]
    fn replacement_before_first_frame_starts_only_on_new_frame() {
        let mut frames = FrameSequence::new(7);
        frames.replace_decoder();
        let (_, starts_view) = frames.next();

        assert!(starts_view);
    }

    #[test]
    fn replacement_keeps_the_last_published_frame() {
        let mut frames = FrameSequence::new(7);
        let (published, _) = frames.next();
        let (tx, rx) = watch::channel(Some(published));
        frames.replace_decoder();

        assert_eq!(*rx.borrow(), Some(published));
        drop(tx);
    }

    #[test]
    fn stale_audio_generation_is_ignored() {
        let update = audio::Update {
            generation: 4,
            snapshot: RemoteAudioSnapshot::default(),
        };

        assert!(accept_audio_update(5, update).is_none());
    }
}
