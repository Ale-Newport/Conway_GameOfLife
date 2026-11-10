//! SWAR ("SIMD within a register") evaluation of two-state outer-totalistic
//! rules: 64 cells per `u64`, neighbour counts computed with full adders.
//!
//! Bit `i` of a row word is the cell at column `64·k + i`. For each row the
//! engines build west- and east-shifted copies (bit `i` holds the cell at
//! column `x − 1` / `x + 1`), then [`Kernel::word`] combines the three rows
//! above, at and below the output row. The adder network follows the design
//! validated in `docs/research/life-engines-hashlife.md` §5.1; Conway's
//! `B3/S23` has a dedicated 30-operation path, every other rule goes through
//! the generic count-plane evaluation.

use patterns::LifeRule;

use crate::view::TexelGrid;

/// Rule evaluation for 64 cells at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Kernel {
    conway: bool,
    birth: [u64; 9],
    survive: [u64; 9],
}

/// `u64::MAX` when bit `n` of `mask` is set, else 0.
fn spread(mask: u16, n: usize) -> u64 {
    if mask >> n & 1 == 1 { u64::MAX } else { 0 }
}

impl Kernel {
    /// Kernel for `rule` (any birth/survival masks, including `B0`).
    pub(crate) fn new(rule: LifeRule) -> Self {
        Self {
            conway: rule == LifeRule::conway(),
            birth: std::array::from_fn(|n| spread(rule.birth, n)),
            survive: std::array::from_fn(|n| spread(rule.survive, n)),
        }
    }

    /// Next state of the 64 cells of the middle word. Each argument is
    /// `[west-shifted, centre, east-shifted]` for the rows above, at and
    /// below the output row.
    #[inline(always)]
    pub(crate) fn word(&self, above: [u64; 3], mid: [u64; 3], below: [u64; 3]) -> u64 {
        let [aw, a, ae] = above;
        let [cw, centre, ce] = mid;
        let [bw, b, be] = below;
        // Two-bit sums of the three cells above and below, and of the two
        // side cells of the middle row (the centre itself is excluded).
        let a1 = aw ^ a ^ ae;
        let a2 = (aw & a) | (ae & (aw ^ a));
        let b1 = bw ^ b ^ be;
        let b2 = (bw & b) | (be & (bw ^ b));
        let m1 = cw ^ ce;
        let m2 = cw & ce;
        // Weight-1 bit of the count and its carry into weight 2.
        let ones = a1 ^ b1 ^ m1;
        let carry = (a1 & b1) | (m1 & (a1 ^ b1));
        if self.conway {
            // count ∈ {2, 3}  ⇔  exactly one of the weight-2 terms is set.
            let x = a2 ^ b2;
            let y = m2 ^ carry;
            let exactly_one = (x ^ y) & !((a2 & b2) | (m2 & carry));
            return (ones | centre) & exactly_one;
        }
        // Weight-2 terms a2 + b2 + m2 + carry (0..=4) as bits c1, c2, c3.
        let x0 = a2 ^ b2 ^ m2;
        let x1 = (a2 & b2) | (m2 & (a2 ^ b2));
        let c1 = x0 ^ carry;
        let half = x0 & carry;
        let c2 = x1 ^ half;
        let c3 = x1 & half;
        let c0 = ones;
        // Minterms of the count (0..=8); count 8 is exactly c3.
        let below8 = !c3;
        let t00 = !c2 & !c1 & below8;
        let t01 = !c2 & c1 & below8;
        let t10 = c2 & !c1 & below8;
        let t11 = c2 & c1 & below8;
        let eq = [t00 & !c0, t00 & c0, t01 & !c0, t01 & c0, t10 & !c0, t10 & c0, t11 & !c0, t11 & c0, c3];
        let mut born = 0;
        let mut stay = 0;
        for ((e, b), s) in eq.iter().zip(&self.birth).zip(&self.survive) {
            born |= e & b;
            stay |= e & s;
        }
        (born & !centre) | (stay & centre)
    }
}

/// West-shifted copy of `row`: bit `i` of `out[k]` is the cell at column
/// `64k + i − 1`; `left` is the cell at column −1.
#[inline]
pub(crate) fn shift_west(row: &[u64], left: bool, out: &mut [u64]) {
    let mut carry = u64::from(left);
    for (o, &w) in out.iter_mut().zip(row) {
        *o = (w << 1) | carry;
        carry = w >> 63;
    }
}

/// East-shifted copy of a row of `width` cells: bit `i` of `out[k]` is the
/// cell at column `64k + i + 1`; `right` is the cell at column `width`.
/// Bits of `row` beyond `width` must be zero.
#[inline]
pub(crate) fn shift_east(row: &[u64], right: bool, width: usize, out: &mut [u64]) {
    let n = row.len();
    for k in 0..n {
        let next = if k + 1 < n { row[k + 1] << 63 } else { 0 };
        out[k] = (row[k] >> 1) | next;
    }
    out[n - 1] |= u64::from(right) << ((width - 1) % 64);
}

/// Mask of the valid bits of the last word of a `width`-cell row.
pub(crate) fn last_word_mask(width: usize) -> u64 {
    match width % 64 {
        0 => u64::MAX,
        r => (1u64 << r) - 1,
    }
}

/// Mask of bits `lo..=hi` (`lo <= hi < 64`).
#[inline]
pub(crate) fn bit_range(lo: u32, hi: u32) -> u64 {
    let upper = if hi == 63 { u64::MAX } else { (1u64 << (hi + 1)) - 1 };
    upper & !((1u64 << lo) - 1)
}

/// Bits of a reversed row: bit `i` of the result is bit `width − 1 − i` of
/// `row` (used for the twisted edges of Klein bottles and cross-surfaces).
pub(crate) fn reverse_row(row: &[u64], width: usize, out: &mut [u64]) {
    let n = row.len();
    let shift = (64 * n - width) as u32;
    for k in 0..n {
        let hi = row[n - 1 - k].reverse_bits();
        let lo = if k + 1 < n { row[n - 2 - k].reverse_bits() } else { 0 };
        out[k] = if shift == 0 { hi } else { (hi >> shift) | (lo << (64 - shift)) };
    }
}

/// Adds the live cells of `word` (bit 0 at column `x_base`, already masked to
/// the clip) to the texel row `row` of `grid`.
#[inline]
pub(crate) fn count_word(word: u64, x_base: i64, grid: &TexelGrid, row: &mut [u32]) {
    if word == 0 {
        return;
    }
    if grid.log2_k >= 6 {
        // A 64-aligned word lies inside one aligned texel block.
        row[grid.column(x_base)] += word.count_ones();
    } else {
        let mut bits = word;
        while bits != 0 {
            let i = bits.trailing_zeros();
            row[grid.column(x_base + i64::from(i))] += 1;
            bits &= bits - 1;
        }
    }
}

/// Appends the live cells of `word` (bit 0 at column `x_base`) to `out`.
#[inline]
pub(crate) fn push_cells(word: u64, x_base: i64, y: i64, out: &mut Vec<(i64, i64)>) {
    let mut bits = word;
    while bits != 0 {
        let i = bits.trailing_zeros();
        out.push((x_base + i64::from(i), y));
        bits &= bits - 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Scalar reference: next state of bit 0..64 from explicit counts.
    fn scalar(rule: LifeRule, rows: [[u64; 3]; 3]) -> u64 {
        let mut out = 0;
        for i in 0..64 {
            let bit = |w: u64| (w >> i & 1) as u8;
            let n: u8 = rows
                .iter()
                .enumerate()
                .map(|(r, [w, c, e])| bit(*w) + bit(*e) + if r == 1 { 0 } else { bit(*c) })
                .sum();
            if rule.next_state(bit(rows[1][1]) == 1, n) {
                out |= 1 << i;
            }
        }
        out
    }

    #[test]
    fn kernel_matches_scalar_counts_for_many_rules() {
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let rules = [
            LifeRule::conway(),
            LifeRule::highlife(),
            LifeRule::seeds(),
            LifeRule::day_and_night(),
            LifeRule::parse("B0123478/S01234678").unwrap(),
            LifeRule::parse("B012345678/S012345678").unwrap(),
            LifeRule::parse("B8/S8").unwrap(),
        ];
        for rule in rules {
            let kernel = Kernel::new(rule);
            for _ in 0..200 {
                let rows = [[next(), next(), next()], [next(), next(), next()], [next(), next(), next()]];
                assert_eq!(kernel.word(rows[0], rows[1], rows[2]), scalar(rule, rows), "rule {rule}");
            }
        }
    }

    #[test]
    fn shifts_and_reversal() {
        let row = [0b1011u64, 1 << 63 | 1];
        let mut out = [0u64; 2];
        shift_west(&row, true, &mut out);
        assert_eq!(out, [0b10111, 0b10], "bit 63 of word 0 is clear, so nothing carries");
        // width 70: last word holds 6 cells.
        let row = [1u64 << 63, 1 << 5];
        shift_east(&row, true, 70, &mut out);
        assert_eq!(out, [1 << 62, 1 << 4 | 1 << 5]);
        let mut rev = [0u64; 2];
        reverse_row(&[1, 1 << 5], 70, &mut rev);
        assert_eq!(rev, [1, 1 << 5], "first and last cells swap places");
        reverse_row(&[0b10, 0], 70, &mut rev);
        assert_eq!(rev, [0, 1 << 4]);
        assert_eq!(last_word_mask(70), 0b11_1111);
        assert_eq!(last_word_mask(128), u64::MAX);
        assert_eq!(bit_range(2, 4), 0b11100);
        assert_eq!(bit_range(0, 63), u64::MAX);
    }
}
