//! Linux remote-screen playback ownership and catalog-driven decoder replacement.

use std::collections::BTreeMap;
use std::sync::Arc;

use moq_mux::catalog::Stream;
use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;

use super::{PlaybackFrame, PlaybackFrameIdentity};

#[derive(Clone, Debug, PartialEq)]
struct Selection {
    name: String,
    catalog: moq_mux::catalog::hang::Catalog<()>,
}

impl Selection {
    fn from_snapshot(
        mut snapshot: moq_mux::catalog::hang::Catalog<()>,
        preferred: Option<&str>,
    ) -> anyhow::Result<Option<Self>> {
        let Some(name) = preferred
            .and_then(|name| preferred_name(name, &snapshot.video.renditions))
            .or_else(|| {
                snapshot
                    .video
                    .renditions
                    .first_key_value()
                    .map(|(name, _)| name.clone())
            })
        else {
            return Ok(None);
        };
        let config = snapshot
            .video
            .renditions
            .remove(&name)
            .expect("selected rendition exists in the snapshot");
        anyhow::ensure!(
            config.broadcast.is_none(),
            "external rendition broadcasts are not supported yet"
        );
        let mut catalog = moq_mux::catalog::hang::Catalog::default();
        catalog.video.display = snapshot.video.display;
        catalog.video.rotation = snapshot.video.rotation;
        catalog.video.flip = snapshot.video.flip;
        catalog.video.renditions.insert(name.clone(), config);
        Ok(Some(Self { name, catalog }))
    }
}

#[derive(Clone, Debug, PartialEq)]
enum CatalogState {
    Selected(Selection),
    Withdrawn,
    Ended,
    Failed(String),
}

impl CatalogState {
    fn is_terminal(&self) -> bool {
        !matches!(self, Self::Selected(_))
    }
}

#[derive(Debug, PartialEq)]
enum SelectionDecision {
    Unchanged,
    Replace(Selection),
    Withdrawn,
}

fn classify_selection<T: PartialEq>(current: &T, next: Option<T>) -> SelectionChange<T> {
    match next {
        None => SelectionChange::Withdrawn,
        Some(next) if next == *current => SelectionChange::Unchanged,
        Some(next) => SelectionChange::Replace(next),
    }
}

#[derive(Debug, PartialEq)]
enum SelectionChange<T> {
    Unchanged,
    Replace(T),
    Withdrawn,
}

fn decide_selection(
    current: &Selection,
    snapshot: moq_mux::catalog::hang::Catalog<()>,
) -> anyhow::Result<SelectionDecision> {
    let next = Selection::from_snapshot(snapshot, Some(&current.name))?;
    Ok(match classify_selection(current, next) {
        SelectionChange::Unchanged => SelectionDecision::Unchanged,
        SelectionChange::Replace(next) => SelectionDecision::Replace(next),
        SelectionChange::Withdrawn => SelectionDecision::Withdrawn,
    })
}

fn preferred_name<T>(current: &str, renditions: &BTreeMap<String, T>) -> Option<String> {
    if renditions.contains_key(current) {
        Some(current.to_string())
    } else {
        renditions.first_key_value().map(|(name, _)| name.clone())
    }
}

struct CatalogMonitor {
    updates: watch::Receiver<CatalogState>,
    task: Option<JoinHandle<()>>,
}

impl CatalogMonitor {
    fn spawn(mut catalog: moq_mux::catalog::Consumer<()>, initial: Selection) -> Self {
        let (updates_tx, updates) = watch::channel(CatalogState::Selected(initial.clone()));
        let task = tokio::spawn(async move {
            let mut current = initial;
            loop {
                let state = match catalog.next().await {
                    Ok(Some(snapshot)) => match decide_selection(&current, snapshot) {
                        Ok(SelectionDecision::Unchanged) => continue,
                        Ok(SelectionDecision::Replace(next)) => {
                            current = next.clone();
                            CatalogState::Selected(next)
                        }
                        Ok(SelectionDecision::Withdrawn) => CatalogState::Withdrawn,
                        Err(error) => CatalogState::Failed(error.to_string()),
                    },
                    Ok(None) => CatalogState::Ended,
                    Err(error) => CatalogState::Failed(error.to_string()),
                };
                let terminal = state.is_terminal();
                if updates_tx.send(state).is_err() || terminal {
                    break;
                }
            }
        });
        Self {
            updates,
            task: Some(task),
        }
    }

    async fn changed(&mut self) -> anyhow::Result<CatalogState> {
        self.updates
            .changed()
            .await
            .map_err(|_| anyhow::anyhow!("remote screen catalog monitor stopped unexpectedly"))?;
        Ok(self.updates.borrow_and_update().clone())
    }

    async fn stop(mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
            let _ = task.await;
        }
    }
}

impl Drop for CatalogMonitor {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
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
            decoder_generation: 1,
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

    fn ensure_started(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.started, "remote screen ended before its first frame");
        Ok(())
    }
}

async fn open_decoder(
    broadcast: &moq_net::broadcast::Consumer,
    selection: &Selection,
) -> anyhow::Result<moq_video::decode::Consumer> {
    let config = selection
        .catalog
        .video
        .renditions
        .get(&selection.name)
        .expect("selection stores its rendition");
    Ok(moq_video::decode::Consumer::new(
        broadcast,
        config,
        selection.name.clone(),
        moq_video::decode::Config::new(),
    )
    .await?)
}

pub(super) async fn run(
    view_generation: u64,
    broadcast: moq_net::broadcast::Consumer,
    started: oneshot::Sender<()>,
    started_ack: oneshot::Receiver<()>,
    frames: watch::Sender<Option<Arc<PlaybackFrame>>>,
) -> anyhow::Result<()> {
    let mut catalog =
        moq_mux::catalog::Consumer::<()>::new(&broadcast, moq_mux::catalog::CatalogFormat::Hang)
            .await?;
    let snapshot = catalog
        .next()
        .await?
        .ok_or_else(|| anyhow::anyhow!("remote screen catalog ended"))?;
    let mut selection = Selection::from_snapshot(snapshot, None)?
        .ok_or_else(|| anyhow::anyhow!("remote screen has no video rendition"))?;
    let mut monitor = CatalogMonitor::spawn(catalog, selection.clone());
    let result = async {
        let mut decoder = open_decoder(&broadcast, &selection).await?;
        let mut frame_sequence = FrameSequence::new(view_generation);
        let mut started = Some(started);
        let mut started_ack = Some(started_ack);

        loop {
            tokio::select! {
                biased;
                update = monitor.changed() => {
                    match update? {
                        CatalogState::Selected(next) => {
                            if next == selection {
                                continue;
                            }
                            // `Consumer::read` is not cancel-safe. This branch cancels
                            // the pending read, so the entire decoder must be dropped
                            // and its worker joined before another decoder is opened.
                            drop(decoder);
                            frame_sequence.replace_decoder();
                            decoder = open_decoder(&broadcast, &next).await?;
                            selection = next;
                        }
                        CatalogState::Withdrawn => {
                            anyhow::bail!("remote screen video rendition was withdrawn");
                        }
                        CatalogState::Ended => {
                            anyhow::bail!("remote screen catalog ended");
                        }
                        CatalogState::Failed(error) => {
                            anyhow::bail!("remote screen catalog failed: {error}");
                        }
                    }
                }
                frame = decoder.read() => {
                    let Some(frame) = frame? else {
                        frame_sequence.ensure_started()?;
                        break Ok(());
                    };
                    let (identity, first_view_frame) = frame_sequence.next();
                    let frame = tokio::task::spawn_blocking(move || {
                        PlaybackFrame::from_video(frame, identity)
                    })
                    .await??;
                    frames.send_replace(Some(Arc::new(frame)));
                    if first_view_frame
                        && let Some(started) = started.take()
                    {
                        let _ = started.send(());
                        if let Some(started_ack) = started_ack.take() {
                            let _ = started_ack.await;
                        }
                    }
                }
            }
        }
    }
    .await;
    monitor.stop().await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn renditions(entries: &[(&str, u32)]) -> BTreeMap<String, u32> {
        entries
            .iter()
            .map(|(name, config)| ((*name).to_string(), *config))
            .collect()
    }

    #[test]
    fn identical_selected_config_does_not_replace_decoder() {
        assert_eq!(
            classify_selection(&1080, Some(1080)),
            SelectionChange::Unchanged
        );
    }

    #[test]
    fn material_config_change_replaces_decoder_only_once() {
        let SelectionChange::Replace(next) = classify_selection(&1080, Some(1920)) else {
            panic!("rotation config should replace the decoder");
        };

        assert_eq!(
            classify_selection(&next, Some(1920)),
            SelectionChange::Unchanged
        );
    }

    #[test]
    fn removed_selected_track_switches_to_the_first_remaining_track() {
        let catalog = renditions(&[("screen-b", 720), ("screen-c", 1080)]);

        assert_eq!(
            preferred_name("screen-a", &catalog),
            Some("screen-b".to_string())
        );
    }

    #[test]
    fn selected_track_is_retained_when_other_tracks_appear() {
        let catalog = renditions(&[("screen-a", 720), ("screen-b", 1080)]);

        assert_eq!(
            preferred_name("screen-b", &catalog),
            Some("screen-b".to_string())
        );
    }

    #[test]
    fn empty_catalog_withdraws_video() {
        assert_eq!(classify_selection(&1080, None), SelectionChange::Withdrawn);
    }

    #[test]
    fn decoder_generation_isolates_equal_sequences() {
        let mut frames = FrameSequence::new(7);
        let (before, _) = frames.next();
        frames.replace_decoder();
        let (after, _) = frames.next();

        assert_eq!(before.sequence, after.sequence);
        assert_ne!(before, after);
    }

    #[test]
    fn replacement_before_first_frame_still_starts_on_new_first_frame() {
        let mut frames = FrameSequence::new(7);
        frames.replace_decoder();
        let (_, starts_view) = frames.next();

        assert!(starts_view);
        assert!(frames.ensure_started().is_ok());
    }

    #[test]
    fn replacement_after_start_does_not_start_view_again() {
        let mut frames = FrameSequence::new(7);
        let (_, starts_view) = frames.next();
        assert!(starts_view);

        frames.replace_decoder();
        let (_, starts_view) = frames.next();
        assert!(!starts_view);
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
    fn first_frame_is_required_before_normal_track_end() {
        let frames = FrameSequence::new(7);

        assert!(frames.ensure_started().is_err());
    }

    #[test]
    fn catalog_terminal_states_end_playback() {
        assert!(CatalogState::Withdrawn.is_terminal());
        assert!(CatalogState::Ended.is_terminal());
        assert!(CatalogState::Failed("invalid catalog".into()).is_terminal());
    }
}
