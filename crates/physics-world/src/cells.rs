//! Small grid helpers shared by the generation and dynamics modules:
//! 8-neighbourhood offsets, distance transforms and smooth interpolation.

use std::collections::VecDeque;

/// D8 neighbour offsets in a fixed order (E, SE, S, SW, W, NW, N, NE).
/// Index `k` and `(k + 4) % 8` are opposite directions.
pub(crate) const DIRS: [(i32, i32); 8] = [(1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1), (0, -1), (1, -1)];

/// Length of each D8 step in cells.
pub(crate) const STEP_LEN: [f32; 8] = [
    1.0,
    std::f32::consts::SQRT_2,
    1.0,
    std::f32::consts::SQRT_2,
    1.0,
    std::f32::consts::SQRT_2,
    1.0,
    std::f32::consts::SQRT_2,
];

/// Direction opposite to `k`.
#[inline]
pub(crate) fn opposite(k: usize) -> usize {
    (k + 4) % 8
}

/// Linear index of the neighbour of `idx` in direction `k`, or `None` when
/// it lies outside a `width × height` grid.
#[inline]
pub(crate) fn neighbor(idx: usize, k: usize, width: usize, height: usize) -> Option<usize> {
    let x = (idx % width) as i64 + i64::from(DIRS[k].0);
    let y = (idx / width) as i64 + i64::from(DIRS[k].1);
    (x >= 0 && y >= 0 && (x as usize) < width && (y as usize) < height).then(|| y as usize * width + x as usize)
}

/// Whether `idx` lies on the outer ring of the grid.
#[inline]
pub(crate) fn is_border(idx: usize, width: usize, height: usize) -> bool {
    let x = idx % width;
    let y = idx / width;
    x == 0 || y == 0 || x + 1 == width || y + 1 == height
}

/// Chebyshev (8-connected) distance in cells from every cell to the nearest
/// cell for which `is_source` holds, computed by multi-source BFS. Cells with
/// no reachable source get `u32::MAX`.
pub(crate) fn distance_to(width: usize, height: usize, is_source: impl Fn(usize) -> bool) -> Vec<u32> {
    let n = width * height;
    let mut dist = vec![u32::MAX; n];
    let mut queue = VecDeque::new();
    for (i, d) in dist.iter_mut().enumerate() {
        if is_source(i) {
            *d = 0;
            queue.push_back(i);
        }
    }
    while let Some(i) = queue.pop_front() {
        let next = dist[i] + 1;
        for k in 0..8 {
            if let Some(j) = neighbor(i, k, width, height)
                && dist[j] == u32::MAX
            {
                dist[j] = next;
                queue.push_back(j);
            }
        }
    }
    dist
}

/// Hermite smoothstep of `x` between edges `e0 < e1`, in `[0, 1]`.
#[inline]
pub(crate) fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neighbours_and_opposites() {
        // 3x3 grid, centre is 4.
        let around: Vec<usize> = (0..8).filter_map(|k| neighbor(4, k, 3, 3)).collect();
        assert_eq!(around, vec![5, 8, 7, 6, 3, 0, 1, 2]);
        for k in 0..8 {
            let j = neighbor(4, k, 3, 3).unwrap();
            assert_eq!(neighbor(j, opposite(k), 3, 3), Some(4));
        }
        assert_eq!(neighbor(0, 4, 3, 3), None);
        assert!(is_border(3, 3, 3) && !is_border(4, 3, 3));
    }

    #[test]
    fn chebyshev_distance_transform() {
        let d = distance_to(5, 5, |i| i == 0);
        assert_eq!(d[0], 0);
        assert_eq!(d[24], 4); // diagonal to the opposite corner
        assert_eq!(d[4], 4);
        let none = distance_to(3, 3, |_| false);
        assert!(none.iter().all(|&v| v == u32::MAX));
    }
}
