//! Error type of the organism engine.

use simulation_core::SimError;
use thiserror::Error;

use crate::id::AgentId;

/// Errors raised while validating agent data (gene schemas, response curves,
/// drives, serialised stores) or recording genealogy events. Messages are
/// written for end users because most of these values come from scenario and
/// species files.
#[derive(Debug, Error, Clone, PartialEq)]
pub enum OrganismError {
    /// A gene declaration is inconsistent (bad range, default or mutation size).
    #[error("gene `{gene}`: {reason}")]
    InvalidGene {
        /// Name of the offending gene.
        gene: String,
        /// What is wrong with it.
        reason: String,
    },
    /// Two genes of a schema share a name.
    #[error("gene name `{0}` is declared more than once")]
    DuplicateGene(String),
    /// A genome does not have one value per schema gene.
    #[error("genome has {found} genes but the schema declares {expected}")]
    GenomeLength {
        /// Number of genes declared by the schema.
        expected: usize,
        /// Number of values in the genome.
        found: usize,
    },
    /// A normalised gene value is not a finite number in `[0, 1]`.
    #[error("gene value {value} at index {index} is not a finite number in [0, 1]")]
    InvalidGeneValue {
        /// Gene index.
        index: usize,
        /// Rejected value.
        value: f32,
    },
    /// A gene index outside the schema.
    #[error("gene index {index} is out of range for a schema of {len} genes")]
    GeneIndex {
        /// Requested index.
        index: usize,
        /// Number of genes in the schema.
        len: usize,
    },
    /// A response curve definition is invalid.
    #[error("invalid response curve: {0}")]
    InvalidCurve(String),
    /// A drive definition is invalid.
    #[error("invalid drive: {0}")]
    InvalidDrive(String),
    /// A birth was recorded twice for the same agent.
    #[error("agent {0} is already recorded in the genealogy")]
    DuplicateBirth(AgentId),
    /// An agent that the genealogy does not know.
    #[error("agent {0} is not recorded in the genealogy")]
    UnknownAgent(AgentId),
    /// A death was recorded twice for the same agent.
    #[error("agent {0} is already recorded as dead")]
    AlreadyDead(AgentId),
    /// A lineage names the agent itself as one of its parents.
    #[error("agent {0} cannot be its own parent")]
    SelfParent(AgentId),
    /// A birth was recorded with an id that is not larger than the last
    /// recorded birth (ids are issued in increasing order and never reused).
    #[error("birth of agent {id} recorded after agent {last}: births must be recorded in increasing id order")]
    BirthOutOfOrder {
        /// The rejected agent.
        id: AgentId,
        /// The last birth recorded so far.
        last: AgentId,
    },
    /// A lineage names a parent that is younger than the child (a larger id).
    #[error("agent {child} names {parent} as a parent, but a parent must be older (have a smaller id)")]
    ParentNotOlder {
        /// The newborn.
        child: AgentId,
        /// The offending parent id.
        parent: AgentId,
    },
    /// Serialised agent-store data violates the store invariants.
    #[error("invalid agent store data: {0}")]
    InvalidStore(String),
    /// Serialised genealogy data violates the genealogy invariants.
    #[error("invalid genealogy data: {0}")]
    InvalidGenealogy(String),
    /// More agents than a [`simulation_core::SpatialGrid`] can index.
    #[error("{0} agents exceed the spatial index capacity of {max} entries", max = u32::MAX)]
    TooManyAgents(usize),
}

impl From<OrganismError> for SimError {
    fn from(error: OrganismError) -> Self {
        let message = error.to_string();
        match error {
            OrganismError::InvalidStore(_) | OrganismError::InvalidGenealogy(_) => SimError::Serialization(message),
            OrganismError::DuplicateBirth(_)
            | OrganismError::UnknownAgent(_)
            | OrganismError::AlreadyDead(_)
            | OrganismError::SelfParent(_)
            | OrganismError::BirthOutOfOrder { .. }
            | OrganismError::ParentNotOlder { .. } => SimError::InvalidCommand(message),
            OrganismError::TooManyAgents(_) => SimError::LimitExceeded(message),
            OrganismError::InvalidGene { .. }
            | OrganismError::DuplicateGene(_)
            | OrganismError::GenomeLength { .. }
            | OrganismError::InvalidGeneValue { .. }
            | OrganismError::GeneIndex { .. }
            | OrganismError::InvalidCurve(_)
            | OrganismError::InvalidDrive(_) => SimError::InvalidConfig(message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_to_sim_error_with_readable_message() {
        let e: SimError = OrganismError::DuplicateGene("speed".into()).into();
        assert_eq!(e, SimError::InvalidConfig("gene name `speed` is declared more than once".into()));
        let e: SimError = OrganismError::UnknownAgent(AgentId(7)).into();
        assert_eq!(e, SimError::InvalidCommand("agent #7 is not recorded in the genealogy".into()));
        let e: SimError = OrganismError::TooManyAgents(5_000_000_000).into();
        assert!(matches!(e, SimError::LimitExceeded(m) if m.contains("4294967295")));
    }
}
