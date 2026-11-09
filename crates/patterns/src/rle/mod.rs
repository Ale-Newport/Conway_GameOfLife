//! Golly extended RLE (`.rle`).
//!
//! ```text
//! #CXRLE Pos=-1,-1 Gen=40      (optional, only at the very top)
//! #N Glider                     name
//! #O Richard K. Guy             author
//! #C A comment                  comments (#c too)
//! #r B3/S23                     rule (if the header has none)
//! #P -1 -1 / #R -1 -1           top-left coordinates (Life32 / XLife)
//! x = 3, y = 3, rule = B3/S23   header (optional)
//! bo$2bo$3o!                    data
//! ```
//!
//! Data tokens: an optional run count followed by `b`/`.` (dead), `o` (state
//! 1), `A`..`X` (states 1-24), a prefix `p`..`y` plus `A`..`X` (states
//! 25-255: `pA` = 25, `yO` = 255), `$` (end of row) or `!` (end of pattern;
//! everything after it is ignored). Whitespace between tokens and line breaks
//! anywhere between tokens are allowed.
//!
//! Compatibility rules follow Golly's reader: whitespace between a count and
//! its tag and counts with a leading zero are errors; a `p`..`y` prefix not
//! followed by `A`..`X` is read as state 1 (Golly's leniency for
//! non-standard files); only the first rule seen (`#r` or header) is used;
//! `#CXRLE` is only interpreted before any other line.
//!
//! Cell positions are relative to the top-left of the encoded rectangle,
//! which is placed at, in order of precedence: `#CXRLE Pos`; Golly's
//! placement when the header rule has a bounded-grid suffix (centred on the
//! origin if the header size fits in the grid, else at the grid's top-left
//! corner, so the cells get the same coordinates as in Golly); `#P`/`#R`
//! coordinates given before the data; `(0, 0)`. Unlike Golly, the reader
//! keeps cells that fall outside a bounded grid; filter them with
//! [`BoundedGrid::contains`](crate::rule::BoundedGrid::contains) to get
//! Golly's result.

mod read;
mod write;

pub(crate) use read::is_header;
pub use read::{RleDocument, parse, parse_document};
pub use write::write;
