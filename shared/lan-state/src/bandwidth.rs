//! Latest connection estimates; observing a view never keeps a connection alive.

use std::sync::{Arc, Mutex, Weak};

/// Estimates in bits per second, not throughput or remaining link capacity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sample {
    /// Local congestion controller's send estimate, if available.
    pub send_bps: Option<u64>,
    /// Remote sender's receive-direction report, if available.
    pub recv_bps: Option<u64>,
}

#[derive(Default, Debug)]
struct State {
    epoch: u64,
    sample: Sample,
}

/// A non-owning view of one connection task's current estimates.
#[derive(Clone, Debug, Default)]
pub struct View(Weak<Mutex<State>>);

impl PartialEq for View {
    fn eq(&self, other: &Self) -> bool {
        // Lifecycle snapshots compare identity; telemetry refreshes independently.
        self.0.ptr_eq(&other.0)
    }
}
impl Eq for View {}

impl View {
    /// Read the latest observation, or unknown after its owner has stopped.
    pub fn sample(&self) -> Sample {
        self.0
            .upgrade()
            .map(|state| state.lock().unwrap().sample)
            .unwrap_or_default()
    }
}

/// The connection task's sole writer, dropped when that task closes or is cancelled.
#[derive(Default)]
pub struct Observation(Arc<Mutex<State>>);

impl Observation {
    /// Create a display handle without extending this observation's lifetime.
    pub fn view(&self) -> View {
        View(Arc::downgrade(&self.0))
    }

    /// Start observing a new transport epoch without carrying over old estimates.
    pub fn reset(&self, epoch: u64) {
        *self.0.lock().unwrap() = State {
            epoch,
            sample: Sample::default(),
        };
    }

    /// Accept a sample only for the transport epoch currently being observed.
    pub fn update(&self, epoch: u64, sample: Sample) {
        let mut state = self.0.lock().unwrap();
        if state.epoch == epoch {
            state.sample = sample;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconnect_clears_both_directions_and_rejects_old_results() {
        let owner = Observation::default();
        let view = owner.view();
        owner.reset(1);
        owner.update(
            1,
            Sample {
                send_bps: Some(123),
                recv_bps: Some(456),
            },
        );
        owner.reset(2);
        owner.update(
            1,
            Sample {
                send_bps: Some(999),
                recv_bps: Some(999),
            },
        );
        assert_eq!(view.sample(), Sample::default());
        owner.update(
            2,
            Sample {
                send_bps: None,
                recv_bps: Some(0),
            },
        );
        assert_eq!(
            view.sample(),
            Sample {
                send_bps: None,
                recv_bps: Some(0)
            }
        );
    }

    #[test]
    fn closed_view_cannot_retain_or_contaminate_reopened_owner() {
        let old = Observation::default();
        let old_view = old.view();
        old.update(
            0,
            Sample {
                send_bps: Some(1),
                recv_bps: None,
            },
        );
        let new = Observation::default();
        let new_view = new.view();
        new.update(
            0,
            Sample {
                send_bps: Some(2),
                recv_bps: None,
            },
        );
        old.update(
            0,
            Sample {
                send_bps: Some(3),
                recv_bps: None,
            },
        );
        assert_eq!(new_view.sample().send_bps, Some(2));
        drop(old);
        assert_eq!(old_view.sample(), Sample::default());
        assert_eq!(new_view.sample().send_bps, Some(2));
        drop(new);
        assert_eq!(new_view.sample(), Sample::default());
    }
}
