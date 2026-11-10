//! Tutorial: build a producers–consumers food web step by step, then a
//! simple and an advanced example, metric explanations and experiments.
//! Page buttons send [`TutorialAction`]s handled by [`handle_actions`],
//! which also understands generic ids used by tests and smoke captures
//! (`load:<preset>`, `editor:<tab>`, `tab:<right tab>`, `overlay:<field>`,
//! `select:first`, `tool:<tool>`, `color:<mode>`, `play`).

use bevy::prelude::*;
use interface::{
    MetricsHistory, ModeScenarios, RightPanelTab, ShellCommand, Toasts, Tutorial, TutorialAction, TutorialPage,
};
use rule_engine::def::{
    Distribution, EndCondition, EntityType, EventEffect, FieldInit, LifeSystem, LifecycleDef, MovementDef, RandomEvent,
    RelationKind, Relationship, ReproductionDef, ReproductionMode, ResourceField,
};
use rule_engine::dsl::parse_rule;
use simulation_core::Control;

use crate::colors::ColorBy;
use crate::editor::rules::RuleMode;
use crate::editor::{ClEditor, EditorTab};
use crate::session::{ClData, ClRunner, ClScenarios, open_preset, start_system};
use crate::state::{ClUi, Tool};

/// Seed of the tutorial world.
pub const TUTORIAL_SEED: u64 = 99;

/// Rule added in the "write a rule" step.
pub const REST_RULE: &str = "rule rest for grazer: when energy > 35 then stay() priority 200";

/// The same rule with a typo, for the "live validation" step.
pub const BROKEN_RULE: &str = "rule rest for grazer:\n    when enrgy > 35\n    then stay()\n    priority 200";

/// Step 1: an empty 48 × 48 torus.
pub fn tutorial_world() -> LifeSystem {
    let mut sys = LifeSystem::new("My food web", 48, 48);
    sys.description =
        "Built step by step in the tutorial: sunlight feeds plants, plants feed grazers, grazers feed hunters.".into();
    sys.end_conditions.push(EndCondition::AllExtinct);
    sys
}

/// Arrivals from the surrounding landscape: a random event that brings
/// `count` entities of `entity` with probability `p` per tick (rescues a
/// level that crashed locally, as in the preset).
fn immigration(sys: &mut LifeSystem, name: &str, entity: &str, p: f64, count: u32) {
    if sys.world.random_events.iter().any(|e| e.name == name) {
        return;
    }
    sys.world.random_events.push(RandomEvent {
        name: name.into(),
        probability: p,
        effect: EventEffect::Immigration { entity: entity.into(), count },
    });
}

/// Step 2: a regrowing, diffusing sunlight field.
pub fn add_sunlight(sys: &mut LifeSystem) {
    if sys.resource_index("sunlight").is_some() {
        return;
    }
    let mut r = ResourceField::new("sunlight");
    r.color = [0.95, 0.90, 0.40, 1.0];
    r.initial = FieldInit::Uniform(1.0);
    r.regen = 0.06;
    r.max = 1.0;
    r.diffusion = 0.2;
    sys.world.resources.push(r);
}

/// Step 3: sessile plants that feed on sunlight and spread where uncrowded.
pub fn add_plants(sys: &mut LifeSystem) {
    add_sunlight(sys);
    if sys.type_index("plant").is_some() {
        return;
    }
    immigration(sys, "seed rain", "plant", 0.1, 3);
    let mut plant = EntityType::new("plant");
    plant.description =
        "Producer: converts sunlight into energy; spreads where fewer than 4 plants grow nearby.".into();
    plant.color = [0.25, 0.75, 0.30, 1.0];
    plant.icon = "🌿".into();
    plant.size = 0.8;
    plant.energy.initial = 5.0;
    plant.energy.max = 15.0;
    plant.energy.metabolism = 0.04;
    plant.reproduction =
        ReproductionDef { mode: ReproductionMode::Asexual, cost: 5.0, cooldown: 10, litter: 1, rate: 0.0 };
    plant.lifecycle = LifecycleDef { max_age: Some(600), maturity: 10 };
    plant.perception = 2.0;
    plant.initial.count = 400;
    plant.initial.distribution = Distribution::Uniform;
    sys.entity_types.push(plant);
    sys.relationships.push(Relationship {
        subject: "plant".into(),
        kind: RelationKind::Consumes { efficiency: 1.0, hunger: Some(14.0), radius: 5.0, bite: 0.3, handling: 0 },
        object: "sunlight".into(),
        priority_shift: 0,
    });
    if let Ok(rule) = parse_rule(
        "rule spread for plant: when age >= 10 and energy >= 10 and nearby(plant, radius=1.5) < 4 \
         then reproduce(probability=0.1) priority 300",
    ) {
        sys.rules.push(rule);
    }
}

/// Step 5: grazers that eat plants (30 % assimilation, handling time 25).
pub fn add_grazers(sys: &mut LifeSystem) {
    add_plants(sys);
    if sys.type_index("grazer").is_some() {
        return;
    }
    immigration(sys, "grazer migration", "grazer", 0.02, 2);
    let mut g = EntityType::new("grazer");
    g.description = "Primary consumer: eats plants (30 % assimilation, one plant per 25 ticks at most).".into();
    g.color = [0.85, 0.75, 0.40, 1.0];
    g.icon = "🐑".into();
    g.size = 1.0;
    g.movement = MovementDef::Directed { speed: 0.6, turn: 0.7 };
    g.energy.initial = 25.0;
    g.energy.max = 40.0;
    g.energy.metabolism = 0.06;
    g.reproduction =
        ReproductionDef { mode: ReproductionMode::Asexual, cost: 10.0, cooldown: 20, litter: 1, rate: 0.05 };
    g.lifecycle = LifecycleDef { max_age: Some(700), maturity: 20 };
    g.perception = 6.0;
    g.initial.count = 60;
    sys.entity_types.push(g);
    sys.relationships.push(Relationship {
        subject: "grazer".into(),
        kind: RelationKind::Consumes { efficiency: 0.3, hunger: Some(25.0), radius: 6.0, bite: 1.0, handling: 25 },
        object: "plant".into(),
        priority_shift: 0,
    });
}

/// Step 7: a hand-written rule (full grazers rest).
pub fn add_rest_rule(sys: &mut LifeSystem) {
    add_grazers(sys);
    sys.rules.retain(|r| !(r.name == "rest" && r.entity.text == "grazer"));
    if let Ok(rule) = parse_rule(REST_RULE) {
        sys.rules.push(rule);
    }
}

/// Step 9: hunters that eat grazers, which flee from them.
pub fn add_hunters(sys: &mut LifeSystem) {
    add_grazers(sys);
    if sys.type_index("hunter").is_some() {
        return;
    }
    immigration(sys, "hunter migration", "hunter", 0.005, 1);
    let mut h = EntityType::new("hunter");
    h.description = "Secondary consumer: eats grazers (50 % assimilation, one grazer per 30 ticks at most).".into();
    h.color = [0.80, 0.25, 0.20, 1.0];
    h.icon = "🐺".into();
    h.size = 1.2;
    h.movement = MovementDef::Directed { speed: 0.75, turn: 0.5 };
    h.energy.initial = 40.0;
    h.energy.max = 60.0;
    h.energy.metabolism = 0.06;
    h.reproduction =
        ReproductionDef { mode: ReproductionMode::Asexual, cost: 15.0, cooldown: 40, litter: 1, rate: 0.1 };
    h.lifecycle = LifecycleDef { max_age: Some(900), maturity: 40 };
    h.perception = 8.0;
    h.initial.count = 10;
    sys.entity_types.push(h);
    sys.relationships.push(Relationship {
        subject: "hunter".into(),
        kind: RelationKind::Consumes { efficiency: 0.5, hunger: Some(45.0), radius: 8.0, bite: 1.0, handling: 30 },
        object: "grazer".into(),
        priority_shift: 0,
    });
    sys.relationships.push(Relationship {
        subject: "grazer".into(),
        kind: RelationKind::Avoids { radius: 3.0, speed_factor: 1.2 },
        object: "hunter".into(),
        priority_shift: 0,
    });
}

/// The tutorial and concepts of the mode.
pub fn tutorial() -> Tutorial {
    Tutorial {
        pages: vec![
            TutorialPage::new(
                "Design life without code",
                "The **Custom Life Creator** runs a world whose *entity types*, *relationships*, *rules* and *fields* \
                 are all data you edit. In this tutorial you build a small food web — **sunlight, plants, grazers \
                 and hunters** — one step at a time.\n\n\
                 Every step has a button below. The **system editor** (left panel, or **E**) shows what each step \
                 changed; the **Apply** bar tells you whether a change applies *next tick* or *at the next reset*.",
            )
            .with_action("tutorial:start", "Start an empty 48 × 48 world"),
            TutorialPage::new(
                "1 · A resource field",
                "Resources are numbers stored per cell. Each tick, per cell: `R := R·(1 − decay)`, then \
                 `R := min(max, R + regen)` while below `max`, then diffusion `R := R + D·Σ(R_n − R)` over the four \
                 neighbours (stable for `D ≤ 0.25`).\n\n\
                 The step adds **sunlight**: full (1.0) everywhere, regrowing 0.06 per tick, diffusing with `D = 0.2`. \
                 Look at it in the *World* tab of the editor.",
            )
            .with_action("tutorial:sunlight", "Add sunlight"),
            TutorialPage::new(
                "2 · Producers",
                "An **entity type** has a colour and glyph, a size (diameter), an energy budget (initial, maximum, \
                 metabolism per tick), reproduction, ageing and an initial population.\n\n\
                 **plant** is sessile, pays `metabolism 0.04` per tick and gains energy through the relationship \
                 **plant consumes sunlight** (bite 0.3 while its energy is below 14). A hand-written rule lets it \
                 spread: `when age >= 10 and energy >= 10 and nearby(plant, radius=1.5) < 4 then \
                 reproduce(probability=0.1)`. A *seed rain* random event (3 plants with probability 0.1 per tick) \
                 stands for seeds blown in from outside. Reproduction never creates energy: the parent pays the cost (5) and \
                 the offspring receives it.",
            )
            .with_action("tutorial:plants", "Add plants"),
            TutorialPage::new(
                "3 · Apply and run",
                "Adding or removing entity types, states, properties, carried resources or resource fields — or \
                 changing the world size or topology — changes the *shape* of the world, so it **requires a reset**: \
                 the edit is stored and used from the next reset on. Everything else (rules, relationships, \
                 parameters, colours…) **applies from the next tick** and keeps the living entities.\n\n\
                 The button applies the draft, restarts the world from it and starts the clock. Watch the plants \
                 fill the world until sunlight limits them.",
            )
            .with_action("tutorial:apply", "Apply, reset and play"),
            TutorialPage::new(
                "4 · Consumers",
                "**grazer** moves by a correlated random walk (speed 0.6, turn ≤ 0.7 rad per tick) and **consumes \
                 plant** with efficiency 0.3 (it keeps 30 % of a plant's energy; the rest is lost as heat), only \
                 while hungry (energy < 25) and at most once every 25 ticks (*handling time*). Its reproduction \
                 rate 0.05 generates a rule too, and a *grazer migration* event brings 2 grazers with \
                 probability 0.02 per tick.\n\n\
                 A new type requires a reset, so this step restarts the world.",
            )
            .with_action("tutorial:grazers", "Add grazers and restart"),
            TutorialPage::new(
                "5 · How relationships become rules",
                "Relationships compile into ordinary rules you can read in the *Rules* tab under **Generated \
                 rules**. `grazer consumes plant` becomes `forage` (move towards the nearest plant while hungry, \
                 priority 500) and `eat` (`on_collision with plant … then eat(other, efficiency=0.3)`, priority \
                 520, cooldown 25).\n\n\
                 Every tick each entity tries its rules by **priority** (higher first). It has one *movement*, \
                 one *exclusive* (eat, attack, reproduce, die, trade) and one *transformation* slot; a rule whose \
                 slot is taken is skipped as a whole. If no rule moved the entity, its default movement runs.",
            )
            .with_action("editor:rules", "Open the rule editor"),
            TutorialPage::new(
                "6 · Write a rule visually",
                "The **visual builder** edits a rule as blocks: an event, condition rows (each group is *all of*; \
                 groups are joined by **OR**), actions with their parameters, and modifiers (priority, \
                 probability, cooldown, costs, products, once). The DSL text below it is generated as you edit.\n\n\
                 This step adds `rule rest for grazer: when energy > 35 then stay() priority 200` — full grazers \
                 stop wandering — and applies it. A rule change applies **next tick**, no reset needed.",
            )
            .with_action("tutorial:rule", "Add the rule and apply it"),
            TutorialPage::new(
                "7 · Live validation",
                "Switch the rule to **DSL text** to type it directly. Every keystroke is parsed and checked against \
                 your definitions; problems are underlined and listed with `line:column`, a code and a hint.\n\n\
                 This step opens the rule with a typo (`enrgy`). The validator reports **E001 unknown property** \
                 with *did you mean `energy`?* Fix the word: the error disappears and the draft is valid again. \
                 While a rule has errors, Apply is disabled.",
            )
            .with_action("tutorial:error", "Show a rule with an error"),
            TutorialPage::new(
                "8 · Predators and the graph",
                "**hunter** consumes grazers (efficiency 0.5, handling 30), and **grazer avoids hunter** (flee \
                 within 3 units at 1.2 × speed, priority 900 — above eating, so fleeing wins).\n\n\
                 The *Relationships* tab draws the **graph**: types as circles placed by trophic level, resources as \
                 squares at the bottom, typed coloured edges (legend below it). Drag nodes to pin them.",
            )
            .with_action("tutorial:hunters", "Add hunters, restart and show the graph"),
            TutorialPage::new(
                "9 · Reading the metrics",
                "The **Metrics** tab plots, per tick: `population` and `population.<type>` (live entities), \
                 `energy.<type>` (total energy held by the type — the *standing energy*), `births.<type>` / \
                 `deaths.<type>` (counters of the last tick), `resource.<field>` (sum over all cells), \
                 `consumed.<field>`, `eaten`, `killed`, `refused` (interactions lost to a competitor) and one \
                 `rule.…` counter per rule (firings in the last tick).\n\n\
                 In this food web the standing energy shrinks up the chain — plants > grazers > hunters — because \
                 each meal keeps only part of the prey's energy and everyone pays maintenance (Lindeman's pyramid).",
            )
            .with_action("tab:metrics", "Open the Metrics tab"),
            TutorialPage::new(
                "Simple example · Bacterial growth",
                "One type, two rules: `feed` (consume nutrient while energy < 18) and `divide` (reproduce at energy \
                 ≥ 12). The nutrient regrows 0.01 per cell and tick on 48 × 48 = 2304 cells, and each bacterium \
                 pays 0.08 per tick, so the population levels off near **K = 0.01 · 2304 / 0.08 ≈ 288**: logistic \
                 growth that nobody programmed — it emerges from resource limitation. Show the *nutrient* overlay \
                 (V) to see the depleted field.",
            )
            .with_action("load:bacterial-growth", "Open bacterial growth"),
            TutorialPage::new(
                "Advanced example · Robot swarm (Rhai)",
                "Robots wander; on contact a DSL rule stops them (`set_state(waiting), stay()`). A **Rhai script** \
                 then keeps a waiting robot still for `10 + 25 × neighbours` ticks and drops a *beacon* marker. \
                 Larger groups hold robots longer, so clusters grow: aggregation from local rules (BEECLUST).\n\n\
                 Scripts run in a sandbox (no files, network or clock; an operation budget per call). A runtime \
                 error disables the script and is listed with its position in the *Scripts* tab.",
            )
            .with_action("example:robot-swarm", "Open the robot swarm and its script"),
            TutorialPage::new(
                "Suggested experiments",
                "- **Trophic cascade**: in *Producers & consumers*, set the hunters' initial count to 0 (Apply & \
                 reset) and compare plant and grazer energy in the Metrics tab.\n\
                 - **Carrying capacity**: in *Bacterial growth*, double `resource.nutrient.regen` (Parameters tab, \
                 applies next tick): the plateau roughly doubles.\n\
                 - **Handling time**: lower the grazers' handling time from 25 to 5 and watch the plants crash.\n\
                 - **Topology**: switch the world to *Bounded* (World tab, requires a reset) and compare.\n\
                 - **Competition**: in *Competitive colonies*, give violet the lower maintenance and see who wins.\n\
                 - **Paint**: with the Paint tool, add sunlight in a stripe and watch plants follow it.",
            )
            .with_action("load:producers-consumers", "Open producers & consumers"),
        ],
        concepts: CONCEPTS.into(),
    }
}

/// Concepts page (Markdown).
pub const CONCEPTS: &str = "## Entities and energy\n\
Entities have continuous positions (one world unit = one cell), a diameter and an energy store. Each tick they pay \
their metabolism; with starvation on they die at energy ≤ 0, and at `max_age`. Eating gains `efficiency × prey \
energy`; consuming a resource gains `amount × energy_value`. Reproduction moves energy from parents to offspring and \
never creates it.\n\n\
## Relationships and rules\n\
A relationship `A <kind> B` compiles into ordinary rules (visible in the Rules tab). Priority bands: avoid 900, \
protect 700, transform 600, consume 500–520, chase 450, compete 400, reproduce 300, trade 200, produce 100; a \
relationship's *priority shift* (±99) moves its rules within the band.\n\n\
A rule fires when its event is active, its cooldown has expired, its costs are affordable, its condition holds, its \
probability roll succeeds and every action can be carried out — and its movement / exclusive / transformation slots \
are still free. Contested interactions (eat, attack, sexual reproduction, trade) are matched one-to-one by priority, \
then distance, then id.\n\n\
## When edits apply\n\
`classify_edit` decides: changes to the *shape* of the world (types, their states, properties or carried resources; \
resource fields; size; topology) **require a reset**; everything else **applies next tick** and keeps the entities \
(cooldowns restart, values are clamped). Initial populations only matter at a reset.\n\n\
## Fields\n\
Resource fields decay, regrow up to a maximum and diffuse (explicit 5-point scheme, `D ≤ 0.25`, conserving the \
total). Consumption requests on one cell are shared proportionally, so a field never goes negative. Environment \
fields (gradients, radial patterns, optional seasonal modulation) are analytic and read-only.\n\n\
## Emergent carrying capacity\n\
When uptake is limited by a regrowing field, a steady population satisfies `N · metabolism = supply`, so \
`K = regen · cells / metabolism` — the logistic curve emerges instead of being imposed.\n\n\
## Scripts\n\
Rhai scripts define `fn run(me)` and only *record intents*, resolved like DSL actions after the type's rules. They \
run sandboxed with an operation budget; any runtime error disables the script and is reported with its position.\n\n\
## Topologies\n\
*Torus* wraps both edges; *bounded* reflects at walls; the *Klein bottle* mirrors x when crossing the top/bottom edge; \
the *cross-surface* (projective plane) mirrors both crossings. Distances and sensing follow the topology.";

/// Reacts to tutorial buttons (and the generic action ids).
#[allow(clippy::too_many_arguments)]
pub fn handle_actions(
    mut actions: MessageReader<TutorialAction>,
    runner: Option<Res<ClRunner>>,
    presets: Res<ClScenarios>,
    mut editor: ResMut<ClEditor>,
    mut data: ResMut<ClData>,
    mut ui: ResMut<ClUi>,
    mut history: ResMut<MetricsHistory>,
    mut list: ResMut<ModeScenarios>,
    mut toasts: ResMut<Toasts>,
    mut shell: MessageWriter<ShellCommand>,
) {
    let ids: Vec<String> = actions.read().map(|a| a.0.clone()).collect();
    let Some(runner) = runner else { return };
    for id in ids {
        let (verb, arg) = id.split_once(':').unwrap_or((id.as_str(), ""));
        let restart = |editor: &mut ClEditor,
                       data: &mut ClData,
                       ui: &mut ClUi,
                       history: &mut MetricsHistory,
                       list: &mut ModeScenarios,
                       toasts: &mut Toasts| {
            let (draft, seed) = (editor.draft.clone(), editor.seed);
            match start_system(&runner, draft, seed, data, ui, history, list) {
                Ok(()) => data.play_when_ready(),
                Err(e) => toasts.error(e),
            }
        };
        match (verb, arg) {
            ("load", preset) | ("example", preset) => {
                match open_preset(preset, &presets, &runner, &mut editor, &mut data, &mut ui, &mut history, &mut list) {
                    Ok(()) if verb == "example" => {
                        editor.open_at(EditorTab::Scripts);
                        editor.sel_script = Some(0);
                    }
                    Ok(()) => {}
                    Err(e) => toasts.error(e),
                }
            }
            ("tutorial", "start") => {
                editor.load(tutorial_world(), TUTORIAL_SEED, "Tutorial");
                let (draft, seed) = (editor.draft.clone(), editor.seed);
                if let Err(e) = start_system(&runner, draft, seed, &mut data, &mut ui, &mut history, &mut list) {
                    toasts.error(e);
                }
                editor.open_at(EditorTab::World);
            }
            ("tutorial", "sunlight") => {
                add_sunlight(&mut editor.draft);
                editor.touch();
                editor.open_at(EditorTab::World);
                ui.overlay = Some("sunlight".into());
            }
            ("tutorial", "plants") => {
                add_plants(&mut editor.draft);
                editor.touch();
                editor.sel_type = editor.draft.type_index("plant").unwrap_or(0);
                editor.open_at(EditorTab::Entities);
            }
            ("tutorial", "apply") => {
                add_plants(&mut editor.draft);
                editor.touch();
                editor.refresh();
                if editor.has_errors() {
                    toasts.error("The draft has errors; fix them in the editor first.");
                } else {
                    restart(&mut editor, &mut data, &mut ui, &mut history, &mut list, &mut toasts);
                    ui.overlay = Some("sunlight".into());
                }
            }
            ("tutorial", "grazers") => {
                add_grazers(&mut editor.draft);
                editor.touch();
                editor.sel_type = editor.draft.type_index("grazer").unwrap_or(0);
                editor.open_at(EditorTab::Entities);
                restart(&mut editor, &mut data, &mut ui, &mut history, &mut list, &mut toasts);
            }
            ("tutorial", "rule") => {
                add_rest_rule(&mut editor.draft);
                editor.touch();
                editor.refresh();
                editor.sel_rule = editor.draft.rules.iter().position(|r| r.name == "rest");
                editor.sel_generated = None;
                editor.rules.mode = RuleMode::Visual;
                editor.open_at(EditorTab::Rules);
                if !editor.has_errors() {
                    runner.edit(rule_engine::CustomLifeCommand::ReplaceSystem(Box::new(editor.draft.clone())));
                    data.refresh_info();
                }
            }
            ("tutorial", "error") => {
                add_rest_rule(&mut editor.draft);
                let i = editor.draft.rules.iter().position(|r| r.name == "rest");
                editor.sel_rule = i;
                editor.sel_generated = None;
                editor.rules.mode = RuleMode::Text;
                let draft = editor.draft.clone();
                editor.rules.sync(&draft.rules, i, &draft);
                editor.rules.text = BROKEN_RULE.into();
                if let (Some(i), Some(rule)) = (i, editor.rules.take_text(&draft)) {
                    editor.draft.rules[i] = rule;
                }
                editor.touch();
                editor.open_at(EditorTab::Rules);
            }
            ("tutorial", "hunters") => {
                add_hunters(&mut editor.draft);
                editor.touch();
                editor.open_at(EditorTab::Relationships);
                editor.sel_rel = editor.draft.relationships.iter().position(|r| r.subject == "hunter");
                restart(&mut editor, &mut data, &mut ui, &mut history, &mut list, &mut toasts);
            }
            ("editor", tab) => match EditorTab::from_name(tab) {
                Some(tab) => editor.open_at(tab),
                None => editor.open = !editor.open,
            },
            ("tab", tab) => {
                let tab = match tab {
                    "parameters" => RightPanelTab::Parameters,
                    "rules" => RightPanelTab::Rules,
                    "metrics" => RightPanelTab::Metrics,
                    _ => RightPanelTab::Properties,
                };
                shell.write(ShellCommand::ShowRightTab(tab));
            }
            ("overlay", field) => ui.overlay = (!field.is_empty() && field != "none").then(|| field.to_string()),
            ("select", _) => {
                let first = data.view.as_ref().and_then(|v| {
                    let wanted = v.legend.iter().position(|l| l.name == arg).map(|t| t as u16);
                    v.entities.iter().find(|e| wanted.is_none_or(|t| e.type_index == t)).map(|e| e.id)
                });
                ui.selected = first;
            }
            ("tool", name) => {
                if let Some(t) = Tool::ALL.into_iter().find(|t| t.label().eq_ignore_ascii_case(name)) {
                    ui.tool = t;
                }
            }
            ("color", mode) => {
                if let Some(c) = ColorBy::ALL.into_iter().find(|c| c.label().eq_ignore_ascii_case(mode)) {
                    ui.color_by = c;
                }
            }
            ("play", _) => runner.send(Control::Play),
            _ => warn!("unknown tutorial action `{id}`"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rule_engine::{CustomLifeSim, compile, scenarios};
    use simulation_core::Simulation;

    #[test]
    fn every_step_compiles_and_the_food_web_lives() {
        let mut sys = tutorial_world();
        compile(&sys).unwrap();
        add_sunlight(&mut sys);
        compile(&sys).unwrap();
        add_plants(&mut sys);
        compile(&sys).unwrap();
        add_grazers(&mut sys);
        compile(&sys).unwrap();
        add_rest_rule(&mut sys);
        compile(&sys).unwrap();
        add_hunters(&mut sys);
        let compiled = compile(&sys).unwrap();
        assert_eq!(compiled.types().len(), 3);
        // Steps are idempotent.
        let before = sys.clone();
        add_hunters(&mut sys);
        add_plants(&mut sys);
        assert_eq!(sys, before);
        let mut sim = CustomLifeSim::new(sys, TUTORIAL_SEED).unwrap();
        sim.advance(1500);
        for name in ["plant", "grazer", "hunter"] {
            assert!(sim.population(name) > 0, "{name} persists over 1500 ticks");
        }
    }

    #[test]
    fn the_grazer_stage_does_not_collapse() {
        let mut sys = tutorial_world();
        add_rest_rule(&mut sys);
        let mut sim = CustomLifeSim::new(sys, TUTORIAL_SEED).unwrap();
        let mut min_plants = usize::MAX;
        for _ in 0..12 {
            sim.advance(100);
            min_plants = min_plants.min(sim.population("plant"));
        }
        assert!(sim.population("plant") > 50, "plants recover: {}", sim.population("plant"));
        assert!(sim.population("grazer") > 0);
        let _ = min_plants;
    }

    #[test]
    fn the_broken_rule_reports_the_typo_with_its_position() {
        let mut sys = tutorial_world();
        add_rest_rule(&mut sys);
        let (rule, diags) = rule_engine::check_rule_text(&sys, BROKEN_RULE);
        assert!(rule.is_some());
        let d = diags.iter().find(|d| d.code == "E001").expect("unknown property");
        assert_eq!(d.span.map(|s| (s.line, s.col)), Some((2, 10)));
        assert_eq!(d.help.as_deref(), Some("did you mean `energy`?"));
    }

    #[test]
    fn tutorial_facts_match_the_presets() {
        let bacteria = scenarios::builtin_by_id("bacterial-growth").unwrap().system;
        let r = &bacteria.world.resources[0];
        let k = f64::from(r.regen) * f64::from(bacteria.world.width * bacteria.world.height)
            / bacteria.entity_types[0].energy.metabolism;
        assert!((k - 288.0).abs() < 1.0, "{k}");
        let preset = scenarios::builtin_by_id("producers-consumers").unwrap().system;
        let mut mine = tutorial_world();
        add_hunters(&mut mine);
        for name in ["plant", "grazer", "hunter"] {
            let a = &mine.entity_types[mine.type_index(name).unwrap()];
            let b = &preset.entity_types[preset.type_index(name).unwrap()];
            assert_eq!(
                (a.energy.clone(), a.reproduction.clone(), a.movement.clone()),
                (b.energy.clone(), b.reproduction.clone(), b.movement.clone()),
                "{name}"
            );
        }
        assert_eq!(mine.world.random_events, preset.world.random_events);
        let pages = tutorial().pages;
        assert!(pages.len() >= 12);
        assert!(pages.iter().all(|p| p.action.is_some()));
    }
}
