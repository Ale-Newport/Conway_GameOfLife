//! Golly macrocell (`.mc`) reading and writing.
//!
//! ```text
//! [M2] (golly 4.0)              header (anything after "[M2]" is ignored)
//! #R B3/S23                     rule (optional, default B3/S23)
//! #G 1024                       generation (optional, default 0)
//! #C any other comment          other '#' lines (#C, #FRAMES, #FRAME ...) are ignored
//! $$..*$...*$.***$              8×8 leaf: rows top to bottom, '.' dead, '*' alive,
//!                               '$' ends a row; trailing dead cells/rows omitted
//! 4 0 1 0 0                     node: level (log2 side) and children nw ne sw se
//! ```
//!
//! Every leaf or node line gets the next index, starting at 1; index 0 is
//! the empty square of the child level; children must be defined earlier and
//! be exactly one level lower; the last line is the root. For compatibility,
//! the "multicell" small nodes are accepted too: `1 a b c d` (a 2×2 block of
//! cell states 0/1) and levels 2 and 3 built from them.
//!
//! **Coordinates.** Golly places a root of level `L` so that it covers
//! `x ∈ [-2^(L-1), 2^(L-1))` and `y ∈ [-2^(L-1) + 1, 2^(L-1) + 1)` (`y`
//! downward): the top-left cell of the root's south-east child is `(0, 1)`.
//! [`crate::Universe`] aligns its quadtree on `(0, 0)` instead, so reading
//! shifts the tree down by one row and writing shifts it up by one (both in
//! time proportional to the number of distinct nodes). Cells therefore keep
//! exactly the coordinates Golly shows for the same file.

use std::collections::HashMap;

use crate::error::HashlifeError;
use crate::query::{Side, edge};
use crate::rule::Rule;
use crate::store::{LEAF_LEVEL, MAX_LEVEL, NodeStore};
use crate::universe::Universe;

/// One parsed line, addressed by its 1-based index.
#[derive(Clone, Copy, Debug)]
enum Entry {
    /// A 2×2 (level 1) or 4×4 (level 2) block, in leaf bit layout at the
    /// top-left of the 8×8 bitmap.
    Small { level: u8, bits: u64 },
    /// An interned node of level 3 (leaf) to 64.
    Node(u32),
    /// A level-65 root (children of level 64): only usable as the root.
    Huge([u32; 4]),
}

/// Parses a macrocell file into a new universe (rule from `#R`, generation
/// from `#G`).
///
/// # Errors
/// [`HashlifeError::Macrocell`] with the 1-based line number for a missing
/// `[M2]` header, malformed lines, references to undefined nodes, children
/// of the wrong level, unsupported rules (non-totalistic or `B0`) and
/// patterns that do not fit in 64-bit coordinates.
pub fn read(text: &str) -> Result<Universe, HashlifeError> {
    let mut universe = Universe::conway();
    read_into(&mut universe, text)?;
    Ok(universe)
}

/// Replaces the pattern, rule and generation of `universe` with the content
/// of a macrocell file. Nodes are interned into the existing store, so
/// memoised results the universe already holds for the same squares (and
/// the same rule) are reused — useful to reset or restore a long run.
///
/// # Errors
/// As [`read`]; on error `universe` keeps its pattern, rule and generation.
pub fn read_into(universe: &mut Universe, text: &str) -> Result<(), HashlifeError> {
    let mut rule = Rule::conway();
    let mut generation = 0u64;
    let mut entries: Vec<Entry> = vec![Entry::Small { level: 0, bits: 0 }];
    let mut seen_header = false;
    let mut last_line = 1;
    for (index, raw) in text.lines().enumerate() {
        let line_no = index + 1;
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let fail = |message: String| HashlifeError::Macrocell { line: line_no, message };
        if !seen_header {
            if !line.starts_with("[M2]") {
                return Err(fail("missing `[M2]` header: this is not a macrocell file".into()));
            }
            seen_header = true;
            continue;
        }
        last_line = line_no;
        if let Some(text) = line.strip_prefix("#R") {
            rule = Rule::parse(text).map_err(|e| fail(e.to_string()))?;
        } else if let Some(text) = line.strip_prefix("#G") {
            let text = text.trim();
            generation =
                text.parse().map_err(|_| fail(format!("generation `{text}` is not an integer in 0..=2^64-1")))?;
        } else if line.starts_with('#') {
            // Comments, timeline frames and other extensions are ignored.
        } else if line.starts_with(['.', '*', '$']) {
            let bits = parse_leaf(line).map_err(fail)?;
            entries.push(Entry::Node(universe.store_mut().leaf(bits)));
        } else {
            let entry = parse_node(line, &entries, universe.store_mut()).map_err(fail)?;
            entries.push(entry);
        }
    }
    if !seen_header {
        return Err(HashlifeError::Macrocell { line: 1, message: "empty input: expected a `[M2]` header".into() });
    }
    let root = match entries.last() {
        Some(&root) if entries.len() > 1 => golly_root_to_ours(universe.store_mut(), root)
            .map_err(|message| HashlifeError::Macrocell { line: last_line, message })?,
        _ => universe.store().empty(LEAF_LEVEL + 1),
    };
    universe.set_rule(rule);
    universe.set_generation(generation);
    universe.set_root(root);
    Ok(())
}

/// Serialises a universe as a macrocell file: header, `#R` rule, `#G`
/// generation (when non-zero) and one line per distinct non-empty node in
/// children-first order. Cells keep their coordinates when the file is
/// opened in Golly or read back with [`read`].
pub fn write(universe: &Universe) -> String {
    let store = universe.store();
    let mut writer = Writer {
        store,
        out: format!("[M2] (emergence-lab hashlife {})\n#R {}\n", env!("CARGO_PKG_VERSION"), universe.rule()),
        count: 0,
        leaves: HashMap::new(),
        nodes: HashMap::new(),
        windows: HashMap::new(),
    };
    if universe.generation() > 0 {
        writer.out.push_str(&format!("#G {}\n", universe.generation()));
    }
    if universe.is_empty() {
        return writer.out;
    }
    let root = universe.root();
    let level = universe.level();
    if edge(store, root, Side::Top, &mut HashMap::new()) != Some(0) {
        // Top row empty: the root shifted up by one row is the Golly root.
        writer.window_up(root, store.empty(level));
    } else {
        // Only possible for a level-64 root with a cell on row i64::MIN:
        // the Golly root is one level larger. With e empty, m = children of
        // the root and wu = window_up, its 4×4 grandchildren are
        //   [0, wu(e,m00), wu(e,m01), 0] [0, wu(m00,m10), wu(m01,m11), 0]
        //   [0, wu(m10,e), wu(m11,e), 0] [0, 0, 0, 0]
        let e = store.empty(level - 1);
        let [m00, m01, m10, m11] = store.node(root).children;
        let b01 = writer.window_up(e, m00);
        let b02 = writer.window_up(e, m01);
        let b11 = writer.window_up(m00, m10);
        let b12 = writer.window_up(m01, m11);
        let b21 = writer.window_up(m10, e);
        let b22 = writer.window_up(m11, e);
        let nw = writer.node(level, [0, b01, 0, b11]);
        let ne = writer.node(level, [b02, 0, b12, 0]);
        let sw = writer.node(level, [0, b21, 0, 0]);
        let se = writer.node(level, [b22, 0, 0, 0]);
        writer.node(level + 1, [nw, ne, sw, se]);
    }
    writer.out
}

/// Parses an 8×8 leaf line.
fn parse_leaf(line: &str) -> Result<u64, String> {
    let (mut x, mut y, mut bits) = (0u32, 0u32, 0u64);
    // Saturating counters: trailing dead cells and empty rows beyond the leaf
    // are harmless, and absurdly long lines cannot overflow.
    for ch in line.chars() {
        match ch {
            '.' => x = x.saturating_add(1),
            '*' => {
                if x > 7 || y > 7 {
                    return Err(format!("live cell at ({x}, {y}) is outside the 8×8 leaf"));
                }
                bits |= 1 << (y * 8 + x);
                x += 1;
            }
            '$' => {
                x = 0;
                y = y.saturating_add(1);
            }
            other => return Err(format!("unexpected character `{other}` in a leaf line (only '.', '*' and '$')")),
        }
    }
    Ok(bits)
}

/// Level of a parsed entry.
fn entry_level(entry: Entry, store: &NodeStore) -> u8 {
    match entry {
        Entry::Small { level, .. } => level,
        Entry::Node(n) => store.node(n).level,
        Entry::Huge(_) => MAX_LEVEL + 1,
    }
}

/// Parses `level nw ne sw se`.
fn parse_node(line: &str, entries: &[Entry], store: &mut NodeStore) -> Result<Entry, String> {
    let fields: Vec<&str> = line.split_whitespace().collect();
    if fields.len() != 5 {
        return Err(format!("expected `level nw ne sw se` (5 integers), found {} fields", fields.len()));
    }
    let mut numbers = [0u64; 5];
    for (slot, field) in numbers.iter_mut().zip(&fields) {
        *slot = field.parse().map_err(|_| format!("`{field}` is not a non-negative integer"))?;
    }
    let level = match u8::try_from(numbers[0]) {
        Ok(l @ 1..=65) => l,
        _ => {
            return Err(format!(
                "node level {} is outside 1..=65 (65 is the largest square of the i64 plane plus the Golly origin row)",
                numbers[0]
            ));
        }
    };
    let refs = [numbers[1], numbers[2], numbers[3], numbers[4]];
    if level == 1 {
        if refs.iter().any(|&v| v > 1) {
            return Err("level-1 nodes hold cell states, which must be 0 or 1 for a two-state rule".into());
        }
        let bits = refs[0] | refs[1] << 1 | refs[2] << 8 | refs[3] << 9;
        return Ok(Entry::Small { level: 1, bits });
    }
    let child_level = level - 1;
    let mut children = [Entry::Small { level: 0, bits: 0 }; 4];
    for (slot, &r) in children.iter_mut().zip(&refs) {
        *slot = if r == 0 {
            match child_level {
                1 | 2 => Entry::Small { level: child_level, bits: 0 },
                _ => Entry::Node(store.empty(child_level)),
            }
        } else {
            let entry = usize::try_from(r)
                .ok()
                .and_then(|i| entries.get(i))
                .copied()
                .ok_or_else(|| format!("node {r} is referenced before it is defined"))?;
            let found = entry_level(entry, store);
            if found != child_level {
                return Err(format!(
                    "child {r} has level {found}, but a level-{level} node needs level-{child_level} children"
                ));
            }
            entry
        };
    }
    let small = |e: Entry| if let Entry::Small { bits, .. } = e { bits } else { 0 };
    let node = |e: Entry| if let Entry::Node(n) = e { n } else { 0 };
    Ok(match level {
        2 => Entry::Small {
            level: 2,
            bits: small(children[0]) | small(children[1]) << 2 | small(children[2]) << 16 | small(children[3]) << 18,
        },
        3 => {
            let bits =
                small(children[0]) | small(children[1]) << 4 | small(children[2]) << 32 | small(children[3]) << 36;
            Entry::Node(store.leaf(bits))
        }
        65 => Entry::Huge(children.map(node)),
        _ => Entry::Node(store.join(children.map(node))),
    })
}

/// Converts the last parsed entry (a root in Golly's frame, see the module
/// docs) into an equivalent node in [`Universe`]'s frame (level >= 4).
fn golly_root_to_ours(store: &mut NodeStore, root: Entry) -> Result<u32, String> {
    let too_big = || "the pattern does not fit in 64-bit coordinates".to_string();
    let mut memo = HashMap::new();
    match root {
        Entry::Small { level, bits } => Ok(small_root(store, level, bits)),
        Entry::Node(n) if store.node(n).level == LEAF_LEVEL => {
            Ok(small_root(store, LEAF_LEVEL, store.node(n).leaf_bits()))
        }
        Entry::Node(n) if store.node(n).level < MAX_LEVEL => {
            // Pad to level L + 1 while shifting down one row (e = empty,
            // g = children, wd = window_down); the 4×4 grandchildren are
            //   [e, e, e, e]  [e, wd(e,g0), wd(e,g1), e]
            //   [e, wd(g0,g2), wd(g1,g3), e]  [e, wd(g2,e), wd(g3,e), e]
            let level = store.node(n).level;
            let e = store.empty(level - 1);
            let [g0, g1, g2, g3] = store.node(n).children;
            let b11 = window_down(store, e, g0, &mut memo);
            let b12 = window_down(store, e, g1, &mut memo);
            let b21 = window_down(store, g0, g2, &mut memo);
            let b22 = window_down(store, g1, g3, &mut memo);
            let b31 = window_down(store, g2, e, &mut memo);
            let b32 = window_down(store, g3, e, &mut memo);
            let nw = store.join([e, e, e, b11]);
            let ne = store.join([e, e, b12, e]);
            let sw = store.join([e, b21, e, b31]);
            let se = store.join([b22, e, b32, e]);
            Ok(store.join([nw, ne, sw, se]))
        }
        Entry::Node(n) => {
            // Level 64: its bottom row would be y = 2^63.
            if edge(store, n, Side::Bottom, &mut HashMap::new()) == Some(u64::MAX) {
                return Err(too_big());
            }
            Ok(window_down(store, store.empty(MAX_LEVEL), n, &mut memo))
        }
        Entry::Huge(children) => {
            // Inverse of the level-65 case of `write`: only the windows below
            // may hold cells, everything else must be empty.
            let side = 1u64 << (MAX_LEVEL - 1); // side of the level-63 grandchildren
            let gg = children.map(|c| store.node(c).children);
            let cell = |r: usize, c: usize| gg[(r / 2) * 2 + c / 2][(r % 2) * 2 + c % 2];
            for r in 0..4 {
                for c in 0..4 {
                    let i = cell(r, c);
                    let fits = match (r, c) {
                        (0, 1 | 2) => edge(store, i, Side::Top, &mut HashMap::new()).is_none_or(|v| v == side - 1),
                        (1, 1 | 2) => true,
                        (2, 1 | 2) => edge(store, i, Side::Bottom, &mut HashMap::new()).is_none_or(|v| v < side - 1),
                        _ => store.node(i).population == 0,
                    };
                    if !fits {
                        return Err(too_big());
                    }
                }
            }
            let m = [
                window_down(store, cell(0, 1), cell(1, 1), &mut memo),
                window_down(store, cell(0, 2), cell(1, 2), &mut memo),
                window_down(store, cell(1, 1), cell(2, 1), &mut memo),
                window_down(store, cell(1, 2), cell(2, 2), &mut memo),
            ];
            Ok(store.join(m))
        }
    }
}

/// Level-4 node holding a Golly root of side `s = 2^level <= 8` whose cells
/// are `bits` (leaf layout): it covers `x in [-s/2, s/2)`, `y in [-s/2 + 1, s/2 + 1)`.
fn small_root(store: &mut NodeStore, level: u8, bits: u64) -> u32 {
    let half = 1u64 << (level - 1);
    let mut leaves = [0u64; 4];
    let mut rest = bits;
    while rest != 0 {
        let bit = u64::from(rest.trailing_zeros());
        rest &= rest - 1;
        // Offsets inside the level-4 square [-8, 8)²: x + 8 and y + 8.
        let (x, y) = (bit % 8 + 8 - half, bit / 8 + 9 - half);
        leaves[usize::from(y >= 8) * 2 + usize::from(x >= 8)] |= 1 << ((y % 8) * 8 + x % 8);
    }
    let children = leaves.map(|b| store.leaf(b));
    store.join(children)
}

/// The square of the same level as `upper` and `lower` that starts at the
/// last row of `upper` when `upper` is stacked on top of `lower`.
fn window_down(store: &mut NodeStore, upper: u32, lower: u32, memo: &mut HashMap<(u32, u32), u32>) -> u32 {
    let (u, d) = (*store.node(upper), *store.node(lower));
    if u.population == 0 && d.population == 0 {
        return store.empty(u.level);
    }
    if let Some(&done) = memo.get(&(upper, lower)) {
        return done;
    }
    let result = if u.level == LEAF_LEVEL {
        store.leaf((u.leaf_bits() >> 56) | (d.leaf_bits() << 8))
    } else {
        let (uc, dc) = (u.children, d.children);
        let children = [
            window_down(store, uc[2], dc[0], memo),
            window_down(store, uc[3], dc[1], memo),
            window_down(store, dc[0], dc[2], memo),
            window_down(store, dc[1], dc[3], memo),
        ];
        store.join(children)
    };
    memo.insert((upper, lower), result);
    result
}

/// Emits node lines with content-based deduplication.
struct Writer<'a> {
    store: &'a NodeStore,
    out: String,
    count: u64,
    leaves: HashMap<u64, u64>,
    nodes: HashMap<(u8, [u64; 4]), u64>,
    windows: HashMap<(u32, u32), u64>,
}

impl Writer<'_> {
    /// Line index of the leaf `bits` (0 if empty), emitting it if new.
    fn leaf(&mut self, bits: u64) -> u64 {
        if bits == 0 {
            return 0;
        }
        if let Some(&index) = self.leaves.get(&bits) {
            return index;
        }
        let mut rest = bits;
        while rest != 0 {
            // Truncation selects the current row.
            let row = rest as u8;
            for x in 0..8 - row.leading_zeros() {
                self.out.push(if row >> x & 1 == 1 { '*' } else { '.' });
            }
            self.out.push('$');
            rest >>= 8;
        }
        self.out.push('\n');
        self.count += 1;
        self.leaves.insert(bits, self.count);
        self.count
    }

    /// Line index of the node with the given child line indices (0 if
    /// empty), emitting it if new.
    fn node(&mut self, level: u8, children: [u64; 4]) -> u64 {
        if children == [0; 4] {
            return 0;
        }
        if let Some(&index) = self.nodes.get(&(level, children)) {
            return index;
        }
        let [nw, ne, sw, se] = children;
        self.out.push_str(&format!("{level} {nw} {ne} {sw} {se}\n"));
        self.count += 1;
        self.nodes.insert((level, children), self.count);
        self.count
    }

    /// Line index of the square starting at row 1 of `upper` stacked on
    /// `lower` (that is, `upper` shifted up by one row).
    fn window_up(&mut self, upper: u32, lower: u32) -> u64 {
        let (u, d) = (*self.store.node(upper), *self.store.node(lower));
        if u.population == 0 && d.population == 0 {
            return 0;
        }
        if let Some(&index) = self.windows.get(&(upper, lower)) {
            return index;
        }
        let index = if u.level == LEAF_LEVEL {
            self.leaf((u.leaf_bits() >> 8) | (d.leaf_bits() << 56))
        } else {
            let (uc, dc) = (u.children, d.children);
            let children = [
                self.window_up(uc[0], uc[2]),
                self.window_up(uc[1], uc[3]),
                self.window_up(uc[2], dc[0]),
                self.window_up(uc[3], dc[1]),
            ];
            self.node(u.level, children)
        };
        self.windows.insert((upper, lower), index);
        index
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted_cells(u: &Universe) -> Vec<(i64, i64)> {
        let mut cells: Vec<_> = u.cells().collect();
        cells.sort_unstable();
        cells
    }

    #[test]
    fn golly_origin_convention() {
        // Level-4 root whose south-east leaf has only its top-left cell: Golly (0, 1).
        let u = read("[M2] (golly 4.0)\n#R B3/S23\n*$\n4 0 0 0 1\n").unwrap();
        assert_eq!(sorted_cells(&u), vec![(0, 1)]);
        // Small "multicell" roots follow the same convention.
        let u = read("[M2]\n1 0 0 0 1\n").unwrap();
        assert_eq!(sorted_cells(&u), vec![(0, 1)]);
        let u = read("[M2]\n1 1 0 0 0\n2 0 0 0 1\n").unwrap();
        assert_eq!(sorted_cells(&u), vec![(0, 1)]);
        let u = read("[M2]\n*$\n").unwrap();
        assert_eq!(sorted_cells(&u), vec![(-4, -3)]);
    }

    #[test]
    fn writes_golly_style_files() {
        let mut u = Universe::conway();
        u.insert_cells([(0, 1)]);
        assert_eq!(write(&u).lines().skip(2).collect::<Vec<_>>(), vec!["*$", "4 0 0 0 1"]);
        u.set_generation(77);
        let text = write(&u);
        assert!(text.starts_with("[M2] (emergence-lab hashlife"));
        assert!(text.contains("\n#R B3/S23\n#G 77\n"));
    }

    #[test]
    fn round_trip_preserves_cells_rule_and_generation() {
        let mut u = Universe::with_rule(Rule::highlife());
        let cells: Vec<(i64, i64)> = (0..300i64).map(|i| ((i * 97) % 211 - 105, (i * 61) % 173 - 90)).collect();
        u.insert_cells(cells);
        u.advance(37).unwrap();
        let back = read(&write(&u)).unwrap();
        assert_eq!(back.rule(), Rule::highlife());
        assert_eq!(back.generation(), 37);
        assert_eq!(sorted_cells(&back), sorted_cells(&u));
        assert_eq!(write(&back), write(&u), "writing is deterministic");
    }

    #[test]
    fn extreme_rows_round_trip() {
        let mut u = Universe::conway();
        u.insert_cells([(0, i64::MIN), (5, i64::MAX), (i64::MIN, 0), (i64::MAX, -3)]);
        let text = write(&u);
        assert!(text.lines().last().unwrap().starts_with("65 "), "needs a level-65 Golly root");
        let back = read(&text).unwrap();
        assert_eq!(sorted_cells(&back), sorted_cells(&u));
        // A Golly level-64 root with a cell on its bottom row (y = 2^63) does not fit.
        let mut text = String::from("[M2]\n$$$$$$$*$\n");
        for level in 4..=64 {
            text.push_str(&format!("{level} 0 0 {} 0\n", level - 3));
        }
        assert!(matches!(read(&text), Err(HashlifeError::Macrocell { .. })));
    }

    #[test]
    fn reports_errors_with_line_numbers() {
        let cases = [
            ("not a macrocell\n", 1),
            ("[M2]\n#R B3-jknr/S23\n", 2),
            ("[M2]\n#R B0/S8\n", 2),
            ("[M2]\n#G -5\n", 2),
            ("[M2]\n..*x$\n", 2),
            ("[M2]\n*$\n4 0 2 0 0\n", 3),
            ("[M2]\n*$\n5 1 0 0 0\n", 3),
            ("[M2]\n*$\n4 1 1 1\n", 3),
            ("[M2]\n1 0 2 0 0\n", 2),
            ("[M2]\n66 0 0 0 0\n", 2),
            ("[M2]\n.........*$\n", 2),
            ("", 1),
        ];
        for (text, line) in cases {
            match read(text) {
                Err(HashlifeError::Macrocell { line: got, .. }) => assert_eq!(got, line, "{text:?}"),
                other => panic!("{text:?} should fail, got {other:?}"),
            }
        }
    }

    #[test]
    fn empty_pattern_and_comments() {
        let u = read("[M2] (golly 2.0)\n#C hello\n#FRAMES 1 0 2^3\n\n").unwrap();
        assert!(u.is_empty());
        assert_eq!(read(&write(&u)).unwrap().population(), 0);
    }
}
