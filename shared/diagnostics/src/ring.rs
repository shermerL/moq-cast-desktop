use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, TryLockError};

/// A severity attached to one captured diagnostic event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    /// An operation failed and requires attention.
    Error,
    /// An operation degraded or recovered from a failure.
    Warn,
    /// A normal lifecycle or state transition.
    Info,
    /// Detailed diagnostics enabled for an allowed target.
    Debug,
}

impl LogLevel {
    /// Return an uppercase display label.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Error => "ERROR",
            Self::Warn => "WARN",
            Self::Info => "INFO",
            Self::Debug => "DEBUG",
        }
    }
}

/// One immutable diagnostic entry retained by the in-application ring.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub(crate) timestamp: String,
    pub(crate) level: LogLevel,
    pub(crate) target: String,
    pub(crate) thread: String,
    pub(crate) event: String,
    pub(crate) line: String,
}

impl Entry {
    /// Return the UTC RFC 3339 timestamp.
    pub fn timestamp(&self) -> &str {
        &self.timestamp
    }

    /// Return the event severity.
    pub fn level(&self) -> LogLevel {
        self.level
    }

    /// Return the tracing target.
    pub fn target(&self) -> &str {
        &self.target
    }

    /// Return the emitting thread identity.
    pub fn thread(&self) -> &str {
        &self.thread
    }

    /// Return the formatted event message and structured fields.
    pub fn event(&self) -> &str {
        &self.event
    }

    /// Return the stable UTF-8 line written to the local log file.
    pub fn line(&self) -> &str {
        &self.line
    }
}

/// A stable bounded copy of the in-application diagnostic ring.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub(crate) revision: u64,
    pub(crate) entries: Vec<Entry>,
    pub(crate) dropped: u64,
}

impl Snapshot {
    /// Return the ring revision represented by this snapshot.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Return entries ordered from oldest to newest.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Return the observed count of diagnostics dropped before persistence or ring insertion.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }
}

pub(crate) struct Ring {
    capacity: usize,
    entries: Mutex<VecDeque<(u64, Entry)>>,
    details: Mutex<VecDeque<(u64, Entry)>>,
    revision: AtomicU64,
    contention_dropped: AtomicU64,
}

impl Ring {
    pub(crate) fn new(capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            capacity: capacity.max(1),
            entries: Mutex::new(VecDeque::with_capacity(capacity.max(1))),
            details: Mutex::new(VecDeque::with_capacity(capacity.max(1))),
            revision: AtomicU64::new(0),
            contention_dropped: AtomicU64::new(0),
        })
    }

    pub(crate) fn push(&self, entry: Entry) {
        // Detailed traffic never holds the lifecycle ring lock or evicts its entries.
        let lane = if entry.level == LogLevel::Debug {
            &self.details
        } else {
            &self.entries
        };
        let mut entries = match lane.try_lock() {
            Ok(entries) => entries,
            Err(TryLockError::Poisoned(error)) => error.into_inner(),
            Err(TryLockError::WouldBlock) => {
                self.contention_dropped.fetch_add(1, Ordering::Relaxed);
                return;
            }
        };
        if entries.len() == self.capacity {
            entries.pop_front();
        }
        let sequence = self.revision.fetch_add(1, Ordering::AcqRel);
        entries.push_back((sequence, entry));
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }

    pub(crate) fn snapshot(&self, dropped: u64) -> Snapshot {
        let entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let details = self
            .details
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut ordered = entries.iter().chain(details.iter()).collect::<Vec<_>>();
        ordered.sort_unstable_by_key(|(sequence, _)| sequence);
        Snapshot {
            revision: self.revision(),
            entries: ordered
                .into_iter()
                .map(|(_, entry)| entry.clone())
                .collect(),
            dropped,
        }
    }

    pub(crate) fn contention_dropped(&self) -> u64 {
        self.contention_dropped.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(event: &str) -> Entry {
        Entry {
            timestamp: "2026-08-25T00:00:00Z".to_owned(),
            level: LogLevel::Info,
            target: "test".to_owned(),
            thread: "test".to_owned(),
            event: event.to_owned(),
            line: format!("{event}\n"),
        }
    }

    #[test]
    fn ring_keeps_bounded_oldest_to_newest_order() {
        let ring = Ring::new(3);
        for event in ["one", "two", "three", "four"] {
            ring.push(entry(event));
        }

        let snapshot = ring.snapshot(0);
        let events = snapshot
            .entries()
            .iter()
            .map(Entry::event)
            .collect::<Vec<_>>();
        assert_eq!(events, ["two", "three", "four"]);
        assert_eq!(snapshot.revision(), 4);
    }

    #[test]
    fn contended_ring_drops_instead_of_blocking_the_emitter() {
        let ring = Ring::new(3);
        let held = ring.entries.lock().unwrap();
        ring.push(entry("dropped"));
        drop(held);

        assert_eq!(ring.contention_dropped(), 1);
        assert!(ring.snapshot(1).entries().is_empty());
    }
    #[test]
    fn detailed_lane_contention_cannot_drop_a_lifecycle_event() {
        let ring = Ring::new(2);
        let held = ring.details.lock().unwrap();
        ring.push(entry("connection"));
        let mut detail = entry("packet");
        detail.level = LogLevel::Debug;
        ring.push(detail);
        drop(held);
        assert_eq!(ring.contention_dropped(), 1);
        assert_eq!(ring.snapshot(1).entries()[0].event(), "connection");
    }

    #[test]
    fn both_lanes_are_bounded_and_snapshot_preserves_insertion_order() {
        let ring = Ring::new(2);
        for n in 0..4 {
            ring.push(entry(&format!("key{n}")));
            let mut detail = entry(&format!("detail{n}"));
            detail.level = LogLevel::Debug;
            ring.push(detail);
        }
        let snapshot = ring.snapshot(0);
        assert_eq!(
            snapshot
                .entries()
                .iter()
                .map(Entry::event)
                .collect::<Vec<_>>(),
            ["key2", "detail2", "key3", "detail3"]
        );
    }
}
