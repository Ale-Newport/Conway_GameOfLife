//! The session shared with the shell: a `Runner<VoxelLife>` whose
//! descriptor follows the running model.
//!
//! The voxel-life descriptor depends on the running rule and world (the
//! rules text, the per-state metrics `state_1 … state_{C−1}`, and the
//! parameter defaults of the loaded scenario); the runner publishes it
//! again after scenario loads (`Control::Replace`) and rule changes, with
//! the glyphs egui's fonts cannot draw replaced ([`displayable`]).

use std::sync::Arc;

use bevy::log::warn;
use bevy::prelude::Resource;
use simulation_core::{ModelDescriptor, Runner, RunnerConfig, Simulation};
use voxel_life::{VoxelCommand, VoxelLife};

/// Characters of the model's texts that egui's built-in fonts cannot draw,
/// with readable replacements (the shell renders the descriptor verbatim).
const REPLACEMENTS: [(char, &str); 3] = [('∈', "in"), ('∉', "not in"), ('∅', "{ }")];

fn replace_glyphs(text: &mut String) {
    if text.chars().any(|c| REPLACEMENTS.iter().any(|(r, _)| *r == c)) {
        let mut out = String::with_capacity(text.len());
        for c in text.chars() {
            match REPLACEMENTS.iter().find(|(r, _)| *r == c) {
                Some((_, with)) => out.push_str(with),
                None => out.push(c),
            }
        }
        *text = out;
    }
}

/// The descriptor with every text drawable by egui's built-in fonts.
pub(crate) fn displayable(mut descriptor: ModelDescriptor) -> ModelDescriptor {
    replace_glyphs(&mut descriptor.summary);
    replace_glyphs(&mut descriptor.rules);
    for spec in &mut descriptor.params {
        replace_glyphs(&mut spec.label);
        replace_glyphs(&mut spec.description);
    }
    for spec in &mut descriptor.metrics {
        replace_glyphs(&mut spec.label);
        replace_glyphs(&mut spec.description);
    }
    descriptor
}

/// Typed handle of the running session (also inserted as the shell's
/// `ActiveSession`).
#[derive(Resource, Clone)]
pub(crate) struct VoxelRunner(pub Arc<Runner<VoxelLife>>);

/// Moves `life` to a simulation thread serving the [`displayable`]
/// descriptor. Clear and Randomize are enabled (Randomize draws the pending
/// random cube with the given seed).
pub(crate) fn spawn_runner(life: VoxelLife) -> Runner<VoxelLife> {
    let config = RunnerConfig { publish_hz: 30.0, ..RunnerConfig::default() };
    Runner::spawn_with_descriptor(life, config, |sim| displayable(sim.descriptor()))
        .with_clear(|sim: &mut VoxelLife| {
            // Clearing cannot fail.
            let _ = sim.apply(VoxelCommand::Clear);
        })
        .with_randomize(|sim: &mut VoxelLife, seed| {
            if let Err(e) = sim.apply(VoxelCommand::Randomize { seed: Some(seed) }) {
                warn!("3D Life: randomize refused: {e}");
            }
        })
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;
    use simulation_core::{MetricSpec, ModelFamily, ParamValue, SessionControl};

    #[test]
    fn displayable_replaces_missing_glyphs_everywhere() {
        let descriptor = ModelDescriptor::new("m", "M", ModelFamily::Voxel)
            .summary("x ∈ A")
            .rules("N ∈ ∅, 2 … 9")
            .metrics(vec![MetricSpec::gauge("k", "∅", "u", "n ∉ S")]);
        let shown = displayable(descriptor);
        assert_eq!(shown.summary, "x in A");
        assert_eq!(shown.rules, "N in { }, 2 … 9", "drawable glyphs are kept");
        assert_eq!(shown.metrics[0].label, "{ }");
        assert_eq!(shown.metrics[0].description, "n not in S");
    }

    #[test]
    fn the_runner_serves_the_displayable_descriptor_of_the_running_rule() {
        let scenario = voxel_life::scenarios::builtin_by_id("life-5766-glider").expect("builtin");
        let runner = spawn_runner(VoxelLife::new(scenario).expect("valid"));
        assert!(runner.descriptor().rules.contains("5-7/6/2/M"), "{}", runner.descriptor().rules);
        assert!(runner.can_clear() && runner.can_randomize());
        // A rule change on the simulation thread publishes a new descriptor.
        runner.set_param("rule", ParamValue::Text("4555".into()));
        let deadline = Instant::now() + Duration::from_secs(5);
        while runner.status().descriptor_seq == 0 {
            assert!(Instant::now() < deadline, "the descriptor was not published again");
            std::thread::sleep(Duration::from_millis(2));
        }
        let rules = &runner.descriptor().rules;
        assert!(rules.contains("4-5/5/2/M") && rules.contains("N in {5}"), "{rules}");
        assert!(!rules.contains('∈'), "replaced for egui's fonts");
        SessionControl::clear(&runner);
        loop {
            let population = runner.request(|sim| sim.stats().population());
            if population.recv_timeout(Duration::from_secs(5)).expect("population") == 0 {
                break;
            }
            assert!(Instant::now() < deadline, "clear was not applied");
        }
    }
}
