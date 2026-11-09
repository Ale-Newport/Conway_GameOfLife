//! Model library: species → glTF model lookup and the action plan that
//! decides, for every [`OrganismAction`], which authored clip to play and
//! which procedural motion to layer on top when the GLB lacks a clip.
//!
//! The fallback rules implement `docs/research/assets-3d.md` §7.3:
//!
//! | missing action | fallback |
//! |---|---|
//! | walk / run (sheep, pig) | idle clip + procedural leg gait on the `*UpLeg*`/`*LowLeg*` joints |
//! | eat / drink | idle clip + head-down nod (neck/head joints, else whole-body pitch) |
//! | rest | idle clip at half rate + lie down (lowered body, folded legs) |
//! | death | idle (or fly) clip frozen + tip over and sink; fish roll belly-up; flyers tumble |
//! | swim (land animals) | walk clip ×0.6 + body lowered to the waterline with a bob |
//! | attack | idle clip + lunge; fish: fast swim ×2.5 + lunge |
//! | jump | locomotion clip + ballistic hop |
//! | glide | fly clip frozen (rate 0) + banking glide |
//! | fly (non-flyers) | drawn as the species' fastest locomotion |

use std::collections::BTreeMap;
use std::path::Path;

use bevy::prelude::Resource;

use crate::action::{
    FROUDE_DEFAULT_RUN, FROUDE_DEFAULT_WALK, OrganismAction, speed_for_froude, stride_frequency, tailbeat_frequency,
};
use crate::manifest::{ManifestError, ModelCategory, ModelEntry, ModelManifest};

/// Relative path of the manifest inside the assets root.
pub const MANIFEST_PATH: &str = "models/manifest.toml";

/// Procedural motion layered on top of (or instead of) an authored clip.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MotionOverlay {
    /// Roll onto the side, then sink slowly (land animals' death).
    TipOverAndSink,
    /// Roll belly-up and sink with drag (fish death).
    BellyUpAndSink,
    /// Fall with gravity while tumbling (flyer death).
    Tumble,
    /// Head lowered to the ground with a feeding nod.
    HeadDip,
    /// Body lowered and legs folded, slow breathing.
    LieDown,
    /// Procedural leg swing on the rig's leg joints.
    LegGait,
    /// Forward thrust of the body.
    Lunge,
    /// Ballistic hop of the whole body.
    Hop,
    /// Body lowered to the waterline with a gentle bob.
    Waterline,
    /// Banking glide with a slow altitude oscillation.
    Glide,
}

/// How playback speed follows the simulated movement speed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RateRule {
    /// Constant rate (the manifest multiplier).
    Fixed,
    /// Legged stride frequency from dynamic similarity.
    Stride,
    /// Fish tail-beat / snake wave frequency proportional to speed.
    Undulation,
}

/// Authored clip selected for an action.
#[derive(Clone, Debug, PartialEq)]
pub struct PlannedClip {
    /// Manifest action key whose clip is used.
    pub key: &'static str,
    /// Exact clip name inside the GLB.
    pub name: String,
    /// Natural playback rate at the action's default speed (0 = frozen pose).
    pub multiplier: f32,
    /// Whether the clip loops.
    pub repeat: bool,
}

/// Whether an action is drawn from an authored clip or with procedural help.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanSource {
    /// The GLB has a clip for this action.
    Authored,
    /// A substitute clip and/or procedural motion is used.
    Fallback,
}

/// Complete recipe for drawing one action on one GLB model.
#[derive(Clone, Debug, PartialEq)]
pub struct ActionPlan {
    /// Action the plan was made for.
    pub action: OrganismAction,
    /// Clip to play, if any.
    pub clip: Option<PlannedClip>,
    /// Procedural motion layered on top.
    pub overlays: Vec<MotionOverlay>,
    /// Playback-rate rule.
    pub rate: RateRule,
    /// Authored or fallback.
    pub source: PlanSource,
}

impl ActionPlan {
    /// Whether the plan uses a given overlay.
    pub fn has(&self, overlay: MotionOverlay) -> bool {
        self.overlays.contains(&overlay)
    }

    /// Whether the plan can be drawn: it plays a clip or moves procedurally.
    pub fn is_drawable(&self) -> bool {
        self.clip.is_some() || !self.overlays.is_empty()
    }
}

/// Body dimensions used to match animation rate to movement speed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocomotionScale {
    /// Hip height (m) for legged locomotion.
    pub hip_height: f32,
    /// Body length (m) for swimming and undulation.
    pub body_length: f32,
    /// Whether the animal moves by body undulation (fish, snakes).
    pub undulatory: bool,
}

impl LocomotionScale {
    /// Scales every length by `factor` (juveniles are smaller).
    pub fn scaled(self, factor: f32) -> Self {
        Self { hip_height: self.hip_height * factor, body_length: self.body_length * factor, ..self }
    }
}

/// Derives locomotion dimensions from a model's real-world size.
///
/// Approximation: hip height is 60 % of the standing height for legged
/// animals (the rest is back, neck and head); body length is the largest
/// horizontal extent.
pub fn locomotion_scale(model: &ModelEntry) -> LocomotionScale {
    let [x, y, z] = model.real_size_m;
    let body_length = x.max(z).max(1e-3);
    let elongated = body_length > 6.0 * y;
    let undulatory = model.category == ModelCategory::Fish || (model.category == ModelCategory::Reptile && elongated);
    LocomotionScale { hip_height: (0.6 * y).max(1e-3), body_length, undulatory }
}

fn is_flyer(model: &ModelEntry) -> bool {
    model.animation_set.contains_key("fly")
}

fn clip(model: &ModelEntry, key: &'static str, multiplier: Option<f32>, repeat: bool) -> Option<PlannedClip> {
    model.clip_for(key).map(|name| PlannedClip {
        key,
        name: name.to_string(),
        multiplier: multiplier.unwrap_or_else(|| model.speed_multiplier(key)),
        repeat,
    })
}

/// First available clip among `keys`, with its manifest multiplier.
fn first_clip(model: &ModelEntry, keys: &[&'static str], repeat: bool) -> Option<PlannedClip> {
    keys.iter().find_map(|k| clip(model, k, None, repeat))
}

/// A clip frozen on its first frame (rate 0), used under procedural poses.
fn frozen(model: &ModelEntry, keys: &[&'static str]) -> Option<PlannedClip> {
    keys.iter().find_map(|k| clip(model, k, Some(0.0), false))
}

fn rate_for(model: &ModelEntry, action: OrganismAction) -> RateRule {
    match action.canonical() {
        OrganismAction::Walk | OrganismAction::Run | OrganismAction::Swim => {
            if locomotion_scale(model).undulatory {
                RateRule::Undulation
            } else {
                RateRule::Stride
            }
        }
        _ => RateRule::Fixed,
    }
}

/// Builds the action plan for `action` on `model`.
///
/// Every action yields a drawable plan for every animated model: either the
/// authored clip, a substitute clip, or procedural motion.
pub fn plan_action(model: &ModelEntry, action: OrganismAction) -> ActionPlan {
    let canonical = action.canonical();
    let one_shot = action.is_one_shot();
    if let Some(key) = canonical.manifest_key()
        && let Some(c) = clip(model, key, None, !one_shot)
    {
        return ActionPlan {
            action,
            clip: Some(c),
            overlays: Vec::new(),
            rate: rate_for(model, action),
            source: PlanSource::Authored,
        };
    }
    let fish = model.category == ModelCategory::Fish;
    let flyer = is_flyer(model);
    let plan = |clip: Option<PlannedClip>, overlays: Vec<MotionOverlay>, rate: RateRule| ActionPlan {
        action,
        clip,
        overlays,
        rate,
        source: PlanSource::Fallback,
    };
    match canonical {
        OrganismAction::Drink => {
            if let Some(c) = clip(model, "eat", None, true) {
                plan(Some(c), Vec::new(), RateRule::Fixed)
            } else {
                plan(first_clip(model, &["idle"], true), vec![MotionOverlay::HeadDip], RateRule::Fixed)
            }
        }
        OrganismAction::Eat => plan(first_clip(model, &["idle"], true), vec![MotionOverlay::HeadDip], RateRule::Fixed),
        OrganismAction::Glide => {
            if flyer {
                plan(frozen(model, &["fly"]), vec![MotionOverlay::Glide], RateRule::Fixed)
            } else {
                let mut p = plan_action(model, OrganismAction::Run);
                p.action = action;
                p.source = PlanSource::Fallback;
                p
            }
        }
        OrganismAction::Fly => {
            // Flight requested for a non-flying species: draw its fastest locomotion.
            let substitute = if fish { OrganismAction::Swim } else { OrganismAction::Run };
            let mut p = plan_action(model, substitute);
            p.action = action;
            p.source = PlanSource::Fallback;
            p
        }
        OrganismAction::Walk | OrganismAction::Run | OrganismAction::Flee => {
            if fish {
                plan(first_clip(model, &["swim", "idle"], true), Vec::new(), RateRule::Undulation)
            } else if canonical == OrganismAction::Run && model.animation_set.contains_key("walk") {
                let c = clip(model, "walk", Some(model.speed_multiplier("walk") * 2.0), true);
                plan(c, Vec::new(), rate_for(model, action))
            } else if flyer {
                plan(first_clip(model, &["fly"], true), Vec::new(), RateRule::Fixed)
            } else {
                plan(first_clip(model, &["idle"], true), vec![MotionOverlay::LegGait], RateRule::Stride)
            }
        }
        OrganismAction::Swim => {
            if flyer {
                plan(frozen(model, &["fly", "idle"]), vec![MotionOverlay::Waterline], RateRule::Fixed)
            } else if let Some(c) = clip(model, "walk", Some(0.6), true) {
                plan(Some(c), vec![MotionOverlay::Waterline], rate_for(model, action))
            } else {
                plan(
                    first_clip(model, &["idle"], true),
                    vec![MotionOverlay::LegGait, MotionOverlay::Waterline],
                    RateRule::Stride,
                )
            }
        }
        OrganismAction::Attack => {
            if fish {
                plan(clip(model, "swim", Some(2.5), true), vec![MotionOverlay::Lunge], RateRule::Fixed)
            } else {
                plan(first_clip(model, &["idle", "fly"], true), vec![MotionOverlay::Lunge], RateRule::Fixed)
            }
        }
        OrganismAction::Death => {
            if fish {
                plan(frozen(model, &["swim", "idle"]), vec![MotionOverlay::BellyUpAndSink], RateRule::Fixed)
            } else if flyer {
                plan(frozen(model, &["fly", "idle"]), vec![MotionOverlay::Tumble], RateRule::Fixed)
            } else {
                plan(frozen(model, &["idle"]), vec![MotionOverlay::TipOverAndSink], RateRule::Fixed)
            }
        }
        OrganismAction::Rest => {
            if fish {
                plan(clip(model, "swim", Some(0.25), true), Vec::new(), RateRule::Fixed)
            } else if flyer {
                plan(frozen(model, &["fly", "idle"]), vec![MotionOverlay::LieDown], RateRule::Fixed)
            } else {
                plan(
                    clip(model, "idle", Some(0.5 * model.speed_multiplier("idle")), true),
                    vec![MotionOverlay::LieDown],
                    RateRule::Fixed,
                )
            }
        }
        OrganismAction::Jump => {
            let c = if fish {
                first_clip(model, &["swim"], true)
            } else {
                first_clip(model, &["walk", "fly", "idle"], true)
            };
            plan(c, vec![MotionOverlay::Hop], RateRule::Fixed)
        }
        OrganismAction::Idle => plan(frozen(model, &["walk", "swim", "fly", "run"]), Vec::new(), RateRule::Fixed),
    }
}

/// Default speed (m/s) the natural cadence of an action's clip represents:
/// Froude 0.25 for walking, Froude 3 for running, one body length per
/// second (times the manifest multiplier) for swimming and undulation.
fn reference_speed(plan: &ActionPlan, scale: LocomotionScale) -> f32 {
    let key = plan.clip.as_ref().map_or("walk", |c| c.key);
    match plan.rate {
        RateRule::Fixed => 0.0,
        RateRule::Undulation => {
            let multiplier = plan.clip.as_ref().map_or(1.0, |c| c.multiplier).max(0.05);
            multiplier * scale.body_length
        }
        RateRule::Stride => {
            let fr = if key == "run" || plan.action.canonical() == OrganismAction::Run {
                FROUDE_DEFAULT_RUN
            } else {
                FROUDE_DEFAULT_WALK
            };
            speed_for_froude(fr, scale.hip_height)
        }
    }
}

/// Lowest and highest playback rates applied to locomotion clips; outside
/// this band clips look broken (frozen or blurred).
pub const PLAYBACK_RATE_RANGE: (f32, f32) = (0.15, 4.0);

/// Playback rate for a plan at the given movement speed (m/s).
///
/// With `speed <= 0` (unknown) the clip plays at its natural rate (the
/// manifest multiplier). Otherwise the rate is the multiplier times the
/// ratio of the biomechanical cycle frequency at `speed` to the frequency at
/// the clip's reference speed: stride frequency `v/λ(v)` for legs (Alexander
/// 1976), tail-beat/wave frequency `∝ v` for undulation (Bainbridge 1958).
pub fn playback_rate(plan: &ActionPlan, scale: LocomotionScale, speed: f32) -> f32 {
    let natural = plan.clip.as_ref().map_or(1.0, |c| c.multiplier);
    if natural == 0.0 || speed <= 0.0 || !speed.is_finite() || plan.rate == RateRule::Fixed {
        return natural;
    }
    let reference = reference_speed(plan, scale);
    if reference <= 0.0 {
        return natural;
    }
    let ratio = match plan.rate {
        RateRule::Stride => {
            let f_ref = stride_frequency(reference, scale.hip_height);
            if f_ref > 0.0 { stride_frequency(speed, scale.hip_height) / f_ref } else { 1.0 }
        }
        RateRule::Undulation => {
            tailbeat_frequency(speed, scale.body_length) / tailbeat_frequency(reference, scale.body_length)
        }
        RateRule::Fixed => 1.0,
    };
    (natural * ratio).clamp(PLAYBACK_RATE_RANGE.0, PLAYBACK_RATE_RANGE.1)
}

/// Loaded manifest indexed by model id and by species id.
#[derive(Resource, Clone, Debug, Default)]
pub struct ModelLibrary {
    models: Vec<ModelEntry>,
    by_id: BTreeMap<String, usize>,
    by_species: BTreeMap<String, Vec<usize>>,
}

impl ModelLibrary {
    /// Indexes a validated manifest.
    pub fn from_manifest(manifest: ModelManifest) -> Self {
        let mut by_id = BTreeMap::new();
        let mut by_species: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (i, m) in manifest.models.iter().enumerate() {
            by_id.insert(m.id.clone(), i);
            for s in &m.species {
                by_species.entry(s.clone()).or_default().push(i);
            }
        }
        Self { models: manifest.models, by_id, by_species }
    }

    /// Reads `<assets_root>/models/manifest.toml`.
    pub fn load(assets_root: &Path) -> Result<Self, ManifestError> {
        ModelManifest::load(&assets_root.join(MANIFEST_PATH)).map(Self::from_manifest)
    }

    /// All models in manifest order.
    pub fn models(&self) -> &[ModelEntry] {
        &self.models
    }

    /// Model by id.
    pub fn model(&self, id: &str) -> Option<&ModelEntry> {
        self.by_id.get(id).map(|&i| &self.models[i])
    }

    /// The model used for `species`: a model whose id equals the species id
    /// wins; otherwise the variant is chosen by `variant` (e.g. a hash of
    /// the organism) among all models listing the species.
    pub fn model_for_species(&self, species: &str, variant: u64) -> Option<&ModelEntry> {
        if let Some(m) = self.model(species)
            && m.species.iter().any(|s| s == species)
        {
            return Some(m);
        }
        let list = self.by_species.get(species)?;
        let index = list[(variant % list.len() as u64) as usize];
        Some(&self.models[index])
    }

    /// Animated model for an animal species (static flora models are ignored).
    pub fn animal_model(&self, species: &str) -> Option<&ModelEntry> {
        let list = self.by_species.get(species)?;
        list.iter().map(|&i| &self.models[i]).find(|m| m.category.is_animal() && m.is_animated())
    }

    /// Every species id listed by animated animal models, sorted.
    pub fn animal_species(&self) -> Vec<&str> {
        self.by_species
            .iter()
            .filter(|(_, idx)| idx.iter().any(|&i| self.models[i].category.is_animal()))
            .map(|(s, _)| s.as_str())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::ModelManifest;

    fn entry(id: &str, category: &str, clips: &[(&str, &str)], speeds: &[(&str, f32)], size: [f32; 3]) -> ModelEntry {
        let mut text = format!(
            "schema_version = 1\nunits = \"metre\"\nup_axis = \"+Y\"\n[[model]]\nid = \"{id}\"\nname = \"n\"\ncategory = \"{category}\"\nspecies = [\"{id}\"]\nsource = \"s\"\nlicense = \"CC0-1.0\"\nauthor = \"a\"\nmodel_path = \"models/{id}.glb\"\nscale = 1.0\nreal_size_m = [{}, {}, {}]\nforward = \"+Z\"\nyaw_offset_deg = 0\ntriangles = 1\nfile_bytes = 1\n[model.animation_set]\n",
            size[0], size[1], size[2]
        );
        for (k, v) in clips {
            text.push_str(&format!("{k} = \"{v}\"\n"));
        }
        text.push_str("[model.anim_speed]\n");
        for (k, v) in speeds {
            text.push_str(&format!("{k} = {v}\n"));
        }
        ModelManifest::parse(&text).unwrap().models.remove(0)
    }

    fn fox() -> ModelEntry {
        entry(
            "fox",
            "animal.mammal",
            &[
                ("idle", "Idle"),
                ("walk", "Walk"),
                ("run", "Gallop"),
                ("eat", "Eating"),
                ("attack", "Attack"),
                ("death", "Death"),
                ("jump", "Jump"),
                ("rest", "Low"),
            ],
            &[],
            [0.2, 0.48, 1.0],
        )
    }

    #[test]
    fn authored_clips_win() {
        let fox = fox();
        let p = plan_action(&fox, OrganismAction::Run);
        assert_eq!(p.source, PlanSource::Authored);
        assert_eq!(p.clip.as_ref().unwrap().name, "Gallop");
        assert_eq!(p.rate, RateRule::Stride);
        assert_eq!(plan_action(&fox, OrganismAction::Flee).clip.unwrap().name, "Gallop");
        let death = plan_action(&fox, OrganismAction::Death);
        assert!(!death.clip.unwrap().repeat, "death holds its last frame");
        // Drink reuses the head-down eating clip.
        assert_eq!(plan_action(&fox, OrganismAction::Drink).clip.unwrap().name, "Eating");
        // Swimming land mammal: slowed walk at the waterline.
        let swim = plan_action(&fox, OrganismAction::Swim);
        assert_eq!(swim.clip.as_ref().unwrap().name, "Walk");
        assert!(swim.has(MotionOverlay::Waterline));
        // Fox cannot fly: drawn as running.
        assert_eq!(plan_action(&fox, OrganismAction::Fly).clip.unwrap().name, "Gallop");
    }

    #[test]
    fn sheep_without_locomotion_clips_uses_leg_gait() {
        let sheep = entry(
            "sheep",
            "animal.mammal",
            &[("idle", "Armature|Idle"), ("jump", "Armature|Jump")],
            &[],
            [0.4, 0.9, 1.3],
        );
        let walk = plan_action(&sheep, OrganismAction::Walk);
        assert_eq!(walk.source, PlanSource::Fallback);
        assert!(walk.has(MotionOverlay::LegGait));
        assert_eq!(walk.clip.unwrap().name, "Armature|Idle");
        let death = plan_action(&sheep, OrganismAction::Death);
        assert!(death.has(MotionOverlay::TipOverAndSink));
        assert_eq!(death.clip.unwrap().multiplier, 0.0);
        for a in OrganismAction::ALL {
            assert!(plan_action(&sheep, a).is_drawable(), "{a}");
        }
    }

    #[test]
    fn fish_and_flyer_fallbacks() {
        let fish = entry(
            "perch",
            "animal.fish",
            &[("idle", "Swim"), ("walk", "Swim"), ("run", "Swim"), ("swim", "Swim")],
            &[("idle", 0.4), ("walk", 0.8), ("run", 1.8)],
            [0.05, 0.1, 0.3],
        );
        let death = plan_action(&fish, OrganismAction::Death);
        assert!(death.has(MotionOverlay::BellyUpAndSink));
        let attack = plan_action(&fish, OrganismAction::Attack);
        assert_eq!(attack.clip.as_ref().unwrap().multiplier, 2.5);
        assert!(attack.has(MotionOverlay::Lunge));
        assert_eq!(plan_action(&fish, OrganismAction::Swim).rate, RateRule::Undulation);
        let hawk = entry("hawk", "animal.bird", &[("idle", "Fly"), ("fly", "Fly")], &[("idle", 0.0)], [1.2, 0.4, 0.96]);
        let glide = plan_action(&hawk, OrganismAction::Glide);
        assert_eq!(glide.clip.as_ref().unwrap().multiplier, 0.0);
        assert!(glide.has(MotionOverlay::Glide));
        assert!(plan_action(&hawk, OrganismAction::Death).has(MotionOverlay::Tumble));
    }

    #[test]
    fn playback_rate_tracks_speed() {
        let fox = fox();
        let scale = locomotion_scale(&fox);
        let walk = plan_action(&fox, OrganismAction::Walk);
        // Unknown speed: natural rate.
        assert_eq!(playback_rate(&walk, scale, 0.0), 1.0);
        // At the reference speed the natural rate is reproduced.
        let v_ref = speed_for_froude(FROUDE_DEFAULT_WALK, scale.hip_height);
        assert!((playback_rate(&walk, scale, v_ref) - 1.0).abs() < 1e-4);
        // Faster movement → faster animation, monotonically, clamped.
        let slow = playback_rate(&walk, scale, 0.5 * v_ref);
        let fast = playback_rate(&walk, scale, 1.5 * v_ref);
        assert!(slow < 1.0 && fast > 1.0);
        assert_eq!(playback_rate(&walk, scale, 1000.0), PLAYBACK_RATE_RANGE.1);
        // A juvenile (smaller) steps faster at the same speed.
        assert!(playback_rate(&walk, scale.scaled(0.5), v_ref) > 1.0);
        // Fish: tail beat ∝ speed, one body length per second at rate 1.
        let fish = entry("perch", "animal.fish", &[("swim", "Swim")], &[], [0.05, 0.1, 0.3]);
        let swim = plan_action(&fish, OrganismAction::Swim);
        let fs = locomotion_scale(&fish);
        assert!((playback_rate(&swim, fs, 0.6) - 2.0).abs() < 1e-4);
    }

    #[test]
    fn library_lookup_by_species() {
        let fox = fox();
        let manifest = ModelManifest {
            schema_version: 1,
            units: "metre".into(),
            up_axis: "+Y".into(),
            sources: vec![],
            models: vec![fox],
        };
        let lib = ModelLibrary::from_manifest(manifest);
        assert_eq!(lib.animal_model("fox").map(|m| m.id.as_str()), Some("fox"));
        assert!(lib.animal_model("dragon").is_none());
        assert_eq!(lib.animal_species(), vec!["fox"]);
        assert_eq!(lib.model_for_species("fox", 99).map(|m| m.id.as_str()), Some("fox"));
    }
}
