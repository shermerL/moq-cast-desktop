//! Estimated audio position and bounded video presentation. No device-clock claims.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use tokio::sync::watch;

// Scheduling safety bounds, independent of the subscription freshness budget.
const MAX_VIDEO_FRAMES: usize = 8;
const MAX_VIDEO_RESIDENCE: Duration = Duration::from_millis(250);
const VIDEO_LATE_LIMIT: Duration = Duration::from_millis(100);
const VIDEO_EARLY_TOLERANCE: Duration = Duration::from_millis(2);
const MAX_CLOCK_AGE: Duration = Duration::from_millis(250);
const MAX_AUDIO_PTS_GAP: Duration = Duration::from_millis(250);

#[derive(Default)]
pub(super) struct AudioTimeline {
    previous: Option<(Duration, Duration)>,
    discontinuous: bool,
}

impl AudioTimeline {
    /// Returns true once on an observable regression or large media-time jump.
    pub(super) fn observe(&mut self, pts: Duration, duration: Duration) -> bool {
        let broken = self
            .previous
            .is_some_and(|(start, end)| pts < start || pts.saturating_sub(end) > MAX_AUDIO_PTS_GAP);
        self.previous = Some((pts, pts.saturating_add(duration)));
        let first = broken && !self.discontinuous;
        self.discontinuous |= broken;
        first
    }

    pub(super) fn is_continuous(&self) -> bool {
        !self.discontinuous
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct AudioAnchor {
    position: Duration,
    end: Duration,
    observed: Instant,
}

impl AudioAnchor {
    pub(super) fn new(end: Duration, buffered: Duration, observed: Instant) -> Option<Self> {
        // Empty or implausibly large queues cannot provide a useful live clock.
        if buffered.is_zero() || buffered > Duration::from_secs(1) || buffered > end {
            return None;
        }
        Some(Self {
            position: end - buffered,
            end,
            observed,
        })
    }

    fn position_at(self, now: Instant) -> Option<Duration> {
        let elapsed = now.saturating_duration_since(self.observed);
        let position = self.position.saturating_add(elapsed);
        (elapsed < MAX_CLOCK_AGE && position < self.end).then_some(position)
    }

    fn expires(self) -> Instant {
        self.observed + MAX_CLOCK_AGE.min(self.end - self.position)
    }
}

/// A channel belongs to exactly one audio task, including its cancellation path.
pub(super) struct AudioClockWriter(watch::Sender<Option<AudioAnchor>>);

pub(super) struct AudioClockReader {
    receiver: watch::Receiver<Option<AudioAnchor>>,
    closed: bool,
}

pub(super) fn audio_clock() -> (AudioClockWriter, AudioClockReader) {
    let (sender, receiver) = watch::channel(None);
    (
        AudioClockWriter(sender),
        AudioClockReader {
            receiver,
            closed: false,
        },
    )
}

impl AudioClockWriter {
    pub(super) fn update(&self, anchor: Option<AudioAnchor>) {
        self.0.send_replace(anchor);
    }
}

impl Drop for AudioClockWriter {
    fn drop(&mut self) {
        self.0.send_replace(None);
    }
}

impl AudioClockReader {
    pub(super) fn anchor(&self) -> Option<AudioAnchor> {
        *self.receiver.borrow()
    }

    pub(super) async fn changed(&mut self) {
        if self.closed {
            std::future::pending::<()>().await;
        }
        if self.receiver.changed().await.is_err() {
            self.closed = true;
        }
    }
}

struct Queued<T> {
    pts: Duration,
    inserted: Instant,
    value: T,
}

#[derive(Default, Debug)]
pub(super) struct Stats {
    pub(super) selected: u64,
    pub(super) late: u64,
    pub(super) superseded: u64,
    pub(super) capacity: u64,
    pub(super) expired: u64,
    pub(super) nonmonotonic: u64,
    pub(super) resets: u64,
    pub(super) peak_queue: usize,
}

pub(super) struct VideoScheduler<T> {
    queue: VecDeque<Queued<T>>,
    last_presented: Option<Duration>,
    stats: Stats,
}

impl<T> Default for VideoScheduler<T> {
    fn default() -> Self {
        Self {
            queue: VecDeque::new(),
            last_presented: None,
            stats: Stats::default(),
        }
    }
}

pub(super) struct Advance<T> {
    pub(super) frame: Option<T>,
    pub(super) deadline: Option<Instant>,
    pub(super) audio_master: bool,
    pub(super) delta_us: Option<i128>,
}

impl<T> VideoScheduler<T> {
    pub(super) fn push(&mut self, pts: Duration, value: T, now: Instant) {
        if self.last_presented.is_some_and(|last| pts <= last)
            || self.queue.back().is_some_and(|last| pts <= last.pts)
        {
            self.stats.nonmonotonic += 1;
            return;
        }
        while self.queue.front().is_some_and(|first| {
            self.queue.len() >= MAX_VIDEO_FRAMES
                || pts.saturating_sub(first.pts) > MAX_VIDEO_RESIDENCE
        }) {
            self.queue.pop_front();
            self.stats.capacity += 1;
        }
        self.queue.push_back(Queued {
            pts,
            inserted: now,
            value,
        });
        self.stats.peak_queue = self.stats.peak_queue.max(self.queue.len());
    }

    pub(super) fn reset(&mut self) {
        self.queue.clear();
        self.last_presented = None;
        self.stats.resets += 1;
    }

    pub(super) fn take_stats(&mut self) -> Stats {
        std::mem::take(&mut self.stats)
    }

    pub(super) fn advance(&mut self, anchor: Option<AudioAnchor>, now: Instant) -> Advance<T> {
        let audio = anchor.and_then(|clock| clock.position_at(now));
        let mut due = None;
        while let Some(front) = self.queue.front() {
            if now.saturating_duration_since(front.inserted) >= MAX_VIDEO_RESIDENCE {
                self.queue.pop_front();
                self.stats.expired += 1;
                continue;
            }
            if let Some(audio) = audio {
                if audio.saturating_sub(front.pts) > VIDEO_LATE_LIMIT {
                    self.queue.pop_front();
                    self.stats.late += 1;
                    continue;
                }
                if front.pts > audio.saturating_add(VIDEO_EARLY_TOLERANCE) {
                    break;
                }
            }
            if due.is_some() {
                self.stats.superseded += 1;
            }
            due = self.queue.pop_front();
        }
        let delta_us = due.as_ref().and_then(|frame| {
            audio.map(|audio| frame.pts.as_micros() as i128 - audio.as_micros() as i128)
        });
        if let Some(frame) = &due {
            self.last_presented = Some(frame.pts);
            self.stats.selected += 1;
        }
        let deadline = self.queue.front().and_then(|next| {
            audio.zip(anchor).map(|(audio, anchor)| {
                let limit = (next.inserted + MAX_VIDEO_RESIDENCE).min(anchor.expires());
                let wait = next
                    .pts
                    .saturating_sub(audio.saturating_add(VIDEO_EARLY_TOLERANCE))
                    .min(limit.saturating_duration_since(now));
                now + wait
            })
        });
        Advance {
            frame: due.map(|frame| frame.value),
            deadline,
            audio_master: audio.is_some(),
            delta_us,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    fn clock(now: Instant) -> Option<AudioAnchor> {
        AudioAnchor::new(ms(1_100), ms(100), now)
    }

    #[test]
    fn audio_pts_regression_or_large_gap_invalidates_only_that_timeline() {
        for next in [ms(900), ms(2_000)] {
            let mut timeline = AudioTimeline::default();
            assert!(!timeline.observe(ms(1_000), ms(10)));
            assert!(timeline.observe(next, ms(10)));
            assert!(!timeline.is_continuous());
            assert!(!timeline.observe(next + ms(10), ms(10)));
            assert!(AudioTimeline::default().is_continuous());
        }
    }

    #[test]
    fn early_waits_and_on_time_presents() {
        let now = Instant::now();
        let mut scheduler = VideoScheduler::default();
        scheduler.push(ms(1_020), 1, now);
        let early = scheduler.advance(clock(now), now);
        assert_eq!(early.frame, None);
        assert_eq!(early.deadline, Some(now + ms(18)));
        let due = scheduler.advance(clock(now), now + ms(18));
        assert_eq!(due.frame, Some(1));
        assert_eq!(due.delta_us, Some(2_000));
    }

    #[test]
    fn extreme_future_pts_cannot_overflow_or_outlive_the_safety_deadline() {
        let now = Instant::now();
        for elapsed in [Duration::ZERO, ms(220)] {
            let mut scheduler = VideoScheduler::default();
            scheduler.push(Duration::MAX, 1, now);
            let at = now + elapsed;
            let anchor = clock(at);
            let next = scheduler.advance(anchor, at);
            assert_eq!(next.frame, None);
            assert_eq!(
                next.deadline,
                Some((at + ms(100)).min(now + MAX_VIDEO_RESIDENCE))
            );
        }
    }

    #[test]
    fn late_frames_drop_and_only_latest_due_is_presented() {
        let now = Instant::now();
        let mut scheduler = VideoScheduler::default();
        scheduler.push(ms(850), 1, now);
        scheduler.push(ms(980), 2, now);
        scheduler.push(ms(1_000), 3, now);
        assert_eq!(scheduler.advance(clock(now), now).frame, Some(3));
        let stats = scheduler.take_stats();
        assert_eq!(stats.late, 1);
        assert_eq!(stats.superseded, 1);
    }

    #[test]
    fn queue_is_bounded_by_count_pts_span_and_wall_residence() {
        let now = Instant::now();
        let mut scheduler = VideoScheduler::default();
        for frame in 0..20 {
            scheduler.push(ms(1_020 + frame), frame, now);
        }
        assert_eq!(scheduler.queue.len(), MAX_VIDEO_FRAMES);
        scheduler.push(ms(2_000), 20, now);
        assert_eq!(scheduler.queue.len(), 1);
        assert_eq!(scheduler.advance(clock(now), now).frame, None);
        assert_eq!(
            scheduler.advance(None, now + MAX_VIDEO_RESIDENCE).frame,
            None
        );
        assert!(scheduler.queue.is_empty());
        assert!(scheduler.take_stats().expired > 0);
    }

    #[test]
    fn absent_failed_and_exhausted_audio_never_hold_video() {
        let now = Instant::now();
        for anchor in [None, clock(now)] {
            let mut scheduler = VideoScheduler::default();
            scheduler.push(ms(1_200), 1, now);
            let next = scheduler.advance(anchor, now + ms(100));
            assert_eq!(next.frame, Some(1));
            assert!(!next.audio_master);
            assert_eq!(next.deadline, None);
        }
        assert!(AudioAnchor::new(ms(1_100), Duration::ZERO, now).is_none());
        assert!(AudioAnchor::new(ms(100), ms(200), now).is_none());
    }

    #[test]
    fn latest_mode_does_not_anchor_to_first_video_pts() {
        let now = Instant::now();
        let mut scheduler = VideoScheduler::default();
        scheduler.push(ms(1_000), 1, now);
        assert_eq!(scheduler.advance(None, now).frame, Some(1));
        scheduler.push(ms(50_000), 2, now);
        assert_eq!(scheduler.advance(None, now).frame, Some(2));
    }

    #[test]
    fn reset_removes_old_generation_and_accepts_new_pts_epoch() {
        let now = Instant::now();
        let mut scheduler = VideoScheduler::default();
        scheduler.push(ms(1_050), "old", now);
        scheduler.reset();
        scheduler.push(ms(10), "new", now);
        assert_eq!(scheduler.advance(None, now).frame, Some("new"));
        scheduler.push(ms(9), "stale", now);
        assert_eq!(scheduler.advance(None, now).frame, None);
        assert_eq!(scheduler.take_stats().nonmonotonic, 1);
    }

    #[tokio::test]
    async fn old_audio_task_cannot_update_or_clear_replacement_clock() {
        let now = Instant::now();
        let (old, mut old_reader) = audio_clock();
        let (current, current_reader) = audio_clock();
        current.update(clock(now));
        old.update(clock(now));
        drop(old);
        old_reader.changed().await;
        assert!(old_reader.anchor().is_none());
        assert!(current_reader.anchor().is_some());
        drop(current);
        assert!(current_reader.anchor().is_none());
    }
}
