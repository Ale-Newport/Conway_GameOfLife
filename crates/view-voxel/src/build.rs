//! Surface view → chunk meshes, built off the main thread.
//!
//! When a new surface view is published (or the colouring changes), one
//! background task:
//!
//! 1. computes the visible bounding box, an occupancy bitset of the visible
//!    voxels (used for depth shading and for picking), the oldest age and a
//!    content hash;
//! 2. assigns every voxel a palette index (colour bucket, shade level,
//!    opacity class; see [`crate::colors`]);
//! 3. runs the rendering crate's [`ChunkedVoxelMesher`], which re-meshes only
//!    the 16³ chunks whose visible faces changed, and converts the changed
//!    chunks to Bevy meshes.
//!
//! The main thread only swaps mesh assets and spawns or despawns chunk
//! entities. One task runs at a time; when it finishes, the newest view is
//! taken, so a slow build skips intermediate generations instead of
//! queueing them.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Instant;

use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy::tasks::futures::check_ready;
use bevy::tasks::{AsyncComputeTaskPool, Task};
use rendering::voxel::{ChunkedVoxelMesher, MAX_VOXEL_COORD, Voxel, VoxelPalette};
use voxel_life::{SurfaceView, VoxelView};

use crate::colors::{
    AO_LEVELS, FULL_TONE, age_bucket, build_palette, height_bucket, palette_index, shade_level, state_bucket, tone,
};
use crate::scene::{SceneAssets, VoxelRoot};
use crate::session::VoxelRunner;
use crate::settings::{AgeScale, ColorMode, Transparency, ViewSettings};

/// Chunk edge in voxels (the model's sparse chunk size).
pub const CHUNK_SIZE: u32 = 16;
/// Largest visible bounding box (in voxels) tracked by the occupancy bitset
/// (16 MiB). Larger boxes skip depth shading and picking.
pub const MAX_OCCUPANCY_CELLS: u64 = 1 << 27;

/// Bitset of visible voxels over a box.
#[derive(Clone, Debug)]
pub struct Occupancy {
    min: IVec3,
    size: IVec3,
    bits: Vec<u64>,
}

impl Occupancy {
    /// An empty bitset over the inclusive box `[min, max]`, or `None` when
    /// the box exceeds [`MAX_OCCUPANCY_CELLS`].
    pub fn new(min: IVec3, max: IVec3) -> Option<Self> {
        let extent = |a: usize| i64::from(max[a]) - i64::from(min[a]) + 1;
        let (x, y, z) = (extent(0), extent(1), extent(2));
        if x <= 0 || y <= 0 || z <= 0 {
            return None;
        }
        let cells = (x as u64).checked_mul(y as u64)?.checked_mul(z as u64)?;
        if cells > MAX_OCCUPANCY_CELLS {
            return None;
        }
        Some(Self { min, size: IVec3::new(x as i32, y as i32, z as i32), bits: vec![0; cells.div_ceil(64) as usize] })
    }

    fn index(&self, p: IVec3) -> Option<usize> {
        let d = [0, 1, 2].map(|a| i64::from(p[a]) - i64::from(self.min[a]));
        if (0..3).any(|a| d[a] < 0 || d[a] >= i64::from(self.size[a])) {
            return None;
        }
        Some(((d[2] * i64::from(self.size.y) + d[1]) * i64::from(self.size.x) + d[0]) as usize)
    }

    /// Marks a voxel (ignored outside the box).
    pub fn insert(&mut self, p: IVec3) {
        if let Some(i) = self.index(p) {
            self.bits[i / 64] |= 1 << (i % 64);
        }
    }

    /// Whether a voxel is marked.
    pub fn contains(&self, p: IVec3) -> bool {
        self.index(p).is_some_and(|i| self.bits[i / 64] >> (i % 64) & 1 == 1)
    }

    /// Marked voxels among the 26 neighbours of `p`.
    pub fn neighbours26(&self, p: IVec3) -> u8 {
        let mut n = 0;
        for dz in -1..=1 {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    if (dx, dy, dz) != (0, 0, 0) && self.contains(p + IVec3::new(dx, dy, dz)) {
                        n += 1;
                    }
                }
            }
        }
        n
    }

    /// Smallest corner.
    pub fn min(&self) -> IVec3 {
        self.min
    }

    /// Largest corner (inclusive).
    pub fn max(&self) -> IVec3 {
        self.min + self.size - IVec3::ONE
    }
}

/// Everything that decides voxel colours.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshStyle {
    /// Ramp.
    pub color_mode: ColorMode,
    /// Age ramp scale.
    pub age_scale: AgeScale,
    /// Translucent voxels.
    pub transparency: Transparency,
    /// Opacity of translucent voxels.
    pub opacity: f32,
    /// Darken crowded voxels.
    pub depth_shading: bool,
    /// Checkerboard tint of odd voxels.
    pub checker: bool,
    /// Height ramp range (z).
    pub height_range: (i32, i32),
}

impl MeshStyle {
    /// The style of the current settings.
    pub fn from_settings(settings: &ViewSettings, height_range: (i32, i32)) -> Self {
        Self {
            color_mode: settings.color_mode,
            age_scale: settings.age_scale,
            transparency: settings.transparency,
            opacity: settings.opacity,
            depth_shading: settings.depth_shading,
            checker: settings.checker,
            height_range: if settings.color_mode == ColorMode::Height { height_range } else { (0, 0) },
        }
    }

    fn palette_key(&self) -> PaletteKey {
        PaletteKey { mode: self.color_mode, opacity_milli: (self.opacity.clamp(0.0, 1.0) * 1000.0).round() as u16 }
    }
}

/// What the palette depends on (index assignment depends on more).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PaletteKey {
    mode: ColorMode,
    opacity_milli: u16,
}

impl PaletteKey {
    fn palette(self) -> VoxelPalette {
        build_palette(self.mode, f32::from(self.opacity_milli) / 1000.0)
    }
}

/// Facts about the last displayed surface view.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SurfaceSummary {
    /// Generation shown.
    pub tick: u64,
    /// Number of states of the rule.
    pub states: u16,
    /// Voxels listed by the model (at most the render cap).
    pub listed: usize,
    /// Visible voxels before the cap.
    pub visible: u64,
    /// Whether the cap cut the list.
    pub truncated: bool,
    /// Voxels handed to the mesher.
    pub drawn: usize,
    /// Voxels beyond the mesher's coordinate range (not drawn).
    pub out_of_range: usize,
    /// Oldest listed voxel.
    pub max_age: u16,
    /// Age at the top of the age ramp.
    pub age_top: u16,
    /// Bounding box of the listed voxels.
    pub bbox: Option<(IVec3, IVec3)>,
    /// Hash of positions, states and ages.
    pub content_hash: u64,
}

/// Mesher input of one view.
pub struct Prepared {
    /// Voxels with palette indices.
    pub voxels: Vec<Voxel>,
    /// Visible-voxel bitset (absent for huge boxes).
    pub occupancy: Option<Occupancy>,
    /// Summary.
    pub summary: SurfaceSummary,
}

/// Assigns palette indices to the voxels of a surface view.
pub fn prepare(view: &SurfaceView, style: &MeshStyle) -> Prepared {
    let mut hasher = DefaultHasher::new();
    let mut lo = IVec3::splat(i32::MAX);
    let mut hi = IVec3::splat(i32::MIN);
    let mut max_age = 0u16;
    for v in &view.voxels {
        let p = IVec3::from_array(v.pos);
        lo = lo.min(p);
        hi = hi.max(p);
        max_age = max_age.max(v.age);
        (v.pos, v.state, v.age).hash(&mut hasher);
    }
    view.visible.hash(&mut hasher);
    let bbox = (!view.voxels.is_empty()).then_some((lo, hi));
    let mut occupancy = bbox.and_then(|(lo, hi)| Occupancy::new(lo, hi));
    if let Some(occupancy) = occupancy.as_mut() {
        for v in &view.voxels {
            occupancy.insert(IVec3::from_array(v.pos));
        }
    }
    let age_top = match style.age_scale {
        AgeScale::Auto => max_age.max(1),
        AgeScale::Fixed(top) => top.max(1),
    };
    let (z_lo, z_hi) = style.height_range;
    let in_range = |c: i32| (-MAX_VOXEL_COORD..=MAX_VOXEL_COORD).contains(&c);
    let mut voxels = Vec::with_capacity(view.voxels.len());
    let mut out_of_range = 0;
    for v in &view.voxels {
        let [x, y, z] = v.pos;
        if !(in_range(x) && in_range(y) && in_range(z)) {
            out_of_range += 1;
            continue;
        }
        let translucent = style.transparency.is_translucent(v.state);
        let color = match style.color_mode {
            ColorMode::State => state_bucket(v.state, view.states),
            ColorMode::Age => age_bucket(v.age, age_top),
            ColorMode::Height => height_bucket(z, z_lo, z_hi),
        };
        // Translucent voxels stay uniform so the mesher can merge glass of
        // one state (no internal faces).
        let shade = if translucent {
            FULL_TONE
        } else {
            let level = match (&occupancy, style.depth_shading) {
                (Some(occupancy), true) => shade_level(occupancy.neighbours26(IVec3::new(x, y, z))),
                _ => AO_LEVELS - 1,
            };
            let odd = style.checker && (i64::from(x) + i64::from(y) + i64::from(z)).rem_euclid(2) == 1;
            tone(level, odd)
        };
        voxels.push(Voxel::new(x, y, z, palette_index(translucent, color, shade)));
    }
    let summary = SurfaceSummary {
        tick: view.tick,
        states: view.states,
        listed: view.voxels.len(),
        visible: view.visible,
        truncated: view.truncated,
        drawn: voxels.len(),
        out_of_range,
        max_age,
        age_top,
        bbox,
        content_hash: hasher.finish(),
    };
    Prepared { voxels, occupancy, summary }
}

/// New or changed meshes of one chunk.
pub struct ChunkMesh {
    chunk: IVec3,
    origin: IVec3,
    opaque: Option<Mesh>,
    translucent: Option<Mesh>,
    faces: usize,
}

struct BuildJob {
    view: Arc<VoxelView>,
    seq: u64,
    style: MeshStyle,
    mesher: ChunkedVoxelMesher,
    palette: Option<(PaletteKey, VoxelPalette)>,
}

/// Result of a background build.
pub struct BuildOutput {
    seq: u64,
    style: MeshStyle,
    mesher: ChunkedVoxelMesher,
    palette: (PaletteKey, VoxelPalette),
    updated: Vec<ChunkMesh>,
    removed: Vec<IVec3>,
    occupancy: Option<Arc<Occupancy>>,
    summary: Option<SurfaceSummary>,
    elapsed_ms: f32,
    error: Option<String>,
}

fn new_mesher() -> ChunkedVoxelMesher {
    ChunkedVoxelMesher::new(CHUNK_SIZE).expect("16 is a valid chunk size")
}

fn run_build(job: BuildJob) -> BuildOutput {
    let start = Instant::now();
    let key = job.style.palette_key();
    let palette = match job.palette {
        Some((k, palette)) if k == key => palette,
        _ => key.palette(),
    };
    let mut mesher = job.mesher;
    let mut out = BuildOutput {
        seq: job.seq,
        style: job.style,
        mesher: new_mesher(),
        palette: (key, palette),
        updated: Vec::new(),
        removed: Vec::new(),
        occupancy: None,
        summary: None,
        elapsed_ms: 0.0,
        error: None,
    };
    if let VoxelView::Surface(view) = &*job.view {
        let prepared = prepare(view, &job.style);
        match mesher.rebuild(&prepared.voxels, &out.palette.1, 1.0) {
            Ok(diff) => {
                out.removed = diff.removed;
                out.updated = diff
                    .updated
                    .into_iter()
                    .map(|u| ChunkMesh {
                        chunk: u.chunk,
                        origin: u.origin,
                        faces: u.meshes.face_count(),
                        opaque: (!u.meshes.opaque.is_empty()).then(|| u.meshes.opaque.to_mesh()),
                        translucent: (!u.meshes.translucent.is_empty()).then(|| u.meshes.translucent.to_mesh()),
                    })
                    .collect();
            }
            Err(e) => out.error = Some(e.to_string()),
        }
        out.occupancy = prepared.occupancy.map(Arc::new);
        out.summary = Some(prepared.summary);
    }
    out.mesher = mesher;
    out.elapsed_ms = start.elapsed().as_secs_f32() * 1000.0;
    out
}

type ChunkKey = (i32, i32, i32);

#[derive(Default)]
struct ChunkEntities {
    opaque: Option<(Entity, Handle<Mesh>)>,
    translucent: Option<(Entity, Handle<Mesh>)>,
    faces: usize,
}

/// Mesh pipeline state: the mesher, the build in flight and the chunk
/// entities on screen.
#[derive(Resource, Default)]
pub struct MeshState {
    mesher: Option<ChunkedVoxelMesher>,
    palette: Option<(PaletteKey, VoxelPalette)>,
    task: Option<Task<BuildOutput>>,
    built: Option<(u64, MeshStyle)>,
    chunks: HashMap<ChunkKey, ChunkEntities>,
    last_content: Option<(u64, u64)>,
    /// Visible-voxel bitset of the displayed view.
    pub occupancy: Option<Arc<Occupancy>>,
    /// Summary of the displayed view.
    pub summary: SurfaceSummary,
    /// Faces drawn.
    pub faces: usize,
    /// Duration of the last background build.
    pub build_ms: f32,
    /// Incremented whenever the displayed generation or content changes.
    pub content_version: u64,
    /// Whether at least one view has been displayed.
    pub ready: bool,
    /// Last mesher error.
    pub error: Option<String>,
}

impl MeshState {
    /// Number of chunks with meshes.
    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }
}

/// Applies finished builds and starts the next one.
#[allow(clippy::too_many_arguments)]
pub(crate) fn pump_builds(
    mut state: ResMut<MeshState>,
    runner: Res<VoxelRunner>,
    settings: Res<ViewSettings>,
    world: Res<crate::requests::WorldState>,
    root: Option<Single<Entity, With<VoxelRoot>>>,
    assets: Option<Res<SceneAssets>>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let (Some(root), Some(assets)) = (root, assets) else { return };
    let root = *root;
    let finished = state.task.as_mut().and_then(check_ready);
    if let Some(output) = finished {
        state.task = None;
        apply_output(&mut state, output, root, &assets, &mut commands, &mut meshes);
    }
    if state.task.is_some() {
        return;
    }
    let Some(published) = runner.0.runner().latest_view() else { return };
    if !matches!(*published.view, VoxelView::Surface(_)) {
        return;
    }
    let style = MeshStyle::from_settings(&settings, world.height_range());
    if state.built == Some((published.seq, style)) {
        return;
    }
    let job = BuildJob {
        view: published.view,
        seq: published.seq,
        style,
        mesher: state.mesher.take().unwrap_or_else(new_mesher),
        palette: state.palette.take(),
    };
    state.task = Some(AsyncComputeTaskPool::get().spawn(async move { run_build(job) }));
}

fn apply_output(
    state: &mut MeshState,
    output: BuildOutput,
    root: Entity,
    assets: &SceneAssets,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
) {
    state.mesher = Some(output.mesher);
    state.palette = Some(output.palette);
    state.built = Some((output.seq, output.style));
    state.build_ms = output.elapsed_ms;
    state.error = output.error;
    for chunk in output.removed {
        if let Some(parts) = state.chunks.remove(&(chunk.x, chunk.y, chunk.z)) {
            for (entity, _) in [parts.opaque, parts.translucent].into_iter().flatten() {
                commands.entity(entity).despawn();
            }
        }
    }
    for update in output.updated {
        let parts = state.chunks.entry((update.chunk.x, update.chunk.y, update.chunk.z)).or_default();
        parts.faces = update.faces;
        let translation = update.origin.as_vec3();
        let mut sync = |slot: &mut Option<(Entity, Handle<Mesh>)>, mesh: Option<Mesh>, translucent: bool| {
            let Some(mesh) = mesh else {
                if let Some((entity, _)) = slot.take() {
                    commands.entity(entity).despawn();
                }
                return;
            };
            if let Some((_, handle)) = slot.as_ref()
                && let Some(mut existing) = meshes.get_mut(handle)
            {
                *existing = mesh;
                return;
            }
            let handle = meshes.add(mesh);
            let material = if translucent { assets.translucent.clone() } else { assets.opaque.clone() };
            let mut entity = commands.spawn((
                Name::new("3D Life chunk"),
                Mesh3d(handle.clone()),
                MeshMaterial3d(material),
                Transform::from_translation(translation),
                ChildOf(root),
            ));
            if translucent {
                entity.insert(NotShadowCaster);
            }
            *slot = Some((entity.id(), handle));
        };
        sync(&mut parts.opaque, update.opaque, false);
        sync(&mut parts.translucent, update.translucent, true);
    }
    state.chunks.retain(|_, parts| parts.opaque.is_some() || parts.translucent.is_some());
    state.faces = state.chunks.values().map(|p| p.faces).sum();
    if output.summary.is_some() {
        state.occupancy = output.occupancy;
    }
    if let Some(summary) = output.summary {
        let content = (summary.tick, summary.content_hash);
        if state.last_content != Some(content) {
            state.last_content = Some(content);
            state.content_version += 1;
        }
        state.summary = summary;
        state.ready = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use voxel_life::{Bounds, SliceRange, SurfaceVoxel, faces};

    fn view(voxels: Vec<SurfaceVoxel>, states: u16) -> SurfaceView {
        SurfaceView {
            tick: 7,
            states,
            world: Bounds { min: [0; 3], max: [15; 3] },
            range: SliceRange::all(),
            visible: voxels.len() as u64,
            truncated: false,
            voxels,
        }
    }

    fn sv(x: i32, y: i32, z: i32, state: u8, age: u16) -> SurfaceVoxel {
        SurfaceVoxel { pos: [x, y, z], state, age, faces: faces::ALL }
    }

    fn style() -> MeshStyle {
        MeshStyle {
            color_mode: ColorMode::State,
            age_scale: AgeScale::Fixed(10),
            transparency: Transparency::Opaque,
            opacity: 0.3,
            depth_shading: true,
            checker: true,
            height_range: (0, 15),
        }
    }

    #[test]
    fn occupancy_marks_and_counts_neighbours() {
        let mut occ = Occupancy::new(IVec3::new(-2, -2, -2), IVec3::new(2, 2, 2)).expect("small");
        for dz in -1..=1 {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    occ.insert(IVec3::new(dx, dy, dz));
                }
            }
        }
        assert!(occ.contains(IVec3::ZERO));
        assert!(!occ.contains(IVec3::splat(2)));
        assert!(!occ.contains(IVec3::splat(9)), "outside the box");
        assert_eq!(occ.neighbours26(IVec3::ZERO), 26);
        assert_eq!(occ.neighbours26(IVec3::new(1, 1, 1)), 7);
        assert_eq!((occ.min(), occ.max()), (IVec3::splat(-2), IVec3::splat(2)));
        assert!(Occupancy::new(IVec3::ZERO, IVec3::splat(1 << 12)).is_none(), "too large");
        assert!(Occupancy::new(IVec3::ONE, IVec3::ZERO).is_none(), "empty box");
    }

    #[test]
    fn prepare_colours_shades_and_summarises() {
        // A lone voxel (fully lit) and a 3x3x3 block whose centre is hidden.
        let mut voxels = vec![sv(10, 10, 10, 1, 3)];
        for z in 0..3 {
            for y in 0..3 {
                for x in 0..3 {
                    if (x, y, z) != (1, 1, 1) {
                        voxels.push(sv(x, y, z, if z == 2 { 2 } else { 1 }, 0));
                    }
                }
            }
        }
        let prepared = prepare(&view(voxels, 5), &style());
        assert_eq!(prepared.summary.listed, 27);
        assert_eq!(prepared.summary.max_age, 3);
        assert_eq!(prepared.summary.bbox, Some((IVec3::ZERO, IVec3::splat(10))));
        let lone = prepared.voxels.iter().find(|v| v.x == 10).expect("lone voxel");
        let (translucent, color, shade) = crate::colors::decode_index(lone.state).expect("index");
        assert!(!translucent);
        assert_eq!((color, shade), (crate::colors::COLORS - 1, FULL_TONE), "alive, fully lit, even parity");
        // The voxel in the middle of a face of the block has 16 visible
        // neighbours (the hidden centre does not count): it is shaded.
        let face = prepared.voxels.iter().find(|v| (v.x, v.y, v.z) == (1, 1, 0)).expect("face voxel");
        let (_, _, shade) = crate::colors::decode_index(face.state).expect("index");
        assert!(shade / 2 < AO_LEVELS - 1, "occluded");
        assert_eq!(shade % 2, 0, "(1, 1, 0) has even parity");
        let odd = prepared.voxels.iter().find(|v| (v.x, v.y, v.z) == (1, 0, 0)).expect("odd voxel");
        assert_eq!(crate::colors::decode_index(odd.state).expect("index").2 % 2, 1, "checker on odd parity");
        // Dying voxels get a different bucket.
        let dying = prepared.voxels.iter().find(|v| v.z == 2).expect("dying");
        assert_ne!(crate::colors::decode_index(dying.state).map(|d| d.1), Some(crate::colors::COLORS - 1));
    }

    #[test]
    fn prepare_respects_transparency_and_shading_switches() {
        let voxels = vec![sv(0, 0, 0, 1, 0), sv(1, 0, 0, 2, 0), sv(0, 1, 0, 1, 0)];
        let ghost = MeshStyle { transparency: Transparency::GhostDying, ..style() };
        let prepared = prepare(&view(voxels.clone(), 3), &ghost);
        let classes: Vec<bool> =
            prepared.voxels.iter().map(|v| crate::colors::decode_index(v.state).expect("index").0).collect();
        assert_eq!(classes, vec![false, true, false], "only the dying voxel is translucent");
        let flat = MeshStyle { depth_shading: false, checker: false, ..style() };
        let prepared = prepare(&view(voxels, 3), &flat);
        assert!(prepared.voxels.iter().all(|v| crate::colors::decode_index(v.state).expect("index").2 == FULL_TONE));
    }

    #[test]
    fn content_hash_tracks_states_and_ages() {
        let a = prepare(&view(vec![sv(1, 2, 3, 1, 0)], 2), &style()).summary.content_hash;
        let same = prepare(&view(vec![sv(1, 2, 3, 1, 0)], 2), &style()).summary.content_hash;
        let older = prepare(&view(vec![sv(1, 2, 3, 1, 1)], 2), &style()).summary.content_hash;
        let moved = prepare(&view(vec![sv(1, 2, 4, 1, 0)], 2), &style()).summary.content_hash;
        assert_eq!(a, same);
        assert_ne!(a, older);
        assert_ne!(a, moved);
    }

    #[test]
    fn auto_age_scale_uses_the_oldest_voxel() {
        let voxels = vec![sv(0, 0, 0, 1, 40), sv(1, 0, 0, 1, 0)];
        let auto = MeshStyle { color_mode: ColorMode::Age, age_scale: AgeScale::Auto, ..style() };
        let prepared = prepare(&view(voxels, 2), &auto);
        assert_eq!(prepared.summary.age_top, 40);
        let oldest = crate::colors::decode_index(prepared.voxels[0].state).expect("index").1;
        assert_eq!(oldest, crate::colors::COLORS - 1);
    }

    #[test]
    fn build_meshes_only_changed_chunks() {
        let job = |voxels: Vec<SurfaceVoxel>, mesher: ChunkedVoxelMesher| BuildJob {
            view: Arc::new(VoxelView::Surface(view(voxels, 2))),
            seq: 1,
            style: style(),
            mesher,
            palette: None,
        };
        let first = run_build(job(vec![sv(0, 0, 0, 1, 0), sv(20, 0, 0, 1, 0)], new_mesher()));
        assert!(first.error.is_none());
        assert_eq!(first.updated.len(), 2, "two chunks");
        assert!(first.updated.iter().all(|u| u.opaque.is_some() && u.translucent.is_none() && u.faces == 6));
        // Moving the far voxel re-meshes only its chunk.
        let second = run_build(job(vec![sv(0, 0, 0, 1, 0), sv(21, 0, 0, 1, 0)], first.mesher));
        assert_eq!(second.updated.len(), 1);
        assert_eq!(second.updated[0].chunk, IVec3::new(1, 0, 0));
        // Removing it removes the chunk.
        let third = run_build(job(vec![sv(0, 0, 0, 1, 0)], second.mesher));
        assert_eq!(third.removed, vec![IVec3::new(1, 0, 0)]);
    }
}
