//! Media clock ownership and bounded raw-video scheduling.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::Notify;

pub(super) const AUDIO_PACING_CEILING: Duration = Duration::from_secs(1);
pub(super) const MAX_VIDEO_FRAMES: usize = 30;

const VIDEO_EARLY_TOLERANCE: Duration = Duration::from_millis(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Clock {
    media: Duration,
    wall: Instant,
}

impl Clock {
    pub(super) fn new(media: Duration, wall: Instant) -> Self {
        Self { media, wall }
    }

    pub(super) fn now_at(self, wall: Instant) -> Duration {
        self.media
            .saturating_add(wall.saturating_duration_since(self.wall))
    }
}

#[derive(Clone, Copy)]
struct AudioState {
    generation: u64,
    anchor: Option<Clock>,
}

#[derive(Default)]
pub(super) struct MediaClock {
    audio: Mutex<Option<AudioState>>,
    changed: Notify,
}

impl MediaClock {
    pub(super) fn audio(self: &Arc<Self>, generation: u64) -> AudioLease {
        let notify = {
            let mut state = self.audio.lock().expect("media clock mutex poisoned");
            let notify = state.is_some_and(|current| current.anchor.is_some());
            *state = Some(AudioState {
                generation,
                anchor: None,
            });
            notify
        };
        if notify {
            self.changed.notify_one();
        }
        AudioLease {
            generation,
            clock: self.clone(),
        }
    }

    pub(super) fn audio_anchor(&self) -> Option<Clock> {
        (*self.audio.lock().expect("media clock mutex poisoned")).and_then(|state| state.anchor)
    }

    pub(super) async fn changed(&self) {
        self.changed.notified().await;
    }

    fn anchor_audio(&self, generation: u64, anchor: Clock) -> bool {
        let notify = {
            let mut state = self.audio.lock().expect("media clock mutex poisoned");
            let Some(current) = state
                .as_mut()
                .filter(|state| state.generation == generation)
            else {
                return false;
            };
            let notify = current.anchor.is_none();
            current.anchor = Some(anchor);
            notify
        };
        if notify {
            self.changed.notify_one();
        }
        true
    }

    fn clear_audio(&self, generation: u64) -> bool {
        let notify = {
            let mut state = self.audio.lock().expect("media clock mutex poisoned");
            let Some(current) = (*state).filter(|state| state.generation == generation) else {
                return false;
            };
            *state = None;
            current.anchor.is_some()
        };
        if notify {
            self.changed.notify_one();
        }
        true
    }
}

pub(super) struct AudioLease {
    generation: u64,
    clock: Arc<MediaClock>,
}

impl AudioLease {
    pub(super) fn anchor(&self, end: Duration, buffered: Duration, wall: Instant) -> bool {
        self.clock.anchor_audio(
            self.generation,
            Clock::new(end.saturating_sub(buffered), wall),
        )
    }
}

impl Drop for AudioLease {
    fn drop(&mut self) {
        self.clock.clear_audio(self.generation);
    }
}

pub(super) fn timestamp(timestamp: moq_tokio::moq_net::Timestamp) -> Duration {
    timestamp_from_micros(timestamp.as_micros())
}

fn timestamp_from_micros(micros: u128) -> Duration {
    Duration::from_micros(micros.min(u128::from(u64::MAX)) as u64)
}

pub(super) fn pcm_frame_end(
    frame_timestamp: moq_tokio::moq_net::Timestamp,
    bytes: usize,
    sample_rate: u32,
    channels: u32,
) -> Duration {
    let stride = u128::from(channels).saturating_mul(size_of::<f32>() as u128);
    if stride == 0 || sample_rate == 0 {
        return timestamp(frame_timestamp);
    }
    let samples = (bytes as u128) / stride;
    let micros = samples
        .saturating_mul(1_000_000)
        .checked_div(u128::from(sample_rate))
        .unwrap_or_default();
    timestamp(frame_timestamp).saturating_add(timestamp_from_micros(micros))
}

struct Scheduled<T> {
    timestamp: Duration,
    value: T,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ClockSource {
    Audio,
    VideoFallback,
}

pub(super) struct VideoScheduler<T> {
    queue: VecDeque<Scheduled<T>>,
    fallback: Option<Clock>,
    source: Option<ClockSource>,
    last_presented: Option<Duration>,
}

pub(super) struct VideoAdvance<T> {
    pub(super) due: Option<T>,
    pub(super) deadline: Option<Instant>,
    pub(super) source_changed: Option<ClockSource>,
    pub(super) skipped_due: usize,
}

pub(super) struct VideoPush<T> {
    pub(super) dropped: Option<T>,
    pub(super) accepted: bool,
}

impl<T> Default for VideoScheduler<T> {
    fn default() -> Self {
        Self {
            queue: VecDeque::with_capacity(MAX_VIDEO_FRAMES),
            fallback: None,
            source: None,
            last_presented: None,
        }
    }
}

impl<T> VideoScheduler<T> {
    pub(super) fn push(&mut self, timestamp: Duration, value: T) -> VideoPush<T> {
        if self
            .last_presented
            .is_some_and(|presented| timestamp <= presented)
        {
            return VideoPush {
                dropped: Some(value),
                accepted: false,
            };
        }
        let mut dropped = None;
        if self.queue.len() == MAX_VIDEO_FRAMES {
            let Some(oldest) = self.queue.front() else {
                unreachable!("a full scheduler has a first frame");
            };
            if timestamp <= oldest.timestamp {
                return VideoPush {
                    dropped: Some(value),
                    accepted: false,
                };
            }
            dropped = self.queue.pop_front().map(|frame| frame.value);
        }
        let index = self
            .queue
            .iter()
            .position(|frame| frame.timestamp > timestamp)
            .unwrap_or(self.queue.len());
        self.queue.insert(index, Scheduled { timestamp, value });
        VideoPush {
            dropped,
            accepted: true,
        }
    }

    pub(super) fn reset(&mut self) {
        self.queue.clear();
        self.fallback = None;
        self.source = None;
        self.last_presented = None;
    }

    pub(super) fn reset_fallback(&mut self) {
        self.fallback = None;
    }

    pub(super) fn advance(&mut self, audio: Option<Clock>, wall: Instant) -> VideoAdvance<T> {
        let source = if audio.is_some() {
            ClockSource::Audio
        } else {
            ClockSource::VideoFallback
        };
        let source_changed = (self.source != Some(source)).then_some(source);
        if source_changed.is_some() {
            self.fallback = None;
            self.source = Some(source);
        }
        if audio.is_none() && self.fallback.is_none() {
            self.fallback = self
                .queue
                .front()
                .map(|frame| Clock::new(frame.timestamp, wall));
        }
        let clock = audio.or(self.fallback);
        let now = clock.map(|clock| clock.now_at(wall));
        let mut due = None;
        let mut skipped_due = 0;
        while self.queue.front().is_some_and(|frame| {
            now.is_none_or(|now| frame.timestamp <= now.saturating_add(VIDEO_EARLY_TOLERANCE))
        }) {
            if due.is_some() {
                skipped_due += 1;
            }
            if let Some(frame) = self.queue.pop_front() {
                self.last_presented = Some(frame.timestamp);
                due = Some(frame.value);
            }
        }
        let deadline = match (clock, self.queue.front()) {
            (Some(clock), Some(next)) => {
                wall.checked_add(next.timestamp.saturating_sub(clock.now_at(wall)))
            }
            _ => None,
        };
        VideoAdvance {
            due,
            deadline,
            source_changed,
            skipped_due,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(micros: u64) -> Duration {
        Duration::from_micros(micros)
    }

    #[test]
    fn old_audio_generation_cannot_update_or_clear_the_new_clock() {
        let media = Arc::new(MediaClock::default());
        let old = media.audio(4);
        assert!(old.anchor(at(1_000_000), Duration::ZERO, Instant::now()));
        let fresh = media.audio(5);
        assert!(fresh.anchor(at(2_000_000), Duration::ZERO, Instant::now()));

        assert!(!old.anchor(at(9_000_000), Duration::ZERO, Instant::now()));
        drop(old);
        assert_eq!(
            media.audio_anchor().map(|clock| clock.media),
            Some(at(2_000_000))
        );
        drop(fresh);
        assert!(media.audio_anchor().is_none());
    }

    #[test]
    fn audio_position_is_wire_frame_end_less_buffered_output() {
        let wire = moq_tokio::moq_net::Timestamp::from_micros(1_000_000).expect("wire timestamp");
        let ten_ms_stereo_f32 = 480 * 2 * size_of::<f32>();
        let end = pcm_frame_end(wire, ten_ms_stereo_f32, 48_000, 2);
        let wall = Instant::now();
        let media = Arc::new(MediaClock::default());
        let lease = media.audio(1);

        assert_eq!(end, at(1_010_000));
        assert!(lease.anchor(end, at(4_000), wall));
        assert_eq!(
            media.audio_anchor().map(|clock| clock.media),
            Some(at(1_006_000))
        );
    }

    #[test]
    fn first_video_frame_builds_fallback_without_audio() {
        let wall = Instant::now();
        let mut video = VideoScheduler::default();
        assert!(video.push(at(900_000), 1).accepted);

        let advance = video.advance(None, wall);

        assert_eq!(advance.due, Some(1));
        assert_eq!(advance.source_changed, Some(ClockSource::VideoFallback));
    }

    #[test]
    fn audio_withdraw_or_failure_rebuilds_fallback_from_the_next_video_frame() {
        let wall = Instant::now();
        let media = Arc::new(MediaClock::default());
        let audio = media.audio(4);
        assert!(audio.anchor(at(2_000_000), Duration::ZERO, wall));
        let mut video = VideoScheduler::default();
        video.push(at(2_000_000), 1);
        video.push(at(3_000_000), 2);
        let advance = video.advance(media.audio_anchor(), wall);
        assert_eq!(advance.source_changed, Some(ClockSource::Audio));

        drop(audio);
        video.reset_fallback();
        let advance = video.advance(media.audio_anchor(), wall + at(10_000));

        assert_eq!(advance.source_changed, Some(ClockSource::VideoFallback));
        assert_eq!(advance.due, Some(2));
    }

    #[test]
    fn only_latest_due_frame_is_returned_and_future_frame_remains() {
        let wall = Instant::now();
        let mut video = VideoScheduler::default();
        video.push(at(30_000), 3);
        video.push(at(10_000), 1);
        video.push(at(100_000), 4);
        video.push(at(20_000), 2);

        let advance = video.advance(Some(Clock::new(at(50_000), wall)), wall);

        assert_eq!(advance.due, Some(3));
        assert_eq!(advance.skipped_due, 2);
        assert_eq!(video.queue.len(), 1);
        assert_eq!(advance.deadline, wall.checked_add(at(50_000)));

        let stale = video.push(at(25_000), 5);
        assert!(!stale.accepted);
        assert_eq!(stale.dropped, Some(5));
    }

    #[test]
    fn video_queue_stays_bounded_and_keeps_the_latest_frames() {
        let mut video = VideoScheduler::default();
        for value in 0..MAX_VIDEO_FRAMES {
            assert!(video.push(at(value as u64), value).accepted);
        }

        let pushed = video.push(at(99_000), MAX_VIDEO_FRAMES);

        assert!(pushed.accepted);
        assert_eq!(pushed.dropped, Some(0));
        assert_eq!(video.queue.len(), MAX_VIDEO_FRAMES);

        let rejected = video.push(Duration::ZERO, MAX_VIDEO_FRAMES + 1);
        assert!(!rejected.accepted);
        assert_eq!(rejected.dropped, Some(MAX_VIDEO_FRAMES + 1));
        assert_eq!(video.queue.len(), MAX_VIDEO_FRAMES);
    }

    #[test]
    fn decoder_replacement_clears_queue_and_clock_source() {
        let wall = Instant::now();
        let mut video = VideoScheduler::default();
        video.push(at(10_000), 1);
        video.advance(None, wall);

        video.reset();

        assert!(video.queue.is_empty());
        assert!(video.fallback.is_none());
        assert!(video.source.is_none());
        assert!(video.push(Duration::ZERO, 2).accepted);
    }
}
