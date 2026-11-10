//! `info <model-or-scenario>`.

use model_registry::{Registry, ScenarioInfo, ScenarioSource};
use serde::Serialize;
use simulation_core::params::ParamKind;
use simulation_core::{MetricKind, ModelDescriptor, ParamSet, ParamSpec};

use crate::commands::build;
use crate::output::{out, outln};
use crate::text::{Align, Table, param_value, short_path};

#[derive(Serialize)]
struct InfoJson<'a> {
    scenario: &'a ScenarioInfo,
    model: &'a model_registry::ModelSpec,
    descriptor: &'a ModelDescriptor,
    values: &'a ParamSet,
    scenarios: Vec<&'a str>,
}

/// Choice parameters with more options than this list them below the table.
const CHOICES_INLINE: usize = 4;

/// Type and admissible values of a parameter.
fn kind_text(spec: &ParamSpec) -> String {
    match &spec.kind {
        ParamKind::Bool => "bool".to_string(),
        ParamKind::Int { min, max } => format!("int [{min}, {max}]"),
        ParamKind::Float { min, max, logarithmic, .. } => {
            format!("float [{min}, {max}]{}", if *logarithmic { " log" } else { "" })
        }
        ParamKind::Text { max_len } => format!("text (max {max_len} chars)"),
        ParamKind::Choice { options } if options.len() > CHOICES_INLINE => {
            format!("choice ({} options, listed below)", options.len())
        }
        ParamKind::Choice { options } => {
            format!("one of {}", options.iter().map(|o| o.value.as_str()).collect::<Vec<_>>().join("|"))
        }
        ParamKind::Color => "colour #rrggbb[aa]".to_string(),
    }
}

fn metric_kind(kind: MetricKind) -> &'static str {
    match kind {
        MetricKind::Gauge => "gauge",
        MetricKind::Counter => "counter",
        MetricKind::Ratio => "ratio",
    }
}

/// `info TARGET`: model and scenario identity, parameters (with the
/// scenario's values), metrics and the formal rules, all read from the
/// model built for the target.
///
/// # Errors
/// Unknown targets and build failures.
pub fn info(registry: &Registry, target: &str, json: bool) -> anyhow::Result<()> {
    let built = build(registry, &model_registry::BuildRequest::new(target))?;
    let descriptor = built.sim.descriptor();
    let values = built.sim.params();
    let spec = registry.model(&built.scenario.model_id)?;
    let scenarios: Vec<&str> = registry.scenarios_of(spec.id).map(|s| s.info.id.as_str()).collect();
    if json {
        let doc =
            InfoJson { scenario: &built.scenario, model: spec, descriptor: &descriptor, values: &values, scenarios };
        outln!("{}", serde_json::to_string_pretty(&doc)?);
        return Ok(());
    }
    let s = &built.scenario;
    outln!("Model      {} — {} ({})", descriptor.id, descriptor.name, descriptor.family.label());
    outln!("Crate      {} (documentation: {})", spec.crate_name, spec.docs);
    let source = match &s.source {
        ScenarioSource::Builtin(_) => s.source.label(),
        ScenarioSource::File(path) => short_path(path),
    };
    outln!("Scenario   {} — {} (seed {}, {source})", s.id, s.title, s.seed);
    outln!(
        "Time       one tick = {} {}; deterministic: {}",
        descriptor.dt,
        descriptor.tick_unit,
        if descriptor.deterministic { "yes" } else { "no" }
    );
    outln!();
    outln!("{}", descriptor.summary);
    if !s.description.is_empty() {
        outln!();
        outln!("{}", s.description);
    }

    outln!();
    outln!("PARAMETERS ({})", descriptor.params.len());
    let mut params = Table::new(&[
        ("KEY", Align::Left),
        ("GROUP", Align::Left),
        ("TYPE", Align::Left),
        ("DEFAULT", Align::Left),
        ("VALUE", Align::Left),
        ("APPLIES", Align::Left),
    ]);
    for p in &descriptor.params {
        params.row(vec![
            p.key.clone(),
            p.group.clone(),
            kind_text(p),
            param_value(&p.default),
            values.get(&p.key).map_or_else(|| "-".to_string(), param_value),
            p.apply.label().to_string(),
        ]);
    }
    out!("{}", params.render());
    for p in &descriptor.params {
        if let ParamKind::Choice { options } = &p.kind
            && options.len() > CHOICES_INLINE
        {
            let values: Vec<&str> = options.iter().map(|o| o.value.as_str()).collect();
            outln!("  {}: {}", p.key, values.join(", "));
        }
    }

    outln!();
    outln!("METRICS ({})", descriptor.metrics.len());
    let mut metrics =
        Table::new(&[("KEY", Align::Left), ("KIND", Align::Left), ("UNIT", Align::Left), ("DESCRIPTION", Align::Left)]);
    for m in &descriptor.metrics {
        metrics.row(vec![m.key.clone(), metric_kind(m.kind).to_string(), m.unit.clone(), m.description.clone()]);
    }
    out!("{}", metrics.render());

    outln!();
    outln!("RULES");
    outln!("{}", descriptor.rules.trim_end());
    outln!();
    outln!("SCENARIOS OF {} ({}): {}", spec.id, scenarios.len(), scenarios.join(", "));
    Ok(())
}
