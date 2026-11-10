//! Per-metageneration readout records and their comparison with the
//! accelerated macro representation.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::accelerated::MacroModel;
use crate::error::MetapixelError;
use crate::macro_grid::MacroGrid;
use crate::readout::TileReading;

/// Most recent readout records kept (older ones are dropped; the running
/// totals keep counting).
pub const HISTORY_CAPACITY: usize = 1024;

/// A tile whose core count was neither 0 nor 29 795 at a readout time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AmbiguousTile {
    /// Column.
    pub col: u32,
    /// Row.
    pub row: u32,
    /// Core count that was read.
    pub count: u64,
}

/// The exact macro state of one metageneration, read from the physical
/// construction at `t = (metagen + 1)·P`, with the accelerated prediction
/// for the same metageneration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadoutRecord {
    /// Metageneration that was read.
    pub metagen: u64,
    /// Physical generation of the readout.
    pub generation: u64,
    /// Tiles read as ON (ambiguous tiles are not set).
    pub exact: MacroGrid,
    /// Tiles whose core count was neither 0 nor 29 795.
    pub ambiguous: Vec<AmbiguousTile>,
    /// Accelerated prediction for the same metageneration (**not** a
    /// physical result).
    pub accelerated: MacroGrid,
    /// Tiles where the exact reading differs from the prediction
    /// (ambiguous tiles included), row-major.
    pub mismatches: Vec<(u32, u32)>,
}

impl ReadoutRecord {
    /// Builds a record from row-major tile readings (one per tile of the
    /// prediction's grid).
    ///
    /// # Errors
    /// [`MetapixelError::InvalidPattern`] if `readings` does not hold exactly
    /// one reading per tile of `accelerated`.
    pub fn new(
        metagen: u64,
        generation: u64,
        readings: &[TileReading],
        accelerated: MacroGrid,
    ) -> Result<Self, MetapixelError> {
        let (width, height) = (accelerated.width(), accelerated.height());
        if readings.len() != accelerated.len() {
            return Err(MetapixelError::InvalidPattern(format!(
                "{} tile readings for a {width}×{height} grid (expected {})",
                readings.len(),
                accelerated.len()
            )));
        }
        let mut exact = MacroGrid::new(width, height)?;
        let mut ambiguous = Vec::new();
        let mut mismatches = Vec::new();
        for (i, reading) in readings.iter().enumerate() {
            let (col, row) = (i as u32 % width, i as u32 / width);
            match reading.state() {
                Some(on) => {
                    exact.set(col, row, on);
                    if on != accelerated.get(col, row) {
                        mismatches.push((col, row));
                    }
                }
                None => {
                    let count = if let TileReading::Ambiguous(c) = reading { *c } else { 0 };
                    ambiguous.push(AmbiguousTile { col, row, count });
                    mismatches.push((col, row));
                }
            }
        }
        Ok(Self { metagen, generation, exact, ambiguous, accelerated, mismatches })
    }

    /// Reading of tile `(col, row)`.
    pub fn reading(&self, col: u32, row: u32) -> TileReading {
        if let Some(a) = self.ambiguous.iter().find(|a| (a.col, a.row) == (col, row)) {
            return TileReading::Ambiguous(a.count);
        }
        if self.exact.get(col, row) { TileReading::On } else { TileReading::Off }
    }

    /// Whether the exact readout agrees with the prediction everywhere.
    pub fn matches_prediction(&self) -> bool {
        self.mismatches.is_empty()
    }
}

/// The accelerated macro representation advanced on demand.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Predictor {
    metagen: u64,
    grid: MacroGrid,
}

impl Predictor {
    /// Starts at metageneration 0 with the construction's macro pattern.
    pub fn new(initial: MacroGrid) -> Self {
        Self { metagen: 0, grid: initial }
    }

    /// Metageneration of the stored prediction.
    pub fn metagen(&self) -> u64 {
        self.metagen
    }

    /// The stored prediction (for [`Predictor::metagen`]).
    pub fn grid(&self) -> &MacroGrid {
        &self.grid
    }

    /// Prediction for `metagen` (≥ the stored one), advancing the stored
    /// state.
    pub fn advance_to(&mut self, model: &MacroModel, metagen: u64) -> &MacroGrid {
        while self.metagen < metagen {
            self.grid = model.step(&self.grid);
            self.metagen += 1;
        }
        &self.grid
    }

    /// Prediction for `metagen` without changing the stored state (earlier
    /// metagens than the stored one are not available).
    pub fn peek(&self, model: &MacroModel, metagen: u64) -> Option<MacroGrid> {
        (metagen >= self.metagen).then(|| model.run(&self.grid, metagen - self.metagen))
    }
}

/// Bounded list of readout records plus running totals.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadoutHistory {
    records: VecDeque<ReadoutRecord>,
    total_readouts: u64,
    total_mismatches: u64,
    total_ambiguous: u64,
}

impl ReadoutHistory {
    /// Appends a record, dropping the oldest beyond [`HISTORY_CAPACITY`].
    pub fn push(&mut self, record: ReadoutRecord) {
        self.total_readouts += 1;
        self.total_mismatches += record.mismatches.len() as u64;
        self.total_ambiguous += record.ambiguous.len() as u64;
        while self.records.len() >= HISTORY_CAPACITY {
            self.records.pop_front();
        }
        self.records.push_back(record);
    }

    /// Checks that the records are consistent with a `width × height` grid
    /// at generation `generation` (used when restoring snapshots): at most
    /// [`HISTORY_CAPACITY`] records of consecutive metagenerations, each read
    /// at `(metagen + 1)·P ≤ generation`, with grids of the right size and
    /// tile lists inside the grid.
    ///
    /// # Errors
    /// A description of the first inconsistency.
    pub fn validate(&self, width: u32, height: u32, generation: u64) -> Result<(), String> {
        if self.records.len() > HISTORY_CAPACITY {
            return Err(format!("{} readout records, at most {HISTORY_CAPACITY} are kept", self.records.len()));
        }
        if (self.records.len() as u64) > self.total_readouts {
            return Err("more readout records than readouts".into());
        }
        let inside = |col: u32, row: u32| col < width && row < height;
        let mut previous: Option<u64> = None;
        for record in &self.records {
            let n = record.metagen;
            if previous.is_some_and(|p| p.checked_add(1) != Some(n)) {
                return Err(format!("readout of metagen {n} does not follow metagen {previous:?}"));
            }
            previous = Some(n);
            if crate::spec::readout_generation(n) != Some(record.generation) || record.generation > generation {
                return Err(format!("metagen {n} cannot have been read at generation {}", record.generation));
            }
            for grid in [&record.exact, &record.accelerated] {
                if (grid.width(), grid.height()) != (width, height) {
                    return Err(format!("metagen {n} holds a {}×{} grid", grid.width(), grid.height()));
                }
            }
            if !record.ambiguous.iter().all(|a| inside(a.col, a.row))
                || !record.mismatches.iter().all(|&(c, r)| inside(c, r))
            {
                return Err(format!("metagen {n} lists tiles outside the grid"));
            }
        }
        Ok(())
    }

    /// Kept records, oldest first.
    pub fn records(&self) -> impl ExactSizeIterator<Item = &ReadoutRecord> {
        self.records.iter()
    }

    /// The most recent record.
    pub fn latest(&self) -> Option<&ReadoutRecord> {
        self.records.back()
    }

    /// The record of `metagen`, if still kept.
    pub fn get(&self, metagen: u64) -> Option<&ReadoutRecord> {
        let first = self.records.front()?.metagen;
        let index = usize::try_from(metagen.checked_sub(first)?).ok()?;
        self.records.get(index).filter(|r| r.metagen == metagen)
    }

    /// Readouts taken since the build (including dropped records).
    pub fn total_readouts(&self) -> u64 {
        self.total_readouts
    }

    /// Mismatching tiles summed over every readout.
    pub fn total_mismatches(&self) -> u64 {
        self.total_mismatches
    }

    /// Ambiguous tiles summed over every readout.
    pub fn total_ambiguous(&self) -> u64 {
        self.total_ambiguous
    }

    /// Approximate heap size in bytes.
    pub fn memory_bytes(&self) -> usize {
        self.records.iter().map(|r| r.exact.len() / 4 + r.ambiguous.len() * 16 + r.mismatches.len() * 8 + 128).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::programming::Programming;

    #[test]
    fn records_compare_exact_and_predicted_states() {
        let predicted = MacroGrid::from_rows(&["O.", ".O"]).unwrap();
        let readings = [TileReading::On, TileReading::On, TileReading::Ambiguous(17), TileReading::On];
        let record = ReadoutRecord::new(3, 4 * 35_328, &readings, predicted).unwrap();
        assert_eq!(record.mismatches, [(1, 0), (0, 1)]);
        assert_eq!(record.reading(0, 1), TileReading::Ambiguous(17));
        assert_eq!(record.reading(1, 1), TileReading::On);
        assert!(!record.matches_prediction());
        let short = ReadoutRecord::new(3, 4 * 35_328, &readings[..3], record.accelerated.clone());
        assert!(matches!(short, Err(MetapixelError::InvalidPattern(_))), "every tile needs a reading");
        let mut history = ReadoutHistory::default();
        for _ in 0..HISTORY_CAPACITY + 2 {
            let n = history.total_readouts();
            let mut r = record.clone();
            r.metagen = n;
            history.push(r);
        }
        assert_eq!(history.records().len(), HISTORY_CAPACITY);
        assert_eq!(history.total_mismatches(), 2 * (HISTORY_CAPACITY as u64 + 2));
        assert_eq!(history.get(1), None, "dropped");
        assert_eq!(history.get(HISTORY_CAPACITY as u64 + 1).map(|r| r.metagen), Some(HISTORY_CAPACITY as u64 + 1));
    }

    #[test]
    fn predictor_advances_and_peeks() {
        let model = MacroModel::new(Programming::conway());
        let mut p = Predictor::new(MacroGrid::from_rows(&["...", "OOO", "..."]).unwrap());
        assert_eq!(p.peek(&model, 1).unwrap().to_rows(), [".O.", ".O.", ".O."]);
        assert_eq!(p.metagen(), 0);
        assert_eq!(p.advance_to(&model, 2).to_rows(), ["...", "OOO", "..."]);
        assert!(p.peek(&model, 1).is_none());
    }
}
