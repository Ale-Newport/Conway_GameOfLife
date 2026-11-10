//! Two-state Life-like engines behind one trait, [`LifeEngine`].
//!
//! | Engine | Storage | Topologies | Best for |
//! |---|---|---|---|
//! | [`DenseEngine`] | one `u8` per cell, double buffer, ghost ring, rayon row bands | all five (the infinite plane as an auto-growing window) | small worlds, simplest reference |
//! | [`BitEngine`] | 64 cells per `u64`, SWAR full adders, ghost border, rayon row bands | bounded, torus, Klein bottle, cross-surface | large finite worlds, dense soups |
//! | [`SparseEngine`] | 64×64 bit tiles in a hash map, active-tile set | infinite plane (`i64` coordinates) | unbounded, localised activity |
//! | [`HashlifeEngine`] | `hashlife::Universe` quadtree | infinite plane | regular patterns, huge jumps in time |
//!
//! All engines run every `B/S` rule. Rules with `B0` (empty space comes
//! alive) are accepted only on finite worlds by the dense and bit engines,
//! with the literal semantics: every cell of the finite world, including one
//! with no live neighbour, follows the rule; on the bounded plane the
//! outside is permanently dead. On the infinite plane `B0` would make the
//! whole plane alive, so it is rejected (no background-phase emulation).
//!
//! Every engine produces exactly the same generations; the property test
//! `tests/engines_agree.rs` checks this against the naive [`reference`](mod@reference)
//! stepper on random soups and rules.

mod bit;
mod dense;
mod hashlife_engine;
mod padded;
pub mod reference;
mod sparse;
pub(crate) mod swar;

pub use bit::BitEngine;
pub use dense::DenseEngine;
pub use hashlife_engine::{BORDER_CELL_BUDGET, HashlifeEngine};
pub(crate) use padded::{Lut, Padded, StepCounts};
pub use sparse::SparseEngine;

use std::any::Any;

use patterns::LifeRule;
use serde::{Deserialize, Serialize};
use simulation_core::Topology;

use crate::error::CaError;
use crate::geometry::{CellRect, World};
use crate::view::TexelGrid;

/// Largest finite world the dense engine accepts (cells).
pub const MAX_DENSE_CELLS: u64 = 1 << 28;
/// Largest finite world the bit engine accepts (cells).
pub const MAX_BIT_CELLS: u64 = 1 << 32;
/// Largest window the dense engine grows to on the infinite plane (cells).
pub const MAX_DENSE_WINDOW_CELLS: u64 = 1 << 26;

/// The four engine implementations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineKind {
    /// [`DenseEngine`].
    Dense,
    /// [`BitEngine`].
    Bit,
    /// [`SparseEngine`].
    Sparse,
    /// [`HashlifeEngine`].
    Hashlife,
}

impl EngineKind {
    /// All engines in display order.
    pub const ALL: [EngineKind; 4] = [Self::Dense, Self::Bit, Self::Sparse, Self::Hashlife];

    /// Stable machine name (`"dense"`, `"bit"`, `"sparse"`, `"hashlife"`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dense => "dense",
            Self::Bit => "bit",
            Self::Sparse => "sparse",
            Self::Hashlife => "hashlife",
        }
    }

    /// Display label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Dense => "Dense (byte per cell)",
            Self::Bit => "Bit-packed SWAR",
            Self::Sparse => "Sparse tiles",
            Self::Hashlife => "Hashlife",
        }
    }

    /// Numeric code used by the `engine` metric (0 dense, 1 bit, 2 sparse, 3 hashlife).
    pub fn code(self) -> u8 {
        self as u8
    }

    /// Parses a machine name.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == name)
    }

    /// Checks that this engine can run `rule` on `world`.
    ///
    /// # Errors
    /// [`CaError::UnsupportedTopology`], [`CaError::UnsupportedRule`] (B0 on
    /// the infinite plane) or [`CaError::InvalidSize`] (world too large).
    pub fn check(self, world: &World, rule: LifeRule) -> Result<(), CaError> {
        let finite = world.is_finite();
        match (self, finite) {
            (Self::Bit, false) => {
                return Err(CaError::UnsupportedTopology {
                    engine: self.label(),
                    topology: Topology::Infinite.label(),
                    hint: "use the Dense, Sparse or Hashlife engine on the infinite plane",
                });
            }
            (Self::Sparse | Self::Hashlife, true) => {
                return Err(CaError::UnsupportedTopology {
                    engine: self.label(),
                    topology: world.topology.label(),
                    hint: "this engine only runs the infinite plane; use the Dense or Bit engine for finite worlds",
                });
            }
            _ => {}
        }
        if !finite && rule.has_b0() {
            return Err(CaError::UnsupportedRule {
                rule: rule.to_string(),
                reason: "B0 (birth with no live neighbour) would fill the infinite plane; \
                         run B0 rules on a finite world with the Dense or Bit engine"
                    .into(),
            });
        }
        let limit = match self {
            Self::Dense => MAX_DENSE_CELLS,
            Self::Bit => MAX_BIT_CELLS,
            _ => u64::MAX,
        };
        if finite && world.cell_count() > limit {
            return Err(CaError::InvalidSize {
                width: u64::from(world.width),
                height: u64::from(world.height),
                reason: format!("the {} engine accepts at most {limit} cells", self.label()),
            });
        }
        Ok(())
    }
}

/// Births and deaths of one generation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenStats {
    /// Cells that were dead and became alive.
    pub births: u64,
    /// Cells that were alive and died.
    pub deaths: u64,
}

impl std::ops::Add for GenStats {
    type Output = Self;

    /// Component-wise sum.
    fn add(self, other: Self) -> Self {
        Self { births: self.births + other.births, deaths: self.deaths + other.deaths }
    }
}

/// A two-state Life-like engine.
///
/// Coordinates are `i64`, `x` to the right and `y` downward. On finite worlds
/// single-cell operations map coordinates through the world's topology
/// ([`World::map`]): they wrap on tori, Klein bottles and cross-surfaces and
/// are ignored outside a bounded plane. Rectangle operations
/// ([`LifeEngine::cells_in_rect`], [`LifeEngine::count_in_rect`],
/// [`LifeEngine::clear_rect`], [`LifeEngine::count_live`]) are clipped to
/// the world and never wrap.
pub trait LifeEngine: Send + Sync {
    /// Which implementation this is.
    fn kind(&self) -> EngineKind;
    /// Display name.
    fn name(&self) -> &'static str {
        self.kind().label()
    }
    /// The world the engine runs.
    fn world(&self) -> World;
    /// The rule being run.
    fn rule(&self) -> LifeRule;
    /// Replaces the rule (cells are kept).
    ///
    /// # Errors
    /// Rules the engine cannot run on its world (see [`EngineKind::check`]).
    fn set_rule(&mut self, rule: LifeRule) -> Result<(), CaError>;
    /// Whether the cell is alive.
    fn get_cell(&self, x: i64, y: i64) -> bool;
    /// Makes sure that live cells can be stored anywhere inside `rect`
    /// (together with the cells already alive). Only the dense engine on the
    /// infinite plane can fail: its window is limited to
    /// [`MAX_DENSE_WINDOW_CELLS`]. Writes of live cells that an engine cannot
    /// hold are dropped, so callers that must not lose cells (engine
    /// switches, editing commands) reserve their bounding box first.
    ///
    /// # Errors
    /// [`CaError::LimitExceeded`] when the cells cannot be held.
    fn reserve(&mut self, rect: CellRect) -> Result<(), CaError> {
        let _ = rect;
        Ok(())
    }
    /// Sets or clears one cell.
    fn set_cell(&mut self, x: i64, y: i64, alive: bool);
    /// Sets or clears many cells.
    fn set_cells(&mut self, cells: &[(i64, i64)], alive: bool) {
        for &(x, y) in cells {
            self.set_cell(x, y, alive);
        }
    }
    /// Kills every cell.
    fn clear(&mut self);
    /// Kills every cell inside `rect` (clipped to the world).
    fn clear_rect(&mut self, rect: CellRect);
    /// Replaces the whole state by the listed live cells.
    fn load_cells(&mut self, cells: &[(i64, i64)]) {
        self.clear();
        self.set_cells(cells, true);
    }
    /// Every live cell, sorted row-major (by `y`, then `x`).
    fn export_cells(&self) -> Vec<(i64, i64)>;
    /// Live cells inside `rect`, sorted row-major. Callers that must bound
    /// the list check [`LifeEngine::count_in_rect`] first.
    fn cells_in_rect(&self, rect: CellRect) -> Vec<(i64, i64)>;
    /// Number of live cells inside `rect`, counted without listing them and
    /// exactly up to `limit`: the smaller of the count and `limit` (so
    /// `limit = u64::MAX` gives the exact count). The dense engine scans
    /// bytes, the bit engine popcounts words, the sparse engine adds whole
    /// tiles' populations and popcounts the tiles cut by the border, and
    /// Hashlife adds cached quadtree populations, descending only into
    /// squares crossing the border.
    fn count_in_rect(&self, rect: CellRect, limit: u64) -> u64;
    /// Advances one generation. Returns its births and deaths, or `None`
    /// when the engine does not count them (Hashlife).
    ///
    /// # Errors
    /// Coordinate or memory limits (Hashlife near the `i64` edge, the dense
    /// window outgrowing its limit); the state is unchanged on error.
    fn step(&mut self) -> Result<Option<GenStats>, CaError>;
    /// Advances `n` generations (Hashlife jumps; the others step `n` times).
    ///
    /// # Errors
    /// As [`LifeEngine::step`].
    fn advance(&mut self, n: u64) -> Result<(), CaError> {
        for _ in 0..n {
            self.step()?;
        }
        Ok(())
    }
    /// Number of live cells (maintained incrementally; constant time).
    fn population(&self) -> u64;
    /// Smallest rectangle containing every live cell.
    fn bounding_box(&self) -> Option<CellRect>;
    /// Adds the number of live cells of every texel of `grid` (cells inside
    /// `grid.clip` only) to `out`, row-major.
    fn count_live(&self, grid: &TexelGrid, out: &mut [u32]);
    /// Approximate heap memory in bytes.
    fn memory_bytes(&self) -> usize;
    /// Downcasting support, e.g. to reach the Hashlife universe for macrocell
    /// export.
    fn as_any(&self) -> &dyn Any;
    /// Mutable downcasting support.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

/// Creates an empty engine of `kind` for `rule` on `world`.
///
/// # Errors
/// See [`EngineKind::check`].
pub fn create_engine(kind: EngineKind, world: World, rule: LifeRule) -> Result<Box<dyn LifeEngine>, CaError> {
    kind.check(&world, rule)?;
    Ok(match kind {
        EngineKind::Dense => Box::new(DenseEngine::new(world, rule)?),
        EngineKind::Bit => Box::new(BitEngine::new(world, rule)?),
        EngineKind::Sparse => Box::new(SparseEngine::new(rule)?),
        EngineKind::Hashlife => Box::new(HashlifeEngine::new(rule)?),
    })
}

/// Most live cells an engine switch copies. A switch lists every live cell
/// (16 bytes each, so at most 1 GiB here) and loads the list into the new
/// engine; [`switch_engine`] refuses larger patterns, and `LifeSim`'s
/// automatic choice keeps the running engine for them.
pub const MAX_SWITCH_CELLS: u64 = 1 << 26;

/// Moves the exact state of `engine` into a new engine of `kind` (same
/// world and rule). Returns `engine` unchanged when it already is of `kind`.
///
/// # Errors
/// See [`EngineKind::check`]; [`CaError::LimitExceeded`] when `engine`
/// holds more than [`MAX_SWITCH_CELLS`] live cells (checked before anything
/// is listed) or when the new engine cannot hold every live cell (the dense
/// engine's window on the infinite plane). No cell is ever dropped.
pub fn switch_engine(engine: Box<dyn LifeEngine>, kind: EngineKind) -> Result<Box<dyn LifeEngine>, CaError> {
    if engine.kind() == kind {
        return Ok(engine);
    }
    transfer(engine.as_ref(), kind)
}

/// A new engine of `kind` holding exactly the live cells of `engine`.
///
/// # Errors
/// As [`switch_engine`].
pub(crate) fn transfer(engine: &dyn LifeEngine, kind: EngineKind) -> Result<Box<dyn LifeEngine>, CaError> {
    let population = engine.population();
    if population > MAX_SWITCH_CELLS {
        return Err(CaError::LimitExceeded(format!(
            "switching to the {} engine copies every live cell one by one; this pattern has {population} live \
             cells, more than the {MAX_SWITCH_CELLS} a switch copies",
            kind.label()
        )));
    }
    let mut next = create_engine(kind, engine.world(), engine.rule())?;
    if let Some(bbox) = engine.bounding_box() {
        next.reserve(bbox)?;
    }
    next.load_cells(&engine.export_cells());
    Ok(next)
}

/// User choice of engine: automatic or a fixed implementation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineChoice {
    /// Chosen by [`select_engine`].
    #[default]
    Auto,
    /// Always the dense engine.
    Dense,
    /// Always the bit-packed engine.
    Bit,
    /// Always the sparse engine.
    Sparse,
    /// Always Hashlife.
    Hashlife,
}

impl EngineChoice {
    /// The fixed engine, or `None` for [`EngineChoice::Auto`].
    pub fn fixed(self) -> Option<EngineKind> {
        match self {
            Self::Auto => None,
            Self::Dense => Some(EngineKind::Dense),
            Self::Bit => Some(EngineKind::Bit),
            Self::Sparse => Some(EngineKind::Sparse),
            Self::Hashlife => Some(EngineKind::Hashlife),
        }
    }

    /// Machine name (`"auto"` or an [`EngineKind::as_str`]).
    pub fn as_str(self) -> &'static str {
        self.fixed().map_or("auto", EngineKind::as_str)
    }

    /// Parses a machine name.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "auto" => Some(Self::Auto),
            "dense" => Some(Self::Dense),
            "bit" => Some(Self::Bit),
            "sparse" => Some(Self::Sparse),
            "hashlife" => Some(Self::Hashlife),
            _ => None,
        }
    }
}

/// Finite worlds up to this many cells use the dense engine automatically.
pub const AUTO_DENSE_MAX_CELLS: u64 = 128 * 128;
/// On the infinite plane, ticks of at least this many generations select
/// Hashlife (when the count is a power of two).
pub const AUTO_HASHLIFE_MIN_STEP: u64 = 16;
/// On the infinite plane, patterns with at least this many cells and a tile
/// regularity of [`AUTO_HASHLIFE_MIN_REGULARITY`] select Hashlife.
pub const AUTO_HASHLIFE_MIN_POPULATION: u64 = 1 << 16;
/// See [`AUTO_HASHLIFE_MIN_POPULATION`].
pub const AUTO_HASHLIFE_MIN_REGULARITY: f64 = 0.5;
/// Largest population whose tile regularity `LifeSim`'s automatic choice
/// measures (measuring lists every live cell). Above it a running Hashlife
/// engine is kept, since leaving it would list every cell too, and the other
/// engines are chosen as for an irregular pattern, so the choice stays
/// bounded in time and memory for any population.
pub const AUTO_REGULARITY_MAX_POPULATION: u64 = 1 << 20;

/// Inputs of the automatic engine choice.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SelectionInput {
    /// World to run.
    pub world: World,
    /// Rule to run.
    pub rule: LifeRule,
    /// Live cells.
    pub population: u64,
    /// Generations advanced per tick.
    pub generations_per_tick: u64,
    /// Fraction of repeated 64×64 tiles (see [`tile_regularity`]); `None`
    /// when not measured.
    pub regularity: Option<f64>,
}

/// The automatic engine choice:
///
/// * finite world of at most [`AUTO_DENSE_MAX_CELLS`] cells → Dense;
/// * larger finite world → Bit;
/// * infinite plane with a power-of-two tick of at least
///   [`AUTO_HASHLIFE_MIN_STEP`] generations (large jumps) → Hashlife;
/// * infinite plane with a large ([`AUTO_HASHLIFE_MIN_POPULATION`]) and
///   regular (tile regularity ≥ [`AUTO_HASHLIFE_MIN_REGULARITY`]) pattern
///   at a power-of-two tick → Hashlife;
/// * any other infinite case (active, chaotic or small) → Sparse.
///
/// # Errors
/// B0 rules on the infinite plane, or a finite world beyond every engine's limit.
pub fn select_engine(input: &SelectionInput) -> Result<EngineKind, CaError> {
    let kind = if input.world.is_finite() {
        if input.world.cell_count() <= AUTO_DENSE_MAX_CELLS { EngineKind::Dense } else { EngineKind::Bit }
    } else {
        let pow2 = input.generations_per_tick.is_power_of_two();
        let jumps = pow2 && input.generations_per_tick >= AUTO_HASHLIFE_MIN_STEP;
        let regular = pow2
            && input.population >= AUTO_HASHLIFE_MIN_POPULATION
            && input.regularity.is_some_and(|r| r >= AUTO_HASHLIFE_MIN_REGULARITY);
        if jumps || regular { EngineKind::Hashlife } else { EngineKind::Sparse }
    };
    kind.check(&input.world, input.rule)?;
    Ok(kind)
}

/// Spatial regularity of a pattern: `1 − distinct / non-empty` over its
/// 64×64 tiles, tile contents being compared through a 64-bit FNV-1a hash of
/// their cell offsets (a collision could only make two different tiles count
/// as one, which merely nudges this heuristic). 0 for chaotic soups whose
/// tiles are all different; close to 1 for glider streams, agar or
/// metapixel arrays made of repeated tiles. `cells` must be sorted row-major.
pub fn tile_regularity(cells: &[(i64, i64)]) -> f64 {
    let mut tiles: Vec<((i64, i64), u32)> = cells
        .iter()
        .map(|&(x, y)| ((x.div_euclid(64), y.div_euclid(64)), (y.rem_euclid(64) * 64 + x.rem_euclid(64)) as u32))
        .collect();
    if tiles.is_empty() {
        return 0.0;
    }
    tiles.sort_unstable();
    // FNV-1a over the cell offsets of each tile: equal tiles hash equally.
    let mut hashes: Vec<u64> = Vec::new();
    let mut current = tiles[0].0;
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for &(key, bit) in &tiles {
        if key != current {
            hashes.push(hash);
            current = key;
            hash = 0xcbf2_9ce4_8422_2325;
        }
        hash = (hash ^ u64::from(bit)).wrapping_mul(0x0100_0000_01b3);
    }
    hashes.push(hash);
    let total = hashes.len();
    hashes.sort_unstable();
    hashes.dedup();
    1.0 - hashes.len() as f64 / total as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world(topology: Topology, w: u32, h: u32) -> World {
        World::new(topology, w, h).unwrap()
    }

    #[test]
    fn engine_topology_support() {
        let finite = world(Topology::Torus, 64, 64);
        let infinite = world(Topology::Infinite, 64, 64);
        let life = LifeRule::conway();
        assert!(EngineKind::Dense.check(&finite, life).is_ok());
        assert!(EngineKind::Dense.check(&infinite, life).is_ok());
        assert!(EngineKind::Bit.check(&infinite, life).is_err());
        assert!(EngineKind::Sparse.check(&finite, life).is_err());
        assert!(EngineKind::Hashlife.check(&infinite, life).is_ok());
        let b0 = LifeRule::parse("B0123478/S01234678").unwrap();
        assert!(EngineKind::Bit.check(&finite, b0).is_ok());
        let err = EngineKind::Sparse.check(&infinite, b0).unwrap_err().to_string();
        assert!(err.contains("B0"), "{err}");
        let huge = world(Topology::Torus, 65_536, 65_536);
        assert!(EngineKind::Dense.check(&huge, life).is_err());
        assert!(EngineKind::Bit.check(&huge, life).is_ok());
    }

    #[test]
    fn automatic_selection() {
        let input = |world, generations_per_tick, population, regularity| SelectionInput {
            world,
            rule: LifeRule::conway(),
            population,
            generations_per_tick,
            regularity,
        };
        let small = world(Topology::Torus, 100, 100);
        let large = world(Topology::Bounded, 2048, 2048);
        let inf = world(Topology::Infinite, 256, 256);
        assert_eq!(select_engine(&input(small, 1, 0, None)).unwrap(), EngineKind::Dense);
        assert_eq!(select_engine(&input(large, 1, 0, None)).unwrap(), EngineKind::Bit);
        assert_eq!(select_engine(&input(inf, 1, 100, None)).unwrap(), EngineKind::Sparse);
        assert_eq!(select_engine(&input(inf, 1024, 100, None)).unwrap(), EngineKind::Hashlife);
        assert_eq!(select_engine(&input(inf, 1000, 100, None)).unwrap(), EngineKind::Sparse, "not a power of two");
        assert_eq!(select_engine(&input(inf, 1, 1 << 20, Some(0.9))).unwrap(), EngineKind::Hashlife);
        assert_eq!(select_engine(&input(inf, 1, 1 << 20, Some(0.1))).unwrap(), EngineKind::Sparse);
    }

    #[test]
    fn regularity_of_repeated_tiles() {
        // Four identical blocks in four tiles: 1 distinct of 4.
        let mut cells = Vec::new();
        for t in 0..4 {
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                cells.push((t * 64 + 10 + dx, 5 + dy));
            }
        }
        cells.sort_unstable_by_key(|&(x, y)| (y, x));
        assert!((tile_regularity(&cells) - 0.75).abs() < 1e-12);
        cells.push((64 * 7, 0));
        cells.sort_unstable_by_key(|&(x, y)| (y, x));
        assert!((tile_regularity(&cells) - 0.6).abs() < 1e-12);
        assert_eq!(tile_regularity(&[]), 0.0);
    }

    #[test]
    fn names_round_trip() {
        for kind in EngineKind::ALL {
            assert_eq!(EngineKind::parse(kind.as_str()), Some(kind));
            assert_eq!(EngineChoice::parse(kind.as_str()).and_then(EngineChoice::fixed), Some(kind));
        }
        assert_eq!(EngineChoice::parse("auto"), Some(EngineChoice::Auto));
        assert_eq!(EngineChoice::parse("gpu"), None);
    }
}
