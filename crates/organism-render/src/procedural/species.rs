//! Data-driven species table for procedural organisms: an id, aliases (the
//! ecosystem ids it can stand in for), adult mass, and a body plan with
//! species-specific proportions and colours. Every animal species listed in
//! the model manifest has an entry here as well, used as its far-LOD
//! stand-in; species without a licensed GLB (bear, big cats, elephant,
//! birds, insects, turtle, lizard...) are drawn only from this table.
//!
//! Dimensions are adult reference values in metres (rounded from standard
//! field-guide figures); they are visual parameters, not simulation data.

use std::collections::BTreeMap;

use super::aquatic::{FishParams, SerpentParams, build_fish, build_serpent};
use super::arthropod::{ArthropodParams, WingRest, build as build_arthropod};
use super::bird::{BirdParams, build as build_bird};
use super::herp::{FrogParams, SnailParams, TurtleParams, build_frog, build_snail, build_turtle};
use super::parts::Detail;
use super::quadruped::{QuadrupedParams, build as build_quadruped};
use super::rig::Rig;

/// Largest linear dimension (m) accepted for a registered species.
pub const MAX_DIMENSION_M: f32 = 100.0;

/// Why a species specification cannot be drawn.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum SpeciesError {
    /// The id is empty.
    #[error("species id is empty")]
    EmptyId,
    /// The mass is not a positive finite number.
    #[error("species `{id}`: mass must be a positive number of kilograms, found {value}")]
    InvalidMass {
        /// Species id.
        id: String,
        /// Value found.
        value: f32,
    },
    /// A body dimension is out of range.
    #[error("species `{id}`: `{field}` must be {expected}, found {value}")]
    InvalidDimension {
        /// Species id.
        id: String,
        /// Parameter name.
        field: &'static str,
        /// What the parameter must be.
        expected: &'static str,
        /// Value found.
        value: f32,
    },
    /// The parameters produce unusable geometry (non-finite or empty).
    #[error("species `{id}`: {problem}")]
    Degenerate {
        /// Species id.
        id: String,
        /// Description of the problem.
        problem: String,
    },
}

/// Body plan with its parameters.
#[derive(Clone, Debug, PartialEq)]
pub enum BodyPlan {
    /// Mammals, lizards, crocodiles.
    Quadruped(QuadrupedParams),
    /// Birds and bats.
    Bird(BirdParams),
    /// Insects and spiders.
    Arthropod(ArthropodParams, WingRest),
    /// Fish, cetaceans, rays.
    Fish(FishParams),
    /// Snakes and worms.
    Serpent(SerpentParams),
    /// Frogs and toads.
    Frog(FrogParams),
    /// Turtles and tortoises.
    Turtle(TurtleParams),
    /// Snails.
    Snail(SnailParams),
}

/// One procedural species.
#[derive(Clone, Debug, PartialEq)]
pub struct SpeciesSpec {
    /// Species id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Other ecosystem ids this entry represents.
    pub aliases: Vec<String>,
    /// Adult mass (kg).
    pub mass_kg: f32,
    /// Body plan.
    pub plan: BodyPlan,
}

impl SpeciesSpec {
    /// Creates a spec.
    pub fn new(id: &str, name: &str, aliases: &[&str], mass_kg: f32, plan: BodyPlan) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            aliases: aliases.iter().map(|s| s.to_string()).collect(),
            mass_kg,
            plan,
        }
    }

    /// Stable seed derived from the id (FNV-1a), used for pattern noise.
    pub fn seed(&self) -> u32 {
        self.id.bytes().fold(0x811c_9dc5u32, |h, b| (h ^ b as u32).wrapping_mul(0x0100_0193))
    }

    /// Wing posture at rest for arthropods.
    pub fn wing_rest(&self) -> Option<WingRest> {
        match &self.plan {
            BodyPlan::Arthropod(_, rest) => Some(*rest),
            _ => None,
        }
    }

    /// Checks that the specification can be drawn: a non-empty id, a
    /// positive mass, positive finite body dimensions (at most
    /// [`MAX_DIMENSION_M`]), bounded counts, and finite, non-empty geometry
    /// once built. Genome-derived species should be validated before they
    /// are registered.
    pub fn validate(&self) -> Result<(), SpeciesError> {
        if self.id.trim().is_empty() {
            return Err(SpeciesError::EmptyId);
        }
        if !(self.mass_kg.is_finite() && self.mass_kg > 0.0) {
            return Err(SpeciesError::InvalidMass { id: self.id.clone(), value: self.mass_kg });
        }
        let dim = |field: &'static str, value: f32, allow_zero: bool| -> Result<(), SpeciesError> {
            let ok =
                value.is_finite() && value <= MAX_DIMENSION_M && if allow_zero { value >= 0.0 } else { value > 0.0 };
            if ok {
                Ok(())
            } else {
                Err(SpeciesError::InvalidDimension {
                    id: self.id.clone(),
                    field,
                    expected: if allow_zero {
                        "a finite length in [0, 100] m"
                    } else {
                        "a finite length in (0, 100] m"
                    },
                    value,
                })
            }
        };
        let count = |field: &'static str, value: f32, lo: f32, hi: f32| -> Result<(), SpeciesError> {
            if value.is_finite() && (lo..=hi).contains(&value) {
                Ok(())
            } else {
                Err(SpeciesError::InvalidDimension {
                    id: self.id.clone(),
                    field,
                    expected: "within its allowed range",
                    value,
                })
            }
        };
        match &self.plan {
            BodyPlan::Quadruped(p) => {
                for (f, v) in [
                    ("body_length", p.body_length),
                    ("body_width", p.body_width),
                    ("body_depth", p.body_depth),
                    ("leg_length", p.leg_length),
                    ("leg_radius", p.leg_radius),
                    ("neck_radius", p.neck_radius),
                    ("head_length", p.head_length),
                    ("head_width", p.head_width),
                    ("head_depth", p.head_depth),
                    ("rump", p.rump),
                    ("fore_ratio", p.fore_ratio),
                    ("foot_ratio", p.foot_ratio),
                    ("snout_width", p.snout_width),
                ] {
                    dim(f, v, false)?;
                }
                for (f, v) in [
                    ("neck_length", p.neck_length),
                    ("ear_size", p.ear_size),
                    ("tail_length", p.tail_length),
                    ("tail_radius", p.tail_radius),
                    ("trunk", p.trunk),
                    ("tusks", p.tusks),
                    ("hump", p.hump),
                    ("belly", p.belly),
                    ("snout", p.snout),
                ] {
                    dim(f, v, true)?;
                }
                for (f, v) in [("neck_angle", p.neck_angle), ("head_pitch", p.head_pitch), ("tail_droop", p.tail_droop)]
                {
                    count(f, v, -std::f32::consts::PI, std::f32::consts::PI)?;
                }
            }
            BodyPlan::Bird(p) => {
                for (f, v) in [
                    ("body_length", p.body_length),
                    ("body_width", p.body_width),
                    ("body_depth", p.body_depth),
                    ("head_radius", p.head_radius),
                    ("leg_length", p.leg_length),
                    ("span", p.span),
                    ("wing_area", p.wing_area),
                ] {
                    dim(f, v, false)?;
                }
                // Bats have no beak; tails and necks may be vestigial.
                for (f, v) in [
                    ("beak_length", p.beak_length),
                    ("beak_depth", p.beak_depth),
                    ("neck_length", p.neck_length),
                    ("tail_length", p.tail_length),
                ] {
                    dim(f, v, true)?;
                }
            }
            BodyPlan::Arthropod(p, _) => {
                for (f, v) in [("head", p.head), ("thorax", p.thorax), ("abdomen", p.abdomen)] {
                    dim(f, v.min_element(), false)?;
                    dim(f, v.max_element(), false)?;
                }
                dim("leg_length", p.leg_length, false)?;
                dim("leg_radius", p.leg_radius, false)?;
                dim("waist", p.waist, true)?;
                dim("mandibles", p.mandibles, true)?;
                count("leg_pairs", f32::from(p.leg_pairs), 1.0, 7.0)?;
                count("hind_legs", p.hind_legs, 0.2, 5.0)?;
                count("flap_hz", p.flap_hz, 0.0, 1000.0)?;
            }
            BodyPlan::Fish(p) => {
                for (f, v) in [("length", p.length), ("depth", p.depth), ("width", p.width)] {
                    dim(f, v, false)?;
                }
                count("segments", f32::from(p.segments), 1.0, 32.0)?;
                count("depth_at", p.depth_at, 0.0, 1.0)?;
            }
            BodyPlan::Serpent(p) => {
                for (f, v) in [
                    ("length", p.length),
                    ("radius", p.radius),
                    ("head_length", p.head_length),
                    ("head_width", p.head_width),
                ] {
                    dim(f, v, false)?;
                }
                count("segments", f32::from(p.segments), 1.0, 64.0)?;
            }
            BodyPlan::Frog(p) => {
                for (f, v) in [("length", p.length), ("width", p.width), ("depth", p.depth), ("hind_leg", p.hind_leg)] {
                    dim(f, v, false)?;
                }
            }
            BodyPlan::Turtle(p) => {
                for (f, v) in [
                    ("shell_length", p.shell_length),
                    ("shell_width", p.shell_width),
                    ("shell_height", p.shell_height),
                    ("head", p.head),
                    ("neck", p.neck),
                    ("leg", p.leg),
                ] {
                    dim(f, v, false)?;
                }
            }
            BodyPlan::Snail(p) => {
                dim("foot_length", p.foot_length, false)?;
                dim("shell_radius", p.shell_radius, false)?;
                count("whorls", p.whorls, 1.0, 8.0)?;
            }
        }
        let rig = self.build(Detail::Low);
        let degenerate = |problem: String| SpeciesError::Degenerate { id: self.id.clone(), problem };
        rig.validate().map_err(degenerate)?;
        let finite = rig.parts.iter().all(|p| p.mesh.positions.iter().flatten().all(|v| v.is_finite()));
        if !finite {
            return Err(degenerate("parameters produce non-finite geometry".into()));
        }
        match rig.rest_bounds() {
            Some((lo, hi)) if (hi - lo).max_element() > 0.0 => Ok(()),
            _ => Err(degenerate("parameters produce no geometry".into())),
        }
    }

    /// Builds the rig at the given detail. Terrestrial rigs are grounded so
    /// their feet touch `y = 0`; aquatic rigs stay centred on their axis.
    pub fn build(&self, detail: Detail) -> Rig {
        let seed = self.seed();
        let m = self.mass_kg;
        let mut rig = match &self.plan {
            BodyPlan::Quadruped(p) => build_quadruped(p, m, seed, detail),
            BodyPlan::Bird(p) => build_bird(p, m, seed, detail),
            BodyPlan::Arthropod(p, _) => build_arthropod(p, m, seed, detail),
            BodyPlan::Fish(p) => build_fish(p, m, seed, detail),
            BodyPlan::Serpent(p) => build_serpent(p, m, seed, detail),
            BodyPlan::Frog(p) => build_frog(p, m, seed, detail),
            BodyPlan::Turtle(p) => build_turtle(p, m, seed, detail),
            BodyPlan::Snail(p) => build_snail(p, m, detail),
        };
        if !rig.motion.aquatic {
            rig.ground();
        }
        rig
    }
}

/// All procedural species indexed by id and alias.
///
/// Every insertion stamps the entry with a new revision number, so caches
/// and spawned visuals can tell when a species was redefined (e.g. a
/// genome-derived organism whose parameters changed).
#[derive(Clone, Debug)]
pub struct SpeciesTable {
    specs: Vec<SpeciesSpec>,
    revisions: Vec<u64>,
    next_revision: u64,
    index: BTreeMap<String, usize>,
}

impl Default for SpeciesTable {
    fn default() -> Self {
        Self::builtin()
    }
}

impl SpeciesTable {
    /// The built-in table.
    pub fn builtin() -> Self {
        let mut t = Self { specs: Vec::new(), revisions: Vec::new(), next_revision: 1, index: BTreeMap::new() };
        for s in super::species_mammals::builtin_species() {
            t.insert(s);
        }
        t
    }

    /// Adds or replaces a species (e.g. a genome-derived evolution-lab
    /// organism). Ids and aliases of the new entry take precedence. The
    /// spec is not validated here; see [`SpeciesSpec::validate`].
    pub fn insert(&mut self, spec: SpeciesSpec) {
        let revision = self.next_revision;
        self.next_revision += 1;
        let idx = match self.index.get(&spec.id) {
            Some(&i) if self.specs[i].id == spec.id => {
                self.specs[i] = spec.clone();
                self.revisions[i] = revision;
                i
            }
            _ => {
                self.specs.push(spec.clone());
                self.revisions.push(revision);
                self.specs.len() - 1
            }
        };
        self.index.insert(spec.id.clone(), idx);
        for a in &spec.aliases {
            self.index.insert(a.clone(), idx);
        }
    }

    /// Looks up a species by id or alias.
    pub fn get(&self, id: &str) -> Option<&SpeciesSpec> {
        self.index.get(id).map(|&i| &self.specs[i])
    }

    /// Revision of the entry `id` (or alias) resolves to; it changes every
    /// time that entry is replaced.
    pub fn revision(&self, id: &str) -> Option<u64> {
        self.index.get(id).map(|&i| self.revisions[i])
    }

    /// All species in insertion order.
    pub fn all(&self) -> &[SpeciesSpec] {
        &self.specs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::procedural::rig::Locomotor;

    /// Lowest rest-pose point of the rig and the role of the part it belongs to.
    fn lowest(rig: &Rig) -> (f32, crate::procedural::rig::PartRole) {
        let mut best = (f32::MAX, crate::procedural::rig::PartRole::Root);
        for (i, part) in rig.parts.iter().enumerate() {
            let t = rig.rest_model_transform(i);
            for p in &part.mesh.positions {
                let y = t.transform_point(bevy::math::Vec3::from(*p)).y;
                if y < best.0 {
                    best = (y, part.role);
                }
            }
        }
        best
    }

    #[test]
    fn every_species_builds_valid_rigs_at_both_details() {
        let table = SpeciesTable::builtin();
        assert!(table.all().len() >= 60, "{}", table.all().len());
        for spec in table.all() {
            for detail in [Detail::Full, Detail::Low] {
                let rig = spec.build(detail);
                rig.validate().unwrap_or_else(|e| panic!("{}: {e}", spec.id));
                let (lo, hi) = rig.rest_bounds().unwrap_or_else(|| panic!("{} has no geometry", spec.id));
                assert!(lo.is_finite() && hi.is_finite(), "{}", spec.id);
                if !rig.motion.aquatic {
                    // Feet are on the ground; nothing else sinks noticeably into it.
                    let (y, role) = lowest(&rig);
                    assert!(
                        y > -0.03 * (hi.y - lo.y).max(0.01),
                        "{} {detail:?}: {role:?} at {y} below ground",
                        spec.id
                    );
                }
                assert!(rig.triangle_count() < 40_000, "{} too heavy: {}", spec.id, rig.triangle_count());
                if detail == Detail::Low {
                    assert!(rig.triangle_count() < 2_500, "{} far LOD too heavy: {}", spec.id, rig.triangle_count());
                }
            }
        }
    }

    #[test]
    fn ids_and_aliases_resolve() {
        let t = SpeciesTable::builtin();
        assert_eq!(t.get("cattle").map(|s| s.id.as_str()), Some("cow"));
        assert_eq!(t.get("bee_proxy").map(|s| s.id.as_str()), Some("bee"));
        assert!(t.get("unicorn").is_none());
        let elephant = t.get("elephant").unwrap().build(Detail::Full);
        assert!(elephant.parts.iter().any(|p| matches!(p.role, crate::procedural::rig::PartRole::Trunk(_))));
        assert_eq!(t.get("spider").unwrap().build(Detail::Low).motion.locomotor, Locomotor::Octopod);
    }

    #[test]
    fn elephant_ears_hang_beside_the_head() {
        use crate::procedural::rig::{PartRole, Side};
        let rig = SpeciesTable::builtin().get("elephant").unwrap().build(Detail::Full);
        let head = rig.find(PartRole::Head).unwrap();
        let head_top = rig.rest_model_transform(head).translation.y;
        for side in Side::BOTH {
            let ear = rig.find(PartRole::Ear(side)).unwrap();
            let t = rig.rest_model_transform(ear);
            let lowest = rig.parts[ear]
                .mesh
                .positions
                .iter()
                .map(|p| t.transform_point(bevy::math::Vec3::from(*p)).y)
                .fold(f32::MAX, f32::min);
            let highest = rig.parts[ear]
                .mesh
                .positions
                .iter()
                .map(|p| t.transform_point(bevy::math::Vec3::from(*p)).y)
                .fold(f32::MIN, f32::max);
            assert!(lowest < head_top - 0.6, "{side:?} ear bottom {lowest} vs head {head_top}");
            assert!(highest < head_top + 0.8, "{side:?} ear top {highest} vs head {head_top}");
        }
    }

    #[test]
    fn custom_species_can_be_registered() {
        let mut t = SpeciesTable::builtin();
        let mut spec = t.get("fox").unwrap().clone();
        spec.id = "evolved_17".into();
        spec.aliases = vec!["lineage_17".into()];
        t.insert(spec.clone());
        assert_eq!(t.get("lineage_17").map(|s| s.id.as_str()), Some("evolved_17"));
        assert_eq!(t.get("fox").map(|s| s.id.as_str()), Some("fox"));
        // Redefining a species bumps its revision (caches must rebuild).
        let before = t.revision("evolved_17").unwrap();
        spec.mass_kg *= 2.0;
        t.insert(spec);
        let after = t.revision("lineage_17").unwrap();
        assert!(after > before);
        assert_eq!(t.revision("fox"), SpeciesTable::builtin().revision("fox"));
    }

    #[test]
    fn every_builtin_species_validates() {
        for spec in SpeciesTable::builtin().all() {
            spec.validate().unwrap_or_else(|e| panic!("{e}"));
        }
    }

    #[test]
    fn invalid_specs_are_rejected_with_typed_errors() {
        let t = SpeciesTable::builtin();
        let mut fox = t.get("fox").unwrap().clone();
        fox.mass_kg = f32::NAN;
        assert!(matches!(fox.validate(), Err(SpeciesError::InvalidMass { .. })));
        let mut fox = t.get("fox").unwrap().clone();
        if let BodyPlan::Quadruped(p) = &mut fox.plan {
            p.leg_length = -0.3;
        }
        let err = fox.validate().unwrap_err();
        assert!(err.to_string().contains("leg_length"), "{err}");
        let mut snail = t.get("snail").unwrap().clone();
        if let BodyPlan::Snail(p) = &mut snail.plan {
            p.whorls = f32::INFINITY;
        }
        assert!(matches!(snail.validate(), Err(SpeciesError::InvalidDimension { field: "whorls", .. })));
        let mut bee = t.get("bee").unwrap().clone();
        if let BodyPlan::Arthropod(p, _) = &mut bee.plan {
            p.thorax.x = f32::NAN;
        }
        assert!(bee.validate().is_err());
        let mut unnamed = t.get("trout").unwrap().clone();
        unnamed.id = " ".into();
        assert_eq!(unnamed.validate(), Err(SpeciesError::EmptyId));
    }
}
