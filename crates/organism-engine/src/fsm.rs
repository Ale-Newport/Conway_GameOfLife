//! A tiny finite-state-machine helper with a bounded transition log.
//!
//! [`StateMachine`] does not own transition rules: models decide the next
//! state themselves (often with [`crate::utility::choose_best`]) and call
//! [`StateMachine::transition`]. The helper tracks how long the agent has been
//! in its state (in ticks) and keeps the most recent transitions for
//! inspection panels and debugging.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

/// Default number of transitions kept by [`StateMachine::new`].
pub const DEFAULT_LOG_CAPACITY: usize = 16;

/// A recorded state change.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transition<S> {
    /// State left.
    pub from: S,
    /// State entered.
    pub to: S,
    /// Tick of the change.
    pub tick: u64,
}

/// Current state, entry tick and a bounded log of recent transitions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateMachine<S> {
    state: S,
    entered_at: u64,
    log: VecDeque<Transition<S>>,
    log_capacity: usize,
    transitions: u64,
}

impl<S: Clone + PartialEq> StateMachine<S> {
    /// Machine in `initial` since `tick`, keeping [`DEFAULT_LOG_CAPACITY`] transitions.
    pub fn new(initial: S, tick: u64) -> Self {
        Self::with_log_capacity(initial, tick, DEFAULT_LOG_CAPACITY)
    }

    /// Machine keeping at most `capacity` transitions (0 disables the log).
    pub fn with_log_capacity(initial: S, tick: u64, capacity: usize) -> Self {
        Self { state: initial, entered_at: tick, log: VecDeque::new(), log_capacity: capacity, transitions: 0 }
    }

    /// Current state.
    pub fn state(&self) -> &S {
        &self.state
    }

    /// Tick at which the current state was entered.
    pub fn entered_at(&self) -> u64 {
        self.entered_at
    }

    /// Ticks spent in the current state as of `now` (0 on the entry tick,
    /// also 0 if `now` precedes the entry tick).
    pub fn time_in_state(&self, now: u64) -> u64 {
        now.saturating_sub(self.entered_at)
    }

    /// Switches to `to` at tick `now`. Returns `false` and changes nothing if
    /// the machine is already in `to` (time in state keeps accumulating).
    pub fn transition(&mut self, to: S, now: u64) -> bool {
        if self.state == to {
            return false;
        }
        let from = std::mem::replace(&mut self.state, to.clone());
        self.entered_at = now;
        self.transitions += 1;
        if self.log_capacity > 0 {
            while self.log.len() >= self.log_capacity {
                self.log.pop_front();
            }
            self.log.push_back(Transition { from, to, tick: now });
        }
        true
    }

    /// The state before the most recent logged transition.
    pub fn previous(&self) -> Option<&S> {
        self.log.back().map(|t| &t.from)
    }

    /// Recent transitions, oldest first.
    pub fn log(&self) -> impl DoubleEndedIterator<Item = &Transition<S>> + ExactSizeIterator {
        self.log.iter()
    }

    /// Total number of transitions since creation (including ones dropped
    /// from the log).
    pub fn transitions(&self) -> u64 {
        self.transitions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
    enum Mode {
        Idle,
        Forage,
        Flee,
    }

    #[test]
    fn tracks_time_in_state_and_logs_transitions() {
        let mut fsm = StateMachine::new(Mode::Idle, 10);
        assert_eq!(fsm.time_in_state(15), 5);
        assert!(!fsm.transition(Mode::Idle, 15), "same state is not a transition");
        assert_eq!(fsm.time_in_state(15), 5);
        assert!(fsm.transition(Mode::Forage, 15));
        assert_eq!((fsm.state(), fsm.entered_at(), fsm.time_in_state(15)), (&Mode::Forage, 15, 0));
        assert!(fsm.transition(Mode::Flee, 22));
        assert_eq!(fsm.previous(), Some(&Mode::Forage));
        let log: Vec<_> = fsm.log().map(|t| (t.from, t.to, t.tick)).collect();
        assert_eq!(log, vec![(Mode::Idle, Mode::Forage, 15), (Mode::Forage, Mode::Flee, 22)]);
        assert_eq!(fsm.time_in_state(5), 0, "clock running backwards saturates");
    }

    #[test]
    fn log_is_bounded() {
        let mut fsm = StateMachine::with_log_capacity(0u8, 0, 3);
        for t in 1..=10u8 {
            fsm.transition(t, u64::from(t));
        }
        assert_eq!(fsm.log().len(), 3);
        assert_eq!(fsm.log().next().map(|t| t.tick), Some(8));
        assert_eq!(fsm.transitions(), 10);
        let mut silent = StateMachine::with_log_capacity(0u8, 0, 0);
        silent.transition(1, 1);
        assert_eq!((silent.log().len(), silent.transitions(), silent.previous()), (0, 1, None));
    }

    #[test]
    fn serde_round_trip() {
        let mut fsm = StateMachine::new(Mode::Idle, 0);
        fsm.transition(Mode::Flee, 3);
        let json = serde_json::to_string(&fsm).unwrap();
        assert_eq!(serde_json::from_str::<StateMachine<Mode>>(&json).unwrap(), fsm);
    }
}
