//! Social behaviour: boids for herds, flocks and schools, pack leaders and
//! shared hunting targets, territorial defence and fights, alarm calls.
//!
//! * **Boids** (`organism-engine` steering): the nearest perceived
//!   conspecifics (at most [`FLOCK_SIZE`]) give cohesion (weight
//!   `social.cohesion`) and alignment (`social.alignment`, default the
//!   cohesion) while wandering, foraging (½) and migrating; a wandering
//!   animal's own random wander is damped by `1 − ½ alignment`; separation
//!   is applied to every animal.
//! * **Packs** (`structure: pack`): each group's leader (alpha) is its
//!   highest-ranking member — adults before juveniles, then mass × health,
//!   then the lower id. Members follow the alpha when they travel, regroup
//!   with it when they have strayed (a wander score rising from 0 at a
//!   quarter of their vision to `social.cohesion` at their vision, at cruise
//!   speed), join its hunt (the alpha's prey becomes theirs while they can
//!   hear it) and let it feed first at a carcass. Only the pack's dominant
//!   adult female and male breed (reproductive suppression).
//! * **Territories**: a territorial adult notices an *intruder* — an adult
//!   conspecific of another group (or, without groups, of the same sex)
//!   inside its territory radius — chases it (`defend`) and fights it on
//!   contact. The intruder perceives a chasing owner as a threat and flees.
//!   Fight: `P(owner wins) = σ(2 ln(S_o/S_i) + 0.5)` with
//!   `S = (aggression + 0.2) · m^{1/3} · health · (1 − ½ injury)`; the
//!   loser is injured (+0.15–0.3) and loses 0.05 health, the winner is
//!   scratched (+0.05) with probability 0.3.
//! * **Alarm calls**: an animal of a calling species that flees from a
//!   predator calls with strength 1; conspecifics that hear a call of
//!   strength `s` at distance `d` perceive the caller's threat with level
//!   `0.7 s (1 − ½ d/hearing)` and, while they flee from it, relay it with
//!   that level as strength (if it is at least 0.2). The alarm thus spreads
//!   a few hops through the group and fades once nobody sees the predator.

use organism_engine::steering;
use organism_engine::{AgentId, AgentStore, Kinematics, MotionLimits};
use simulation_core::SimRng;
use simulation_core::glam::Vec2;

use super::perceive::{Ctx, PreyPercept, Senses, Threat, ThreatSource};
use super::{Action, Animal, Sex, Target};
use crate::species::{AnimalTraits, Species, SpeciesCatalog};

/// Nearest conspecifics used for boids.
pub const FLOCK_SIZE: usize = 8;
/// Level of a threat learnt from an alarm call of strength 1 at distance 0.
const ALARM_GAIN: f32 = 0.7;
/// Weakest heard alarm an animal relays.
pub const MIN_RELAY: f32 = 0.2;
/// Level of a chasing territory owner at distance 0.
const RIVAL_LEVEL: f32 = 0.8;
/// Home advantage of a territory owner (logit).
const HOME_ADVANTAGE: f32 = 0.5;

/// Rank of an animal within its group (higher leads).
fn rank(a: &Animal) -> (bool, f32) {
    (a.is_adult(), a.mass_kg * a.health)
}

/// Ranking of one group of a pack species.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PackRank {
    /// Group id.
    pub group: u64,
    /// Highest-ranking member (the alpha).
    pub leader: AgentId,
    /// Highest-ranking adult female and adult male: the breeding pair.
    pub breeders: [Option<AgentId>; 2],
}

/// Whether rank `r` beats `best` (iteration is in id order, so ties keep
/// the lower id).
fn outranks(r: (bool, f32), best: (bool, f32)) -> bool {
    r.0 && !best.0 || (r.0 == best.0 && r.1 > best.1)
}

/// Rankings of the groups of pack species, sorted by group id.
pub(crate) fn pack_leaders(animals: &AgentStore<Animal>, catalog: &SpeciesCatalog) -> Vec<PackRank> {
    type Slot = (AgentId, (bool, f32));
    let mut best: Vec<(u64, Slot, [Option<Slot>; 2])> = Vec::new();
    for (id, a) in animals.iter() {
        let Some(group) = a.group else { continue };
        if !catalog.animals()[usize::from(a.species)].social.is_pack() {
            continue;
        }
        let r = rank(a);
        let i = match best.iter().position(|(g, _, _)| *g == group) {
            Some(i) => i,
            None => {
                best.push((group, (id, r), [None, None]));
                best.len() - 1
            }
        };
        let entry = &mut best[i];
        if outranks(r, entry.1.1) {
            entry.1 = (id, r);
        }
        if a.is_adult() {
            let breeder = &mut entry.2[sex_index(a.sex)];
            if breeder.is_none_or(|(_, b)| outranks(r, b)) {
                *breeder = Some((id, r));
            }
        }
    }
    let mut ranks: Vec<PackRank> = best
        .into_iter()
        .map(|(group, (leader, _), breeders)| PackRank {
            group,
            leader,
            breeders: breeders.map(|b| b.map(|(id, _)| id)),
        })
        .collect();
    ranks.sort_unstable_by_key(|r| r.group);
    ranks
}

/// Index of a sex in [`PackRank::breeders`].
fn sex_index(sex: Sex) -> usize {
    match sex {
        Sex::Female => 0,
        Sex::Male => 1,
    }
}

/// Ranking of a group in a sorted list.
fn rank_of(ranks: &[PackRank], group: Option<u64>) -> Option<&PackRank> {
    let group = group?;
    ranks.binary_search_by_key(&group, |r| r.group).ok().map(|i| &ranks[i])
}

/// Leader of a group, if the group belongs to a pack species.
pub(crate) fn leader_of(ctx: &Ctx<'_>, group: Option<u64>) -> Option<AgentId> {
    leader_in(ctx.leaders, group)
}

/// Leader of a group in a sorted ranking list.
pub(crate) fn leader_in(ranks: &[PackRank], group: Option<u64>) -> Option<AgentId> {
    rank_of(ranks, group).map(|r| r.leader)
}

/// Whether an animal may breed: in a pack only the dominant adult female
/// and male do (reproductive suppression); everyone else may.
pub(crate) fn may_breed(ctx: &Ctx<'_>, id: AgentId, a: &Animal) -> bool {
    rank_of(ctx.leaders, a.group).is_none_or(|r| r.breeders[sex_index(a.sex)] == Some(id))
}

/// Whether `other` intrudes into the territory of `me` (both of `species`).
pub(crate) fn intrudes(me: &Animal, other: &Animal, home: Vec2, radius: f32) -> bool {
    let strangers = match (me.group, other.group) {
        (Some(a), Some(b)) => a != b,
        _ => me.sex == other.sex,
    };
    other.is_adult() && strangers && other.kin.position.distance(home) <= radius
}

/// Handles one perceived conspecific for the social senses: flock
/// membership, alarm calls heard, rivals chasing me and intruders.
#[allow(clippy::too_many_arguments)]
pub(crate) fn conspecific(
    ctx: &Ctx<'_>,
    me: &Animal,
    my_id: AgentId,
    species: &Species,
    other: &Animal,
    other_id: AgentId,
    distance: f32,
    hearing: f32,
    flock: &mut Vec<(f32, AgentId, Kinematics)>,
    senses: &mut Senses,
) {
    // Every perceived conspecific is a boids neighbour candidate; the
    // nearest `FLOCK_SIZE` are kept.
    flock.push((distance, other_id, other.kin));
    if let Some((threat, strength)) = other.alarm
        && species.social.alarm_calls
        && distance <= hearing
    {
        let level = ALARM_GAIN * strength * (1.0 - 0.5 * distance / hearing.max(1e-3));
        if senses.threat.is_none_or(|t| level > t.level) {
            senses.threat = Some(Threat {
                source: ThreatSource::Alarm,
                position: threat,
                velocity: Vec2::ZERO,
                distance: threat.distance(me.kin.position),
                level,
            });
        }
    }
    if other.action == Action::Defend && other.target == Target::Animal(my_id) {
        let reach = hearing.max(4.0);
        let level = RIVAL_LEVEL * (1.0 - distance / reach).max(0.2);
        if senses.threat.is_none_or(|t| level > t.level) {
            senses.threat = Some(Threat {
                source: ThreatSource::Rival(other_id),
                position: other.kin.position,
                velocity: other.kin.velocity,
                distance,
                level,
            });
        }
    }
    let traits = &ctx.traits[usize::from(me.species)];
    if let Some(home) = me.home
        && traits.territory_cells > 0.0
        && me.is_adult()
        && intrudes(me, other, home, traits.territory_cells)
        && senses.intruder.is_none_or(|(_, _, d)| distance < d)
    {
        senses.intruder = Some((other_id, other.kin.position, distance));
    }
}

/// Keeps the nearest flock mates (ties by id) as boids neighbours.
pub(crate) fn finish_flock(mut flock: Vec<(f32, AgentId, Kinematics)>, senses: &mut Senses) {
    flock.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    flock.truncate(FLOCK_SIZE);
    senses.flock = flock.into_iter().map(|(_, _, k)| k).collect();
}

/// Distances to the leader (cells) between which a pack member's urge to
/// regroup rises from 0 to the species' cohesion: a quarter of its vision
/// (at least 2 cells) and its vision.
pub(crate) fn regroup_range(traits: &AnimalTraits) -> (f32, f32) {
    let near = (0.25 * traits.vision_cells).max(2.0);
    (near, traits.vision_cells.max(near + 1.0))
}

/// Pack senses: the leader's position and, if it is hunting, its prey.
pub(crate) fn pack(ctx: &Ctx<'_>, me: &Animal, my_id: AgentId, species: &Species, hearing: f32, senses: &mut Senses) {
    if !species.social.is_pack() {
        return;
    }
    let Some(leader_id) = leader_of(ctx, me.group) else { return };
    if leader_id == my_id {
        return;
    }
    let Some(leader) = ctx.animals.get(leader_id) else { return };
    let pos = me.kin.position;
    let d = leader.kin.position.distance(pos);
    senses.leader = Some((leader.kin.position, d));
    if leader.action != Action::Hunt || d > 2.0 * hearing.max(4.0) {
        return;
    }
    let Target::Animal(prey_id) = leader.target else { return };
    let Some(prey) = ctx.animals.get(prey_id) else { return };
    if prey.hidden {
        return;
    }
    let distance = prey.kin.position.distance(pos);
    let own = senses.prey.filter(|p| p.id == prey_id).map_or(0.0, |p| p.value);
    senses.prey = Some(PreyPercept {
        id: prey_id,
        position: prey.kin.position,
        velocity: prey.kin.velocity,
        distance,
        value: own.max(0.8),
    });
    senses.pack_hunt = true;
}

/// Boids cohesion and alignment with the flock (zero for solitary species).
pub(crate) fn boids(kin: &Kinematics, senses: &Senses, species: &Species, limits: MotionLimits) -> Vec2 {
    if senses.flock.is_empty() {
        return Vec2::ZERO;
    }
    let radius = f32::INFINITY;
    let cohesion = steering::cohesion(kin, senses.flock.iter(), radius, limits) * species.social.cohesion;
    let alignment = steering::alignment(kin, senses.flock.iter(), radius, limits) * species.social.alignment_weight();
    cohesion + alignment
}

/// Pack members travelling with their leader.
pub(crate) fn follow_leader(kin: &Kinematics, senses: &Senses, limits: MotionLimits) -> Vec2 {
    match senses.leader {
        Some((p, d)) if d > 2.0 => steering::seek(kin, p, limits),
        _ => Vec2::ZERO,
    }
}

/// Outcome of a territorial fight.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Fight {
    pub owner: AgentId,
    pub intruder: AgentId,
    pub owner_wins: bool,
    /// Injury of the loser and of the winner.
    pub injuries: (f32, f32),
}

/// Fighting strength.
fn strength(species: &Species, a: &Animal) -> f32 {
    (species.combat.aggression + 0.2) * a.mass_kg.max(1e-6).cbrt() * a.health.max(0.01) * (1.0 - 0.5 * a.injury)
}

/// Resolves a fight between a territory owner and an intruder (pure apart
/// from the owner's own random stream).
pub(crate) fn resolve_fight(ctx: &Ctx<'_>, owner_id: AgentId, intruder_id: AgentId) -> Option<Fight> {
    let owner = ctx.animals.get(owner_id)?;
    let intruder = ctx.animals.get(intruder_id)?;
    let species = &ctx.catalog.animals()[usize::from(owner.species)];
    let (so, si) = (strength(species, owner), strength(species, intruder));
    let logit = 2.0 * (so / si.max(1e-6)).ln() + HOME_ADVANTAGE;
    let p = 1.0 / (1.0 + (-logit).exp());
    let mut rng: SimRng = crate::rng::stream(ctx.seed, crate::rng::ATTACK, ctx.tick, owner_id.get() ^ 0xF1_6E7);
    let owner_wins = rng.chance(f64::from(p));
    let loser = 0.15 + 0.15 * rng.f32();
    let winner = if rng.chance(0.3) { 0.05 } else { 0.0 };
    Some(Fight { owner: owner_id, intruder: intruder_id, owner_wins, injuries: (loser, winner) })
}

impl crate::state::EcoState {
    /// Applies the outcome of territorial fights: the loser is injured and
    /// loses health (it may die of its injuries), the winner may be
    /// scratched.
    pub(crate) fn apply_fights(&mut self, fights: &[Fight], deaths: &mut super::physiology::Deaths) {
        for f in fights {
            self.counters.fights += 1;
            let (loser, winner) = if f.owner_wins { (f.intruder, f.owner) } else { (f.owner, f.intruder) };
            if let Some(l) = self.animals.get_mut(loser) {
                l.injury = (l.injury + f.injuries.0).min(1.0);
                l.health = (l.health - 0.05).max(0.0);
                if l.health <= 0.0 {
                    deaths.entry(loser).or_insert((super::DeathCause::Injury, Some(winner)));
                }
            }
            if let Some(w) = self.animals.get_mut(winner) {
                w.injury = (w.injury + f.injuries.1).min(1.0);
            }
            let message = format!("#{} drove off #{} in a territorial fight", winner.get(), loser.get());
            self.log(
                simulation_core::SimEvent::new(self.tick, "fight", message)
                    .subject(winner.get())
                    .object(loser.get())
                    .severity(simulation_core::EventSeverity::Trace),
            );
        }
    }
}
