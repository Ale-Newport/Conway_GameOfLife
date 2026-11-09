//! The [`AnimatedOrganism`] component simulations attach to their organism
//! entities, and the system that keeps each organism's visual (GLB scene,
//! procedural rig, static far mesh or marker) in sync with its species,
//! age and level of detail.
//!
//! The organism entity's `Transform` is owned by the caller (position from
//! the simulation, heading as a rotation about +Y). Organisms face local
//! **+Z**; aquatic rigs are centred on their swimming axis, terrestrial ones
//! stand on `y = 0` of the organism frame.

use std::collections::{BTreeMap, BTreeSet};

use bevy::prelude::*;

use crate::action::OrganismAction;
use crate::catalog::{NearSource, OrganismCatalog};
use crate::glb::{GlbModelAssets, spawn_glb_scene};
use crate::lod::{LodLevel, LodMarker, OrganismLod};
use crate::materials::OrganismMaterials;
use crate::procedural::mesh::{MeshData, Rgba};
use crate::procedural::palette::{mix, srgb};
use crate::procedural::parts::Detail;
use crate::procedural::runtime::{ProceduralAssetCache, spawn_rig, spawn_static};

/// An organism to draw: species, visible action, movement speed and age.
#[derive(Component, Clone, Debug, PartialEq)]
#[require(Transform, Visibility, OrganismLod, OrganismVisualSlot)]
pub struct AnimatedOrganism {
    /// Species id (ecosystem id, manifest species id or procedural id/alias).
    pub species: String,
    /// What the organism is doing.
    pub action: OrganismAction,
    /// Movement speed in m/s, used to match strides and wing/tail beats;
    /// 0 lets the renderer pick a natural speed for locomotion actions.
    pub speed: f32,
    /// Body-size factor: 1 for adults, smaller for juveniles.
    pub age_scale: f32,
}

impl AnimatedOrganism {
    /// Idle adult of `species`.
    pub fn new(species: impl Into<String>) -> Self {
        Self { species: species.into(), action: OrganismAction::Idle, speed: 0.0, age_scale: 1.0 }
    }

    /// Same organism doing `action`.
    pub fn with_action(mut self, action: OrganismAction) -> Self {
        self.action = action;
        self
    }

    /// Same organism moving at `speed` m/s.
    pub fn with_speed(mut self, speed: f32) -> Self {
        self.speed = speed.max(0.0);
        self
    }

    /// Same organism at a body-size factor (juveniles < 1); non-finite
    /// values fall back to 1.
    pub fn with_age_scale(mut self, age_scale: f32) -> Self {
        self.age_scale = sanitize_age_scale(age_scale);
        self
    }

    /// Movement speed actually used for animation: the `speed` field when
    /// it is a finite positive number, else 0 (the renderer then picks a
    /// natural speed for locomotion). The fields are public, so values a
    /// simulation writes directly are sanitised here.
    pub fn effective_speed(&self) -> f32 {
        if self.speed.is_finite() && self.speed > 0.0 { self.speed } else { 0.0 }
    }

    /// Body-size factor actually drawn: `age_scale` clamped to
    /// `[MIN_AGE_SCALE, MAX_AGE_SCALE]`, 1 when it is not finite.
    pub fn effective_age_scale(&self) -> f32 {
        sanitize_age_scale(self.age_scale)
    }
}

/// Smallest drawn body-size factor.
pub const MIN_AGE_SCALE: f32 = 0.05;
/// Largest drawn body-size factor (galleries use it as a display scale).
pub const MAX_AGE_SCALE: f32 = 1000.0;

fn sanitize_age_scale(age_scale: f32) -> f32 {
    if age_scale.is_finite() { age_scale.clamp(MIN_AGE_SCALE, MAX_AGE_SCALE) } else { 1.0 }
}

/// How an organism is currently drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VisualKind {
    /// Authored glTF scene with clips.
    Glb,
    /// Animated procedural rig.
    Procedural,
    /// Static merged low-detail procedural mesh.
    StaticLow,
    /// Billboard marker.
    Marker,
}

/// The visual child of an organism (scaled by its age factor).
#[derive(Component, Clone, Copy, Debug)]
pub struct OrganismVisual {
    /// Organism entity.
    pub owner: Entity,
    /// Representation.
    pub kind: VisualKind,
}

/// Bookkeeping of an organism's current visual.
#[derive(Component, Clone, Debug, Default)]
pub struct OrganismVisualSlot {
    entity: Option<Entity>,
    kind: Option<VisualKind>,
    species: String,
    /// Procedural-species revision the visual was built from.
    revision: u64,
}

impl OrganismVisualSlot {
    /// Current visual kind, once one has been spawned.
    pub fn kind(&self) -> Option<VisualKind> {
        self.kind
    }
}

/// Problems found while drawing organisms (for UI display).
#[derive(Resource, Clone, Debug, Default)]
pub struct OrganismRenderDiagnostics {
    /// Species ids that are neither in the manifest nor in the procedural
    /// table; such organisms are shown as magenta markers.
    pub unknown_species: BTreeSet<String>,
}

/// Marker disc meshes per marker colour (species of the same colour, and
/// every revision of a redefined species, share one mesh).
#[derive(Resource, Default)]
pub struct MarkerMeshes {
    meshes: BTreeMap<[u32; 4], Handle<Mesh>>,
}

/// Colour of markers for unknown species.
pub const UNKNOWN_SPECIES_COLOR: u32 = 0xff00ff;

/// Disc of unit diameter in the XY plane, bright centre fading to a darker
/// rim so neighbouring markers stay distinguishable.
pub fn marker_disc(color: Rgba) -> MeshData {
    let mut m = MeshData::new();
    let center = m.vertex(Vec3::ZERO, Vec3::Z, mix(color, [1.0; 4], 0.25));
    let n = 20;
    let rim: Vec<u32> = (0..n)
        .map(|k| {
            let a = k as f32 / n as f32 * std::f32::consts::TAU;
            m.vertex(Vec3::new(0.5 * a.cos(), 0.5 * a.sin(), 0.0), Vec3::Z, mix(color, [0.0, 0.0, 0.0, 1.0], 0.35))
        })
        .collect();
    for k in 0..n {
        m.triangle(center, rim[k], rim[(k + 1) % n]);
    }
    m
}

/// Representation wanted for a species at a level of detail.
pub fn desired_kind(catalog: &OrganismCatalog, species: &str, level: LodLevel) -> VisualKind {
    match level {
        LodLevel::Near | LodLevel::Mid => match catalog.near_source(species) {
            Some(NearSource::Glb(_)) => VisualKind::Glb,
            Some(NearSource::Procedural(_)) => VisualKind::Procedural,
            None => VisualKind::Marker,
        },
        LodLevel::Far if catalog.procedural(species).is_some() => VisualKind::StaticLow,
        LodLevel::Far | LodLevel::Marker => VisualKind::Marker,
    }
}

/// Spawns, replaces and rescales organism visuals.
#[allow(clippy::too_many_arguments)]
pub fn sync_organism_visuals(
    mut commands: Commands,
    catalog: Res<OrganismCatalog>,
    mut cache: ResMut<ProceduralAssetCache>,
    mut glb: ResMut<GlbModelAssets>,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    materials: Res<OrganismMaterials>,
    mut markers: ResMut<MarkerMeshes>,
    mut diagnostics: ResMut<OrganismRenderDiagnostics>,
    mut organisms: Query<(Entity, &AnimatedOrganism, &OrganismLod, &mut OrganismVisualSlot)>,
    mut visuals: Query<&mut Transform, With<OrganismVisual>>,
) {
    for (owner, org, lod, mut slot) in &mut organisms {
        let kind = desired_kind(&catalog, &org.species, lod.level);
        let age = org.effective_age_scale();
        // GLB visuals do not depend on the procedural table.
        // Revisions only change when the catalog does, so the lookup is
        // skipped for unchanged organisms in steady state.
        let revision = if kind == VisualKind::Glb {
            0
        } else if catalog.is_changed() || slot.kind != Some(kind) || slot.species != org.species {
            catalog.procedural_revision(&org.species)
        } else {
            slot.revision
        };
        if slot.kind == Some(kind) && slot.species == org.species && slot.revision == revision {
            if let Some(e) = slot.entity
                && let Ok(mut t) = visuals.get_mut(e)
                && (t.scale.x - age).abs() > 1e-4
            {
                t.scale = Vec3::splat(age);
            }
            continue;
        }
        if let Some(old) = slot.entity.take() {
            commands.entity(old).despawn();
        }
        let visual = commands
            .spawn((
                OrganismVisual { owner, kind },
                Transform::from_scale(Vec3::splat(age)),
                Visibility::default(),
                ChildOf(owner),
            ))
            .id();
        let variation = owner.to_bits();
        match (kind, catalog.near_source(&org.species)) {
            (VisualKind::Glb, Some(NearSource::Glb(model))) => {
                spawn_glb_scene(&mut commands, visual, owner, model, &mut glb, &asset_server);
            }
            (VisualKind::Procedural, Some(NearSource::Procedural(spec))) => {
                let assets = cache.get_or_build(spec, revision, Detail::Full, &mut meshes);
                spawn_rig(&mut commands, visual, owner, assets, &materials, variation);
            }
            (VisualKind::StaticLow, _) => {
                if let Some(spec) = catalog.procedural(&org.species) {
                    let assets = cache.get_or_build(spec, revision, Detail::Low, &mut meshes);
                    spawn_static(&mut commands, visual, &assets, &materials);
                }
            }
            _ => {
                let color = match catalog.procedural(&org.species) {
                    Some(spec) => cache.get_or_build(spec, revision, Detail::Low, &mut meshes).marker,
                    None => {
                        if diagnostics.unknown_species.insert(org.species.clone()) {
                            warn!("organism-render: unknown species `{}`; drawing a magenta marker", org.species);
                        }
                        srgb(UNKNOWN_SPECIES_COLOR)
                    }
                };
                let mesh = markers
                    .meshes
                    .entry(color.map(f32::to_bits))
                    .or_insert_with(|| meshes.add(marker_disc(color).to_mesh()))
                    .clone();
                commands.spawn((
                    Mesh3d(mesh),
                    MeshMaterial3d(materials.marker.clone()),
                    Transform::IDENTITY,
                    LodMarker { owner, extent: catalog.extent(&org.species) },
                    ChildOf(visual),
                ));
            }
        }
        slot.entity = Some(visual);
        slot.kind = Some(kind);
        slot.species = org.species.clone();
        slot.revision = revision;
    }
}

/// Despawns the visual of entities that stopped being organisms (the
/// `AnimatedOrganism` component was removed but the entity lives on).
pub fn remove_stale_visuals(
    mut commands: Commands,
    mut removed: RemovedComponents<AnimatedOrganism>,
    mut slots: Query<&mut OrganismVisualSlot, Without<AnimatedOrganism>>,
) {
    for entity in removed.read() {
        if let Ok(mut slot) = slots.get_mut(entity) {
            if let Some(visual) = slot.entity.take() {
                commands.entity(visual).despawn();
            }
            *slot = OrganismVisualSlot::default();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_clamps_inputs() {
        let o = AnimatedOrganism::new("fox").with_speed(-3.0).with_age_scale(1e6).with_action(OrganismAction::Run);
        assert_eq!(o.speed, 0.0);
        assert_eq!(o.age_scale, MAX_AGE_SCALE);
        assert_eq!(o.action, OrganismAction::Run);
        assert_eq!(AnimatedOrganism::new("fox").with_age_scale(f32::NAN).age_scale, 1.0);
    }

    #[test]
    fn effective_values_sanitise_public_fields() {
        let mut o = AnimatedOrganism::new("fox");
        o.speed = f32::INFINITY;
        o.age_scale = f32::NAN;
        assert_eq!(o.effective_speed(), 0.0);
        assert_eq!(o.effective_age_scale(), 1.0);
        o.speed = 2.5;
        o.age_scale = 0.0;
        assert_eq!(o.effective_speed(), 2.5);
        assert_eq!(o.effective_age_scale(), MIN_AGE_SCALE);
    }

    #[test]
    fn visuals_follow_species_redefinition_and_component_removal() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .init_resource::<OrganismCatalog>()
            .init_resource::<OrganismMaterials>()
            .init_resource::<ProceduralAssetCache>()
            .init_resource::<GlbModelAssets>()
            .init_resource::<MarkerMeshes>()
            .init_resource::<OrganismRenderDiagnostics>()
            .add_systems(Update, (sync_organism_visuals, remove_stale_visuals).chain());
        let mut spec = app.world().resource::<OrganismCatalog>().species.get("wolf").unwrap().clone();
        spec.id = "evolved".into();
        spec.aliases.clear();
        app.world_mut().resource_mut::<OrganismCatalog>().register(spec.clone()).unwrap();
        let e = app.world_mut().spawn(AnimatedOrganism::new("evolved")).id();
        app.update();
        let first = app.world().get::<OrganismVisualSlot>(e).unwrap().entity.unwrap();
        app.update();
        assert_eq!(app.world().get::<OrganismVisualSlot>(e).unwrap().entity, Some(first), "stable when unchanged");
        if let crate::procedural::species::BodyPlan::Quadruped(p) = &mut spec.plan {
            p.body_length *= 1.5;
        }
        app.world_mut().resource_mut::<OrganismCatalog>().register(spec).unwrap();
        app.update();
        let second = app.world().get::<OrganismVisualSlot>(e).unwrap().entity.unwrap();
        assert_ne!(first, second, "redefined species is redrawn");
        assert!(app.world().get_entity(first).is_err(), "old visual despawned");
        app.world_mut().entity_mut(e).remove::<AnimatedOrganism>();
        app.update();
        assert!(app.world().get_entity(second).is_err(), "visual of a removed organism is despawned");
        assert!(app.world().get::<OrganismVisualSlot>(e).unwrap().entity.is_none());
    }

    #[test]
    fn visual_kind_follows_lod_and_catalog() {
        let c = OrganismCatalog::default();
        assert_eq!(desired_kind(&c, "elephant", LodLevel::Near), VisualKind::Procedural);
        assert_eq!(desired_kind(&c, "elephant", LodLevel::Far), VisualKind::StaticLow);
        assert_eq!(desired_kind(&c, "elephant", LodLevel::Marker), VisualKind::Marker);
        assert_eq!(desired_kind(&c, "dragon", LodLevel::Near), VisualKind::Marker);
    }

    #[test]
    fn marker_disc_is_a_fan() {
        let m = marker_disc([1.0, 0.0, 0.0, 1.0]);
        assert_eq!(m.triangle_count(), 20);
    }
}
