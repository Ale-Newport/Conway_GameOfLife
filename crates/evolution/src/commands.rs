//! User interventions. Every command is validated first; on error nothing
//! changes and the message says why.

use organism_engine::AgentId;
use serde::{Deserialize, Serialize};
use simulation_core::{EventSeverity, SimEvent, SimRng};

use crate::config::Shape;
use crate::error::EvolutionError;
use crate::genes::TraitValue;
use crate::organism::Species;
use crate::world::{DeathCause, EvolutionWorld, TickReport};

/// Random stream tag for command-driven randomness (spawns, culls). The
/// n-th randomised command of a run uses `derive(seed ^ tag, n)`, so
/// commands never perturb the simulation's own generator and replay
/// identically (the counter is part of snapshots).
const COMMAND_STREAM: u64 = 0x434F_4D4D_414E_4400;

/// An intervention in a running experiment.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvolutionCommand {
    /// Adds `amount` food to every land cell inside `area` (food above
    /// capacity decays back logistically).
    AddFood {
        /// Area fed.
        area: Shape,
        /// Food units added per cell.
        amount: f32,
    },
    /// Shifts the whole temperature field by `delta` °C at once.
    ChangeTemperature {
        /// Temperature jump (°C).
        delta: f32,
    },
    /// Sets the directional climate shift (°C per 1000 ticks).
    SetTemperatureShift {
        /// New rate.
        per_1000_ticks: f32,
    },
    /// Switches a configured barrier on or off.
    SetBarrier {
        /// Barrier index in the scenario.
        index: usize,
        /// New state.
        active: bool,
    },
    /// Toggles a configured barrier.
    ToggleBarrier {
        /// Barrier index in the scenario.
        index: usize,
    },
    /// Adds predators with the given gene means (empty: defaults).
    IntroducePredators {
        /// Number of predators.
        count: u32,
        /// Where to place them (anywhere on land if `None`).
        area: Option<Shape>,
        /// Gene means.
        traits: Vec<TraitValue>,
    },
    /// Kills a random fraction of the matching organisms.
    Cull {
        /// Fraction in `[0, 1]` removed (rounded to whole organisms).
        fraction: f32,
        /// Only this species (both if `None`).
        species: Option<Species>,
        /// Only inside this area (everywhere if `None`).
        area: Option<Shape>,
    },
    /// Spawns organisms with a custom genome (new lineages).
    Spawn {
        /// Species.
        species: Species,
        /// Number of organisms.
        count: u32,
        /// Where to place them.
        area: Option<Shape>,
        /// Gene values (phenotypic units); missing genes use defaults.
        traits: Vec<TraitValue>,
        /// Gene spread around the values (normalised units, 0 = identical clones).
        spread: f32,
    },
}

impl EvolutionWorld {
    /// Applies a command.
    pub fn apply(&mut self, command: EvolutionCommand) -> Result<(), EvolutionError> {
        let message = match command {
            EvolutionCommand::AddFood { area, amount } => {
                area.validate("area")?;
                if !(amount.is_finite() && (0.0..=1000.0).contains(&amount)) {
                    return Err(EvolutionError::InvalidCommand(format!("food amount {amount} is outside [0, 1000]")));
                }
                let added = self.env.add_food(&area, amount);
                format!("added {added:.0} food units")
            }
            EvolutionCommand::ChangeTemperature { delta } => {
                if !(delta.is_finite() && (-50.0..=50.0).contains(&delta)) {
                    return Err(EvolutionError::InvalidCommand(format!(
                        "temperature change {delta} is outside [-50, 50] °C"
                    )));
                }
                self.env.shift_temperature(delta);
                format!("temperature changed by {delta:+.1} °C")
            }
            EvolutionCommand::SetTemperatureShift { per_1000_ticks } => {
                crate::params::set(
                    &mut self.config,
                    "temperature_shift_rate",
                    &simulation_core::ParamValue::Float(f64::from(per_1000_ticks)),
                )
                .map_err(|e| EvolutionError::InvalidCommand(e.to_string()))?;
                format!("climate shift set to {per_1000_ticks:+.2} °C per 1000 ticks")
            }
            EvolutionCommand::SetBarrier { index, active } => self.switch_barrier(index, Some(active))?,
            EvolutionCommand::ToggleBarrier { index } => self.switch_barrier(index, None)?,
            EvolutionCommand::IntroducePredators { count, area, traits } => {
                if let Some(a) = &area {
                    a.validate("area")?;
                }
                let mut rng = self.command_rng();
                let spread = self.config.predators.spread;
                let ids = self.spawn_with(Species::Predator, count, area.as_ref(), &traits, spread, &mut rng)?;
                format!("introduced {} predators", ids.len())
            }
            EvolutionCommand::Cull { fraction, species, area } => {
                if !(fraction.is_finite() && (0.0..=1.0).contains(&fraction)) {
                    return Err(EvolutionError::InvalidCommand(format!("cull fraction {fraction} is outside [0, 1]")));
                }
                if let Some(a) = &area {
                    a.validate("area")?;
                }
                let removed = self.cull(fraction, species, area.as_ref());
                format!("culled {removed} organisms")
            }
            EvolutionCommand::Spawn { species, count, area, traits, spread } => {
                if let Some(a) = &area {
                    a.validate("area")?;
                }
                let mut rng = self.command_rng();
                let ids = self.spawn_with(species, count, area.as_ref(), &traits, spread, &mut rng)?;
                format!("spawned {} {} with a custom genome", ids.len(), species.label())
            }
        };
        self.events.push(SimEvent::new(self.tick, "intervention", message).severity(EventSeverity::Info));
        self.refresh_stats();
        Ok(())
    }

    /// Random stream for the next randomised command.
    fn command_rng(&mut self) -> SimRng {
        self.commands_applied += 1;
        SimRng::derive(self.seed ^ COMMAND_STREAM, self.commands_applied)
    }

    fn switch_barrier(&mut self, index: usize, active: Option<bool>) -> Result<String, EvolutionError> {
        let current = self.env.barrier_active().get(index).copied().ok_or_else(|| {
            EvolutionError::InvalidCommand(format!(
                "barrier {index} does not exist (the scenario has {})",
                self.config.barriers.len()
            ))
        })?;
        let active = active.unwrap_or(!current);
        self.env.set_barrier(&self.config, index, active)?;
        let name = &self.config.barriers[index].name;
        Ok(format!("barrier `{name}` {}", if active { "raised" } else { "removed" }))
    }

    /// Removes `round(fraction · n)` of the `n` matching organisms, chosen
    /// uniformly (partial Fisher–Yates shuffle on the command stream).
    fn cull(&mut self, fraction: f32, species: Option<Species>, area: Option<&Shape>) -> usize {
        let mut candidates: Vec<AgentId> = self
            .store
            .iter()
            .filter(|(_, o)| species.is_none_or(|s| o.species == s))
            .filter(|(_, o)| area.is_none_or(|a| a.contains(o.kinematics.position.x, o.kinematics.position.y)))
            .map(|(id, _)| id)
            .collect();
        let k = (fraction * candidates.len() as f32).round() as usize;
        let mut rng = self.command_rng();
        for i in 0..k {
            let j = i + rng.index(candidates.len() - i);
            candidates.swap(i, j);
        }
        let mut chosen: Vec<(AgentId, DeathCause)> =
            candidates[..k].iter().map(|&id| (id, DeathCause::Culled)).collect();
        chosen.sort_unstable();
        let mut report = TickReport::default();
        self.bury(&chosen, &mut report);
        self.store.commit();
        for (slot, n) in self.pending.deaths.iter_mut().zip(report.deaths) {
            *slot += n;
        }
        k
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Barrier, EvolutionConfig, Terrain};

    fn world() -> EvolutionWorld {
        let mut c = EvolutionConfig::default();
        c.world.width = 40;
        c.world.height = 30;
        c.prey.initial = 60;
        c.barriers.push(Barrier {
            name: "river".into(),
            terrain: Terrain::Water,
            shapes: vec![Shape::Rect { x: 19.0, y: 0.0, w: 2.0, h: 30.0 }],
            outside: false,
            active: false,
        });
        EvolutionWorld::new(c, 5).unwrap()
    }

    #[test]
    fn spawn_with_custom_genome_and_cull() {
        let mut w = world();
        let traits = vec![TraitValue::new("speed", 1.5), TraitValue::new("size", 2.0)];
        w.apply(EvolutionCommand::Spawn { species: Species::Prey, count: 10, area: None, traits, spread: 0.0 })
            .unwrap();
        assert_eq!(w.count(Species::Prey), 70);
        let spawned: Vec<_> = w.organisms().iter().rev().take(10).map(|(_, o)| o.phenotype(w.schema())).collect();
        assert!(spawned.iter().all(|p| (p.speed - 1.5).abs() < 1e-5 && (p.size - 2.0).abs() < 1e-5));
        w.apply(EvolutionCommand::Cull { fraction: 0.5, species: Some(Species::Prey), area: None }).unwrap();
        assert_eq!(w.count(Species::Prey), 35);
        assert_eq!(w.genealogy().summary().living, 35);
        w.step();
        assert!(w.last_report().deaths[DeathCause::Culled.index()] == 35);
    }

    #[test]
    fn invalid_commands_change_nothing() {
        let mut w = world();
        let before = w.organisms().len();
        let bad_trait = vec![TraitValue::new("acclimation", 1.0)];
        let spawn =
            EvolutionCommand::Spawn { species: Species::Prey, count: 5, area: None, traits: bad_trait, spread: 0.0 };
        assert!(matches!(w.apply(spawn), Err(EvolutionError::AcquiredTrait(_))));
        assert!(w.apply(EvolutionCommand::Cull { fraction: 1.5, species: None, area: None }).is_err());
        assert!(w.apply(EvolutionCommand::ToggleBarrier { index: 4 }).is_err());
        assert!(
            w.apply(EvolutionCommand::AddFood { area: Shape::Disc { x: 1.0, y: 1.0, r: -2.0 }, amount: 1.0 }).is_err()
        );
        let ocean = Some(Shape::Rect { x: 19.2, y: 0.0, w: 1.5, h: 30.0 });
        w.apply(EvolutionCommand::SetBarrier { index: 0, active: true }).unwrap();
        let spawn =
            EvolutionCommand::Spawn { species: Species::Prey, count: 5, area: ocean, traits: vec![], spread: 0.0 };
        assert!(w.apply(spawn).is_err(), "no land in the area");
        assert_eq!(w.organisms().len(), before);
    }

    #[test]
    fn environment_commands() {
        let mut w = world();
        w.apply(EvolutionCommand::ToggleBarrier { index: 0 }).unwrap();
        assert_eq!(w.environment().region_count(), 2);
        w.apply(EvolutionCommand::ToggleBarrier { index: 0 }).unwrap();
        assert_eq!(w.environment().region_count(), 1);
        let before = w.environment().total_food();
        w.apply(EvolutionCommand::AddFood { area: Shape::Disc { x: 5.0, y: 5.0, r: 3.0 }, amount: 2.0 }).unwrap();
        assert!(w.environment().total_food() > before);
        w.apply(EvolutionCommand::ChangeTemperature { delta: 3.0 }).unwrap();
        assert_eq!(w.environment().temperature_offset(), 3.0);
        w.apply(EvolutionCommand::SetTemperatureShift { per_1000_ticks: 5.0 }).unwrap();
        assert_eq!(w.config().climate.shift_rate, 5.0);
        assert!(w.apply(EvolutionCommand::SetTemperatureShift { per_1000_ticks: 500.0 }).is_err());
        w.apply(EvolutionCommand::IntroducePredators { count: 4, area: None, traits: vec![] }).unwrap();
        assert_eq!(w.count(Species::Predator), 4);
    }
}
