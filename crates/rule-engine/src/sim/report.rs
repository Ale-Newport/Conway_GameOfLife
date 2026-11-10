//! Descriptor, metrics and view construction.

use std::fmt::Write as _;

use simulation_core::glam::Vec2;
use simulation_core::{MetricSpec, Metrics, ModelDescriptor, ModelFamily};

use crate::compile::CompiledSystem;
use crate::compile::ir::REvent;
use crate::sim::frame::env_value;
use crate::sim::space::Space;
use crate::sim::view::{
    CustomLifeView, CustomLifeViewRequest, EntityView, FieldView, LegendEntry, RuleListing, downsample,
};
use crate::sim::{MODEL_ID, State, params};

/// The executed tick pipeline, shown in the rule panel above the rules.
const PIPELINE: &str = "\
Each tick (one time unit):

1. **Freeze** the world (entities, fields) and index positions.
2. **Decide**, for every entity in id order: rules whose event is active (`on_birth` on the first tick of life, \
`on_state_enter` on the tick after a state change, `on_collision` while touching another entity — `other` is the \
nearest touching entity —, `on_tick` always) in priority order (higher first, then declaration order); then the \
type's Rhai scripts; then, if no movement was chosen, the type's default movement (random walk or correlated random \
walk). A rule fires when its cooldown has expired, its costs are affordable, its condition holds, its probability \
roll succeeds and every action can be carried out (targets exist; `eat`, `attack` and `trade` targets touch the \
entity; a trade needs both parties to carry both resources, hold the amounts and have room for what they receive; \
reproduction needs maturity, the reproduction cooldown and energy > cost). Each entity has one movement \
slot, one exclusive slot (`eat`, `attack`, `reproduce`, `die`, `trade`) and one transformation slot \
(`transform_into`); a rule whose slot is taken is skipped as a whole. Randomness comes from streams keyed by \
(seed, tick, entity, rule).
3. **Resolve**: contested interactions (`eat`, `attack`, sexual `reproduce`, `trade`) are matched one-to-one by \
priority, then distance, then id; every entity takes part in at most one interaction per tick, and a firing whose \
interaction is refused is cancelled entirely (no cost, no cooldown, no effects). Firings that would exceed the \
entity cap are cancelled. Then, in order: costs and cooldowns; `set`/`add`/`set_state`; eating (prey dies, eater \
gains prey energy × efficiency), attacks (victim loses energy, dies at ≤ 0), trades (atomic), reproduction (each \
parent pays the cost, offspring share it equally) — an entity eaten or killed by an interaction carries out no \
further exclusive action (it does not reproduce or die a second time); `consume` (requests on one cell are shared proportionally; \
carried resources fill the stock, others become energy × energy value); `emit`; metabolism; movement \
(semi-implicit Euler with the entity speed as limit, then the topology); transformations; births and spawns; deaths \
(`die`, eaten, killed, energy ≤ 0 when starvation is on, age ≥ max age) followed by `on_death` rules (only `emit` \
and `spawn`, evaluated on the end-of-tick state); carried resources of the dead are dropped; ageing.
4. **Fields**: decay, regrowth up to the maximum, explicit diffusion (zero flux at bounded edges).
5. **Random events** (catastrophe victims die like any other entity: `on_death` rules run, carried resources are \
dropped), then **end conditions**.";

fn sanitize(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).collect()
}

fn rule_metric_key(index: usize, entity: &str, name: &str) -> String {
    format!("rule.{index}.{}.{}", sanitize(entity), sanitize(name))
}

/// Metric declarations for a compiled system.
pub fn metric_specs(compiled: &CompiledSystem) -> Vec<MetricSpec> {
    let mut specs = vec![
        MetricSpec::gauge("population", "Population", "entities", "Live entities of all types."),
        MetricSpec::counter("births", "Births", "Entities born or spawned during the last tick."),
        MetricSpec::counter("deaths", "Deaths", "Entities that died during the last tick."),
        MetricSpec::counter("rule_firings", "Rule firings", "Rules that fired during the last tick."),
        MetricSpec::gauge("script_errors", "Script errors", "errors", "Scripts disabled by a runtime error."),
        MetricSpec::counter("eaten", "Eaten", "Successful eat actions in the last tick."),
        MetricSpec::counter("killed", "Killed", "Entities killed by attacks in the last tick."),
        MetricSpec::counter("trades", "Trades", "Completed trades in the last tick."),
        MetricSpec::counter("refused", "Refused interactions", "Firings cancelled because their target was taken."),
        MetricSpec::counter("births_refused", "Refused births", "Births and spawns refused by the entity cap."),
        MetricSpec::counter("random_events", "Random events", "Random events that happened in the last tick."),
    ];
    for t in compiled.types() {
        let n = &t.def.name;
        specs.push(MetricSpec::gauge(
            &format!("population.{n}"),
            &format!("{n} population"),
            "entities",
            "Live entities of the type.",
        ));
        specs.push(MetricSpec::gauge(
            &format!("energy.{n}"),
            &format!("{n} energy"),
            "energy",
            "Total energy held by the type.",
        ));
        specs.push(MetricSpec::counter(
            &format!("births.{n}"),
            &format!("{n} births"),
            "Births of the type in the last tick.",
        ));
        specs.push(MetricSpec::counter(
            &format!("deaths.{n}"),
            &format!("{n} deaths"),
            "Deaths of the type in the last tick.",
        ));
    }
    for r in &compiled.source().world.resources {
        let n = &r.name;
        specs.push(MetricSpec::gauge(
            &format!("resource.{n}"),
            &format!("{n} total"),
            "units",
            "Sum of the field over all cells.",
        ));
        specs.push(MetricSpec::counter(
            &format!("consumed.{n}"),
            &format!("{n} consumed"),
            "Amount consumed in the last tick.",
        ));
    }
    for (i, r) in compiled.rules().iter().enumerate() {
        let entity = &compiled.types()[usize::from(r.entity)].def.name;
        specs.push(MetricSpec::counter(
            &rule_metric_key(i, entity, &r.name),
            &format!("{entity}: {}", r.name),
            "Firings of this rule in the last tick.",
        ));
    }
    specs
}

/// Formal description including the current rules.
pub fn descriptor(compiled: &CompiledSystem) -> ModelDescriptor {
    let sys = compiled.source();
    let mut rules = format!(
        "## Custom life: {}\n\n{PIPELINE}\n\n### Rules (per entity type, in evaluation order)\n\n```text\n",
        sys.name
    );
    rules.push_str(&compiled.rules_text());
    rules.push_str("\n```\n");
    if !compiled.scripts().scripts().is_empty() {
        rules.push_str("\n### Scripts (run after the rules of each entity)\n\n");
        for s in compiled.scripts().scripts() {
            let entity = &compiled.types()[usize::from(s.entity)].def.name;
            let event = match s.event {
                REvent::Tick => "on_tick".to_string(),
                REvent::Birth => "on_birth".to_string(),
                REvent::Death => "on_death".to_string(),
                REvent::Collision(_) => "on_collision".to_string(),
                REvent::StateEnter(_) => "on_state_enter".to_string(),
            };
            let _ = writeln!(rules, "- `{}` for `{entity}`, {event}, priority {}", s.name, s.priority);
        }
    }
    let summary = if sys.description.is_empty() {
        "User-defined entities, relationships, rules and scripts on a grid world with resource fields.".to_string()
    } else {
        sys.description.clone()
    };
    ModelDescriptor::new(MODEL_ID, "Custom Life Creator", ModelFamily::Hybrid)
        .summary(&summary)
        .rules(&rules)
        .params(params::param_specs(sys))
        .metrics(metric_specs(compiled))
        .time("tick", 1.0)
}

/// Measurements of the current state (counters refer to the last tick).
pub fn metrics(state: &State, compiled: &CompiledSystem) -> Metrics {
    let s = &state.stats;
    let mut m = Metrics::new(state.tick, state.tick as f64);
    let types = compiled.types().len();
    let mut pop = vec![0u32; types];
    let mut energy = vec![0f64; types];
    for e in state.store.as_slice() {
        pop[usize::from(e.kind)] += 1;
        energy[usize::from(e.kind)] += e.energy;
    }
    let sum = |v: &[u32]| v.iter().map(|&x| f64::from(x)).sum::<f64>();
    m.push("population", state.store.len() as f64)
        .push("births", sum(&s.births))
        .push("deaths", sum(&s.deaths))
        .push("rule_firings", sum(&s.rule_fired))
        .push("script_errors", state.script_disabled.iter().filter(|d| **d).count() as f64)
        .push("eaten", f64::from(s.eaten))
        .push("killed", f64::from(s.killed))
        .push("trades", f64::from(s.trades))
        .push("refused", f64::from(s.interactions_refused))
        .push("births_refused", s.births_refused as f64)
        .push("random_events", f64::from(s.random_events));
    for (k, t) in compiled.types().iter().enumerate() {
        let n = &t.def.name;
        m.push(&format!("population.{n}"), f64::from(pop[k]));
        m.push(&format!("energy.{n}"), energy[k]);
        m.push(&format!("births.{n}"), f64::from(s.births.get(k).copied().unwrap_or(0)));
        m.push(&format!("deaths.{n}"), f64::from(s.deaths.get(k).copied().unwrap_or(0)));
    }
    for (r, def) in compiled.source().world.resources.iter().enumerate() {
        m.push(&format!("resource.{}", def.name), state.fields[r].sum());
        m.push(&format!("consumed.{}", def.name), s.consumed.get(r).copied().unwrap_or(0.0));
    }
    for (i, r) in compiled.rules().iter().enumerate() {
        let entity = &compiled.types()[usize::from(r.entity)].def.name;
        m.push(&rule_metric_key(i, entity, &r.name), f64::from(s.rule_fired.get(i).copied().unwrap_or(0)));
    }
    m
}

/// Builds the view.
pub fn view(
    state: &State,
    compiled: &CompiledSystem,
    req: &CustomLifeViewRequest,
    reset_pending: bool,
) -> CustomLifeView {
    let sys = compiled.source();
    let (w, h) = (sys.world.width as usize, sys.world.height as usize);
    let mut counts = vec![0u32; compiled.types().len()];
    let entities = state
        .store
        .iter()
        .map(|(id, e)| {
            counts[usize::from(e.kind)] += 1;
            let ty = &compiled.types()[usize::from(e.kind)];
            EntityView {
                id: id.get(),
                type_index: e.kind,
                position: e.kin.position,
                heading: e.kin.heading,
                state: e.state,
                size: e.size,
                color: ty.def.color,
                energy: (e.energy / ty.def.energy.max).clamp(0.0, 1.0) as f32,
            }
        })
        .collect();
    let mut fields = Vec::new();
    if req.include_fields {
        let res = req.field_resolution.max(1) as usize;
        let make = |name: &str, resource: bool, values: &[f32], color: [f32; 4]| {
            let (values, fw, fh) = downsample(values, w, h, res);
            let (min, max) = values.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), &v| (a.min(v), b.max(v)));
            FieldView {
                name: name.to_string(),
                resource,
                width: fw as u32,
                height: fh as u32,
                cell_width: w as f32 / fw as f32,
                cell_height: h as f32 / fh as f32,
                values,
                min: if min.is_finite() { min } else { 0.0 },
                max: if max.is_finite() { max } else { 0.0 },
                color,
            }
        };
        for (r, def) in sys.world.resources.iter().enumerate() {
            fields.push(make(&def.name, true, state.fields[r].as_slice(), def.color));
        }
        // Environment fields are analytic: sample them directly at the centre
        // of every output block instead of evaluating every cell.
        let space = Space::new(sys.world.topology, sys.world.width, sys.world.height);
        let (fx, fy) = (w.div_ceil(res).max(1), h.div_ceil(res).max(1));
        let (fw, fh) = (w.div_ceil(fx), h.div_ceil(fy));
        let centre = |i: usize, f: usize, n: usize| (i * f + (i * f + f).min(n)) as f32 / 2.0;
        for def in &sys.world.environment {
            let mut values = Vec::with_capacity(fw * fh);
            for by in 0..fh {
                for bx in 0..fw {
                    let p = Vec2::new(centre(bx, fx, w), centre(by, fy, h));
                    values.push(env_value(def, p, state.tick, &space) as f32);
                }
            }
            let (min, max) = values.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), &v| (a.min(v), b.max(v)));
            fields.push(FieldView {
                name: def.name.clone(),
                resource: false,
                width: fw as u32,
                height: fh as u32,
                cell_width: w as f32 / fw as f32,
                cell_height: h as f32 / fh as f32,
                values,
                min: if min.is_finite() { min } else { 0.0 },
                max: if max.is_finite() { max } else { 0.0 },
                color: [0.9, 0.8, 0.3, 1.0],
            });
        }
    }
    let legend = compiled
        .types()
        .iter()
        .enumerate()
        .map(|(i, t)| LegendEntry {
            index: i as u16,
            name: t.def.name.clone(),
            description: t.def.description.clone(),
            color: t.def.color,
            icon: t.def.icon.clone(),
            sprite: t.def.sprite.clone(),
            states: t.states.clone(),
            count: counts[i],
        })
        .collect();
    let rules = if req.include_rules {
        compiled
            .types()
            .iter()
            .flat_map(|t| t.rules.iter().map(move |&r| (t, r as usize)))
            .map(|(t, r)| {
                let rule = &compiled.rules()[r];
                RuleListing {
                    entity: t.def.name.clone(),
                    name: rule.name.clone(),
                    origin: rule.origin.clone(),
                    text: rule.text.clone(),
                    fired_total: state.fired_total.get(r).copied().unwrap_or(0),
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    CustomLifeView {
        tick: state.tick,
        width: sys.world.width,
        height: sys.world.height,
        topology: sys.world.topology,
        entities,
        fields,
        legend,
        graph: compiled.graph().clone(),
        rules,
        script_errors: state.script_errors.clone(),
        warnings: compiled.warnings().iter().map(ToString::to_string).collect(),
        diagnostics: compiled.warnings().to_vec(),
        reset_pending,
        finished: state.finished.clone(),
        recent_events: state.events.recent(req.recent_events).cloned().collect(),
    }
}
