//! # organism-render
//!
//! Bevy visuals for living things in the Ecosystem, Evolution and
//! Civilization views: recognisable, animated animals and plants. Animals
//! are never drawn as cubes, spheres or blobs; every species has either an
//! authored glTF model or a procedural body plan with its silhouette cues.
//!
//! * [`AnimatedOrganism`] — attach to an organism entity (position/heading
//!   from the simulation, facing +Z) with a species id, an
//!   [`OrganismAction`], the movement speed (m/s) and an age scale.
//! * [`library`] / [`manifest`] — `assets/models/manifest.toml`: species →
//!   GLB model, clip names per action, scale/yaw, and the per-action
//!   fallback plan (authored clip, substitute clip or procedural overlay).
//! * [`glb`] — glTF scenes with a shared `AnimationGraph` per model,
//!   `AnimationTransitions` crossfades (0.25 s) and stride-matched playback.
//! * [`procedural`] — parametric body plans (quadrupeds, birds/bats,
//!   insects/spiders, fish/cetaceans/rays, snakes/worms, frogs, turtles,
//!   snails) built from lofted superellipse sections, articulated as child
//!   entities and animated procedurally (Froude-number gaits, tripod gaits,
//!   Pennycuick wing beats, travelling-wave undulation, hops, eat, attack,
//!   rest, death).
//! * [`plants`] — procedural trees with growth stages, ground cover with
//!   wind sway, seasonal tint and burnt state, and manifest GLB plants.
//! * [`lod`] — visual level of detail (near / mid / far / marker); never
//!   feeds back into the simulation.
//!
//! Approximations are documented where they are made (display-capped wing
//! beats, clip cadence assumptions, rigid-segment bodies).

#![warn(missing_docs)]

pub mod action;
pub mod catalog;
pub mod glb;
pub mod glb_overlay;
pub mod library;
pub mod lod;
pub mod manifest;
pub mod materials;
pub mod organism;
pub mod plants;
pub mod procedural;

use std::path::PathBuf;

use bevy::app::AnimationSystems;
use bevy::prelude::*;

pub use action::OrganismAction;
pub use catalog::OrganismCatalog;
pub use lod::{LodLevel, LodSettings, OrganismLod};
pub use organism::{AnimatedOrganism, OrganismRenderDiagnostics};
pub use plants::{PlantSeason, ProceduralTree, TreeStyle, Wind};

/// System sets of the plugin, in execution order within `Update`.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum OrganismSystems {
    /// Level-of-detail classification.
    Lod,
    /// Spawning / replacing visuals.
    Visuals,
    /// Clip control, procedural animation, plants.
    Animate,
}

/// Registers organism and plant rendering.
///
/// `assets_root` is the directory that contains `models/manifest.toml`
/// (the same directory Bevy's `AssetPlugin` serves). If the manifest cannot
/// be read, the error is logged, kept in [`OrganismCatalog::manifest_error`]
/// and every species is drawn procedurally.
#[derive(Clone, Debug)]
pub struct OrganismRenderPlugin {
    /// Filesystem path of the assets root.
    pub assets_root: PathBuf,
}

impl Plugin for OrganismRenderPlugin {
    fn build(&self, app: &mut App) {
        if !app.world().contains_resource::<OrganismCatalog>() {
            app.insert_resource(OrganismCatalog::load(&self.assets_root));
        }
        app.init_resource::<materials::OrganismMaterials>()
            .init_resource::<procedural::runtime::ProceduralAssetCache>()
            .init_resource::<glb::GlbModelAssets>()
            .init_resource::<organism::MarkerMeshes>()
            .init_resource::<OrganismRenderDiagnostics>()
            .init_resource::<LodSettings>()
            .configure_sets(Update, (OrganismSystems::Lod, OrganismSystems::Visuals, OrganismSystems::Animate).chain())
            .add_systems(Update, lod::update_organism_lod.in_set(OrganismSystems::Lod))
            .add_systems(
                Update,
                (organism::sync_organism_visuals, organism::remove_stale_visuals)
                    .chain()
                    .in_set(OrganismSystems::Visuals),
            )
            .add_systems(
                Update,
                (
                    glb::prepare_glb_models,
                    glb::attach_glb_players,
                    glb::drive_glb_animation,
                    procedural::runtime::animate_procedural_rigs,
                    lod::orient_markers,
                )
                    .chain()
                    .in_set(OrganismSystems::Animate),
            )
            .add_systems(
                PostUpdate,
                glb::apply_glb_bone_overlays.after(AnimationSystems).before(TransformSystems::Propagate),
            )
            .add_observer(glb::on_world_instance_ready);
        plants::build(app);
    }
}
