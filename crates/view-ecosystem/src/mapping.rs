//! Mapping of model state to `organism-render` visuals: which visual id a
//! species is drawn with, which animation an animal shows, how fast it
//! moves (m/s) and how big it is; tree styles and growth stages.

use ecosystem::species::{AnimalTraits, Locomotion, TreeSpecies};
use ecosystem::trees::TreeStage;
use ecosystem::view::{AnimalView, TreeView};
use ecosystem::{Action, Species};
use organism_render::catalog::OrganismCatalog;
use organism_render::{OrganismAction, TreeStyle as RenderTreeStyle};

/// The `organism-render` species id an ecosystem species is drawn with.
///
/// * a species with an authored model (`visual.model`) is drawn with that
///   GLB: the id is one of the model's species names, preferring the name
///   whose procedural stand-in (used at far distances) is the species' own
///   rig;
/// * otherwise its procedural rig (`visual.procedural`);
/// * otherwise its own id (organism-render resolves ids and aliases).
pub fn render_id(species: &Species, catalog: &OrganismCatalog) -> String {
    let rig = species.visual.procedural.as_deref();
    if let Some(model_id) = species.visual.model.as_deref()
        && catalog.use_glb
        && let Some(model) = catalog.library.model(model_id)
        && model.category.is_animal()
        && model.is_animated()
    {
        // Names that resolve to this very model.
        let names: Vec<&String> = model
            .species
            .iter()
            .filter(|s| catalog.library.animal_model(s).is_some_and(|m| m.id == model.id))
            .collect();
        let own_rig = names.iter().find(|s| {
            rig.is_some_and(|r| catalog.procedural(s).zip(catalog.species.get(r)).is_some_and(|(a, b)| a.id == b.id))
        });
        let with_far = names.iter().find(|s| catalog.procedural(s).is_some());
        if let Some(name) = own_rig.or(with_far).or(names.first()) {
            return (*name).clone();
        }
    }
    if let Some(r) = rig
        && catalog.species.get(r).is_some()
    {
        return r.to_string();
    }
    species.id.clone()
}

/// The visual of adult males when the model manifest has a distinct male
/// model for the species' visual (`<id>_male`, e.g. a red deer stag with
/// antlers next to the hind), else `None`.
pub fn male_render_id(base: &str, catalog: &OrganismCatalog) -> Option<String> {
    let id = format!("{base}_male");
    let male = catalog.library.animal_model(&id)?;
    let distinct = catalog.library.animal_model(base).is_none_or(|m| m.id != male.id);
    (catalog.use_glb && male.is_animated() && distinct).then_some(id)
}

/// Inputs of [`organism_action`] beyond the model's action.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MotionContext {
    /// Speed relative to the species' cruising speed.
    pub cruise_ratio: f32,
    /// Airborne (a flier more than 1 m up).
    pub airborne: bool,
    /// In water deep enough to swim (or an aquatic species).
    pub swimming: bool,
    /// Killed prey within the last ticks.
    pub attacking: bool,
}

/// Below this fraction of cruising speed an animal counts as standing.
pub const STANDING_RATIO: f32 = 0.08;
/// Above this fraction of cruising speed a walker runs.
pub const RUNNING_RATIO: f32 = 1.35;

/// The animation shown for a model action. An animal in water too deep to
/// wade swims (treading water when it holds still: the swim pose keeps its
/// body low in the water, where standing or lying would put it on the
/// surface).
pub fn organism_action(action: Action, ctx: MotionContext) -> OrganismAction {
    let moving = ctx.cruise_ratio > STANDING_RATIO;
    let locomotion = || {
        if ctx.airborne {
            OrganismAction::Fly
        } else if ctx.swimming {
            OrganismAction::Swim
        } else if !moving {
            OrganismAction::Idle
        } else if ctx.cruise_ratio > RUNNING_RATIO {
            OrganismAction::Run
        } else {
            OrganismAction::Walk
        }
    };
    if ctx.attacking && !ctx.airborne {
        return OrganismAction::Attack;
    }
    match action {
        Action::Graze | Action::Browse | Action::Forage | Action::Feed => {
            if ctx.airborne || (moving && ctx.cruise_ratio > 0.5) {
                locomotion()
            } else if ctx.swimming && !moving {
                OrganismAction::Swim
            } else {
                OrganismAction::Eat
            }
        }
        Action::Drink => {
            if ctx.swimming {
                OrganismAction::Swim
            } else if moving {
                locomotion()
            } else {
                OrganismAction::Drink
            }
        }
        Action::Rest | Action::Sleep | Action::Hibernate | Action::Torpor | Action::Incubate => {
            if ctx.airborne {
                OrganismAction::Fly
            } else if ctx.swimming {
                OrganismAction::Swim
            } else {
                OrganismAction::Rest
            }
        }
        Action::Flee => {
            if ctx.airborne || ctx.swimming || !moving {
                locomotion()
            } else {
                OrganismAction::Flee
            }
        }
        Action::Idle
        | Action::Wander
        | Action::Hunt
        | Action::SeekMate
        | Action::FollowParent
        | Action::Defend
        | Action::Migrate => locomotion(),
    }
}

/// Display label of a model action.
pub fn action_label(action: Action) -> &'static str {
    match action {
        Action::Idle => "Idle",
        Action::Wander => "Wandering",
        Action::Graze => "Grazing",
        Action::Browse => "Browsing",
        Action::Forage => "Foraging",
        Action::Hunt => "Hunting",
        Action::Feed => "Feeding on a carcass",
        Action::Flee => "Fleeing",
        Action::Drink => "Drinking",
        Action::Rest => "Resting",
        Action::Sleep => "Sleeping",
        Action::SeekMate => "Seeking a mate",
        Action::FollowParent => "Following its mother",
        Action::Incubate => "Incubating",
        Action::Torpor => "Torpid (too cold)",
        Action::Defend => "Defending its territory",
        Action::Migrate => "Migrating",
        Action::Hibernate => "Hibernating",
    }
}

/// Physical speed (m/s) of a model velocity (cells per tick): the model
/// moves an animal at `v` m/s by `v · motion_seconds / cell_size` cells
/// per tick, so the ratio to the species' sprint gives the speed without
/// needing `motion_seconds`.
pub fn speed_m_s(velocity_cells: f32, traits: &AnimalTraits) -> f32 {
    if traits.sprint_cells > 0.0 { velocity_cells / traits.sprint_cells * traits.sprint_m_s } else { 0.0 }
}

/// Speed relative to cruising.
pub fn cruise_ratio(velocity_cells: f32, traits: &AnimalTraits) -> f32 {
    if traits.cruise_cells > 0.0 { velocity_cells / traits.cruise_cells } else { 0.0 }
}

/// Drawn body-size factor: the model's display length (body length ×
/// visual scale × growth `(m/M)^{1/3}`) over the species' body length, so
/// the renderer's adult model is scaled by the species' visual scale and
/// the animal's growth.
pub fn age_scale(animal: &AnimalView, species: &Species) -> f32 {
    let length = species.body.length_m.max(1e-4);
    (animal.size_m / length).clamp(0.05, 1000.0)
}

/// Whether an animal is drawn swimming: aquatic species always, swimmers
/// when the water at their position is deeper than they can wade.
pub fn is_swimming(species: &Species, water_depth: f32) -> bool {
    species.habitat.aquatic
        || (species.can(Locomotion::Swim) && water_depth > species.habitat.max_wade_depth_m.max(0.3))
}

/// Height (m) of an animal relative to the scene: fliers above the ground,
/// swimmers relative to the water surface (negative altitudes are below
/// it), walkers on the ground.
pub fn animal_height(altitude_m: f32, ground: f32, surface: f32, swimming: bool) -> f32 {
    if swimming {
        // Never below the bed.
        (surface + altitude_m.min(0.0)).max(ground + 0.05)
    } else {
        ground + altitude_m.max(0.0)
    }
}

/// Display label of a tree's growth stage.
pub fn tree_stage_label(stage: TreeStage) -> &'static str {
    match stage {
        TreeStage::Seedling => "seedling",
        TreeStage::Sapling => "sapling",
        TreeStage::Mature => "mature",
        TreeStage::Old => "old",
        TreeStage::Dead => "dead (standing snag)",
    }
}

/// Renderer tree style of a model tree.
pub fn tree_style(tree: &TreeView) -> RenderTreeStyle {
    if tree.stage == TreeStage::Dead {
        return RenderTreeStyle::Dead;
    }
    match tree.style {
        ecosystem::species::TreeStyle::Broadleaf => RenderTreeStyle::Broadleaf,
        ecosystem::species::TreeStyle::Conifer => RenderTreeStyle::Conifer,
        ecosystem::species::TreeStyle::Palm => RenderTreeStyle::Palm,
        ecosystem::species::TreeStyle::Acacia => RenderTreeStyle::Acacia,
    }
}

/// Growth stage (0..1) of a tree for the renderer: its height over the
/// species' maximum height.
pub fn tree_growth(height_m: f32, species: &TreeSpecies) -> f32 {
    (height_m / species.max_height_m.max(0.1)).clamp(0.02, 1.0)
}

/// Height (m) the renderer gives a tree of `style` at `growth` (see
/// `organism_render::plants::tree::generate_tree`).
pub fn rendered_tree_height(style: RenderTreeStyle, growth: f32) -> f32 {
    style.adult_height() * (0.08 + 0.92 * growth.clamp(0.0, 1.0).powf(0.8))
}

/// Uniform scale that makes the rendered tree as tall as the model's.
pub fn tree_scale(style: RenderTreeStyle, growth: f32, height_m: f32) -> f32 {
    (height_m / rendered_tree_height(style, growth).max(0.01)).clamp(0.01, 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn ctx(cruise_ratio: f32) -> MotionContext {
        MotionContext { cruise_ratio, airborne: false, swimming: false, attacking: false }
    }

    #[test]
    fn actions_map_to_animations() {
        assert_eq!(organism_action(Action::Graze, ctx(0.0)), OrganismAction::Eat);
        assert_eq!(organism_action(Action::Wander, ctx(0.0)), OrganismAction::Idle);
        assert_eq!(organism_action(Action::Wander, ctx(0.9)), OrganismAction::Walk);
        assert_eq!(organism_action(Action::Hunt, ctx(2.5)), OrganismAction::Run);
        assert_eq!(organism_action(Action::Flee, ctx(2.5)), OrganismAction::Flee);
        assert_eq!(organism_action(Action::Sleep, ctx(0.0)), OrganismAction::Rest);
        assert_eq!(organism_action(Action::Hibernate, ctx(0.0)), OrganismAction::Rest);
        assert_eq!(organism_action(Action::Drink, ctx(0.0)), OrganismAction::Drink);
        let flying = MotionContext { airborne: true, ..ctx(1.0) };
        assert_eq!(organism_action(Action::Migrate, flying), OrganismAction::Fly);
        assert_eq!(organism_action(Action::Forage, flying), OrganismAction::Fly);
        let swimming = MotionContext { swimming: true, ..ctx(1.0) };
        assert_eq!(organism_action(Action::Wander, swimming), OrganismAction::Swim);
        // In deep water nothing stands or lies on the surface: it treads water.
        let afloat = MotionContext { swimming: true, ..ctx(0.0) };
        for action in [Action::Rest, Action::Sleep, Action::Wander, Action::Drink, Action::Graze] {
            assert_eq!(organism_action(action, afloat), OrganismAction::Swim, "{action:?}");
        }
        let attacking = MotionContext { attacking: true, ..ctx(2.0) };
        assert_eq!(organism_action(Action::Feed, attacking), OrganismAction::Attack);
        // Every model action yields some animation.
        for action in [
            Action::Idle,
            Action::Wander,
            Action::Graze,
            Action::Browse,
            Action::Forage,
            Action::Hunt,
            Action::Feed,
            Action::Flee,
            Action::Drink,
            Action::Rest,
            Action::Sleep,
            Action::SeekMate,
            Action::FollowParent,
            Action::Incubate,
            Action::Torpor,
            Action::Defend,
            Action::Migrate,
            Action::Hibernate,
        ] {
            let _ = organism_action(action, ctx(1.0));
            assert!(!action_label(action).is_empty());
        }
    }

    #[test]
    fn speeds_follow_the_species_scale() {
        let catalog = ecosystem::SpeciesCatalog::builtin().unwrap();
        let scale = ecosystem::units::TimeScale {
            ticks_per_day: 240,
            days_per_year: 120,
            cell_size_m: 20.0,
            motion_seconds: 5.0,
        };
        let wolf = catalog.animal("wolf").unwrap();
        let traits = AnimalTraits::derive(wolf, &scale);
        assert!((speed_m_s(traits.sprint_cells, &traits) - traits.sprint_m_s).abs() < 1e-4);
        assert!((cruise_ratio(traits.cruise_cells, &traits) - 1.0).abs() < 1e-6);
        assert_eq!(speed_m_s(0.0, &traits), 0.0);
    }

    #[test]
    fn heights_of_walkers_fliers_and_swimmers() {
        assert_eq!(animal_height(0.0, 10.0, 10.0, false), 10.0);
        assert_eq!(animal_height(30.0, 10.0, 10.0, false), 40.0);
        assert_eq!(animal_height(-2.0, 4.0, 10.0, true), 8.0);
        assert_eq!(animal_height(-20.0, 4.0, 10.0, true), 4.05, "clamped above the bed");
        assert_eq!(animal_height(0.0, 4.0, 10.0, true), 10.0, "surface swimmers float");
    }

    #[test]
    fn tree_scale_matches_model_height() {
        for style in [RenderTreeStyle::Broadleaf, RenderTreeStyle::Conifer, RenderTreeStyle::Dead] {
            for (g, h) in [(0.1f32, 2.0f32), (0.5, 9.0), (1.0, 25.0)] {
                let drawn = rendered_tree_height(style, g) * tree_scale(style, g, h);
                assert!((drawn - h).abs() < 1e-3, "{style:?} {g} {h}");
            }
        }
    }

    #[test]
    fn every_species_resolves_to_a_drawable_visual() {
        let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
        let render = OrganismCatalog::load(&assets);
        assert!(render.manifest_error.is_none(), "{:?}", render.manifest_error);
        let catalog = ecosystem::SpeciesCatalog::builtin().unwrap();
        for s in catalog.animals() {
            let id = render_id(s, &render);
            assert!(render.knows(&id), "{} → `{id}` is not drawable", s.id);
            if let Some(model) = &s.visual.model {
                let near = render.library.animal_model(&id).map(|m| m.id.clone());
                assert_eq!(near.as_deref(), Some(model.as_str()), "{} uses its authored model", s.id);
            }
            if s.visual.model.is_none() {
                assert!(render.procedural(&id).is_some(), "{} has a procedural rig", s.id);
            }
            if let Some(male) = male_render_id(&id, &render) {
                assert!(render.knows(&male), "{} males → `{male}` is not drawable", s.id);
            }
        }
        // Red deer stags carry antlers: the manifest's stag model.
        let deer = render_id(catalog.animal("deer").unwrap(), &render);
        let stag = male_render_id(&deer, &render).expect("a male deer model");
        assert_eq!(render.library.animal_model(&stag).map(|m| m.id.as_str()), Some("stag"));
        let wolf = render_id(catalog.animal("wolf").unwrap(), &render);
        assert_eq!(male_render_id(&wolf, &render), None, "no male model: both sexes alike");
    }
}
