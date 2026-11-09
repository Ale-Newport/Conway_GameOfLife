//! Drainage analysis used at generation time: ocean connectivity,
//! Priority-Flood depression filling (Barnes, Lehman & Mulla 2014, with an
//! ε-gradient so flats drain), D8 flow directions, flow accumulation, lake
//! selection and river tracing/carving.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, VecDeque};

use rayon::prelude::*;
use simulation_core::SimRng;

use crate::cells::{STEP_LEN, is_border, neighbor};

/// Receiver code of cells without a downstream neighbour (ocean cells).
pub const NO_RECEIVER: u8 = 8;
/// Receiver code of border cells that drain off the map.
pub const OFF_MAP: u8 = 9;
/// Minimum rise (m) imposed per step when filling flats and depressions.
pub(crate) const FILL_EPSILON: f64 = 0.005;
/// Fill depth (m) above which a depression cell counts as part of a basin.
const BASIN_MIN_FILL: f64 = 0.05;
/// Minimum drop (m) between consecutive river cells after carving.
const RIVER_DROP_M: f32 = 0.01;

/// Ocean mask: cells below sea level (elevation < 0) that are 8-connected,
/// through other below-sea cells, to a below-sea border cell.
pub(crate) fn ocean_mask(elevation: &[f32], width: usize, height: usize) -> Vec<bool> {
    let n = width * height;
    let mut ocean = vec![false; n];
    let mut queue = VecDeque::new();
    for i in 0..n {
        if is_border(i, width, height) && elevation[i] < 0.0 {
            ocean[i] = true;
            queue.push_back(i);
        }
    }
    while let Some(i) = queue.pop_front() {
        for k in 0..8 {
            if let Some(j) = neighbor(i, k, width, height)
                && !ocean[j]
                && elevation[j] < 0.0
            {
                ocean[j] = true;
                queue.push_back(j);
            }
        }
    }
    ocean
}

/// Min-heap entry ordered by level, then by index (deterministic ties).
#[derive(PartialEq)]
struct Entry {
    level: f64,
    idx: usize,
}

impl Eq for Entry {}

impl Ord for Entry {
    fn cmp(&self, other: &Self) -> Ordering {
        other.level.total_cmp(&self.level).then_with(|| other.idx.cmp(&self.idx))
    }
}

impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Priority-Flood + ε: returns the filled surface. Ocean cells are seeded at
/// the sea surface (0 m) and land border cells at their own elevation; every
/// other cell is raised to at least `ε` above the cell it was reached from,
/// so the filled surface has no depressions and no perfectly flat areas.
pub(crate) fn priority_flood(elevation: &[f32], ocean: &[bool], width: usize, height: usize) -> Vec<f64> {
    let n = width * height;
    let mut filled: Vec<f64> = elevation.iter().map(|&z| f64::from(z)).collect();
    let mut closed = vec![false; n];
    let mut heap = BinaryHeap::new();
    for i in 0..n {
        if ocean[i] {
            filled[i] = 0.0;
        } else if !is_border(i, width, height) {
            continue;
        }
        closed[i] = true;
        heap.push(Entry { level: filled[i], idx: i });
    }
    while let Some(Entry { level, idx }) = heap.pop() {
        for k in 0..8 {
            if let Some(j) = neighbor(idx, k, width, height)
                && !closed[j]
            {
                closed[j] = true;
                filled[j] = filled[j].max(level + FILL_EPSILON);
                heap.push(Entry { level: filled[j], idx: j });
            }
        }
    }
    filled
}

/// D8 steepest-descent receivers on the filled surface. Ocean cells get
/// [`NO_RECEIVER`]; border cells without a lower neighbour get [`OFF_MAP`].
pub(crate) fn d8_receivers(filled: &[f64], ocean: &[bool], width: usize, height: usize) -> Vec<u8> {
    let mut receivers = vec![NO_RECEIVER; width * height];
    receivers.par_chunks_mut(width).enumerate().for_each(|(y, row)| {
        for (x, out) in row.iter_mut().enumerate() {
            let i = y * width + x;
            if ocean[i] {
                continue;
            }
            let mut best = None;
            let mut best_slope = 0.0f64;
            for (k, len) in STEP_LEN.iter().enumerate() {
                if let Some(j) = neighbor(i, k, width, height) {
                    let slope = (filled[i] - filled[j]) / f64::from(*len);
                    if slope > best_slope {
                        best_slope = slope;
                        best = Some(k as u8);
                    }
                }
            }
            *out = match best {
                Some(k) => k,
                None if is_border(i, width, height) => OFF_MAP,
                None => NO_RECEIVER,
            };
        }
    });
    receivers
}

/// Downstream cell of `i`, if it has one inside the map.
#[inline]
pub(crate) fn downstream(i: usize, receivers: &[u8], width: usize, height: usize) -> Option<usize> {
    let r = receivers[i];
    if r < 8 { neighbor(i, usize::from(r), width, height) } else { None }
}

/// Flow accumulation: every land cell contributes `weights[i]` (e.g. its
/// relative annual precipitation) to itself and all cells downstream.
/// Cells are processed from the highest to the lowest filled level (ties by
/// index), which is a topological order because receivers are strictly lower.
pub(crate) fn flow_accumulation(
    filled: &[f64],
    receivers: &[u8],
    ocean: &[bool],
    weights: &[f32],
    width: usize,
    height: usize,
) -> Vec<f32> {
    let n = width * height;
    let mut order: Vec<usize> = (0..n).filter(|&i| !ocean[i]).collect();
    order.sort_by(|&a, &b| filled[b].total_cmp(&filled[a]).then(a.cmp(&b)));
    let mut acc: Vec<f64> = (0..n).map(|i| if ocean[i] { 0.0 } else { f64::from(weights[i]) }).collect();
    for &i in &order {
        if let Some(j) = downstream(i, receivers, width, height)
            && !ocean[j]
        {
            acc[j] += acc[i];
        }
    }
    acc.into_iter().map(|a| a as f32).collect()
}

/// Selects which closed depressions become lakes and fills the others.
///
/// Depression cells (fill depth > 5 cm) are grouped into 8-connected
/// basins. With `t = lake_tendency > 0`, basins of at least two cells whose
/// maximum depth reaches `0.5 m + 10 m × (1 − t)²` are kept as lakes, deepest
/// first (ties by first cell index), as long as the total lake area stays
/// within `0.25 t²` of the non-ocean cells. Every other land cell is raised
/// to the filled surface (sediment fill), so the terrain drains everywhere
/// except into lakes. Returns the lake mask; lake depth is
/// `filled − elevation`.
pub(crate) fn select_lakes(
    elevation: &mut [f32],
    filled: &[f64],
    ocean: &[bool],
    width: usize,
    height: usize,
    lake_tendency: f32,
) -> Vec<bool> {
    let n = width * height;
    let in_basin: Vec<bool> =
        (0..n).map(|i| !ocean[i] && filled[i] - f64::from(elevation[i]) > BASIN_MIN_FILL).collect();
    let mut basins: Vec<(f64, Vec<usize>)> = Vec::new();
    let mut seen = vec![false; n];
    let mut queue = VecDeque::new();
    for start in 0..n {
        if !in_basin[start] || seen[start] {
            continue;
        }
        let mut component = Vec::new();
        seen[start] = true;
        queue.push_back(start);
        let mut max_depth = 0.0f64;
        while let Some(i) = queue.pop_front() {
            component.push(i);
            max_depth = max_depth.max(filled[i] - f64::from(elevation[i]));
            for k in 0..8 {
                if let Some(j) = neighbor(i, k, width, height)
                    && in_basin[j]
                    && !seen[j]
                {
                    seen[j] = true;
                    queue.push_back(j);
                }
            }
        }
        basins.push((max_depth, component));
    }
    basins.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1[0].cmp(&b.1[0])));
    let t = f64::from(lake_tendency);
    let min_depth = 0.5 + 10.0 * (1.0 - t).powi(2);
    let land = ocean.iter().filter(|o| !**o).count() as f64;
    let budget = (0.25 * t * t * land) as usize;
    let mut used = 0;
    let mut lake = vec![false; n];
    for (depth, cells) in &basins {
        if t > 0.0 && cells.len() >= 2 && *depth >= min_depth && used + cells.len() <= budget {
            used += cells.len();
            for &i in cells {
                lake[i] = true;
            }
        }
    }
    for i in 0..n {
        if !ocean[i] && !lake[i] {
            elevation[i] = filled[i] as f32;
        }
    }
    lake
}

/// Channel depth (m) carved for a river cell with accumulation `acc`.
fn channel_depth(acc: f32) -> f32 {
    (0.6 + 0.35 * (1.0 + acc / 50.0).ln()).min(4.0)
}

/// Inputs to [`place_rivers`].
pub(crate) struct RiverInputs<'a> {
    pub width: usize,
    pub height: usize,
    pub count: u32,
    pub receivers: &'a [u8],
    pub accumulation: &'a [f32],
    pub ocean: &'a [bool],
    pub lake: &'a [bool],
    pub precipitation: &'a [f32],
}

/// Places `count` river sources and carves their channels.
///
/// Sources are drawn (seeded) with weight `(z / z_max)² × precipitation ×
/// √accumulation` among land cells above 25 % of the highest land, at least
/// `max(3, max(w, h) / (1.5 √(count + 1)))` cells apart (Chebyshev). Each
/// river follows the D8 receivers to the ocean, the map edge or a lake (and
/// on through the lake's outlet). Channels are carved `0.6–4 m` deep
/// (growing with accumulation), the first cell after a lake is not carved
/// (it is the lake's spill rim), and the bed is forced to drop at least 1 cm
/// per step, so every river cell is strictly higher than the next one.
pub(crate) fn place_rivers(inputs: &RiverInputs<'_>, elevation: &mut [f32], rng: &mut SimRng) -> Vec<bool> {
    let RiverInputs { width, height, count, receivers, accumulation, ocean, lake, precipitation } = *inputs;
    let n = width * height;
    let mut river = vec![false; n];
    if count == 0 {
        return river;
    }
    let is_land = |i: usize| !ocean[i] && !lake[i];
    let z_max = (0..n).filter(|&i| is_land(i)).map(|i| elevation[i]).fold(0.0f32, f32::max);
    if z_max <= 0.0 {
        return river;
    }
    let weights: Vec<f64> = (0..n)
        .map(|i| {
            if !is_land(i) || receivers[i] == NO_RECEIVER || elevation[i] < 0.25 * z_max {
                return 0.0;
            }
            let rel = f64::from(elevation[i] / z_max);
            rel * rel * f64::from(precipitation[i].max(0.0)) * f64::from(accumulation[i].max(0.0)).sqrt()
        })
        .collect();
    let mut cumulative = Vec::with_capacity(n);
    let mut total = 0.0;
    for &weight in &weights {
        total += weight;
        cumulative.push(total);
    }
    if total <= 0.0 {
        return river;
    }
    let spacing = (width.max(height) as f64 / (1.5 * f64::from(count + 1).sqrt())).max(3.0) as i64;
    let mut sources: Vec<usize> = Vec::new();
    let mut attempts = 0;
    while sources.len() < count as usize && attempts < count * 30 {
        attempts += 1;
        let target = rng.f64() * total;
        let i = cumulative.partition_point(|&c| c <= target).min(n - 1);
        if weights[i] <= 0.0 || river[i] {
            continue;
        }
        let (x, y) = ((i % width) as i64, (i / width) as i64);
        let too_close = sources.iter().any(|&s| {
            let (sx, sy) = ((s % width) as i64, (s / width) as i64);
            (sx - x).abs().max((sy - y).abs()) < spacing
        });
        if too_close {
            continue;
        }
        sources.push(i);
        carve_river(i, inputs, elevation, &mut river);
    }
    river
}

/// Traces one river downstream from `source` and carves its bed.
fn carve_river(source: usize, inputs: &RiverInputs<'_>, elevation: &mut [f32], river: &mut [bool]) {
    let RiverInputs { width, height, receivers, accumulation, ocean, lake, .. } = *inputs;
    let mut prev: Option<f32> = None;
    let mut after_lake = false;
    let mut cur = source;
    for _ in 0..width * height {
        if lake[cur] {
            prev = None;
            after_lake = true;
        } else {
            let mut z = elevation[cur];
            if !river[cur] && !after_lake {
                z -= channel_depth(accumulation[cur]);
            }
            if let Some(p) = prev {
                z = z.min(p - RIVER_DROP_M);
            }
            elevation[cur] = z.min(elevation[cur]);
            river[cur] = true;
            prev = Some(elevation[cur]);
            after_lake = false;
        }
        match downstream(cur, receivers, width, height) {
            Some(next) if !ocean[next] => cur = next,
            _ => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 5x5 bowl: border at 10 m, interior 5 m with a 1 m pit in the centre,
    /// and a notch at (0, 2) at 3 m through which everything drains.
    fn bowl() -> Vec<f32> {
        let mut z = vec![10.0f32; 25];
        for y in 1..4 {
            for x in 1..4 {
                z[y * 5 + x] = 5.0;
            }
        }
        z[12] = 1.0;
        z[10] = 3.0;
        z
    }

    #[test]
    fn priority_flood_removes_depressions() {
        let z = bowl();
        let ocean = vec![false; 25];
        let filled = priority_flood(&z, &ocean, 5, 5);
        for i in 0..25 {
            assert!(filled[i] >= f64::from(z[i]));
        }
        // The pit is filled to just above its spill path (≥ 5 m).
        assert!(filled[12] > 5.0 && filled[12] < 5.1);
        let receivers = d8_receivers(&filled, &ocean, 5, 5);
        // Every interior cell has a strictly lower receiver.
        for y in 1..4 {
            for x in 1..4 {
                let i = y * 5 + x;
                let j = downstream(i, &receivers, 5, 5).unwrap();
                assert!(filled[j] < filled[i]);
            }
        }
        // All interior flow leaves through the notch, which drains off-map.
        let weights = vec![1.0f32; 25];
        let acc = flow_accumulation(&filled, &receivers, &ocean, &weights, 5, 5);
        assert_eq!(receivers[10], OFF_MAP);
        assert!(acc[10] >= 10.0, "notch should collect the 9 interior cells and itself, got {}", acc[10]);
    }

    #[test]
    fn lakes_kept_or_filled_by_tendency() {
        let ocean = vec![false; 25];
        let mut z = bowl();
        let filled = priority_flood(&z, &ocean, 5, 5);
        // Pit depth ≈ 4 m: kept with tendency 1 (min depth 0.5 m) only if ≥ 2 cells.
        let lake = select_lakes(&mut z, &filled, &ocean, 5, 5, 1.0);
        assert!(!lake[12], "single-cell pits are filled");
        assert!((f64::from(z[12]) - filled[12]).abs() < 1e-3);
        // A two-cell basin is kept.
        let mut z = bowl();
        z[13] = 1.5;
        let filled = priority_flood(&z, &ocean, 5, 5);
        let lake = select_lakes(&mut z, &filled, &ocean, 5, 5, 1.0);
        assert!(lake[12] && lake[13]);
        assert_eq!(z[12], 1.0, "lake bottoms keep their depth");
        let mut z0 = bowl();
        z0[13] = 1.5;
        let none = select_lakes(&mut z0, &filled, &ocean, 5, 5, 0.0);
        assert!(none.iter().all(|&l| !l));
        // Tendency 0.5 allows 0.25 × 0.25 × 25 = 1 lake cell: the basin does not fit.
        let mut z1 = bowl();
        z1[13] = 1.5;
        let capped = select_lakes(&mut z1, &filled, &ocean, 5, 5, 0.5);
        assert!(capped.iter().all(|&l| !l));
    }

    /// Random rough terrain (some cells below sea level) and its ocean mask.
    fn random_terrain(rng: &mut SimRng, width: usize, height: usize) -> (Vec<f32>, Vec<bool>) {
        let z: Vec<f32> = (0..width * height).map(|_| rng.range_f32(-4.0, 20.0)).collect();
        let ocean = ocean_mask(&z, width, height);
        (z, ocean)
    }

    #[test]
    fn priority_flood_matches_a_brute_force_fixpoint() {
        // The ε-filled surface is the unique W with W = 0 on the ocean,
        // W = z on land border cells and W_i = max(z_i, min_j W_j + ε)
        // elsewhere. Brute force: relax from +∞ until nothing changes.
        let mut rng = SimRng::seed_from_u64(77);
        for _ in 0..25 {
            let (w, h) = (3 + rng.index(12), 3 + rng.index(12));
            let n = w * h;
            let (z, ocean) = random_terrain(&mut rng, w, h);
            let filled = priority_flood(&z, &ocean, w, h);
            let seed = |i: usize| ocean[i] || is_border(i, w, h);
            let mut brute: Vec<f64> = (0..n)
                .map(|i| {
                    if ocean[i] {
                        0.0
                    } else if seed(i) {
                        f64::from(z[i])
                    } else {
                        f64::INFINITY
                    }
                })
                .collect();
            let mut changed = true;
            while changed {
                changed = false;
                for i in (0..n).filter(|&i| !seed(i)) {
                    let lowest =
                        (0..8).filter_map(|k| neighbor(i, k, w, h)).map(|j| brute[j]).fold(f64::INFINITY, f64::min);
                    let value = f64::from(z[i]).max(lowest + FILL_EPSILON);
                    if value < brute[i] {
                        brute[i] = value;
                        changed = true;
                    }
                }
            }
            for i in 0..n {
                assert!((filled[i] - brute[i]).abs() < 1e-9, "{w}x{h} cell {i}: {} vs {}", filled[i], brute[i]);
            }
        }
    }

    #[test]
    fn receivers_and_accumulation_match_brute_force() {
        let mut rng = SimRng::seed_from_u64(78);
        for _ in 0..25 {
            let (w, h) = (3 + rng.index(14), 3 + rng.index(14));
            let n = w * h;
            let (z, ocean) = random_terrain(&mut rng, w, h);
            let filled = priority_flood(&z, &ocean, w, h);
            let receivers = d8_receivers(&filled, &ocean, w, h);
            let weights: Vec<f32> = (0..n).map(|_| rng.range_f32(0.5, 2.0)).collect();
            let acc = flow_accumulation(&filled, &receivers, &ocean, &weights, w, h);
            let mut brute = vec![0.0f64; n];
            for i in (0..n).filter(|&i| !ocean[i]) {
                // D8: the receiver is the first neighbour of steepest positive descent.
                let mut best: Option<(usize, f64)> = None;
                for (k, len) in STEP_LEN.iter().enumerate() {
                    if let Some(j) = neighbor(i, k, w, h) {
                        let slope = (filled[i] - filled[j]) / f64::from(*len);
                        if slope > 0.0 && best.is_none_or(|(_, s)| slope > s) {
                            best = Some((k, slope));
                        }
                    }
                }
                let expected = best.map_or(if is_border(i, w, h) { OFF_MAP } else { NO_RECEIVER }, |(k, _)| k as u8);
                assert_eq!(receivers[i], expected, "receiver of cell {i}");
                assert_ne!(expected, NO_RECEIVER, "every land cell drains after filling");
                // Accumulation: add the cell's weight to every land cell on its
                // downstream path (which must end within n steps).
                let mut cur = i;
                for step in 0..=n {
                    assert!(step < n, "drainage cycle from cell {i}");
                    brute[cur] += f64::from(weights[i]);
                    match downstream(cur, &receivers, w, h) {
                        Some(next) if !ocean[next] => cur = next,
                        _ => break,
                    }
                }
            }
            for i in 0..n {
                let expected = brute[i] as f32;
                assert!((acc[i] - expected).abs() <= 1e-4 * expected.max(1.0), "cell {i}: {} vs {expected}", acc[i]);
            }
        }
    }

    #[test]
    fn ocean_requires_border_connection() {
        // Left column below sea level, an isolated below-sea pit at (3, 2).
        let mut z = vec![5.0f32; 25];
        for y in 0..5 {
            z[y * 5] = -10.0;
        }
        z[13] = -3.0;
        let ocean = ocean_mask(&z, 5, 5);
        assert!(ocean[0] && ocean[20]);
        assert!(!ocean[13]);
    }
}
