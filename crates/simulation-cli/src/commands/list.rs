//! `list models` and `list scenarios`.

use model_registry::{Registry, ScenarioSource};
use serde::Serialize;

use crate::output::{out, outln};
use crate::text::{Align, Table, short_path};

/// One line of `list models`.
#[derive(Debug, Serialize)]
struct ModelRow {
    id: String,
    name: Option<String>,
    family: Option<String>,
    summary: Option<String>,
    crate_name: String,
    docs: String,
    scenario_folder: String,
    default_scenario: String,
    deterministic: Option<bool>,
    tick_unit: Option<String>,
    scenarios: usize,
    error: Option<String>,
}

fn model_rows(registry: &Registry) -> Vec<ModelRow> {
    registry
        .models()
        .iter()
        .map(|spec| {
            let info = registry.model_info(spec.id);
            let (ok, error) = match info {
                Ok(info) => (Some(info), None),
                Err(e) => (None, Some(e.to_string())),
            };
            ModelRow {
                id: spec.id.to_string(),
                name: ok.as_ref().map(|i| i.name.clone()),
                family: ok.as_ref().map(|i| i.family.label().to_string()),
                summary: ok.as_ref().map(|i| i.summary.clone()),
                crate_name: spec.crate_name.to_string(),
                docs: spec.docs.to_string(),
                scenario_folder: spec.folder.to_string(),
                default_scenario: format!("{}/{}", spec.folder, spec.default_scenario),
                deterministic: ok.as_ref().map(|i| i.deterministic),
                tick_unit: ok.as_ref().map(|i| i.tick_unit.clone()),
                scenarios: registry.scenarios_of(spec.id).count(),
                error,
            }
        })
        .collect()
}

/// `list models`.
///
/// # Errors
/// JSON serialisation failures.
pub fn models(registry: &Registry, json: bool) -> anyhow::Result<()> {
    let rows = model_rows(registry);
    if json {
        outln!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }
    let mut table = Table::new(&[
        ("ID", Align::Left),
        ("NAME", Align::Left),
        ("FAMILY", Align::Left),
        ("CRATE", Align::Left),
        ("SCENARIOS", Align::Right),
        ("DEFAULT SCENARIO", Align::Left),
    ]);
    for row in &rows {
        table.row(vec![
            row.id.clone(),
            row.name.clone().unwrap_or_else(|| "-".into()),
            row.family.clone().unwrap_or_else(|| "-".into()),
            row.crate_name.clone(),
            row.scenarios.to_string(),
            row.default_scenario.clone(),
        ]);
    }
    out!("{}", table.render());
    outln!("{} models", rows.len());
    for row in rows.iter().filter(|r| r.error.is_some()) {
        eprintln!("warning: {}: {}", row.id, row.error.as_deref().unwrap_or_default());
    }
    Ok(())
}

#[derive(Serialize)]
struct ScenarioList<'a> {
    scenarios: Vec<&'a model_registry::ScenarioInfo>,
    problems: &'a [model_registry::ScenarioProblem],
}

/// `list scenarios [--model ID]`.
///
/// # Errors
/// Unknown model ids.
pub fn scenarios(registry: &Registry, model: Option<&str>, json: bool) -> anyhow::Result<()> {
    if let Some(id) = model {
        registry.model(id)?;
    }
    let selected: Vec<_> =
        registry.scenarios().iter().filter(|s| model.is_none_or(|m| s.info.model_id == m)).map(|s| &s.info).collect();
    if json {
        let list = ScenarioList { scenarios: selected, problems: registry.problems() };
        outln!("{}", serde_json::to_string_pretty(&list)?);
        return Ok(());
    }
    let mut table = Table::new(&[
        ("ID", Align::Left),
        ("MODEL", Align::Left),
        ("SEED", Align::Right),
        ("SOURCE", Align::Left),
        ("TITLE", Align::Left),
    ]);
    for info in &selected {
        let source = match &info.source {
            ScenarioSource::Builtin(_) => "builtin".to_string(),
            ScenarioSource::File(path) => short_path(path),
        };
        table.row(vec![info.id.clone(), info.model_id.clone(), info.seed.to_string(), source, info.title.clone()]);
    }
    out!("{}", table.render());
    let dirs: Vec<String> = registry.scenario_dirs().iter().map(|d| short_path(d)).collect();
    outln!(
        "{} scenarios; scenario directories: {}",
        selected.len(),
        if dirs.is_empty() { "none".to_string() } else { dirs.join(", ") }
    );
    for problem in registry.problems() {
        eprintln!("warning: skipped {problem}");
    }
    Ok(())
}
