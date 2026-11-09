//! Species → visual source resolution, combining the glTF [`ModelLibrary`]
//! with the procedural [`SpeciesTable`].
//!
//! Near and mid distances use the authored GLB when the manifest has an
//! animated model for the species, otherwise the procedural rig. Far
//! distances always use the procedural low-detail version, which exists for
//! every manifest animal species (enforced by tests).

use std::path::Path;

use bevy::prelude::*;

use crate::library::ModelLibrary;
use crate::manifest::ModelEntry;
use crate::procedural::species::{SpeciesError, SpeciesSpec, SpeciesTable};

/// How an organism is drawn up close.
#[derive(Clone, Copy, Debug)]
pub enum NearSource<'a> {
    /// Authored, animated glTF model.
    Glb(&'a ModelEntry),
    /// Procedural articulated rig.
    Procedural(&'a SpeciesSpec),
}

/// Everything known about how to draw species.
#[derive(Resource, Clone, Debug)]
pub struct OrganismCatalog {
    /// glTF models from `assets/models/manifest.toml`.
    pub library: ModelLibrary,
    /// Procedural species.
    pub species: SpeciesTable,
    /// Why the manifest could not be used, if it could not (organisms then
    /// fall back to procedural rigs).
    pub manifest_error: Option<String>,
    /// Whether authored GLBs are used for near views (false forces
    /// procedural rigs everywhere, e.g. for comparison).
    pub use_glb: bool,
}

impl Default for OrganismCatalog {
    fn default() -> Self {
        Self { library: ModelLibrary::default(), species: SpeciesTable::builtin(), manifest_error: None, use_glb: true }
    }
}

impl OrganismCatalog {
    /// Loads the manifest from `<assets_root>/models/manifest.toml`. A
    /// missing or invalid manifest is recorded in `manifest_error` and logged;
    /// procedural rigs still work.
    pub fn load(assets_root: &Path) -> Self {
        match ModelLibrary::load(assets_root) {
            Ok(library) => Self { library, ..Self::default() },
            Err(e) => {
                let message = e.to_string();
                error!("organism-render: {message}; using procedural organisms only");
                Self { manifest_error: Some(message), ..Self::default() }
            }
        }
    }

    /// Source for near / mid views.
    pub fn near_source(&self, species: &str) -> Option<NearSource<'_>> {
        if self.use_glb
            && let Some(model) = self.library.animal_model(species)
        {
            return Some(NearSource::Glb(model));
        }
        self.species.get(species).map(NearSource::Procedural)
    }

    /// Procedural spec used for far views (and near views without GLB).
    pub fn procedural(&self, species: &str) -> Option<&SpeciesSpec> {
        self.species.get(species).or_else(|| {
            // A species known only to the manifest: use the stand-in of
            // another species sharing its model.
            let model = self.library.animal_model(species)?;
            model.species.iter().find_map(|s| self.species.get(s))
        })
    }

    /// Whether the species can be drawn at all.
    pub fn knows(&self, species: &str) -> bool {
        self.near_source(species).is_some() || self.procedural(species).is_some()
    }

    /// Representative size (m) of an adult, for LOD distances.
    pub fn extent(&self, species: &str) -> f32 {
        if let Some(NearSource::Glb(m)) = self.near_source(species) {
            return m.max_extent().max(0.01);
        }
        self.procedural(species).map_or(1.0, approximate_extent)
    }

    /// Validates and registers (or replaces) a procedural species, e.g. a
    /// genome-derived evolution-lab organism. Organisms already showing the
    /// species are redrawn with the new body plan on the next frame. An
    /// invalid spec is rejected and the table is left unchanged.
    pub fn register(&mut self, spec: SpeciesSpec) -> Result<(), SpeciesError> {
        spec.validate()?;
        self.species.insert(spec);
        Ok(())
    }

    /// Revision of the procedural spec used for `species` (0 when it has
    /// none); changes whenever that spec is re-registered.
    pub fn procedural_revision(&self, species: &str) -> u64 {
        self.procedural(species).and_then(|spec| self.species.revision(&spec.id)).unwrap_or(0)
    }
}

/// Size estimate from the body-plan parameters (no mesh generation).
fn approximate_extent(spec: &SpeciesSpec) -> f32 {
    use crate::procedural::species::BodyPlan;
    let e = match &spec.plan {
        BodyPlan::Quadruped(p) => p.body_length + p.neck_length + p.head_length + p.tail_length,
        BodyPlan::Bird(p) => p.span.max(p.body_length),
        BodyPlan::Arthropod(p, _) => {
            let body = 2.0 * (p.head.z + p.thorax.z + p.abdomen.z) + p.leg_length;
            let wings = match p.wings {
                crate::procedural::arthropod::Wings::Membrane { length, .. }
                | crate::procedural::arthropod::Wings::Scaled { length, .. } => 2.0 * length,
                _ => 0.0,
            };
            body.max(wings)
        }
        BodyPlan::Fish(p) => p.length,
        BodyPlan::Serpent(p) => p.length,
        BodyPlan::Frog(p) => p.length + p.hind_leg,
        BodyPlan::Turtle(p) => p.shell_length,
        BodyPlan::Snail(p) => p.foot_length,
    };
    e.max(0.005)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_catalog_is_procedural_only() {
        let c = OrganismCatalog::default();
        assert!(matches!(c.near_source("elephant"), Some(NearSource::Procedural(_))));
        assert!(c.near_source("dragon").is_none());
        assert!(!c.knows("dragon"));
        assert!(c.extent("elephant") > 5.0 && c.extent("bee") < 0.05);
    }

    #[test]
    fn registering_validates_and_bumps_revisions() {
        let mut c = OrganismCatalog::default();
        let mut spec = c.species.get("wolf").unwrap().clone();
        spec.id = "evolved_wolf".into();
        spec.aliases.clear();
        assert_eq!(c.procedural_revision("evolved_wolf"), 0);
        c.register(spec.clone()).unwrap();
        let r1 = c.procedural_revision("evolved_wolf");
        assert!(r1 > 0);
        spec.mass_kg = 0.0;
        assert!(c.register(spec.clone()).is_err());
        assert_eq!(c.procedural_revision("evolved_wolf"), r1, "rejected spec leaves the table unchanged");
        spec.mass_kg = 50.0;
        c.register(spec).unwrap();
        assert!(c.procedural_revision("evolved_wolf") > r1);
    }

    #[test]
    fn missing_manifest_is_reported_not_fatal() {
        let c = OrganismCatalog::load(Path::new("/nonexistent/assets"));
        assert!(c.manifest_error.as_deref().is_some_and(|e| e.contains("manifest")));
        assert!(c.knows("fox"));
    }
}
