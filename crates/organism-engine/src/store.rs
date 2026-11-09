//! Agent storage with stable identifiers and deferred structural changes.
//!
//! [`AgentStore`] keeps agents in two parallel `Vec`s (ids and values) sorted
//! by [`AgentId`]. Because ids come from a monotonically increasing counter,
//! new agents are always appended and the order never has to be repaired; a
//! lookup by id is a binary search over the id column (`O(log n)`, no hashing,
//! no iteration-order surprises). Iteration is always in id order, which is the
//! canonical deterministic order used by every model.
//!
//! Structural changes are *deferred*: [`AgentStore::queue_spawn`] reserves an
//! id immediately but the agent only becomes visible after
//! [`AgentStore::commit`]; [`AgentStore::queue_despawn`] likewise takes effect
//! at commit. [`AgentStore::split_mut`] hands out mutable access to the agents
//! *and* the queue at the same time, so a system can iterate while scheduling
//! births and deaths:
//!
//! ```
//! use organism_engine::store::AgentStore;
//!
//! let mut store: AgentStore<u32> = [5, 0, 7].into_iter().collect();
//! let (mut agents, pending) = store.split_mut();
//! for (id, energy) in agents.iter_mut() {
//!     if *energy == 0 {
//!         pending.despawn(id);
//!     } else {
//!         *energy -= 1;
//!         pending.spawn(1);
//!     }
//! }
//! let report = store.commit();
//! assert_eq!(report.despawned.len(), 1);
//! assert_eq!(store.len(), 4);
//! ```

use serde::{Deserialize, Serialize};

use crate::error::OrganismError;
use crate::id::AgentId;

/// Raw value of the first id issued by a new store. `AgentId(0)` is never
/// issued, so dependents may use it as a sentinel in packed formats.
pub const FIRST_AGENT_ID: u64 = 1;

/// Largest id counter accepted when deserialising. No run can spawn 2⁶³
/// agents, and the bound keeps [`PendingChanges::spawn`] from overflowing on
/// crafted snapshot data.
const MAX_ID_COUNTER: u64 = 1 << 63;

/// Spawns and despawns scheduled on an [`AgentStore`], applied by
/// [`AgentStore::commit`]. Also owns the id counter, so ids can be reserved
/// while the agents themselves are mutably borrowed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PendingChanges<T> {
    next_id: u64,
    spawns: Vec<(AgentId, T)>,
    despawns: Vec<AgentId>,
}

impl<T> PendingChanges<T> {
    fn new() -> Self {
        Self { next_id: FIRST_AGENT_ID, spawns: Vec::new(), despawns: Vec::new() }
    }

    /// Schedules `agent` for insertion and returns the id it will have. The id
    /// is reserved immediately (so it can be recorded, e.g. as a child in a
    /// genealogy); the agent becomes visible at the next commit.
    pub fn spawn(&mut self, agent: T) -> AgentId {
        let id = AgentId(self.next_id);
        // 2^64 ids cannot be exhausted by any simulation; plain addition keeps
        // the overflow check of debug builds.
        self.next_id += 1;
        self.spawns.push((id, agent));
        id
    }

    /// Schedules the removal of `id` at the next commit. Removing an id that is
    /// not alive at commit time has no effect and is reported in
    /// [`CommitReport::ignored`]. Despawning an agent queued for spawning in
    /// the same batch removes it right after insertion.
    pub fn despawn(&mut self, id: AgentId) {
        self.despawns.push(id);
    }

    /// Queued spawns in id order.
    pub fn spawns(&self) -> impl ExactSizeIterator<Item = (AgentId, &T)> + DoubleEndedIterator {
        self.spawns.iter().map(|(id, agent)| (*id, agent))
    }

    /// Queued despawns in request order (duplicates included).
    pub fn despawns(&self) -> &[AgentId] {
        &self.despawns
    }

    /// Whether nothing is queued.
    pub fn is_empty(&self) -> bool {
        self.spawns.is_empty() && self.despawns.is_empty()
    }

    /// The id the next spawn will receive.
    pub fn next_id(&self) -> AgentId {
        AgentId(self.next_id)
    }
}

/// Result of [`AgentStore::commit`].
#[derive(Clone, Debug, PartialEq)]
pub struct CommitReport<T> {
    /// Ids inserted by this commit, increasing. Agents that were spawned and
    /// despawned in the same batch appear here and in `despawned`.
    pub spawned: Vec<AgentId>,
    /// Removed agents with their final state, in increasing id order.
    pub despawned: Vec<(AgentId, T)>,
    /// Despawn requests for ids that were not alive (already removed or never
    /// issued), increasing and without duplicates.
    pub ignored: Vec<AgentId>,
}

/// Agents of type `T` with stable ids, id-ordered iteration and deferred
/// spawning/despawning. See the [module documentation](self).
///
/// Serialisation includes the pending queue and the id counter, so a restored
/// store continues issuing exactly the same ids. Deserialisation validates the
/// ordering invariants and rejects corrupted data with
/// [`OrganismError::InvalidStore`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "StoreData<T>")]
pub struct AgentStore<T> {
    ids: Vec<AgentId>,
    agents: Vec<T>,
    pending: PendingChanges<T>,
}

/// Unvalidated serialised form of an [`AgentStore`].
#[derive(Deserialize)]
struct StoreData<T> {
    ids: Vec<AgentId>,
    agents: Vec<T>,
    pending: PendingChanges<T>,
}

impl<T> TryFrom<StoreData<T>> for AgentStore<T> {
    type Error = OrganismError;

    fn try_from(data: StoreData<T>) -> Result<Self, Self::Error> {
        let invalid = |message: String| Err(OrganismError::InvalidStore(message));
        if data.ids.len() != data.agents.len() {
            return invalid(format!("{} ids for {} agents", data.ids.len(), data.agents.len()));
        }
        let next = data.pending.next_id;
        if !(FIRST_AGENT_ID..=MAX_ID_COUNTER).contains(&next) {
            return invalid(format!("the id counter {next} is outside [{FIRST_AGENT_ID}, {MAX_ID_COUNTER}]"));
        }
        // Committed ids, then queued spawns, increase strictly from
        // FIRST_AGENT_ID and stay below the counter.
        let committed = data.ids.iter().map(|id| ("agent", *id));
        let queued = data.pending.spawns.iter().map(|(id, _)| ("queued spawn", *id));
        let mut previous: Option<AgentId> = None;
        for (kind, id) in committed.chain(queued) {
            if id.0 < FIRST_AGENT_ID {
                return invalid(format!("{kind} id {id} is reserved and never issued"));
            }
            if let Some(p) = previous
                && id <= p
            {
                return invalid(format!("{kind} ids are not strictly increasing ({p} before {id})"));
            }
            if id.0 >= next {
                return invalid(format!("{kind} id {id} is not below the id counter {next}"));
            }
            previous = Some(id);
        }
        Ok(Self { ids: data.ids, agents: data.agents, pending: data.pending })
    }
}

impl<T> Default for AgentStore<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> FromIterator<T> for AgentStore<T> {
    /// Builds a store whose agents receive ids `1, 2, 3, ...` in iteration order.
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let mut store = Self::new();
        for agent in iter {
            store.pending.spawn(agent);
        }
        store.commit();
        store
    }
}

impl<T> AgentStore<T> {
    /// Empty store; the first spawned agent gets id [`FIRST_AGENT_ID`].
    pub fn new() -> Self {
        Self { ids: Vec::new(), agents: Vec::new(), pending: PendingChanges::new() }
    }

    /// Number of live (committed) agents.
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// Whether there are no live agents.
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// Whether `id` is a live agent.
    pub fn contains(&self, id: AgentId) -> bool {
        self.index_of(id).is_some()
    }

    /// Position of `id` in id order (also its slot in [`AgentStore::as_slice`]).
    pub fn index_of(&self, id: AgentId) -> Option<usize> {
        self.ids.binary_search(&id).ok()
    }

    /// Shared access to a live agent.
    pub fn get(&self, id: AgentId) -> Option<&T> {
        self.index_of(id).map(|i| &self.agents[i])
    }

    /// Mutable access to a live agent.
    pub fn get_mut(&mut self, id: AgentId) -> Option<&mut T> {
        self.index_of(id).map(|i| &mut self.agents[i])
    }

    /// Mutable access to two distinct live agents at once (e.g. a trade or an
    /// attack). Returns `None` if either is missing or `a == b`.
    pub fn get_pair_mut(&mut self, a: AgentId, b: AgentId) -> Option<(&mut T, &mut T)> {
        pair_mut(&self.ids, &mut self.agents, a, b)
    }

    /// Live ids in increasing order (parallel to [`AgentStore::as_slice`]).
    pub fn ids(&self) -> &[AgentId] {
        &self.ids
    }

    /// Live agents in id order, e.g. for `rayon` parallel iteration.
    pub fn as_slice(&self) -> &[T] {
        &self.agents
    }

    /// Mutable live agents in id order (the ids cannot be changed).
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.agents
    }

    /// Iterates over `(id, agent)` in id order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (AgentId, &T)> + DoubleEndedIterator {
        self.ids.iter().copied().zip(self.agents.iter())
    }

    /// Iterates mutably over `(id, agent)` in id order.
    pub fn iter_mut(&mut self) -> impl ExactSizeIterator<Item = (AgentId, &mut T)> + DoubleEndedIterator {
        self.ids.iter().copied().zip(self.agents.iter_mut())
    }

    /// Schedules a spawn; see [`PendingChanges::spawn`].
    pub fn queue_spawn(&mut self, agent: T) -> AgentId {
        self.pending.spawn(agent)
    }

    /// Schedules a despawn; see [`PendingChanges::despawn`].
    pub fn queue_despawn(&mut self, id: AgentId) {
        self.pending.despawn(id);
    }

    /// The queued changes.
    pub fn pending(&self) -> &PendingChanges<T> {
        &self.pending
    }

    /// The id the next spawn will receive.
    pub fn next_id(&self) -> AgentId {
        self.pending.next_id()
    }

    /// Splits the store into mutable agent access and the change queue, so a
    /// system can mutate agents while scheduling spawns and despawns.
    pub fn split_mut(&mut self) -> (AgentsMut<'_, T>, &mut PendingChanges<T>) {
        (AgentsMut { ids: &self.ids, agents: &mut self.agents }, &mut self.pending)
    }

    /// Applies the queued changes: first all spawns (in id order, appended),
    /// then all despawns. The resulting store contents do not depend on the
    /// order in which despawns were queued.
    pub fn commit(&mut self) -> CommitReport<T> {
        let spawns = std::mem::take(&mut self.pending.spawns);
        let mut despawns = std::mem::take(&mut self.pending.despawns);

        let mut spawned = Vec::with_capacity(spawns.len());
        self.ids.reserve(spawns.len());
        self.agents.reserve(spawns.len());
        for (id, agent) in spawns {
            // Queued ids were reserved after every committed id, so appending
            // keeps the id column sorted (validated on deserialisation too).
            debug_assert!(self.ids.last().is_none_or(|last| *last < id));
            spawned.push(id);
            self.ids.push(id);
            self.agents.push(agent);
        }

        let mut despawned = Vec::new();
        let mut ignored = Vec::new();
        if !despawns.is_empty() {
            despawns.sort_unstable();
            despawns.dedup();
            let ids = std::mem::take(&mut self.ids);
            let agents = std::mem::take(&mut self.agents);
            self.ids.reserve(ids.len().saturating_sub(despawns.len()));
            self.agents.reserve(ids.len().saturating_sub(despawns.len()));
            let mut queue = despawns.into_iter().peekable();
            for (id, agent) in ids.into_iter().zip(agents) {
                while let Some(missing) = queue.next_if(|d| *d < id) {
                    ignored.push(missing);
                }
                if queue.next_if_eq(&id).is_some() {
                    despawned.push((id, agent));
                } else {
                    self.ids.push(id);
                    self.agents.push(agent);
                }
            }
            ignored.extend(queue);
        }
        CommitReport { spawned, despawned, ignored }
    }
}

/// Mutable view of the live agents of an [`AgentStore`], obtained from
/// [`AgentStore::split_mut`] together with the change queue.
#[derive(Debug)]
pub struct AgentsMut<'a, T> {
    ids: &'a [AgentId],
    agents: &'a mut [T],
}

impl<T> AgentsMut<'_, T> {
    /// Number of live agents.
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// Whether there are no live agents.
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// Live ids in increasing order.
    pub fn ids(&self) -> &[AgentId] {
        self.ids
    }

    /// Shared access to a live agent.
    pub fn get(&self, id: AgentId) -> Option<&T> {
        self.ids.binary_search(&id).ok().map(|i| &self.agents[i])
    }

    /// Mutable access to a live agent.
    pub fn get_mut(&mut self, id: AgentId) -> Option<&mut T> {
        self.ids.binary_search(&id).ok().map(|i| &mut self.agents[i])
    }

    /// Mutable access to two distinct live agents; see [`AgentStore::get_pair_mut`].
    pub fn get_pair_mut(&mut self, a: AgentId, b: AgentId) -> Option<(&mut T, &mut T)> {
        pair_mut(self.ids, self.agents, a, b)
    }

    /// Iterates over `(id, agent)` in id order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (AgentId, &T)> + DoubleEndedIterator {
        self.ids.iter().copied().zip(self.agents.iter())
    }

    /// Iterates mutably over `(id, agent)` in id order.
    pub fn iter_mut(&mut self) -> impl ExactSizeIterator<Item = (AgentId, &mut T)> + DoubleEndedIterator {
        self.ids.iter().copied().zip(self.agents.iter_mut())
    }
}

fn pair_mut<'a, T>(ids: &[AgentId], agents: &'a mut [T], a: AgentId, b: AgentId) -> Option<(&'a mut T, &'a mut T)> {
    let ia = ids.binary_search(&a).ok()?;
    let ib = ids.binary_search(&b).ok()?;
    let [x, y] = agents.get_disjoint_mut([ia, ib]).ok()?;
    Some((x, y))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_stable_monotonic_and_iteration_is_in_id_order() {
        let mut store = AgentStore::new();
        let a = store.queue_spawn("a");
        let b = store.queue_spawn("b");
        assert_eq!((a, b), (AgentId(1), AgentId(2)));
        assert!(store.is_empty(), "spawns are deferred");
        assert!(store.get(a).is_none());
        let report = store.commit();
        assert_eq!(report.spawned, vec![a, b]);
        store.queue_despawn(a);
        let c = store.queue_spawn("c");
        store.commit();
        assert_eq!(c, AgentId(3), "ids are never reused");
        assert_eq!(store.ids(), &[b, c]);
        assert_eq!(store.iter().map(|(_, v)| *v).collect::<Vec<_>>(), vec!["b", "c"]);
        assert_eq!(store.get(c), Some(&"c"));
        assert_eq!(store.index_of(c), Some(1));
        assert!(!store.contains(a));
    }

    #[test]
    fn matches_a_btreemap_reference_model() {
        use std::collections::{BTreeMap, BTreeSet};

        let mut rng = simulation_core::SimRng::seed_from_u64(99);
        let mut store: AgentStore<u64> = AgentStore::new();
        let mut reference: BTreeMap<AgentId, u64> = BTreeMap::new();
        let mut issued = 0u64;
        for _ in 0..300 {
            let mut spawns = Vec::new();
            for _ in 0..rng.below(6) {
                let value = rng.next_u64();
                issued += 1;
                assert_eq!(store.queue_spawn(value), AgentId(issued));
                spawns.push((AgentId(issued), value));
            }
            // Despawns mix live ids, ids queued in this batch, dead ids, #0 and unissued ids.
            let mut despawns = BTreeSet::new();
            for _ in 0..rng.below(8) {
                let id = AgentId(rng.below(issued + 3));
                store.queue_despawn(id);
                despawns.insert(id);
            }
            let report = store.commit();

            reference.extend(spawns.iter().copied());
            let (mut despawned, mut ignored) = (Vec::new(), Vec::new());
            for id in despawns {
                match reference.remove(&id) {
                    Some(value) => despawned.push((id, value)),
                    None => ignored.push(id),
                }
            }
            assert_eq!(report.spawned, spawns.iter().map(|(id, _)| *id).collect::<Vec<_>>());
            assert_eq!(report.despawned, despawned);
            assert_eq!(report.ignored, ignored);
            assert!(store.iter().map(|(id, v)| (id, *v)).eq(reference.iter().map(|(id, v)| (*id, *v))));
            assert!(reference.iter().all(|(id, v)| store.get(*id) == Some(v)));
        }
        assert!(store.len() > 10, "the run kept a non-trivial population");
    }

    #[test]
    fn despawn_order_does_not_change_the_result() {
        let base: AgentStore<u32> = (0..10).collect();
        let mut forward = base.clone();
        let mut backward = base.clone();
        for id in [2, 5, 9, 5, 42] {
            forward.queue_despawn(AgentId(id));
        }
        for id in [42, 9, 5, 2] {
            backward.queue_despawn(AgentId(id));
        }
        let rf = forward.commit();
        let rb = backward.commit();
        assert_eq!(forward, backward);
        assert_eq!(rf, rb);
        assert_eq!(rf.despawned, vec![(AgentId(2), 1), (AgentId(5), 4), (AgentId(9), 8)]);
        assert_eq!(rf.ignored, vec![AgentId(42)]);
        assert_eq!(forward.len(), 7);
    }

    #[test]
    fn spawn_and_despawn_in_the_same_batch() {
        let mut store: AgentStore<i32> = AgentStore::new();
        let id = store.queue_spawn(1);
        store.queue_despawn(id);
        let report = store.commit();
        assert_eq!(report.spawned, vec![id]);
        assert_eq!(report.despawned, vec![(id, 1)]);
        assert!(store.is_empty());
    }

    #[test]
    fn split_mut_allows_scheduling_while_iterating() {
        let mut store: AgentStore<u32> = [3, 0, 2].into_iter().collect();
        let (mut agents, pending) = store.split_mut();
        let mut children = Vec::new();
        for (id, energy) in agents.iter_mut() {
            if *energy == 0 {
                pending.despawn(id);
            } else {
                *energy -= 1;
                children.push(pending.spawn(10));
            }
        }
        assert_eq!(agents.get(AgentId(1)), Some(&2));
        assert_eq!(pending.spawns().len(), 2);
        let report = store.commit();
        assert_eq!(report.despawned, vec![(AgentId(2), 0)]);
        assert_eq!(children, vec![AgentId(4), AgentId(5)]);
        assert_eq!(store.iter().map(|(id, v)| (id.0, *v)).collect::<Vec<_>>(), vec![(1, 2), (3, 1), (4, 10), (5, 10)]);
    }

    #[test]
    fn pair_access_requires_distinct_live_agents() {
        let mut store: AgentStore<i32> = [1, 2].into_iter().collect();
        let (x, y) = store.get_pair_mut(AgentId(2), AgentId(1)).unwrap();
        std::mem::swap(x, y);
        assert_eq!(store.as_slice(), &[2, 1]);
        assert!(store.get_pair_mut(AgentId(1), AgentId(1)).is_none());
        assert!(store.get_pair_mut(AgentId(1), AgentId(9)).is_none());
    }

    #[test]
    fn serde_round_trip_preserves_pending_changes_and_counter() {
        let mut store: AgentStore<String> = ["x".to_string(), "y".to_string()].into_iter().collect();
        store.queue_spawn("z".into());
        store.queue_despawn(AgentId(1));
        let json = serde_json::to_string(&store).unwrap();
        let mut restored: AgentStore<String> = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, store);
        let bytes = postcard::to_stdvec(&store).unwrap();
        let from_bytes: AgentStore<String> = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(from_bytes, store);
        restored.commit();
        store.commit();
        assert_eq!(restored.queue_spawn("w".into()), store.queue_spawn("w".into()));
    }

    #[test]
    fn deserialisation_rejects_broken_invariants() {
        let unsorted = r#"{"ids":[2,1],"agents":[0,0],"pending":{"next_id":3,"spawns":[],"despawns":[]}}"#;
        let err = serde_json::from_str::<AgentStore<u8>>(unsorted).unwrap_err().to_string();
        assert!(err.contains("not strictly increasing"), "{err}");
        let counter = r#"{"ids":[1,5],"agents":[0,0],"pending":{"next_id":3,"spawns":[],"despawns":[]}}"#;
        assert!(serde_json::from_str::<AgentStore<u8>>(counter).is_err());
        let lengths = r#"{"ids":[1],"agents":[0,0],"pending":{"next_id":3,"spawns":[],"despawns":[]}}"#;
        assert!(serde_json::from_str::<AgentStore<u8>>(lengths).is_err());
        let spawn = r#"{"ids":[1,2],"agents":[0,0],"pending":{"next_id":3,"spawns":[[2,0]],"despawns":[]}}"#;
        assert!(serde_json::from_str::<AgentStore<u8>>(spawn).is_err());
        // A zero counter would issue the reserved id #0; a huge one would overflow.
        let zero = r#"{"ids":[],"agents":[],"pending":{"next_id":0,"spawns":[],"despawns":[]}}"#;
        let err = serde_json::from_str::<AgentStore<u8>>(zero).unwrap_err().to_string();
        assert!(err.contains("id counter 0 is outside"), "{err}");
        let huge = r#"{"ids":[],"agents":[],"pending":{"next_id":18446744073709551615,"spawns":[],"despawns":[]}}"#;
        assert!(serde_json::from_str::<AgentStore<u8>>(huge).is_err());
        let reserved = r#"{"ids":[],"agents":[],"pending":{"next_id":5,"spawns":[[0,1]],"despawns":[]}}"#;
        let err = serde_json::from_str::<AgentStore<u8>>(reserved).unwrap_err().to_string();
        assert!(err.contains("queued spawn id #0 is reserved"), "{err}");
    }
}
