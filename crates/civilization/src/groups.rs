//! Group formation, persistence and splitting from mutual affinity.
//!
//! Every update builds an undirected *bond graph*: agents `i` and `j` are
//! linked when their **mutual** affinity `min(a_ij, a_ji)` reaches the bond
//! threshold `θ` (or `θ/2` if they already share a group, a hysteresis that
//! keeps groups from flickering). Connected components with at least
//! [`MIN_GROUP_SIZE`] members are groups. Group identity is carried over by
//! maximum member overlap: components and previous groups are matched
//! greedily by overlap (ties to the lower group id), so a group that splits
//! keeps its id in the larger part and the other part founds a new group,
//! and merging groups keep the id of the best-overlapping one.

use organism_engine::AgentStore;
use serde::{Deserialize, Serialize};
use simulation_core::{EventLog, EventSeverity, SimEvent};

use crate::agent::Agent;
use crate::config::Rules;

/// Smallest group.
pub const MIN_GROUP_SIZE: usize = 3;

/// Stable group identifier (never reused; `0` is never issued).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GroupId(pub u32);

/// One group.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Group {
    /// Identifier.
    pub id: GroupId,
    /// Display colour (RGBA, golden-ratio hues so neighbours differ).
    pub color: [f32; 4],
    /// Tick the group was founded.
    pub founded: u64,
    /// Current member count.
    pub members: u32,
    /// Tiles claimed (including contested ones).
    pub territory: u32,
    /// Claimed tiles that are contested.
    pub contested: u32,
}

/// Distinct colour for group `id`: hue `frac(id · 0.618…)`, saturation 0.65,
/// value 0.92.
pub fn group_color(id: GroupId) -> [f32; 4] {
    let hue = (f64::from(id.0) * 0.618_033_988_75).fract() as f32 * 6.0;
    let (s, v) = (0.65f32, 0.92f32);
    let c = v * s;
    let x = c * (1.0 - (hue % 2.0 - 1.0).abs());
    let (r, g, b) = match hue as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    [r + m, g + m, b + m, 1.0]
}

/// All groups, sorted by id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Groups {
    list: Vec<Group>,
    next_id: u32,
}

impl Default for Groups {
    fn default() -> Self {
        Self { list: Vec::new(), next_id: 1 }
    }
}

struct UnionFind(Vec<usize>);

impl UnionFind {
    fn find(&mut self, mut i: usize) -> usize {
        while self.0[i] != i {
            self.0[i] = self.0[self.0[i]];
            i = self.0[i];
        }
        i
    }

    /// Union keeping the smaller index as root (deterministic).
    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
            self.0[hi] = lo;
        }
    }
}

impl Groups {
    /// Groups in id order.
    pub fn iter(&self) -> impl Iterator<Item = &Group> {
        self.list.iter()
    }

    /// Number of groups.
    pub fn len(&self) -> usize {
        self.list.len()
    }

    /// Whether there are no groups.
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// Group by id.
    pub fn get(&self, id: GroupId) -> Option<&Group> {
        self.list.binary_search_by_key(&id, |g| g.id).ok().map(|i| &self.list[i])
    }

    /// Recomputes groups from the bond graph and writes each agent's group.
    pub fn update(&mut self, agents: &mut AgentStore<Agent>, rules: &Rules, tick: u64, events: &mut EventLog) {
        let n = agents.len();
        let mut uf = UnionFind((0..n).collect());
        let threshold = rules.bond_threshold;
        for (i, (id, agent)) in agents.iter().enumerate() {
            for rel in agent.relations.iter().filter(|r| r.other > id) {
                let Some(j) = agents.index_of(rel.other) else { continue };
                let other = &agents.as_slice()[j];
                let mutual = rel.affinity.min(other.relations.affinity(id));
                let same = agent.group.is_some() && agent.group == other.group;
                if mutual >= if same { 0.5 * threshold } else { threshold } {
                    uf.union(i, j);
                }
            }
        }
        // Components in order of their smallest member slot.
        let mut members: Vec<Vec<usize>> = Vec::new();
        let mut root_slot: Vec<Option<usize>> = vec![None; n];
        for i in 0..n {
            let r = uf.find(i);
            let c = *root_slot[r].get_or_insert_with(|| {
                members.push(Vec::new());
                members.len() - 1
            });
            members[c].push(i);
        }
        let components: Vec<Vec<usize>> = members.into_iter().filter(|c| c.len() >= MIN_GROUP_SIZE).collect();

        // Overlap of each component with previous groups.
        let mut overlaps: Vec<(usize, GroupId, usize)> = Vec::new();
        for (c, comp) in components.iter().enumerate() {
            let mut counts: Vec<(GroupId, usize)> = Vec::new();
            for &slot in comp {
                if let Some(g) = agents.as_slice()[slot].group {
                    match counts.iter_mut().find(|(id, _)| *id == g) {
                        Some(entry) => entry.1 += 1,
                        None => counts.push((g, 1)),
                    }
                }
            }
            overlaps.extend(counts.into_iter().map(|(g, k)| (c, g, k)));
        }
        overlaps.sort_by(|a, b| b.2.cmp(&a.2).then(a.1.cmp(&b.1)).then(a.0.cmp(&b.0)));
        let mut assigned: Vec<Option<GroupId>> = vec![None; components.len()];
        let mut used: Vec<GroupId> = Vec::new();
        for (c, g, _) in overlaps {
            if assigned[c].is_none() && !used.contains(&g) && self.get(g).is_some() {
                assigned[c] = Some(g);
                used.push(g);
            }
        }
        let previous: Vec<GroupId> = self.list.iter().map(|g| g.id).collect();
        let mut next_list: Vec<Group> = Vec::new();
        let mut new_group = vec![None; n];
        for (c, comp) in components.iter().enumerate() {
            let id = match assigned[c] {
                Some(id) => id,
                None => {
                    let id = GroupId(self.next_id);
                    self.next_id += 1;
                    let split_from = comp.iter().find_map(|&s| agents.as_slice()[s].group);
                    let message = match split_from {
                        Some(old) => format!("group {} split off from group {}", id.0, old.0),
                        None => format!("group {} formed with {} members", id.0, comp.len()),
                    };
                    events.push(
                        SimEvent::new(tick, if split_from.is_some() { "group_split" } else { "group_formed" }, message)
                            .subject(u64::from(id.0))
                            .severity(EventSeverity::Major),
                    );
                    id
                }
            };
            let template = self.get(id).cloned();
            next_list.push(Group {
                id,
                color: group_color(id),
                founded: template.as_ref().map_or(tick, |g| g.founded),
                members: comp.len() as u32,
                territory: template.as_ref().map_or(0, |g| g.territory),
                contested: template.as_ref().map_or(0, |g| g.contested),
            });
            for &slot in comp {
                new_group[slot] = Some(id);
            }
        }
        for old in previous.into_iter().filter(|g| !used.contains(g)) {
            events.push(
                SimEvent::new(tick, "group_dissolved", format!("group {} dissolved", old.0)).subject(u64::from(old.0)),
            );
        }
        next_list.sort_by_key(|g| g.id);
        self.list = next_list;
        for (agent, group) in agents.as_mut_slice().iter_mut().zip(new_group) {
            agent.group = group;
        }
    }

    /// Writes per-group territory statistics.
    pub fn set_territory(&mut self, stats: &[(GroupId, u32, u32)]) {
        for g in &mut self.list {
            let (area, contested) = stats.iter().find(|(id, _, _)| *id == g.id).map_or((0, 0), |(_, a, c)| (*a, *c));
            g.territory = area;
            g.contested = contested;
        }
    }

    /// Checks the id ordering after deserialisation.
    pub fn validate(&self) -> Result<(), String> {
        let mut last = 0;
        for g in &self.list {
            if g.id.0 <= last || g.id.0 >= self.next_id {
                return Err(format!("group ids are not increasing below the counter ({})", g.id.0));
            }
            last = g.id.0;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use organism_engine::{AgentId, AgentStore};
    use simulation_core::glam::Vec2;

    use super::*;
    use crate::agent::{ADULT_AGE, Needs, Preferences, Sex};

    fn store(n: usize) -> AgentStore<Agent> {
        let prefs = Preferences { risk: 0.5, cooperation: 0.5, trade: 0.5, time: 0.5 };
        (0..n)
            .map(|_| Agent::new(Vec2::ZERO, Sex::Female, ADULT_AGE, ADULT_AGE * 3, prefs, Needs::new(0.0, 0.0, 0.0)))
            .collect()
    }

    fn bond(s: &mut AgentStore<Agent>, a: u64, b: u64, value: f32) {
        s.get_mut(AgentId(a)).unwrap().relations.adjust(AgentId(b), value, 12);
        s.get_mut(AgentId(b)).unwrap().relations.adjust(AgentId(a), value, 12);
    }

    #[test]
    fn components_of_mutual_bonds_become_groups() {
        let mut s = store(8);
        let (rules, mut log, mut groups) = (Rules::default(), EventLog::new(100), Groups::default());
        for (a, b) in [(1, 2), (2, 3), (4, 5), (5, 6), (6, 4)] {
            bond(&mut s, a, b, 0.5);
        }
        // One-sided affinity does not bond.
        s.get_mut(AgentId(7)).unwrap().relations.adjust(AgentId(8), 0.9, 12);
        groups.update(&mut s, &rules, 0, &mut log);
        assert_eq!(groups.len(), 2);
        let g1 = s.get(AgentId(1)).unwrap().group.unwrap();
        assert_eq!(s.get(AgentId(3)).unwrap().group, Some(g1));
        assert_ne!(s.get(AgentId(4)).unwrap().group, Some(g1));
        assert_eq!(s.get(AgentId(7)).unwrap().group, None);
        let members: Vec<AgentId> = s.iter().filter(|(_, a)| a.group == Some(g1)).map(|(id, _)| id).collect();
        assert_eq!(members, vec![AgentId(1), AgentId(2), AgentId(3)]);
        assert_ne!(group_color(GroupId(1)), group_color(GroupId(2)));
    }

    #[test]
    fn identity_persists_and_splits_create_new_ids() {
        let mut s = store(7);
        let (rules, mut log, mut groups) = (Rules::default(), EventLog::new(100), Groups::default());
        for (a, b) in [(1, 2), (2, 3), (3, 4), (4, 5), (5, 6), (6, 7)] {
            bond(&mut s, a, b, 0.5);
        }
        groups.update(&mut s, &rules, 0, &mut log);
        let original = s.get(AgentId(1)).unwrap().group.unwrap();
        // Hysteresis: weakening below θ but above θ/2 keeps the group.
        bond(&mut s, 3, 4, -0.3);
        groups.update(&mut s, &rules, 24, &mut log);
        assert_eq!(groups.len(), 1);
        // Breaking the chain splits it: the larger part keeps the id.
        bond(&mut s, 3, 4, -0.5);
        groups.update(&mut s, &rules, 48, &mut log);
        assert_eq!(groups.len(), 2);
        assert_eq!(s.get(AgentId(7)).unwrap().group, Some(original), "larger part (4..7) keeps the id");
        let other = s.get(AgentId(1)).unwrap().group.unwrap();
        assert!(other > original);
        assert!(log.iter().any(|e| e.kind == "group_split"));
        assert_eq!(groups.get(original).unwrap().founded, 0);
    }
}
