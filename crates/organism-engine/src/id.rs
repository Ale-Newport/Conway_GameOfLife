//! Stable agent identifiers.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Stable identifier of an agent.
///
/// Ids are issued by an [`crate::store::AgentStore`] from a monotonically
/// increasing counter, are never reused within a store, and order agents by
/// creation time. All deterministic tie-breaking in this crate ("lowest id
/// wins") relies on this ordering. Serialised as a plain integer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgentId(pub u64);

impl AgentId {
    /// The raw integer value (e.g. for [`crate::intent::Claim::target`] or
    /// event subjects).
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for AgentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

impl From<AgentId> for u64 {
    fn from(id: AgentId) -> Self {
        id.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orders_by_value_and_serialises_transparently() {
        assert!(AgentId(3) < AgentId(10));
        assert_eq!(AgentId(42).to_string(), "#42");
        assert_eq!(serde_json::to_string(&AgentId(42)).unwrap(), "42");
        assert_eq!(serde_json::from_str::<AgentId>("42").unwrap(), AgentId(42));
        assert_eq!(u64::from(AgentId(9)), AgentId(9).get());
    }
}
