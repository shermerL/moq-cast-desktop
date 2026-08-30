//! Generation-scoped remote audio continuity observations.

use std::cmp;
use std::mem::size_of;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

pub(super) const SUMMARY_INTERVAL: Duration = Duration::from_secs(10);
const PTS_ROUNDING_TOLERANCE: Duration = Duration::from_millis(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(super) enum TeardownReason {
    Stop = 1,
    Replacement = 2,
    Withdraw = 3,
    Ended = 4,
    DecodeError = 5,
    SinkError = 6,
    StartError = 7,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OwnerTeardownReason {
    Stop,
    Replacement,
    Withdraw,
}

impl From<OwnerTeardownReason> for TeardownReason {
    fn from(reason: OwnerTeardownReason) -> Self {
        match reason {
            OwnerTeardownReason::Stop => TeardownReason::Stop,
            OwnerTeardownReason::Replacement => TeardownReason::Replacement,
            OwnerTeardownReason::Withdraw => TeardownReason::Withdraw,
        }
    }
}

impl TeardownReason {
    fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Stop),
            2 => Some(Self::Replacement),
            3 => Some(Self::Withdraw),
            4 => Some(Self::Ended),
            5 => Some(Self::DecodeError),
            6 => Some(Self::SinkError),
            7 => Some(Self::StartError),
            _ => None,
        }
    }
}

#[derive(Default)]
pub(super) struct TeardownControl {
    owner_reason: AtomicU8,
    emitted_reason: AtomicU8,
}

impl TeardownControl {
    pub(super) fn set_owner_reason(&self, reason: OwnerTeardownReason) {
        self.owner_reason
            .store(TeardownReason::from(reason) as u8, Ordering::Release);
    }

    fn owner_reason(&self) -> Option<TeardownReason> {
        TeardownReason::from_code(self.owner_reason.load(Ordering::Acquire))
    }

    fn try_mark_emitted(&self, reason: TeardownReason) -> bool {
        self.emitted_reason
            .compare_exchange(0, reason as u8, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    pub(super) fn summary_emitted(&self) -> bool {
        self.emitted_reason.load(Ordering::Acquire) != 0
    }

    #[cfg(test)]
    fn emitted_reason(&self) -> Option<TeardownReason> {
        TeardownReason::from_code(self.emitted_reason.load(Ordering::Acquire))
    }
}

impl TeardownReason {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Stop => "stop",
            Self::Replacement => "replacement",
            Self::Withdraw => "withdraw",
            Self::Ended => "ended",
            Self::DecodeError => "decode_error",
            Self::SinkError => "sink_error",
            Self::StartError => "start_error",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FrameTiming {
    start: Duration,
    end: Duration,
    sample_frames: u64,
}

impl FrameTiming {
    pub(super) fn new(start: Duration, bytes: usize, sample_rate: u32, channels: u32) -> Self {
        let sample_frames = pcm_sample_frames(bytes, channels);
        let duration = pcm_duration(sample_frames, sample_rate);
        Self {
            start,
            end: start.saturating_add(duration),
            sample_frames,
        }
    }

    pub(super) fn end(self) -> Duration {
        self.end
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Metrics {
    observed_buffered_current: Option<Duration>,
    post_submit_buffered_min: Option<Duration>,
    post_submit_buffered_max: Option<Duration>,
    decoded_frames: u64,
    pcm_sample_frames: u64,
    sink_write_attempts: u64,
    sink_write_failures: u64,
    submitted_chunks: u64,
    submitted_bytes: u64,
    submitted_sample_frames: u64,
    submitted_duration: Duration,
    max_submitted_chunk_duration: Duration,
    pts_gap_count: u64,
    pts_regression_count: u64,
    max_pts_gap: Duration,
    max_pts_regression: Duration,
    pacing_delay_requested_count: u64,
    pacing_delay_requested_total: Duration,
    pacing_delay_requested_max: Duration,
}

pub(super) struct Tracker {
    generation: u64,
    path: String,
    track: String,
    codec: String,
    selected_at: Instant,
    last_summary_at: Instant,
    last_expected_pts: Option<Duration>,
    first_decoded: bool,
    first_submitted: bool,
    logged_gap: bool,
    logged_regression: bool,
    logged_pacing_delay_requested: bool,
    metrics: Metrics,
    teardown: Arc<TeardownControl>,
}

impl Tracker {
    pub(super) fn new(
        generation: u64,
        path: &str,
        track: &str,
        codec: &str,
        selected_at: Instant,
        teardown: Arc<TeardownControl>,
    ) -> Self {
        tracing::info!(
            broadcast = path,
            track,
            codec,
            audio_generation = generation,
            "remote audio continuity generation started"
        );
        Self {
            generation,
            path: path.to_owned(),
            track: track.to_owned(),
            codec: codec.to_owned(),
            selected_at,
            last_summary_at: selected_at,
            last_expected_pts: None,
            first_decoded: false,
            first_submitted: false,
            logged_gap: false,
            logged_regression: false,
            logged_pacing_delay_requested: false,
            metrics: Metrics::default(),
            teardown,
        }
    }

    #[cfg(test)]
    fn generation(&self) -> u64 {
        self.generation
    }

    pub(super) fn observe_frame(
        &mut self,
        timing: FrameTiming,
        now: impl FnOnce() -> Instant,
    ) -> Option<Duration> {
        self.metrics.decoded_frames = self.metrics.decoded_frames.saturating_add(1);
        self.metrics.pcm_sample_frames = self
            .metrics
            .pcm_sample_frames
            .saturating_add(timing.sample_frames);

        if let Some(expected) = self.last_expected_pts {
            if timing.start > expected.saturating_add(PTS_ROUNDING_TOLERANCE) {
                let gap = timing.start.saturating_sub(expected);
                self.metrics.pts_gap_count = self.metrics.pts_gap_count.saturating_add(1);
                self.metrics.max_pts_gap = cmp::max(self.metrics.max_pts_gap, gap);
                if !self.logged_gap {
                    self.logged_gap = true;
                    tracing::warn!(
                        broadcast = %self.path,
                        track = %self.track,
                        audio_generation = self.generation,
                        expected_pts_us = duration_us(expected),
                        actual_pts_us = duration_us(timing.start),
                        pts_gap_us = duration_us(gap),
                        "remote PCM PTS gap observed"
                    );
                }
            } else if timing.start.saturating_add(PTS_ROUNDING_TOLERANCE) < expected {
                let regression = expected.saturating_sub(timing.start);
                self.metrics.pts_regression_count =
                    self.metrics.pts_regression_count.saturating_add(1);
                self.metrics.max_pts_regression =
                    cmp::max(self.metrics.max_pts_regression, regression);
                if !self.logged_regression {
                    self.logged_regression = true;
                    tracing::warn!(
                        broadcast = %self.path,
                        track = %self.track,
                        audio_generation = self.generation,
                        expected_pts_us = duration_us(expected),
                        actual_pts_us = duration_us(timing.start),
                        pts_regression_us = duration_us(regression),
                        "remote PCM PTS regression observed"
                    );
                }
            }
        }
        self.last_expected_pts = Some(timing.end);

        let time_to_first_decoded = if self.first_decoded {
            None
        } else {
            self.first_decoded = true;
            let elapsed = now().saturating_duration_since(self.selected_at);
            tracing::info!(
                broadcast = %self.path,
                track = %self.track,
                codec = %self.codec,
                audio_generation = self.generation,
                time_to_first_pcm_decoded = ?elapsed,
                frame_pts_us = duration_us(timing.start),
                frame_sample_frames = timing.sample_frames,
                "first remote PCM frame decoded"
            );
            Some(elapsed)
        };

        time_to_first_decoded
    }

    pub(super) fn observe_buffered(&mut self, buffered: Duration) {
        self.metrics.observed_buffered_current = Some(buffered);
        if self.first_submitted {
            self.metrics.post_submit_buffered_min = Some(
                self.metrics
                    .post_submit_buffered_min
                    .map_or(buffered, |current| cmp::min(current, buffered)),
            );
            self.metrics.post_submit_buffered_max = Some(
                self.metrics
                    .post_submit_buffered_max
                    .map_or(buffered, |current| cmp::max(current, buffered)),
            );
        }
    }

    pub(super) fn observe_write(
        &mut self,
        bytes: usize,
        sample_rate: u32,
        channels: u32,
        succeeded: bool,
        now: impl FnOnce() -> Instant,
    ) -> Option<Duration> {
        let sample_frames = pcm_sample_frames(bytes, channels);
        let duration = pcm_duration(sample_frames, sample_rate);
        self.metrics.sink_write_attempts = self.metrics.sink_write_attempts.saturating_add(1);

        if !succeeded {
            self.metrics.sink_write_failures = self.metrics.sink_write_failures.saturating_add(1);
            return None;
        }

        self.metrics.submitted_chunks = self.metrics.submitted_chunks.saturating_add(1);
        self.metrics.submitted_bytes = self
            .metrics
            .submitted_bytes
            .saturating_add(u64::try_from(bytes).unwrap_or(u64::MAX));
        self.metrics.submitted_sample_frames = self
            .metrics
            .submitted_sample_frames
            .saturating_add(sample_frames);
        self.metrics.submitted_duration = self.metrics.submitted_duration.saturating_add(duration);
        self.metrics.max_submitted_chunk_duration =
            cmp::max(self.metrics.max_submitted_chunk_duration, duration);
        let time_to_first_pcm_submit = if self.first_submitted {
            None
        } else {
            self.first_submitted = true;
            let elapsed = now().saturating_duration_since(self.selected_at);
            tracing::info!(
                broadcast = %self.path,
                track = %self.track,
                codec = %self.codec,
                audio_generation = self.generation,
                time_to_first_pcm_submit = ?elapsed,
                chunk_bytes = bytes,
                chunk_sample_frames = sample_frames,
                chunk_duration_us = duration_us(duration),
                "first remote PCM sink write returned successfully"
            );
            Some(elapsed)
        };

        time_to_first_pcm_submit
    }

    pub(super) fn observe_pacing_delay_requested(&mut self, delay: Duration) {
        self.metrics.pacing_delay_requested_count =
            self.metrics.pacing_delay_requested_count.saturating_add(1);
        self.metrics.pacing_delay_requested_total = self
            .metrics
            .pacing_delay_requested_total
            .saturating_add(delay);
        self.metrics.pacing_delay_requested_max =
            cmp::max(self.metrics.pacing_delay_requested_max, delay);

        if !self.logged_pacing_delay_requested {
            self.logged_pacing_delay_requested = true;
            tracing::warn!(
                broadcast = %self.path,
                track = %self.track,
                audio_generation = self.generation,
                observed_buffered_us = ?self.metrics.observed_buffered_current.map(duration_us),
                audio_pacing_ceiling_us =
                    duration_us(crate::runtime::playback_sync::AUDIO_PACING_CEILING),
                pacing_delay_requested_us = duration_us(delay),
                "remote PCM submission pacing requested a delay"
            );
        }
    }

    pub(super) fn summary_due(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.last_summary_at) >= SUMMARY_INTERVAL
    }

    pub(super) fn mark_summary(&mut self, now: Instant) {
        self.last_summary_at = now;
    }

    pub(super) fn maybe_log_summary(&mut self, now: Instant) {
        if self.summary_due(now) {
            self.log_periodic_summary();
            self.mark_summary(now);
        }
    }

    pub(super) fn finish(self, reason: TeardownReason) {
        self.log_teardown(reason);
    }

    #[cfg(test)]
    fn metrics(&self) -> &Metrics {
        &self.metrics
    }

    fn log_periodic_summary(&self) {
        tracing::debug!(
            broadcast = %self.path,
            track = %self.track,
            codec = %self.codec,
            audio_generation = self.generation,
            observed_buffered_current_us = ?self.metrics.observed_buffered_current.map(duration_us),
            post_submit_buffered_min_us = ?self.metrics.post_submit_buffered_min.map(duration_us),
            post_submit_buffered_max_us = ?self.metrics.post_submit_buffered_max.map(duration_us),
            decoded_frames = self.metrics.decoded_frames,
            pcm_sample_frames = self.metrics.pcm_sample_frames,
            sink_write_attempts = self.metrics.sink_write_attempts,
            sink_write_failures = self.metrics.sink_write_failures,
            submitted_chunks = self.metrics.submitted_chunks,
            submitted_bytes = self.metrics.submitted_bytes,
            submitted_sample_frames = self.metrics.submitted_sample_frames,
            submitted_duration_us = duration_us(self.metrics.submitted_duration),
            max_submitted_chunk_duration_us =
                duration_us(self.metrics.max_submitted_chunk_duration),
            pts_gap_count = self.metrics.pts_gap_count,
            pts_regression_count = self.metrics.pts_regression_count,
            max_pts_gap_us = duration_us(self.metrics.max_pts_gap),
            max_pts_regression_us = duration_us(self.metrics.max_pts_regression),
            pacing_delay_requested_count = self.metrics.pacing_delay_requested_count,
            pacing_delay_requested_total_us =
                duration_us(self.metrics.pacing_delay_requested_total),
            pacing_delay_requested_max_us =
                duration_us(self.metrics.pacing_delay_requested_max),
            "remote audio continuity summary"
        );
    }

    fn log_teardown(&self, reason: TeardownReason) {
        if !self.teardown.try_mark_emitted(reason) {
            return;
        }
        tracing::info!(
            broadcast = %self.path,
            track = %self.track,
            codec = %self.codec,
            audio_generation = self.generation,
            teardown_reason = reason.as_str(),
            observed_buffered_current_us = ?self.metrics.observed_buffered_current.map(duration_us),
            post_submit_buffered_min_us = ?self.metrics.post_submit_buffered_min.map(duration_us),
            post_submit_buffered_max_us = ?self.metrics.post_submit_buffered_max.map(duration_us),
            decoded_frames = self.metrics.decoded_frames,
            pcm_sample_frames = self.metrics.pcm_sample_frames,
            sink_write_attempts = self.metrics.sink_write_attempts,
            sink_write_failures = self.metrics.sink_write_failures,
            submitted_chunks = self.metrics.submitted_chunks,
            submitted_bytes = self.metrics.submitted_bytes,
            submitted_sample_frames = self.metrics.submitted_sample_frames,
            submitted_duration_us = duration_us(self.metrics.submitted_duration),
            max_submitted_chunk_duration_us =
                duration_us(self.metrics.max_submitted_chunk_duration),
            pts_gap_count = self.metrics.pts_gap_count,
            pts_regression_count = self.metrics.pts_regression_count,
            max_pts_gap_us = duration_us(self.metrics.max_pts_gap),
            max_pts_regression_us = duration_us(self.metrics.max_pts_regression),
            pacing_delay_requested_count = self.metrics.pacing_delay_requested_count,
            pacing_delay_requested_total_us =
                duration_us(self.metrics.pacing_delay_requested_total),
            pacing_delay_requested_max_us =
                duration_us(self.metrics.pacing_delay_requested_max),
            "remote audio continuity generation ended"
        );
    }
}

impl Drop for Tracker {
    fn drop(&mut self) {
        if let Some(reason) = self.teardown.owner_reason() {
            self.log_teardown(reason);
        }
    }
}

fn pcm_sample_frames(bytes: usize, channels: u32) -> u64 {
    let stride = (channels as usize).saturating_mul(size_of::<f32>());
    if stride == 0 {
        return 0;
    }
    u64::try_from(bytes / stride).unwrap_or(u64::MAX)
}

fn pcm_duration(sample_frames: u64, sample_rate: u32) -> Duration {
    if sample_rate == 0 {
        return Duration::ZERO;
    }
    let micros = u128::from(sample_frames)
        .saturating_mul(1_000_000)
        .checked_div(u128::from(sample_rate))
        .unwrap_or_default();
    Duration::from_micros(micros.min(u128::from(u64::MAX)) as u64)
}

fn duration_us(duration: Duration) -> u64 {
    duration.as_micros().min(u128::from(u64::MAX)) as u64
}

pub(super) fn pacing_delay(buffered: Duration) -> Option<Duration> {
    (buffered > crate::runtime::playback_sync::AUDIO_PACING_CEILING)
        .then(|| buffered - crate::runtime::playback_sync::AUDIO_PACING_CEILING)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::time::{Duration, Instant};

    use super::*;

    fn at(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    fn tracker_with_control(now: Instant, teardown: Arc<TeardownControl>) -> Tracker {
        Tracker::new(7, "screen", "audio", "opus", now, teardown)
    }

    fn tracker(now: Instant) -> Tracker {
        tracker_with_control(now, Arc::new(TeardownControl::default()))
    }

    #[test]
    fn continuous_pcm_pts_have_no_discontinuity() {
        let now = Instant::now();
        let mut tracker = tracker(now);

        tracker.observe_frame(FrameTiming::new(at(0), 3_840, 48_000, 2), || now);
        tracker.observe_frame(FrameTiming::new(at(10), 3_840, 48_000, 2), || now);

        assert_eq!(tracker.metrics().pts_gap_count, 0);
        assert_eq!(tracker.metrics().pts_regression_count, 0);
    }

    #[test]
    fn positive_gap_and_regression_are_counted_from_expected_pts() {
        let now = Instant::now();
        let mut gap = tracker(now);
        gap.observe_frame(FrameTiming::new(at(0), 3_840, 48_000, 2), || now);
        gap.observe_frame(FrameTiming::new(at(15), 3_840, 48_000, 2), || now);
        assert_eq!(gap.metrics().pts_gap_count, 1);
        assert_eq!(gap.metrics().max_pts_gap, at(5));

        let mut regression = tracker(now);
        regression.observe_frame(FrameTiming::new(at(10), 3_840, 48_000, 2), || now);
        regression.observe_frame(FrameTiming::new(at(15), 3_840, 48_000, 2), || now);
        assert_eq!(regression.metrics().pts_regression_count, 1);
        assert_eq!(regression.metrics().max_pts_regression, at(5));
    }

    #[test]
    fn pts_rounding_tolerance_has_an_explicit_boundary() {
        let now = Instant::now();
        let mut within_gap = tracker(now);
        within_gap.observe_frame(FrameTiming::new(at(0), 3_840, 48_000, 2), || now);
        within_gap.observe_frame(FrameTiming::new(at(11), 3_840, 48_000, 2), || now);
        assert_eq!(within_gap.metrics().pts_gap_count, 0);

        let mut beyond_gap = tracker(now);
        beyond_gap.observe_frame(FrameTiming::new(at(0), 3_840, 48_000, 2), || now);
        beyond_gap.observe_frame(
            FrameTiming::new(Duration::from_micros(11_001), 3_840, 48_000, 2),
            || now,
        );
        assert_eq!(beyond_gap.metrics().pts_gap_count, 1);

        let mut within_regression = tracker(now);
        within_regression.observe_frame(FrameTiming::new(at(10), 3_840, 48_000, 2), || now);
        within_regression.observe_frame(FrameTiming::new(at(19), 3_840, 48_000, 2), || now);
        assert_eq!(within_regression.metrics().pts_regression_count, 0);

        let mut beyond_regression = tracker(now);
        beyond_regression.observe_frame(FrameTiming::new(at(10), 3_840, 48_000, 2), || now);
        beyond_regression.observe_frame(
            FrameTiming::new(Duration::from_micros(18_999), 3_840, 48_000, 2),
            || now,
        );
        assert_eq!(beyond_regression.metrics().pts_regression_count, 1);
    }

    #[test]
    fn abnormal_pcm_timing_saturates() {
        let timing = FrameTiming::new(Duration::MAX, usize::MAX, 1, 1);

        assert_eq!(timing.end(), Duration::MAX);
        assert!(timing.sample_frames > 0);
    }

    #[test]
    fn active_buffered_range_starts_after_first_successful_submit() {
        let now = Instant::now();
        let mut tracker = tracker(now);

        tracker.observe_buffered(at(30));
        tracker.observe_buffered(Duration::ZERO);

        assert_eq!(
            tracker.metrics().observed_buffered_current,
            Some(Duration::ZERO)
        );
        assert_eq!(tracker.metrics().post_submit_buffered_min, None);
        assert_eq!(tracker.metrics().post_submit_buffered_max, None);

        tracker.observe_write(3_840, 48_000, 2, true, || now);
        tracker.observe_buffered(at(10));
        tracker.observe_buffered(at(50));

        assert_eq!(tracker.metrics().observed_buffered_current, Some(at(50)));
        assert_eq!(tracker.metrics().post_submit_buffered_min, Some(at(10)));
        assert_eq!(tracker.metrics().post_submit_buffered_max, Some(at(50)));
    }

    #[test]
    fn first_decode_and_submit_latency_are_recorded_once() {
        let selected = Instant::now();
        let mut tracker = tracker(selected);

        let first_decode = tracker.observe_frame(FrameTiming::new(at(0), 3_840, 48_000, 2), || {
            selected + at(4)
        });
        let second_decode = tracker
            .observe_frame(FrameTiming::new(at(10), 3_840, 48_000, 2), || {
                selected + at(8)
            });
        let first_submit = tracker.observe_write(3_840, 48_000, 2, true, || selected + at(12));
        let second_submit = tracker.observe_write(3_840, 48_000, 2, true, || selected + at(16));

        assert_eq!(first_decode, Some(at(4)));
        assert_eq!(second_decode, None);
        assert_eq!(first_submit, Some(at(12)));
        assert_eq!(second_submit, None);
    }

    #[test]
    fn latency_clock_providers_are_only_called_for_first_events() {
        let selected = Instant::now();
        let calls = Cell::new(0);
        let mut tracker = tracker(selected);

        let mut now = || {
            calls.set(calls.get() + 1);
            selected + at(4)
        };
        tracker.observe_frame(FrameTiming::new(at(0), 3_840, 48_000, 2), &mut now);
        tracker.observe_frame(FrameTiming::new(at(10), 3_840, 48_000, 2), &mut now);
        tracker.observe_write(1_920, 48_000, 2, false, &mut now);
        tracker.observe_write(3_840, 48_000, 2, true, &mut now);
        tracker.observe_write(3_840, 48_000, 2, true, &mut now);

        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn failed_write_does_not_count_as_submitted_or_set_first_submit() {
        let selected = Instant::now();
        let mut tracker = tracker(selected);

        let failed = tracker.observe_write(1_920, 48_000, 2, false, || selected + at(1));
        let submitted = tracker.observe_write(3_840, 48_000, 2, true, || selected + at(2));

        assert_eq!(failed, None);
        assert_eq!(submitted, Some(at(2)));
        assert_eq!(tracker.metrics().sink_write_attempts, 2);
        assert_eq!(tracker.metrics().sink_write_failures, 1);
        assert_eq!(tracker.metrics().submitted_chunks, 1);
        assert_eq!(tracker.metrics().submitted_bytes, 3_840);
        assert_eq!(tracker.metrics().submitted_sample_frames, 480);
        assert_eq!(tracker.metrics().submitted_duration, at(10));
        assert_eq!(tracker.metrics().max_submitted_chunk_duration, at(10));
    }

    #[test]
    fn pacing_delay_has_a_strict_ceiling_boundary() {
        assert_eq!(
            pacing_delay(crate::runtime::playback_sync::AUDIO_PACING_CEILING),
            None
        );
        assert_eq!(
            pacing_delay(
                crate::runtime::playback_sync::AUDIO_PACING_CEILING + Duration::from_micros(1)
            ),
            Some(Duration::from_micros(1))
        );
    }

    #[test]
    fn requested_pacing_delays_saturate_and_only_enter_warning_state_once() {
        let now = Instant::now();
        let mut tracker = tracker(now);

        assert!(!tracker.logged_pacing_delay_requested);
        tracker.observe_pacing_delay_requested(Duration::MAX);
        assert!(tracker.logged_pacing_delay_requested);
        tracker.observe_pacing_delay_requested(Duration::MAX);

        assert_eq!(tracker.metrics().pacing_delay_requested_count, 2);
        assert_eq!(
            tracker.metrics().pacing_delay_requested_total,
            Duration::MAX
        );
        assert_eq!(tracker.metrics().pacing_delay_requested_max, Duration::MAX);
        assert!(tracker.logged_pacing_delay_requested);
    }

    #[test]
    fn periodic_summary_uses_a_low_frequency_boundary() {
        let selected = Instant::now();
        let mut tracker = tracker(selected);

        assert!(!tracker.summary_due(selected + SUMMARY_INTERVAL - at(1)));
        assert!(tracker.summary_due(selected + SUMMARY_INTERVAL));
        tracker.maybe_log_summary(selected + SUMMARY_INTERVAL);
        assert!(!tracker.summary_due(selected + SUMMARY_INTERVAL + at(1)));
        assert!(tracker.summary_due(selected + SUMMARY_INTERVAL * 2));
    }

    #[test]
    fn tracker_state_is_generation_scoped() {
        let now = Instant::now();
        let mut old = Tracker::new(
            7,
            "screen",
            "audio",
            "opus",
            now,
            Arc::new(TeardownControl::default()),
        );
        let fresh = Tracker::new(
            8,
            "screen",
            "audio",
            "opus",
            now,
            Arc::new(TeardownControl::default()),
        );

        old.observe_buffered(at(25));

        assert_eq!(old.generation(), 7);
        assert_eq!(old.metrics().observed_buffered_current, Some(at(25)));
        assert_eq!(fresh.generation(), 8);
        assert_eq!(fresh.metrics().observed_buffered_current, None);
    }

    #[test]
    fn teardown_reasons_remain_distinct() {
        assert_eq!(TeardownReason::Stop.as_str(), "stop");
        assert_eq!(TeardownReason::Replacement.as_str(), "replacement");
        assert_eq!(TeardownReason::Withdraw.as_str(), "withdraw");
        assert_eq!(TeardownReason::Ended.as_str(), "ended");
        assert_eq!(TeardownReason::DecodeError.as_str(), "decode_error");
        assert_eq!(TeardownReason::SinkError.as_str(), "sink_error");
        assert_eq!(TeardownReason::StartError.as_str(), "start_error");
    }

    #[test]
    fn owner_cancellation_emits_one_complete_teardown_reason() {
        for (owner, expected) in [
            (OwnerTeardownReason::Stop, TeardownReason::Stop),
            (
                OwnerTeardownReason::Replacement,
                TeardownReason::Replacement,
            ),
            (OwnerTeardownReason::Withdraw, TeardownReason::Withdraw),
        ] {
            let teardown = Arc::new(TeardownControl::default());
            let mut tracker = tracker_with_control(Instant::now(), teardown.clone());
            tracker.observe_buffered(at(25));
            teardown.set_owner_reason(owner);

            drop(tracker);

            assert_eq!(teardown.emitted_reason(), Some(expected));
        }
    }

    #[test]
    fn natural_teardown_wins_once_when_owner_reason_is_already_set() {
        for natural in [
            TeardownReason::Ended,
            TeardownReason::DecodeError,
            TeardownReason::SinkError,
            TeardownReason::StartError,
        ] {
            let teardown = Arc::new(TeardownControl::default());
            teardown.set_owner_reason(OwnerTeardownReason::Stop);
            tracker_with_control(Instant::now(), teardown.clone()).finish(natural);

            assert_eq!(teardown.emitted_reason(), Some(natural));
        }
    }

    #[test]
    fn cancellation_before_tracker_creation_has_no_empty_summary() {
        let teardown = TeardownControl::default();
        teardown.set_owner_reason(OwnerTeardownReason::Stop);

        assert!(!teardown.summary_emitted());
        assert_eq!(teardown.emitted_reason(), None);
    }
}
