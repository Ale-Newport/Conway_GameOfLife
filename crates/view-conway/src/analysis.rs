//! Period analysis: does the world (or the selection on its own) return to
//! its starting state, and after how many generations and with which
//! displacement? The model does not offer this, so it runs here on a copy
//! of the model (restored from a snapshot, or the selection's cells on an
//! empty infinite plane) in a background thread; the live run is never
//! touched.
//!
//! On the infinite plane the comparison ignores translation (spaceships are
//! detected with their displacement); on finite worlds the whole state must
//! repeat exactly. Only a return to the *starting* state is detected: a
//! methuselah that settles into still lifes after a transient is reported as
//! "no repetition".

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bevy::prelude::*;
use cellular_automata::{CellRect, LifeCommand, LifeConfig, LifeMode, LifeSim, PasteMode, PatternLibrary, Rotation};
use patterns::Pattern;
use simulation_core::{Control, ParamValue, Simulation, Snapshot, Topology};

use crate::session::{ConwayRunner, lock};

/// Largest population analysed.
pub const MAX_ANALYSIS_POPULATION: u64 = 200_000;
/// Most generations examined.
pub const MAX_ANALYSIS_GENERATIONS: u64 = 4_096;
/// Time budget of one analysis.
pub const ANALYSIS_BUDGET: Duration = Duration::from_secs(4);

/// What the analysis found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnalysisResult {
    /// No live cell to analyse.
    Empty,
    /// Every cell is dead after this many generations.
    DiesOut {
        /// Generations until extinction.
        after: u64,
    },
    /// The state is unchanged after one generation.
    StillLife,
    /// The state returns after `period` generations in place.
    Oscillator {
        /// Period in generations.
        period: u64,
    },
    /// The state returns translated by `(dx, dy)` after `period` generations.
    Spaceship {
        /// Period in generations.
        period: u64,
        /// Horizontal displacement per period.
        dx: i64,
        /// Vertical displacement per period.
        dy: i64,
    },
    /// No return to the starting state within the examined generations.
    NotFound {
        /// Generations examined.
        examined: u64,
    },
    /// Too many live cells.
    TooLarge {
        /// Live cells.
        population: u64,
    },
    /// The copy could not be built.
    Failed(String),
}

impl AnalysisResult {
    /// Sentence for the panel.
    pub fn describe(&self, finite: bool) -> String {
        match self {
            Self::Empty => "There is no live cell to analyse.".into(),
            Self::DiesOut { after } => format!("Dies out: no live cell is left after {after} generations."),
            Self::StillLife => "Still life: unchanged by one generation (period 1).".into(),
            Self::Oscillator { period } if finite => {
                format!("The whole world returns to this state every {period} generations.")
            }
            Self::Oscillator { period } => format!("Oscillator: back in place after {period} generations."),
            Self::Spaceship { period, dx, dy } => {
                format!("Moves by ({dx}, {dy}) every {period} generations ({}).", speed_label(*period, *dx, *dy))
            }
            Self::NotFound { examined } => format!(
                "Does not return to this state within {examined} generations (growth, debris or a longer period)."
            ),
            Self::TooLarge { population } => {
                format!("{population} live cells: more than the {MAX_ANALYSIS_POPULATION} the analysis examines.")
            }
            Self::Failed(e) => format!("The analysis could not run: {e}"),
        }
    }
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

/// Speed in Life notation: `c/4 diagonal`, `c/2 orthogonal`, `2c/5`...
pub fn speed_label(period: u64, dx: i64, dy: i64) -> String {
    let (ax, ay) = (dx.unsigned_abs(), dy.unsigned_abs());
    let d = ax.max(ay);
    let g = gcd(d, period).max(1);
    let fraction = if d / g == 1 { format!("c/{}", period / g) } else { format!("{}c/{}", d / g, period / g) };
    let direction = if ax == 0 || ay == 0 {
        "orthogonal"
    } else if ax == ay {
        "diagonal"
    } else {
        "oblique"
    };
    format!("{fraction} {direction}")
}

/// A state reduced for comparison: sorted cells, translated to their
/// bounding box when translation is ignored, plus that translation.
fn canonical(cells: Vec<(i64, i64)>, translate: bool) -> (Vec<(i64, i64)>, (i64, i64)) {
    if !translate || cells.is_empty() {
        return (cells, (0, 0));
    }
    let x0 = cells.iter().map(|c| c.0).min().unwrap_or(0);
    let y0 = cells.iter().map(|c| c.1).min().unwrap_or(0);
    (cells.into_iter().map(|(x, y)| (x - x0, y - y0)).collect(), (x0, y0))
}

/// Steps `sim` one generation at a time and compares with the start.
pub fn analyse(sim: &mut LifeSim, translate: bool, cancel: &AtomicBool, budget: Duration) -> AnalysisResult {
    let population = sim.population();
    if population == 0 {
        return AnalysisResult::Empty;
    }
    if population > MAX_ANALYSIS_POPULATION {
        return AnalysisResult::TooLarge { population };
    }
    for (key, value) in [
        ("track_ages", ParamValue::Bool(false)),
        ("track_activity", ParamValue::Bool(false)),
        ("generations_per_tick", ParamValue::Int(1)),
        ("engine", ParamValue::Text("auto".into())),
    ] {
        if let Err(e) = sim.set_param(key, value) {
            return AnalysisResult::Failed(e.to_string());
        }
    }
    let (start, origin) = canonical(sim.live_cells(), translate);
    let started = Instant::now();
    for generation in 1..=MAX_ANALYSIS_GENERATIONS {
        sim.step();
        if let Some(reason) = sim.halt_reason() {
            return AnalysisResult::Failed(reason.to_string());
        }
        if sim.population() == 0 {
            return AnalysisResult::DiesOut { after: generation };
        }
        if sim.population() == start.len() as u64 {
            let (now, at) = canonical(sim.live_cells(), translate);
            if now == start {
                let (dx, dy) = (at.0 - origin.0, at.1 - origin.1);
                return match (generation, dx, dy) {
                    (1, 0, 0) => AnalysisResult::StillLife,
                    (period, 0, 0) => AnalysisResult::Oscillator { period },
                    (period, dx, dy) => AnalysisResult::Spaceship { period, dx, dy },
                };
            }
        }
        if cancel.load(Ordering::Relaxed) || started.elapsed() > budget {
            return AnalysisResult::NotFound { examined: generation };
        }
    }
    AnalysisResult::NotFound { examined: MAX_ANALYSIS_GENERATIONS }
}

/// What is analysed.
enum Source {
    World(Result<Snapshot, String>),
    Cells(Result<Pattern, String>),
}

fn copy_of(source: Source) -> Result<LifeSim, String> {
    let empty = |topology| LifeConfig { topology, init: vec![], width: 16, height: 16, ..LifeConfig::default() };
    match source {
        Source::World(snapshot) => {
            let snapshot = snapshot?;
            let mut sim = LifeSim::new(&empty(Topology::Torus), 0, &PatternLibrary::builtin(), LifeMode::Classic)
                .map_err(|e| e.to_string())?;
            sim.restore(&snapshot).map_err(|e| e.to_string())?;
            Ok(sim)
        }
        Source::Cells(pattern) => {
            let pattern = pattern?;
            let mut sim = LifeSim::new(&empty(Topology::Infinite), 0, &PatternLibrary::builtin(), LifeMode::Classic)
                .map_err(|e| e.to_string())?;
            sim.apply(LifeCommand::Paste {
                pattern,
                x: 0,
                y: 0,
                rotation: Rotation::None,
                mirror: false,
                mode: PasteMode::Or,
            })
            .map_err(|e| e.to_string())?;
            Ok(sim)
        }
    }
}

/// A finished analysis.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnalysisReport {
    /// Generation of the live run when the analysis started.
    pub generation: u64,
    /// The selection analysed on its own, or `None` for the whole world.
    pub selection: Option<CellRect>,
    /// Whether the compared state was a finite world (exact comparison).
    pub finite: bool,
    /// The finding.
    pub result: AnalysisResult,
}

/// Messages of a running analysis.
type Outcome = (u64, bool, AnalysisResult);

/// The period analysis of the mode.
#[derive(Resource, Default)]
pub struct Analysis {
    /// Last finished analysis.
    pub report: Option<AnalysisReport>,
    running: Option<CellRectOrWorld>,
    pending: Mutex<Option<Receiver<Outcome>>>,
    cancel: Arc<AtomicBool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CellRectOrWorld(Option<CellRect>);

impl Analysis {
    /// Whether an analysis runs.
    pub fn running(&self) -> bool {
        self.running.is_some()
    }

    /// Starts analysing the selection (alone on an empty plane) or the world.
    pub fn start(&mut self, runner: &ConwayRunner, selection: Option<CellRect>, infinite_world: bool) {
        self.cancel.store(true, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = cancel.clone();
        let (tx, rx) = mpsc::channel();
        *lock(&self.pending) = Some(rx);
        self.running = Some(CellRectOrWorld(selection));
        // Runs on the simulation thread only long enough to copy the state;
        // the analysis itself runs on its own thread.
        runner.send(Control::Query(Box::new(move |sim: &LifeSim| {
            let generation = sim.tick();
            let (source, translate) = match selection {
                Some(rect) => (Source::Cells(sim.copy_region(rect).map_err(|e| e.to_string())), true),
                None => (Source::World(sim.snapshot().map_err(|e| e.to_string())), infinite_world),
            };
            let spawned = std::thread::Builder::new().name("conway-analysis".into()).spawn(move || {
                let result = match copy_of(source) {
                    Ok(mut copy) => analyse(&mut copy, translate, &cancel, ANALYSIS_BUDGET),
                    Err(e) => AnalysisResult::Failed(e),
                };
                let _ = tx.send((generation, !translate, result));
            });
            if let Err(e) = spawned {
                warn!("could not start the period analysis: {e}");
            }
        })));
    }

    /// Reports, without running anything, that `population` live cells are
    /// too many to analyse.
    pub fn report_too_large(&mut self, generation: u64, selection: Option<CellRect>, finite: bool, population: u128) {
        self.cancel();
        let population = u64::try_from(population).unwrap_or(u64::MAX);
        self.report =
            Some(AnalysisReport { generation, selection, finite, result: AnalysisResult::TooLarge { population } });
    }

    /// Stops a running analysis (its thread notices within one generation).
    pub fn cancel(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        *lock(&self.pending) = None;
        self.running = None;
    }
}

/// Collects finished analyses.
pub(crate) fn poll_analysis(mut analysis: ResMut<Analysis>) {
    let received = lock(&analysis.pending).as_ref().map(Receiver::try_recv);
    match received {
        Some(Ok((generation, finite, result))) => {
            *lock(&analysis.pending) = None;
            let selection = analysis.running.take().and_then(|r| r.0);
            analysis.report = Some(AnalysisReport { generation, selection, finite, result });
        }
        Some(Err(TryRecvError::Disconnected)) => {
            *lock(&analysis.pending) = None;
            analysis.running = None;
        }
        Some(Err(TryRecvError::Empty)) | None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on_plane(text: &str) -> LifeSim {
        let pattern = Pattern::from_ascii(text).unwrap();
        copy_of(Source::Cells(Ok(pattern))).unwrap()
    }

    fn run(text: &str) -> AnalysisResult {
        analyse(&mut on_plane(text), true, &AtomicBool::new(false), Duration::from_secs(30))
    }

    #[test]
    fn classic_objects_are_recognised() {
        assert_eq!(run("OO\nOO"), AnalysisResult::StillLife);
        assert_eq!(run("OOO"), AnalysisResult::Oscillator { period: 2 });
        assert_eq!(run(".O.\n..O\nOOO"), AnalysisResult::Spaceship { period: 4, dx: 1, dy: 1 });
        // LWSS moving left: (-2, 0) every 4 generations.
        assert_eq!(run("O..O.\n....O\nO...O\n.OOOO"), AnalysisResult::Spaceship { period: 4, dx: 2, dy: 0 });
        assert_eq!(run("O"), AnalysisResult::DiesOut { after: 1 });
        assert!(matches!(run(".O\nO.\nOO\n.OO"), AnalysisResult::NotFound { .. } | AnalysisResult::DiesOut { .. }));
        let mut empty = copy_of(Source::Cells(Ok(Pattern::default()))).unwrap();
        assert_eq!(analyse(&mut empty, true, &AtomicBool::new(false), ANALYSIS_BUDGET), AnalysisResult::Empty);
    }

    #[test]
    fn finite_worlds_compare_exactly_and_snapshots_restore() {
        // A glider on an 8 x 8 torus returns to the same place after 32 generations.
        let config = LifeConfig {
            topology: Topology::Torus,
            width: 8,
            height: 8,
            init: vec![cellular_automata::InitItem::Pattern {
                source: cellular_automata::PatternSource::Text("bo$2bo$3o!".into()),
                x: 1,
                y: 1,
                rotation: Rotation::None,
                mirror: false,
            }],
            ..LifeConfig::default()
        };
        let torus = LifeSim::new(&config, 1, &PatternLibrary::builtin(), LifeMode::Classic).unwrap();
        let mut sim = copy_of(Source::World(torus.snapshot().map_err(|e| e.to_string()))).unwrap();
        assert_eq!(sim.world().topology, Topology::Torus);
        let r = analyse(&mut sim, false, &AtomicBool::new(false), Duration::from_secs(30));
        assert_eq!(r, AnalysisResult::Oscillator { period: 32 });
        assert!(r.describe(true).contains("whole world"));
    }

    #[test]
    fn limits_and_labels() {
        let cancelled = AtomicBool::new(true);
        let r = analyse(&mut on_plane(".OO\nOO.\n.O."), true, &cancelled, ANALYSIS_BUDGET);
        assert_eq!(r, AnalysisResult::NotFound { examined: 1 }, "cancellation stops after one generation");
        assert_eq!(speed_label(4, 1, 1), "c/4 diagonal");
        assert_eq!(speed_label(4, -2, 0), "c/2 orthogonal");
        assert_eq!(speed_label(5, 2, 0), "2c/5 orthogonal");
        assert_eq!(speed_label(6, 2, 1), "c/3 oblique");
        assert!(AnalysisResult::Spaceship { period: 4, dx: 1, dy: 1 }.describe(false).contains("c/4 diagonal"));
    }
}
