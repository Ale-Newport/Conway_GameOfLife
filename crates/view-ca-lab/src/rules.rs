//! Rule analysis shown by the laboratory: live validation of typed rules,
//! the comparison with Conway's B3/S23, Wolfram rule tables, equivalent
//! rules and a measured behaviour class, and turmite rule presets.
//!
//! Everything here is derived from the rule itself (and, for the measured
//! class, from runs of the model's own [`cellular_automata::wolfram::eca_step`]).

use cellular_automata::wolfram::eca_step;
use cellular_automata::{Boundary, TurmiteRule};
use patterns::{GenerationsRule, LifeRule};
use simulation_core::SimRng;

/// Conway's rule.
pub const CONWAY: LifeRule = LifeRule::conway();

/// Neighbour counts of a mask as text: `"2, 3"`, or `"none"`.
pub fn counts(mask: u16) -> String {
    let list: Vec<String> = (0..=8u16).filter(|n| mask >> n & 1 == 1).map(|n| n.to_string()).collect();
    if list.is_empty() { "none".to_string() } else { list.join(", ") }
}

/// Number list `"6"`, `"1, 4 and 5"`.
fn and_list(mask: u16) -> String {
    let list: Vec<String> = (0..=8u16).filter(|n| mask >> n & 1 == 1).map(|n| n.to_string()).collect();
    match list.len() {
        0 => String::new(),
        1 => list[0].clone(),
        n => format!("{} and {}", list[..n - 1].join(", "), list[n - 1]),
    }
}

fn neighbours(mask: u16) -> &'static str {
    if mask.count_ones() == 1 && mask & 0b10 != 0 { "neighbour" } else { "neighbours" }
}

/// How the birth and survival sets differ from Conway's B3/S23, one
/// sentence per difference (an empty list for Conway's rule itself).
pub fn conway_differences(birth: u16, survive: u16) -> Vec<String> {
    let mut out = Vec::new();
    let (b, s) = (CONWAY.birth, CONWAY.survive);
    let extra_b = birth & !b;
    let missing_b = b & !birth;
    let extra_s = survive & !s;
    let missing_s = s & !survive;
    if extra_b != 0 {
        out.push(format!("Dead cells are also born with {} live {}.", and_list(extra_b), neighbours(extra_b)));
    }
    if missing_b != 0 {
        out.push(format!("No birth with {} live {} (Conway: birth on 3).", and_list(missing_b), neighbours(missing_b)));
    }
    if extra_s != 0 {
        out.push(format!("Live cells also survive with {} live {}.", and_list(extra_s), neighbours(extra_s)));
    }
    if missing_s != 0 {
        out.push(format!(
            "Live cells no longer survive with {} live {} (Conway: survival on 2 or 3).",
            and_list(missing_s),
            neighbours(missing_s)
        ));
    }
    out
}

/// Whether a two-state Moore rule is unchanged by swapping live and dead
/// cells: `B = { 8 − k : k ∉ S }` (automata catalogue §1.2).
pub fn self_complementary(rule: LifeRule) -> bool {
    let mirrored: u16 = (0..=8u16).filter(|k| rule.survive >> k & 1 == 0).map(|k| 1 << (8 - k)).sum();
    mirrored == rule.birth
}

/// Result of checking a typed Life-like rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuleCheck {
    /// Nothing typed.
    Empty,
    /// A valid rule for the current world.
    Valid(LifeRule),
    /// The text does not parse, or the rule cannot run on this world.
    Invalid(String),
}

/// Checks a typed Life-like rule as the model will (any notation
/// `LifeRule::parse` accepts; B0 only on finite worlds).
pub fn check_life_rule(text: &str, infinite_world: bool) -> RuleCheck {
    if text.trim().is_empty() {
        return RuleCheck::Empty;
    }
    match LifeRule::parse(text) {
        Ok(rule) if rule.has_b0() && infinite_world => RuleCheck::Invalid(
            "B0 (birth with no live neighbour) would fill the infinite plane; it is only allowed on finite worlds"
                .to_string(),
        ),
        Ok(rule) => RuleCheck::Valid(rule),
        Err(e) => RuleCheck::Invalid(e.to_string()),
    }
}

/// Checks a typed Generations rule (name or notation, as the model parses it).
pub fn check_generations_rule(text: &str) -> Result<GenerationsRule, String> {
    if text.trim().is_empty() {
        return Err("type a rule such as /2/3, B2/S/C3 or 345/2/4".to_string());
    }
    cellular_automata::generations::parse_generations_rule(text).map_err(|e| e.to_string())
}

/// Checks a typed turmite rule.
pub fn check_turmite_rule(text: &str) -> Result<TurmiteRule, String> {
    TurmiteRule::parse(text).map_err(|e| e.to_string())
}

/// Turmite rules offered as presets (Golly's generator examples and
/// Wikipedia's, automata catalogue §5.2). Only the behaviour of RL and
/// LLRR is described: it is asserted by the model's tests.
pub const TURMITE_PRESETS: [(&str, &str); 7] = [
    ("RL", "Langton's ant: about 10,000 chaotic steps, then a highway"),
    ("LLRR", "Grows a figure that is mirror-symmetric again and again"),
    ("RLR", "3 colours"),
    ("LRRL", "4 colours"),
    ("LRRRRRLLR", "9 colours"),
    ("LLRRRLRLRLLR", "12 colours"),
    ("RRLLLRLLLRRR", "12 colours"),
];

// ---------------------------------------------------------------- Wolfram

/// Neighbourhood `(l, c, r)` of a table entry `i = 4l + 2c + r`.
pub fn neighbourhood(i: u8) -> [bool; 3] {
    [i & 4 != 0, i & 2 != 0, i & 1 != 0]
}

/// New centre cell for neighbourhood `i` (bit `i` of the rule).
pub fn output(rule: u8, i: u8) -> bool {
    rule >> (i & 7) & 1 == 1
}

/// The rule with the output of neighbourhood `i` flipped.
pub fn toggle_output(rule: u8, i: u8) -> u8 {
    rule ^ (1 << (i & 7))
}

/// Left-right mirror image: `f'(l, c, r) = f(r, c, l)`.
pub fn mirror(rule: u8) -> u8 {
    (0..8u8).filter(|&i| output(rule, (i & 2) | (i >> 2 & 1) | (i << 2 & 4))).map(|i| 1u8 << i).sum()
}

/// 0/1 complement: `f'(l, c, r) = 1 − f(1 − l, 1 − c, 1 − r)`.
pub fn complement(rule: u8) -> u8 {
    (0..8u8).filter(|&i| !output(rule, 7 - i)).map(|i| 1u8 << i).sum()
}

/// The rule, its mirror image, its complement and its mirrored complement
/// (they behave identically up to reflection and/or swapping 0 and 1).
pub fn equivalents(rule: u8) -> [u8; 4] {
    [rule, mirror(rule), complement(rule), mirror(complement(rule))]
}

/// The equivalent rules other than `rule` itself, each once, with how they
/// relate to it (`"mirror"`, `"complement"`, `"mirrored complement"`).
pub fn distinct_equivalents(rule: u8) -> Vec<(u8, &'static str)> {
    let [_, m, c, mc] = equivalents(rule);
    let mut out: Vec<(u8, &'static str)> = Vec::new();
    for (r, label) in [(m, "mirror"), (c, "complement"), (mc, "mirrored complement")] {
        if r != rule && out.iter().all(|(x, _)| *x != r) {
            out.push((r, label));
        }
    }
    out
}

/// Behaviour class of an elementary rule measured from random rows, in
/// Wolfram's terms (Wolfram 1984): I evolves to a uniform row; II to
/// simple stable or periodic structures; III to aperiodic (chaotic)
/// patterns; IV to complex localised structures. The measurement cannot
/// tell III from IV.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeasuredClass {
    /// Every random row became uniform.
    Uniform,
    /// The rows became periodic (possibly shifting around the ring).
    Periodic {
        /// Longest period seen (in steps).
        period: u32,
        /// Longest transient before the cycle (in steps).
        transient: u32,
    },
    /// No row repeated (up to a rotation) within the measured steps.
    Aperiodic,
}

impl MeasuredClass {
    /// Short label (`"Class I"`, ...).
    pub fn label(self) -> &'static str {
        match self {
            MeasuredClass::Uniform => "Class I",
            MeasuredClass::Periodic { .. } => "Class II",
            MeasuredClass::Aperiodic => "Class III or IV",
        }
    }

    /// One-line description of what was measured.
    pub fn detail(self) -> String {
        match self {
            MeasuredClass::Uniform => "every random row became uniform".to_string(),
            MeasuredClass::Periodic { period, transient } => format!(
                "random rows settle into a cycle of period {period} (up to a shift) after at most {transient} steps"
            ),
            MeasuredClass::Aperiodic => format!("no row repeated (up to a shift) within {CLASS_STEPS} steps"),
        }
    }
}

/// Ring width of the class measurement (199 is prime, so neither rule 90
/// nor rule 150 collapses as on rings of 2^k cells).
pub const CLASS_WIDTH: usize = 199;
/// Steps of the class measurement.
pub const CLASS_STEPS: u32 = 1200;
/// Random rows (density 1/2) tried by the class measurement.
pub const CLASS_TRIALS: u64 = 3;

/// Lexicographically least rotation of a 0/1 row (Booth's algorithm), so
/// rows that differ only by a shift around the ring compare equal.
pub fn least_rotation(row: &[bool]) -> Vec<bool> {
    let n = row.len();
    if n == 0 {
        return Vec::new();
    }
    let s: Vec<bool> = row.iter().chain(row.iter()).copied().collect();
    let mut f: Vec<isize> = vec![-1; 2 * n];
    let mut k: usize = 0;
    for j in 1..2 * n {
        let sj = s[j];
        let mut i = f[j - k - 1];
        while i != -1 && sj != s[k + i as usize + 1] {
            if !sj && s[k + i as usize + 1] {
                k = j - i as usize - 1;
            }
            i = f[i as usize];
        }
        if sj != s[k + (i + 1) as usize] {
            // i == -1 here.
            if !sj && s[k] {
                k = j;
            }
            f[j - k] = -1;
        } else {
            f[j - k] = i + 1;
        }
    }
    s[k..k + n].to_vec()
}

fn bits(row: &[u64], width: usize) -> Vec<bool> {
    (0..width).map(|x| row[x / 64] >> (x % 64) & 1 == 1).collect()
}

/// Measures the behaviour class of an elementary rule: [`CLASS_TRIALS`]
/// random rows on a ring of [`CLASS_WIDTH`] cells, [`CLASS_STEPS`] steps
/// each, with the model's own step function.
pub fn measure_class(rule: u8) -> MeasuredClass {
    let width = CLASS_WIDTH;
    let words = width.div_ceil(64);
    let mut uniform = true;
    let mut period_max = 0u32;
    let mut transient_max = 0u32;
    for trial in 0..CLASS_TRIALS {
        let mut rng = SimRng::seed_from_u64(0x5EED_0000 + trial);
        let mut row = vec![0u64; words];
        for x in 0..width {
            if rng.f64() < 0.5 {
                row[x / 64] |= 1 << (x % 64);
            }
        }
        let mut seen: std::collections::HashMap<Vec<bool>, u32> = std::collections::HashMap::new();
        let mut cycle = None;
        for t in 0..=CLASS_STEPS {
            let key = least_rotation(&bits(&row, width));
            if let Some(&first) = seen.get(&key) {
                cycle = Some((t - first, first));
                break;
            }
            seen.insert(key, t);
            row = eca_step(rule, &row, width, Boundary::Periodic);
        }
        let Some((period, transient)) = cycle else { return MeasuredClass::Aperiodic };
        let ones: u32 = row.iter().map(|w| w.count_ones()).sum();
        let is_uniform = period == 1 && (ones == 0 || ones as usize == width);
        uniform &= is_uniform;
        period_max = period_max.max(period);
        transient_max = transient_max.max(transient);
    }
    if uniform {
        MeasuredClass::Uniform
    } else {
        MeasuredClass::Periodic { period: period_max, transient: transient_max }
    }
}

/// What the literature says about a rule's class, for the rules whose
/// classification is undisputed (by equivalence class).
pub fn literature_note(rule: u8) -> Option<&'static str> {
    let group = equivalents(rule);
    if group.contains(&110) {
        Some("Wolfram's standard example of class IV; Cook (2004) proved rule 110 Turing-complete.")
    } else if group.contains(&30) {
        Some("Wolfram's standard example of class III; its centre column served as a random number generator.")
    } else {
        None
    }
}

/// Closed forms of well-known rules (checked against the rule tables in tests).
pub fn closed_form(rule: u8) -> Option<&'static str> {
    match rule {
        30 => Some("new = l XOR (c OR r)"),
        90 => Some("new = l XOR r"),
        150 => Some("new = l XOR c XOR r"),
        184 => Some("a 1 (car) moves right when the cell ahead is 0"),
        204 => Some("new = c (identity)"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conway_comparison() {
        assert!(conway_differences(CONWAY.birth, CONWAY.survive).is_empty());
        let high = LifeRule::highlife();
        assert_eq!(
            conway_differences(high.birth, high.survive),
            vec!["Dead cells are also born with 6 live neighbours."]
        );
        let seeds = LifeRule::seeds();
        assert_eq!(
            conway_differences(seeds.birth, seeds.survive),
            vec![
                "Dead cells are also born with 2 live neighbours.",
                "No birth with 3 live neighbours (Conway: birth on 3).",
                "Live cells no longer survive with 2 and 3 live neighbours (Conway: survival on 2 or 3).",
            ]
        );
        let two = LifeRule::parse("B36/S125").unwrap();
        let diff = conway_differences(two.birth, two.survive);
        assert!(diff.contains(&"Live cells also survive with 1 and 5 live neighbours.".to_string()), "{diff:?}");
        let gnarl = LifeRule::parse("B1/S1").unwrap();
        assert!(conway_differences(gnarl.birth, gnarl.survive)[0].ends_with("with 1 live neighbour."));
        assert_eq!(counts(0), "none");
        assert_eq!(counts(LifeRule::day_and_night().birth), "3, 6, 7, 8");
    }

    #[test]
    fn inversion_symmetry_matches_the_catalogue() {
        assert!(self_complementary(LifeRule::day_and_night()));
        assert!(self_complementary(LifeRule::parse("B4678/S35678").unwrap()), "Anneal");
        assert!(self_complementary(LifeRule::parse("B1357/S02468").unwrap()), "Fredkin");
        assert!(!self_complementary(CONWAY));
        assert!(!self_complementary(LifeRule::highlife()));
    }

    #[test]
    fn typed_rules_are_checked_like_the_model() {
        assert_eq!(check_life_rule("  ", false), RuleCheck::Empty);
        assert_eq!(check_life_rule("b36s23", true), RuleCheck::Valid(LifeRule::highlife()));
        assert_eq!(check_life_rule("23/3", true), RuleCheck::Valid(CONWAY), "S/B order");
        assert!(matches!(check_life_rule("B0/S8", true), RuleCheck::Invalid(m) if m.contains("finite")));
        assert!(matches!(check_life_rule("B0/S8", false), RuleCheck::Valid(_)));
        assert!(matches!(check_life_rule("B3/S2x", false), RuleCheck::Invalid(_)));
        assert_eq!(check_generations_rule("Brian's Brain"), Ok(GenerationsRule::brians_brain()));
        assert_eq!(check_generations_rule("B2/S345/C4"), Ok(GenerationsRule::star_wars()));
        assert!(check_generations_rule("/2/1").is_err());
        assert!(check_turmite_rule("LLRR").is_ok());
        assert!(check_turmite_rule("LX").is_err());
        assert!(check_turmite_rule("L").is_err());
        for (rule, _) in TURMITE_PRESETS {
            assert!(check_turmite_rule(rule).is_ok(), "{rule}");
        }
    }

    #[test]
    fn wolfram_tables_and_equivalences() {
        // Rule 30 = 00011110: 100, 011, 010, 001 give 1 (catalogue §6).
        let ones: Vec<u8> = (0..8).filter(|&i| output(30, i)).collect();
        assert_eq!(ones, vec![1, 2, 3, 4]);
        assert_eq!(toggle_output(30, 7), 158);
        assert_eq!(toggle_output(toggle_output(110, 3), 3), 110);
        assert_eq!(neighbourhood(6), [true, true, false]);
        // Catalogue §6: 30 ~ 86, 135, 149; 110 ~ 124, 137, 193; 184 ~ 226 and its own mirror-complement.
        assert_eq!(equivalents(30), [30, 86, 135, 149]);
        assert_eq!(equivalents(110), [110, 124, 137, 193]);
        assert_eq!(mirror(184), 226);
        assert_eq!(mirror(complement(184)), 184);
        assert_eq!(mirror(90), 90);
        assert_eq!(distinct_equivalents(90), vec![(165, "complement")], "90 is its own mirror");
        assert_eq!(distinct_equivalents(30), vec![(86, "mirror"), (135, "complement"), (149, "mirrored complement")]);
        assert_eq!(distinct_equivalents(184), vec![(226, "mirror")], "184's mirrored complement is itself");
        assert!(distinct_equivalents(150).is_empty() && distinct_equivalents(204).is_empty());
        assert_eq!(mirror(150), 150);
        for rule in 0..=255u8 {
            assert_eq!(mirror(mirror(rule)), rule);
            assert_eq!(complement(complement(rule)), rule);
        }
    }

    #[test]
    fn closed_forms_match_the_tables() {
        for rule in [30u8, 90, 150, 204] {
            for i in 0..8u8 {
                let [l, c, r] = neighbourhood(i);
                let expected = match rule {
                    30 => l ^ (c | r),
                    90 => l ^ r,
                    150 => l ^ c ^ r,
                    _ => c,
                };
                assert_eq!(output(rule, i), expected, "rule {rule}, neighbourhood {i:03b}");
            }
        }
        // Traffic: a car (1) at c stays iff blocked (r = 1); one arrives iff l = 1 and c = 0.
        for i in 0..8u8 {
            let [l, c, r] = neighbourhood(i);
            assert_eq!(output(184, i), (c && r) || (l && !c));
        }
    }

    #[test]
    fn least_rotation_is_shift_invariant() {
        let row = [true, false, false, true, true, false, true];
        let base = least_rotation(&row);
        for k in 0..row.len() {
            let rotated: Vec<bool> = (0..row.len()).map(|i| row[(i + k) % row.len()]).collect();
            assert_eq!(least_rotation(&rotated), base);
        }
        assert_eq!(base, vec![false, false, true, true, false, true, true]);
        assert!(least_rotation(&[]).is_empty());
    }

    #[test]
    fn measured_classes() {
        assert_eq!(measure_class(0), MeasuredClass::Uniform);
        assert_eq!(measure_class(255), MeasuredClass::Uniform);
        // Wolfram's class 1 rules (equivalence classes of 0, 8, 32, 40, 128, 136, 160, 168).
        for rule in [8u8, 32, 128, 136, 160] {
            assert_eq!(measure_class(rule), MeasuredClass::Uniform, "rule {rule}");
        }
        assert!(matches!(measure_class(4), MeasuredClass::Periodic { period: 1, .. }), "rule 4 freezes");
        assert!(matches!(measure_class(184), MeasuredClass::Periodic { .. }), "traffic settles");
        assert!(matches!(measure_class(204), MeasuredClass::Periodic { period: 1, transient: 0 }));
        for rule in [30u8, 45, 90, 105, 110, 150] {
            assert_eq!(measure_class(rule), MeasuredClass::Aperiodic, "rule {rule}");
        }
        assert_eq!(MeasuredClass::Aperiodic.label(), "Class III or IV");
        assert!(literature_note(137).is_some_and(|n| n.contains("class IV")));
        assert!(literature_note(149).is_some_and(|n| n.contains("class III")));
        assert_eq!(literature_note(90), None);
    }
}
