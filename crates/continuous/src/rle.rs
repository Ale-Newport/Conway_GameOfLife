//! Codec for the 8-bit cell RLE used by Bert Chan's Lenia `animals.json`.
//!
//! Re-implemented from the format description in
//! `docs/research/continuous-life.md` §4.1 (no code was copied):
//!
//! * An optional decimal run count precedes each symbol.
//! * `.` or `b` = 0, `o` = 255, `A`…`X` = 1…24, and the two-character
//!   symbols `p`…`y` followed by `A`…`X` = `(c₀ − 'p')·24 + (c₁ − 'A') + 25`
//!   (so `yO` = 255).
//! * `$` ends a row; a count before `$` appends `count − 1` empty rows.
//!   `!` ends the pattern (end of input is accepted too). Whitespace is
//!   ignored. Rows are right-padded with zeros.
//! * Cell value = symbol value / 255. Only 2-D patterns are supported
//!   (`%` and `#` plane separators are rejected).

use simulation_core::Grid2;

use crate::error::ContinuousError;

/// Largest side accepted by the decoder.
pub const MAX_SIDE: usize = 4096;

fn err(position: usize, message: impl Into<String>) -> ContinuousError {
    ContinuousError::Rle { position, message: message.into() }
}

/// Decodes an RLE string into a grid of values in `[0, 1]`.
pub fn decode(text: &str) -> Result<Grid2<f32>, ContinuousError> {
    let mut rows: Vec<Vec<u8>> = Vec::new();
    let mut current: Vec<u8> = Vec::new();
    let mut count: Option<usize> = None;
    let mut chars = text.char_indices().peekable();
    let mut finished = false;
    while let Some((pos, c)) = chars.next() {
        if finished {
            if !c.is_whitespace() {
                return Err(err(pos, format!("unexpected `{c}` after the end marker `!`")));
            }
            continue;
        }
        let value = match c {
            c if c.is_whitespace() => continue,
            '0'..='9' => {
                let digit = c as usize - '0' as usize;
                let next = count.unwrap_or(0).checked_mul(10).and_then(|v| v.checked_add(digit));
                match next {
                    Some(v) if v <= MAX_SIDE * MAX_SIDE => count = Some(v),
                    _ => return Err(err(pos, "run count is too large")),
                }
                continue;
            }
            '$' => {
                let n = count.take().unwrap_or(1);
                if n == 0 {
                    return Err(err(pos, "run count must be at least 1"));
                }
                // Check before allocating: a huge count must not reserve rows.
                if rows.len() + n > MAX_SIDE {
                    return Err(err(pos, format!("pattern is taller than {MAX_SIDE} rows")));
                }
                rows.push(std::mem::take(&mut current));
                rows.extend(std::iter::repeat_n(Vec::new(), n - 1));
                continue;
            }
            '!' => {
                if count.is_some() {
                    return Err(err(pos, "run count without a symbol before `!`"));
                }
                finished = true;
                continue;
            }
            '%' | '#' => return Err(err(pos, "3-D and 4-D patterns are not supported")),
            '.' | 'b' => 0u8,
            'o' => 255,
            'A'..='X' => (c as u8 - b'A') + 1,
            'p'..='y' => {
                let Some((pos2, c2)) = chars.next() else {
                    return Err(err(pos, format!("`{c}` must be followed by a letter A–X")));
                };
                if !('A'..='X').contains(&c2) {
                    return Err(err(pos2, format!("`{c}{c2}` is not a valid two-letter state")));
                }
                let v = u32::from(c as u8 - b'p') * 24 + u32::from(c2 as u8 - b'A') + 25;
                u8::try_from(v).map_err(|_| err(pos, format!("state `{c}{c2}` = {v} exceeds 255")))?
            }
            other => return Err(err(pos, format!("unexpected character `{other}`"))),
        };
        let n = count.take().unwrap_or(1);
        if n == 0 {
            return Err(err(pos, "run count must be at least 1"));
        }
        if current.len() + n > MAX_SIDE {
            return Err(err(pos, format!("row is wider than {MAX_SIDE} cells")));
        }
        current.extend(std::iter::repeat_n(value, n));
    }
    if count.is_some() {
        return Err(err(text.len(), "run count without a symbol at the end"));
    }
    rows.push(current);
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    let height = rows.len();
    if width == 0 {
        return Err(err(0, "pattern has no cells"));
    }
    if height > MAX_SIDE {
        return Err(err(text.len(), format!("pattern is taller than {MAX_SIDE} rows")));
    }
    let mut grid = Grid2::new(width, height, 0.0f32);
    for (y, row) in rows.iter().enumerate() {
        for (x, &v) in row.iter().enumerate() {
            grid.set(x, y, f32::from(v) / 255.0);
        }
    }
    Ok(grid)
}

/// Encodes values (clamped to `[0, 1]` and rounded to 8 bits) as RLE.
/// Trailing zero cells of each row are omitted, as in Chan's files.
pub fn encode(grid: &Grid2<f32>) -> String {
    let quantize = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let rows: Vec<Vec<u8>> = (0..grid.height())
        .map(|y| {
            let mut row: Vec<u8> = grid.row(y).iter().map(|&v| quantize(v)).collect();
            while row.last() == Some(&0) {
                row.pop();
            }
            row
        })
        .collect();
    let mut out = String::new();
    let last = rows.len().saturating_sub(1);
    let mut y = 0;
    while y < rows.len() {
        encode_row(&rows[y], &mut out);
        if y == last {
            break;
        }
        // Merge following empty rows (except the final row, which `!` closes).
        let mut skipped = 0;
        while y + 1 + skipped < last && rows[y + 1 + skipped].is_empty() {
            skipped += 1;
        }
        if skipped > 0 {
            out.push_str(&(skipped + 1).to_string());
        }
        out.push('$');
        y += 1 + skipped;
    }
    out.push('!');
    out
}

fn encode_row(row: &[u8], out: &mut String) {
    let mut i = 0;
    while i < row.len() {
        let v = row[i];
        let run = row[i..].iter().take_while(|&&x| x == v).count();
        if run > 1 {
            out.push_str(&run.to_string());
        }
        push_symbol(v, out);
        i += run;
    }
}

fn push_symbol(v: u8, out: &mut String) {
    match v {
        0 => out.push('.'),
        1..=24 => out.push(char::from(b'A' + v - 1)),
        _ => {
            let k = v - 25;
            out.push(char::from(b'p' + k / 24));
            out.push(char::from(b'A' + k % 24));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use simulation_core::SimRng;

    /// Orbium unicaudatus, `animals.json` index 4 (MIT, Copyright (c) 2018 Bert Chan).
    const ORBIUM: &str = "7.MD6.qL$6.pKqEqFURpApBRAqQ$5.VqTrSsBrOpXpWpTpWpUpCrQ$4.CQrQsTsWsApITNPpGqGvL$3.IpIpWrOsGsBqXpJ4.LsFrL$A.DpKpSpJpDqOqUqSqE5.ExD$qL.pBpTT2.qCrGrVrWqM5.sTpP$.pGpWpD3.qUsMtItQtJ6.tL$.uFqGH3.pXtOuR2vFsK5.sM$.tUqL4.GuNwAwVxBwNpC4.qXpA$2.uH5.vBxGyEyMyHtW4.qIpL$2.wV5.tIyG3yOxQqW2.FqHpJ$2.tUS4.rM2yOyJyOyHtVpPMpFqNV$2.HsR4.pUxAyOxLxDxEuVrMqBqGqKJ$3.sLpE3.pEuNxHwRwGvUuLsHrCqTpR$3.TrMS2.pFsLvDvPvEuPtNsGrGqIP$4.pRqRpNpFpTrNtGtVtStGsMrNqNpF$5.pMqKqLqRrIsCsLsIrTrFqJpHE$6.RpSqJqPqVqWqRqKpRXE$8.OpBpIpJpFTK!";

    #[test]
    fn decodes_orbium_reference() {
        let g = decode(ORBIUM).expect("valid RLE");
        assert_eq!((g.width(), g.height()), (20, 20));
        assert!((g.sum() - 19_600.0 / 255.0).abs() < 1e-4, "sum {}", g.sum());
        let max = g.as_slice().iter().copied().fold(0.0f32, f32::max);
        assert_eq!(max, 1.0);
    }

    #[test]
    fn symbol_values() {
        let g = decode("o.AXpAyO2B$3$b!").expect("valid");
        let row: Vec<u8> = g.row(0).iter().map(|v| (v * 255.0).round() as u8).collect();
        assert_eq!(row, vec![255, 0, 1, 24, 25, 255, 2, 2]);
        assert_eq!(g.height(), 5);
        assert!(g.row(1).iter().all(|&v| v == 0.0));
    }

    #[test]
    fn round_trip_on_quantised_values() {
        let mut rng = SimRng::seed_from_u64(7);
        for (w, h) in [(9, 7), (20, 20), (1, 1), (33, 4)] {
            let mut g = Grid2::from_fn(w, h, |_, _| {
                if rng.chance(0.4) { 0.0 } else { (rng.below(256) as f32 / 255.0).max(1.0 / 255.0) }
            });
            // Make sure the last column and row are non-zero so the size survives.
            g.set(w - 1, h - 1, 1.0);
            let back = decode(&encode(&g)).expect("round trip");
            assert_eq!((back.width(), back.height()), (w, h));
            for (a, b) in back.as_slice().iter().zip(g.as_slice()) {
                assert!((a - b).abs() < 1e-6);
            }
        }
        // Empty rows in the middle and at the end.
        let mut g = Grid2::new(3, 5, 0.0f32);
        g.set(0, 0, 0.5);
        g.set(2, 3, 1.0);
        let text = encode(&g);
        let back = decode(&text).expect("round trip");
        assert_eq!((back.width(), back.height()), (3, 5), "{text}");
        assert_eq!(*back.get(2, 3), 1.0);
    }

    #[test]
    fn errors_have_positions() {
        assert!(matches!(decode("3.q!"), Err(ContinuousError::Rle { position: 3, .. })));
        assert!(matches!(decode("AB%C!"), Err(ContinuousError::Rle { position: 2, .. })));
        assert!(matches!(decode("A!x"), Err(ContinuousError::Rle { position: 2, .. })));
        assert!(matches!(decode("0A!"), Err(ContinuousError::Rle { .. })));
        assert!(matches!(decode("5"), Err(ContinuousError::Rle { .. })));
        assert!(matches!(decode("!"), Err(ContinuousError::Rle { .. })));
        assert!(matches!(decode("99999A!"), Err(ContinuousError::Rle { .. })));
        assert!(matches!(decode("A?"), Err(ContinuousError::Rle { position: 1, .. })));
        // A huge row-skip count is rejected before any row is allocated.
        assert!(matches!(decode("A16000000$A!"), Err(ContinuousError::Rle { position: 9, .. })));
        assert!(decode(&format!("A{}$A!", MAX_SIDE - 1)).is_ok());
        assert!(decode(&format!("A{MAX_SIDE}$A!")).is_err());
    }
}
