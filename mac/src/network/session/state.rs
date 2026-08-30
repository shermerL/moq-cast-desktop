//! Generation-guarded outbound transport state.

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

    pub(crate) fn phase(self) -> TransportPhase {
        self.phase
    }

    pub(crate) fn direction(self) -> TransportDirection {
        self.direction
    }
}

#[derive(Default)]
pub(crate) struct PeerStates {
    next_generation: u64,
    states: BTreeMap<String, TransportState>,
}

impl PeerStates {
    pub(crate) fn begin(&mut self, peer: &str) -> TransportState {
        let state = TransportState::outbound(self.fresh_generation(), TransportPhase::Connecting);
        self.states.insert(peer.to_owned(), state);
        state
    }

    pub(crate) fn transition(
        &mut self,
        peer: &str,
        generation: u64,
        phase: TransportPhase,
    ) -> Option<TransportState> {
        let current = self.states.get_mut(peer)?;
        if current.generation != generation {
            return None;
        }
        let state = TransportState::outbound(generation, phase);
        self.states.insert(peer.to_owned(), state);
        Some(state)
    }

    pub(crate) fn disconnect(&mut self, peer: &str) -> Option<TransportState> {
        self.states.remove(peer)?;
        Some(TransportState::outbound(
            self.fresh_generation(),
            TransportPhase::Disconnected,
        ))
    }

    #[cfg(test)]
    pub(super) fn get(&self, peer: &str) -> Option<TransportState> {
        self.states.get(peer).copied()
    }

    fn fresh_generation(&mut self) -> u64 {
        self.next_generation = self.next_generation.saturating_add(1);
        self.next_generation
    }
}

#[cfg(test)]
mod tests {
    use super::{PeerStates, TransportPhase};

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
        assert_eq!(disconnected.phase(), TransportPhase::Disconnected);
        assert_eq!(states.get("peer"), None);
    }
}
