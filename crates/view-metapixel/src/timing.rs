//! Time arithmetic of the construction: step sizes (Hashlife jumps per
//! tick), where the run is inside the 35,328-generation cycle, the next
//! readout and the labels shown for them. Everything derives from
//! `metapixel::spec` and `metapixel::PhaseInfo`.

use interface::format::format_count;
use metapixel::spec::{PERIOD, TRANSITION_END, TRANSITION_START};
use metapixel::{CycleStage, MAX_STEP_EXPONENT, StepSize};

/// Every step size the model accepts, smallest first.
pub fn step_options() -> Vec<StepSize> {
    (0..=MAX_STEP_EXPONENT).map(StepSize::Pow2).chain(std::iter::once(StepSize::Metagen)).collect()
}

/// The next larger (`up`) or smaller step size, staying at the ends.
pub fn neighbour_step(step: StepSize, up: bool) -> StepSize {
    let options = step_options();
    let i = options.iter().position(|s| *s == step).unwrap_or(options.len() - 1);
    let j = if up { (i + 1).min(options.len() - 1) } else { i.saturating_sub(1) };
    options[j]
}

/// Short label of a step size: `"2^10 = 1,024"` or `"1 metagen = 35,328"`.
pub fn step_label(step: StepSize) -> String {
    match step {
        StepSize::Pow2(k) => format!("2^{k} = {}", format_count(1 << k)),
        StepSize::Metagen => format!("1 metagen = {}", format_count(PERIOD)),
    }
}

/// How a step compares with one metageneration: `"1/34.5 metagen"`.
pub fn metagen_fraction_text(generations: u64) -> String {
    if generations == 0 {
        return "0 metagen".to_string();
    }
    if generations.is_multiple_of(PERIOD) {
        let n = generations / PERIOD;
        return format!("{n} metagen{}", if n == 1 { "" } else { "s" });
    }
    let ratio = PERIOD as f64 / generations as f64;
    if ratio >= 1.0 {
        let text = if ratio >= 100.0 { format!("{ratio:.0}") } else { format!("{ratio:.1}") };
        format!("1/{} metagen", text.trim_end_matches(".0"))
    } else {
        format!("{:.2} metagens", generations as f64 / PERIOD as f64)
    }
}

/// Exponents of the power-of-two Hashlife steps that advance exactly
/// `generations` (one per set bit, smallest first — how
/// `hashlife::Universe::advance` decomposes a jump).
pub fn hashlife_jumps(generations: u64) -> Vec<u32> {
    (0..64).filter(|bit| generations >> bit & 1 == 1).collect()
}

/// `"2^9 + 2^11 + 2^15"`.
pub fn jumps_text(generations: u64) -> String {
    let jumps = hashlife_jumps(generations);
    if jumps.is_empty() {
        return "no step".to_string();
    }
    jumps.iter().map(|k| format!("2^{k}")).collect::<Vec<_>>().join(" + ")
}

/// Generations from `t` to the next exact readout (the next multiple of P
/// strictly after `t`).
pub fn generations_to_next_readout(t: u64) -> u64 {
    (t / PERIOD + 1).saturating_mul(PERIOD) - t
}

/// Ticks until the next readout has been taken with `step` generations per
/// tick. A tick that crosses the readout time pauses there to read the
/// tiles and then completes its full step, so the last tick may end after
/// the readout.
pub fn ticks_to_next_readout(t: u64, step: StepSize) -> u64 {
    generations_to_next_readout(t).div_ceil(step.generations())
}

/// Generations before the switching window at which "watch the next
/// transition" starts (`t mod P = 4,096`).
pub const TRANSITION_LEAD_IN: u64 = 224;

/// The next generation at or after `t` that lies [`TRANSITION_LEAD_IN`]
/// generations before a switching window (`t mod P = 4,096` with
/// `t ≥ P`: the first cycle has no transition).
pub fn transition_lead_in(t: u64) -> u64 {
    let offset = TRANSITION_START - TRANSITION_LEAD_IN;
    let cycle = (t / PERIOD).max(1);
    let candidate = cycle.saturating_mul(PERIOD).saturating_add(offset);
    if candidate >= t { candidate } else { (cycle + 1).saturating_mul(PERIOD).saturating_add(offset) }
}

/// The next exact readout time strictly after `t`.
pub fn next_readout(t: u64) -> u64 {
    (t / PERIOD + 1).saturating_mul(PERIOD)
}

/// Fraction of the cycle completed, `(t mod P) / P`.
pub fn cycle_fraction(phase: u64) -> f32 {
    (phase % PERIOD) as f32 / PERIOD as f32
}

/// Name of a cycle stage.
pub fn stage_label(stage: CycleStage) -> &'static str {
    match stage {
        CycleStage::FirstCycle => "first cycle",
        CycleStage::Transition => "displays switching",
        CycleStage::Stable => "steady",
    }
}

/// One-sentence explanation of a cycle stage.
pub fn stage_help(stage: CycleStage) -> String {
    match stage {
        CycleStage::FirstCycle => format!(
            "t < {}: the proximity fuses burn and the first signals are exchanged; the displays keep showing \
             metagen 0.",
            format_count(PERIOD)
        ),
        CycleStage::Transition => format!(
            "t mod P is in [{}, {}]: cores may be half switched, so only the live threshold reading describes them.",
            format_count(TRANSITION_START),
            format_count(TRANSITION_END)
        ),
        CycleStage::Stable => "Every display shows the current metagen; the cores are steady.".to_string(),
    }
}

/// Fractions of the cycle where the transition window starts and ends
/// (for drawing it on a progress bar).
pub fn transition_window_fractions() -> (f32, f32) {
    (TRANSITION_START as f32 / PERIOD as f32, (TRANSITION_END + 1) as f32 / PERIOD as f32)
}

/// Generations per second of a tick that advanced `generations` in `ms`.
pub fn generations_per_second(generations: u64, ms: f64) -> Option<f64> {
    (ms.is_finite() && ms > 0.0).then(|| generations as f64 * 1000.0 / ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use metapixel::PhaseInfo;

    #[test]
    fn step_options_are_the_model_options() {
        let options = step_options();
        assert_eq!(options.len(), 17);
        assert_eq!(options[0].generations(), 1);
        assert_eq!(options[15].generations(), 32_768);
        assert_eq!(options[16], StepSize::Metagen);
        for step in &options {
            assert_eq!(StepSize::parse(&step.value()), Some(*step), "every option is accepted by the model");
        }
        assert_eq!(neighbour_step(StepSize::Metagen, true), StepSize::Metagen);
        assert_eq!(neighbour_step(StepSize::Metagen, false), StepSize::Pow2(15));
        assert_eq!(neighbour_step(StepSize::Pow2(0), false), StepSize::Pow2(0));
        assert_eq!(neighbour_step(StepSize::Pow2(3), true), StepSize::Pow2(4));
        assert_eq!(step_label(StepSize::Pow2(10)), "2^10 = 1,024");
        assert_eq!(step_label(StepSize::Metagen), "1 metagen = 35,328");
    }

    #[test]
    fn fractions_and_jumps() {
        assert_eq!(metagen_fraction_text(1024), "1/34.5 metagen");
        assert_eq!(metagen_fraction_text(512), "1/69 metagen");
        assert_eq!(metagen_fraction_text(1), "1/35328 metagen");
        assert_eq!(metagen_fraction_text(PERIOD), "1 metagen");
        assert_eq!(metagen_fraction_text(3 * PERIOD), "3 metagens");
        assert_eq!(metagen_fraction_text(2 * 32_768), "1.86 metagens");
        assert_eq!(hashlife_jumps(PERIOD), vec![9, 11, 15], "35,328 = 2^9 + 2^11 + 2^15");
        assert_eq!(jumps_text(PERIOD), "2^9 + 2^11 + 2^15");
        assert_eq!(jumps_text(1024), "2^10");
        assert_eq!(jumps_text(0), "no step");
        let total: u64 = hashlife_jumps(123_456_789).iter().map(|k| 1u64 << k).sum();
        assert_eq!(total, 123_456_789);
    }

    #[test]
    fn readout_arithmetic() {
        assert_eq!(generations_to_next_readout(0), PERIOD);
        assert_eq!(generations_to_next_readout(PERIOD), PERIOD);
        assert_eq!(generations_to_next_readout(PERIOD + 1000), PERIOD - 1000);
        assert_eq!(ticks_to_next_readout(0, StepSize::Pow2(10)), 35, "the 35th tick reads at P and ends at 35,840");
        assert_eq!(ticks_to_next_readout(PERIOD, StepSize::Metagen), 1);
        assert_eq!(cycle_fraction(PERIOD / 2), 0.5);
        let (a, b) = transition_window_fractions();
        assert!(a < b && b < 0.25);
        assert_eq!(generations_per_second(PERIOD, 1000.0), Some(PERIOD as f64));
        assert_eq!(generations_per_second(5, 0.0), None);
    }

    #[test]
    fn transition_lead_ins() {
        assert_eq!(TRANSITION_START - TRANSITION_LEAD_IN, 4_096);
        assert_eq!(transition_lead_in(0), PERIOD + 4_096, "the first cycle has no transition");
        assert_eq!(transition_lead_in(PERIOD), PERIOD + 4_096);
        assert_eq!(transition_lead_in(PERIOD + 4_096), PERIOD + 4_096);
        assert_eq!(transition_lead_in(PERIOD + 5_000), 2 * PERIOD + 4_096, "already switching: the next one");
        for t in [0, 17, PERIOD - 1, 3 * PERIOD + 9_999] {
            let g = transition_lead_in(t);
            assert!(g >= t && g >= PERIOD && g % PERIOD == 4_096);
            assert_eq!(PhaseInfo::at(g + TRANSITION_LEAD_IN, None).stage, CycleStage::Transition);
            assert_eq!(PhaseInfo::at(g, None).stage, CycleStage::Stable);
        }
        assert_eq!(next_readout(0), PERIOD);
        assert_eq!(next_readout(PERIOD), 2 * PERIOD);
    }

    #[test]
    fn stages_match_the_model() {
        assert_eq!(stage_label(PhaseInfo::at(10, None).stage), "first cycle");
        assert_eq!(stage_label(PhaseInfo::at(PERIOD + 5000, Some(0)).stage), "displays switching");
        assert_eq!(stage_label(PhaseInfo::at(PERIOD + 9000, Some(0)).stage), "steady");
        assert!(stage_help(CycleStage::Transition).contains("4,320"));
    }
}
