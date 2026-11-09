//! Bevy runtime for procedural organisms: shared mesh assets per species
//! and detail, entity hierarchies (one entity per rig part), and the
//! per-frame animation system with action crossfades.

use std::collections::BTreeMap;
use std::sync::Arc;

use bevy::prelude::*;

use super::arthropod::WingRest;
use super::mesh::Rgba;
use super::parts::Detail;
use super::pose::{
    CROSSFADE_SECONDS, PoseInput, cycle_frequency, default_speed, display_flap_hz, ease, effective_action, part_offset,
};
use super::rig::{PartMaterial, PartRole, Rig, RigMotion};
use super::species::SpeciesSpec;
use crate::action::OrganismAction;
use crate::lod::{LodLevel, OrganismLod};
use crate::materials::OrganismMaterials;
use crate::organism::AnimatedOrganism;

/// One part of a rig as shared GPU assets.
#[derive(Clone, Debug)]
pub struct RigPartAsset {
    /// Animation role.
    pub role: PartRole,
    /// Parent part index.
    pub parent: Option<usize>,
    /// Rest transform relative to the parent.
    pub rest: Transform,
    /// Mesh, if the part has geometry.
    pub mesh: Option<Handle<Mesh>>,
    /// Surface finish.
    pub material: PartMaterial,
}

/// Shared assets of one species at one level of detail.
#[derive(Clone, Debug)]
pub struct RigAssets {
    /// Species id.
    pub species: String,
    /// Parts in parent-before-child order.
    pub parts: Vec<RigPartAsset>,
    /// Rest-pose meshes merged per material (static far LOD).
    pub merged: Vec<(Handle<Mesh>, PartMaterial)>,
    /// Animation dimensions.
    pub motion: Arc<RigMotion>,
    /// Arthropod wing posture at rest.
    pub wing_rest: WingRest,
    /// Marker colour.
    pub marker: Rgba,
    /// Largest rest-pose extent (m), used for LOD distances.
    pub extent: f32,
}

impl RigAssets {
    /// Uploads a rig's meshes.
    pub fn from_rig(species: &str, rig: &Rig, wing_rest: WingRest, meshes: &mut Assets<Mesh>) -> Self {
        let parts = rig
            .parts
            .iter()
            .map(|p| RigPartAsset {
                role: p.role,
                parent: p.parent,
                rest: p.rest,
                mesh: (!p.mesh.is_empty()).then(|| meshes.add(p.mesh.to_mesh())),
                material: p.material,
            })
            .collect();
        let merged =
            rig.merged_by_material().into_iter().map(|(mat, mesh)| (meshes.add(mesh.to_mesh()), mat)).collect();
        let extent = rig.rest_bounds().map_or(1.0, |(lo, hi)| (hi - lo).max_element());
        Self {
            species: species.to_string(),
            parts,
            merged,
            motion: Arc::new(rig.motion.clone()),
            wing_rest,
            marker: rig.marker_color,
            extent,
        }
    }
}

/// Cache of generated species assets, keyed by (species id, detail) and
/// tagged with the species revision they were built from.
#[derive(Resource, Default)]
pub struct ProceduralAssetCache {
    map: BTreeMap<(String, Detail), (u64, Arc<RigAssets>)>,
}

impl ProceduralAssetCache {
    /// Returns the assets for `spec` (at table revision `revision`, see
    /// [`super::species::SpeciesTable::revision`]) at `detail`, generating
    /// them on first use and regenerating them when the species was
    /// redefined. Replaced meshes are released once no visual uses them.
    pub fn get_or_build(
        &mut self,
        spec: &SpeciesSpec,
        revision: u64,
        detail: Detail,
        meshes: &mut Assets<Mesh>,
    ) -> Arc<RigAssets> {
        let key = (spec.id.clone(), detail);
        if let Some((built, a)) = self.map.get(&key)
            && *built == revision
        {
            return a.clone();
        }
        let rig = spec.build(detail);
        let assets =
            Arc::new(RigAssets::from_rig(&spec.id, &rig, spec.wing_rest().unwrap_or(WingRest::FoldedBack), meshes));
        self.map.insert(key, (revision, assets.clone()));
        assets
    }

    /// Number of cached species/detail combinations.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether nothing has been generated yet.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// Root of an animated procedural organism (also the root part).
#[derive(Component, Clone, Debug)]
pub struct ProceduralRig {
    /// Organism entity this rig draws.
    pub owner: Entity,
    /// Shared assets.
    pub assets: Arc<RigAssets>,
    /// Part entities, index-aligned with `assets.parts`.
    pub parts: Vec<Entity>,
}

/// Marks a rig part entity.
#[derive(Component, Clone, Copy, Debug)]
pub struct ProceduralPart {
    /// Index in [`RigAssets::parts`].
    pub index: usize,
}

/// Animation state of a procedural rig.
#[derive(Component, Clone, Debug)]
pub struct ProceduralMotionState {
    /// Action being shown.
    pub action: OrganismAction,
    /// Action being faded out.
    pub previous: OrganismAction,
    /// Seconds since `action` started.
    pub t: f32,
    /// Seconds since `previous` started.
    pub previous_t: f32,
    /// Crossfade progress (1 = done).
    pub fade: f32,
    /// Locomotion cycle (cycles).
    pub cycle: f32,
    /// Wing-beat cycle (cycles).
    pub flap: f32,
    /// Per-instance clock offset (s), so herds are not synchronised.
    pub clock_offset: f32,
}

impl ProceduralMotionState {
    /// Fresh state; `variation` desynchronises instances.
    pub fn new(variation: u64) -> Self {
        let offset = ((variation.wrapping_mul(0x9e37_79b9_7f4a_7c15) >> 40) as f32) / (1u64 << 24) as f32;
        Self {
            action: OrganismAction::Idle,
            previous: OrganismAction::Idle,
            t: 10.0,
            previous_t: 10.0,
            fade: 1.0,
            cycle: offset,
            flap: offset,
            clock_offset: 100.0 * offset,
        }
    }
}

/// Spawns the animated hierarchy of `assets` under `parent`; returns the
/// root entity.
pub fn spawn_rig(
    commands: &mut Commands,
    parent: Entity,
    owner: Entity,
    assets: Arc<RigAssets>,
    materials: &OrganismMaterials,
    variation: u64,
) -> Entity {
    let mut entities: Vec<Entity> = Vec::with_capacity(assets.parts.len());
    for (i, part) in assets.parts.iter().enumerate() {
        let parent_entity = part.parent.map_or(parent, |p| entities[p]);
        let mut e =
            commands.spawn((part.rest, Visibility::default(), ProceduralPart { index: i }, ChildOf(parent_entity)));
        if let Some(mesh) = &part.mesh {
            e.insert((Mesh3d(mesh.clone()), MeshMaterial3d(materials.for_part(part.material))));
        }
        entities.push(e.id());
    }
    let root = entities[0];
    commands
        .entity(root)
        .insert((ProceduralRig { owner, assets, parts: entities }, ProceduralMotionState::new(variation)));
    root
}

/// Spawns the static merged far-LOD version of `assets` under `parent`.
pub fn spawn_static(commands: &mut Commands, parent: Entity, assets: &RigAssets, materials: &OrganismMaterials) {
    for (mesh, mat) in &assets.merged {
        commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(materials.for_part(*mat)),
            Transform::IDENTITY,
            ChildOf(parent),
        ));
    }
}

fn blend(a: &Transform, b: &Transform, w: f32) -> Transform {
    Transform {
        translation: a.translation.lerp(b.translation, w),
        rotation: a.rotation.slerp(b.rotation, w),
        scale: a.scale.lerp(b.scale, w),
    }
}

/// Advances procedural animation state and poses every rig part.
/// Mid-LOD organisms advance their clocks every frame but only rewrite
/// their part transforms on their scheduled update frames.
pub fn animate_procedural_rigs(
    time: Res<Time>,
    organisms: Query<(&AnimatedOrganism, Option<&OrganismLod>)>,
    mut rigs: Query<(&ProceduralRig, &mut ProceduralMotionState)>,
    mut parts: Query<&mut Transform, With<ProceduralPart>>,
) {
    let dt = time.delta_secs();
    let now = time.elapsed_secs();
    for (rig, mut state) in &mut rigs {
        let Ok((org, lod)) = organisms.get(rig.owner) else { continue };
        let motion = &rig.assets.motion;
        let size = org.effective_age_scale();
        let action = effective_action(motion, org.action);
        if action != state.action {
            state.previous = state.action;
            state.previous_t = state.t;
            state.action = action;
            state.t = 0.0;
            state.fade = 0.0;
        }
        let locomotion = matches!(action, OrganismAction::Walk | OrganismAction::Run | OrganismAction::Swim);
        let speed = if org.effective_speed() > 0.0 {
            org.effective_speed()
        } else if locomotion {
            default_speed(motion, action, size)
        } else {
            0.0
        };
        state.cycle += cycle_frequency(motion, action, speed, size) * dt;
        state.flap += display_flap_hz(motion)
            * dt
            * if motion.aquatic { (speed / (motion.body_length * size).max(1e-3)).clamp(0.5, 2.0) } else { 1.0 };
        state.t += dt;
        state.previous_t += dt;
        state.fade = (state.fade + dt / CROSSFADE_SECONDS).min(1.0);
        if lod.is_some_and(|l| l.level == LodLevel::Mid && !l.update_this_frame) {
            continue;
        }
        let input = PoseInput {
            action,
            speed,
            cycle: state.cycle,
            flap: state.flap,
            t: state.t,
            clock: now + state.clock_offset,
            size,
            wing_rest: rig.assets.wing_rest,
        };
        let prev_input = PoseInput { action: state.previous, t: state.previous_t, ..input };
        let w = ease(state.fade);
        for (part, &entity) in rig.assets.parts.iter().zip(&rig.parts) {
            let Ok(mut transform) = parts.get_mut(entity) else { continue };
            let current = part_offset(motion, part.role, &part.rest, &input).apply(&part.rest);
            *transform = if w < 1.0 {
                let previous = part_offset(motion, part.role, &part.rest, &prev_input).apply(&part.rest);
                blend(&previous, &current, w)
            } else {
                current
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::procedural::species::SpeciesTable;

    #[test]
    fn assets_are_cached_per_species_and_detail() {
        let mut meshes = Assets::<Mesh>::default();
        let mut cache = ProceduralAssetCache::default();
        let table = SpeciesTable::builtin();
        let fox = table.get("fox").unwrap();
        let a = cache.get_or_build(fox, 1, Detail::Full, &mut meshes);
        let b = cache.get_or_build(fox, 1, Detail::Full, &mut meshes);
        assert!(Arc::ptr_eq(&a, &b));
        let low = cache.get_or_build(fox, 1, Detail::Low, &mut meshes);
        assert_eq!(cache.len(), 2);
        // A redefined species (new revision) is rebuilt, not served stale.
        let mut bigger = fox.clone();
        if let crate::procedural::species::BodyPlan::Quadruped(p) = &mut bigger.plan {
            p.body_length *= 2.0;
        }
        let c = cache.get_or_build(&bigger, 2, Detail::Full, &mut meshes);
        assert!(!Arc::ptr_eq(&a, &c));
        assert!(c.extent > a.extent);
        assert_eq!(cache.len(), 2);
        assert!(!low.merged.is_empty());
        assert!(a.parts.len() > low.parts.len());
        assert!((0.8..2.0).contains(&a.extent), "{}", a.extent);
    }

    #[test]
    fn motion_states_are_desynchronised() {
        let a = ProceduralMotionState::new(1);
        let b = ProceduralMotionState::new(2);
        assert_ne!(a.cycle, b.cycle);
        assert!((0.0..1.0).contains(&a.cycle));
    }
}
