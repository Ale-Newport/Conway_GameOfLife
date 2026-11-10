//! Per-generation age and activity tracking for [`super::LifeSim`].
//!
//! The tracker keeps the sorted list of live cells of the last tracked
//! generation together with their ages, and a sorted trail of recently
//! changed cells. After every generation the model passes the new live set;
//! a linear merge finds births and deaths:
//!
//! * **age** (view channel G): generations a cell has been alive, counting
//!   the generation it was born in as 1 and saturating at 255;
//! * **activity** (view channel B): 255 when a cell was born or died during
//!   the last tick (in any of its generations), then 16 less per tick (gone
//!   after 16 ticks). With one generation per tick this is "255 in the
//!   generation of the change, −16 per generation".
//!
//! Edits (commands) are merged the same way for the edited region, without
//! ageing anything: a painted cell starts at age 1 and its change shows in
//! the trail. Cost: O(live cells + trail) per generation, independent of the
//! engine, so tracking works on every topology including the infinite plane.

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

use crate::engine::GenStats;
use crate::error::CaError;
use crate::geometry::CellRect;
use crate::view::TexelGrid;

/// Activity lost per tick.
pub const TRAIL_DECAY: u8 = 16;

type Cell = (i64, i64);

#[inline]
fn key(c: Cell) -> (i64, i64) {
    (c.1, c.0)
}

/// Age and activity state.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Tracker {
    ages_on: bool,
    activity_on: bool,
    /// Live cells of the last tracked generation, sorted row-major.
    live: Vec<Cell>,
    /// Age of each cell of `live`.
    ages: Vec<u8>,
    /// Recently changed cells with their activity, sorted row-major.
    trail: Vec<(Cell, u8)>,
}

impl Tracker {
    /// Whether any tracking is enabled.
    pub(crate) fn enabled(&self) -> bool {
        self.ages_on || self.activity_on
    }

    pub(crate) fn ages_on(&self) -> bool {
        self.ages_on
    }

    pub(crate) fn activity_on(&self) -> bool {
        self.activity_on
    }

    /// Changes what is tracked. When tracking starts, `live` (the current
    /// live cells, sorted) become age 1 with no trail; when it stops, the
    /// data is dropped.
    pub(crate) fn configure(&mut self, ages: bool, activity: bool, live: impl FnOnce() -> Vec<Cell>) {
        let was = self.enabled();
        self.ages_on = ages;
        self.activity_on = activity;
        if !self.enabled() {
            self.live.clear();
            self.ages.clear();
            self.trail.clear();
        } else if !was {
            self.restart(live());
        }
    }

    /// Forgets history: every live cell has age 1, the trail is empty.
    pub(crate) fn restart(&mut self, live: Vec<Cell>) {
        if !self.enabled() {
            return;
        }
        self.ages = vec![1; live.len()];
        self.live = live;
        self.trail.clear();
    }

    /// Records one generation: `next` is the new live set (sorted). Returns
    /// its births and deaths. `new_tick` is true for the first generation of
    /// a tick: the trail then decays once before this generation's changes
    /// are set to full activity.
    pub(crate) fn generation(&mut self, next: Vec<Cell>, new_tick: bool) -> GenStats {
        let old_live = std::mem::take(&mut self.live);
        let old_ages = std::mem::take(&mut self.ages);
        let mut ages = Vec::with_capacity(next.len());
        let mut changed = Vec::new();
        let mut stats = GenStats::default();
        let (mut i, mut j) = (0, 0);
        while i < old_live.len() || j < next.len() {
            let take_old = j == next.len() || (i < old_live.len() && key(old_live[i]) < key(next[j]));
            if take_old {
                stats.deaths += 1;
                changed.push(old_live[i]);
                i += 1;
            } else if i < old_live.len() && old_live[i] == next[j] {
                ages.push(old_ages[i].saturating_add(1));
                i += 1;
                j += 1;
            } else {
                stats.births += 1;
                changed.push(next[j]);
                ages.push(1);
                j += 1;
            }
        }
        self.live = next;
        self.ages = ages;
        self.update_trail(&changed, new_tick);
        stats
    }

    /// Merges an edit: `cells` are the live cells now inside `rect` (sorted);
    /// cells outside `rect` are unchanged. Nothing ages.
    pub(crate) fn edit(&mut self, rect: CellRect, cells: Vec<Cell>) {
        if !self.enabled() {
            return;
        }
        let old_live = std::mem::take(&mut self.live);
        let old_ages = std::mem::take(&mut self.ages);
        let mut live = Vec::with_capacity(old_live.len() + cells.len());
        let mut ages = Vec::with_capacity(old_live.len() + cells.len());
        let mut changed = Vec::new();
        let (mut i, mut j) = (0, 0);
        while i < old_live.len() || j < cells.len() {
            let order = if j == cells.len() {
                Ordering::Less
            } else if i == old_live.len() {
                Ordering::Greater
            } else {
                key(old_live[i]).cmp(&key(cells[j]))
            };
            match order {
                Ordering::Less => {
                    let o = old_live[i];
                    if rect.contains(o.0, o.1) {
                        changed.push(o); // erased by the edit
                    } else {
                        live.push(o);
                        ages.push(old_ages[i]);
                    }
                    i += 1;
                }
                Ordering::Equal => {
                    live.push(old_live[i]);
                    ages.push(old_ages[i]);
                    i += 1;
                    j += 1;
                }
                Ordering::Greater => {
                    changed.push(cells[j]);
                    live.push(cells[j]);
                    ages.push(1);
                    j += 1;
                }
            }
        }
        self.live = live;
        self.ages = ages;
        self.update_trail(&changed, false);
    }

    /// Decays the trail (when a new tick starts) and sets `changed` (sorted)
    /// to full activity.
    fn update_trail(&mut self, changed: &[Cell], decay: bool) {
        if !self.activity_on {
            return;
        }
        let old = std::mem::take(&mut self.trail);
        let mut out = Vec::with_capacity(old.len() + changed.len());
        let mut c = changed.iter().peekable();
        for (cell, value) in old {
            while let Some(&&n) = c.peek() {
                if key(n) < key(cell) {
                    out.push((n, 255));
                    c.next();
                } else {
                    break;
                }
            }
            if c.peek() == Some(&&cell) {
                out.push((cell, 255));
                c.next();
            } else {
                let v = if decay { value.saturating_sub(TRAIL_DECAY) } else { value };
                if v > 0 {
                    out.push((cell, v));
                }
            }
        }
        out.extend(c.map(|&n| (n, 255)));
        self.trail = out;
    }

    /// Calls `f(i)` for every entry of the sorted list `list` inside `clip`.
    /// Rows are located by binary search and only the entries between the
    /// clip's columns are visited, so nothing outside the rectangle is read
    /// (cost O(rows holding entries × log n + entries inside)).
    fn each_in<T>(list: &[T], cell: impl Fn(&T) -> Cell, clip: &CellRect, mut f: impl FnMut(usize)) {
        let mut i = list.partition_point(|e| key(cell(e)) < (clip.y0, i64::MIN));
        while i < list.len() {
            let (_, y) = cell(&list[i]);
            if y > clip.y1 {
                break;
            }
            // Row `y`: its entries between columns x0 and x1, found by binary search.
            let start = i + list[i..].partition_point(|e| key(cell(e)) < (y, clip.x0));
            let end = start + list[start..].partition_point(|e| key(cell(e)) <= (y, clip.x1));
            for j in start..end {
                f(j);
            }
            // Skip the rest of the row.
            i = end + list[end..].partition_point(|e| cell(e).1 == y);
        }
    }

    /// View channels G (mean age of live cells) and B (maximum activity) for
    /// every texel of `grid`; zeros for what is not tracked.
    pub(crate) fn texels(&self, grid: &TexelGrid) -> (Vec<u8>, Vec<u8>) {
        let mut age = vec![0u8; grid.len()];
        let mut activity = vec![0u8; grid.len()];
        let Some(clip) = grid.clip else {
            return (age, activity);
        };
        if self.ages_on {
            let mut sum = vec![0u64; grid.len()];
            let mut count = vec![0u32; grid.len()];
            Self::each_in(
                &self.live,
                |&c| c,
                &clip,
                |i| {
                    let (x, y) = self.live[i];
                    let t = grid.index(x, y);
                    sum[t] += u64::from(self.ages[i]);
                    count[t] += 1;
                },
            );
            for ((a, s), n) in age.iter_mut().zip(sum).zip(count) {
                if n > 0 {
                    *a = ((s + u64::from(n) / 2) / u64::from(n)) as u8;
                }
            }
        }
        if self.activity_on {
            Self::each_in(
                &self.trail,
                |e| e.0,
                &clip,
                |i| {
                    let ((x, y), v) = self.trail[i];
                    let t = grid.index(x, y);
                    activity[t] = activity[t].max(v);
                },
            );
        }
        (age, activity)
    }

    /// Age of a live cell (0 if dead or not tracked).
    pub(crate) fn age(&self, x: i64, y: i64) -> u8 {
        if !self.ages_on {
            return 0;
        }
        self.live.binary_search_by_key(&(y, x), |&c| key(c)).map_or(0, |i| self.ages[i])
    }

    /// Activity of a cell (0 if not tracked).
    pub(crate) fn activity(&self, x: i64, y: i64) -> u8 {
        if !self.activity_on {
            return 0;
        }
        self.trail.binary_search_by_key(&(y, x), |e| key(e.0)).map_or(0, |i| self.trail[i].1)
    }

    /// Heap bytes.
    pub(crate) fn memory_bytes(&self) -> usize {
        self.live.capacity() * 16 + self.ages.capacity() + self.trail.capacity() * 24
    }

    /// Checks restored data: one age per live cell, both lists strictly
    /// sorted row-major, and no data when nothing is tracked.
    pub(crate) fn validate(&self) -> Result<(), CaError> {
        fn sorted(cells: impl Iterator<Item = Cell>) -> bool {
            let mut previous: Option<Cell> = None;
            for c in cells {
                if previous.is_some_and(|p| key(p) >= key(c)) {
                    return false;
                }
                previous = Some(c);
            }
            true
        }
        let consistent = self.live.len() == self.ages.len()
            && sorted(self.live.iter().copied())
            && sorted(self.trail.iter().map(|e| e.0))
            && (self.enabled() || (self.live.is_empty() && self.trail.is_empty()));
        if consistent { Ok(()) } else { Err(CaError::invalid("snapshot tracking data is inconsistent")) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages_and_trails_follow_a_blinker() {
        let mut t = Tracker::default();
        let horizontal = vec![(0, 1), (1, 1), (2, 1)];
        let vertical = vec![(1, 0), (1, 1), (1, 2)];
        t.configure(true, true, || horizontal.clone());
        assert_eq!(t.age(1, 1), 1);
        let stats = t.generation(vertical.clone(), true);
        assert_eq!(stats, GenStats { births: 2, deaths: 2 });
        assert_eq!((t.age(1, 1), t.age(1, 0)), (2, 1));
        assert_eq!((t.activity(0, 1), t.activity(1, 0), t.activity(1, 1)), (255, 255, 0));
        t.generation(horizontal.clone(), true);
        assert_eq!(t.age(1, 1), 3);
        assert_eq!(t.activity(0, 1), 255);
        // A still life: activity decays by 16 per generation.
        let mut s = Tracker::default();
        s.configure(false, true, Vec::new);
        s.generation(vec![(5, 5)], true);
        s.generation(vec![(5, 5)], true);
        assert_eq!(s.activity(5, 5), 255 - TRAIL_DECAY);
        for _ in 0..20 {
            s.generation(vec![(5, 5)], true);
        }
        assert_eq!(s.activity(5, 5), 0);
        assert!(s.trail.is_empty());
    }

    #[test]
    fn edits_only_touch_their_region() {
        let mut t = Tracker::default();
        t.configure(true, true, || vec![(0, 0), (5, 0), (9, 9)]);
        t.generation(vec![(0, 0), (5, 0), (9, 9)], true);
        t.edit(CellRect::new(4, 0, 6, 1), vec![(4, 1)]);
        assert_eq!(t.live, vec![(0, 0), (4, 1), (9, 9)]);
        assert_eq!((t.age(0, 0), t.age(4, 1), t.age(9, 9)), (2, 1, 2));
        assert_eq!((t.activity(5, 0), t.activity(4, 1), t.activity(0, 0)), (255, 255, 0));
    }

    #[test]
    fn texels_average_ages() {
        use crate::view::GridViewRequest;
        let mut t = Tracker::default();
        t.configure(true, false, || vec![(0, 0), (1, 0)]);
        t.generation(vec![(0, 0), (1, 0), (1, 1)], true);
        let grid = TexelGrid::plan(&GridViewRequest { x0: 0, y0: 0, x1: 1, y1: 1, max_width: 1, max_height: 1 }, None);
        let (age, activity) = t.texels(&grid);
        assert_eq!(age, vec![2], "(2 + 2 + 1) / 3 rounds to 2");
        assert_eq!(activity, vec![0]);
    }

    #[test]
    fn activity_decays_per_tick_and_marks_every_change_of_the_tick() {
        let mut t = Tracker::default();
        t.configure(false, true, Vec::new);
        // Tick 1 = three generations: a cell born in the first one, another in the last.
        t.generation(vec![(0, 0)], true);
        t.generation(vec![(0, 0)], false);
        t.generation(vec![(0, 0), (3, 3)], false);
        assert_eq!((t.activity(0, 0), t.activity(3, 3)), (255, 255), "both changed during this tick");
        // Tick 2: nothing changes; everything decays once.
        for first in [true, false, false] {
            t.generation(vec![(0, 0), (3, 3)], first);
        }
        assert_eq!((t.activity(0, 0), t.activity(3, 3)), (255 - TRAIL_DECAY, 255 - TRAIL_DECAY));
    }

    #[test]
    fn texels_only_read_entries_inside_the_request() {
        use crate::view::GridViewRequest;
        let mut t = Tracker::default();
        let live: Vec<Cell> = (0..20).flat_map(|y| (0..20).map(move |x| (x, y))).collect();
        t.configure(true, true, || live.clone());
        let mut visited = Vec::new();
        let clip = CellRect::new(5, 3, 7, 4);
        Tracker::each_in(&t.live, |&c| c, &clip, |i| visited.push(t.live[i]));
        assert_eq!(visited, vec![(5, 3), (6, 3), (7, 3), (5, 4), (6, 4), (7, 4)]);
        // Rows without entries are skipped, and so are entries left of the clip in the next row.
        let sparse = vec![(9, 0), (1, 5), (6, 5), (2, 9)];
        let mut seen = Vec::new();
        Tracker::each_in(&sparse, |&c| c, &CellRect::new(4, 0, 8, 9), |i| seen.push(sparse[i]));
        assert_eq!(seen, vec![(6, 5)]);
        let grid = TexelGrid::plan(&GridViewRequest { x0: 5, y0: 3, x1: 7, y1: 4, max_width: 8, max_height: 8 }, None);
        let (age, _) = t.texels(&grid);
        assert!(age.iter().all(|&a| a == 1));
    }
}
