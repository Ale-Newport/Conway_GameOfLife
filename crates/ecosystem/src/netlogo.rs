//! Re-implementation of the rules of NetLogo's *Wolf Sheep Predation*
//! (Wilensky 1997, CC BY-NC-SA; no code or text copied — the rules are
//! re-implemented from the description in `docs/research/ecology-alife.md`
//! §1.3).
//!
//! Per tick, in this order:
//!
//! 1. **Sheep**, in a fresh seeded random order: turn right by `random 50`°,
//!    left by `random 50`°, move forward one cell (torus); in the grass
//!    version lose 1 energy, eat the grass of the patch if it is green (it
//!    turns brown, +`sheep_gain_from_food`), die if energy < 0; then with
//!    probability `sheep_reproduce %` halve the energy and hatch one lamb
//!    with the same energy that turns randomly and steps forward.
//! 2. **Wolves**, in a fresh random order: move; lose 1 energy; kill one
//!    random sheep on the same patch (+`wolf_gain_from_food`); die if
//!    energy < 0; reproduce like sheep (not gated by energy).
//! 3. **Grass**: a brown patch turns green when its countdown is ≤ 0 (and
//!    the countdown resets to `grass_regrowth_time`), otherwise counts down.
//!
//! Animals hatched during a phase act from the next tick on (NetLogo's
//! `ask` evaluates its agentset first) but can be eaten this tick. Energies
//! are NetLogo units stored in [`Animal::energy_kj`]; there is no physiology,
//! nutrient cycle or utility AI in this variant.

use organism_engine::{AgentId, Kinematics, Lineage};
use serde::{Deserialize, Serialize};
use simulation_core::SimRng;
use simulation_core::glam::Vec2;

use crate::animal::{Action, Animal, DeathCause, ReproState, Sex};
use crate::config::NetLogoConfig;
use crate::error::EcosystemError;
use crate::species::PlantLayer;
use crate::state::{EcoState, Runtime};

/// Patch grass of the NetLogo variant.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NetLogoState {
    /// Species index of the sheep.
    pub sheep: u16,
    /// Species index of the wolves.
    pub wolf: u16,
    /// Whether each patch is green.
    pub green: Vec<bool>,
    /// Regrowth countdown of each patch.
    pub countdown: Vec<i64>,
}

impl NetLogoState {
    /// Sets up grass and the initial animals (NetLogo `setup`).
    pub(crate) fn init(state: &mut EcoState, rt: &Runtime, config: &NetLogoConfig) -> Result<Self, EcosystemError> {
        let sheep = state.catalog.animal_index(&config.sheep_species)?;
        let wolf = state.catalog.animal_index(&config.wolf_species)?;
        let n = state.world.width() * state.world.height();
        let mut green = vec![true; n];
        let mut countdown = vec![i64::from(config.grass_regrowth_time); n];
        if config.grass {
            for i in 0..n {
                green[i] = state.rng.chance(0.5);
                countdown[i] = if green[i] {
                    i64::from(config.grass_regrowth_time)
                } else {
                    random_int(&mut state.rng, config.grass_regrowth_time)
                };
            }
        }
        let gains = [
            (sheep, config.initial_sheep, config.sheep_gain_from_food),
            (wolf, config.initial_wolves, config.wolf_gain_from_food),
        ];
        for (species, count, gain) in gains {
            for _ in 0..count {
                let (w, h) = (state.world.width() as f32, state.world.height() as f32);
                let position = Vec2::new(state.rng.range_f32(0.0, w), state.rng.range_f32(0.0, h));
                let heading = (random_int(&mut state.rng, 360) as f32).to_radians();
                // NetLogo: energy = random (2 × gain), an integer in [0, 2·gain).
                let energy = random_int(&mut state.rng, (2.0 * gain).floor() as u32) as f32;
                spawn(state, rt, species, position, heading, energy, None)?;
            }
        }
        state.animals.commit();
        let mut netlogo = Self { sheep, wolf, green, countdown };
        netlogo.sync_vegetation(state);
        Ok(netlogo)
    }

    /// Mirrors the grass into the vegetation grass layer (1 = green) for views.
    fn sync_vegetation(&mut self, state: &mut EcoState) {
        for (i, &g) in self.green.iter().enumerate() {
            state.vegetation.set(i, PlantLayer::Grass, if g { 1.0 } else { 0.0 });
        }
    }

    /// Number of green patches.
    pub fn green_patches(&self) -> usize {
        self.green.iter().filter(|g| **g).count()
    }
}

/// NetLogo `random n`: a uniform integer in `[0, n)` (0 when `n = 0`).
fn random_int(rng: &mut SimRng, n: u32) -> i64 {
    if n == 0 { 0 } else { rng.below(u64::from(n)) as i64 }
}

fn spawn(
    state: &mut EcoState,
    rt: &Runtime,
    species: u16,
    position: Vec2,
    heading: f32,
    energy: f32,
    parent: Option<&Lineage>,
) -> Result<AgentId, EcosystemError> {
    let traits = &rt.traits[usize::from(species)];
    let id = state.animals.next_id();
    let sex = if state.rng.chance(0.5) { Sex::Female } else { Sex::Male };
    let lineage = match parent {
        Some(p) => Lineage::offspring(id, p, None, state.tick),
        None => Lineage::founder(id, state.tick),
    };
    let mut animal = Animal::new(
        species,
        sex,
        Kinematics::new(position, heading),
        state.tick as i64,
        organism_engine::Genome::defaults(&rt.schema),
        lineage,
    );
    animal.mass_kg = traits.adult_mass_kg;
    animal.energy_kj = energy;
    animal.repro = ReproState::Ready;
    animal.action = Action::Wander;
    let lineage = animal.lineage;
    let queued = state.animals.queue_spawn(animal);
    debug_assert_eq!(queued, id);
    state.genealogy.record_birth(lineage)?;
    Ok(queued)
}

/// `rt random 50 lt random 50 fd 1` on the torus.
fn wiggle_and_move(kin: &mut Kinematics, rng: &mut SimRng, w: f32, h: f32) {
    let turn = (random_int(rng, 50) - random_int(rng, 50)) as f32;
    kin.heading += turn.to_radians();
    step_forward(kin, 1.0, w, h);
}

fn step_forward(kin: &mut Kinematics, distance: f32, w: f32, h: f32) {
    let dir = Vec2::new(kin.heading.cos(), kin.heading.sin());
    kin.velocity = dir * distance;
    let p = kin.position + kin.velocity;
    kin.position = Vec2::new(p.x.rem_euclid(w), p.y.rem_euclid(h));
    if kin.position.x >= w {
        kin.position.x = 0.0;
    }
    if kin.position.y >= h {
        kin.position.y = 0.0;
    }
}

fn patch(p: Vec2, w: usize, h: usize) -> usize {
    let x = (p.x.floor() as usize).min(w - 1);
    let y = (p.y.floor() as usize).min(h - 1);
    y * w + x
}

impl EcoState {
    /// One tick of the NetLogo rules.
    pub(crate) fn netlogo_tick(&mut self, rt: &Runtime) -> Result<(), EcosystemError> {
        let Some(mut nl) = self.netlogo.take() else { return Ok(()) };
        let crate::config::Rules::NetLogo(cfg) = self.config.rules.clone() else {
            self.netlogo = Some(nl);
            return Ok(());
        };
        let result = self.netlogo_phases(rt, &mut nl, &cfg);
        nl.sync_vegetation(self);
        self.netlogo = Some(nl);
        result
    }

    fn netlogo_phases(
        &mut self,
        rt: &Runtime,
        nl: &mut NetLogoState,
        cfg: &NetLogoConfig,
    ) -> Result<(), EcosystemError> {
        let (w, h) = (self.world.width(), self.world.height());
        let (wf, hf) = (w as f32, h as f32);
        let mut rng = crate::rng::stream(self.seed, crate::rng::NETLOGO, self.tick, 0);
        // 1. Sheep.
        let mut order: Vec<AgentId> =
            self.animals.iter().filter(|(_, a)| a.species == nl.sheep).map(|(id, _)| id).collect();
        rng.shuffle(&mut order);
        let mut lambs: Vec<(Lineage, Vec2, f32)> = Vec::new();
        for id in order {
            let (dead, lamb) = {
                let Some(sheep) = self.animals.get_mut(id) else { continue };
                wiggle_and_move(&mut sheep.kin, &mut rng, wf, hf);
                let mut dead = false;
                if cfg.grass {
                    sheep.energy_kj -= 1.0;
                    let p = patch(sheep.kin.position, w, h);
                    sheep.action = if nl.green[p] { Action::Graze } else { Action::Wander };
                    if nl.green[p] {
                        nl.green[p] = false;
                        sheep.energy_kj += cfg.sheep_gain_from_food;
                    }
                    dead = sheep.energy_kj < 0.0;
                }
                let mut lamb = None;
                if !dead && rng.chance(f64::from(cfg.sheep_reproduce_percent) / 100.0) {
                    sheep.energy_kj /= 2.0;
                    sheep.stats.offspring += 1;
                    lamb = Some((sheep.lineage, sheep.kin.position, sheep.energy_kj));
                }
                (dead, lamb)
            };
            if dead {
                self.animals.queue_despawn(id);
                self.counters.death(nl.sheep, DeathCause::Starvation);
                self.genealogy.record_death(id, self.tick)?;
            }
            lambs.extend(lamb);
        }
        for (parent, position, energy) in lambs {
            self.hatch(rt, nl.sheep, &parent, position, energy, &mut rng, wf, hf)?;
        }
        self.animals.commit();
        // 2. Wolves: index the sheep by patch.
        let mut on_patch: Vec<Vec<AgentId>> = vec![Vec::new(); w * h];
        for (id, a) in self.animals.iter() {
            if a.species == nl.sheep {
                on_patch[patch(a.kin.position, w, h)].push(id);
            }
        }
        let mut order: Vec<AgentId> =
            self.animals.iter().filter(|(_, a)| a.species == nl.wolf).map(|(id, _)| id).collect();
        rng.shuffle(&mut order);
        let mut cubs: Vec<(Lineage, Vec2, f32)> = Vec::new();
        for id in order {
            let (prey, dead, cub) = {
                let Some(wolf) = self.animals.get_mut(id) else { continue };
                wiggle_and_move(&mut wolf.kin, &mut rng, wf, hf);
                wolf.energy_kj -= 1.0;
                wolf.action = Action::Wander;
                let p = patch(wolf.kin.position, w, h);
                let mut prey = None;
                if !on_patch[p].is_empty() {
                    let k = rng.index(on_patch[p].len());
                    prey = Some(on_patch[p].swap_remove(k));
                    wolf.energy_kj += cfg.wolf_gain_from_food;
                    wolf.action = Action::Feed;
                    wolf.stats.kills += 1;
                }
                let dead = wolf.energy_kj < 0.0;
                let mut cub = None;
                if !dead && rng.chance(f64::from(cfg.wolf_reproduce_percent) / 100.0) {
                    wolf.energy_kj /= 2.0;
                    wolf.stats.offspring += 1;
                    cub = Some((wolf.lineage, wolf.kin.position, wolf.energy_kj));
                }
                (prey, dead, cub)
            };
            if let Some(prey) = prey {
                self.animals.queue_despawn(prey);
                self.counters.death(nl.sheep, DeathCause::Predation);
                self.counters.kills += 1;
                self.genealogy.record_death(prey, self.tick)?;
            }
            if dead {
                self.animals.queue_despawn(id);
                self.counters.death(nl.wolf, DeathCause::Starvation);
                self.genealogy.record_death(id, self.tick)?;
            }
            cubs.extend(cub);
        }
        for (parent, position, energy) in cubs {
            self.hatch(rt, nl.wolf, &parent, position, energy, &mut rng, wf, hf)?;
        }
        self.animals.commit();
        // 3. Grass.
        if cfg.grass {
            for i in 0..nl.green.len() {
                if !nl.green[i] {
                    if nl.countdown[i] <= 0 {
                        nl.green[i] = true;
                        nl.countdown[i] = i64::from(cfg.grass_regrowth_time);
                    } else {
                        nl.countdown[i] -= 1;
                    }
                }
            }
        }
        Ok(())
    }

    /// `hatch 1 [rt random-float 360 fd 1]` with the parent's (halved) energy.
    #[allow(clippy::too_many_arguments)]
    fn hatch(
        &mut self,
        rt: &Runtime,
        species: u16,
        parent: &Lineage,
        position: Vec2,
        energy: f32,
        rng: &mut SimRng,
        w: f32,
        h: f32,
    ) -> Result<(), EcosystemError> {
        let mut kin = Kinematics::new(position, (rng.f32() * 360.0).to_radians());
        step_forward(&mut kin, 1.0, w, h);
        spawn(self, rt, species, kin.position, kin.heading, energy, Some(parent))?;
        self.counters.birth(species);
        Ok(())
    }
}

/// Spawns one NetLogo animal at `position` with `random(2 × gain)` energy
/// (spawn command of the NetLogo variant). Queued until the next commit.
pub(crate) fn spawn_with_energy(
    state: &mut EcoState,
    rt: &Runtime,
    species: u16,
    position: Vec2,
    gain: f32,
) -> Result<AgentId, EcosystemError> {
    let heading = (random_int(&mut state.rng, 360) as f32).to_radians();
    let energy = random_int(&mut state.rng, (2.0 * gain).floor() as u32) as f32;
    spawn(state, rt, species, position, heading, energy, None)
}

#[cfg(test)]
mod tests {
    use crate::scenario::find_builtin;
    use crate::state::{EcoState, Runtime};

    fn state(sheep: u32, wolves: u32) -> (EcoState, Runtime) {
        let mut scenario = find_builtin("wolf_sheep_grass").unwrap();
        if let crate::config::Rules::NetLogo(n) = &mut scenario.config.rules {
            n.initial_sheep = sheep;
            n.initial_wolves = wolves;
        }
        let catalog = crate::SpeciesCatalog::builtin().unwrap();
        EcoState::build(scenario.config, 3, catalog, crate::params::RuntimeParams::default()).unwrap()
    }

    #[test]
    fn grazed_patches_regrow_after_regrowth_time_plus_one_ticks() {
        let (mut st, rt) = state(0, 0);
        let regrowth = 30;
        {
            let nl = st.netlogo.as_mut().unwrap();
            nl.green.iter_mut().for_each(|g| *g = false);
            nl.countdown.iter_mut().for_each(|c| *c = regrowth);
        }
        for _ in 0..regrowth {
            st.netlogo_tick(&rt).unwrap();
            st.tick += 1;
            assert_eq!(st.netlogo.as_ref().unwrap().green_patches(), 0);
        }
        st.netlogo_tick(&rt).unwrap();
        let nl = st.netlogo.as_ref().unwrap();
        assert_eq!(nl.green_patches(), nl.green.len(), "brown for exactly regrowth + 1 ticks");
        assert!(nl.countdown.iter().all(|&c| c == regrowth), "countdown reset when turning green");
    }

    #[test]
    fn sheep_without_food_lose_one_energy_per_tick_and_die_below_zero() {
        let (mut st, rt) = state(1, 0);
        {
            let nl = st.netlogo.as_mut().unwrap();
            nl.green.iter_mut().for_each(|g| *g = false);
            nl.countdown.iter_mut().for_each(|c| *c = 1_000);
        }
        let id = st.animals.ids()[0];
        st.animals.get_mut(id).unwrap().energy_kj = 2.0;
        // Reproduction would halve the energy; make it impossible.
        if let crate::config::Rules::NetLogo(n) = &mut st.config.rules {
            n.sheep_reproduce_percent = 0.0;
        }
        for expected in [1.0, 0.0] {
            st.netlogo_tick(&rt).unwrap();
            st.tick += 1;
            assert_eq!(st.animals.get(id).unwrap().energy_kj, expected);
        }
        st.netlogo_tick(&rt).unwrap();
        assert!(st.animals.get(id).is_none(), "energy −1 < 0: the sheep dies");
    }

    #[test]
    fn a_wolf_eats_at_most_one_sheep_on_its_patch_per_tick() {
        let (mut st, rt) = state(3, 1);
        if let crate::config::Rules::NetLogo(n) = &mut st.config.rules {
            n.sheep_reproduce_percent = 0.0;
            n.wolf_reproduce_percent = 0.0;
        }
        // Put everybody on one patch; sheep and the wolf all move 1 first,
        // so use a 1-patch-wide check: stack them and make moves land on the
        // same patch by giving every animal the same heading and position.
        let p = simulation_core::glam::Vec2::new(10.5, 10.5);
        for a in st.animals.as_mut_slice() {
            a.kin.position = p;
            a.kin.heading = 0.0;
            a.energy_kj = 50.0;
        }
        let wolf = st.netlogo.as_ref().unwrap().wolf;
        let sheep = st.netlogo.as_ref().unwrap().sheep;
        let before = st.animals.as_slice().iter().filter(|a| a.species == sheep).count();
        st.netlogo_tick(&rt).unwrap();
        let after = st.animals.as_slice().iter().filter(|a| a.species == sheep).count();
        let w = st.animals.as_slice().iter().find(|a| a.species == wolf).unwrap();
        // Random turns of up to ±49° and a unit step can leave the patch, so
        // the wolf eats one sheep or none, never more.
        assert!(before - after <= 1);
        assert!(w.energy_kj == 49.0 || w.energy_kj == 69.0, "energy {}", w.energy_kj);
    }
}
