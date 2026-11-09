//! The memoised Hashlife recursion.
//!
//! `result(n)` for a node `n` of level `L ≥ 4` is the level `L − 1` centre of
//! `n` advanced by `2^e` generations, where `e = min(k, L − 2)` and `k` is the
//! step exponent of the current top-level step. Light cone: in `2^(L−2)`
//! generations information travels at most `2^(L−2)` cells, so the centre's
//! future depends only on cells inside `n`.
//!
//! ```text
//!  level L node, 4×4 grandchildren g[r][c]       9 overlapping level L−1 squares
//!  ┌────┬────┬────┬────┐                          n_ij = join(g[i][j],   g[i][j+1],
//!  │g00 │g01 │g02 │g03 │                                      g[i+1][j], g[i+1][j+1])
//!  ├────┼────┼────┼────┤  full speed (e = L−2):   m_ij = result(n_ij)        (+2^(L−3))
//!  │g10 │g11 │g12 │g13 │  slow (e < L−2):         m_ij = centre(n_ij)        (+0)
//!  ├────┼────┼────┼────┤  then, for the 2×2 blocks of the 3×3 grid of m:
//!  │g20 │g21 │g22 │g23 │     q = join(m_ij, m_i,j+1, m_i+1,j, m_i+1,j+1)
//!  ├────┼────┼────┼────┤     r_q = result(q)                           (+2^(L−3) or +2^e)
//!  │g30 │g31 │g32 │g33 │  result(n) = join(r_00, r_01, r_10, r_11)
//!  └────┴────┴────┴────┘
//! ```
//!
//! Level 4 (16×16, four leaves) is the base case, evolved bit-parallel by
//! [`Kernel::evolve_centre`]. The memo is one slot per node tagged with the
//! effective exponent `e`, so nothing has to be cleared when `k` changes: a
//! lookup only hits when the tag equals the `e` needed now. Nodes with
//! `L ≤ k + 2` always need `e = L − 2`, so their results survive any change
//! between such step sizes; larger nodes recompute and overwrite their slot.
//!
//! **GC safety.** A collection may run at the entry of `result` for an
//! uncached interior node. Every node index held in a Rust local across such
//! a call is pushed on `store.pins` (or is reachable from a pinned node), and
//! each frame truncates the pins back to their entry length when it returns.

use crate::leaf::{Kernel, centre_of_quad};
use crate::store::{LEAF_LEVEL, NONE, Node, NodeStore};

/// One top-level step in progress: the store, the compiled rule and the step
/// exponent `k`.
pub(crate) struct Stepper<'a> {
    pub(crate) store: &'a mut NodeStore,
    pub(crate) kernel: &'a Kernel,
    pub(crate) k: u32,
}

impl Stepper<'_> {
    /// Centre of `n` (level ≥ 4) after `2^min(k, L−2)` generations.
    /// The caller must have pinned `n` or hold it reachable from a GC root.
    pub(crate) fn result(&mut self, n: u32) -> u32 {
        let node = *self.store.node(n);
        let level = node.level;
        if node.population == 0 {
            return self.store.empty(level - 1);
        }
        let exp = self.k.min(u32::from(level) - 2) as u8;
        if node.result != NONE && node.result_exp == exp {
            return node.result;
        }
        let result = if level == LEAF_LEVEL + 1 {
            let bits = node.children.map(|c| self.store.node(c).leaf_bits());
            let centre = self.kernel.evolve_centre(bits, 1 << exp);
            self.store.leaf(centre)
        } else {
            if self.store.needs_gc() {
                self.store.collect(&[]);
            }
            self.recurse(&node, exp)
        };
        self.store.set_result(n, result, exp);
        result
    }

    /// Interior case (level ≥ 5); see the module documentation.
    fn recurse(&mut self, node: &Node, exp: u8) -> u32 {
        let level = node.level;
        let pin_base = self.store.pins.len();
        let ch = node.children;
        let [a, b, c, d] = ch.map(|i| self.store.node(i).children);
        let g =
            [[a[0], a[1], b[0], b[1]], [a[2], a[3], b[2], b[3]], [c[0], c[1], d[0], d[1]], [c[2], c[3], d[2], d[3]]];
        let mut m = [[0u32; 3]; 3];
        if exp == level - 2 {
            for i in 0..3 {
                for j in 0..3 {
                    let sub = match (i, j) {
                        (0, 0) => ch[0],
                        (0, 2) => ch[1],
                        (2, 0) => ch[2],
                        (2, 2) => ch[3],
                        _ => self.store.join([g[i][j], g[i][j + 1], g[i + 1][j], g[i + 1][j + 1]]),
                    };
                    self.store.pins.push(sub);
                    m[i][j] = self.result(sub);
                    self.store.pins.push(m[i][j]);
                }
            }
        } else if level == LEAF_LEVEL + 2 {
            for i in 0..3 {
                for j in 0..3 {
                    let bits = |s: &NodeStore, x: u32| s.node(x).leaf_bits();
                    let centre = centre_of_quad(
                        bits(self.store, g[i][j]),
                        bits(self.store, g[i][j + 1]),
                        bits(self.store, g[i + 1][j]),
                        bits(self.store, g[i + 1][j + 1]),
                    );
                    m[i][j] = self.store.leaf(centre);
                    self.store.pins.push(m[i][j]);
                }
            }
        } else {
            for i in 0..3 {
                for j in 0..3 {
                    let quarter = |s: &NodeStore, x: u32, q: usize| s.node(x).children[q];
                    let centre = [
                        quarter(self.store, g[i][j], 3),
                        quarter(self.store, g[i][j + 1], 2),
                        quarter(self.store, g[i + 1][j], 1),
                        quarter(self.store, g[i + 1][j + 1], 0),
                    ];
                    m[i][j] = self.store.join(centre);
                    self.store.pins.push(m[i][j]);
                }
            }
        }
        let mut out = [0u32; 4];
        for (slot, (i, j)) in out.iter_mut().zip([(0, 0), (0, 1), (1, 0), (1, 1)]) {
            let q = self.store.join([m[i][j], m[i][j + 1], m[i + 1][j], m[i + 1][j + 1]]);
            self.store.pins.push(q);
            *slot = self.result(q);
            self.store.pins.push(*slot);
        }
        let result = self.store.join(out);
        self.store.pins.truncate(pin_base);
        result
    }
}
