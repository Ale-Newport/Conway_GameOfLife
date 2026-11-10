//! Text helpers: exact (possibly huge) numbers, and explanations of
//! topologies and engines derived from the model's own selection rules.

use cellular_automata::engine::{
    AUTO_DENSE_MAX_CELLS, AUTO_HASHLIFE_MIN_POPULATION, AUTO_HASHLIFE_MIN_REGULARITY, AUTO_HASHLIFE_MIN_STEP,
    AUTO_REGULARITY_MAX_POPULATION, MAX_DENSE_CELLS, MAX_DENSE_WINDOW_CELLS, MAX_SWITCH_CELLS,
};
use cellular_automata::{EngineChoice, EngineKind, World};
use simulation_core::Topology;

/// Integer with thousands separators (`1,099,511,627,776`).
pub fn thousands(value: u128) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Power-of-two reading of a large count: `2^40` when exact, `≈ 2^40.3`
/// otherwise; `None` below 2^16, where the plain number is readable.
pub fn power_of_two_hint(value: u128) -> Option<String> {
    if value < 1 << 16 {
        return None;
    }
    if value.is_power_of_two() {
        return Some(format!("2^{}", value.trailing_zeros()));
    }
    Some(format!("≈ 2^{:.1}", (value as f64).log2()))
}

/// A fraction as a percentage with three significant digits (`4.65 %`,
/// `0.00253 %`); exactly `0 %` for zero.
pub fn percent(fraction: f64) -> String {
    let p = fraction * 100.0;
    if p == 0.0 || !p.is_finite() {
        return "0 %".into();
    }
    let digits = (2 - p.abs().log10().floor() as i32).clamp(0, 12) as usize;
    format!("{p:.digits$} %")
}

/// Short topology name.
pub fn topology_name(topology: Topology) -> &'static str {
    match topology {
        Topology::Bounded => "Bounded plane",
        Topology::Torus => "Torus",
        Topology::KleinBottle => "Klein bottle",
        Topology::CrossSurface => "Cross-surface",
        Topology::Infinite => "Infinite plane",
    }
}

/// One-sentence description of how a topology joins the edges (matches
/// `LifeSim`'s rules text and `Topology::wrap`).
pub fn topology_blurb(topology: Topology) -> &'static str {
    match topology {
        Topology::Bounded => "Cells outside the W × H rectangle are permanently dead; gliders die at the walls.",
        Topology::Torus => "Opposite edges are joined: what leaves on one side comes back on the other.",
        Topology::KleinBottle => {
            "Left and right edges are joined; crossing the top or bottom edge also mirrors x (x becomes W - 1 - x)."
        }
        Topology::CrossSurface => {
            "Crossing the top or bottom edge mirrors x, crossing the left or right edge mirrors y (projective plane)."
        }
        Topology::Infinite => "Unbounded in every direction with 64-bit coordinates; W × H is only the home region.",
    }
}

/// World summary: `512 × 512 torus` or `infinite plane (home 256 × 192)`.
pub fn world_summary(world: World) -> String {
    match world.topology {
        Topology::Infinite => format!("Infinite plane · home {} × {}", world.width, world.height),
        t => format!("{} × {} {}", world.width, world.height, topology_name(t).to_lowercase()),
    }
}

/// Short engine name.
pub fn engine_name(kind: EngineKind) -> &'static str {
    match kind {
        EngineKind::Dense => "Dense",
        EngineKind::Bit => "Bit-packed",
        EngineKind::Sparse => "Sparse tiles",
        EngineKind::Hashlife => "Hashlife",
    }
}

/// What an engine does (from `docs/models/cellular-automata.md` §2).
pub fn engine_blurb(kind: EngineKind) -> &'static str {
    match kind {
        EngineKind::Dense => {
            "One byte per cell with a ghost ring, stepped through a lookup table into a second buffer."
        }
        EngineKind::Bit => {
            "64 cells per machine word, neighbours counted with bitwise full adders on parallel row bands."
        }
        EngineKind::Sparse => {
            "64 × 64 bit tiles in a hash map; only tiles that changed in the last generation (and their neighbours) \
             are recomputed."
        }
        EngineKind::Hashlife => {
            "Gosper's quadtree with hash-consed nodes and memoised results: identical squares are stored and \
             computed once, and a tick of 2^k generations is one memoised jump."
        }
    }
}

/// Engine choice label.
pub fn choice_name(choice: EngineChoice) -> &'static str {
    match choice.fixed() {
        None => "Automatic",
        Some(kind) => engine_name(kind),
    }
}

/// Why the automatic choice runs `kind` on `world` with `generations` per
/// tick, following `cellular_automata::select_engine`.
pub fn auto_reason(world: World, kind: EngineKind, generations: u64) -> String {
    let cells = world.cell_count();
    let pow2 = generations.is_power_of_two();
    let note =
        "The choice is re-evaluated on reset, when a pattern is loaded and when the generations per tick change.";
    let reason = match (world.is_finite(), kind) {
        (true, EngineKind::Dense) => format!(
            "Finite world of {} cells, at most {} (128 × 128): the byte-per-cell engine has the least overhead.",
            thousands(u128::from(cells)),
            thousands(u128::from(AUTO_DENSE_MAX_CELLS))
        ),
        (true, EngineKind::Bit) => format!(
            "Finite world of {} cells, more than {} (128 × 128): packing 64 cells per word is fastest.",
            thousands(u128::from(cells)),
            thousands(u128::from(AUTO_DENSE_MAX_CELLS))
        ),
        (false, EngineKind::Hashlife) if pow2 && generations >= AUTO_HASHLIFE_MIN_STEP => format!(
            "Infinite plane with {generations} generations per tick, a power of two of at least \
             {AUTO_HASHLIFE_MIN_STEP}: Hashlife advances each tick in one memoised jump."
        ),
        (false, EngineKind::Hashlife) => format!(
            "Infinite plane with a large, regular pattern (at the last selection: at least {} live cells and a tile \
             regularity of at least {AUTO_HASHLIFE_MIN_REGULARITY}, or more than {} live cells, which Hashlife keeps \
             without measuring): Hashlife shares the repeated structure.",
            thousands(u128::from(AUTO_HASHLIFE_MIN_POPULATION)),
            thousands(u128::from(AUTO_REGULARITY_MAX_POPULATION))
        ),
        (false, EngineKind::Sparse) => format!(
            "Infinite plane with {generations} generation(s) per tick (below {AUTO_HASHLIFE_MIN_STEP} or not a \
             power of two) and no large regular pattern: sparse tiles cost only where cells change."
        ),
        _ => "The automatic choice has not been re-evaluated since the world changed.".to_string(),
    };
    format!("{reason} {note}")
}

/// Why `choice` cannot be selected now, or `None` when it can (the
/// automatic choice always can). Mirrors the model's own checks
/// (`EngineKind::check`, Hashlife's power-of-two ticks, and a switch to
/// another engine copying at most [`MAX_SWITCH_CELLS`] live cells).
pub fn choice_unavailable(
    choice: EngineChoice,
    world: World,
    generations: u64,
    running: EngineKind,
    population: u128,
    bbox_area: Option<u128>,
) -> Option<String> {
    let kind = choice.fixed()?;
    let finite = world.is_finite();
    match kind {
        EngineKind::Bit if !finite => return Some("The bit-packed engine runs finite worlds only.".into()),
        EngineKind::Sparse | EngineKind::Hashlife if finite => {
            return Some(format!("{} runs the infinite plane only.", engine_name(kind)));
        }
        EngineKind::Dense if finite && world.cell_count() > MAX_DENSE_CELLS => {
            return Some(format!(
                "The dense engine accepts at most {} cells; this world has {}.",
                thousands(u128::from(MAX_DENSE_CELLS)),
                thousands(u128::from(world.cell_count()))
            ));
        }
        EngineKind::Dense if !finite && bbox_area.is_some_and(|a| a > u128::from(MAX_DENSE_WINDOW_CELLS)) => {
            return Some(format!(
                "The live cells span more than the dense window's {} cells.",
                thousands(u128::from(MAX_DENSE_WINDOW_CELLS))
            ));
        }
        EngineKind::Hashlife if !generations.is_power_of_two() => {
            return Some(format!("Hashlife advances in powers of two; {generations} generations per tick is not one."));
        }
        _ => {}
    }
    if kind != running && population > u128::from(MAX_SWITCH_CELLS) {
        return Some(format!(
            "Switching engines copies every live cell one by one; {} live cells are more than the {} a switch \
             copies.",
            thousands(population),
            thousands(u128::from(MAX_SWITCH_CELLS))
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world(topology: Topology, w: u32, h: u32) -> World {
        World::new(topology, w, h).expect("valid world")
    }

    #[test]
    fn huge_numbers_are_exact() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_000), "1,000");
        assert_eq!(thousands(1 << 40), "1,099,511,627,776");
        assert_eq!(thousands(u128::from(u64::MAX)), "18,446,744,073,709,551,615");
        assert_eq!(power_of_two_hint(1000), None);
        assert_eq!(power_of_two_hint(1 << 40).as_deref(), Some("2^40"));
        assert_eq!(power_of_two_hint(3 << 20).as_deref(), Some("≈ 2^21.6"));
        assert_eq!(percent(0.04651), "4.65 %");
        assert_eq!(percent(0.5), "50.0 %");
        assert_eq!(percent(1.0), "100 %");
        assert_eq!(percent(0.0000253), "0.00253 %");
        assert_eq!(percent(0.0), "0 %");
    }

    #[test]
    fn auto_reasons_follow_the_selection_rules() {
        let small = world(Topology::Torus, 128, 128);
        assert!(auto_reason(small, EngineKind::Dense, 1).contains("16,384 cells, at most 16,384"));
        let large = world(Topology::Torus, 512, 512);
        assert!(auto_reason(large, EngineKind::Bit, 1).contains("262,144 cells, more than"));
        let inf = world(Topology::Infinite, 256, 256);
        assert!(auto_reason(inf, EngineKind::Hashlife, 64).contains("64 generations per tick"));
        assert!(auto_reason(inf, EngineKind::Hashlife, 1).contains("regular pattern"));
        assert!(auto_reason(inf, EngineKind::Sparse, 1).contains("sparse tiles"));
        assert!(auto_reason(inf, EngineKind::Bit, 1).contains("not been re-evaluated"));
    }

    #[test]
    fn unavailable_engines_are_explained() {
        let torus = world(Topology::Torus, 64, 64);
        let inf = world(Topology::Infinite, 64, 64);
        let none = |c, w, g| choice_unavailable(c, w, g, EngineKind::Dense, 10, Some(100));
        assert!(none(EngineChoice::Auto, torus, 3).is_none());
        assert!(none(EngineChoice::Bit, inf, 1).is_some());
        assert!(none(EngineChoice::Sparse, torus, 1).is_some());
        assert!(none(EngineChoice::Hashlife, inf, 3).unwrap().contains("powers of two"));
        assert!(none(EngineChoice::Hashlife, inf, 4).is_none());
        assert!(none(EngineChoice::Dense, inf, 1).is_none());
        let huge = choice_unavailable(EngineChoice::Dense, inf, 1, EngineKind::Sparse, 10, Some(1 << 30));
        assert!(huge.unwrap().contains("dense window"));
        // The model copies at most MAX_SWITCH_CELLS live cells into another engine, in either direction.
        let limit = u128::from(MAX_SWITCH_CELLS);
        assert!(choice_unavailable(EngineChoice::Sparse, inf, 1, EngineKind::Hashlife, limit, None).is_none());
        let leave = choice_unavailable(EngineChoice::Sparse, inf, 1, EngineKind::Hashlife, limit + 1, None);
        assert!(leave.unwrap().contains("67,108,864 a switch copies"));
        let enter = choice_unavailable(EngineChoice::Hashlife, inf, 1, EngineKind::Sparse, limit + 1, None);
        assert!(enter.unwrap().contains("copies every live cell"));
        let stay = choice_unavailable(EngineChoice::Hashlife, inf, 1, EngineKind::Hashlife, 1 << 30, None);
        assert!(stay.is_none());
        let auto = choice_unavailable(EngineChoice::Auto, inf, 1, EngineKind::Hashlife, 1 << 30, None);
        assert!(auto.is_none(), "the model keeps Hashlife for patterns too large to measure");
    }

    #[test]
    fn summaries_and_names() {
        assert_eq!(world_summary(world(Topology::Torus, 512, 256)), "512 × 256 torus");
        assert_eq!(world_summary(world(Topology::Infinite, 256, 192)), "Infinite plane · home 256 × 192");
        for kind in EngineKind::ALL {
            assert!(!engine_blurb(kind).is_empty() && !engine_name(kind).is_empty());
        }
        assert_eq!(choice_name(EngineChoice::Auto), "Automatic");
        assert_eq!(choice_name(EngineChoice::Hashlife), "Hashlife");
    }
}
