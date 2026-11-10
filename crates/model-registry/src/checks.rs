//! Validation: determinism, snapshot round trips and the quick checks that
//! the model crates make possible through their public APIs.
//!
//! * **Determinism** — two simulations built from the same scenario and
//!   seed must report bit-identical metrics after every tick.
//! * **Snapshot round trip** — a snapshot encoded to bytes and decoded must
//!   be identical; restoring it (into the same instance and into a fresh
//!   instance built with another seed) must replay the same metrics. The
//!   metrics of Hashlife's node store ([`REPRESENTATION_METRICS`]: node
//!   count, arena bytes, root level) describe the memoisation cache, which a
//!   snapshot does not contain, and are left out of this comparison only.
//! * **Model-specific checks** — Life-like rules: every engine that supports
//!   the world (dense, bit-packed, sparse, Hashlife) evolves the scenario's
//!   cells to the same cells as the model; Hashlife: the still lifes,
//!   oscillators and spaceships of the `patterns` library show their
//!   catalogued period and displacement; Flow-Lenia: mass is conserved per
//!   step (relative change below 1e-6, the bound of the continuous crate's
//!   own tests); ecosystem: the energy and nutrient books balance (relative
//!   error below 1e-5, the bound of the ecosystem crate's own tests);
//!   metapixel: the macro states read from the exact B3/S23 pattern after
//!   two metagenerations equal the accelerated macro model's prediction.

use cellular_automata::{EngineKind, create_engine};
use continuous::ModelKind;
use serde::Serialize;
use simulation_core::{DynSimulation, Metrics, Snapshot};

use crate::build::{continuous_sim, ecosystem_sim, hashlife_sim, life_sim, metapixel_sim};
use crate::error::RegistryError;
use crate::registry::{BuildRequest, Registry};
use crate::scenario::{Scenario, ScenarioData};

/// Largest relative mass change per Flow-Lenia step.
pub const FLOW_MASS_TOLERANCE: f64 = 1e-6;
/// Largest relative balance error of the ecosystem's books.
pub const LEDGER_TOLERANCE: f64 = 1e-5;
/// Metrics of a Hashlife node store (cache size and layout, not model
/// state): excluded from snapshot replay comparisons.
pub const REPRESENTATION_METRICS: [&str; 3] = ["node_count", "memory_bytes", "root_level"];

/// Outcome of one check.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    /// The property holds.
    Passed,
    /// The property does not hold.
    Failed,
    /// The check does not apply (explained in the detail).
    Skipped,
}

/// One check and its outcome.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CheckResult {
    /// Check name.
    pub name: String,
    /// Outcome.
    pub status: CheckStatus,
    /// What was compared, or the first difference found.
    pub detail: String,
}

impl CheckResult {
    fn new(name: &str, status: CheckStatus, detail: String) -> Self {
        Self { name: name.to_string(), status, detail }
    }

    fn outcome(name: &str, failure: Option<String>, success: String) -> Self {
        match failure {
            Some(detail) => Self::new(name, CheckStatus::Failed, detail),
            None => Self::new(name, CheckStatus::Passed, success),
        }
    }
}

/// Every check run for one model.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ValidationReport {
    /// Model id.
    pub model_id: String,
    /// Qualified id of the scenario used.
    pub scenario_id: String,
    /// Seed used.
    pub seed: u64,
    /// Ticks per phase.
    pub ticks: u64,
    /// Results.
    pub checks: Vec<CheckResult>,
}

impl ValidationReport {
    /// Whether no check failed.
    pub fn passed(&self) -> bool {
        self.checks.iter().all(|c| c.status != CheckStatus::Failed)
    }
}

/// Runs every check on `target` (a model id or scenario id) with `ticks`
/// ticks per phase.
///
/// # Errors
/// Errors building the simulation; failing checks are reported in the
/// [`ValidationReport`], not as errors.
pub fn validate(
    registry: &Registry,
    target: &str,
    seed: Option<u64>,
    ticks: u64,
) -> Result<ValidationReport, RegistryError> {
    let scenario = registry.resolve(target)?;
    let seed = seed.unwrap_or(scenario.info.seed);
    let request = BuildRequest { target: target.to_string(), seed: Some(seed), params: Vec::new() };
    let mut checks = vec![determinism(registry, &request, ticks)?, snapshot_round_trip(registry, &request, ticks)?];
    checks.extend(quick_checks(registry, &scenario, seed, ticks)?);
    Ok(ValidationReport {
        model_id: scenario.info.model_id.clone(),
        scenario_id: scenario.info.id.clone(),
        seed,
        ticks,
        checks,
    })
}

/// Metrics after each of up to `ticks` single-tick advances (stops early
/// when the model stops advancing).
fn trace(sim: &mut dyn DynSimulation, ticks: u64) -> Vec<Metrics> {
    let mut out = Vec::new();
    for _ in 0..ticks {
        if sim.is_finished() || sim.advance(1) == 0 {
            break;
        }
        out.push(sim.metrics());
    }
    out
}

/// Removes [`REPRESENTATION_METRICS`] from a trace.
fn state_metrics(trace: Vec<Metrics>) -> Vec<Metrics> {
    trace.into_iter().map(without_representation).collect()
}

fn without_representation(mut m: Metrics) -> Metrics {
    m.values.retain(|(k, _)| !REPRESENTATION_METRICS.contains(&k.as_str()));
    m
}

/// First difference between two measurements (bitwise comparison).
fn metric_difference(a: &Metrics, b: &Metrics) -> Option<String> {
    if a.tick != b.tick {
        return Some(format!("ticks differ: {} vs {}", a.tick, b.tick));
    }
    if a.values.len() != b.values.len() {
        return Some(format!("tick {}: {} vs {} metric values", a.tick, a.values.len(), b.values.len()));
    }
    a.values.iter().zip(&b.values).find_map(|((ka, va), (kb, vb))| {
        (ka != kb || va.to_bits() != vb.to_bits()).then(|| format!("tick {}: `{ka}` = {va} vs `{kb}` = {vb}", a.tick))
    })
}

fn trace_difference(a: &[Metrics], b: &[Metrics]) -> Option<String> {
    if a.len() != b.len() {
        return Some(format!("one run advanced {} ticks, the other {}", a.len(), b.len()));
    }
    a.iter().zip(b).find_map(|(x, y)| metric_difference(x, y))
}

fn values(trace: &[Metrics]) -> usize {
    trace.iter().map(|m| m.values.len()).sum()
}

fn not_deterministic(name: &str, sim: &dyn DynSimulation) -> Option<CheckResult> {
    (!sim.descriptor().deterministic).then(|| {
        CheckResult::new(name, CheckStatus::Skipped, "the model declares that its runs are not reproducible".into())
    })
}

/// Two runs from the same seed report identical metrics.
fn determinism(registry: &Registry, request: &BuildRequest, ticks: u64) -> Result<CheckResult, RegistryError> {
    const NAME: &str = "determinism";
    let mut a = registry.build(request)?.sim;
    if let Some(skip) = not_deterministic(NAME, a.as_ref()) {
        return Ok(skip);
    }
    let mut b = registry.build(request)?.sim;
    if let Some(d) = metric_difference(&a.metrics(), &b.metrics()) {
        return Ok(CheckResult::new(NAME, CheckStatus::Failed, format!("initial state differs: {d}")));
    }
    let (ta, tb) = (trace(a.as_mut(), ticks), trace(b.as_mut(), ticks));
    Ok(CheckResult::outcome(
        NAME,
        trace_difference(&ta, &tb),
        format!("two runs, {} ticks, {} metric values identical", ta.len(), values(&ta)),
    ))
}

/// Snapshot → bytes → snapshot is lossless and restoring replays the run.
/// Snapshot, encoding and restore errors are check failures.
fn snapshot_round_trip(registry: &Registry, request: &BuildRequest, ticks: u64) -> Result<CheckResult, RegistryError> {
    const NAME: &str = "snapshot round trip";
    let mut sim = registry.build(request)?.sim;
    if let Some(skip) = not_deterministic(NAME, sim.as_ref()) {
        return Ok(skip);
    }
    let fresh = registry.build(&BuildRequest { seed: Some(request.seed.unwrap_or(0) ^ 1), ..request.clone() })?.sim;
    Ok(match replay_from_snapshot(sim.as_mut(), fresh, ticks) {
        Ok(detail) => CheckResult::new(NAME, CheckStatus::Passed, detail),
        Err(detail) => CheckResult::new(NAME, CheckStatus::Failed, detail),
    })
}

/// Runs `ticks / 2` ticks, snapshots, runs on, then restores the snapshot
/// into `sim` and into `fresh` (built with another seed) and replays.
/// Returns a description of what matched, or of the first problem.
fn replay_from_snapshot(
    sim: &mut dyn DynSimulation,
    mut fresh: Box<dyn DynSimulation>,
    ticks: u64,
) -> Result<String, String> {
    let half = (ticks / 2).max(1);
    trace(sim, half);
    let snapshot = sim.snapshot().map_err(|e| format!("snapshot failed: {e}"))?;
    let at_snapshot = without_representation(sim.metrics());
    let excluded: Vec<&str> = REPRESENTATION_METRICS.into_iter().filter(|k| sim.metrics().get(k).is_some()).collect();
    let bytes = snapshot.to_bytes().map_err(|e| format!("encoding the snapshot failed: {e}"))?;
    let decoded = Snapshot::from_bytes(&bytes).map_err(|e| format!("decoding the snapshot failed: {e}"))?;
    if decoded != snapshot {
        return Err("the decoded snapshot differs from the original".into());
    }
    let original = state_metrics(trace(sim, half));

    sim.restore(&decoded).map_err(|e| format!("restore at tick {} failed: {e}", snapshot.tick))?;
    if let Some(d) = metric_difference(&at_snapshot, &without_representation(sim.metrics())) {
        return Err(format!("after restore: {d}"));
    }
    if let Some(d) = trace_difference(&original, &state_metrics(trace(sim, half))) {
        return Err(format!("replay: {d}"));
    }
    fresh.restore(&decoded).map_err(|e| format!("restore into a fresh instance failed: {e}"))?;
    if let Some(d) = trace_difference(&original, &state_metrics(trace(fresh.as_mut(), half))) {
        return Err(format!("fresh instance replay: {d}"));
    }
    let mut detail = format!(
        "snapshot at tick {} ({} bytes) restored into the same and a fresh instance; {} ticks replayed identically",
        snapshot.tick,
        bytes.len(),
        original.len()
    );
    if !excluded.is_empty() {
        detail.push_str(&format!(" (cache metrics {} not compared)", excluded.join(", ")));
    }
    Ok(detail)
}

/// Model-specific checks for the scenario's model.
fn quick_checks(
    registry: &Registry,
    scenario: &Scenario,
    seed: u64,
    ticks: u64,
) -> Result<Vec<CheckResult>, RegistryError> {
    Ok(match &scenario.data {
        ScenarioData::Ca(_) => match life_sim(registry.assets(), scenario, seed)? {
            Some(sim) => vec![engine_agreement(sim, ticks)],
            None => Vec::new(),
        },
        ScenarioData::Hashlife(_) => vec![hashlife_patterns(scenario, seed)?],
        ScenarioData::Continuous(_) => {
            let sim = continuous_sim(scenario, seed)?;
            if sim.kind() == ModelKind::FlowLenia { vec![flow_mass(sim, ticks)] } else { Vec::new() }
        }
        ScenarioData::Ecosystem(_) => vec![ledger_balance(ecosystem_sim(registry.assets(), scenario, seed)?, ticks)],
        ScenarioData::Metapixel(_) => vec![metapixel_readout(metapixel_sim(registry.assets(), scenario, seed)?)],
        _ => Vec::new(),
    })
}

fn sorted(mut cells: Vec<(i64, i64)>) -> Vec<(i64, i64)> {
    cells.sort_unstable();
    cells
}

/// Every engine able to run the world reaches the model's cells.
fn engine_agreement(mut sim: cellular_automata::LifeSim, ticks: u64) -> CheckResult {
    use simulation_core::Simulation;
    const NAME: &str = "engine agreement";
    let (world, rule) = (sim.world(), sim.rule());
    let initial = sim.live_cells();
    let bbox = sim.engine().bounding_box();
    let start = sim.tick();
    sim.advance(ticks.max(1));
    let generations = sim.tick() - start;
    if let Some(reason) = sim.halt_reason() {
        return CheckResult::new(NAME, CheckStatus::Failed, format!("the model halted: {reason}"));
    }
    let expected = sorted(sim.live_cells());
    let mut compared = Vec::new();
    let mut skipped = Vec::new();
    for kind in EngineKind::ALL {
        if let Err(e) = kind.check(&world, rule) {
            skipped.push(format!("{}: {e}", kind.as_str()));
            continue;
        }
        let prepared = create_engine(kind, world, rule).and_then(|mut engine| {
            if let Some(rect) = bbox {
                engine.reserve(rect)?;
            }
            engine.load_cells(&initial);
            engine.advance(generations)?;
            Ok(engine)
        });
        let engine = match prepared {
            Ok(engine) => engine,
            Err(e) => {
                skipped.push(format!("{}: {e}", kind.as_str()));
                continue;
            }
        };
        let cells = sorted(engine.export_cells());
        if cells != expected {
            return CheckResult::new(
                NAME,
                CheckStatus::Failed,
                format!(
                    "after {generations} generations the {} engine has {} live cells, the model {}",
                    kind.as_str(),
                    cells.len(),
                    expected.len()
                ),
            );
        }
        compared.push(kind.as_str());
    }
    if compared.is_empty() {
        return CheckResult::new(NAME, CheckStatus::Skipped, format!("no engine applies ({})", skipped.join("; ")));
    }
    let mut detail = format!(
        "{} engine(s) [{}] match the model after {generations} generations ({} live cells)",
        compared.len(),
        compared.join(", "),
        expected.len()
    );
    if !skipped.is_empty() {
        detail.push_str(&format!("; not applicable: {}", skipped.join("; ")));
    }
    CheckResult::new(NAME, CheckStatus::Passed, detail)
}

/// Catalogued periods and displacements of the embedded patterns.
fn hashlife_patterns(scenario: &Scenario, seed: u64) -> Result<CheckResult, RegistryError> {
    use simulation_core::Simulation;
    const NAME: &str = "pattern periods";
    let mut checked = Vec::new();
    for entry in patterns::builtin_patterns() {
        let generations = match entry.kind {
            "still_life" => 1,
            "oscillator" | "spaceship" => entry.period,
            _ => continue,
        };
        let probe = crate::scenario::Scenario::new(
            &scenario.info.folder,
            ScenarioData::Hashlife(entry),
            scenario.info.source.clone(),
        );
        let mut sim = hashlife_sim(&probe, seed)?;
        let (dx, dy) = entry.displacement.unwrap_or((0, 0));
        let expected = sorted(sim.universe().cells().map(|(x, y)| (x + dx, y + dy)).collect());
        sim.advance(generations);
        let after = sorted(sim.universe().cells().collect());
        if after != expected {
            return Ok(CheckResult::new(
                NAME,
                CheckStatus::Failed,
                format!(
                    "`{}` does not repeat after {generations} generation(s) with displacement ({dx}, {dy})",
                    entry.id
                ),
            ));
        }
        checked.push(entry.id);
    }
    Ok(CheckResult::new(
        NAME,
        CheckStatus::Passed,
        format!(
            "{} still lifes, oscillators and spaceships repeat as catalogued: {}",
            checked.len(),
            checked.join(", ")
        ),
    ))
}

/// Flow-Lenia conserves mass at every step.
fn flow_mass(mut sim: continuous::ContinuousSim, ticks: u64) -> CheckResult {
    use simulation_core::Simulation;
    const NAME: &str = "mass conservation";
    let initial = sim.mass();
    let mut previous = initial;
    let mut worst = 0.0f64;
    for step in 0..ticks.max(1) {
        sim.step();
        let mass = sim.mass();
        let change = ((mass - previous) / previous.abs().max(f64::MIN_POSITIVE)).abs();
        if !change.is_finite() || change > FLOW_MASS_TOLERANCE {
            return CheckResult::new(
                NAME,
                CheckStatus::Failed,
                format!("step {}: relative mass change {change:e} exceeds {FLOW_MASS_TOLERANCE:e}", step + 1),
            );
        }
        worst = worst.max(change);
        previous = mass;
    }
    CheckResult::new(
        NAME,
        CheckStatus::Passed,
        format!(
            "{} steps, mass {initial:.6} → {previous:.6}, largest relative change per step {worst:.3e} (limit {FLOW_MASS_TOLERANCE:e})",
            ticks.max(1)
        ),
    )
}

/// The ecosystem's energy and nutrient ledgers balance against the stocks.
fn ledger_balance(mut sim: ecosystem::EcosystemSim, ticks: u64) -> CheckResult {
    use simulation_core::Simulation;
    const NAME: &str = "energy and nutrient books";
    if matches!(sim.config().rules, ecosystem::Rules::NetLogo(_)) {
        return CheckResult::new(
            NAME,
            CheckStatus::Skipped,
            "the scenario runs the NetLogo Wolf Sheep Predation rules, which the energy and nutrient books do not \
             cover"
                .into(),
        );
    }
    sim.advance(ticks.max(1));
    let stocks = sim.stocks();
    let energy = sim.ledgers().energy.relative_error(stocks.energy());
    let nutrients = sim.ledgers().nutrients.relative_error(stocks.nutrients());
    let detail = format!(
        "after {} ticks: energy error {energy:.3e}, nutrient error {nutrients:.3e} (limit {LEDGER_TOLERANCE:e})",
        sim.tick()
    );
    let ok = energy.is_finite() && nutrients.is_finite() && energy < LEDGER_TOLERANCE && nutrients < LEDGER_TOLERANCE;
    CheckResult::new(NAME, if ok { CheckStatus::Passed } else { CheckStatus::Failed }, detail)
}

/// Number of metagenerations read by [`metapixel_readout`].
const METAGENS_CHECKED: u64 = 2;

/// The exact macro readout agrees with the accelerated prediction.
fn metapixel_readout(mut sim: metapixel::MetapixelSim) -> CheckResult {
    use simulation_core::Simulation;
    const NAME: &str = "macro readout";
    sim.advance(METAGENS_CHECKED * metapixel::PERIOD);
    if let Some(reason) = sim.halt_reason() {
        return CheckResult::new(NAME, CheckStatus::Failed, format!("the model halted: {reason}"));
    }
    let records: Vec<&metapixel::ReadoutRecord> = sim.history().records().collect();
    if records.is_empty() {
        return CheckResult::new(
            NAME,
            CheckStatus::Failed,
            format!("no readout after {} generations", METAGENS_CHECKED * metapixel::PERIOD),
        );
    }
    if let Some(bad) = records.iter().find(|r| !r.matches_prediction()) {
        return CheckResult::new(
            NAME,
            CheckStatus::Failed,
            format!(
                "metagen {}: {} tile(s) differ from the accelerated prediction ({} ambiguous)",
                bad.metagen,
                bad.mismatches.len(),
                bad.ambiguous.len()
            ),
        );
    }
    let tiles: usize = records.iter().map(|r| r.accelerated.len()).sum();
    CheckResult::new(
        NAME,
        CheckStatus::Passed,
        format!(
            "{} readout(s) up to generation {}: all {tiles} tile readings match the accelerated prediction",
            records.len(),
            sim.tick()
        ),
    )
}
