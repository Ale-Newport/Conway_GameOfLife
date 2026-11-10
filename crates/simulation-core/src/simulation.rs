//! The [`Simulation`] trait implemented by every model.
//!
//! Design rules shared by all implementations:
//!
//! * **Ticks, not frames.** A model advances in discrete ticks of fixed
//!   simulated duration ([`ModelDescriptor::dt`]). How many ticks run per
//!   second is decided by the caller (the [`crate::runner::Runner`], a test or
//!   the CLI); rendering frame rate never changes results.
//! * **Determinism.** All randomness comes from a [`crate::SimRng`] stored in
//!   the model state. Parallel code (rayon) must only compute pure functions
//!   over read-only buffers, with any conflicts resolved in a fixed order.
//! * **Commands.** Every external mutation (editing cells, spawning an
//!   organism, starting a fire) goes through [`Simulation::apply`], so inputs
//!   can be validated, recorded and replayed.
//! * **Views.** Renderers never borrow model internals; they receive an owned
//!   [`Simulation::View`] built on request for the region they display.

use crate::error::SimError;
use crate::metrics::Metrics;
use crate::model::ModelDescriptor;
use crate::params::{ApplyPolicy, ParamSet, ParamValue};
use crate::snapshot::Snapshot;

/// A deterministic, tick-based simulation model.
pub trait Simulation: Send + 'static {
    /// Mode-specific mutations (edits, interventions, rule changes).
    type Command: Send + 'static;
    /// What a renderer asks for (visible region, level of detail...).
    type ViewRequest: Clone + Default + Send + Sync + 'static;
    /// Owned, render-ready data produced for a [`Simulation::ViewRequest`].
    type View: Send + Sync + 'static;

    /// Formal description of the model, its parameters and metrics.
    fn descriptor(&self) -> ModelDescriptor;

    /// Number of ticks executed since the last reset. For cellular models one
    /// tick is one generation; engines that can jump several generations in
    /// one call (Hashlife) still report generations here.
    fn tick(&self) -> u64;

    /// Advances the model by one tick (or by its configured step size).
    fn step(&mut self);

    /// Advances by up to `max_ticks` ticks and returns how many generations
    /// were actually advanced. Engines with faster bulk evolution override it.
    fn advance(&mut self, max_ticks: u64) -> u64 {
        let start = self.tick();
        for _ in 0..max_ticks {
            self.step();
            if self.is_finished() {
                break;
            }
        }
        self.tick() - start
    }

    /// Restores the initial state of the current scenario (same seed).
    fn reset(&mut self);

    /// Applies an external command.
    fn apply(&mut self, command: Self::Command) -> Result<(), SimError>;

    /// Current parameter values.
    fn params(&self) -> ParamSet;

    /// Changes one parameter and reports when the change becomes effective.
    /// The default implementation rejects every key.
    fn set_param(&mut self, key: &str, value: ParamValue) -> Result<ApplyPolicy, SimError> {
        let _ = value;
        Err(SimError::UnknownParameter(key.to_string()))
    }

    /// Measurements for the current tick.
    fn metrics(&self) -> Metrics;

    /// Builds render data for the requested region / level of detail.
    fn view(&self, request: &Self::ViewRequest) -> Self::View;

    /// Captures the complete state, including random generator state.
    fn snapshot(&self) -> Result<Snapshot, SimError>;

    /// Restores a state previously captured with [`Simulation::snapshot`].
    fn restore(&mut self, snapshot: &Snapshot) -> Result<(), SimError>;

    /// Approximate memory used by the model state, in bytes.
    fn memory_bytes(&self) -> usize {
        0
    }

    /// Whether a scenario end condition has been reached (extinction, a
    /// configured tick limit...). Runners pause when this becomes true.
    fn is_finished(&self) -> bool {
        false
    }
}

/// Object-safe subset of [`Simulation`] used by tools that handle many model
/// types uniformly (CLI experiments, batch comparisons, the model registry).
pub trait DynSimulation: Send {
    /// See [`Simulation::descriptor`].
    fn descriptor(&self) -> ModelDescriptor;
    /// See [`Simulation::tick`].
    fn tick(&self) -> u64;
    /// See [`Simulation::step`].
    fn step(&mut self);
    /// See [`Simulation::advance`].
    fn advance(&mut self, max_ticks: u64) -> u64;
    /// See [`Simulation::reset`].
    fn reset(&mut self);
    /// See [`Simulation::params`].
    fn params(&self) -> ParamSet;
    /// See [`Simulation::set_param`].
    fn set_param(&mut self, key: &str, value: ParamValue) -> Result<ApplyPolicy, SimError>;
    /// See [`Simulation::metrics`].
    fn metrics(&self) -> Metrics;
    /// See [`Simulation::snapshot`].
    fn snapshot(&self) -> Result<Snapshot, SimError>;
    /// See [`Simulation::restore`].
    fn restore(&mut self, snapshot: &Snapshot) -> Result<(), SimError>;
    /// See [`Simulation::memory_bytes`].
    fn memory_bytes(&self) -> usize;
    /// See [`Simulation::is_finished`].
    fn is_finished(&self) -> bool;
}

/// Adapter that exposes any [`Simulation`] as a [`DynSimulation`].
///
/// A wrapper is used instead of a blanket implementation so that both traits
/// can be imported together without method-name ambiguity.
pub struct Erased<S>(pub S);

/// Boxes a simulation behind the object-safe [`DynSimulation`] interface.
pub fn erase<S: Simulation>(sim: S) -> Box<dyn DynSimulation> {
    Box::new(Erased(sim))
}

impl<S: Simulation> DynSimulation for Erased<S> {
    fn descriptor(&self) -> ModelDescriptor {
        self.0.descriptor()
    }
    fn tick(&self) -> u64 {
        self.0.tick()
    }
    fn step(&mut self) {
        self.0.step();
    }
    fn advance(&mut self, max_ticks: u64) -> u64 {
        self.0.advance(max_ticks)
    }
    fn reset(&mut self) {
        self.0.reset();
    }
    fn params(&self) -> ParamSet {
        self.0.params()
    }
    fn set_param(&mut self, key: &str, value: ParamValue) -> Result<ApplyPolicy, SimError> {
        self.0.set_param(key, value)
    }
    fn metrics(&self) -> Metrics {
        self.0.metrics()
    }
    fn snapshot(&self) -> Result<Snapshot, SimError> {
        self.0.snapshot()
    }
    fn restore(&mut self, snapshot: &Snapshot) -> Result<(), SimError> {
        self.0.restore(snapshot)
    }
    fn memory_bytes(&self) -> usize {
        self.0.memory_bytes()
    }
    fn is_finished(&self) -> bool {
        self.0.is_finished()
    }
}

#[cfg(test)]
pub(crate) mod test_model {
    //! A tiny deterministic model used by the core crate's own tests.

    use serde::{Deserialize, Serialize};

    use super::*;
    use crate::model::ModelFamily;
    use crate::params::ParamSpec;
    use crate::rng::SimRng;

    /// Random walk counter: `value += rng.below(step_max + 1)` per tick.
    #[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
    pub struct Walker {
        pub seed: u64,
        pub tick: u64,
        pub value: u64,
        pub step_max: u64,
        pub rng: SimRng,
    }

    impl Walker {
        pub fn new(seed: u64) -> Self {
            Self { seed, tick: 0, value: 0, step_max: 3, rng: SimRng::seed_from_u64(seed) }
        }
    }

    pub enum WalkerCommand {
        Add(u64),
        Fail,
    }

    impl Simulation for Walker {
        type Command = WalkerCommand;
        type ViewRequest = ();
        type View = u64;

        fn descriptor(&self) -> ModelDescriptor {
            ModelDescriptor::new("test.walker", "Walker", ModelFamily::AgentBased)
                .rules(&format!("value += uniform integer in 0..={}", self.step_max))
                .params(vec![ParamSpec::int("step_max", "Max step", 0, 100, 3)])
        }
        fn tick(&self) -> u64 {
            self.tick
        }
        fn step(&mut self) {
            self.value += self.rng.below(self.step_max + 1);
            self.tick += 1;
        }
        fn reset(&mut self) {
            *self = Self::new(self.seed);
        }
        fn apply(&mut self, command: WalkerCommand) -> Result<(), SimError> {
            match command {
                WalkerCommand::Add(n) => {
                    self.value += n;
                    Ok(())
                }
                WalkerCommand::Fail => Err(SimError::InvalidCommand("requested failure".into())),
            }
        }
        fn params(&self) -> ParamSet {
            ParamSet::new().with("step_max", ParamValue::Int(self.step_max as i64))
        }
        fn set_param(&mut self, key: &str, value: ParamValue) -> Result<ApplyPolicy, SimError> {
            let spec = self.descriptor().param(key).cloned().ok_or_else(|| SimError::UnknownParameter(key.into()))?;
            let v = spec.validate(&value)?;
            self.step_max = v.as_i64().unwrap_or(3) as u64;
            Ok(spec.apply)
        }
        fn metrics(&self) -> Metrics {
            Metrics::new(self.tick, self.tick as f64).with("value", self.value as f64)
        }
        fn view(&self, _request: &()) -> u64 {
            self.value
        }
        fn snapshot(&self) -> Result<Snapshot, SimError> {
            Snapshot::encode("test.walker", 1, self.tick, self)
        }
        fn restore(&mut self, snapshot: &Snapshot) -> Result<(), SimError> {
            *self = snapshot.decode("test.walker", 1)?;
            Ok(())
        }
    }

    #[test]
    fn dyn_dispatch_and_snapshot_replay() {
        let mut sim: Box<dyn DynSimulation> = erase(Walker::new(1));
        sim.advance(10);
        let snap = sim.snapshot().unwrap();
        sim.advance(25);
        let later = sim.metrics().get("value");
        sim.restore(&snap).unwrap();
        assert_eq!(sim.tick(), 10);
        sim.advance(25);
        assert_eq!(sim.metrics().get("value"), later, "restored run must replay identically");
    }
}
