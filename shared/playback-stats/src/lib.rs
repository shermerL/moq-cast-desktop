//! Session-owned, opt-in observations of decoded output and unique UI frames.
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

const PERIOD: Duration = Duration::from_secs(1);
const WINDOW: Duration = Duration::from_secs(60);
const POINTS: usize = 61;
const EVENTS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    pub start: f64,
    pub at: f64,
    pub decoded_fps: Option<f64>,
    pub ui_fps: Option<f64>,
    pub codec_generation: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkKind {
    User,
    DecoderChanged,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mark {
    pub at: f64,
    pub kind: MarkKind,
}

#[derive(Debug, Clone)]
pub struct StatsSnapshot {
    pub generation: u64,
    pub enabled: bool,
    pub now: f64,
    pub samples: Vec<Sample>,
    pub marks: Vec<Mark>,
    pub missed: u64,
    pub overwritten_marks: u64,
}

#[derive(Default, Debug, Clone)]
pub struct PlaybackStats(Arc<Mutex<Option<Arc<Session>>>>);

#[derive(Debug)]
pub struct WatchSession {
    owner: PlaybackStats,
    session: Arc<Session>,
}

#[derive(Default, Debug)]
struct Counts {
    enabled: AtomicBool,
    decoded: AtomicU64,
}

#[derive(Clone, Debug)]
pub struct DecoderCounter(Arc<Counts>);

impl DecoderCounter {
    pub fn decoded(&self) {
        if self.0.enabled.load(Relaxed) {
            self.0.decoded.fetch_add(1, Relaxed);
        }
    }
}

#[derive(Debug)]
struct Decoder {
    generation: u64,
    counts: Arc<Counts>,
    accepted: u64,
    last_sequence: Option<u64>,
}

#[derive(Debug, Default)]
struct History {
    enabled: bool,
    frozen_at: Option<Duration>,
    closed: bool,
    worker: bool,
    decoder: Option<Decoder>,
    had_decoder: bool,
    baseline: Option<(Duration, u64, u64)>,
    deadline: Duration,
    samples: VecDeque<Sample>,
    marks: VecDeque<Mark>,
    missed: u64,
    overwritten_marks: u64,
}

impl History {
    fn baseline_at(&mut self, at: Duration) {
        self.baseline = self
            .decoder
            .as_ref()
            .map(|d| (at, d.counts.decoded.load(Relaxed), d.accepted));
    }

    fn gap(&mut self, start: Duration, end: Duration) {
        self.point(Sample {
            start: start.as_secs_f64(),
            at: end.as_secs_f64(),
            decoded_fps: None,
            ui_fps: None,
            codec_generation: self.decoder.as_ref().map(|d| d.generation),
        });
    }

    fn point(&mut self, point: Sample) {
        if self.samples.len() == POINTS {
            self.samples.pop_front();
        }
        self.samples.push_back(point);
    }

    fn prune(&mut self, now: Duration) {
        let oldest = now.saturating_sub(WINDOW).as_secs_f64();
        while self.samples.front().is_some_and(|s| s.start < oldest) {
            self.samples.pop_front();
        }
        while self.marks.front().is_some_and(|m| m.at < oldest) {
            self.marks.pop_front();
        }
    }

    fn mark(&mut self, now: Duration, kind: MarkKind) {
        self.prune(now);
        if self.marks.len() == EVENTS {
            self.marks.pop_front();
            self.overwritten_marks = self.overwritten_marks.saturating_add(1);
        }
        self.marks.push_back(Mark {
            at: now.as_secs_f64(),
            kind,
        });
    }

    fn set_enabled(&mut self, now: Duration, enabled: bool) {
        if let Some(d) = &self.decoder {
            d.counts.enabled.store(enabled, Relaxed);
        }
        self.enabled = enabled;
        if enabled {
            self.frozen_at = None;
            self.prune(now);
            if !self.samples.is_empty() {
                self.gap(now, now);
            }
            self.baseline_at(now);
            self.deadline = now + PERIOD;
        } else {
            self.frozen_at = Some(now);
            self.baseline = None;
        }
    }

    fn tick(&mut self, now: Duration, interrupted: bool) {
        if !self.enabled || now < self.deadline {
            return;
        }
        let skipped = (now - self.deadline).as_secs();
        // The same observation must end this interval and begin the next. The reader
        // can increment the atomic counter while the sampler holds the history lock.
        let current = self
            .decoder
            .as_ref()
            .map(|d| (now, d.counts.decoded.load(Relaxed), d.accepted));
        let previous_deadline = self.deadline;
        self.deadline += Duration::from_secs(skipped + 1);
        if skipped != 0 || interrupted {
            self.missed = self.missed.saturating_add(skipped);
            let start = self.baseline.map_or(previous_deadline - PERIOD, |b| b.0);
            // Only the portion inside the visible window can be retained after a long suspend.
            self.gap(start.max(now.saturating_sub(WINDOW)), now);
        } else if let (Some((start, decoded, accepted)), Some((_, next_decoded, next_accepted))) =
            (self.baseline, current)
        {
            let elapsed = (now - start).as_secs_f64();
            let rate = |new: u64, old| new.checked_sub(old).map(|n| n as f64 / elapsed);
            self.point(Sample {
                start: start.as_secs_f64(),
                at: now.as_secs_f64(),
                decoded_fps: rate(next_decoded, decoded),
                ui_fps: rate(next_accepted, accepted),
                codec_generation: self.decoder.as_ref().map(|d| d.generation),
            });
        } else {
            self.gap(previous_deadline - PERIOD, now);
        }
        self.baseline = current;
        self.prune(now);
    }
}

#[derive(Debug)]
struct Session {
    generation: u64,
    origin: Instant,
    #[cfg(target_os = "linux")]
    boot_origin: Option<Duration>,
    state: Mutex<History>,
    changed: Condvar,
}

#[cfg(target_os = "linux")]
fn boot_time() -> Option<Duration> {
    let mut ts = std::mem::MaybeUninit::<libc::timespec>::uninit();
    // CLOCK_BOOTTIME is monotonic and includes system suspend on Linux.
    let status = unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, ts.as_mut_ptr()) };
    if status != 0 {
        return None;
    }
    let ts = unsafe { ts.assume_init() };
    Some(Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32))
}

impl Session {
    fn now(&self) -> Duration {
        #[cfg(target_os = "linux")]
        if let Some(origin) = self.boot_origin
            && let Some(now) = boot_time()
        {
            return now.saturating_sub(origin);
        }
        self.origin.elapsed()
    }

    fn close(&self) {
        let mut h = self.state.lock().unwrap();
        h.closed = true;
        h.set_enabled(self.now(), false);
        self.changed.notify_one();
    }

    fn run(&self) {
        let mut h = self.state.lock().unwrap();
        #[cfg(target_os = "linux")]
        let mut suspend_offset = self.now().saturating_sub(self.origin.elapsed());
        while h.enabled && !h.closed {
            let now = self.now();
            if now >= h.deadline {
                #[cfg(target_os = "linux")]
                let interrupted = {
                    // Two clocks distinguish actual suspend from normal scheduling jitter.
                    // Ignore sub-100ms discrepancies between the consecutive clock reads.
                    let offset = now.saturating_sub(self.origin.elapsed());
                    let changed =
                        offset.saturating_sub(suspend_offset) >= Duration::from_millis(100);
                    suspend_offset = offset;
                    changed
                };
                #[cfg(not(target_os = "linux"))]
                let interrupted = false;
                h.tick(now, interrupted);
            } else {
                let timeout = h.deadline - now;
                h = self.changed.wait_timeout(h, timeout).unwrap().0;
            }
        }
        h.worker = false;
    }
}

impl PlaybackStats {
    pub fn begin(&self, generation: u64) -> WatchSession {
        let session = Arc::new(Session {
            generation,
            origin: Instant::now(),
            #[cfg(target_os = "linux")]
            boot_origin: boot_time(),
            state: Mutex::default(),
            changed: Condvar::new(),
        });
        if let Some(old) = self.0.lock().unwrap().replace(session.clone()) {
            old.close();
        }
        WatchSession {
            owner: self.clone(),
            session,
        }
    }

    fn session(&self, generation: u64) -> Option<Arc<Session>> {
        self.0
            .lock()
            .unwrap()
            .as_ref()
            .filter(|s| s.generation == generation)
            .cloned()
    }

    pub fn set_sampling(&self, generation: u64, enabled: bool) {
        let Some(s) = self.session(generation) else {
            return;
        };
        let mut h = s.state.lock().unwrap();
        if h.closed || h.enabled == enabled {
            return;
        }
        h.set_enabled(s.now(), enabled);
        if enabled && !h.worker {
            let worker = s.clone();
            h.worker = std::thread::Builder::new()
                .name("playback-stats".into())
                .spawn(move || worker.run())
                .is_ok();
            if !h.worker {
                h.set_enabled(s.now(), false);
            }
        }
        s.changed.notify_one();
    }

    pub fn accepted(&self, view_generation: u64, codec_generation: u64, sequence: u64) {
        let Some(s) = self.session(view_generation) else {
            return;
        };
        let mut h = s.state.lock().unwrap();
        let enabled = h.enabled;
        if let Some(d) = h.decoder.as_mut()
            && d.generation == codec_generation
            && d.last_sequence.is_none_or(|last| sequence > last)
        {
            d.last_sequence = Some(sequence);
            if enabled {
                d.accepted = d.accepted.wrapping_add(1);
            }
        }
    }

    pub fn mark(&self, generation: u64) {
        let Some(s) = self.session(generation) else {
            return;
        };
        let mut h = s.state.lock().unwrap();
        if h.enabled {
            h.mark(s.now(), MarkKind::User);
        }
    }

    pub fn is_sampling(&self, generation: u64) -> bool {
        self.session(generation)
            .is_some_and(|s| s.state.lock().unwrap().enabled)
    }

    pub fn snapshot(&self, generation: u64) -> Option<StatsSnapshot> {
        let s = self.session(generation)?;
        let mut h = s.state.lock().unwrap();
        if h.closed {
            return None;
        }
        let now = h.frozen_at.unwrap_or_else(|| s.now());
        h.prune(now);
        Some(StatsSnapshot {
            generation,
            enabled: h.enabled,
            now: now.as_secs_f64(),
            samples: h.samples.iter().copied().collect(),
            marks: h.marks.iter().copied().collect(),
            missed: h.missed,
            overwritten_marks: h.overwritten_marks,
        })
    }
}

impl WatchSession {
    pub fn generation(&self) -> u64 {
        self.session.generation
    }

    pub fn decoder(&self, codec_generation: u64) -> DecoderCounter {
        let counts = Arc::<Counts>::default();
        let decoder = Decoder {
            generation: codec_generation,
            counts: counts.clone(),
            accepted: 0,
            last_sequence: None,
        };
        self.replace_decoder(Some(decoder));
        DecoderCounter(counts)
    }

    pub fn clear_decoder(&self) {
        self.replace_decoder(None);
    }

    fn replace_decoder(&self, decoder: Option<Decoder>) {
        let s = &self.session;
        let mut h = s.state.lock().unwrap();
        if h.closed || (decoder.is_none() && h.decoder.is_none()) {
            return;
        }
        if let Some(old) = &h.decoder {
            old.counts.enabled.store(false, Relaxed);
        }
        if let Some(new) = &decoder {
            new.counts.enabled.store(h.enabled, Relaxed);
        }
        let rebuilt = decoder.is_some() && h.had_decoder;
        h.had_decoder |= decoder.is_some();
        h.decoder = decoder;
        if h.enabled {
            let now = s.now();
            let start = h.baseline.map_or(now, |b| b.0);
            h.gap(start, now);
            if rebuilt {
                h.mark(now, MarkKind::DecoderChanged);
            }
            h.baseline_at(now);
            h.deadline = now + PERIOD;
            s.changed.notify_one();
        }
    }
}

impl Drop for WatchSession {
    fn drop(&mut self) {
        let mut current = self.owner.0.lock().unwrap();
        if current
            .as_ref()
            .is_some_and(|s| Arc::ptr_eq(s, &self.session))
        {
            current.take();
        }
        self.session.close();
    }
}

#[cfg(test)]
mod tests;
