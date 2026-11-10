//! A tiny hash-consed quadtree used only to *write* Golly macrocell text.
//!
//! The construction never inserts millions of cells one by one: each
//! distinct 2048×2048 square is built once from a dense bitmap, identical
//! squares and sub-squares are shared by content (hash-consing), and the
//! resulting directed acyclic graph is serialised as a macrocell document
//! that `hashlife::macrocell::read` interns straight into the engine.

use std::collections::HashMap;
use std::fmt::Write;

/// Node id of the empty square of any level (macrocell index 0).
pub(crate) const EMPTY: u32 = 0;
/// Level of an 8×8 leaf.
pub(crate) const LEAF_LEVEL: u8 = 3;

#[derive(Clone, Copy, Debug)]
enum DocNode {
    Empty,
    Leaf(u64),
    Inner { level: u8, children: [u32; 4] },
}

/// Hash-consed nodes awaiting serialisation. Maps are only used for
/// lookups; the output order is a depth-first walk from the root, so the
/// text is deterministic.
#[derive(Debug)]
pub(crate) struct DocBuilder {
    nodes: Vec<DocNode>,
    leaves: HashMap<u64, u32>,
    inner: HashMap<(u8, [u32; 4]), u32>,
}

impl DocBuilder {
    pub(crate) fn new() -> Self {
        Self { nodes: vec![DocNode::Empty], leaves: HashMap::new(), inner: HashMap::new() }
    }

    /// The leaf with bit `y * 8 + x` set for each live cell `(x, y)`.
    pub(crate) fn leaf(&mut self, bits: u64) -> u32 {
        if bits == 0 {
            return EMPTY;
        }
        if let Some(&id) = self.leaves.get(&bits) {
            return id;
        }
        let id = self.push(DocNode::Leaf(bits));
        self.leaves.insert(bits, id);
        id
    }

    /// The node of `level` (≥ 4) with children `[nw, ne, sw, se]` of level
    /// `level − 1`.
    pub(crate) fn join(&mut self, level: u8, children: [u32; 4]) -> u32 {
        if children == [EMPTY; 4] {
            return EMPTY;
        }
        if let Some(&id) = self.inner.get(&(level, children)) {
            return id;
        }
        let id = self.push(DocNode::Inner { level, children });
        self.inner.insert((level, children), id);
        id
    }

    fn push(&mut self, node: DocNode) -> u32 {
        // The arena never approaches u32::MAX entries for metapixel grids
        // (a few distinct nodes per distinct tile neighbourhood).
        let id = u32::try_from(self.nodes.len()).unwrap_or(u32::MAX);
        self.nodes.push(node);
        id
    }

    /// Number of distinct non-empty nodes created so far.
    pub(crate) fn node_count(&self) -> usize {
        self.nodes.len() - 1
    }

    /// Builds the level-11 node of a 2048×2048 square from a dense leaf
    /// bitmap: `leaves[ly * 256 + lx]` is the 8×8 leaf at `(8·lx, 8·ly)`.
    pub(crate) fn square_from_leaves(&mut self, leaves: &[u64]) -> u32 {
        let mut side = 256usize;
        let mut level = LEAF_LEVEL;
        let mut current: Vec<u32> = leaves.iter().map(|&bits| self.leaf(bits)).collect();
        while side > 1 {
            let half = side / 2;
            level += 1;
            let mut next = Vec::with_capacity(half * half);
            for y in 0..half {
                for x in 0..half {
                    let at = |dx: usize, dy: usize| current[(2 * y + dy) * side + 2 * x + dx];
                    next.push(self.join(level, [at(0, 0), at(1, 0), at(0, 1), at(1, 1)]));
                }
            }
            current = next;
            side = half;
        }
        current[0]
    }

    /// Serialises the graph below `root` as a macrocell document after the
    /// given header lines (the root is the last line, as Golly expects).
    pub(crate) fn to_macrocell(&self, root: u32, header: &str) -> String {
        let mut out = String::from(header);
        if root == EMPTY {
            return out;
        }
        let mut line_of = vec![0u64; self.nodes.len()];
        let mut next_line = 0u64;
        // Iterative post-order walk: children before parents.
        let mut stack = vec![(root, false)];
        while let Some((id, expanded)) = stack.pop() {
            if id == EMPTY || line_of[id as usize] != 0 {
                continue;
            }
            match self.nodes[id as usize] {
                DocNode::Empty => {}
                DocNode::Leaf(bits) => {
                    next_line += 1;
                    line_of[id as usize] = next_line;
                    write_leaf(&mut out, bits);
                }
                DocNode::Inner { level, children } => {
                    if expanded {
                        next_line += 1;
                        line_of[id as usize] = next_line;
                        let [a, b, c, d] = children.map(|child| line_of[child as usize]);
                        // Infallible: writing to a String.
                        let _ = writeln!(out, "{level} {a} {b} {c} {d}");
                    } else {
                        stack.push((id, true));
                        for &child in children.iter().rev() {
                            stack.push((child, false));
                        }
                    }
                }
            }
        }
        out
    }
}

/// Appends a Golly leaf line: rows top to bottom, `.` dead, `*` alive, `$`
/// ends a row; trailing dead cells and rows are omitted.
fn write_leaf(out: &mut String, bits: u64) {
    let last_row = 7 - (bits.leading_zeros() / 8) as usize;
    for row in 0..=last_row {
        let byte = (bits >> (8 * row)) as u8;
        if byte != 0 {
            let last_col = 7 - byte.leading_zeros() as usize;
            for col in 0..=last_col {
                out.push(if byte >> col & 1 == 1 { '*' } else { '.' });
            }
        }
        out.push('$');
    }
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaves_use_golly_syntax() {
        let mut out = String::new();
        write_leaf(&mut out, 1 << (8 + 2) | 1 << (3 * 8 + 7));
        assert_eq!(out, "$..*$$.......*$\n");
    }

    #[test]
    fn identical_squares_are_shared_and_round_trip_through_hashlife() {
        let mut builder = DocBuilder::new();
        let mut leaves = vec![0u64; 256 * 256];
        leaves[0] = 0b111 << 8; // a blinker at (0..=2, 1)
        leaves[256 * 255 + 255] = 1 << 63; // a cell at (2047, 2047)
        let a = builder.square_from_leaves(&leaves);
        let b = builder.square_from_leaves(&leaves);
        assert_eq!(a, b, "hash-consing shares equal squares");
        let nodes = builder.node_count();
        let root = builder.join(12, [a, EMPTY, EMPTY, a]);
        assert_eq!(builder.node_count(), nodes + 1);
        let text = builder.to_macrocell(root, "[M2]\n#R B3/S23\n");
        let universe = hashlife::macrocell::read(&text).unwrap();
        assert_eq!(universe.population(), 8);
        // Golly frame: a level-12 root covers x ∈ [-2048, 2048), y ∈ [-2047, 2049).
        assert!(universe.get_cell(-2048, -2046) && universe.get_cell(-2046, -2046));
        assert!(universe.get_cell(-1, 0) && universe.get_cell(2047, 2048));
    }
}
