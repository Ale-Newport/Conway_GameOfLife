//! Snapshot payload of [`super::MetapixelSim`] (schema version 1).

use serde::{Deserialize, Serialize};
use simulation_core::SimRng;

use super::{MetapixelSim, Predictor, ReadoutHistory, StepSize, WorldConfig};
use crate::accelerated::MacroModel;
use crate::build::BuildStats;
use crate::error::MetapixelError;
use crate::macro_grid::MacroGrid;
use crate::readout::TileGeometry;
use crate::spec::PERIOD;

/// Everything needed to continue a run identically: the physical pattern
/// (Golly macrocell text, which carries the generation), the macro
/// pattern it was built from, both configurations, the readout history,
/// the accelerated predictor and the random generator.
#[derive(Serialize, Deserialize)]
pub(super) struct SavedState {
    active: WorldConfig,
    pending: WorldConfig,
    initial: MacroGrid,
    step: StepSize,
    node_limit: u64,
    rng: SimRng,
    macrocell: String,
    generation: u64,
    history: ReadoutHistory,
    predictor: Predictor,
    build_stats: BuildStats,
    halted: Option<String>,
}

impl SavedState {
    pub(super) fn capture(sim: &MetapixelSim) -> Self {
        Self {
            active: sim.active,
            pending: sim.pending,
            initial: sim.initial.clone(),
            step: sim.step,
            node_limit: sim.universe.node_limit() as u64,
            rng: sim.rng.clone(),
            macrocell: hashlife::macrocell::write(&sim.universe),
            generation: sim.universe.generation(),
            history: sim.history.clone(),
            predictor: sim.predictor.clone(),
            build_stats: sim.build_stats,
            halted: sim.halted.clone(),
        }
    }

    /// Validates the payload and replaces the model state; on error nothing
    /// changes.
    pub(super) fn apply(self, sim: &mut MetapixelSim) -> Result<(), MetapixelError> {
        let corrupt = |message: String| MetapixelError::InvalidPattern(format!("snapshot: {message}"));
        for config in [self.active, self.pending] {
            MacroGrid::new(config.width, config.height)?;
        }
        let (width, height) = (self.active.width, self.active.height);
        if (self.initial.width(), self.initial.height()) != (width, height) {
            return Err(corrupt("the macro pattern does not match the grid size".into()));
        }
        let predicted = self.predictor.grid();
        if (predicted.width(), predicted.height()) != (width, height) {
            return Err(corrupt("the accelerated prediction does not match the grid size".into()));
        }
        // The predictor is advanced to metagen n when n is read, at (n + 1)·P.
        if self.predictor.metagen() > (self.generation / PERIOD).saturating_sub(1) {
            return Err(corrupt(format!(
                "the accelerated prediction is at metagen {} but generation {} has not read it yet",
                self.predictor.metagen(),
                self.generation
            )));
        }
        self.history.validate(width, height, self.generation).map_err(corrupt)?;
        if let StepSize::Pow2(k) = self.step
            && k > super::MAX_STEP_EXPONENT
        {
            return Err(corrupt(format!("step exponent {k} is out of range")));
        }
        let node_limit = usize::try_from(self.node_limit)
            .ok()
            .filter(|&n| (hashlife::MIN_NODE_LIMIT..=super::MAX_NODE_LIMIT as usize).contains(&n))
            .ok_or_else(|| corrupt(format!("node limit {} is out of range", self.node_limit)))?;
        let mut universe = hashlife::Universe::conway();
        hashlife::macrocell::read_into(&mut universe, &self.macrocell)?;
        if universe.generation() != self.generation {
            return Err(corrupt(format!(
                "pattern generation {} differs from the recorded generation {}",
                universe.generation(),
                self.generation
            )));
        }
        if universe.rule().to_string() != "B3/S23" {
            return Err(corrupt(format!("the physical rule must be B3/S23, found {}", universe.rule())));
        }
        universe.set_node_limit(node_limit);
        if self.active.programming != sim.active.programming {
            sim.programmed = sim.tiles.program(self.active.programming);
        }
        sim.model = MacroModel::new(self.active.programming);
        sim.geometry = TileGeometry::at_origin(self.active.width, self.active.height);
        sim.active = self.active;
        sim.pending = self.pending;
        sim.initial = self.initial;
        sim.step = self.step;
        sim.rng = self.rng;
        sim.universe = universe;
        sim.history = self.history;
        sim.predictor = self.predictor;
        sim.build_stats = self.build_stats;
        sim.halted = self.halted;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::tile::TileSet;

    fn sim() -> MetapixelSim {
        let tiles = Arc::new(TileSet::from_dir(&TileSet::source_tree_dir()).unwrap());
        let pattern = MacroGrid::from_rows(&["O.", ".O"]).unwrap();
        let config = WorldConfig { width: 2, height: 2, ..WorldConfig::default() };
        MetapixelSim::new(tiles, config, pattern).unwrap()
    }

    #[test]
    fn inconsistent_payloads_are_rejected_and_change_nothing() {
        let mut target = sim();
        let population = target.universe.population();
        type Corruption = (&'static str, fn(&mut SavedState));
        let corruptions: [Corruption; 5] = [
            ("prediction size", |s| s.predictor = Predictor::new(MacroGrid::new(3, 2).unwrap())),
            ("prediction ahead", |s| {
                let model = MacroModel::new(s.active.programming);
                s.predictor.advance_to(&model, 1);
            }),
            ("initial size", |s| s.initial = MacroGrid::new(1, 1).unwrap()),
            ("generation", |s| s.generation += 1),
            ("node limit", |s| s.node_limit = 1),
        ];
        for (what, corrupt) in corruptions {
            let mut saved = SavedState::capture(&target);
            corrupt(&mut saved);
            assert!(saved.apply(&mut target).is_err(), "{what}");
            assert_eq!(target.universe.population(), population, "{what}: unchanged");
        }
        // A consistent payload is accepted.
        assert!(SavedState::capture(&target).apply(&mut target).is_ok());
    }
}
