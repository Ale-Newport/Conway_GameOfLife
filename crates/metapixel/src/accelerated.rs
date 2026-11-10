//! The **accelerated macro representation — not a physical simulation**.
//!
//! [`MacroModel`] applies the programmed rule directly to the macro grid,
//! one metageneration per step, with the finite-grid semantics measured on
//! the real construction: tiles outside the grid do not exist, so they are
//! permanently OFF (never send signals, never born). It exists to *predict*
//! what the exact construction should display and to flag any disagreement;
//! it is never evidence that the construction works.

use patterns::LifeRule;

use crate::macro_grid::MacroGrid;
use crate::programming::{Neighbourhood, Programming};
use crate::spec::Direction;

/// Label every user interface must show next to accelerated states.
pub const ACCELERATED_LABEL: &str = "Accelerated macro representation — not a physical simulation";
/// Label for states read from the physical construction.
pub const EXACT_LABEL: &str = "Exact: read from the physical B3/S23 construction (Hashlife)";

/// A synchronous Life-like stepper on the macro grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MacroModel {
    rule: LifeRule,
    neighbourhood: Neighbourhood,
}

impl MacroModel {
    /// The stepper for a programming.
    pub fn new(programming: Programming) -> Self {
        Self { rule: programming.rule, neighbourhood: programming.neighbourhood }
    }

    /// Number of ON listened-to neighbours of `(col, row)`; tiles outside
    /// the grid count as OFF.
    pub fn neighbour_count(&self, grid: &MacroGrid, col: u32, row: u32) -> u8 {
        let count = Direction::ALL
            .into_iter()
            .filter(|&d| self.neighbourhood.listens(d))
            .filter(|&d| {
                let (dx, dy) = d.offset();
                grid.get_signed(i64::from(col) + dx, i64::from(row) + dy)
            })
            .count();
        // At most 8 directions.
        count as u8
    }

    /// One metageneration: every tile's next state is
    /// `rule.next_state(current, neighbour_count)`; the grid size is fixed.
    pub fn step(&self, grid: &MacroGrid) -> MacroGrid {
        let mut next = grid.clone();
        for row in 0..grid.height() {
            for col in 0..grid.width() {
                let alive = grid.get(col, row);
                next.set(col, row, self.rule.next_state(alive, self.neighbour_count(grid, col, row)));
            }
        }
        next
    }

    /// `metagens` steps from `grid`.
    pub fn run(&self, grid: &MacroGrid, metagens: u64) -> MacroGrid {
        (0..metagens).fold(grid.clone(), |g, _| self.step(&g))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn life() -> MacroModel {
        MacroModel::new(Programming::conway())
    }

    #[test]
    fn blinker_oscillates_on_a_dead_boundary_grid() {
        let h = MacroGrid::from_rows(&["...", "OOO", "..."]).unwrap();
        let v = life().step(&h);
        assert_eq!(v.to_rows(), [".O.", ".O.", ".O."]);
        assert_eq!(life().step(&v), h);
        // In a 1×3 grid the blinker cannot grow vertically and dies.
        let thin = MacroGrid::from_rows(&["OOO"]).unwrap();
        assert_eq!(life().step(&thin).to_rows(), [".O."]);
        assert_eq!(life().run(&thin, 2).population(), 0);
    }

    #[test]
    fn neighbourhood_restricts_counts() {
        let grid = MacroGrid::from_rows(&["O..", "...", "..."]).unwrap();
        let moore = life();
        assert_eq!(moore.neighbour_count(&grid, 1, 1), 1);
        let vn = MacroModel::new(Programming { rule: LifeRule::conway(), neighbourhood: Neighbourhood::von_neumann() });
        assert_eq!(vn.neighbour_count(&grid, 1, 1), 0, "von Neumann tiles ignore diagonals");
        assert_eq!(vn.neighbour_count(&grid, 1, 0), 1);
    }

    #[test]
    fn b0_births_stay_inside_the_grid() {
        let rule = LifeRule::parse("B0/S").unwrap();
        let model = MacroModel::new(Programming { rule, neighbourhood: Neighbourhood::moore() });
        let single = MacroGrid::new(1, 1).unwrap();
        let next = model.step(&single);
        assert_eq!(next.population(), 1, "an isolated OFF tile is born under B0");
        assert_eq!(model.step(&next).population(), 0, "and dies again (no survival digits)");
    }
}
