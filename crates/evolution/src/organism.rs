//! Organisms: heritable genome plus individual (non-heritable) state, and the
//! physiology that turns phenotypes into energy costs.
//!
//! Per tick, with phenotype `p` (size `M`, efficiency `η`, tolerance `σ`) and
//! the temperature `T` under the organism:
//!
//! ```text
//! mismatch      m = (T − (T_opt + acclimation)) / σ
//! basal         b₀ · M^0.75 / η · (1 + c_σ (σ − 1)) · (1 + c_T · min(m², 9))
//! movement      c_m · M · v²                (v = speed actually used this tick)
//! speed upkeep  c_u · M · v_max²            (locomotor capacity, paid every tick)
//! perception    c_p · radius
//! storage       E_max = s · M               (energy above E_max is not stored)
//! intake        i₀ · M^0.75 / η             (food units per tick: rate–yield trade-off)
//! health        −= d · (|m| − 1) if |m| > 1, else += recovery (max 1)
//! senescence    h(a) = h₀ · exp(ln 1000 · a / (L · maturity))   (Gompertz)
//! ```
//!
//! Acquired traits (non-heritable genome slots) change during life only:
//! acclimation moves towards `T − T_opt` at `acclimation_rate` per tick
//! (limited to ±4 °C by the gene range); experience grows by
//! `learning_rate · (1 − e)` on every tick spent escaping a predator and
//! decays by `forgetting_rate · e` otherwise.

use organism_engine::steering::WanderState;
use organism_engine::{AgentId, Bounds, Genome, GenomeSchema, Kinematics, Lineage, MotionLimits};
use serde::{Deserialize, Serialize};
use simulation_core::glam::Vec2;

use crate::behaviour::Decision;
use crate::config::{EvolutionConfig, PhysiologyConfig};
use crate::environment::Environment;
use crate::genes::{ACCLIMATION, EXPERIENCE, Phenotype};

/// The two populations of the laboratory.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Species {
    /// Herbivores eating food from the cells.
    Prey,
    /// Carnivores hunting prey.
    Predator,
}

impl Species {
    /// Display label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Prey => "prey",
            Self::Predator => "predator",
        }
    }
}

/// What an organism did in its last tick (for display and for the
/// interaction phase).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Activity {
    /// Random search at cruising speed.
    #[default]
    Wander,
    /// Moving towards perceived food.
    Forage,
    /// Standing on food and eating.
    Graze,
    /// Escaping a predator at full speed.
    Flee,
    /// Pursuing a prey at full speed.
    Hunt,
    /// Approaching or contacting a mate.
    Court,
    /// Satiated or handling predator idling at reduced speed.
    Rest,
    /// Hungry predator that lost the tick to an encounter with a rival.
    Contest,
}

/// One organism. The genome holds the heritable genes and the two acquired
/// traits; everything else is individual state that is never inherited.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Organism {
    /// Prey or predator.
    pub species: Species,
    /// Position, velocity and heading.
    pub kinematics: Kinematics,
    /// State of the wander behaviour.
    pub wander: WanderState,
    /// Genes (normalised).
    pub genome: Genome,
    /// Energy reserve.
    pub energy: f32,
    /// Health in `[0, 1]`; thermal stress lowers it, death at 0.
    pub health: f32,
    /// Age in ticks.
    pub age: u32,
    /// Ticks until the organism may breed again.
    pub refractory: u32,
    /// Ticks a predator still spends handling (eating) its last kill.
    pub handling: u32,
    /// Parents, generation and birth tick.
    pub lineage: Lineage,
    /// Founder at the root of the organism's line of first parents (a
    /// "matriline"; used for lineage counts).
    pub founder: AgentId,
    /// Offspring produced so far.
    pub offspring: u32,
    /// Behaviour of the last tick.
    pub activity: Activity,
    /// Prey hunted, predator fled from or mate courted in the last tick.
    pub target: Option<AgentId>,
}

impl Organism {
    /// A new organism at rest.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        species: Species,
        position: Vec2,
        heading: f32,
        genome: Genome,
        energy: f32,
        age: u32,
        lineage: Lineage,
        founder: AgentId,
    ) -> Self {
        Self {
            species,
            kinematics: Kinematics::new(position, heading),
            wander: WanderState::default(),
            genome,
            energy,
            health: 1.0,
            age,
            refractory: 0,
            handling: 0,
            lineage,
            founder,
            offspring: 0,
            activity: Activity::Wander,
            target: None,
        }
    }

    /// Expressed phenotype.
    pub fn phenotype(&self, schema: &GenomeSchema) -> Phenotype {
        Phenotype::express(&self.genome, schema)
    }

    /// Energy as a fraction of storage capacity.
    pub fn condition(&self, p: &Phenotype, phys: &PhysiologyConfig) -> f32 {
        self.energy / storage(p, phys)
    }

    /// Movement, energy budget, thermal physiology, acquired traits and
    /// ageing for one tick (pure in everything but `self`).
    pub(crate) fn live(
        &mut self,
        decision: &Decision,
        env: &Environment,
        config: &EvolutionConfig,
        schema: &GenomeSchema,
        tick: u64,
    ) {
        let phys = &config.physiology;
        let p = self.phenotype(schema);
        let limits = MotionLimits::new(decision.speed_cap, p.speed * phys.agility);
        let max = Vec2::new(env.width() as f32, env.height() as f32) - Vec2::splat(1e-3);
        let bounds = Bounds::Clamp { min: Vec2::ZERO, max };
        self.kinematics.integrate_with(decision.accel, 1.0, limits, bounds, |q| env.is_passable(q));
        self.wander = decision.wander;
        self.activity = decision.activity;
        self.target = decision.target;

        let temperature = env.temperature_at(&config.climate, self.kinematics.position, tick);
        let mismatch = thermal_mismatch(&p, temperature);
        let speed = self.kinematics.speed();
        self.energy -= basal_cost(&p, phys, mismatch)
            + movement_cost(&p, phys, speed)
            + speed_upkeep(&p, phys)
            + perception_cost(&p, phys);
        if mismatch.abs() > 1.0 {
            self.health -= phys.thermal_damage * (mismatch.abs() - 1.0);
        } else {
            self.health = (self.health + phys.health_recovery).min(1.0);
        }

        let acclimation = p.acclimation + phys.acclimation_rate * (temperature - p.temp_optimum - p.acclimation);
        set_acquired(&mut self.genome, schema, ACCLIMATION, acclimation);
        let experience = if decision.threatened {
            p.experience + phys.learning_rate * (1.0 - p.experience)
        } else {
            p.experience - phys.forgetting_rate * p.experience
        };
        set_acquired(&mut self.genome, schema, EXPERIENCE, experience);

        self.age = self.age.saturating_add(1);
        self.refractory = self.refractory.saturating_sub(1);
        self.handling = self.handling.saturating_sub(1);
    }
}

/// Writes an acquired (non-heritable) trait, clamped to its gene range.
fn set_acquired(genome: &mut Genome, schema: &GenomeSchema, index: usize, value: f32) {
    if let Some(spec) = schema.gene(index) {
        // `normalise` clamps into [0, 1] (NaN → 0) and genome lengths are
        // checked on creation and restore, so this cannot fail.
        let result = genome.set_normalised(index, spec.normalise(value));
        debug_assert!(result.is_ok(), "acquired trait write failed: {result:?}");
    }
}

/// Energy storage capacity `s · M`.
pub fn storage(p: &Phenotype, phys: &PhysiologyConfig) -> f32 {
    phys.storage * p.size
}

/// Maximum food intake per tick `i₀ · M^0.75 / η`.
pub fn intake_capacity(p: &Phenotype, phys: &PhysiologyConfig) -> f32 {
    phys.intake_rate * p.size.powf(0.75) / p.efficiency
}

/// Normalised thermal mismatch `(T − (T_opt + acclimation)) / σ`.
pub fn thermal_mismatch(p: &Phenotype, temperature: f32) -> f32 {
    (temperature - p.effective_optimum()) / p.temp_tolerance
}

/// Basal metabolism including the generalist cost and thermal stress.
pub fn basal_cost(p: &Phenotype, phys: &PhysiologyConfig, mismatch: f32) -> f32 {
    let generalist = 1.0 + phys.tolerance_cost * (p.temp_tolerance - 1.0);
    let stress = 1.0 + phys.thermal_stress_cost * (mismatch * mismatch).min(9.0);
    phys.basal_cost * p.size.powf(0.75) / p.efficiency * generalist * stress
}

/// Movement cost `c_m · M · v²`.
pub fn movement_cost(p: &Phenotype, phys: &PhysiologyConfig, speed: f32) -> f32 {
    phys.movement_cost * p.size * speed * speed
}

/// Upkeep of locomotor capacity `c_u · M · v_max²`.
pub fn speed_upkeep(p: &Phenotype, phys: &PhysiologyConfig) -> f32 {
    phys.speed_upkeep * p.size * p.speed * p.speed
}

/// Perception cost `c_p · radius`.
pub fn perception_cost(p: &Phenotype, phys: &PhysiologyConfig) -> f32 {
    phys.perception_cost * p.perception
}

/// Gompertz senescence hazard per tick at `age`, capped at 1.
pub fn senescence_hazard(age: u32, p: &Phenotype, phys: &PhysiologyConfig) -> f64 {
    let scale = f64::from(phys.longevity_ratio) * f64::from(p.maturity.max(1));
    (f64::from(phys.senescence_onset) * (1000f64.ln() * f64::from(age) / scale).exp()).min(1.0)
}

/// Contact distance at which a predator can strike.
pub fn capture_radius(p: &Phenotype) -> f32 {
    0.5 + 0.25 * p.size
}

/// Kill probability of one attack: `p₀ · min(1, 2 M_pred / (M_pred + M_prey))`
/// (large prey find a size refuge).
pub fn kill_probability(predator: &Phenotype, prey: &Phenotype, base: f32) -> f64 {
    let ratio = 2.0 * predator.size / (predator.size + prey.size);
    f64::from(base * ratio.min(1.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genes::{self, SIZE, SPEED, TEMP_OPTIMUM};

    fn pheno(overrides: &[(usize, f32)]) -> Phenotype {
        let schema = genes::schema(0.05).unwrap();
        let mut g = Genome::defaults(&schema);
        for &(i, v) in overrides {
            g.set_normalised(i, schema.genes()[i].normalise(v)).unwrap();
        }
        Phenotype::express(&g, &schema)
    }

    #[test]
    fn costs_follow_the_declared_scaling_laws() {
        let phys = PhysiologyConfig::default();
        let small = pheno(&[(SIZE, 1.0)]);
        let big = pheno(&[(SIZE, 2.0)]);
        let ratio = basal_cost(&big, &phys, 0.0) / basal_cost(&small, &phys, 0.0);
        assert!((ratio - 2f32.powf(0.75)).abs() < 1e-4, "Kleiber scaling: {ratio}");
        assert!((movement_cost(&big, &phys, 1.0) / movement_cost(&small, &phys, 1.0) - 2.0).abs() < 1e-5);
        assert!((movement_cost(&small, &phys, 1.0) / movement_cost(&small, &phys, 0.5) - 4.0).abs() < 1e-5);
        assert!((storage(&big, &phys) / storage(&small, &phys) - 2.0).abs() < 1e-5);
        let fast = pheno(&[(SPEED, 1.4)]);
        assert_eq!(basal_cost(&fast, &phys, 0.0), basal_cost(&small, &phys, 0.0));
        assert!((speed_upkeep(&fast, &phys) / speed_upkeep(&pheno(&[(SPEED, 0.7)]), &phys) - 4.0).abs() < 1e-4);
        assert!(basal_cost(&small, &phys, 2.0) > basal_cost(&small, &phys, 1.0));
        assert_eq!(basal_cost(&small, &phys, 3.0), basal_cost(&small, &phys, 30.0), "stress multiplier is capped");
    }

    #[test]
    fn thermal_mismatch_uses_acclimation_and_tolerance() {
        let p = pheno(&[(TEMP_OPTIMUM, 20.0)]);
        assert!((thermal_mismatch(&p, 30.0) - 2.0).abs() < 1e-5, "(30 − 20) / 5");
        let mut acclimated = p;
        acclimated.acclimation = 4.0;
        assert!((thermal_mismatch(&acclimated, 30.0) - 1.2).abs() < 1e-5, "(30 − 24) / 5");
    }

    #[test]
    fn senescence_is_gompertz_in_units_of_maturity() {
        let phys = PhysiologyConfig::default();
        let p = pheno(&[]);
        assert!((senescence_hazard(0, &p, &phys) - 1e-4).abs() < 1e-9);
        let at_l = senescence_hazard(400, &p, &phys);
        assert!((at_l - 0.1).abs() < 1e-6, "{at_l}");
        assert_eq!(senescence_hazard(10_000, &p, &phys), 1.0);
    }

    #[test]
    fn large_prey_have_a_size_refuge() {
        let pred = pheno(&[(SIZE, 1.0)]);
        assert!((kill_probability(&pred, &pheno(&[(SIZE, 1.0)]), 0.5) - 0.5).abs() < 1e-6);
        assert!(kill_probability(&pred, &pheno(&[(SIZE, 3.0)]), 0.5) < 0.26);
        assert!((kill_probability(&pred, &pheno(&[(SIZE, 0.5)]), 0.5) - 0.5).abs() < 1e-6);
    }
}
