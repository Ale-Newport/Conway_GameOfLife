//! Every pattern of the asset catalog (`assets/patterns/catalog.toml`) is
//! checked against its declared data with this crate's engines: initial
//! population and size, periods, displacements, gun output, puffer motion,
//! eater/reflector recovery, methuselah stabilisation and growth rates.
//! All 80 checks together take a few seconds (Hashlife jumps through the
//! long methuselahs and breeders). Two long-horizon checks — the breeder at
//! generation 10^12 and the sawtooth's second return to 177 cells — are
//! `#[ignore]`d; run them with `-- --ignored`.

mod common;

use cellular_automata::{EngineKind, LifeEngine};
use common::{
    Cells, Ship, catalog, catalog_cells, classify, components, engine, normalize, run, ship_library, ship_velocity,
    sorted, translate, without_ships,
};
use patterns::{CatalogEntry, LifeRule};

const LIFE: LifeRule = LifeRule::conway();

fn extra<'a>(entry: &'a CatalogEntry, key: &str) -> Option<&'a str> {
    entry.extra.get(key).map(String::as_str)
}

fn extra_u64(entry: &CatalogEntry, key: &str) -> Option<u64> {
    extra(entry, key).and_then(|v| v.trim().parse().ok())
}

/// Smallest `p` in `1..=limit` with `states[t] == states[t + p]` for every
/// `t` in `0..p_window`, comparing shapes up to `transform`.
fn smallest_period<F: Fn(&Cells, &Cells) -> bool>(
    states: &[Cells],
    window: usize,
    limit: usize,
    same: F,
) -> Option<usize> {
    (1..=limit).find(|&p| (0..window).all(|t| t + p < states.len() && same(&states[t], &states[t + p])))
}

fn states(engine: &mut Box<dyn LifeEngine>, count: usize) -> Vec<Cells> {
    let mut out = vec![engine.export_cells()];
    for _ in 1..count {
        engine.step().expect("in range");
        out.push(engine.export_cells());
    }
    out
}

fn check_basics(entry: &CatalogEntry, cells: &Cells) {
    if let Some(pop) = entry.population {
        assert_eq!(cells.len() as u64, pop, "{}: initial population", entry.id);
    }
    let (shape, _) = normalize(cells);
    let w = shape.iter().map(|c| c.0).max().map_or(0, |m| m + 1) as u64;
    let h = shape.iter().map(|c| c.1).max().map_or(0, |m| m + 1) as u64;
    if let (Some(ew), Some(eh)) = (entry.width, entry.height) {
        assert_eq!((w, h), (ew, eh), "{}: bounding box", entry.id);
    }
}

fn check_still_life(entry: &CatalogEntry, cells: &Cells) {
    assert_eq!(run(LIFE, cells, 1), *cells, "{}: still life", entry.id);
}

fn check_oscillator(entry: &CatalogEntry, cells: &Cells) {
    let period = entry.period.expect("period") as usize;
    let mut e = engine(EngineKind::Sparse, LIFE, cells);
    let s = states(&mut e, period + 1);
    let p = smallest_period(&s, 1, period, |a, b| a == b);
    assert_eq!(p, Some(period), "{}: oscillator period", entry.id);
    if let Some(range) = extra(entry, "population_range") {
        let pops: Vec<usize> = s[..period].iter().map(Vec::len).collect();
        let expected = format!("[{}, {}]", pops.iter().min().unwrap_or(&0), pops.iter().max().unwrap_or(&0));
        assert_eq!(range.replace(' ', ""), expected.replace(' ', ""), "{}: population range", entry.id);
    }
}

fn check_spaceship(entry: &CatalogEntry, cells: &Cells) {
    let period = entry.period.expect("period") as usize;
    let (dx, dy) = entry.displacement.expect("displacement");
    let mut e = engine(EngineKind::Sparse, LIFE, cells);
    let s = states(&mut e, period + 1);
    assert_eq!(s[period], translate(cells, dx, dy), "{}: displacement after one period", entry.id);
    let p = smallest_period(&s, 1, period, |a, b| normalize(a).0 == normalize(b).0);
    assert_eq!(p, Some(period), "{}: no shorter period", entry.id);
}

fn check_gun(entry: &CatalogEntry, cells: &Cells, ships: &[Ship]) {
    let period = entry.period.expect("period") as usize;
    let (shape, (ox, oy)) = normalize(cells);
    let (w, h) = (shape.iter().map(|c| c.0).max().unwrap_or(0) + 1, shape.iter().map(|c| c.1).max().unwrap_or(0) + 1);
    let inside = |c: &(i64, i64)| (ox - 6..ox + w + 6).contains(&c.0) && (oy - 6..oy + h + 6).contains(&c.1);
    let warmup = 4 * w.max(h) as u64 + 600;
    let mut e = engine(EngineKind::Sparse, LIFE, cells);
    e.advance(warmup).expect("in range");
    let s: Vec<Cells> =
        states(&mut e, 3 * period + 1).into_iter().map(|c| c.into_iter().filter(inside).collect()).collect();
    assert_eq!(smallest_period(&s, period, 2 * period, |a, b| a == b), Some(period), "{}: gun period", entry.id);
    let emits = extra(entry, "emits").expect("emits");
    if emits == "loafer" {
        // The Golly file absorbs its own loafer stream: only the period is observable.
        return;
    }
    let ship = ships.iter().find(|s| s.name.eq_ignore_ascii_case(emits)).expect("known ship");
    let per_period = extra_u64(entry, "emits_per_period").unwrap_or(1);
    let span = 2 * lcm(period as u64, ship.period);
    let count = |e: &dyn LifeEngine| {
        let outside: Cells = e.export_cells().into_iter().filter(|c| !inside(c)).collect();
        components(&outside).iter().filter(|c| classify(ships, c).is_some_and(|s| s.name == ship.name)).count() as u64
    };
    let before = count(e.as_ref());
    e.advance(span).expect("in range");
    assert_eq!(count(e.as_ref()) - before, span / period as u64 * per_period, "{}: emitted {}s", entry.id, ship.name);
}

fn lcm(a: u64, b: u64) -> u64 {
    let gcd = |mut a: u64, mut b: u64| {
        while b != 0 {
            (a, b) = (b, a % b);
        }
        a
    };
    a / gcd(a, b) * b
}

/// Cells of the leading part: the slab of depth `depth` behind the front
/// along the direction of motion, free spaceships removed.
fn head(ships: &[Ship], cells: &Cells, (ux, uy): (i64, i64), depth: i64) -> Cells {
    let body = without_ships(ships, cells);
    let front = body.iter().map(|&(x, y)| x * ux + y * uy).max().unwrap_or(0);
    body.into_iter().filter(|&(x, y)| x * ux + y * uy >= front - depth).collect()
}

fn check_puffer(entry: &CatalogEntry, cells: &Cells, ships: &[Ship]) {
    let period = entry.period.expect("period");
    let (dx, dy) = entry.displacement.expect("displacement");
    let (shape, _) = normalize(cells);
    let depth = shape.iter().map(|c| c.0.max(c.1)).max().unwrap_or(0) + 5;
    let direction = (dx.signum(), dy.signum());
    let mut e = engine(EngineKind::Hashlife, LIFE, cells);
    e.advance(3000.max(2 * period)).expect("in range");
    let mut heads = Vec::new();
    for _ in 0..3 {
        heads.push(head(ships, &e.export_cells(), direction, depth));
        e.advance(period).expect("in range");
    }
    for pair in heads.windows(2) {
        assert!(!pair[0].is_empty(), "{}: empty head", entry.id);
        assert_eq!(pair[1], translate(&pair[0], dx, dy), "{}: head moves by ({dx}, {dy}) every {period}", entry.id);
    }
}

fn check_eater(entry: &CatalogEntry, cells: &Cells, ships: &[Ship]) {
    let object: Cells = components(cells).into_iter().filter(|c| classify(ships, c).is_none()).flatten().collect();
    let object = sorted(object);
    assert_eq!(Some(object.len() as u64), extra_u64(entry, "object_population"), "{}: object size", entry.id);
    assert_eq!(run(LIFE, cells, 90), object, "{}: the glider is eaten and the eater restored", entry.id);
}

fn check_reflector(entry: &CatalogEntry, cells: &Cells, ships: &[Ship]) {
    let parts = components(cells);
    let incoming = parts.iter().find(|c| classify(ships, c).is_some()).expect("demonstration glider").clone();
    let object: Cells = sorted(parts.into_iter().filter(|c| classify(ships, c).is_none()).flatten().collect());
    let in_velocity = ship_velocity(&incoming, 4);
    let after = run(LIFE, &object.iter().chain(&incoming).copied().collect::<Vec<_>>(), 240);
    let object_after = run(LIFE, &object, 240);
    let free: Vec<Cells> = components(&after).into_iter().filter(|c| classify(ships, c).is_some()).collect();
    assert_eq!(free.len(), 1, "{}: exactly one glider leaves", entry.id);
    let rest: Cells =
        sorted(components(&after).into_iter().filter(|c| classify(ships, c).is_none()).flatten().collect());
    assert_eq!(rest, object_after, "{}: the reflector is restored", entry.id);
    let out_velocity = ship_velocity(&free[0], 4);
    assert_ne!(out_velocity, in_velocity, "{}: the glider changes direction", entry.id);
}

fn check_methuselah(entry: &CatalogEntry, cells: &Cells, ships: &[Ship]) {
    let mut e = engine(EngineKind::Hashlife, LIFE, cells);
    if let Some(dies) = extra_u64(entry, "dies_at") {
        e.advance(dies - 1).expect("in range");
        assert!(e.population() > 0, "{}: alive at generation {}", entry.id, dies - 1);
        e.step().expect("in range");
        assert_eq!(e.population(), 0, "{}: dies at generation {dies}", entry.id);
        return;
    }
    let stable = extra_u64(entry, "stabilizes_at").expect("stabilizes_at");
    let final_pop = extra_u64(entry, "final_population").expect("final_population");
    e.advance(stable - 1).expect("in range");
    let before = without_ships(ships, &e.export_cells());
    e.step().expect("in range");
    let at = e.export_cells();
    assert_eq!(at.len() as u64, final_pop, "{}: population at generation {stable}", entry.id);
    let at = without_ships(ships, &at);
    e.advance(59).expect("in range");
    let before_later = without_ships(ships, &e.export_cells());
    e.step().expect("in range");
    let later = without_ships(ships, &e.export_cells());
    assert_eq!(at, later, "{}: periodic (ignoring free spaceships) from generation {stable}", entry.id);
    assert_ne!(before, before_later, "{}: not yet periodic at generation {}", entry.id, stable - 1);
}

fn check_growth(entry: &CatalogEntry, cells: &Cells) {
    if extra(entry, "growth") == Some("linear") {
        let period = extra_u64(entry, "growth_period").expect("growth_period");
        let step = extra_u64(entry, "growth_per_period").expect("growth_per_period") as i64;
        let from = extra_u64(entry, "linear_from").expect("linear_from");
        let mut e = engine(EngineKind::Sparse, LIFE, cells);
        e.advance(from - 1).expect("in range");
        let mut pops = Vec::new();
        for _ in 0..=2 * period {
            pops.push(e.population() as i64);
            e.step().expect("in range");
        }
        // pops[i] is the population at generation from - 1 + i.
        let p = period as usize;
        assert_ne!(pops[p] - pops[0], step, "{}: not linear yet at generation {}", entry.id, from - 1);
        for i in 1..=p {
            assert_eq!(
                pops[i + p] - pops[i],
                step,
                "{}: +{step} per {period} at generation {}",
                entry.id,
                from - 1 + i as u64
            );
        }
        return;
    }
    // Quadratic growth: the measured populations are listed in `verification`.
    let text = extra(entry, "verification").expect("verification");
    let samples: Vec<(u64, u64)> = text
        .split("gen ")
        .skip(1)
        .filter_map(|part| {
            let (g, rest) = part.split_once(": ")?;
            let n: String = rest.chars().take_while(char::is_ascii_digit).collect();
            Some((g.parse().ok()?, n.parse().ok()?))
        })
        .collect();
    assert_eq!(samples.len(), 4, "{}: four population samples", entry.id);
    let mut e = engine(EngineKind::Hashlife, LIFE, cells);
    let mut generation = 0;
    let mut diffs = Vec::new();
    for &(g, expected) in &samples {
        e.advance(g - generation).expect("in range");
        generation = g;
        assert_eq!(e.population(), expected, "{}: population at generation {g}", entry.id);
        diffs.push(expected as i64);
    }
    let second = |i: usize| diffs[i + 2] - 2 * diffs[i + 1] + diffs[i];
    assert!(second(0) > 0 && second(0) == second(1), "{}: constant positive second difference", entry.id);
}

fn check_other(entry: &CatalogEntry, cells: &Cells, ships: &[Ship]) {
    match entry.id.as_str() {
        "switch-engine" => check_methuselah(entry, cells, ships),
        "sawtooth-177" => {
            let mut e = engine(EngineKind::Sparse, LIFE, cells);
            let (mut low, mut high) = (u64::MAX, 0);
            for _ in 1..5760 {
                e.step().expect("in range");
                low = low.min(e.population());
                high = high.max(e.population());
            }
            e.step().expect("in range");
            assert_eq!(e.population(), 177, "sawtooth population at generation 5760");
            assert!(low > 177, "sawtooth stays above 177 in between");
            assert_eq!(high, 297, "sawtooth peak");
        }
        "lwss-synthesis" => {
            let after = run(LIFE, cells, 120);
            let parts = components(&after);
            assert_eq!(parts.len(), 1, "exactly one object remains");
            assert_eq!(classify(ships, &parts[0]).map(|s| s.name), Some("lwss"));
        }
        other => panic!("no check for `other` pattern {other}"),
    }
}

fn verify(filter: impl Fn(&CatalogEntry) -> bool) -> usize {
    let catalog = catalog();
    let ships = ship_library(&catalog);
    let mut checked = 0;
    for entry in catalog.entries().iter().filter(|e| filter(e)) {
        let cells = catalog_cells(&catalog, &entry.id);
        check_basics(entry, &cells);
        match entry.kind.as_deref().unwrap_or_default() {
            "still_life" => check_still_life(entry, &cells),
            "oscillator" => check_oscillator(entry, &cells),
            "spaceship" => check_spaceship(entry, &cells),
            "gun" => check_gun(entry, &cells, &ships),
            "puffer" | "rake" => check_puffer(entry, &cells, &ships),
            "eater" => check_eater(entry, &cells, &ships),
            "reflector" => check_reflector(entry, &cells, &ships),
            "methuselah" => check_methuselah(entry, &cells, &ships),
            "growth" | "breeder" => check_growth(entry, &cells),
            "other" => check_other(entry, &cells, &ships),
            kind => panic!("{}: unknown kind {kind}", entry.id),
        }
        checked += 1;
    }
    checked
}

#[test]
fn every_catalog_pattern_matches_its_declared_behaviour() {
    assert_eq!(verify(|_| true), 80);
}

#[test]
#[ignore = "long horizon: generation 10^12 (run with --ignored)"]
fn breeder_population_at_generation_10_to_the_12() {
    // Rokicki (2006), reproduced by docs/research/life-engines-hashlife.md §3.12.
    let catalog = catalog();
    let mut e = cellular_automata::HashlifeEngine::new(LIFE).expect("B3/S23");
    e.set_cells(&catalog_cells(&catalog, "breeder-1"), true);
    e.advance(1_000_000_000_000).expect("in range");
    assert_eq!(e.universe().population_exact(), 1_302_083_334_180_208_337_404);
}

#[test]
#[ignore = "long horizon: 702,720 generations (run with --ignored)"]
fn sawtooth_returns_to_177_cells_at_its_second_low() {
    // Population 177 at generations 48·(121^n − 1); n = 2 gives 702,720.
    let catalog = catalog();
    let mut e = engine(EngineKind::Hashlife, LIFE, &catalog_cells(&catalog, "sawtooth-177"));
    e.advance(48 * (121 * 121 - 1)).expect("in range");
    assert_eq!(e.population(), 177);
    e.advance(1).expect("in range");
    assert!(e.population() > 177);
}

#[test]
fn gosper_gun_emits_one_glider_every_30_generations() {
    let catalog = catalog();
    let ships = ship_library(&catalog);
    let gun = catalog_cells(&catalog, "gosper-glider-gun");
    let mut e = engine(EngineKind::Sparse, LIFE, &gun);
    e.advance(120).expect("in range");
    let gliders = |e: &dyn LifeEngine| {
        components(&e.export_cells()).iter().filter(|c| classify(&ships, c).is_some_and(|s| s.name == "glider")).count()
    };
    let mut previous = (gliders(e.as_ref()), e.population());
    for _ in 0..10 {
        e.advance(30).expect("in range");
        let now = (gliders(e.as_ref()), e.population());
        assert_eq!(now.0, previous.0 + 1, "one more glider per 30 generations");
        assert_eq!(now.1, previous.1 + 5, "five more cells per 30 generations");
        previous = now;
    }
    // Every glider flies the same way (towards the bottom right).
    for g in components(&e.export_cells()).iter().filter(|c| classify(&ships, c).is_some()) {
        assert_eq!(ship_velocity(g, 4), (1, 1));
    }
}

#[test]
fn r_pentomino_stabilises_at_1103_with_116_cells_on_every_unbounded_engine() {
    let catalog = catalog();
    let ships = ship_library(&catalog);
    let r = catalog_cells(&catalog, "r-pentomino");
    for kind in [EngineKind::Sparse, EngineKind::Hashlife, EngineKind::Dense] {
        let mut e = engine(kind, LIFE, &r);
        e.advance(1103).expect("in range");
        assert_eq!(e.population(), 116, "{kind:?}");
        let settled = without_ships(&ships, &e.export_cells());
        e.advance(60).expect("in range");
        assert_eq!(without_ships(&ships, &e.export_cells()), settled, "{kind:?}: only gliders still move");
    }
}
