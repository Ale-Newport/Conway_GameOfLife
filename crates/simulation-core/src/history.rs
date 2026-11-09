//! Time travel through stored snapshots.
//!
//! Two independent mechanisms are provided:
//!
//! * [`SnapshotHistory`] records a snapshot every `interval` ticks inside a
//!   memory budget, allowing the user to jump back to an earlier state and
//!   continue from there.
//! * [`UndoStack`] records the state *before* each user edit so edits can be
//!   undone and redone.
//!
//! Both restore stored states; neither inverts the rules mathematically (most
//! cellular automata are irreversible, so that would be impossible in general).

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::snapshot::Snapshot;

/// Configuration of periodic snapshot recording.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryConfig {
    /// Record one snapshot every `interval` ticks (0 disables recording).
    pub interval: u64,
    /// Total memory budget for stored snapshots.
    pub max_bytes: usize,
    /// Maximum number of stored snapshots.
    pub max_entries: usize,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self { interval: 50, max_bytes: 256 * 1024 * 1024, max_entries: 512 }
    }
}

/// Periodic snapshots within a memory budget (oldest evicted first).
#[derive(Debug, Default)]
pub struct SnapshotHistory {
    config: HistoryConfig,
    entries: VecDeque<Snapshot>,
    bytes: usize,
    skipped_too_large: u64,
}

impl SnapshotHistory {
    /// Empty history with the given configuration.
    pub fn new(config: HistoryConfig) -> Self {
        Self { config, ..Self::default() }
    }

    /// Current configuration.
    pub fn config(&self) -> HistoryConfig {
        self.config
    }

    /// Replaces the configuration, evicting entries that no longer fit.
    pub fn set_config(&mut self, config: HistoryConfig) {
        self.config = config;
        self.enforce_budget();
    }

    /// Whether a snapshot should be captured at `tick`: when nothing is stored
    /// yet, or when at least `interval` ticks passed since the latest entry
    /// (engines that jump many generations per tick are handled too).
    pub fn wants(&self, tick: u64) -> bool {
        self.config.interval > 0
            && self.entries.back().is_none_or(|last| tick > last.tick && tick - last.tick >= self.config.interval)
    }

    /// Stores a snapshot. Snapshots larger than the whole budget are skipped
    /// (and counted) rather than evicting everything else.
    pub fn push(&mut self, snapshot: Snapshot) {
        let size = snapshot.size_bytes();
        if size > self.config.max_bytes {
            self.skipped_too_large += 1;
            return;
        }
        // Keep the history ordered by tick: drop entries from an abandoned future.
        self.truncate_after(snapshot.tick.saturating_sub(1));
        self.bytes += size;
        self.entries.push_back(snapshot);
        self.enforce_budget();
    }

    fn enforce_budget(&mut self) {
        while self.bytes > self.config.max_bytes || self.entries.len() > self.config.max_entries {
            match self.entries.pop_front() {
                Some(old) => self.bytes -= old.size_bytes(),
                None => break,
            }
        }
    }

    /// Removes snapshots taken after `tick` (used when the user goes back in
    /// time and resumes, which creates a new future).
    pub fn truncate_after(&mut self, tick: u64) {
        while self.entries.back().is_some_and(|s| s.tick > tick) {
            if let Some(s) = self.entries.pop_back() {
                self.bytes -= s.size_bytes();
            }
        }
    }

    /// Removes everything.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }

    /// Stored snapshots, oldest first.
    pub fn entries(&self) -> impl Iterator<Item = &Snapshot> {
        self.entries.iter()
    }

    /// Snapshot by index (0 = oldest).
    pub fn get(&self, index: usize) -> Option<&Snapshot> {
        self.entries.get(index)
    }

    /// Latest snapshot taken at or before `tick`.
    pub fn at_or_before(&self, tick: u64) -> Option<&Snapshot> {
        self.entries.iter().rev().find(|s| s.tick <= tick)
    }

    /// Number of stored snapshots.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether no snapshot is stored.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Bytes currently used.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Number of snapshots skipped because they exceeded the whole budget.
    pub fn skipped_too_large(&self) -> u64 {
        self.skipped_too_large
    }
}

/// Snapshot-based undo/redo of user edits within a memory budget.
#[derive(Debug)]
pub struct UndoStack {
    undo: VecDeque<Snapshot>,
    redo: Vec<Snapshot>,
    bytes: usize,
    max_bytes: usize,
}

impl UndoStack {
    /// Empty stack with a memory budget.
    pub fn new(max_bytes: usize) -> Self {
        Self { undo: VecDeque::new(), redo: Vec::new(), bytes: 0, max_bytes }
    }

    /// Records the state before an edit. Clears the redo branch.
    pub fn record(&mut self, before: Snapshot) {
        for s in self.redo.drain(..) {
            self.bytes -= s.size_bytes();
        }
        let size = before.size_bytes();
        if size > self.max_bytes {
            return;
        }
        self.bytes += size;
        self.undo.push_back(before);
        while self.bytes > self.max_bytes {
            match self.undo.pop_front() {
                Some(old) => self.bytes -= old.size_bytes(),
                None => break,
            }
        }
    }

    /// Pops the state to restore for an undo; `current` becomes redoable.
    pub fn undo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let previous = self.undo.pop_back()?;
        self.bytes -= previous.size_bytes();
        self.bytes += current.size_bytes();
        self.redo.push(current);
        Some(previous)
    }

    /// Pops the state to restore for a redo; `current` becomes undoable.
    pub fn redo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let next = self.redo.pop()?;
        self.bytes -= next.size_bytes();
        self.bytes += current.size_bytes();
        self.undo.push_back(current);
        Some(next)
    }

    /// Whether an undo is available.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Whether a redo is available.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Drops all recorded states.
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.bytes = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(tick: u64, size: usize) -> Snapshot {
        Snapshot { model_id: "m".into(), schema_version: 1, tick, payload: vec![0; size] }
    }

    #[test]
    fn history_respects_budget_and_order() {
        let overhead = snap(0, 0).size_bytes();
        let mut h =
            SnapshotHistory::new(HistoryConfig { interval: 10, max_bytes: 3 * (100 + overhead), max_entries: 100 });
        for t in (0..60).step_by(10) {
            assert!(h.wants(t));
            h.push(snap(t, 100));
        }
        assert_eq!(h.len(), 3);
        assert_eq!(h.get(0).unwrap().tick, 30);
        assert_eq!(h.at_or_before(45).unwrap().tick, 40);
        h.push(snap(35, 100)); // branching: drops 40 and 50
        assert_eq!(h.entries().map(|s| s.tick).collect::<Vec<_>>(), vec![30, 35]);
    }

    #[test]
    fn oversized_snapshots_are_skipped() {
        let mut h = SnapshotHistory::new(HistoryConfig { interval: 1, max_bytes: 10, max_entries: 10 });
        h.push(snap(1, 1000));
        assert!(h.is_empty());
        assert_eq!(h.skipped_too_large(), 1);
    }

    #[test]
    fn undo_redo_cycle() {
        let mut u = UndoStack::new(1 << 20);
        u.record(snap(0, 1)); // before edit 1
        u.record(snap(0, 2)); // before edit 2
        let restored = u.undo(snap(0, 3)).unwrap();
        assert_eq!(restored.payload.len(), 2);
        let again = u.redo(snap(0, 2)).unwrap();
        assert_eq!(again.payload.len(), 3);
        assert!(u.can_undo());
        assert!(!u.can_redo());
    }
}
