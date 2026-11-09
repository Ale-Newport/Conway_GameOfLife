//! The public [`Universe`]: an unbounded two-state Life-like plane evolved by
//! Hashlife.

use std::collections::HashMap;
use std::fmt;

use crate::error::HashlifeError;
use crate::leaf::{Kernel, NE_QUADRANT, NW_QUADRANT, SE_QUADRANT, SW_QUADRANT};
use crate::query::Rect;
use crate::rule::Rule;
use crate::step::Stepper;
use crate::store::{LEAF_LEVEL, MAX_LEVEL, NodeStore};

/// Largest exponent accepted by [`Universe::step_pow2`] (one call advances at
/// most 2^61 generations; [`Universe::advance`] splits larger requests).
pub const MAX_STEP_EXPONENT: u32 = 61;
/// Default soft limit on live nodes before automatic garbage collection
/// (2^23 nodes ≈ 400 MB including the intern table).
pub const DEFAULT_NODE_LIMIT: usize = 1 << 23;
/// Smallest accepted node limit.
pub const MIN_NODE_LIMIT: usize = 1024;
/// Smallest root level (a 16×16 square).
const MIN_ROOT_LEVEL: u8 = LEAF_LEVEL + 1;

/// Half the side of a node of `level`.
#[inline]
pub(crate) fn half_side(level: u8) -> i128 {
    1i128 << (level - 1)
}

/// An unbounded plane of two-state cells evolved by Hashlife under an
/// outer-totalistic rule without `B0`.
///
/// Coordinates are `i64` with `x` growing to the right and `y` downward. The
/// quadtree root is centred on the origin: a root of level `L` covers
/// `[-2^(L-1), 2^(L-1))` on both axes, and every node of level `j` is aligned
/// to multiples of `2^j`. The root is kept as small as possible while all
/// live cells lie in its central half (unless they span more than `±2^62`).
///
/// # Example
/// ```
/// use hashlife::Universe;
///
/// let mut life = Universe::new(1 << 3, (1 << 2) | (1 << 3)).unwrap(); // B3/S23
/// life.insert_cells([(1, 0), (2, 1), (0, 2), (1, 2), (2, 2)]); // a glider
/// life.advance(4 * 1000).unwrap();
/// assert_eq!(life.generation(), 4000);
/// assert_eq!(life.population(), 5);
/// assert_eq!(life.bounding_box(), Some((1000, 1000, 1002, 1002)));
/// ```
///
/// # Panics
/// Like `Vec` on capacity overflow, the engine panics if more than
/// `2^32 - 2` nodes would be alive at once (well over 128 GiB of memory).
#[derive(Clone)]
pub struct Universe {
    store: NodeStore,
    rule: Rule,
    kernel: Kernel,
    root: u32,
    generation: u64,
}

impl Universe {
    /// Creates an empty universe for the rule with the given masks (bit `n`
    /// = neighbour count `n`, e.g. `1 << 3` and `(1 << 2) | (1 << 3)` for
    /// Conway's Life).
    ///
    /// # Errors
    /// [`HashlifeError::BirthOnZero`] for `B0` rules and
    /// [`HashlifeError::InvalidRule`] for bits above 8.
    pub fn new(birth_mask: u16, survive_mask: u16) -> Result<Self, HashlifeError> {
        Ok(Self::with_rule(Rule::new(birth_mask, survive_mask)?))
    }

    /// Creates an empty universe for `rule`.
    pub fn with_rule(rule: Rule) -> Self {
        let store = NodeStore::new(DEFAULT_NODE_LIMIT);
        let root = store.empty(MIN_ROOT_LEVEL);
        Self { store, rule, kernel: Kernel::new(rule), root, generation: 0 }
    }

    /// Creates an empty Conway's Life (`B3/S23`) universe.
    pub fn conway() -> Self {
        Self::with_rule(Rule::conway())
    }

    /// The rule being run.
    pub fn rule(&self) -> Rule {
        self.rule
    }

    /// Changes the rule. Every memoised result depends on the rule, so they
    /// are all discarded (the cells and the generation are unchanged).
    pub fn set_rule(&mut self, rule: Rule) {
        if rule != self.rule {
            self.rule = rule;
            self.kernel = Kernel::new(rule);
            self.store.clear_results();
        }
    }

    /// Number of generations run so far.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Overrides the generation counter (e.g. after loading a pattern).
    pub fn set_generation(&mut self, generation: u64) {
        self.generation = generation;
    }

    /// Number of live cells, saturating at `u64::MAX`.
    /// Constant time (populations are cached per node).
    pub fn population(&self) -> u64 {
        self.store.node(self.root).population
    }

    /// Whether no cell is alive.
    pub fn is_empty(&self) -> bool {
        self.population() == 0
    }

    /// Level of the quadtree root (its side is `2^level`).
    pub fn root_level(&self) -> u32 {
        u32::from(self.level())
    }

    /// Sets or clears the cell at `(x, y)`.
    pub fn set_cell(&mut self, x: i64, y: i64, alive: bool) {
        if !alive && !self.contains(x, y) {
            return;
        }
        self.collect_if_needed();
        if alive {
            self.reach(x, y, x, y);
        }
        let origin = -half_side(self.level());
        // In range: the root contains (x, y), so the offsets are in [0, 2^64).
        let (rx, ry) = ((i128::from(x) - origin) as u64, (i128::from(y) - origin) as u64);
        self.root = self.set_rec(self.root, rx, ry, alive);
        if !alive {
            self.normalize_root();
        }
    }

    /// Whether the cell at `(x, y)` is alive.
    pub fn get_cell(&self, x: i64, y: i64) -> bool {
        if !self.contains(x, y) {
            return false;
        }
        let origin = -half_side(self.level());
        let (mut rx, mut ry) = ((i128::from(x) - origin) as u64, (i128::from(y) - origin) as u64);
        let mut n = self.store.node(self.root);
        while n.level > LEAF_LEVEL {
            if n.population == 0 {
                return false;
            }
            let half = 1u64 << (n.level - 1);
            let q = usize::from(ry >= half) * 2 + usize::from(rx >= half);
            rx &= half - 1;
            ry &= half - 1;
            n = self.store.node(n.children[q]);
        }
        n.leaf_bits() >> (ry * 8 + rx) & 1 == 1
    }

    /// Sets every listed cell alive (duplicates are harmless). The cells are
    /// built into a quadtree in one pass and merged with the current content,
    /// which is much faster than repeated [`Universe::set_cell`] calls.
    pub fn insert_cells<I: IntoIterator<Item = (i64, i64)>>(&mut self, cells: I) {
        let points: Vec<(i64, i64)> = cells.into_iter().collect();
        let Some(&(first_x, first_y)) = points.first() else {
            return;
        };
        let (mut min_x, mut min_y, mut max_x, mut max_y) = (first_x, first_y, first_x, first_y);
        for &(x, y) in &points {
            min_x = min_x.min(x);
            max_x = max_x.max(x);
            min_y = min_y.min(y);
            max_y = max_y.max(y);
        }
        self.collect_if_needed();
        self.reach(min_x, min_y, max_x, max_y);
        let level = self.level();
        let origin = -half_side(level);
        // Converted in place (same layout), so bulk loads of millions of cells
        // do not hold two copies of the list.
        let mut relative: Vec<(u64, u64)> = points
            .into_iter()
            .map(|(x, y)| ((i128::from(x) - origin) as u64, (i128::from(y) - origin) as u64))
            .collect();
        let built = build(&mut self.store, level, &mut relative);
        let mut memo = HashMap::new();
        self.root = union(&mut self.store, self.root, built, &mut memo);
    }

    /// Kills every cell; the rule and the generation counter are kept.
    pub fn clear(&mut self) {
        self.root = self.store.empty(MIN_ROOT_LEVEL);
    }

    /// Kills every cell in the inclusive rectangle `[x0, x1] × [y0, y1]`
    /// (nothing happens if `x0 > x1` or `y0 > y1`).
    pub fn clear_rect(&mut self, x0: i64, y0: i64, x1: i64, y1: i64) {
        if x0 > x1 || y0 > y1 {
            return;
        }
        self.collect_if_needed();
        let rect = Rect::new(x0, y0, x1, y1);
        let origin = -half_side(self.level());
        self.root = self.clear_rec(self.root, origin, origin, &rect);
        self.normalize_root();
    }

    /// Advances `2^k` generations in one Hashlife step (Golly's "step size"
    /// `2^k`). Memoised results are reused across calls with the same `k`.
    ///
    /// # Errors
    /// * [`HashlifeError::StepTooLarge`] if `k > MAX_STEP_EXPONENT`;
    /// * [`HashlifeError::GenerationOverflow`] if the generation counter
    ///   would exceed `u64::MAX`;
    /// * [`HashlifeError::CoordinateOverflow`] if live cells lie more than
    ///   `2^61` cells from the origin, or `k` needs a root larger than the
    ///   `i64` plane. The universe is unchanged when an error is returned.
    pub fn step_pow2(&mut self, k: u32) -> Result<(), HashlifeError> {
        if k > MAX_STEP_EXPONENT {
            return Err(HashlifeError::StepTooLarge { exponent: k, max: MAX_STEP_EXPONENT });
        }
        let generations = 1u64 << k;
        let target = self
            .generation
            .checked_add(generations)
            .ok_or(HashlifeError::GenerationOverflow { generation: self.generation, requested: generations })?;
        if self.is_empty() {
            self.generation = target;
            return Ok(());
        }
        // The root keeps all live cells in its central half, so a root of
        // level L >= k + 2 can be padded once (to L + 1 <= 64) and its result
        // still contains everything reachable in 2^k <= 2^(L-2) generations.
        let needed = self.level().max(k as u8 + 2);
        if !self.is_centred(self.root) || needed >= MAX_LEVEL {
            return Err(HashlifeError::CoordinateOverflow(format!(
                "cannot advance 2^{k} generations: live cells must stay within ±2^61 of the origin \
                 (and 2^k must be at most 2^61) so the result fits in 64-bit coordinates"
            )));
        }
        while self.level() < needed {
            self.root = self.expand(self.root);
        }
        let top = self.expand(self.root);
        self.store.pins.clear();
        self.store.pins.extend([self.root, top]);
        if self.store.needs_gc() {
            self.store.collect(&[]);
        }
        let next = Stepper { store: &mut self.store, kernel: &self.kernel, k }.result(top);
        self.store.pins.clear();
        self.root = next;
        self.generation = target;
        self.normalize_root();
        Ok(())
    }

    /// Advances exactly `n` generations by decomposing `n` into powers of
    /// two (one [`Universe::step_pow2`] per set bit, smallest first; bits
    /// above [`MAX_STEP_EXPONENT`] become repeated `2^61` steps).
    ///
    /// # Errors
    /// [`HashlifeError::GenerationOverflow`] (checked before anything runs)
    /// and the errors of [`Universe::step_pow2`]. If a later power-of-two
    /// step fails, the steps already taken remain applied and
    /// [`Universe::generation`] reports how far the universe got.
    pub fn advance(&mut self, n: u64) -> Result<(), HashlifeError> {
        if self.generation.checked_add(n).is_none() {
            return Err(HashlifeError::GenerationOverflow { generation: self.generation, requested: n });
        }
        let mut remaining = n;
        while remaining != 0 {
            let bit = remaining.trailing_zeros();
            if bit <= MAX_STEP_EXPONENT {
                self.step_pow2(bit)?;
            } else {
                for _ in 0..1u64 << (bit - MAX_STEP_EXPONENT) {
                    self.step_pow2(MAX_STEP_EXPONENT)?;
                }
            }
            remaining &= remaining - 1;
        }
        Ok(())
    }

    /// Number of live nodes in the arena (including memoised results and
    /// garbage not yet collected).
    pub fn node_count(&self) -> usize {
        self.store.live()
    }

    /// Bytes allocated by the node arena and its intern table.
    pub fn memory_bytes(&self) -> usize {
        self.store.memory_bytes()
    }

    /// Frees every node that is neither part of the current pattern nor a
    /// memoised result of such a node (recursively), exactly like Golly's
    /// collector: the memo of everything still in use survives, so results
    /// never change and nothing has to be recomputed. Unlike the automatic
    /// collections (which may run inside a step and never move nodes), this
    /// one also compacts the arena and returns its unused capacity to the
    /// allocator, so [`Universe::memory_bytes`] shrinks with the live set.
    /// Returns the number of nodes freed.
    pub fn garbage_collect(&mut self) -> usize {
        let mut roots = [self.root];
        let freed = self.store.compact(&mut roots);
        self.root = roots[0];
        freed
    }

    /// Forgets every memoised result, then collects and compacts like
    /// [`Universe::garbage_collect`], keeping only the nodes of the current
    /// pattern. Frees the most memory, at the price of recomputing futures on
    /// the next steps (results stay identical). Returns the number of nodes
    /// freed.
    pub fn clear_memo(&mut self) -> usize {
        self.store.clear_results();
        self.garbage_collect()
    }

    /// Sets the soft limit on live nodes (at least [`MIN_NODE_LIMIT`]).
    /// When the arena reaches it — between or during steps — garbage is
    /// collected. The limit is soft: collections keep the pattern and the
    /// memo of its nodes, and are spaced so that the live set may double
    /// before the next one (no thrashing). Use [`Universe::clear_memo`] to
    /// trade speed for memory explicitly.
    pub fn set_node_limit(&mut self, limit: usize) {
        self.store.set_node_limit(limit.max(MIN_NODE_LIMIT));
    }

    /// The soft limit on live nodes.
    pub fn node_limit(&self) -> usize {
        self.store.node_limit()
    }

    /// Number of garbage collections run so far (automatic and explicit).
    pub fn gc_count(&self) -> u64 {
        self.store.collections()
    }

    // ----- crate-internal helpers ------------------------------------------

    pub(crate) fn store(&self) -> &NodeStore {
        &self.store
    }

    pub(crate) fn store_mut(&mut self) -> &mut NodeStore {
        &mut self.store
    }

    pub(crate) fn root(&self) -> u32 {
        self.root
    }

    /// Replaces the root with `root` (level >= 4) and restores the root
    /// invariants (live cells in the central half, smallest such root).
    pub(crate) fn set_root(&mut self, root: u32) {
        debug_assert!(self.store.node(root).level >= MIN_ROOT_LEVEL);
        self.root = root;
        self.normalize_root();
    }

    #[inline]
    pub(crate) fn level(&self) -> u8 {
        self.store.node(self.root).level
    }

    fn collect_if_needed(&mut self) {
        if self.store.needs_gc() {
            self.store.collect(&[self.root]);
        }
    }

    fn contains(&self, x: i64, y: i64) -> bool {
        let h = half_side(self.level());
        (-h..h).contains(&i128::from(x)) && (-h..h).contains(&i128::from(y))
    }

    /// Grows the root until the box lies in its central half (or the root
    /// covers the whole plane).
    fn reach(&mut self, min_x: i64, min_y: i64, max_x: i64, max_y: i64) {
        while self.level() < MAX_LEVEL {
            let q = half_side(self.level() - 1);
            let inside = |v: i64| (-q..q).contains(&i128::from(v));
            if inside(min_x) && inside(min_y) && inside(max_x) && inside(max_y) {
                return;
            }
            self.root = self.expand(self.root);
        }
    }

    /// Doubles the side of `n` (level <= 63), keeping it centred.
    pub(crate) fn expand(&mut self, n: u32) -> u32 {
        let node = *self.store.node(n);
        let e = self.store.empty(node.level - 1);
        let [a, b, c, d] = node.children;
        let nw = self.store.join([e, e, e, a]);
        let ne = self.store.join([e, e, b, e]);
        let sw = self.store.join([e, c, e, e]);
        let se = self.store.join([d, e, e, e]);
        self.store.join([nw, ne, sw, se])
    }

    /// Whether every live cell of `n` (level >= 4) lies in its central half.
    pub(crate) fn is_centred(&self, n: u32) -> bool {
        let node = self.store.node(n);
        let [a, b, c, d] = node.children;
        if node.level == MIN_ROOT_LEVEL {
            let bits = |i: u32| self.store.node(i).leaf_bits();
            return bits(a) & !SE_QUADRANT == 0
                && bits(b) & !SW_QUADRANT == 0
                && bits(c) & !NE_QUADRANT == 0
                && bits(d) & !NW_QUADRANT == 0;
        }
        let only = |i: u32, keep: usize| {
            self.store
                .node(i)
                .children
                .iter()
                .enumerate()
                .all(|(q, &g)| q == keep || self.store.node(g).population == 0)
        };
        only(a, 3) && only(b, 2) && only(c, 1) && only(d, 0)
    }

    /// The level `L - 1` centre of `n` (level >= 5).
    fn centre(&mut self, n: u32) -> u32 {
        let [a, b, c, d] = self.store.node(n).children;
        let quarter = |i: u32, q: usize| self.store.node(i).children[q];
        let centre = [quarter(a, 3), quarter(b, 2), quarter(c, 1), quarter(d, 0)];
        self.store.join(centre)
    }

    /// Restores the root invariants: live cells in the central half (unless
    /// the root already covers the whole plane) and the smallest such root.
    fn normalize_root(&mut self) {
        while self.level() < MAX_LEVEL && !self.is_centred(self.root) {
            self.root = self.expand(self.root);
        }
        while self.level() > MIN_ROOT_LEVEL && self.is_centred(self.root) {
            let centre = self.centre(self.root);
            if !self.is_centred(centre) {
                break;
            }
            self.root = centre;
        }
    }

    fn set_rec(&mut self, n: u32, rx: u64, ry: u64, alive: bool) -> u32 {
        let node = *self.store.node(n);
        if node.level == LEAF_LEVEL {
            let bit = 1u64 << (ry * 8 + rx);
            let bits = node.leaf_bits();
            let updated = if alive { bits | bit } else { bits & !bit };
            return if updated == bits { n } else { self.store.leaf(updated) };
        }
        if !alive && node.population == 0 {
            return n;
        }
        let half = 1u64 << (node.level - 1);
        let q = usize::from(ry >= half) * 2 + usize::from(rx >= half);
        let child = self.set_rec(node.children[q], rx & (half - 1), ry & (half - 1), alive);
        if child == node.children[q] {
            return n;
        }
        let mut children = node.children;
        children[q] = child;
        self.store.join(children)
    }

    fn clear_rec(&mut self, n: u32, nx: i128, ny: i128, rect: &Rect) -> u32 {
        let node = *self.store.node(n);
        let size = 1i128 << node.level;
        if node.population == 0 || !rect.intersects(nx, ny, size) {
            return n;
        }
        if rect.contains_square(nx, ny, size) {
            return self.store.empty(node.level);
        }
        if node.level == LEAF_LEVEL {
            return self.store.leaf(node.leaf_bits() & !rect.leaf_mask(nx, ny));
        }
        let half = size / 2;
        let offsets = [(0, 0), (half, 0), (0, half), (half, half)];
        let mut children = node.children;
        for (child, (dx, dy)) in children.iter_mut().zip(offsets) {
            *child = self.clear_rec(*child, nx + dx, ny + dy, rect);
        }
        if children == node.children { n } else { self.store.join(children) }
    }
}

impl fmt::Debug for Universe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Universe")
            .field("rule", &self.rule.to_string())
            .field("generation", &self.generation)
            .field("population", &self.population())
            .field("root_level", &self.level())
            .field("node_count", &self.node_count())
            .finish()
    }
}

/// Moves the items satisfying `pred` to the front; returns how many there are.
fn partition<T>(items: &mut [T], pred: impl Fn(&T) -> bool) -> usize {
    let mut split = 0;
    let mut i = 0;
    while i < items.len() {
        if pred(&items[i]) {
            items.swap(i, split);
            split += 1;
        }
        i += 1;
    }
    split
}

/// Builds the level-`level` node holding `points` (coordinates relative to
/// the node's top-left corner, each in `[0, 2^level)`).
fn build(store: &mut NodeStore, level: u8, points: &mut [(u64, u64)]) -> u32 {
    if points.is_empty() {
        return store.empty(level);
    }
    if level == LEAF_LEVEL {
        let bits = points.iter().fold(0u64, |acc, &(x, y)| acc | 1 << (y * 8 + x));
        return store.leaf(bits);
    }
    let half = 1u64 << (level - 1);
    let split_y = partition(points, |p| p.1 < half);
    let (top, bottom) = points.split_at_mut(split_y);
    for p in bottom.iter_mut() {
        p.1 -= half;
    }
    let split_top = partition(top, |p| p.0 < half);
    let split_bottom = partition(bottom, |p| p.0 < half);
    let (nw, ne) = top.split_at_mut(split_top);
    let (sw, se) = bottom.split_at_mut(split_bottom);
    for p in ne.iter_mut().chain(se.iter_mut()) {
        p.0 -= half;
    }
    let children = [
        build(store, level - 1, nw),
        build(store, level - 1, ne),
        build(store, level - 1, sw),
        build(store, level - 1, se),
    ];
    store.join(children)
}

/// Cell-wise OR of two nodes of the same level.
fn union(store: &mut NodeStore, a: u32, b: u32, memo: &mut HashMap<(u32, u32), u32>) -> u32 {
    if a == b {
        return a;
    }
    let (na, nb) = (*store.node(a), *store.node(b));
    if nb.population == 0 {
        return a;
    }
    if na.population == 0 {
        return b;
    }
    if na.level == LEAF_LEVEL {
        return store.leaf(na.leaf_bits() | nb.leaf_bits());
    }
    if let Some(&done) = memo.get(&(a, b)) {
        return done;
    }
    let mut children = [0u32; 4];
    for (slot, (&ca, &cb)) in children.iter_mut().zip(na.children.iter().zip(&nb.children)) {
        *slot = union(store, ca, cb, memo);
    }
    let merged = store.join(children);
    memo.insert((a, b), merged);
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_get_and_root_invariants() {
        let mut u = Universe::conway();
        let cells = [(0, 0), (-1, -1), (7, -8), (1000, -1000), (i64::MIN, i64::MAX), (i64::MAX, i64::MIN)];
        for &(x, y) in &cells {
            u.set_cell(x, y, true);
            assert!(u.get_cell(x, y));
        }
        assert_eq!(u.population(), cells.len() as u64);
        assert!(!u.get_cell(1, 1));
        assert_eq!(u.root_level(), 64);
        for &(x, y) in &cells[4..] {
            u.set_cell(x, y, false);
        }
        assert_eq!(u.population(), 4);
        // Smallest root keeping (1000, -1000) in its central half: 2^11 > 1000.
        assert_eq!(u.root_level(), 12);
        assert!(u.is_centred(u.root()));
        u.store.check_invariants();
    }

    #[test]
    fn insert_cells_matches_set_cell() {
        let cells: Vec<(i64, i64)> = (0..500i64).map(|i| ((i * 37) % 101 - 50, (i * 53) % 97 - 40)).collect();
        let mut a = Universe::conway();
        a.insert_cells(cells.iter().copied());
        let mut b = Universe::conway();
        for &(x, y) in &cells {
            b.set_cell(x, y, true);
        }
        assert_eq!(a.population(), b.population());
        for &(x, y) in &cells {
            assert!(a.get_cell(x, y));
        }
        // Union with existing content.
        a.insert_cells([(5000, 5000), (0, 0)]);
        assert!(a.get_cell(5000, 5000));
        assert_eq!(a.population(), b.population() + 1 + u64::from(!b.get_cell(0, 0)));
        a.store.check_invariants();
    }

    #[test]
    fn clear_rect_kills_exactly_the_rectangle() {
        let mut u = Universe::conway();
        u.insert_cells((-20..20).flat_map(|x| (-20..20).map(move |y| (x, y))));
        u.clear_rect(-3, -5, 6, 2);
        for x in -20..20 {
            for y in -20..20 {
                let inside = (-3..=6).contains(&x) && (-5..=2).contains(&y);
                assert_eq!(u.get_cell(x, y), !inside, "({x}, {y})");
            }
        }
        assert_eq!(u.population(), 1600 - 10 * 8);
        u.clear_rect(i64::MIN, i64::MIN, i64::MAX, i64::MAX);
        assert!(u.is_empty());
        assert_eq!(u.root_level(), 4);
    }

    #[test]
    fn step_errors_leave_the_universe_unchanged() {
        let mut u = Universe::conway();
        u.insert_cells([(0, 0), (1, 0), (0, 1), (1, 1)]);
        assert!(matches!(u.step_pow2(62), Err(HashlifeError::StepTooLarge { .. })));
        u.set_generation(u64::MAX - 1);
        assert!(matches!(u.step_pow2(1), Err(HashlifeError::GenerationOverflow { .. })));
        assert!(matches!(u.advance(2), Err(HashlifeError::GenerationOverflow { .. })));
        u.set_generation(0);
        u.set_cell(1 << 62, 0, true);
        assert!(matches!(u.step_pow2(0), Err(HashlifeError::CoordinateOverflow(_))));
        assert_eq!(u.generation(), 0);
        assert_eq!(u.population(), 5);
    }

    #[test]
    fn huge_still_life_steps_beyond_one_call() {
        let mut u = Universe::conway();
        u.insert_cells([(0, 0), (1, 0), (0, 1), (1, 1)]);
        u.advance(1u64 << 63).unwrap();
        assert_eq!(u.generation(), 1u64 << 63);
        assert_eq!(u.population(), 4);
        assert!(u.get_cell(1, 1));
    }
}
