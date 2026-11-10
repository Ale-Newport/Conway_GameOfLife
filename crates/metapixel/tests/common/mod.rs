//! Shared helpers of the metapixel integration tests: tile loading, a small
//! SHA-256 implementation (FIPS 180-4) for the golden hashes of the research
//! note, and construction helpers.

#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use hashlife::Universe;
use metapixel::{MacroGrid, MetapixelSim, Programming, TileSet, WorldConfig};

/// `assets/patterns/otca` of this source tree.
pub fn otca_dir() -> PathBuf {
    TileSet::source_tree_dir()
}

/// The unprogrammed tiles, loaded once per test binary.
pub fn tiles() -> Arc<TileSet> {
    static TILES: OnceLock<Arc<TileSet>> = OnceLock::new();
    TILES
        .get_or_init(|| Arc::new(TileSet::from_dir(&otca_dir()).expect("the OTCA tiles ship in assets/patterns/otca")))
        .clone()
}

/// A simulation of `rows` with the given rule and neighbourhood.
pub fn sim(rows: &[&str], rule: &str, neighbourhood: &str) -> MetapixelSim {
    let pattern = MacroGrid::from_rows(rows).unwrap();
    let config = WorldConfig {
        programming: Programming::parse(rule, neighbourhood).unwrap(),
        width: pattern.width(),
        height: pattern.height(),
        seed: 1,
    };
    MetapixelSim::new(tiles(), config, pattern).unwrap()
}

/// Golden hash of the research note: SHA-256 of `"x y\n"` for every live
/// cell, sorted by `y` then `x`.
pub fn cell_hash(universe: &Universe) -> String {
    let mut cells: Vec<(i64, i64)> = universe.cells().collect();
    cells.sort_unstable_by_key(|&(x, y)| (y, x));
    let mut text = String::with_capacity(cells.len() * 12);
    for (x, y) in cells {
        text.push_str(&format!("{x} {y}\n"));
    }
    hex(&sha256(text.as_bytes()))
}

/// Lower-case hexadecimal.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98,
    0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
    0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8,
    0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
    0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
    0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
    0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
    0xc67178f2,
];

/// SHA-256 of `data`.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h: [u32; 8] =
        [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut message = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());
    for block in message.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for (i, word) in block.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes(*word);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut s = h;
        for i in 0..64 {
            let (a, b, c, d, e, f, g, hh) = (s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]);
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            s = [t1.wrapping_add(t2), a, b, c, d.wrapping_add(t1), e, f, g];
        }
        for (x, y) in h.iter_mut().zip(s) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 32];
    for (chunk, word) in out.as_chunks_mut::<4>().0.iter_mut().zip(h) {
        chunk.copy_from_slice(&word.to_be_bytes());
    }
    out
}
