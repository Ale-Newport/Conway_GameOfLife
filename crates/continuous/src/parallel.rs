//! Deterministic row-block parallelism.
//!
//! Every parallel loop in this crate goes through [`for_row_blocks`]: the
//! output buffer is split into blocks of whole rows and each block is computed
//! by a pure function of read-only inputs. Because every row is computed by
//! exactly the same code whatever the block boundaries are, results are
//! bitwise identical for any thread count, and identical to the serial path.
//!
//! Following the measurements in `docs/research/continuous-life.md` §7.5,
//! rows are grouped into `rows / (2·threads)` blocks (per-row tasks were
//! slower than one thread); buffers smaller than [`PARALLEL_MIN_CELLS`] (up to
//! 256² cells) run serially.

use rayon::prelude::*;

/// Buffers with fewer elements than this run serially. The research note measured
/// rayon breaking even around 256² on an idle machine; on the loaded build
/// machine the parallel 256² FFT step was slower and noisier than the serial
/// one (see `docs/models/continuous.md`, Performance), so parallelism starts
/// above 256².
pub const PARALLEL_MIN_CELLS: usize = 256 * 256 + 1;

/// Rows per block for a buffer of `len` elements and `rows` rows: all rows
/// (serial) below [`PARALLEL_MIN_CELLS`], otherwise `rows / (2·threads)`.
fn block_rows(len: usize, rows: usize) -> usize {
    if len < PARALLEL_MIN_CELLS || rows < 2 {
        return rows.max(1);
    }
    let threads = rayon::current_num_threads().max(1);
    (rows / (2 * threads)).max(1)
}

/// Calls `f(first_row, block)` for consecutive blocks of whole rows of
/// `data` (rows of `row_len` elements). Blocks run in parallel when the
/// buffer is large enough. `f` must compute each row independently of the
/// block it belongs to.
pub fn for_row_blocks<T, F>(data: &mut [T], row_len: usize, f: F)
where
    T: Send,
    F: Fn(usize, &mut [T]) + Sync,
{
    if row_len == 0 || data.is_empty() {
        return;
    }
    let rows = data.len() / row_len;
    let block = block_rows(data.len(), rows);
    if block >= rows {
        f(0, data);
        return;
    }
    data.par_chunks_mut(block * row_len).enumerate().for_each(|(i, chunk)| f(i * block, chunk));
}

/// Like [`for_row_blocks`], but also hands each block its own scratch value
/// from `pool`. The pool grows (with `make`) to the number of blocks and is
/// kept between calls, so scratch memory is allocated once.
pub fn for_row_blocks_with<T, S, F>(data: &mut [T], row_len: usize, pool: &mut Vec<S>, make: impl Fn() -> S, f: F)
where
    T: Send,
    S: Send,
    F: Fn(usize, &mut [T], &mut S) + Sync,
{
    if row_len == 0 || data.is_empty() {
        return;
    }
    let rows = data.len() / row_len;
    let block = block_rows(data.len(), rows);
    let blocks = rows.div_ceil(block);
    while pool.len() < blocks {
        pool.push(make());
    }
    if blocks == 1 {
        f(0, data, &mut pool[0]);
        return;
    }
    data.par_chunks_mut(block * row_len)
        .zip(pool.par_iter_mut())
        .enumerate()
        .for_each(|(i, (chunk, scratch))| f(i * block, chunk, scratch));
}

/// Sums `f(row)` for every row in a fixed order: per-row partial sums are
/// computed (in parallel for large grids) and then added sequentially, so the
/// result does not depend on the thread count.
pub fn sum_rows<const N: usize, F>(rows: usize, cells: usize, f: F) -> [f64; N]
where
    F: Fn(usize) -> [f64; N] + Sync,
{
    let partial: Vec<[f64; N]> = if cells < PARALLEL_MIN_CELLS {
        (0..rows).map(&f).collect()
    } else {
        (0..rows).into_par_iter().map(&f).collect()
    };
    let mut total = [0.0; N];
    for row in &partial {
        for (t, v) in total.iter_mut().zip(row) {
            *t += v;
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_cover_every_row_once() {
        let width = 300;
        let height = 400;
        let mut data = vec![0u32; width * height];
        for_row_blocks(&mut data, width, |row0, block| {
            for (i, row) in block.chunks_mut(width).enumerate() {
                row.iter_mut().for_each(|v| *v += (row0 + i) as u32 + 1);
            }
        });
        for (y, row) in data.chunks(width).enumerate() {
            assert!(row.iter().all(|&v| v == y as u32 + 1));
        }
    }

    #[test]
    fn scratch_pool_is_reused() {
        let width = 512;
        let mut data = vec![0u32; width * 512];
        let mut pool: Vec<Vec<u32>> = Vec::new();
        for _ in 0..3 {
            for_row_blocks_with(
                &mut data,
                width,
                &mut pool,
                || vec![0u32; 4],
                |_, block, scratch| {
                    scratch[0] += 1;
                    block.iter_mut().for_each(|v| *v += 1);
                },
            );
        }
        assert!(data.iter().all(|&v| v == 3));
        // Every pooled buffer was used on every call: no new allocations.
        assert!(pool.iter().all(|s| s[0] == 3), "{:?}", pool.iter().map(|s| s[0]).collect::<Vec<_>>());
    }

    #[test]
    fn ordered_sums_match_serial() {
        let s = sum_rows::<2, _>(1000, 1 << 20, |r| [r as f64, 1.0]);
        assert_eq!(s, [499_500.0, 1000.0]);
    }
}
