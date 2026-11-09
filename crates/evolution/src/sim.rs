//! [`EvolutionSim`]: the evolution laboratory as a [`Simulation`].

use simulation_core::{
    ApplyPolicy, EventSeverity, MetricSpec, Metrics, ModelDescriptor, ModelFamily, ParamSet, ParamValue, SimError,
    SimEvent, Simulation, Snapshot,
};

use crate::commands::EvolutionCommand;
use crate::config::EvolutionConfig;
use crate::error::EvolutionError;
use crate::genes::{ACCLIMATION, EXPERIENCE, GENES, HERITABLE_COUNT, PERCEPTION, SIZE, SPEED};
use crate::params;
use crate::scenarios::Scenario;
use crate::speciation::SpeciationStatus;
use crate::view::{EvolutionView, EvolutionViewRequest};
use crate::world::{DeathCause, EvolutionWorld, WorldState};

/// Model identifier.
pub const MODEL_ID: &str = "evolution";
/// Snapshot schema version.
pub const SCHEMA_VERSION: u32 = 1;

const SUMMARY: &str = "Prey (and optional predators) with heritable genomes live in a 2D world of regrowing food, \
a temperature field and switchable barriers. Selection is never computed: it emerges from who survives and \
breeds. Tracks gene statistics, drift, divergence between islands, genealogy and an explicit speciation criterion.";

const RULES: &str = r#"### Evolution Laboratory

**World.** `W × H` cells (one world unit each), walled edges. Barriers paint water/mountain (impassable) or land
onto a land world in list order and can be switched on and off. Regions = connected land components (islands)
or a grid of sectors.

**Food** (land only): capacity `K` = background or richest patch × scale. Per tick, with `F₀ = max(F, s·K)`:
`F₁ = K F₀ / (F₀ + (K − F₀) e^{−r})` (exact logistic step), `F ← F + F₁ − F₀`.

**Temperature:** `T = T₀ + G_x (x/W − ½) + G_y (y/H − ½) + Δ + A sin(2π t/P)`, `Δ += shift_rate/1000` per tick.

**Genome.** 13 heritable genes (normalised to [0, 1], expressed linearly): speed, perception, size, efficiency,
thermal optimum and tolerance, clutch, effort, maturity, boldness, choosiness, two neutral markers. Two acquired
slots (acclimation, experience) are non-heritable: offspring start at 0, they never mutate and are excluded from
genetic distance.

**Energy per tick** (size `M`, efficiency `η`, tolerance `σ`, top speed `v_max`, mismatch
`m = (T − T_opt − acclimation)/σ`): basal `b₀ M^0.75/η · (1 + c_σ(σ − 1)) · (1 + c_T min(m², 9))` + movement
`c_m M v²` (speed used) + speed upkeep `c_u M v_max²` + perception `c_p r`. Storage `E_max = s M`. Intake ≤
`i₀ M^0.75/η` food per tick. Health `−= d(|m| − 1)` if `|m| > 1`, else recovers. Acclimation moves towards
`T − T_opt` (rate `a`, ±4 °C); experience `+= λ(1 − e)` while escaping, `−= φ e` otherwise.

**Behaviour** (decided in parallel from read-only state; acceleration ≤ `agility × v_max`). Prey: flee the nearest
predator in sight within `min(r, r(1 − boldness + ½ experience))` at full speed; else court (sexual, ready) the nearest ready
mutually acceptable mate in sight; else graze if the cell holds ≥ half a bite; else seek the best of 8 marched
rays (`food/(1 + 0.15 d)`); else wander. Searching, grazing and courting use `min(v_max, cruise_speed)`; full
speed only to flee or hunt. Predators: hunt (pursue the nearest visible prey) while energy
< `(0.3 + 0.6 boldness) E_max` and not handling a kill; a hungry predator with `n` rivals within 3 units loses the
tick with probability `1 − e^{−c n}` (interference, then idles); hungry predators with no prey in sight wander at
cruising speed; satiated ones court or rest at 60 % of cruising speed.

**Interactions.** Predation: a hunting predator within `0.5 + 0.25 M` of its target strikes; one predator per prey
(closest, then lowest id); kill probability `p₀ min(1, 2M_pred/(M_pred + M_prey))`; gain
`α (ε_body M_prey + E_prey)` (stored up to `E_max`), then `handling_ticks` without hunting. Feeding: non-fleeing prey claim
`min(intake, room)` from their cell; shared proportionally.

**Reproduction.** Ready = mature, rested, health ≥ ½, `E ≥ θ E_max`. Asexual: every ready organism breeds. Sexual:
courting organisms in contact (`≤ mate_radius`) with a ready partner that accepts them — condition
`≥ θ + choosiness(1 − θ)` and genetic distance `≤ mate_distance_limit` — pair one-to-one, closest pair first. Each
parent spends `effort × E`; the clutch (`n` = clutch gene, or rounded mean) shares `birth_efficiency × Σ`
equally; a newborn stores at most `s M` (the excess is lost). Offspring genome: clone (asexual) or crossover
(sexual, uniform or one-point over the heritable genes only), then per-gene mutation with probability `μ` by
`N(0, size)`, reflected or clamped. Non-evolving predators clone their first parent exactly.

**Mortality.** Starvation (`E ≤ 0`), thermal (`health ≤ 0`), predation, background `μ_b`, Gompertz senescence
`h₀ exp(ln 1000 · age / (L · maturity))`. Under the *constant predation* treatment predators neither breed nor die.

**Selection is not computed anywhere**: there is no fitness function; gene frequencies change only through who
survives and breeds (and through drift and mutation).

**Speciation (explicit observable criterion).** Applies only with sexual reproduction and `mate_distance_limit < 1`.
Prey are split by deterministic 2-means; both clusters ≥ `max(5, f N)`; mean cross-cluster genetic distance
> `mate_distance_limit`; persisting while the mean generation advances by `speciation_generations`.
"#;

/// The evolution laboratory as a [`Simulation`].
#[derive(Clone, Debug)]
pub struct EvolutionSim {
    scenario_id: String,
    scenario_name: String,
    /// Configuration used by the next reset (includes changes to parameters
    /// that require a reset); the running world keeps its own copy.
    config: EvolutionConfig,
    world: EvolutionWorld,
}

impl EvolutionSim {
    /// Builds an experiment from a configuration and a seed.
    pub fn new(config: EvolutionConfig, seed: u64) -> Result<Self, EvolutionError> {
        let world = EvolutionWorld::new(config.clone(), seed)?;
        Ok(Self { scenario_id: "custom".into(), scenario_name: "Custom experiment".into(), config, world })
    }

    /// Builds the initial state of a scenario.
    pub fn from_scenario(scenario: &Scenario) -> Result<Self, EvolutionError> {
        let mut sim = Self::new(scenario.config.clone(), scenario.seed)?;
        sim.scenario_id.clone_from(&scenario.id);
        sim.scenario_name.clone_from(&scenario.name);
        Ok(sim)
    }

    /// Identifier of the scenario the experiment was built from.
    pub fn scenario_id(&self) -> &str {
        &self.scenario_id
    }

    /// Display name of the scenario.
    pub fn scenario_name(&self) -> &str {
        &self.scenario_name
    }

    /// The running world (organisms, environment, statistics, genealogy).
    pub fn world(&self) -> &EvolutionWorld {
        &self.world
    }

    /// Configuration that the next reset will use.
    pub fn pending_config(&self) -> &EvolutionConfig {
        &self.config
    }
}

/// Metric declarations; [`Simulation::metrics`] reports them in this order.
pub fn metric_specs() -> Vec<MetricSpec> {
    let mut specs = vec![
        MetricSpec::gauge("prey", "Prey", "individuals", "Living prey."),
        MetricSpec::gauge("predators", "Predators", "individuals", "Living predators."),
        MetricSpec::counter("births", "Births", "Organisms born in the last tick."),
        MetricSpec::counter("deaths", "Deaths", "Organisms that died in the last tick (all causes)."),
    ];
    for cause in DeathCause::ALL {
        let key = format!("deaths_{}", cause.name());
        let label = format!("Deaths: {}", cause.name());
        specs.push(MetricSpec::counter(&key, &label, "Deaths of this cause in the last tick."));
    }
    specs.extend([
        MetricSpec::counter("matings", "Matings", "Mating pairs formed in the last tick (sexual mode)."),
        MetricSpec::counter("mutations", "Mutations", "Mutated genes among the last tick's newborns."),
        MetricSpec::gauge("mean_generation", "Mean generation", "", "Mean generation number of living prey."),
        MetricSpec::gauge("max_generation", "Max generation", "", "Largest generation number among living prey."),
    ]);
    for g in &GENES[..HERITABLE_COUNT] {
        let mean = format!("mean_{}", g.name);
        specs.push(MetricSpec::gauge(&mean, &format!("Mean {}", g.label), g.unit, "Mean phenotype of living prey."));
        let sd = format!("sd_{}", g.name);
        specs.push(MetricSpec::gauge(&sd, &format!("SD {}", g.label), g.unit, "Standard deviation among prey."));
    }
    specs.extend([
        MetricSpec::gauge("mean_acclimation", "Mean acclimation", "°C", "Acquired (non-heritable) thermal shift."),
        MetricSpec::gauge("mean_experience", "Mean experience", "", "Acquired (non-heritable) predator wariness."),
        MetricSpec::gauge("predator_mean_speed", "Predator speed", "units/tick", "Mean predator speed gene."),
        MetricSpec::gauge("predator_mean_perception", "Predator perception", "units", "Mean predator perception."),
        MetricSpec::gauge("predator_mean_size", "Predator size", "mass", "Mean predator body size."),
        MetricSpec::gauge("genetic_diversity", "Genetic diversity", "", "Mean pairwise genetic distance among prey."),
        MetricSpec::ratio("heterozygosity", "Heterozygosity", "Mean expected heterozygosity of gene classes."),
        MetricSpec::ratio("fst", "F_ST", "Fixation index of gene classes between prey regions."),
        MetricSpec::gauge(
            "region_divergence",
            "Region divergence",
            "",
            "Largest net genetic distance between regions.",
        ),
        MetricSpec::gauge(
            "regions_populated",
            "Populated regions",
            "regions",
            "Regions holding at least one organism.",
        ),
        MetricSpec::gauge("lineages", "Surviving lineages", "founders", "Founders with living descendants."),
        MetricSpec::gauge("genealogy_nodes", "Genealogy nodes", "nodes", "Recorded (unpruned) genealogy nodes."),
        MetricSpec::gauge("food_total", "Food", "units", "Food on all cells."),
        MetricSpec::gauge("mean_temperature", "Mean temperature", "°C", "Mean temperature over land."),
        MetricSpec::gauge("mean_condition", "Prey condition", "", "Mean prey energy as a fraction of storage."),
        MetricSpec::gauge(
            "cluster_distance",
            "Cluster distance",
            "",
            "Mean genetic distance between the two prey clusters.",
        ),
        MetricSpec::gauge("speciation", "Speciation", "", "1 once the explicit speciation criterion has been met."),
    ]);
    specs
}

impl Simulation for EvolutionSim {
    type Command = EvolutionCommand;
    type ViewRequest = EvolutionViewRequest;
    type View = EvolutionView;

    fn descriptor(&self) -> ModelDescriptor {
        ModelDescriptor::new(MODEL_ID, "Evolution Laboratory", ModelFamily::AgentBased)
            .summary(SUMMARY)
            .rules(RULES)
            .params(params::specs())
            .metrics(metric_specs())
            .time("tick", 1.0)
    }

    fn tick(&self) -> u64 {
        self.world.tick()
    }

    fn step(&mut self) {
        self.world.step();
    }

    fn reset(&mut self) {
        match EvolutionWorld::new(self.config.clone(), self.world.seed()) {
            Ok(world) => self.world = world,
            Err(e) => {
                // Parameters are validated when set, so this only reports an
                // unexpected inconsistency instead of panicking.
                let message = format!("reset failed: {e}");
                self.world
                    .events
                    .push(SimEvent::new(self.world.tick(), "reset", message).severity(EventSeverity::Major));
            }
        }
    }

    fn apply(&mut self, command: EvolutionCommand) -> Result<(), SimError> {
        let shift = matches!(command, EvolutionCommand::SetTemperatureShift { .. });
        self.world.apply(command)?;
        if shift {
            self.config.climate.shift_rate = self.world.config.climate.shift_rate;
        }
        Ok(())
    }

    fn params(&self) -> ParamSet {
        params::specs().into_iter().filter_map(|s| params::get(&self.config, &s.key).ok().map(|v| (s.key, v))).collect()
    }

    fn set_param(&mut self, key: &str, value: ParamValue) -> Result<ApplyPolicy, SimError> {
        if !params::specs().iter().any(|s| s.key == key) {
            return Err(SimError::UnknownParameter(key.to_string()));
        }
        let mut candidate = self.config.clone();
        let policy = params::set(&mut candidate, key, &value)?;
        candidate.validate()?;
        if policy == ApplyPolicy::RequiresReset {
            EvolutionWorld::check_founders(&candidate)?;
        } else {
            self.world.reconfigure(key, &value)?;
        }
        self.config = candidate;
        Ok(policy)
    }

    fn metrics(&self) -> Metrics {
        let w = &self.world;
        let s = w.stats();
        let r = w.last_report();
        let mut m = Metrics::new(w.tick(), w.tick() as f64);
        m.push("prey", f64::from(s.prey.count))
            .push("predators", f64::from(s.predators.count))
            .push("births", f64::from(r.births()))
            .push("deaths", f64::from(r.total_deaths()));
        for cause in DeathCause::ALL {
            m.push(&format!("deaths_{}", cause.name()), f64::from(r.deaths[cause.index()]));
        }
        m.push("matings", f64::from(r.matings))
            .push("mutations", f64::from(r.mutations))
            .push("mean_generation", s.prey.mean_generation)
            .push("max_generation", f64::from(s.prey.max_generation));
        for (i, g) in GENES[..HERITABLE_COUNT].iter().enumerate() {
            m.push(&format!("mean_{}", g.name), s.prey.mean(i));
            m.push(&format!("sd_{}", g.name), s.prey.sd(i));
        }
        let speciated = matches!(w.speciation().status(), SpeciationStatus::Speciated(_));
        m.push("mean_acclimation", s.prey.mean(ACCLIMATION))
            .push("mean_experience", s.prey.mean(EXPERIENCE))
            .push("predator_mean_speed", s.predators.mean(SPEED))
            .push("predator_mean_perception", s.predators.mean(PERCEPTION))
            .push("predator_mean_size", s.predators.mean(SIZE))
            .push("genetic_diversity", s.genetic_diversity)
            .push("heterozygosity", s.heterozygosity)
            .push("fst", s.fst)
            .push("region_divergence", s.max_net_divergence())
            .push("regions_populated", s.regions.len() as f64)
            .push("lineages", s.lineages.len() as f64)
            .push("genealogy_nodes", w.genealogy().len() as f64)
            .push("food_total", w.environment().total_food())
            .push("mean_temperature", w.environment().mean_land_temperature(&w.config().climate, w.tick()))
            .push("mean_condition", s.prey.mean_condition)
            .push("cluster_distance", w.speciation().last_split().map_or(0.0, |c| c.between))
            .push("speciation", if speciated || !w.speciation().history().is_empty() { 1.0 } else { 0.0 });
        m
    }

    fn view(&self, request: &EvolutionViewRequest) -> EvolutionView {
        self.world.view(request)
    }

    fn snapshot(&self) -> Result<Snapshot, SimError> {
        Snapshot::encode(MODEL_ID, SCHEMA_VERSION, self.world.tick(), &self.world.state())
    }

    fn restore(&mut self, snapshot: &Snapshot) -> Result<(), SimError> {
        let state: WorldState = snapshot.decode(MODEL_ID, SCHEMA_VERSION)?;
        let world = EvolutionWorld::from_state(state)?;
        self.config = world.config().clone();
        self.world = world;
        Ok(())
    }

    fn memory_bytes(&self) -> usize {
        let w = &self.world;
        let organism = std::mem::size_of::<crate::organism::Organism>() + crate::genes::GENE_COUNT * 4;
        let cells = w.environment().width() * w.environment().height();
        w.organisms().len() * (organism + 8) + w.genealogy().len() * 96 + cells * (4 + 4 + 2 + 1)
    }

    /// Finished at `max_ticks`, or once nothing can change any more: no
    /// organism is left, or the prey are extinct and the only survivors are
    /// predators under the constant regime (which never breed or die).
    fn is_finished(&self) -> bool {
        let w = &self.world;
        let counts = (w.stats().prey.count, w.stats().predators.count);
        let frozen = counts.0 == 0 && (counts.1 == 0 || w.config().predators.constant);
        frozen || (w.config().max_ticks > 0 && w.tick() >= w.config().max_ticks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small() -> EvolutionConfig {
        let mut c = EvolutionConfig::default();
        c.world.width = 40;
        c.world.height = 30;
        c.prey.initial = 50;
        c
    }

    #[test]
    fn metrics_follow_the_declared_specs() {
        let mut sim = EvolutionSim::new(small(), 1).unwrap();
        sim.step();
        let keys: Vec<String> = sim.metrics().keys().map(str::to_string).collect();
        let declared: Vec<String> = sim.descriptor().metrics.into_iter().map(|m| m.key).collect();
        assert_eq!(keys, declared);
    }

    #[test]
    fn parameters_report_their_policy_and_reset_applies_them() {
        let mut sim = EvolutionSim::new(small(), 2).unwrap();
        assert_eq!(sim.set_param("width", ParamValue::Int(32)), Ok(ApplyPolicy::RequiresReset));
        assert_eq!(sim.world().environment().width(), 40, "a reset parameter waits for the reset");
        assert_eq!(sim.params().i64_or("width", 0), 32);
        assert_eq!(sim.set_param("mutation_size", ParamValue::Float(0.1)), Ok(ApplyPolicy::NextTick));
        assert_eq!(sim.world().schema().genes()[0].mutation_sd, 0.1);
        assert!(matches!(sim.set_param("bogus", ParamValue::Int(1)), Err(SimError::UnknownParameter(_))));
        assert!(sim.set_param("mutation_rate", ParamValue::Float(2.0)).is_err());
        sim.reset();
        assert_eq!(sim.world().environment().width(), 32);
        assert_eq!(sim.tick(), 0);
    }

    #[test]
    fn snapshot_round_trip_and_rejection() {
        let mut sim = EvolutionSim::new(small(), 3).unwrap();
        sim.advance(20);
        let snap = sim.snapshot().unwrap();
        let mut other = EvolutionSim::new(small(), 99).unwrap();
        other.restore(&snap).unwrap();
        assert_eq!(other.snapshot().unwrap(), snap);
        let wrong = Snapshot::encode("other.model", SCHEMA_VERSION, 0, &1u8).unwrap();
        assert!(matches!(sim.restore(&wrong), Err(SimError::WrongModel { .. })));
        let mut corrupt = snap.clone();
        corrupt.payload.truncate(corrupt.payload.len() / 2);
        assert!(sim.restore(&corrupt).is_err());
    }

    #[test]
    fn speciation_status_follows_its_preconditions_at_once() {
        let mut c = small();
        c.reproduction.mode = crate::ReproductionMode::Sexual;
        c.reproduction.mate_distance_limit = 0.3;
        let mut sim = EvolutionSim::new(c, 6).unwrap();
        assert_eq!(sim.world().speciation().status(), &SpeciationStatus::Monitoring);
        sim.set_param("speciation_enabled", ParamValue::Bool(false)).unwrap();
        assert!(matches!(sim.world().speciation().status(), SpeciationStatus::NotApplicable(_)));
        sim.set_param("speciation_enabled", ParamValue::Bool(true)).unwrap();
        assert_eq!(sim.world().speciation().status(), &SpeciationStatus::Monitoring);
        sim.set_param("mate_distance_limit", ParamValue::Float(1.0)).unwrap();
        assert!(matches!(sim.world().speciation().status(), SpeciationStatus::NotApplicable(_)));
        sim.set_param("mate_distance_limit", ParamValue::Float(0.3)).unwrap();
        sim.set_param("reproduction_mode", ParamValue::Text("asexual".into())).unwrap();
        assert!(matches!(sim.world().speciation().status(), SpeciationStatus::NotApplicable(_)));
    }

    #[test]
    fn a_world_of_constant_predators_without_prey_is_finished() {
        let mut c = small();
        c.predators.initial = 5;
        c.predators.constant = true;
        let mut sim = EvolutionSim::new(c, 4).unwrap();
        assert!(!sim.is_finished());
        sim.apply(EvolutionCommand::Cull { fraction: 1.0, species: Some(crate::Species::Prey), area: None }).unwrap();
        assert!(sim.is_finished(), "immortal, sterile predators alone can never change the world");
        assert_eq!(sim.set_param("predators_constant", ParamValue::Bool(false)), Ok(ApplyPolicy::NextTick));
        assert!(!sim.is_finished(), "dynamic predators still starve");
        sim.set_param("max_ticks", ParamValue::Int(1)).unwrap();
        sim.apply(EvolutionCommand::Spawn {
            species: crate::Species::Prey,
            count: 3,
            area: None,
            traits: vec![],
            spread: 0.0,
        })
        .unwrap();
        assert!(!sim.is_finished());
        sim.step();
        assert!(sim.is_finished(), "max_ticks reached");
    }
}
