//! Hash-consed node arena with a non-moving mark-and-sweep collector.
//!
//! * Nodes live in a `Vec<Node>` and are addressed by `u32` indices. A node is
//!   immutable once created, and every distinct square exists exactly once
//!   (hash-consing), so index equality is content equality.
//! * The intern table is open addressing with linear probing. Each slot packs
//!   the upper 32 bits of the key hash (a tag that filters most mismatches
//!   without touching the arena) with the node index.
//! * Garbage collection never moves nodes: dead slots go to a free list. This
//!   lets a collection run in the middle of a Hashlife step, where Rust locals
//!   hold raw indices; every such local is kept alive through the `pins` stack.

use std::mem::size_of;

/// "No node" marker (also used for "no memoised result").
pub(crate) const NONE: u32 = u32::MAX;
/// Level of the 8×8 leaves.
pub(crate) const LEAF_LEVEL: u8 = 3;
/// Largest node level: a level-64 square covers the whole `i64` plane.
pub(crate) const MAX_LEVEL: u8 = 64;
/// Level stored in free arena slots.
const FREE_LEVEL: u8 = 0;
const EMPTY_SLOT: u64 = u64::MAX;
const TAG_MASK: u64 = 0xFFFF_FFFF_0000_0000;
const MIN_TABLE: usize = 1 << 10;

/// One quadtree node (32 bytes).
///
/// Interior nodes (level ≥ 4) store their children `[nw, ne, sw, se]`.
/// Leaves (level 3) store their 8×8 bitmap in `children[0]` (low half) and
/// `children[1]` (high half); the other two slots are zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Node {
    pub(crate) children: [u32; 4],
    /// Number of live cells, saturating at `u64::MAX` (exact up to level 31).
    pub(crate) population: u64,
    /// Memoised result (centre after `2^result_exp` generations) or [`NONE`].
    pub(crate) result: u32,
    pub(crate) level: u8,
    pub(crate) result_exp: u8,
}

const FREE_NODE: Node = Node { children: [0; 4], population: 0, result: NONE, level: FREE_LEVEL, result_exp: 0 };

impl Node {
    /// Bitmap of a leaf (meaningless for interior nodes).
    #[inline]
    pub(crate) fn leaf_bits(&self) -> u64 {
        u64::from(self.children[0]) | (u64::from(self.children[1]) << 32)
    }
}

/// Hash of a node key: a folded 64×64→128-bit multiply of the two halves of
/// the key. Both constants have all-ones upper halves, so for interior nodes
/// (whose child indices are never `u32::MAX`) neither factor can be zero.
#[inline]
fn hash_key(level: u8, c: [u32; 4]) -> u64 {
    let x = (u64::from(c[0]) | (u64::from(c[1]) << 32)) ^ 0xFFFF_FFFF_A076_1D65;
    let y = (u64::from(c[2]) | (u64::from(c[3]) << 32)) ^ 0xFFFF_FFFF_E703_7ED1 ^ u64::from(level);
    let product = u128::from(x) * u128::from(y);
    // Folding the two halves of the product (truncation intended).
    (product as u64) ^ ((product >> 64) as u64)
}

/// The node arena, intern table, canonical empty nodes and GC state.
#[derive(Clone, Debug)]
pub(crate) struct NodeStore {
    nodes: Vec<Node>,
    free: Vec<u32>,
    table: Vec<u64>,
    occupied: usize,
    empties: [u32; MAX_LEVEL as usize + 1],
    /// GC roots for nodes held in Rust locals during a step (see module docs).
    pub(crate) pins: Vec<u32>,
    node_limit: usize,
    gc_threshold: usize,
    collections: u64,
}

impl NodeStore {
    /// Creates a store with canonical empty nodes for levels 3 to 64.
    pub(crate) fn new(node_limit: usize) -> Self {
        let mut store = Self {
            nodes: Vec::new(),
            free: Vec::new(),
            table: vec![EMPTY_SLOT; MIN_TABLE],
            occupied: 0,
            empties: [NONE; MAX_LEVEL as usize + 1],
            pins: Vec::new(),
            node_limit,
            gc_threshold: node_limit,
            collections: 0,
        };
        let mut empty = store.leaf(0);
        store.empties[usize::from(LEAF_LEVEL)] = empty;
        for level in LEAF_LEVEL + 1..=MAX_LEVEL {
            empty = store.join([empty; 4]);
            store.empties[usize::from(level)] = empty;
        }
        store
    }

    /// The node at index `i`.
    #[inline]
    pub(crate) fn node(&self, i: u32) -> &Node {
        &self.nodes[i as usize]
    }

    /// The canonical empty node of `level` (3..=64).
    #[inline]
    pub(crate) fn empty(&self, level: u8) -> u32 {
        self.empties[usize::from(level)]
    }

    /// Interns the leaf with bitmap `bits`.
    #[inline]
    pub(crate) fn leaf(&mut self, bits: u64) -> u32 {
        // Truncating casts split the bitmap into its two halves.
        self.intern(LEAF_LEVEL, [bits as u32, (bits >> 32) as u32, 0, 0])
    }

    /// Interns the interior node with children `[nw, ne, sw, se]` (all of the
    /// same level, at most 63).
    #[inline]
    pub(crate) fn join(&mut self, children: [u32; 4]) -> u32 {
        let level = self.node(children[0]).level + 1;
        debug_assert!(children.iter().all(|&c| self.node(c).level + 1 == level), "children of mixed levels");
        debug_assert!(level <= MAX_LEVEL);
        self.intern(level, children)
    }

    fn intern(&mut self, level: u8, children: [u32; 4]) -> u32 {
        if (self.occupied + 1) * 2 > self.table.len() {
            self.rebuild_table(self.table.len() * 2);
        }
        let hash = hash_key(level, children);
        let tag = hash & TAG_MASK;
        let mask = self.table.len() - 1;
        // Truncation keeps the low bits used as the table position.
        let mut pos = hash as usize & mask;
        loop {
            let slot = self.table[pos];
            if slot == EMPTY_SLOT {
                break;
            }
            if slot & TAG_MASK == tag {
                let index = slot as u32;
                let node = &self.nodes[index as usize];
                if node.children == children && node.level == level {
                    return index;
                }
            }
            pos = (pos + 1) & mask;
        }
        let population = if level == LEAF_LEVEL {
            u64::from((u64::from(children[0]) | (u64::from(children[1]) << 32)).count_ones())
        } else {
            children.iter().fold(0u64, |acc, &c| acc.saturating_add(self.nodes[c as usize].population))
        };
        let index = self.alloc(Node { children, population, result: NONE, level, result_exp: 0 });
        self.table[pos] = tag | u64::from(index);
        self.occupied += 1;
        index
    }

    fn alloc(&mut self, node: Node) -> u32 {
        if let Some(index) = self.free.pop() {
            self.nodes[index as usize] = node;
            return index;
        }
        let index = match u32::try_from(self.nodes.len()) {
            Ok(i) if i != NONE => i,
            // Reaching 2^32 - 1 live nodes needs more than 128 GiB for the
            // arena alone; like `Vec` capacity overflow this is a hard limit.
            _ => panic!("hashlife node arena exhausted: more than 2^32 - 2 live nodes"),
        };
        self.nodes.push(node);
        index
    }

    /// Records the memoised result of node `n` for step exponent `exp`.
    #[inline]
    pub(crate) fn set_result(&mut self, n: u32, result: u32, exp: u8) {
        let node = &mut self.nodes[n as usize];
        node.result = result;
        node.result_exp = exp;
    }

    /// Forgets every memoised result (needed when the rule changes).
    pub(crate) fn clear_results(&mut self) {
        for node in &mut self.nodes {
            node.result = NONE;
        }
    }

    /// Number of live (allocated) nodes.
    #[inline]
    pub(crate) fn live(&self) -> usize {
        self.nodes.len() - self.free.len()
    }

    /// Whether the live node count has reached the collection threshold.
    #[inline]
    pub(crate) fn needs_gc(&self) -> bool {
        self.live() >= self.gc_threshold
    }

    /// Soft limit on live nodes.
    pub(crate) fn node_limit(&self) -> usize {
        self.node_limit
    }

    /// Changes the soft limit; the next collection happens when it is reached.
    pub(crate) fn set_node_limit(&mut self, limit: usize) {
        self.node_limit = limit;
        self.gc_threshold = limit;
    }

    /// Number of garbage collections run so far.
    pub(crate) fn collections(&self) -> u64 {
        self.collections
    }

    /// Bytes allocated by the arena, intern table and bookkeeping vectors.
    pub(crate) fn memory_bytes(&self) -> usize {
        self.nodes.capacity() * size_of::<Node>()
            + self.table.capacity() * size_of::<u64>()
            + (self.free.capacity() + self.pins.capacity()) * size_of::<u32>()
            + size_of::<Self>()
    }

    /// Collects garbage. Live nodes are those reachable from `roots`, the
    /// pins and the canonical empty nodes, following children **and**
    /// memoised results (as Golly does): the memo of everything still in use
    /// survives, so a collection never forces recomputation. Returns the
    /// number of freed nodes.
    pub(crate) fn collect(&mut self, roots: &[u32]) -> usize {
        let before = self.live();
        let marks = self.mark(roots);
        self.sweep(&marks);
        self.rebuild_table(MIN_TABLE);
        // Hysteresis: never collect again before the live set has doubled,
        // so a live set close to (or above) the soft limit cannot make
        // collections thrash.
        self.gc_threshold = self.node_limit.max(self.live().saturating_mul(2));
        self.collections += 1;
        before - self.live()
    }

    /// Collects garbage like [`NodeStore::collect`], then **compacts**: the
    /// surviving nodes are renumbered densely (keeping their relative order),
    /// `roots` are rewritten to the new indices, and the unused capacity of
    /// the arena, free list and intern table is returned to the allocator, so
    /// [`NodeStore::memory_bytes`] drops with the live set.
    ///
    /// Moving nodes is only sound when no index is held outside the store
    /// except `roots`: it must not run during a step (the pins must be empty).
    pub(crate) fn compact(&mut self, roots: &mut [u32]) -> usize {
        debug_assert!(self.pins.is_empty(), "compaction during a step");
        let before = self.live();
        let marks = self.mark(roots);
        let is_marked = |i: usize| marks[i / 64] >> (i % 64) & 1 == 1;
        let mut remap = vec![NONE; self.nodes.len()];
        let mut next = 0u32;
        for (i, slot) in remap.iter_mut().enumerate() {
            if is_marked(i) {
                *slot = next;
                next += 1;
            }
        }
        // New indices never exceed old ones, so a forward pass only
        // overwrites slots that were already read.
        for i in 0..self.nodes.len() {
            let target = remap[i];
            if target == NONE {
                continue;
            }
            let mut node = self.nodes[i];
            if node.level > LEAF_LEVEL {
                node.children = node.children.map(|c| remap[c as usize]);
            }
            if node.result != NONE {
                node.result = remap[node.result as usize];
            }
            self.nodes[target as usize] = node;
        }
        self.nodes.truncate(next as usize);
        self.nodes.shrink_to_fit();
        self.free = Vec::new();
        for index in self.empties.iter_mut().chain(roots.iter_mut()) {
            if *index != NONE {
                *index = remap[*index as usize];
            }
        }
        self.rebuild_table(MIN_TABLE);
        self.table.shrink_to_fit();
        self.gc_threshold = self.node_limit.max(self.live().saturating_mul(2));
        self.collections += 1;
        before - self.live()
    }

    /// Marks everything reachable through children and results.
    fn mark(&self, roots: &[u32]) -> Vec<u64> {
        let mut marks = vec![0u64; self.nodes.len().div_ceil(64)];
        let mut stack: Vec<u32> = Vec::with_capacity(1024);
        stack.extend_from_slice(roots);
        stack.extend_from_slice(&self.pins);
        stack.extend(self.empties.iter().copied().filter(|&e| e != NONE));
        while let Some(i) = stack.pop() {
            let (word, bit) = (i as usize / 64, 1u64 << (i % 64));
            if marks[word] & bit != 0 {
                continue;
            }
            marks[word] |= bit;
            let node = &self.nodes[i as usize];
            if node.level > LEAF_LEVEL {
                stack.extend_from_slice(&node.children);
            }
            if node.result != NONE {
                stack.push(node.result);
            }
        }
        marks
    }

    /// Frees unmarked nodes (marked nodes only refer to marked nodes).
    fn sweep(&mut self, marks: &[u64]) {
        let is_marked = |i: usize| marks[i / 64] >> (i % 64) & 1 == 1;
        let mut len = 0;
        for (i, node) in self.nodes.iter_mut().enumerate() {
            if is_marked(i) {
                len = i + 1;
            } else {
                *node = FREE_NODE;
            }
        }
        self.nodes.truncate(len);
        self.free.clear();
        // Descending order, so allocation reuses the lowest indices first.
        for i in (0..len).rev() {
            if self.nodes[i].level == FREE_LEVEL {
                // `len` never exceeds the u32 index range.
                self.free.push(i as u32);
            }
        }
    }

    /// Rebuilds the intern table from the live nodes, with at least
    /// `min_capacity` slots and a load factor of at most one half.
    fn rebuild_table(&mut self, min_capacity: usize) {
        let live = self.live();
        let capacity = min_capacity.max((live * 2 + 2).next_power_of_two()).max(MIN_TABLE);
        self.table.clear();
        self.table.resize(capacity, EMPTY_SLOT);
        let mask = capacity - 1;
        for (index, node) in self.nodes.iter().enumerate() {
            if node.level == FREE_LEVEL {
                continue;
            }
            let hash = hash_key(node.level, node.children);
            let mut pos = hash as usize & mask;
            while self.table[pos] != EMPTY_SLOT {
                pos = (pos + 1) & mask;
            }
            self.table[pos] = (hash & TAG_MASK) | index as u64;
        }
        self.occupied = live;
    }

    /// Checks every structural invariant (tests only): live children and
    /// results exist at the right level, populations are exact sums, every
    /// live node is findable in the intern table and no square is duplicated.
    #[cfg(test)]
    pub(crate) fn check_invariants(&self) {
        use std::collections::HashSet;
        let mut keys = HashSet::new();
        let mut in_table = 0;
        for &slot in &self.table {
            if slot != EMPTY_SLOT {
                in_table += 1;
                let node = self.node(slot as u32);
                assert_ne!(node.level, FREE_LEVEL, "table points at a free slot");
            }
        }
        assert_eq!(in_table, self.live(), "table size differs from live node count");
        for (i, node) in self.nodes.iter().enumerate() {
            if node.level == FREE_LEVEL {
                continue;
            }
            assert!(keys.insert((node.level, node.children)), "duplicate node {i}");
            if node.level > LEAF_LEVEL {
                let mut population = 0u64;
                for &c in &node.children {
                    let child = self.node(c);
                    assert_eq!(child.level + 1, node.level, "child level mismatch at node {i}");
                    population = population.saturating_add(child.population);
                }
                assert_eq!(population, node.population, "population of node {i}");
            } else {
                assert_eq!(u64::from(node.leaf_bits().count_ones()), node.population);
            }
            if node.result != NONE {
                let result = self.node(node.result);
                assert_eq!(result.level + 1, node.level, "result level mismatch at node {i}");
            }
            let hash = hash_key(node.level, node.children);
            let mask = self.table.len() - 1;
            let mut pos = hash as usize & mask;
            loop {
                let slot = self.table[pos];
                assert_ne!(slot, EMPTY_SLOT, "node {i} is not in the intern table");
                if slot as u32 as usize == i {
                    break;
                }
                pos = (pos + 1) & mask;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_consing_returns_identical_indices() {
        let mut store = NodeStore::new(1 << 20);
        let a = store.leaf(0x1234);
        let b = store.leaf(0x1234);
        assert_eq!(a, b);
        let c = store.leaf(0x8000_0000_0000_0001);
        let n1 = store.join([a, c, a, c]);
        let n2 = store.join([a, c, a, c]);
        assert_eq!(n1, n2);
        assert_ne!(n1, store.join([c, a, a, c]));
        assert_eq!(store.node(n1).population, 2 * 5 + 2 * 2);
        assert_eq!(store.node(n1).level, 4);
        store.check_invariants();
    }

    #[test]
    fn empty_nodes_are_canonical_for_every_level() {
        let mut store = NodeStore::new(1 << 20);
        for level in LEAF_LEVEL + 1..=MAX_LEVEL {
            let child = store.empty(level - 1);
            assert_eq!(store.join([child; 4]), store.empty(level));
            assert_eq!(store.node(store.empty(level)).population, 0);
        }
    }

    #[test]
    fn table_grows_and_collection_frees_unreachable_nodes() {
        let mut store = NodeStore::new(1 << 20);
        let keep: Vec<u32> = (1..5000u64).map(|i| store.leaf(i.wrapping_mul(0x9E37_79B9_7F4A_7C15))).collect();
        store.check_invariants();
        let root = store.join([keep[0], keep[1], keep[2], keep[3]]);
        let live_before = store.live();
        let freed = store.collect(&[root]);
        assert_eq!(freed, live_before - store.live());
        // Root, its 4 leaves and the 62 canonical empty nodes survive.
        assert_eq!(store.live(), 1 + 4 + 62);
        store.check_invariants();
        // Freed slots are reused and interning still finds survivors.
        assert_eq!(store.join([keep[0], keep[1], keep[2], keep[3]]), root);
        let fresh = store.leaf(42);
        assert!((fresh as usize) < live_before);
        store.check_invariants();
    }

    #[test]
    fn compaction_renumbers_densely_keeps_content_and_releases_capacity() {
        let mut store = NodeStore::new(1 << 20);
        let bits = |i: u64| i.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let garbage: Vec<u32> = (1..20_000u64).map(|i| store.leaf(bits(i))).collect();
        let (a, b) = (store.leaf(0xAB00), store.leaf(0xCD00));
        let node = store.join([a, b, garbage[5], a]);
        let result = store.leaf(0x1234_5678_0000);
        store.set_result(node, result, 2);
        let peak = store.memory_bytes();
        let mut roots = [node];
        let freed = store.compact(&mut roots);
        let node = roots[0];
        // 62 canonical empties, the node, its three distinct leaves and its result.
        assert_eq!(store.live(), 62 + 1 + 3 + 1);
        assert_eq!(freed, 62 + 20_000 + 3 - store.live());
        assert!(store.memory_bytes() < peak / 8, "capacity is returned: {} vs {peak}", store.memory_bytes());
        let kept = *store.node(node);
        assert_eq!(kept.children.map(|c| store.node(c).leaf_bits()), [0xAB00, 0xCD00, bits(6), 0xAB00]);
        assert_eq!((store.node(kept.result).leaf_bits(), kept.result_exp), (0x1234_5678_0000, 2));
        assert_eq!(store.node(store.empty(10)).level, 10);
        // Dense numbering, and interning finds the survivors at their new indices.
        assert!(store.free.is_empty() && store.nodes.len() == store.live());
        assert_eq!(store.leaf(0xCD00), kept.children[1]);
        assert_eq!(store.join(kept.children), node);
        assert_eq!(store.join([store.empty(9); 4]), store.empty(10));
        store.check_invariants();
    }

    #[test]
    fn collection_keeps_memoised_results_and_clearing_releases_them() {
        let mut store = NodeStore::new(1 << 20);
        let leaf = store.leaf(0xFF);
        let node = store.join([leaf; 4]);
        let result = store.leaf(0xF0F0);
        store.set_result(node, result, 2);
        store.collect(&[node]);
        assert_eq!(store.node(node).result, result, "results of live nodes are kept");
        assert_eq!(store.node(result).leaf_bits(), 0xF0F0, "and so are the result nodes");
        // Even far above the soft limit nothing in use is dropped.
        store.set_node_limit(2);
        store.collect(&[node]);
        assert_eq!(store.node(node).result, result);
        let live = store.live();
        store.clear_results();
        assert_eq!(store.collect(&[node]), 1, "only the result leaf becomes garbage");
        assert_eq!(store.live(), live - 1);
        store.check_invariants();
    }
}
