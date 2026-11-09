//! Territory claimed around group buildings (an XTENT-style influence model).
//!
//! Each finished building `b` of group `g` is a centre with strength
//! `S_b = 4 · w_kind · (1 + 0.25 · ln n_g)` (`n_g` = group members) and exerts
//! the influence `I_b(t) = S_b − k · d(b, t)` on tile `t` at Euclidean
//! distance `d` (Renfrew & Level 1979; `k` = territory falloff). A tile's
//! influence from a group is the maximum over the group's buildings; the tile
//! belongs to the group with the largest positive influence. It is
//! **contested** when a second group's influence is also positive and within
//! [`CONTEST_MARGIN`] of the owner's.

use simulation_core::glam::Vec2;

use crate::buildings::Buildings;
use crate::config::Rules;
use crate::groups::{GroupId, Groups};
use crate::world::{Claim, World};

/// Influence difference below which a border tile is contested.
pub const CONTEST_MARGIN: f32 = 1.0;
/// Base strength of a building centre.
pub const BASE_STRENGTH: f32 = 4.0;

#[derive(Clone, Copy, Default)]
struct Best {
    first: Option<(GroupId, f32)>,
    second: Option<(GroupId, f32)>,
}

impl Best {
    /// Online top-2 of per-group maxima (exact because maxima only grow and
    /// values dropped from the top two are below the current second).
    fn offer(&mut self, g: GroupId, v: f32) {
        match (self.first, self.second) {
            (Some((g1, v1)), _) if g1 == g => self.first = Some((g, v1.max(v))),
            (Some((g1, v1)), Some((g2, v2))) if g2 == g => {
                let v2 = v2.max(v);
                if v2 > v1 {
                    self.first = Some((g2, v2));
                    self.second = Some((g1, v1));
                } else {
                    self.second = Some((g2, v2));
                }
            }
            (Some((_, v1)), _) if v > v1 => {
                self.second = self.first;
                self.first = Some((g, v));
            }
            (Some(_), second) if second.is_none_or(|(_, v2)| v > v2) => self.second = Some((g, v)),
            (None, _) => self.first = Some((g, v)),
            _ => {}
        }
    }
}

/// Recomputes the territory layer of `world` and the per-group statistics.
pub fn update(world: &mut World, buildings: &Buildings, groups: &mut Groups, rules: &Rules) {
    let (w, h) = (world.width() as i64, world.height() as i64);
    let mut best = vec![Best::default(); world.len()];
    let falloff = rules.territory_falloff.max(1e-3);
    for b in buildings.iter().filter(|b| b.is_complete()) {
        let Some(g) = b.group else { continue };
        let Some(group) = groups.get(g) else { continue };
        let strength = BASE_STRENGTH * b.kind.influence() * (1.0 + 0.25 * (group.members.max(1) as f32).ln());
        let radius = (strength / falloff).floor() as i64;
        let centre = world.center(b.tile);
        let (cx, cy) = (centre.x as i64, centre.y as i64);
        for y in (cy - radius).max(0)..=(cy + radius).min(h - 1) {
            for x in (cx - radius).max(0)..=(cx + radius).min(w - 1) {
                let d = centre.distance(Vec2::new(x as f32 + 0.5, y as f32 + 0.5));
                let influence = strength - falloff * d;
                if influence > 0.0 {
                    best[(y * w + x) as usize].offer(g, influence);
                }
            }
        }
    }
    let mut stats: Vec<(GroupId, u32, u32)> = groups.iter().map(|g| (g.id, 0, 0)).collect();
    let claims: Vec<Claim> = best
        .iter()
        .map(|b| match b.first {
            Some((owner, v1)) => {
                let contested = b.second.is_some_and(|(_, v2)| v1 - v2 < CONTEST_MARGIN);
                if let Some(s) = stats.iter_mut().find(|(id, _, _)| *id == owner) {
                    s.1 += 1;
                    s.2 += u32::from(contested);
                }
                Claim { owner: Some(owner), contested }
            }
            None => Claim::default(),
        })
        .collect();
    world.set_territory(claims);
    groups.set_territory(&stats);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_two_tracks_per_group_maxima() {
        let mut b = Best::default();
        for (g, v) in [(1, 2.0), (2, 1.0), (3, 0.5), (3, 3.0), (2, 2.5), (1, 1.0)] {
            b.offer(GroupId(g), v);
        }
        assert_eq!(b.first, Some((GroupId(3), 3.0)));
        assert_eq!(b.second, Some((GroupId(2), 2.5)));
    }
}
