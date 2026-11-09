//! Bit-level operations on 8×8 leaves and the bit-parallel base case.
//!
//! A leaf is a `u64` holding an 8×8 square: bit `y * 8 + x` is the cell in
//! column `x` (to the right) and row `y` (downward), so every byte is one row.
//!
//! The base case evolves a 16×16 square (four leaves) by 1, 2 or 4
//! generations and returns its centre 8×8. The square is held as four `u64`
//! words of four 16-bit rows each ("lanes"), and the neighbour count of all 64
//! cells of a word is computed at once with a bit-sliced adder (SWAR).

use crate::rule::Rule;

/// Cells of the north-west 4×4 quadrant of a leaf.
pub(crate) const NW_QUADRANT: u64 = 0x0000_0000_0F0F_0F0F;
/// Cells of the north-east 4×4 quadrant of a leaf.
pub(crate) const NE_QUADRANT: u64 = 0x0000_0000_F0F0_F0F0;
/// Cells of the south-west 4×4 quadrant of a leaf.
pub(crate) const SW_QUADRANT: u64 = 0x0F0F_0F0F_0000_0000;
/// Cells of the south-east 4×4 quadrant of a leaf.
pub(crate) const SE_QUADRANT: u64 = 0xF0F0_F0F0_0000_0000;

/// Every 16-bit lane except its lowest bit (cleans up after `<< 1`).
const NOT_LANE_LOW: u64 = 0xFFFE_FFFE_FFFE_FFFE;
/// Every 16-bit lane except its highest bit (cleans up after `>> 1`).
const NOT_LANE_HIGH: u64 = 0x7FFF_7FFF_7FFF_7FFF;

/// The centre 8×8 of the 16×16 square formed by four leaves (no time step).
#[inline]
pub(crate) fn centre_of_quad(nw: u64, ne: u64, sw: u64, se: u64) -> u64 {
    ((nw >> 36) & NW_QUADRANT) | ((ne >> 28) & NE_QUADRANT) | ((sw << 28) & SW_QUADRANT) | ((se << 36) & SE_QUADRANT)
}

/// Moves the four bytes of `x` into the low byte of the four 16-bit lanes.
#[inline]
fn spread(x: u32) -> u64 {
    let x = u64::from(x);
    let x = (x | (x << 16)) & 0x0000_FFFF_0000_FFFF;
    (x | (x << 8)) & 0x00FF_00FF_00FF_00FF
}

/// Inverse of [`spread`]: gathers the low byte of each 16-bit lane.
#[inline]
fn pack(x: u64) -> u32 {
    let x = x & 0x00FF_00FF_00FF_00FF;
    let x = (x | (x >> 8)) & 0x0000_FFFF_0000_FFFF;
    // Truncation keeps exactly the four gathered bytes.
    (x | (x >> 16)) as u32
}

/// What a neighbour count does under the rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fate {
    /// Dead cells with this count are born; live cells die.
    Birth,
    /// Live cells with this count survive; dead cells stay dead.
    Survival,
    /// The cell is alive next generation whatever its state.
    Both,
}

/// Rule compiled into the list of neighbour counts that produce a live cell.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Kernel {
    terms: [(u8, Fate); 9],
    len: usize,
}

impl Kernel {
    /// Compiles `rule`.
    pub(crate) fn new(rule: Rule) -> Self {
        let mut terms = [(0, Fate::Both); 9];
        let mut len = 0;
        for count in 0..=8u8 {
            let born = rule.birth() >> count & 1 == 1;
            let survives = rule.survive() >> count & 1 == 1;
            let fate = match (born, survives) {
                (true, true) => Fate::Both,
                (true, false) => Fate::Birth,
                (false, true) => Fate::Survival,
                (false, false) => continue,
            };
            terms[len] = (count, fate);
            len += 1;
        }
        Self { terms, len }
    }

    /// Evolves the 16×16 square `[nw, ne, sw, se]` by `generations` (1, 2 or
    /// 4) and returns its centre 8×8 as a leaf.
    ///
    /// Cells outside the square are treated as dead, so after `g` generations
    /// only the centre `(16 - 2g)²` cells are exact; for `g <= 4` that always
    /// includes the returned centre 8×8 (light-cone argument).
    pub(crate) fn evolve_centre(&self, [nw, ne, sw, se]: [u64; 4], generations: u32) -> u64 {
        debug_assert!(matches!(generations, 1 | 2 | 4));
        // Truncating casts select the upper or lower four rows of a leaf.
        let mut words = [
            spread(nw as u32) | (spread(ne as u32) << 8),
            spread((nw >> 32) as u32) | (spread((ne >> 32) as u32) << 8),
            spread(sw as u32) | (spread(se as u32) << 8),
            spread((sw >> 32) as u32) | (spread((se >> 32) as u32) << 8),
        ];
        for _ in 1..generations {
            words = self.generation(words, 0..4);
        }
        // The last generation only needs the centre rows 4..12 (words 1, 2).
        words = self.generation(words, 1..3);
        u64::from(pack(words[1] >> 4)) | (u64::from(pack(words[2] >> 4)) << 32)
    }

    /// One synchronous generation of the 16×16 square held in four words;
    /// only the words in `range` are computed (the others are left zero).
    #[inline]
    fn generation(&self, words: [u64; 4], range: std::ops::Range<usize>) -> [u64; 4] {
        // Per word: sum of the west and east neighbours ("pair", 0..=2) and
        // of west + centre + east ("triple", 0..=3), as two bit-planes each.
        let mut pair = [[0u64; 2]; 4];
        let mut triple = [[0u64; 2]; 4];
        for (k, &x) in words.iter().enumerate() {
            let west = (x << 1) & NOT_LANE_LOW;
            let east = (x >> 1) & NOT_LANE_HIGH;
            let p0 = west ^ east;
            let p1 = west & east;
            pair[k] = [p0, p1];
            triple[k] = [p0 ^ x, p1 | (p0 & x)];
        }
        let mut out = [0u64; 4];
        for (k, slot) in out.iter_mut().enumerate().take(range.end).skip(range.start) {
            // Triple sums of the row above / below each lane (the lane above
            // lane 0 is lane 3 of the previous word, and so on).
            let above = |plane: usize| (triple[k][plane] << 16) | if k > 0 { triple[k - 1][plane] >> 48 } else { 0 };
            let below = |plane: usize| (triple[k][plane] >> 16) | if k < 3 { triple[k + 1][plane] << 48 } else { 0 };
            let (u0, u1, d0, d1) = (above(0), above(1), below(0), below(1));
            let [p0, p1] = pair[k];
            // Add three 2-bit numbers: (u1 u0) + (p1 p0) + (d1 d0) -> s3 s2 s1 s0.
            let s0 = u0 ^ p0 ^ d0;
            let carry0 = (u0 & p0) | (d0 & (u0 ^ p0));
            let x1 = u1 ^ p1 ^ d1;
            let y1 = (u1 & p1) | (d1 & (u1 ^ p1));
            let s1 = x1 ^ carry0;
            let z1 = x1 & carry0;
            let s2 = y1 ^ z1;
            let s3 = y1 & z1;
            *slot = self.apply([s0, s1, s2, s3], words[k]);
        }
        out
    }

    /// Applies the rule to bit-sliced neighbour counts `s` (planes of weight
    /// 1, 2, 4, 8) and the current states `alive`.
    #[inline]
    fn apply(&self, s: [u64; 4], alive: u64) -> u64 {
        let mut next = 0;
        for &(count, fate) in &self.terms[..self.len] {
            let equal = if count == 8 {
                s[3]
            } else {
                let literal = |plane: usize| if count >> plane & 1 == 1 { s[plane] } else { !s[plane] };
                !s[3] & literal(2) & literal(1) & literal(0)
            };
            next |= equal
                & match fate {
                    Fate::Birth => !alive,
                    Fate::Survival => alive,
                    Fate::Both => !0,
                };
        }
        next
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference: evolve a 16×16 boolean grid with dead surroundings.
    fn naive(grid: &[[bool; 16]; 16], rule: Rule) -> [[bool; 16]; 16] {
        let mut out = [[false; 16]; 16];
        for y in 0..16i32 {
            for x in 0..16i32 {
                let mut n = 0;
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let (nx, ny) = (x + dx, y + dy);
                        if (dx, dy) != (0, 0) && (0..16).contains(&nx) && (0..16).contains(&ny) {
                            n += u32::from(grid[ny as usize][nx as usize]);
                        }
                    }
                }
                out[y as usize][x as usize] = rule.next_state(grid[y as usize][x as usize], n);
            }
        }
        out
    }

    fn to_leaves(grid: &[[bool; 16]; 16]) -> [u64; 4] {
        let mut leaves = [0u64; 4];
        for (y, row) in grid.iter().enumerate() {
            for (x, &alive) in row.iter().enumerate() {
                if alive {
                    let quadrant = (y / 8) * 2 + x / 8;
                    leaves[quadrant] |= 1 << ((y % 8) * 8 + x % 8);
                }
            }
        }
        leaves
    }

    #[test]
    fn spread_and_pack_are_inverse() {
        for x in [0u32, 1, 0xFF, 0x1234_5678, u32::MAX, 0x8000_0001] {
            assert_eq!(pack(spread(x)), x);
        }
    }

    #[test]
    fn centre_of_quad_extracts_the_middle() {
        let mut grid = [[false; 16]; 16];
        for (y, row) in grid.iter_mut().enumerate() {
            for (x, cell) in row.iter_mut().enumerate() {
                *cell = (x * 7 + y * 3) % 5 == 0;
            }
        }
        let leaves = to_leaves(&grid);
        let centre = centre_of_quad(leaves[0], leaves[1], leaves[2], leaves[3]);
        for y in 0..8 {
            for x in 0..8 {
                assert_eq!(centre >> (y * 8 + x) & 1 == 1, grid[y + 4][x + 4], "cell ({x}, {y})");
            }
        }
    }

    #[test]
    fn base_case_matches_naive_for_several_rules() {
        let rules =
            [Rule::conway(), Rule::highlife(), Rule::parse("B2/S").unwrap(), Rule::parse("B1/S012345678").unwrap()];
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        for rule in rules {
            let kernel = Kernel::new(rule);
            for _ in 0..200 {
                let mut grid = [[false; 16]; 16];
                for row in &mut grid {
                    for cell in row.iter_mut() {
                        state ^= state << 13;
                        state ^= state >> 7;
                        state ^= state << 17;
                        *cell = state.is_multiple_of(3);
                    }
                }
                for generations in [1u32, 2, 4] {
                    let mut expected = grid;
                    for _ in 0..generations {
                        expected = naive(&expected, rule);
                    }
                    let got = kernel.evolve_centre(to_leaves(&grid), generations);
                    for y in 0..8 {
                        for x in 0..8 {
                            assert_eq!(
                                got >> (y * 8 + x) & 1 == 1,
                                expected[y + 4][x + 4],
                                "rule {rule}, {generations} generations, cell ({x}, {y})"
                            );
                        }
                    }
                }
            }
        }
    }
}
