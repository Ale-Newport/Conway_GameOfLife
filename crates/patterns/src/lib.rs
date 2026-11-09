//! # patterns
//!
//! Notation, pattern model and file formats for two-dimensional cellular
//! automata, shared by the Life engines, Hashlife, the OTCA metapixel, the
//! command-line tool and the desktop pattern browser.
//!
//! * [`rule`] — Life-like rules ([`LifeRule`], `B3/S23`, `23/3`, `b3s23`...),
//!   Generations rules ([`GenerationsRule`], `/2/3`, `B2/S/C3`), well-known
//!   rule names and Golly's bounded-grid suffixes ([`BoundedGrid`], `:T100,100`).
//! * [`pattern`] — the [`Pattern`] / [`Cell`] model with bounding boxes,
//!   normalisation, rotations, reflections, translation and merging.
//! * File formats: [`rle`] (Golly extended RLE, multi-state), [`plaintext`]
//!   (`.cells`), [`life105`] and [`life106`], plus format detection with
//!   [`parse_any`] and [`read_pattern_file`]. Every reader reports errors
//!   with line and column.
//! * [`catalog`] — loader for `assets/patterns/catalog.toml` and an embedded
//!   library of classic patterns ([`catalog::builtin_patterns`]) so tools keep
//!   working without the assets folder.
//!
//! Coordinates follow the workspace convention: `x` grows to the right and
//! `y` grows downward, exactly as rows are written in RLE files.

pub mod catalog;
pub mod error;
pub mod format;
pub mod life105;
pub mod life106;
pub mod pattern;
pub mod plaintext;
pub mod rle;
pub mod rule;
mod text;

pub use catalog::{BuiltinPattern, Catalog, CatalogEntry, builtin_patterns};
pub use error::PatternError;
pub use format::{Format, detect_format, parse_any, read_pattern_file, write_format};
pub use pattern::{Cell, MAX_CELLS, MAX_DENSE_AREA, Pattern};
pub use rule::{BoundedGrid, GenerationsRule, GridKind, LifeRule, Rule, RuleError};
