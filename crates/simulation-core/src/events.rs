//! Bounded log of notable simulation events (births, deaths, fires,
//! extinctions...), used for the per-organism life history and the
//! environment event timeline.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

/// Importance of an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum EventSeverity {
    /// Routine event (feeding, movement decisions...).
    Trace,
    /// Noteworthy event (birth, death...).
    #[default]
    Info,
    /// Significant change (extinction, disaster start...).
    Major,
}

/// One logged event.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SimEvent {
    /// Tick at which it happened.
    pub tick: u64,
    /// Machine-readable category, e.g. `"birth"`, `"fire"`.
    pub kind: String,
    /// Human-readable description.
    pub message: String,
    /// Entity the event is about, if any.
    pub subject: Option<u64>,
    /// Secondary entity (prey, partner...), if any.
    pub object: Option<u64>,
    /// Importance.
    pub severity: EventSeverity,
}

impl SimEvent {
    /// New event without subjects.
    pub fn new(tick: u64, kind: &str, message: impl Into<String>) -> Self {
        Self {
            tick,
            kind: kind.to_string(),
            message: message.into(),
            subject: None,
            object: None,
            severity: EventSeverity::Info,
        }
    }

    /// Sets the subject entity.
    pub fn subject(mut self, id: u64) -> Self {
        self.subject = Some(id);
        self
    }

    /// Sets the object entity.
    pub fn object(mut self, id: u64) -> Self {
        self.object = Some(id);
        self
    }

    /// Sets the severity.
    pub fn severity(mut self, severity: EventSeverity) -> Self {
        self.severity = severity;
        self
    }
}

/// Ring buffer of events with a fixed capacity (oldest dropped first).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventLog {
    capacity: usize,
    events: VecDeque<SimEvent>,
    total: u64,
}

impl Default for EventLog {
    fn default() -> Self {
        Self::new(10_000)
    }
}

impl EventLog {
    /// Log keeping at most `capacity` events.
    pub fn new(capacity: usize) -> Self {
        Self { capacity: capacity.max(1), events: VecDeque::new(), total: 0 }
    }

    /// Appends an event.
    pub fn push(&mut self, event: SimEvent) {
        if self.events.len() == self.capacity {
            self.events.pop_front();
        }
        self.events.push_back(event);
        self.total += 1;
    }

    /// Stored events, oldest first.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &SimEvent> {
        self.events.iter()
    }

    /// Up to `n` most recent events, newest first.
    pub fn recent(&self, n: usize) -> impl Iterator<Item = &SimEvent> {
        self.events.iter().rev().take(n)
    }

    /// Events whose subject or object is `id`, oldest first.
    pub fn about(&self, id: u64) -> impl Iterator<Item = &SimEvent> {
        self.events.iter().filter(move |e| e.subject == Some(id) || e.object == Some(id))
    }

    /// Number of stored events.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Whether the log is empty.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Events ever pushed (including dropped ones).
    pub fn total(&self) -> u64 {
        self.total
    }

    /// Removes all events.
    pub fn clear(&mut self) {
        self.events.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacity_and_filters() {
        let mut log = EventLog::new(3);
        for t in 0..5 {
            log.push(SimEvent::new(t, "birth", format!("born {t}")).subject(t % 2));
        }
        assert_eq!(log.len(), 3);
        assert_eq!(log.total(), 5);
        assert_eq!(log.recent(1).next().unwrap().tick, 4);
        assert_eq!(log.about(0).count(), 2);
    }
}
