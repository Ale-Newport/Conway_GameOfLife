//! Data fetched from the simulation thread on demand: world description and
//! fresh descriptor, the 2D slice of the inspector, per-state counts inside
//! the selected region and the probe of one voxel.
//!
//! Every request runs as a closure on the simulation thread
//! (`Runner::request`) and is polled without blocking. At most one request
//! of each kind is in flight; a new one is sent only when its inputs change
//! (the inputs include the displayed content version, so a paused world is
//! not polled repeatedly).

use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bevy::prelude::*;
use interface::{MetricsHistory, ModeScenarios, SessionStatus};
use simulation_core::{ParamSet, Simulation};
use voxel_life::{
    Axis, Bounds, Evidence, Neighbourhood, Rule3D, SliceView, VoxelLife, VoxelView, VoxelViewRequest, WorldConfig,
    WorldKind,
};

use crate::build::MeshState;
use crate::coords::ModelFrame;
use crate::scene::CameraAction;
use crate::session::VoxelRunner;
use crate::settings::ViewSettings;

/// A value being computed on the simulation thread.
pub(crate) struct Pending<T> {
    // `Receiver` is not `Sync`; the mutex makes the holder a valid resource
    // (it is only accessed through `&mut self`, so it never blocks).
    rx: Option<Mutex<Receiver<T>>>,
}

impl<T> Default for Pending<T> {
    fn default() -> Self {
        Self { rx: None }
    }
}

impl<T> Pending<T> {
    /// Whether a result is awaited.
    pub(crate) fn is_waiting(&self) -> bool {
        self.rx.is_some()
    }

    /// Starts waiting for `rx` (dropping any previous request).
    pub(crate) fn start(&mut self, rx: Receiver<T>) {
        self.rx = Some(Mutex::new(rx));
    }

    /// The result, if it arrived. A dropped sender (the session ended)
    /// stops the wait.
    pub(crate) fn poll(&mut self) -> Option<T> {
        let rx = self.rx.as_mut()?.get_mut().unwrap_or_else(std::sync::PoisonError::into_inner);
        match rx.try_recv() {
            Ok(value) => {
                self.rx = None;
                Some(value)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.rx = None;
                None
            }
        }
    }
}

/// Scenario fields shown in the Properties tab.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ScenarioMeta {
    /// Identifier.
    pub id: String,
    /// Display name.
    pub name: String,
    /// What it shows.
    pub description: String,
    /// How well the behaviour is established.
    pub evidence: Evidence,
    /// Source.
    pub reference: String,
}

/// Description of the running world.
#[derive(Clone, Debug)]
pub(crate) struct WorldInfo {
    /// The loaded scenario.
    pub scenario: ScenarioMeta,
    /// The rule being executed.
    pub rule: Rule3D,
    /// Current world configuration.
    pub config: WorldConfig,
    /// Configuration the next reset uses.
    pub pending: WorldConfig,
    /// Coordinates that can hold voxels.
    pub bounds: Bounds,
    /// Why the model halted, if it did.
    pub halt: Option<String>,
    /// Generation of the reading.
    pub tick: u64,
    /// Voxels per state at that generation (index 0 unused).
    pub counts: Vec<u64>,
    /// Box of the non-empty voxels at that generation.
    pub population_bounds: Option<Bounds>,
}

impl WorldInfo {
    /// Reads the world description (on the simulation thread).
    pub(crate) fn capture(sim: &VoxelLife) -> Self {
        let scenario = sim.scenario();
        Self {
            scenario: ScenarioMeta {
                id: scenario.id.clone(),
                name: scenario.name.clone(),
                description: scenario.description.clone(),
                evidence: scenario.evidence,
                reference: scenario.reference.clone(),
            },
            rule: sim.rule(),
            config: sim.config().clone(),
            pending: sim.pending_config().clone(),
            bounds: sim.bounds(),
            halt: sim.halt_reason().map(str::to_string),
            tick: sim.tick(),
            counts: sim.stats().counts().to_vec(),
            population_bounds: sim.stats().bounds(),
        }
    }

    /// Whether the world is finite.
    pub(crate) fn is_finite(&self) -> bool {
        self.config.kind.is_finite()
    }

    /// The nominal box: the world for finite worlds, the initialisation box
    /// for the unbounded world.
    pub(crate) fn nominal(&self) -> Bounds {
        Bounds::from_size(self.config.size)
    }

    /// What identifies "the same world" for view settings: a new scenario,
    /// world kind or size resets the cut, the region and the camera.
    pub(crate) fn signature(&self) -> (String, WorldKind, [u32; 3]) {
        (self.scenario.id.clone(), self.config.kind, self.config.size)
    }
}

/// Population and alive voxels at generation `tick`: from the recorded
/// metrics, else from the world description when it was read at that
/// generation (a new runner records no sample of its first generation).
pub(crate) fn current_counts(history: &MetricsHistory, info: Option<&WorldInfo>, tick: u64) -> Option<(u64, u64)> {
    let store = history.store();
    if let Some(population) = store.last("population").filter(|p| p.tick == tick) {
        let alive = store.last("state_1").filter(|p| p.tick == tick).map_or(0.0, |p| p.value);
        return Some((population.value as u64, alive as u64));
    }
    let info = info.filter(|i| i.tick == tick)?;
    let population = info.counts.iter().skip(1).sum();
    Some((population, info.counts.get(1).copied().unwrap_or(0)))
}

/// Smallest box holding the non-empty voxels, from the latest metrics (only
/// when they belong to the latest recorded generation).
pub(crate) fn population_bounds(history: &MetricsHistory) -> Option<Bounds> {
    let store = history.store();
    let tick = store.last("population")?.tick;
    let mut out = Bounds { min: [0; 3], max: [0; 3] };
    for (a, name) in ["x", "y", "z"].iter().enumerate() {
        let lo = store.last(&format!("bbox_min_{name}")).filter(|p| p.tick == tick)?;
        let hi = store.last(&format!("bbox_max_{name}")).filter(|p| p.tick == tick)?;
        out.min[a] = lo.value as i32;
        out.max[a] = hi.value as i32;
    }
    Some(out)
}

/// Cached world description and the trigger logic that refreshes it.
#[derive(Resource, Default)]
pub(crate) struct WorldState {
    /// Latest description (shared, so panels can hold it cheaply).
    pub info: Option<Arc<WorldInfo>>,
    pending: Pending<WorldInfo>,
    last_params: Option<ParamSet>,
    last_tick: Option<u64>,
    last_finished: bool,
    since_refresh: Duration,
    force: bool,
    signature: Option<(String, WorldKind, [u32; 3])>,
    /// Incremented whenever a different world (scenario, kind or size)
    /// arrives.
    pub generation: u64,
    /// Latest population box (from the metrics).
    pub population: Option<Bounds>,
}

impl WorldState {
    /// A state that requests the world description at once.
    pub(crate) fn requesting() -> Self {
        Self { force: true, ..Self::default() }
    }

    /// Marks the description stale (e.g. after loading a scenario).
    pub(crate) fn invalidate(&mut self) {
        self.force = true;
    }

    /// Layers the cut, region and inspector may use: the world box when it
    /// is finite, else the initialisation box joined with the population.
    pub(crate) fn layers(&self) -> Bounds {
        let Some(info) = &self.info else { return Bounds { min: [0; 3], max: [0; 3] } };
        if info.is_finite() {
            return info.bounds;
        }
        match self.population {
            Some(population) => info.nominal().union(population),
            None => info.nominal(),
        }
    }

    /// Model placement (fixed per world).
    pub(crate) fn frame(&self) -> ModelFrame {
        self.info.as_ref().map(|i| ModelFrame::for_box(i.nominal())).unwrap_or_default()
    }

    /// Height (z) range for the height ramp.
    pub(crate) fn height_range(&self) -> (i32, i32) {
        let layers = self.layers();
        (layers.min[2], layers.max[2])
    }
}

/// Time between unconditional refreshes (catches world changes that
/// trigger nothing else, such as a reset at generation 0 after changing
/// the edges).
const REFRESH_PERIOD: Duration = Duration::from_secs(3);

/// Requests and receives the world description; resets per-world view
/// settings and frames the camera when a different world arrives.
#[allow(clippy::too_many_arguments)]
pub(crate) fn refresh_world(
    mut world: ResMut<WorldState>,
    runner: Res<VoxelRunner>,
    status: Res<SessionStatus>,
    history: Res<MetricsHistory>,
    time: Res<Time<Real>>,
    mut settings: ResMut<ViewSettings>,
    mut scenarios: ResMut<ModeScenarios>,
    mut camera: MessageWriter<CameraAction>,
) {
    if let Some(info) = world.pending.poll() {
        // A loaded save may hold another scenario: keep the picker honest.
        let selected =
            scenarios.get(crate::MODE).and_then(|l| l.selected.as_deref()) == Some(info.scenario.id.as_str());
        if !selected {
            scenarios.mark_selected(crate::MODE, &info.scenario.id);
        }
        let signature = info.signature();
        let is_new = world.signature.as_ref() != Some(&signature);
        world.signature = Some(signature);
        world.info = Some(Arc::new(info));
        if is_new {
            world.population = world.info.as_ref().and_then(|i| i.population_bounds);
            let layers = world.layers();
            settings.reset_for_world(layers);
            // Start the inspector on the layer through the population.
            if let Some(population) = world.population {
                let z = Axis::Z.index();
                settings.inspector.layer = population.min[z] + (population.max[z] - population.min[z]) / 2;
            }
            world.generation += 1;
            camera.write(CameraAction::FrameWorld);
        }
    }
    world.since_refresh += time.delta();
    let Some(status) = status.get() else { return };
    // The world description, read on the simulation thread, is exact for
    // its generation (also the first one, of which a new runner records no
    // metrics); while the run moves on, the latest metrics carry the
    // population box.
    let from_info = world.info.as_ref().filter(|i| i.tick == status.tick).map(|i| i.population_bounds);
    world.population = match from_info {
        Some(bounds) => bounds,
        None => population_bounds(&history),
    };
    let mut trigger = std::mem::take(&mut world.force) || world.since_refresh >= REFRESH_PERIOD;
    trigger |= world.last_params.as_ref() != Some(&status.params);
    trigger |= world.last_tick.is_some_and(|t| status.tick < t);
    trigger |= status.finished != world.last_finished;
    world.last_params = Some(status.params.clone());
    world.last_tick = Some(status.tick);
    world.last_finished = status.finished;
    if !trigger {
        return;
    }
    if world.pending.is_waiting() {
        world.force = true;
        return;
    }
    world.since_refresh = Duration::ZERO;
    let rx = runner.0.request(|sim| WorldInfo::capture(sim));
    world.pending.start(rx);
}

/// Inputs of a slice request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SliceKey {
    axis: Axis,
    layer: i32,
    content: u64,
}

/// The 2D slice inspector's data.
#[derive(Resource, Default)]
pub(crate) struct SliceState {
    pending: Pending<Option<SliceView>>,
    requested: Option<SliceKey>,
    /// Latest slice.
    pub view: Option<SliceView>,
    /// Alive and dying voxels of the latest slice.
    pub counts: (u64, u64),
    /// Bumped whenever `view` changes (the panel re-uploads its texture).
    pub version: u64,
    /// Set by the panel while the inspector is visible.
    pub wanted: bool,
    shown: bool,
}

impl SliceState {
    /// Whether the inspector was drawn in the last frame.
    pub(crate) fn shown_recently(&self) -> bool {
        self.shown
    }
}

/// Alive (state 1) and dying (state ≥ 2) cells of a slice.
pub(crate) fn slice_counts(cells: &[u8]) -> (u64, u64) {
    cells.iter().fold((0, 0), |(alive, dying), &s| match s {
        0 => (alive, dying),
        1 => (alive + 1, dying),
        _ => (alive, dying + 1),
    })
}

/// Fetches the inspector's layer when it is visible and its inputs changed.
pub(crate) fn poll_slice(
    mut slice: ResMut<SliceState>,
    runner: Res<VoxelRunner>,
    settings: Res<ViewSettings>,
    mesh: Res<MeshState>,
) {
    if let Some(view) = slice.pending.poll() {
        slice.counts = view.as_ref().map_or((0, 0), |v| slice_counts(v.cells.as_slice()));
        slice.view = view;
        slice.version += 1;
    }
    slice.shown = std::mem::take(&mut slice.wanted);
    if !slice.shown || slice.pending.is_waiting() {
        return;
    }
    let (axis, layer) = settings.inspector_layer();
    let key = SliceKey { axis, layer, content: mesh.content_version };
    if slice.requested == Some(key) {
        return;
    }
    slice.requested = Some(key);
    let rx = runner.0.request(move |sim| match sim.view(&VoxelViewRequest::Slice { axis, layer, window: None }) {
        VoxelView::Slice(view) => Some(view),
        _ => None,
    });
    slice.pending.start(rx);
}

/// Population of the selected region.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RegionCounts {
    /// The counted box (clipped to the world).
    pub bounds: Option<Bounds>,
    /// `counts[s]` = voxels in state `s` (index 0 unused).
    pub counts: Vec<u64>,
    /// Voxels in the box.
    pub volume: u64,
}

impl RegionCounts {
    /// Non-empty voxels in the box.
    pub(crate) fn population(&self) -> u64 {
        self.counts.iter().skip(1).sum()
    }
}

/// Counts voxels per state inside `region` (on the simulation thread).
/// Iterates the box when it is small compared with the population, the
/// population otherwise.
pub(crate) fn count_region(sim: &VoxelLife, region: Bounds) -> RegionCounts {
    let states = usize::from(sim.rule().states());
    let mut counts = vec![0u64; states];
    let Some(area) = sim.bounds().intersection(&region) else {
        return RegionCounts { bounds: None, counts, volume: 0 };
    };
    let volume = area.volume();
    let population = sim.stats().population();
    if volume <= population.saturating_mul(2).max(4096) {
        for z in area.min[2]..=area.max[2] {
            for y in area.min[1]..=area.max[1] {
                for x in area.min[0]..=area.max[0] {
                    let s = usize::from(sim.state_at([x, y, z]));
                    if s != 0 && s < states {
                        counts[s] += 1;
                    }
                }
            }
        }
    } else {
        for voxel in sim.voxels() {
            let s = usize::from(voxel.state);
            if area.contains(voxel.pos) && s < states {
                counts[s] += 1;
            }
        }
    }
    RegionCounts { bounds: Some(area), counts, volume }
}

/// Inputs of a region request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RegionKey {
    bounds: Bounds,
    content: u64,
}

/// Counts of the selected region.
#[derive(Resource, Default)]
pub(crate) struct RegionState {
    pending: Pending<RegionCounts>,
    requested: Option<RegionKey>,
    /// Latest counts.
    pub counts: Option<RegionCounts>,
}

/// Recounts the region when it or the content changed.
pub(crate) fn poll_region(
    mut region: ResMut<RegionState>,
    runner: Res<VoxelRunner>,
    settings: Res<ViewSettings>,
    mesh: Res<MeshState>,
) {
    if let Some(counts) = region.pending.poll() {
        region.counts = Some(counts);
    }
    if !settings.region.enabled || region.pending.is_waiting() {
        return;
    }
    let bounds = settings.region.bounds();
    let key = RegionKey { bounds, content: mesh.content_version };
    if region.requested == Some(key) {
        return;
    }
    region.requested = Some(key);
    let rx = runner.0.request(move |sim| count_region(sim, bounds));
    region.pending.start(rx);
}

/// What the probe found at one voxel.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ProbeResult {
    /// Position.
    pub pos: [i32; 3],
    /// State (0 empty, 1 alive, 2… dying).
    pub state: u8,
    /// Generations since birth or placement.
    pub age: u16,
    /// Neighbours in state 1 (the only ones counted).
    pub alive_neighbours: u32,
    /// Neighbourhood of the rule.
    pub neighbourhood: Neighbourhood,
    /// State in the next generation.
    pub next_state: u8,
    /// The rule.
    pub rule: Rule3D,
    /// Generation of the reading.
    pub tick: u64,
    /// Whether the position lies inside the world.
    pub inside: bool,
}

/// Alive neighbours of `pos` for the neighbour `offsets`, wrapping each
/// coordinate modulo `wrap` on a torus (so on an axis shorter than 3 a voxel
/// reached through several offsets is counted once per offset, as the
/// engines do).
pub(crate) fn alive_neighbours(
    pos: [i32; 3],
    offsets: &[[i32; 3]],
    wrap: Option<[u32; 3]>,
    state_at: impl Fn([i32; 3]) -> u8,
) -> u32 {
    offsets
        .iter()
        .filter(|d| {
            let mut p = [0i32; 3];
            for a in 0..3 {
                let c = i64::from(pos[a]) + i64::from(d[a]);
                p[a] = match wrap {
                    Some(size) => c.rem_euclid(i64::from(size[a].max(1))) as i32,
                    None => c.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
                };
            }
            state_at(p) == 1
        })
        .count() as u32
}

/// Reads one voxel and predicts its next state (on the simulation thread).
pub(crate) fn probe(sim: &VoxelLife, pos: [i32; 3]) -> ProbeResult {
    let rule = sim.rule();
    let voxel = sim.voxel(pos);
    let config = sim.config();
    let wrap = (config.kind == WorldKind::DenseTorus).then_some(config.size);
    let n = alive_neighbours(pos, rule.neighbourhood().offsets(), wrap, |p| sim.state_at(p));
    ProbeResult {
        pos,
        state: voxel.state,
        age: voxel.age,
        alive_neighbours: n,
        neighbourhood: rule.neighbourhood(),
        next_state: rule.next_state(voxel.state, n),
        rule,
        tick: sim.tick(),
        inside: sim.bounds().contains(pos),
    }
}

/// The pinned voxel and its latest reading.
#[derive(Resource, Default)]
pub(crate) struct ProbeState {
    pending: Pending<ProbeResult>,
    requested: Option<([i32; 3], u64)>,
    /// Voxel selected with the Inspect tool.
    pub pinned: Option<[i32; 3]>,
    /// Latest reading of the pinned voxel.
    pub result: Option<ProbeResult>,
}

/// Re-reads the pinned voxel when it or the content changed.
pub(crate) fn poll_probe(mut probe_state: ResMut<ProbeState>, runner: Res<VoxelRunner>, mesh: Res<MeshState>) {
    if let Some(result) = probe_state.pending.poll()
        && Some(result.pos) == probe_state.pinned
    {
        probe_state.result = Some(result);
    }
    let Some(pos) = probe_state.pinned else {
        probe_state.result = None;
        return;
    };
    if probe_state.pending.is_waiting() {
        return;
    }
    let key = (pos, mesh.content_version);
    if probe_state.requested == Some(key) {
        return;
    }
    probe_state.requested = Some(key);
    let rx = runner.0.request(move |sim| probe(sim, pos));
    probe_state.pending.start(rx);
}

#[cfg(test)]
mod tests {
    use super::*;
    use simulation_core::Simulation;
    use voxel_life::{VoxelCommand, scenarios};

    fn life(id: &str) -> VoxelLife {
        VoxelLife::new(scenarios::builtin_by_id(id).expect("builtin")).expect("valid")
    }

    #[test]
    fn slice_counts_split_alive_and_dying() {
        assert_eq!(slice_counts(&[0, 1, 2, 9, 1, 0]), (2, 2));
        assert_eq!(slice_counts(&[]), (0, 0));
    }

    #[test]
    fn neighbour_counting_wraps_on_a_torus_only() {
        let alive = [[0, 0, 0], [3, 0, 0]];
        let state_at = |p: [i32; 3]| u8::from(alive.contains(&p));
        let offsets = Neighbourhood::VonNeumann.offsets();
        // At (0,0,0) on a 4-wide torus the -x neighbour is (3,0,0).
        assert_eq!(alive_neighbours([0, 0, 0], offsets, Some([4, 4, 4]), state_at), 1);
        assert_eq!(alive_neighbours([0, 0, 0], offsets, None, state_at), 0);
        // A torus 1 voxel deep reaches the voxel's own plane through ±z: an
        // alive voxel counts itself twice, as the engine does.
        let one = |p: [i32; 3]| u8::from(p == [1, 1, 0]);
        assert_eq!(alive_neighbours([1, 1, 0], offsets, Some([4, 4, 1]), one), 2);
        // Dying neighbours do not count.
        let dying = |p: [i32; 3]| if p == [1, 0, 0] { 2 } else { 0 };
        assert_eq!(alive_neighbours([0, 0, 0], offsets, None, dying), 0);
    }

    #[test]
    fn probe_predicts_the_engine() {
        // Every voxel of the 5766 glider's neighbourhood: the probe's
        // prediction must equal the state after one real step.
        let mut sim = life("life-5766-glider");
        let positions: Vec<[i32; 3]> = sim
            .voxels()
            .iter()
            .flat_map(|v| Neighbourhood::Moore.offsets().iter().map(move |d| [0, 1, 2].map(|a| v.pos[a] + d[a])))
            .collect();
        let predictions: Vec<([i32; 3], u8)> = positions.iter().map(|&p| (p, probe(&sim, p).next_state)).collect();
        sim.step();
        for (pos, predicted) in predictions {
            assert_eq!(sim.state_at(pos), predicted, "voxel {pos:?}");
        }
    }

    #[test]
    fn region_counts_match_both_strategies() {
        let mut sim = life("builder-1");
        for _ in 0..12 {
            sim.step();
        }
        let population = sim.stats().population();
        let everything = count_region(&sim, sim.bounds());
        assert_eq!(everything.population(), population);
        assert_eq!(everything.volume, 64 * 64 * 64);
        assert_eq!(&everything.counts[1..], &sim.stats().counts()[1..]);
        // A small box (iterated voxel by voxel) and the same box counted from
        // the population list agree.
        let small = Bounds { min: [28, 28, 28], max: [35, 35, 35] };
        let direct = count_region(&sim, small);
        let listed = sim.voxels().iter().filter(|v| small.contains(v.pos)).count() as u64;
        assert_eq!(direct.population(), listed);
        // A box outside the world counts nothing.
        let outside = count_region(&sim, Bounds { min: [100; 3], max: [110; 3] });
        assert_eq!((outside.bounds, outside.population()), (None, 0));
        // Clipping keeps only the inside part.
        let clipped = count_region(&sim, Bounds { min: [-10; 3], max: [3; 3] });
        assert_eq!(clipped.volume, 64);
    }

    #[test]
    fn world_info_describes_the_running_rule() {
        let mut sim = life("rokicki-ship");
        let info = WorldInfo::capture(&sim);
        assert_eq!(info.scenario.id, "rokicki-ship");
        assert_eq!(info.config.kind, WorldKind::DenseTorus);
        assert!(info.is_finite());
        sim.apply(VoxelCommand::Clear).expect("clear");
        sim.set_param("rule", simulation_core::ParamValue::Text("B6/S567".into())).expect("valid rule");
        let info = WorldInfo::capture(&sim);
        assert_eq!(info.rule.to_string(), "5-7/6/2/M");
    }
}
