//! Media clock ownership and bounded video presentation scheduling.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::Notify;

// This ceiling paces a writer that runs ahead; it is not startup prebuffering or target latency.
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
            Clock::new(audio_media_position(end, buffered), wall),
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

pub(super) fn timestamp_from_micros(micros: u128) -> Duration {
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

fn audio_media_position(end: Duration, buffered: Duration) -> Duration {
    end.saturating_sub(buffered)
}

struct Scheduled<T> {
    timestamp: Duration,
    value: T,
}

pub(super) struct VideoScheduler<T> {
    queue: VecDeque<Scheduled<T>>,
    fallback: Option<Clock>,
    using_audio: bool,
}

pub(super) struct VideoAdvance<T> {
    pub(super) due: Option<T>,
    pub(super) deadline: Option<Instant>,
}

impl<T> Default for VideoScheduler<T> {
    fn default() -> Self {
        Self {
            queue: VecDeque::with_capacity(MAX_VIDEO_FRAMES),
            fallback: None,
            using_audio: false,
        }
    }
}

impl<T> VideoScheduler<T> {
    pub(super) fn has_capacity(&self) -> bool {
        self.queue.len() < MAX_VIDEO_FRAMES
    }

    pub(super) fn push(&mut self, timestamp: Duration, value: T) -> Result<(), T> {
        if !self.has_capacity() {
            return Err(value);
        }
        self.queue.push_back(Scheduled { timestamp, value });
        Ok(())
    }

    pub(super) fn reset(&mut self) {
        self.queue.clear();
        self.fallback = None;
    }

    pub(super) fn reset_fallback(&mut self) {
        self.fallback = None;
    }

    pub(super) fn advance(&mut self, audio: Option<Clock>, wall: Instant) -> VideoAdvance<T> {
        let has_audio = audio.is_some();
        if has_audio != self.using_audio {
            self.fallback = None;
            self.using_audio = has_audio;
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
        while self.queue.front().is_some_and(|frame| {
            now.is_none_or(|now| frame.timestamp <= now.saturating_add(VIDEO_EARLY_TOLERANCE))
        }) {
            due = self.queue.pop_front().map(|frame| frame.value);
        }
        let deadline = match (clock, self.queue.front()) {
            (Some(clock), Some(next)) => {
                wall.checked_add(next.timestamp.saturating_sub(clock.now_at(wall)))
            }
            _ => None,
        };
        VideoAdvance { due, deadline }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(micros: u64) -> Duration {
        Duration::from_micros(micros)
    }

    #[test]
    fn clock_advances_monotonically_from_media_and_wall_anchor() {
        let wall = Instant::now();
        let clock = Clock::new(at(1_000_000), wall);

        assert_eq!(clock.now_at(wall), at(1_000_000));
        assert_eq!(clock.now_at(wall + at(250_000)), at(1_250_000));
    }

    #[test]
    fn audio_position_is_frame_end_less_buffered_audio() {
        assert_eq!(AUDIO_PACING_CEILING, Duration::from_secs(1));
        assert_eq!(
            audio_media_position(at(2_000_000), at(125_000)),
            at(1_875_000)
        );
        assert_eq!(audio_media_position(at(20_000), at(50_000)), Duration::ZERO);
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

    #[tokio::test]
    async fn first_audio_anchor_notifies_the_scheduler() {
        let media = Arc::new(MediaClock::default());
        let lease = media.audio(1);

        assert!(lease.anchor(at(1_000_000), Duration::ZERO, Instant::now()));
        media.changed().await;
    }

    #[test]
    fn pcm_frame_end_uses_wire_pts_and_interleaved_sample_count() {
        let wire =
            moq_tokio::moq_net::Timestamp::from_micros(1_000_000).expect("valid wire timestamp");
        let ten_ms_stereo_f32 = 480 * 2 * size_of::<f32>();

        assert_eq!(timestamp(wire), at(1_000_000));
        assert_eq!(
            pcm_frame_end(wire, ten_ms_stereo_f32, 48_000, 2),
            at(1_010_000)
        );
    }

    #[test]
    fn first_video_frame_builds_fallback_without_audio() {
        let wall = Instant::now();
        let mut video = VideoScheduler::default();
        assert!(video.push(at(900_000), 1).is_ok());

        let advance = video.advance(None, wall);

        assert_eq!(advance.due, Some(1));
        assert!(video.fallback.is_some());
    }

    #[test]
    fn audio_source_switch_resets_and_rebuilds_fallback() {
        let wall = Instant::now();
        let mut video = VideoScheduler::default();
        assert!(video.push(at(2_000_000), 1).is_ok());
        assert!(video.push(at(3_000_000), 2).is_ok());
        video.advance(None, wall);
        assert!(video.fallback.is_some());

        video.advance(Some(Clock::new(at(2_000_000), wall)), wall);
        assert!(video.fallback.is_none());

        video.advance(None, wall + at(10_000));
        assert_eq!(video.fallback.map(|clock| clock.media), Some(at(3_000_000)));
    }

    #[test]
    fn audio_generation_change_rebuilds_video_fallback() {
        let wall = Instant::now();
        let mut video = VideoScheduler::default();
        assert!(video.push(at(10_000), 1).is_ok());
        assert!(video.push(at(20_000), 2).is_ok());
        video.advance(None, wall);
        let previous = video.fallback;

        video.reset_fallback();
        video.advance(None, wall + at(5_000));

        assert_ne!(video.fallback, previous);
        assert_eq!(video.fallback.map(|clock| clock.media), Some(at(20_000)));
    }

    #[test]
    fn only_latest_due_video_frame_is_returned_and_future_frame_remains() {
        let wall = Instant::now();
        let mut video = VideoScheduler::default();
        assert!(video.push(at(10_000), 1).is_ok());
        assert!(video.push(at(20_000), 2).is_ok());
        assert!(video.push(at(30_000), 3).is_ok());
        assert!(video.push(at(100_000), 4).is_ok());

        let advance = video.advance(Some(Clock::new(at(50_000), wall)), wall);

        assert_eq!(advance.due, Some(3));
        assert_eq!(video.queue.len(), 1);
        assert_eq!(advance.deadline, wall.checked_add(at(50_000)));
    }

    #[test]
    fn decoder_replacement_clears_queue_and_fallback_clock() {
        let wall = Instant::now();
        let mut video = VideoScheduler::default();
        assert!(video.push(at(10_000), 1).is_ok());
        assert!(video.push(at(20_000), 2).is_ok());
        video.advance(None, wall);

        video.reset();

        assert!(video.queue.is_empty());
        assert!(video.fallback.is_none());
        assert!(video.has_capacity());
    }

    #[test]
    fn video_queue_rejects_frames_beyond_its_capacity() {
        let mut video = VideoScheduler::default();
        for value in 0..MAX_VIDEO_FRAMES {
            assert!(video.push(at(value as u64), value).is_ok());
        }

        assert_eq!(
            video.push(at(99_000), MAX_VIDEO_FRAMES),
            Err(MAX_VIDEO_FRAMES)
        );
        assert_eq!(video.queue.len(), MAX_VIDEO_FRAMES);
        assert!(!video.has_capacity());
    }

    #[test]
    fn wire_timestamp_conversion_saturates() {
        assert_eq!(
            timestamp_from_micros(u128::MAX),
            Duration::from_micros(u64::MAX)
        );
    }
}
