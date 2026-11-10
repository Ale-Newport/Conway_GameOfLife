//! `validate`: determinism, snapshot round trips and model-specific checks.

use model_registry::{CheckStatus, Registry, ValidationReport};
use serde::Serialize;

use crate::cli::ValidateArgs;
use crate::output::outln;
use crate::text::{Align, Table};

/// Outcome for one target: a report, or the error that prevented building it.
#[derive(Serialize)]
struct Outcome {
    target: String,
    report: Option<ValidationReport>,
    error: Option<String>,
}

impl Outcome {
    fn passed(&self) -> bool {
        self.report.as_ref().is_some_and(ValidationReport::passed)
    }
}

fn status(s: CheckStatus) -> &'static str {
    match s {
        CheckStatus::Passed => "PASS",
        CheckStatus::Failed => "FAIL",
        CheckStatus::Skipped => "SKIP",
    }
}

/// `validate [TARGETS]`. Returns whether every check passed.
///
/// # Errors
/// JSON serialisation failures (build and check failures are reported in
/// the output and the result).
pub fn validate(registry: &Registry, args: &ValidateArgs) -> anyhow::Result<bool> {
    let targets: Vec<String> = if args.targets.is_empty() {
        registry.models().iter().map(|m| m.id.to_string()).collect()
    } else {
        args.targets.clone()
    };
    let outcomes: Vec<Outcome> = targets
        .iter()
        .map(|target| match model_registry::validate(registry, target, args.seed, args.ticks) {
            Ok(report) => Outcome { target: target.clone(), report: Some(report), error: None },
            Err(e) => Outcome { target: target.clone(), report: None, error: Some(e.to_string()) },
        })
        .collect();
    let passed = outcomes.iter().filter(|o| o.passed()).count();
    if args.json {
        outln!("{}", serde_json::to_string_pretty(&outcomes)?);
        return Ok(passed == outcomes.len());
    }
    for outcome in &outcomes {
        match (&outcome.report, &outcome.error) {
            (Some(r), _) => {
                outln!("{}  ({}, seed {}, {} ticks per phase)", r.model_id, r.scenario_id, r.seed, r.ticks);
                let mut table = Table::new(&[("", Align::Left), ("CHECK", Align::Left), ("RESULT", Align::Left)]);
                for c in &r.checks {
                    table.row(vec![status(c.status).to_string(), c.name.clone(), c.detail.clone()]);
                }
                for line in table.render().lines().skip(1) {
                    outln!("  {line}");
                }
            }
            (None, error) => {
                outln!("{}", outcome.target);
                outln!("  FAIL  build  {}", error.as_deref().unwrap_or_default());
            }
        }
    }
    outln!();
    outln!("{} target(s) validated: {passed} passed, {} failed", outcomes.len(), outcomes.len() - passed);
    Ok(passed == outcomes.len())
}
