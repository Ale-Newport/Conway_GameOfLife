//! Lineage records and a memory-bounded genealogy.
//!
//! Every agent born in a model can carry a [`Lineage`] (parents, generation,
//! birth tick). A [`Genealogy`] records births and deaths and keeps only the
//! part of the family graph that still matters: a node is kept while the agent
//! is alive or while at least one of its descendants is alive. When the last
//! living descendant of a dead ancestor dies, the ancestor is *pruned*
//! (cascading upwards). Summary counts ([`GenealogySummary`]) survive pruning.
//!
//! **Birth order.** Births must be recorded in increasing [`AgentId`] order
//! and every parent must have a smaller id than its child. Ids from an
//! [`crate::store::AgentStore`] satisfy both naturally (ids are issued by an
//! increasing counter and a parent exists before its child is spawned). The
//! rule guarantees that a parent is always recorded before its children, so
//! parent/child links are complete, and that pruned ids can never be
//! recorded a second time.
//!
//! The graph is a DAG (sexual reproduction gives two parents). Pruning alone
//! bounds memory by the living population plus the ancestry of the living;
//! because that ancestry still grows with the number of generations,
//! [`Genealogy::with_max_nodes`] adds a hard cap that *truncates* the oldest
//! dead ancestors first (their descendants then point to parents that are no
//! longer recorded). Living agents are never removed, so the cap is
//! `max(max_nodes, living)`. Finding the oldest dead node is `O(log n)`.
//!
//! Invariants (validated on deserialisation): every recorded node is alive or
//! has at least one recorded child; parents have smaller ids than their
//! children; parent/child links are symmetric; no recorded id exceeds the
//! last recorded birth.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::error::OrganismError;
use crate::id::AgentId;

/// Ancestry of one agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Lineage {
    /// The agent.
    pub id: AgentId,
    /// Up to two parents (`[mother, None]` for asexual reproduction).
    pub parents: [Option<AgentId>; 2],
    /// 0 for founders, otherwise one more than the older parent's generation.
    pub generation: u32,
    /// Tick of birth.
    pub birth_tick: u64,
}

impl Lineage {
    /// A founder (no recorded parents, generation 0).
    pub fn founder(id: AgentId, birth_tick: u64) -> Self {
        Self { id, parents: [None, None], generation: 0, birth_tick }
    }

    /// Offspring of one or two parents; its generation is one more than the
    /// largest parent generation (saturating).
    pub fn offspring(id: AgentId, first: &Lineage, second: Option<&Lineage>, birth_tick: u64) -> Self {
        let generation = second.map_or(first.generation, |s| s.generation.max(first.generation)).saturating_add(1);
        Self { id, parents: [Some(first.id), second.map(|s| s.id)], generation, birth_tick }
    }

    /// Distinct parent ids in slot order (a self-fertilised agent lists its
    /// single parent once).
    pub fn parent_ids(&self) -> impl Iterator<Item = AgentId> {
        let [a, b] = self.parents;
        a.into_iter().chain(b.filter(|b| Some(*b) != a))
    }

    /// Checks that no parent is the agent itself or younger than it.
    fn check_parents(&self) -> Result<(), OrganismError> {
        for parent in self.parent_ids() {
            if parent == self.id {
                return Err(OrganismError::SelfParent(self.id));
            }
            if parent > self.id {
                return Err(OrganismError::ParentNotOlder { child: self.id, parent });
            }
        }
        Ok(())
    }
}

/// A recorded agent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenealogyNode {
    /// Ancestry as recorded at birth.
    pub lineage: Lineage,
    /// Tick of death, `None` while alive.
    pub death_tick: Option<u64>,
    children: Vec<AgentId>,
}

impl GenealogyNode {
    /// Whether the agent is alive.
    pub fn is_alive(&self) -> bool {
        self.death_tick.is_none()
    }

    /// Recorded children (increasing ids). Children that were pruned are no
    /// longer listed.
    pub fn children(&self) -> &[AgentId] {
        &self.children
    }
}

/// Counters that survive pruning.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenealogySummary {
    /// Births recorded.
    pub births: u64,
    /// Deaths recorded.
    pub deaths: u64,
    /// Agents currently alive.
    pub living: u64,
    /// Nodes removed because no descendant was alive any more.
    pub pruned: u64,
    /// Dead ancestors removed by the node cap although descendants were alive.
    pub truncated: u64,
    /// Largest generation ever recorded.
    pub max_generation: u32,
}

/// Parent → child link, for tree views.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct GenealogyEdge {
    /// The parent.
    pub parent: AgentId,
    /// The child.
    pub child: AgentId,
}

/// Births, deaths and the pruned family graph. See the [module docs](self).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "GenealogyData")]
pub struct Genealogy {
    nodes: BTreeMap<AgentId, GenealogyNode>,
    summary: GenealogySummary,
    max_nodes: Option<usize>,
    /// Id of the last recorded birth; later births must have larger ids.
    last_birth: Option<AgentId>,
    /// Recorded dead nodes, so the cap finds the oldest one without a scan.
    /// Derived from `nodes` (rebuilt on deserialisation).
    #[serde(skip)]
    dead: BTreeSet<AgentId>,
}

/// Unvalidated serialised form of a [`Genealogy`].
#[derive(Deserialize)]
struct GenealogyData {
    nodes: BTreeMap<AgentId, GenealogyNode>,
    summary: GenealogySummary,
    max_nodes: Option<usize>,
    last_birth: Option<AgentId>,
}

impl TryFrom<GenealogyData> for Genealogy {
    type Error = OrganismError;

    fn try_from(data: GenealogyData) -> Result<Self, Self::Error> {
        let fail = |m: String| Err(OrganismError::InvalidGenealogy(m));
        if let Some((&newest, _)) = data.nodes.last_key_value()
            && data.last_birth.is_none_or(|last| last < newest)
        {
            let last = data.last_birth.map_or_else(|| "none".to_string(), |l| l.to_string());
            return fail(format!("node {newest} is newer than the last recorded birth ({last})"));
        }
        let mut dead = BTreeSet::new();
        for (id, node) in &data.nodes {
            if node.lineage.id != *id {
                return fail(format!("node {id} holds the lineage of {}", node.lineage.id));
            }
            if let Err(error) = node.lineage.check_parents() {
                return fail(error.to_string());
            }
            if !node.is_alive() {
                if node.children.is_empty() {
                    return fail(format!("dead node {id} has no recorded descendants and should have been pruned"));
                }
                dead.insert(*id);
            }
            if node.children.windows(2).any(|w| w[0] >= w[1]) {
                return fail(format!("children of {id} are not strictly increasing"));
            }
            for child in &node.children {
                let linked = data.nodes.get(child).is_some_and(|c| c.lineage.parent_ids().any(|p| p == *id));
                if !linked {
                    return fail(format!("child {child} of {id} is missing or does not name it as a parent"));
                }
            }
            for parent in node.lineage.parent_ids() {
                if let Some(p) = data.nodes.get(&parent)
                    && p.children.binary_search(id).is_err()
                {
                    return fail(format!("parent {parent} does not list its child {id}"));
                }
            }
        }
        let living = (data.nodes.len() - dead.len()) as u64;
        if living != data.summary.living {
            return fail(format!("summary counts {} living agents but {living} are recorded", data.summary.living));
        }
        Ok(Self {
            nodes: data.nodes,
            summary: data.summary,
            max_nodes: data.max_nodes,
            last_birth: data.last_birth,
            dead,
        })
    }
}

impl Genealogy {
    /// Genealogy without a node cap (memory bounded by pruning only).
    pub fn new() -> Self {
        Self::default()
    }

    /// Genealogy that truncates the oldest dead ancestors whenever more than
    /// `max_nodes` nodes are recorded.
    pub fn with_max_nodes(max_nodes: usize) -> Self {
        Self { max_nodes: Some(max_nodes), ..Self::default() }
    }

    /// The node cap, if any.
    pub fn max_nodes(&self) -> Option<usize> {
        self.max_nodes
    }

    /// Number of recorded nodes.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether nothing is recorded.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Counters.
    pub fn summary(&self) -> &GenealogySummary {
        &self.summary
    }

    /// Id of the most recent birth; the next birth must have a larger id.
    pub fn last_birth(&self) -> Option<AgentId> {
        self.last_birth
    }

    /// Whether `id` is recorded (alive, or dead with living descendants).
    pub fn contains(&self, id: AgentId) -> bool {
        self.nodes.contains_key(&id)
    }

    /// Recorded node of `id`.
    pub fn get(&self, id: AgentId) -> Option<&GenealogyNode> {
        self.nodes.get(&id)
    }

    /// Recorded nodes in id order.
    pub fn nodes(&self) -> impl ExactSizeIterator<Item = &GenealogyNode> + DoubleEndedIterator {
        self.nodes.values()
    }

    /// Records a birth. The id must be larger than every previously recorded
    /// birth ([`OrganismError::BirthOutOfOrder`], or
    /// [`OrganismError::DuplicateBirth`] if it is still recorded) and every
    /// parent must be older ([`OrganismError::SelfParent`],
    /// [`OrganismError::ParentNotOlder`]). Parents that are not recorded
    /// (founders' ancestors, pruned or truncated agents) are kept in the
    /// lineage but produce no link. Nothing changes when an error is returned.
    pub fn record_birth(&mut self, lineage: Lineage) -> Result<(), OrganismError> {
        let id = lineage.id;
        if self.nodes.contains_key(&id) {
            return Err(OrganismError::DuplicateBirth(id));
        }
        if let Some(last) = self.last_birth
            && id <= last
        {
            return Err(OrganismError::BirthOutOfOrder { id, last });
        }
        lineage.check_parents()?;
        for parent in lineage.parent_ids() {
            if let Some(p) = self.nodes.get_mut(&parent) {
                // `id` exceeds every recorded id, so pushing keeps the list sorted.
                p.children.push(id);
            }
        }
        self.nodes.insert(id, GenealogyNode { lineage, death_tick: None, children: Vec::new() });
        self.last_birth = Some(id);
        self.summary.births += 1;
        self.summary.living += 1;
        self.summary.max_generation = self.summary.max_generation.max(lineage.generation);
        self.enforce_cap();
        Ok(())
    }

    /// Records a death and prunes every node left without living
    /// descendants. A death that leaves a dead ancestor in place lowers the
    /// bound `max(max_nodes, living)`, so the cap is enforced here as well.
    /// Returns the number of nodes pruned (truncations are only counted in
    /// [`GenealogySummary::truncated`]).
    pub fn record_death(&mut self, id: AgentId, tick: u64) -> Result<usize, OrganismError> {
        let node = self.nodes.get_mut(&id).ok_or(OrganismError::UnknownAgent(id))?;
        if !node.is_alive() {
            return Err(OrganismError::AlreadyDead(id));
        }
        node.death_tick = Some(tick);
        self.dead.insert(id);
        self.summary.deaths += 1;
        self.summary.living = self.summary.living.saturating_sub(1);
        let pruned_before = self.summary.pruned;
        self.prune_from(id);
        self.enforce_cap();
        Ok((self.summary.pruned - pruned_before) as usize)
    }

    /// Parent → child links between recorded nodes, ordered by child id and
    /// then parent slot.
    pub fn edges(&self) -> Vec<GenealogyEdge> {
        self.nodes
            .values()
            .flat_map(|node| {
                node.lineage
                    .parent_ids()
                    .filter(|p| self.nodes.contains_key(p))
                    .map(move |parent| GenealogyEdge { parent, child: node.lineage.id })
            })
            .collect()
    }

    /// Removes a node from the graph storage (not from its parents' lists).
    fn remove_node(&mut self, id: AgentId) -> Option<GenealogyNode> {
        let node = self.nodes.remove(&id)?;
        self.dead.remove(&id);
        Some(node)
    }

    /// Removes `start` and then every ancestor left dead and childless.
    fn prune_from(&mut self, start: AgentId) {
        let mut stack = vec![start];
        while let Some(id) = stack.pop() {
            let prunable = self.nodes.get(&id).is_some_and(|n| !n.is_alive() && n.children.is_empty());
            if !prunable {
                continue;
            }
            if let Some(node) = self.remove_node(id) {
                self.summary.pruned += 1;
                stack.extend(self.unlink_from_parents(&node));
            }
        }
    }

    /// Removes `node` from its parents' child lists and returns the parents
    /// that became dead and childless.
    fn unlink_from_parents(&mut self, node: &GenealogyNode) -> Vec<AgentId> {
        let id = node.lineage.id;
        let mut orphaned = Vec::new();
        for parent in node.lineage.parent_ids() {
            if let Some(p) = self.nodes.get_mut(&parent) {
                if let Ok(slot) = p.children.binary_search(&id) {
                    p.children.remove(slot);
                }
                if !p.is_alive() && p.children.is_empty() {
                    orphaned.push(parent);
                }
            }
        }
        orphaned
    }

    /// Truncates the oldest dead ancestors (in id order) until the cap holds
    /// or only living agents remain.
    fn enforce_cap(&mut self) {
        let Some(max) = self.max_nodes else {
            return;
        };
        while self.nodes.len() > max {
            let Some(&oldest_dead) = self.dead.first() else {
                return;
            };
            if let Some(node) = self.remove_node(oldest_dead) {
                self.summary.truncated += 1;
                for parent in self.unlink_from_parents(&node) {
                    self.prune_from(parent);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use simulation_core::SimRng;

    use super::*;

    fn id(n: u64) -> AgentId {
        AgentId(n)
    }

    fn edges(g: &Genealogy) -> Vec<(u64, u64)> {
        g.edges().iter().map(|e| (e.parent.0, e.child.0)).collect()
    }

    #[test]
    fn lineage_generations() {
        let a = Lineage::founder(id(1), 0);
        let b = Lineage::offspring(id(2), &a, None, 5);
        let c = Lineage::offspring(id(3), &b, Some(&a), 9);
        assert_eq!((b.generation, c.generation), (1, 2));
        assert_eq!(c.parents, [Some(id(2)), Some(id(1))]);
        let selfed = Lineage::offspring(id(4), &a, Some(&a), 9);
        assert_eq!(selfed.parent_ids().collect::<Vec<_>>(), vec![id(1)]);
    }

    #[test]
    fn a_dying_line_is_pruned_with_its_ancestors() {
        let mut g = Genealogy::new();
        let a = Lineage::founder(id(1), 0);
        let b = Lineage::offspring(id(2), &a, None, 1);
        let c = Lineage::offspring(id(3), &b, None, 2);
        for l in [a, b, c] {
            g.record_birth(l).unwrap();
        }
        assert_eq!(edges(&g), vec![(1, 2), (2, 3)]);
        assert_eq!(g.record_death(id(1), 3).unwrap(), 0, "ancestor of living agents is kept");
        assert_eq!(g.record_death(id(2), 4).unwrap(), 0);
        assert!(g.contains(id(1)) && !g.get(id(1)).unwrap().is_alive());
        assert_eq!(g.record_death(id(3), 5).unwrap(), 3, "the whole line goes");
        assert!(g.is_empty());
        let s = g.summary();
        assert_eq!((s.births, s.deaths, s.living, s.pruned, s.max_generation), (3, 3, 0, 3, 2));
    }

    #[test]
    fn branches_and_two_parent_families() {
        let mut g = Genealogy::new();
        let mum = Lineage::founder(id(1), 0);
        let dad = Lineage::founder(id(2), 0);
        let kid = Lineage::offspring(id(3), &mum, Some(&dad), 1);
        let sib = Lineage::offspring(id(4), &mum, None, 1);
        for l in [mum, dad, kid, sib] {
            g.record_birth(l).unwrap();
        }
        assert_eq!(edges(&g), vec![(1, 3), (2, 3), (1, 4)]);
        assert_eq!(g.get(id(1)).unwrap().children(), &[id(3), id(4)]);
        g.record_death(id(1), 2).unwrap();
        g.record_death(id(2), 2).unwrap();
        assert_eq!(g.record_death(id(3), 3).unwrap(), 2, "kid and dad go; mum stays for sib");
        assert_eq!(g.nodes().map(|n| n.lineage.id.0).collect::<Vec<_>>(), vec![1, 4]);
        assert_eq!(edges(&g), vec![(1, 4)]);
        assert_eq!(g.record_death(id(4), 4).unwrap(), 2);
        assert!(g.is_empty());
    }

    #[test]
    fn errors_are_reported() {
        let mut g = Genealogy::new();
        g.record_birth(Lineage::founder(id(1), 0)).unwrap();
        assert_eq!(g.record_birth(Lineage::founder(id(1), 0)), Err(OrganismError::DuplicateBirth(id(1))));
        let selfish = Lineage { id: id(2), parents: [Some(id(2)), None], generation: 1, birth_tick: 0 };
        assert_eq!(g.record_birth(selfish), Err(OrganismError::SelfParent(id(2))));
        assert_eq!(g.record_death(id(9), 0), Err(OrganismError::UnknownAgent(id(9))));
        let child = Lineage::offspring(id(3), &Lineage::founder(id(1), 0), None, 1);
        g.record_birth(child).unwrap();
        g.record_death(id(1), 1).unwrap();
        assert_eq!(g.record_death(id(1), 2), Err(OrganismError::AlreadyDead(id(1))));
        // Unknown (never recorded) parents are accepted without a link.
        let stranger = Lineage::offspring(id(5), &Lineage::founder(id(4), 0), None, 3);
        g.record_birth(stranger).unwrap();
        assert_eq!(edges(&g), vec![(1, 3)]);
    }

    #[test]
    fn births_must_be_recorded_in_id_order_with_older_parents() {
        let founder = Lineage::founder(id(1), 0);
        let child = Lineage::offspring(id(5), &founder, None, 1);
        // Recording the child first would leave the founder without its child
        // link (wrongly pruned on death, unserialisable): rejected instead.
        let mut g = Genealogy::new();
        g.record_birth(child).unwrap();
        assert_eq!(g.record_birth(founder), Err(OrganismError::BirthOutOfOrder { id: id(1), last: id(5) }));
        assert_eq!(g.len(), 1, "a rejected birth changes nothing");
        // A parent younger than its child is rejected.
        let late_parent = Lineage::founder(id(9), 0);
        let mut g = Genealogy::new();
        let orphan = Lineage::offspring(id(6), &late_parent, None, 1);
        assert_eq!(g.record_birth(orphan), Err(OrganismError::ParentNotOlder { child: id(6), parent: id(9) }));
        assert!(g.is_empty() && g.summary().births == 0);
        // A pruned id cannot be recorded again (it would count twice).
        g.record_birth(Lineage::founder(id(1), 0)).unwrap();
        assert_eq!(g.record_death(id(1), 1).unwrap(), 1);
        assert_eq!(
            g.record_birth(Lineage::founder(id(1), 2)),
            Err(OrganismError::BirthOutOfOrder { id: id(1), last: id(1) })
        );
        assert_eq!(g.summary().births, 1);
        assert_eq!(g.last_birth(), Some(id(1)));
    }

    /// Structural invariants that must hold after every operation.
    fn assert_invariants(g: &Genealogy, living: &BTreeSet<AgentId>) {
        let recorded_living: BTreeSet<AgentId> = g.nodes().filter(|n| n.is_alive()).map(|n| n.lineage.id).collect();
        assert_eq!(&recorded_living, living, "exactly the living agents are alive in the genealogy");
        assert!(g.nodes().all(|n| n.is_alive() || !n.children().is_empty()), "no dead leaves");
        let s = g.summary();
        assert_eq!(s.births, s.pruned + s.truncated + g.len() as u64);
        assert_eq!(s.living, living.len() as u64);
        if let Some(max) = g.max_nodes() {
            assert!(g.len() <= max.max(living.len()), "cap {max}, {} nodes", g.len());
        }
        // The in-memory state always passes snapshot validation.
        let back: Genealogy = postcard::from_bytes(&postcard::to_stdvec(g).unwrap()).unwrap();
        assert_eq!(&back, g);
    }

    #[test]
    fn capped_genealogy_with_long_lived_founders_keeps_its_invariants() {
        let mut rng = SimRng::seed_from_u64(77);
        let mut g = Genealogy::with_max_nodes(260);
        let mut all: BTreeMap<AgentId, Lineage> = BTreeMap::new();
        let mut living: BTreeSet<AgentId> = BTreeSet::new();
        // 200 immortal founders: the oldest ids stay alive, so the oldest dead
        // node is far from the start of the id order.
        for n in 1..=200 {
            let l = Lineage::founder(id(n), 0);
            g.record_birth(l).unwrap();
            all.insert(l.id, l);
            living.insert(l.id);
        }
        let mut next = 201;
        let mut mortal: Vec<AgentId> = Vec::new();
        for tick in 1..1500u64 {
            if mortal.len() < 40 || rng.chance(0.5) {
                let pool: Vec<AgentId> = living.iter().copied().collect();
                let a = all[&pool[rng.index(pool.len())]];
                let b = rng.chance(0.5).then(|| all[&pool[rng.index(pool.len())]]);
                let l = Lineage::offspring(id(next), &a, b.as_ref(), tick);
                next += 1;
                g.record_birth(l).unwrap();
                all.insert(l.id, l);
                living.insert(l.id);
                mortal.push(l.id);
            } else {
                let victim = mortal.swap_remove(rng.index(mortal.len()));
                living.remove(&victim);
                g.record_death(victim, tick).unwrap();
            }
            if tick % 50 == 0 {
                assert_invariants(&g, &living);
            }
        }
        assert_invariants(&g, &living);
        assert!(g.summary().truncated > 0, "the cap was exercised");
    }

    #[test]
    fn node_cap_truncates_oldest_dead_ancestors_but_never_the_living() {
        let mut g = Genealogy::with_max_nodes(3);
        let mut parent = Lineage::founder(id(1), 0);
        g.record_birth(parent).unwrap();
        for n in 2..=10 {
            let child = Lineage::offspring(id(n), &parent, None, n);
            g.record_birth(child).unwrap();
            g.record_death(parent.id, n).unwrap();
            parent = child;
        }
        assert_eq!(g.len(), 3);
        assert_eq!(g.nodes().map(|n| n.lineage.id.0).collect::<Vec<_>>(), vec![8, 9, 10]);
        assert_eq!(g.summary().truncated, 7);
        assert_eq!(edges(&g), vec![(8, 9), (9, 10)]);
        let mut crowded = Genealogy::with_max_nodes(1);
        for n in 1..=3 {
            crowded.record_birth(Lineage::founder(id(n), 0)).unwrap();
        }
        assert_eq!(crowded.len(), 3, "living agents are never removed");
    }

    #[test]
    fn random_population_keeps_exactly_the_ancestry_of_the_living() {
        let mut rng = SimRng::seed_from_u64(12);
        let mut g = Genealogy::new();
        let mut all: BTreeMap<AgentId, Lineage> = BTreeMap::new();
        let mut alive: Vec<AgentId> = Vec::new();
        let mut next = 1;
        for _ in 0..30 {
            let l = Lineage::founder(id(next), 0);
            next += 1;
            g.record_birth(l).unwrap();
            all.insert(l.id, l);
            alive.push(l.id);
        }
        for tick in 1..3000u64 {
            if (alive.len() < 60 && rng.chance(0.55)) || alive.len() < 5 {
                let a = all[&alive[rng.index(alive.len())]];
                let b = rng.chance(0.5).then(|| all[&alive[rng.index(alive.len())]]);
                let l = Lineage::offspring(id(next), &a, b.as_ref(), tick);
                next += 1;
                g.record_birth(l).unwrap();
                all.insert(l.id, l);
                alive.push(l.id);
            } else {
                let victim = alive.swap_remove(rng.index(alive.len()));
                g.record_death(victim, tick).unwrap();
            }
        }
        // Expected node set: the living and all their ancestors.
        let mut expected = BTreeSet::new();
        let mut stack: Vec<AgentId> = alive.clone();
        while let Some(a) = stack.pop() {
            if expected.insert(a) {
                stack.extend(all[&a].parent_ids());
            }
        }
        let recorded: BTreeSet<AgentId> = g.nodes().map(|n| n.lineage.id).collect();
        assert_eq!(recorded, expected);
        assert!(g.len() < all.len(), "agents that died without living descendants were pruned");
        let s = g.summary();
        assert_eq!(s.births, all.len() as u64);
        assert_eq!(s.living, alive.len() as u64);
        assert_eq!(s.births - s.pruned, g.len() as u64);
        assert_invariants(&g, &alive.iter().copied().collect());
    }

    #[test]
    fn serde_round_trip_and_validation() {
        let mut g = Genealogy::with_max_nodes(100);
        let a = Lineage::founder(id(1), 0);
        let b = Lineage::founder(id(2), 0);
        let c = Lineage::offspring(id(3), &a, Some(&b), 4);
        for l in [a, b, c] {
            g.record_birth(l).unwrap();
        }
        g.record_death(id(1), 5).unwrap();
        let json = serde_json::to_string(&g).unwrap();
        let back: Genealogy = serde_json::from_str(&json).unwrap();
        assert_eq!(back, g);
        let bytes = postcard::to_stdvec(&g).unwrap();
        assert_eq!(postcard::from_bytes::<Genealogy>(&bytes).unwrap(), g);
        // Break the symmetric links: drop child 3 from parent 2.
        let broken = json.replace(r#""death_tick":null,"children":[3]"#, r#""death_tick":null,"children":[]"#);
        assert_ne!(broken, json);
        let err = serde_json::from_str::<Genealogy>(&broken).unwrap_err().to_string();
        assert!(err.contains("does not list its child"), "{err}");
    }
}
