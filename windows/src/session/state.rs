//! Generation-guarded typed transport state.

use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TransportDirection {
    Inbound,
    Outbound,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TransportPhase {
    Connecting,
    Connected,
    Rejected,
    Failed,
    Disconnected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TransportState {
    generation: u64,
    direction: TransportDirection,
    phase: TransportPhase,
}

impl TransportState {
    pub(crate) fn inbound(generation: u64, phase: TransportPhase) -> Self {
        Self {
            generation,
            direction: TransportDirection::Inbound,
            phase,
        }
    }

    pub(crate) fn outbound(generation: u64, phase: TransportPhase) -> Self {
        Self {
            generation,
            direction: TransportDirection::Outbound,
            phase,
        }
    }

    pub(crate) fn generation(self) -> u64 {
        self.generation
    }

    pub(crate) fn direction(self) -> TransportDirection {
        self.direction
    }

    pub(crate) fn phase(self) -> TransportPhase {
        self.phase
    }
}

#[derive(Default)]
pub(super) struct PeerStates {
    states: BTreeMap<String, TransportState>,
}

impl PeerStates {
    pub(super) fn begin(&mut self, peer: &str) -> TransportState {
        let state =
            TransportState::outbound(self.next_generation(peer), TransportPhase::Connecting);
        self.states.insert(peer.to_owned(), state);
        state
    }

    pub(super) fn transition(
        &mut self,
        peer: &str,
        generation: u64,
        phase: TransportPhase,
    ) -> Option<TransportState> {
        let current = self.states.get(peer)?;
        if current.generation != generation {
            return None;
        }
        let state = TransportState::outbound(generation, phase);
        self.states.insert(peer.to_owned(), state);
        Some(state)
    }

    pub(super) fn disconnect(&mut self, peer: &str) -> Option<TransportState> {
        self.states.get(peer)?;
        let state =
            TransportState::outbound(self.next_generation(peer), TransportPhase::Disconnected);
        self.states.insert(peer.to_owned(), state);
        Some(state)
    }

    #[cfg(test)]
    pub(super) fn get(&self, peer: &str) -> Option<TransportState> {
        self.states.get(peer).copied()
    }

    fn next_generation(&self, peer: &str) -> u64 {
        self.states
            .get(peer)
            .map_or(1, |state| state.generation.saturating_add(1))
    }
}

#[cfg(test)]
mod tests {
    use super::{PeerStates, TransportDirection, TransportPhase, TransportState};

    #[test]
    fn inbound_and_outbound_states_are_typed() {
        assert_eq!(
            TransportState::inbound(1, TransportPhase::Connected).direction(),
            TransportDirection::Inbound
        );
        assert_eq!(
            TransportState::outbound(2, TransportPhase::Connected).direction(),
            TransportDirection::Outbound
        );
    }

    #[test]
    fn old_generation_cannot_override_new_state() {
        let mut states = PeerStates::default();
        let old = states.begin("peer");
        let current = states.begin("peer");
        assert!(
            states
                .transition("peer", old.generation(), TransportPhase::Connected)
                .is_none()
        );
        assert_eq!(states.get("peer"), Some(current));
    }

    #[test]
    fn disconnect_advances_generation_and_discards_late_results() {
        let mut states = PeerStates::default();
        let old = states.begin("peer");
        let disconnected = states.disconnect("peer").expect("known peer");
        assert!(disconnected.generation() > old.generation());
        assert!(
            states
                .transition("peer", old.generation(), TransportPhase::Connected)
                .is_none()
        );
        assert_eq!(states.get("peer"), Some(disconnected));
    }
}
