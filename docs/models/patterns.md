# patterns: rule notation, pattern model, file formats and catalog

`crates/patterns` is a foundation crate. It is not a simulation model, so it
does not implement `Simulation` and has no parameters, metrics or random
state. It provides what the Life engines (`cellular-automata`), `hashlife`
(through plain cell lists), `metapixel`, the command-line tool and the
desktop pattern browser need to read, write and describe two-dimensional
cellular-automaton patterns:

* **rule notation**: two-state Life-like rules, multi-state Generations rules,
  well-known rule names and Golly's bounded-grid suffixes;
* the **pattern model**: `Pattern` and `Cell`, with bounding boxes,
  normalisation, rotations, reflections, translation, merging and dense
  conversions;
* **file formats**: Golly extended RLE, plaintext (`.cells`), Life 1.05 and
  Life 1.06, with format detection;
* the **catalog**: a loader for `assets/patterns/catalog.toml` and an embedded
  library of 18 classic patterns that works without the assets folder.

Dependencies: `simulation-core` (for `Topology`, `Grid2`, `SimError`),
`serde`, `toml` and `thiserror`, all workspace dependencies. `serde_json` is a
dev-dependency. No third-party crate was added. The crate does no I/O except in
`Catalog::load` and `Catalog::load_pattern`, which read files.

Coordinates follow the workspace convention: `x` grows to the right, `y`
grows downward, the same as RLE rows. Patterns use `i64` coordinates.

---

## 1. Modules and public API

| Module | Contents |
|---|---|
| `rule` | `LifeRule`, `GenerationsRule`, `Rule` (either kind), `BoundedGrid`, `GridKind`, `RuleError`, `MAX_GRID_SIZE` |
| `pattern` | `Cell`, `Pattern`, `MAX_CELLS`, `MAX_DENSE_AREA` |
| `rle` | `parse`, `parse_document` → `RleDocument`, `write` |
| `plaintext`, `life105`, `life106` | `parse`, `write` |
| `format` | `Format`, `detect_format`, `parse_any`, `read_pattern_file`, `write_format` |
| `catalog` | `Catalog`, `CatalogEntry`, `BuiltinPattern`, `builtin_patterns` |
| `error` | `PatternError` (converts into `simulation_core::SimError`) |

The most common items are re-exported at the crate root.

---

## 2. Rule notation (`rule`)

### 2.1 Life-like rules

`LifeRule { birth: u16, survive: u16 }`. Bit *n* of `birth` means that a dead
cell with *n* live Moore neighbours is born. Bit *n* of `survive` means that a
live cell with *n* live neighbours survives. Every other cell is dead in the
next generation. Only bits 0..=8 have meaning. `next_state(alive, n)`,
`births(n)` and `survives(n)` evaluate the rule. `has_b0()` reports rules in
which empty space gives birth; sparse engines and Hashlife cannot run those
directly.

`LifeRule::parse` ignores case and whitespace. It accepts:

| Input | Result |
|---|---|
| `B3/S23`, `b3s23`, `B3S23`, `S23/B3` | B3/S23 |
| `23/3` (MCell / Life 1.05 order: survival first) | B3/S23 |
| `B2/S`, `B234` (no survival part), `/3` | B2/S, B234/S, B3/S |
| `B3/23` (one labelled part; the other part is the missing one, as in Golly) | B3/S23 |
| `Life`, `HighLife`, `Day & Night`, `day and night`, `2x2`, ... | the named rule |
| `B3/S23:T100,100` | B3/S23; the suffix is checked and dropped (use `parse_with_grid` to keep it) |

`Display` gives the canonical form: `B`, the birth digits in ascending order,
`/S`, then the survival digits (for example `B36/S23`). Errors are
`RuleError { message, column }`, where `column` is the 1-based character
position. Some examples:

* `B39/S23`: "digit 9 is not a valid neighbour count (0-8) (at column 3)".
* `B3/B6`: "'B' appears more than once".
* `2/3/4`: "a rule with three '/'-separated parts is a Generations rule (S/B/C)".
* `B2e/S`: isotropic letters are rejected with "non-totalistic (isotropic)
  rule notation is not supported".
* `H` or `V` suffixes, `MAP…`, `W…` (Wolfram) and `R…` (Larger than Life)
  rules are rejected with their own messages.
* `Brian's Brain` gives "is a Generations rule".
* `LifeHistory` gives "unknown rule name".

Named rules (`name()` and `from_name()`), following the LifeWiki list of
Life-like cellular automata: Life B3/S23, HighLife B36/S23, Seeds B2/S,
Day & Night B3678/S34678, Life without Death B3/S012345678, Maze B3/S12345,
Mazectric B3/S1234, 2x2 B36/S125, Replicator B1357/S1357, Fredkin
B1357/S02468, Diamoeba B35678/S5678, Morley B368/S245, Anneal B4678/S35678,
Coral B3/S45678, Gnarl B1/S1, Long Life B345/S5, Stains B3678/S235678,
Assimilation B345/S4567, Amoeba B357/S1358, Pseudo Life B357/S238, Serviettes
B234/S, Live Free or Die B2/S0, DotLife B3/S023, Coagulations B378/S235678,
Walled Cities B45678/S2345, Flock B3/S12, Vote B5678/S45678.

### 2.2 Generations rules

`GenerationsRule { birth, survive, states }` follows Golly's
`generationsalgo`. State 0 is dead, state 1 is alive, and states 2..C−1 are
dying. Only state-1 cells count as neighbours. With *n* live neighbours:

* 0 → 1 if *n* ∈ B, otherwise it stays 0;
* 1 → 1 if *n* ∈ S, otherwise 2, or 0 when C = 2;
* *k* ≥ 2 → *k*+1, wrapping to 0 when *k*+1 = C.

`next_state(state, n)` implements exactly this. The parser accepts these
forms:

* Golly/MCell `S/B/C`: `345/2/4`, `/2/3`;
* LifeWiki/LifeViewer: `B2/S/C3`, `B2/S345/C4` (also with `G` instead of `C`);
* Golly's mixed form `B2/S/3`;
* Catagolue: `g3b2s`;
* names: Brian's Brain, Star Wars, Banners, Bloomerang, Caterpillars,
  Cooties, Fireworks, Frogs, Lava, Lines, Sticks, Transers, Xtasy (taken from
  Golly's Generations help page).

`Display` gives Golly's canonical `S/B/C` form, for example `345/2/4`, which is
what Golly writes into RLE headers. `to_bsc_string()` gives the LifeWiki form
`B2/S345/C4`. `Rule::parse` chooses the family from the shape of the input
(three parts, or a `C`/`G` label at the start of a part). A Generations rule
with C = 2 is returned as `Rule::Life`.

### 2.3 Bounded grids

`BoundedGrid { kind, width, height, horizontal_twist, vertical_twist,
horizontal_shift, vertical_shift }` parses Golly's suffix, following
`lifealgo::setgridsize` and `Help/bounded.html`:

```text
suffix := ":" [ kind [ width ["*"] [shift] [ "," height ["*"] [shift] ] ] ]
kind   := P | T | K | C | S          shift := ("+" | "-") digits
```

* If the height is missing it equals the width. A size of 0 means infinite;
  this is allowed only for P and T (a tube or a half-plane).
* `:`, `:T` and any suffix with both sizes 0 describe the unbounded plane, and
  parsing them gives `None`.
* A twist (`*`) is allowed only on a Klein bottle. `:K10,20` means `:K10,20*`.
* A shift is allowed only on a torus, or on the twisted edge of a Klein
  bottle. Golly's normalisation is applied: shifts reduce modulo the edge
  length as soon as they are read (so `:T30+30,20+5` is `:T30,20+5`; only
  one pair of edges may have a non-zero shift), and a Klein bottle shift
  becomes 1 if that edge has an even length and 0 otherwise.
* A sphere is written `:S30` only.
* Sizes are limited to 2,000,000,000. Golly clamps larger values silently;
  this crate reports an error instead.

`Display` reproduces Golly's `canonicalsuffix`, for example `:T30+5,20`,
`:K30*+1,20` or `:S30`. `topology()` maps a grid to `simulation_core::Topology`
when an equivalent exists:

| Golly | Core |
|---|---|
| plane `P w,h` | `Bounded` |
| unshifted torus `T w,h` | `Torus` |
| Klein bottle with twisted horizontal (top/bottom) edges, unshifted | `KleinBottle` |
| cross-surface `C w,h` | `CrossSurface` |

Tubes, half-planes, spheres, shifted tori and Klein bottles with twisted
vertical edges have no core equivalent and map to `None`. Golly places the
top-left grid cell at `(-(w/2), -(h/2))`. `top_left()` returns that offset, so
cell coordinates read from a file (which the RLE reader places exactly as
Golly does, see §4.1) become grid indices by subtracting it.
`contains(x, y)` tells whether a cell in those coordinates lies inside the
grid (an infinite dimension contains everything); Golly drops the cells of a
file that fall outside, and filtering with `contains` does the same.

---

## 3. Pattern model (`pattern`)

`Pattern { name, description, author, rule: Option<String>, comments,
cells: Vec<Cell> }`, where `Cell { x: i64, y: i64, state: u8 }`.

* **Cells.** The list holds the non-dead cells. Readers produce them in
  row-major order (sorted by `y`, then `x`) with no duplicates. Cells in state
  0 are ignored by every query.
* **Rule.** The rule is kept exactly as written in the file, because Golly
  files also name rule-table rules such as `LifeHistory` or `WireWorld`.
  Use `Rule::parse_with_grid` to interpret it.
* **Description.** No file format has a description field. Writers emit the
  description as the first comment lines, and readers return those lines in
  `comments`. The catalog fills `description` from `catalog.toml`.

Queries: `bounding_box()` returns the inclusive `(min_x, min_y, max_x, max_y)`
or `None`. `width()`, `height()` (both `u64`, saturating), `population()`,
`max_state()`, `is_empty()` and `live_cells()` work as named.

Transforms all return a new pattern:

* `normalized()` moves the top-left of the bounding box to (0,0), sorts the
  cells by `y` then `x`, merges duplicate positions (the later entry wins) and
  drops state-0 cells.
* The rotations and reflections keep the top-left corner of the bounding box
  fixed, so a pattern turns in place. The table below gives the formulas, where
  (x₀,y₀)–(x₁,y₁) is the bounding box.
* `translated(dx, dy)` moves every cell.
* `merged(other, dx, dy)` combines the two patterns. Where both have a cell,
  `other`'s state wins, like Golly's copy-mode paste. The result is sorted and
  has no duplicates.

| Transform | x′ | y′ |
|---|---|---|
| `rotate_cw` | x₀ + (y₁ − y) | y₀ + (x − x₀) |
| `rotate_ccw` | x₀ + (y − y₀) | y₀ + (x₁ − x) |
| `rotate_180` | x₀ + x₁ − x | y₀ + y₁ − y |
| `flip_horizontal` | x₀ + x₁ − x | y |
| `flip_vertical` | x | y₀ + y₁ − y |

The transforms use 128-bit intermediate values. Results outside the `i64`
range saturate at the limit.

Dense helpers:

* `to_ascii()` draws `.`/`O`/`2`–`9`/`#`, for debugging.
* `from_ascii()` reads `.`, `O`/`o`/`*` and the digits `1`–`9`. It trims lines,
  so indented string literals work in tests.
* `to_grid()` returns a `Grid2<u8>` of the bounding box. `from_grid()` does
  the reverse.
* `from_live_cells()` builds a pattern from coordinate lists, and
  `live_cells()` returns them. These are the cell lists exchanged with
  Hashlife.

---

## 4. File formats

All readers accept `\n`, `\r\n` and lone `\r` line endings and skip a UTF-8 byte
order mark. Errors are `PatternError::Syntax { format, line, column, message }`,
with a 1-based line and character column.

### 4.1 RLE (`rle`)

This reader is an independent implementation written from Golly's
`readpattern.cpp`, `writepattern.cpp` and `Help/formats.html`. No Golly code
was copied.

* **Comment lines.**
  * `#N` is the name and `#O` the author. Several of either are joined with
    newlines.
  * `#C`, `#c` and unknown tags such as `#D` are comments. One separating
    space is removed from each.
  * `#r` gives the rule.
  * `#P`, `#p` and `#R` followed by two integers, before the data, give the
    top-left coordinates. Otherwise the line is kept as a comment.
  * `#CXRLE Pos=x,y Gen=n` is read only before any other line, as in Golly.
* **Header.** `x = W, y = H, rule = R`, with its keys in any order and any
  spacing; it may also start with `y`. The rule value runs up to the next
  whitespace, because rules such as `R2,C0,S4-7,…` or `B3/S23:T10,10`
  contain commas. Unknown keys are ignored. The header is optional. The
  first rule seen, from `#r` or the header, is used.
* **Data.**
  * A run count followed by one of these tags:
    * `b` or `.` for dead;
    * `o` for state 1;
    * `A`..`X` for states 1–24;
    * `p`..`y` followed by `A`..`X` for states 25–255 (24·(prefix−`p`+1) + letter);
    * `$` for the end of a row;
    * `!` for the end of the pattern. Everything after it is ignored.
  * Whitespace and line breaks may appear between tokens.
  * These are errors, as in Golly: whitespace between a count and its tag,
    a count with a leading zero, and a count left open at the end of a line.
  * A `p`..`y` prefix that is not followed by a letter reads as state 1.
    Golly does the same for M. Niemiec's non-standard files.
  * Any other character is an error, reported with its position.
* **Positions.** Cells are placed relative to the top-left corner of the
  encoded rectangle, and that corner is placed at, in order of precedence:
  1. `#CXRLE Pos=x,y`;
  2. Golly's placement when the rule known at the header line (from the
     header or an earlier `#r`) has a bounded-grid suffix: the rectangle is
     centred on the origin, at `(-(x/2), -(y/2))` from the header size, when
     that size fits in the grid, and starts at the grid's top-left corner
     otherwise (`readrle` in Golly);
  3. `#P`/`#R` coordinates given before the data (XLife and Life32 write
     them at the top; such lines after the data has started are comments);
  4. `(0, 0)`.

  With this rule the 417 files of Golly's collection that a Golly-based
  driver could load give exactly the cells Golly loads, at the same
  coordinates (differential check, §8). The cursor is tracked with 128-bit integers, so a
  rectangle spanning the whole `i64` range (`x = 18446744073709551616`) is
  read; a cell that would fall outside `i64` is an error. A header size
  larger than `u64::MAX` is clamped, since Golly treats the size as
  informational. `parse_document` also returns the header size, `Gen` and the
  position.
* **Writer** (follows Golly's `writerle`):
  * The first line is `#CXRLE Pos=x,y`, written whenever a reader would
    otherwise place the bounding box elsewhere: at (0,0), or at Golly's
    position when the rule has a bounded-grid suffix. Then come `#N`, `#O`
    and `#C` (description, then comments), and the header.
  * Metadata values are split into lines at `\n`, `\r\n` and a lone `\r`,
    the terminators the readers accept, so no text can spill onto a line of
    its own and be read back as pattern data. The plaintext and Life 1.05
    writers do the same.
  * Data lines are at most 70 characters. Golly's break rule is used,
    including the extra column it reserves for the second letter of a
    multi-state token.
  * A run of 1 is written without a count, trailing dead cells are dropped,
    and blank rows are folded into `n$`.
  * The letter form is used when a cell has a state above 1 or the rule has
    more than two states.
  * Rules this crate can parse are written in canonical form, as Golly does.
    Other rules are written without whitespace and trailing commas (a header
    rule value ends at the first whitespace, and readers drop a final comma),
    and a blank rule is left out, so the header always reads back.
  * An empty pattern is written as `x = 0, y = 0[, rule = R]` followed by `!`.

### 4.2 Plaintext (`plaintext`)

* Lines starting with `!` are comments. `!Name:` and `!Author:` (in any case)
  set the name and author.
* In rows, `.` is dead, and `O` is alive; `o` and `*` are also read as alive.
* Trailing dead cells may be left out. Blank lines after the first row are
  empty rows.
* The writer pads every row to the full width.
* The format is two-state only, so writing a multi-state pattern returns
  `PatternError::Unsupported`.

### 4.3 Life 1.05 and 1.06

* **Life 1.05** (`life105`).
  * `#D` and `#C` lines are comments. `#N` means Conway's rules, and `#R`
    gives a rule.
  * `#P x y` starts a block of `.`/`*` rows. A `#P` with no coordinates
    continues below the previous block, as in Golly. The `#Life 1.05` header
    is optional.
  * The writer emits `#D Name:` and `#D Author:` lines, `#N` (Conway's Life
    on the unbounded plane) or `#R` in S/B order with any bounded-grid suffix
    (`#R 23/3:T10,10`, which Golly reads), then blocks at most 80 columns
    wide.
* **Life 1.06** (`life106`).
  * One `x y` pair per line. Golly's extra `#D`, `#N` and `#R` lines are also
    read. Duplicate cells are merged.
  * The writer produces the strict format: the header and the coordinates.
    Name, comments and rule are therefore not stored.

### 4.4 Detection (`detect_format`, `parse_any`)

The content is checked first, in Golly's `loadpattern` order:

1. `#Life 1.06` selects Life 1.06.
2. `#Life 1.05` or `#P ` selects Life 1.05.
3. `#MCell` and `[M2]` (macrocell) are reported as unsupported. Macrocell
   files belong to the `hashlife` crate.
4. A first character `#` or `x`, or a header line that starts with `y =`,
   selects RLE.
5. `!` selects plaintext.

Content without a signature (bare rows or headerless RLE) is decided by the
file extension: `.rle`, `.cells`, or `.lif`/`.life`, where coordinates mean
1.06 and rows mean 1.05. Without a known extension, Golly's headerless-RLE test
is used (only counts, `b`, `o`, `$` and an optional final `!`, with some digit
or the `!`), and anything else is plaintext. `.gz`, `.zip`, `.mc` and `.mcl`
files are reported as unsupported.

`read_pattern_file(path)` reads a file from disk and calls `parse_any` with its
file name. Bytes that are not valid UTF-8 (Latin-1 accents in the comments of
old files) are replaced with U+FFFD instead of failing, since pattern data is
ASCII and Golly reads bytes. The catalog loads its files this way.

---

## 5. Catalog (`catalog`)

`Catalog::load(path)` accepts the catalog file, or the directory that
contains `catalog.toml`.

* **Entries.** The entries are an array named `[[pattern]]` or `[[patterns]]`.
  Only `id`, `name` and `file` are required. The other known fields are
  `category`, `kind`, `description`, `discoverer`, `year`, `period`,
  `dx`/`dy` or `displacement = [dx, dy]`, `width`, `height`, `population`,
  `expected_behavior`, `complexity`, `source`, `license` and `rule`.
* **Unknown fields.** Fields such as `speed`, `verified` or `stabilizes_at`
  are kept as text in `CatalogEntry::extra`.
* **Types.** Integer fields also accept numeric strings, and text fields
  accept numbers and booleans. Any other type is an error that names the entry
  and the field.
* **Library table.** An optional `[library]` table supplies `name()` and a
  default `rule`.
* **Validation.**
  * Ids must be unique.
  * `file` must be a relative path without `..`, so a catalog cannot read
    files outside its directory.
  * `dx`/`dy` must agree with `displacement` when both are given.
  * TOML syntax errors are reported with their line and column.

Other methods:

* `entries()` and `by_id()` look up entries.
* `categories()` lists categories in order of first appearance, and
  `in_category()` lists the entries of one category.
* `load_pattern(&entry)` reads the file with `read_pattern_file`, then fills any
  missing name, description, author (from the discoverer) or rule (from the
  entry, then the library default).

**Embedded library.** `builtin_patterns()` and `Catalog::builtin()` hold 18
classic patterns that were transcribed by hand: block, beehive, loaf, boat,
tub, blinker, toad, beacon, pulsar, pentadecathlon, glider, LWSS, MWSS, HWSS,
Gosper glider gun, R-pentomino, diehard and acorn.

* Their ids, categories and kinds match the asset catalog. The UI can
  therefore fall back to them without changes, for example
  `Catalog::load(p).unwrap_or_else(|_| Catalog::builtin())`; the caller should
  report the error.
* Each RLE string is stored in the exact form the writer produces.
* The asset catalog uses period 0 for still lifes and methuselahs, and this
  library follows that convention.

---

## 6. Limits, approximations and limitations

These are deliberate and explicit.

* **Rules.** Only outer-totalistic rules on the Moore neighbourhood are
  supported.
  * Isotropic non-totalistic (Hensel) letters, the hexagonal (`H`) and von
    Neumann (`V`) neighbourhoods, `MAP` rules, Wolfram `W` rules,
    Larger-than-Life and rule-table names are rejected by the rule parsers.
    The pattern readers still keep such rule strings verbatim.
  * Generations rules are limited to 255 states because `states` is a `u8`;
    Golly allows 256.
  * Generations rules with B0 are rejected, as in Golly.
* **States.** Cell states are 0–255. An RLE state token above 255 (for
  example `yX` = 264) is an error.
* **Resource limits.**
  * A reader produces at most `MAX_CELLS` = 2²⁵ (about 33.5 million) live
    cells; beyond that it returns `PatternError::LimitExceeded`. This stops a
    tiny file with a huge run count from exhausting memory. A `Cell` takes
    24 bytes, so a pattern at the limit still holds about 800 MB. The limit
    is a guard against runaway input, not a memory budget for small
    machines.
  * Long dead runs and blank rows cost nothing.
  * The dense writers (plaintext, Life 1.05), `to_grid()` and `to_ascii()`
    refuse bounding boxes larger than `MAX_DENSE_AREA` = 2²⁸ cells.
    `to_ascii()` returns a one-line summary in that case.
* **Coordinates.**
  * Readers report a `pattern coordinates overflow` error instead of wrapping
    `i64`.
  * Transforms saturate at the limits of `i64`.
* **Bounded grids.** The RLE reader places a pattern in a bounded grid as
  Golly does, but it keeps the cells that fall outside the grid (Golly drops
  them); `BoundedGrid::contains` filters them. The Life 1.0x and plaintext
  readers do not interpret bounded grids, as in Golly.
* **Metadata.**
  * `#CXRLE Gen` is returned by `rle::parse_document` but is not stored in
    `Pattern`.
  * Life 1.06 output keeps cells only.
  * Plaintext and Life 1.05 write a multi-line name or author (and a Life
    1.05 rule this crate cannot parse) on one line.
  * The rule written in RLE is canonicalised when this crate can parse it,
    for example `b3s23` becomes `B3/S23`.
* **Unsupported formats.** Macrocell, MCell, compressed files, dblife, `.rle3`
  and Golly's "text pattern" quirk (`$` meaning 10 dead cells in bare text)
  are not handled. Bare text is read as strict plaintext.

---

## 7. References

* Golly 5.1 sources, read for behaviour only; no code was copied:
  * `gollybase/readpattern.cpp`: `readrle`, `readpclife`, `loadpattern`,
    `isplainrle`;
  * `gollybase/writepattern.cpp`: `writerle`, `AddRun`;
  * `gollybase/lifealgo.cpp`: `setgridsize`, `canonicalsuffix`;
  * `gollybase/liferules.cpp` and `gollybase/generationsalgo.cpp`: rule
    grammars and canonical forms;
  * `Help/formats.html`, `Help/bounded.html` and
    `Help/Algorithms/Generations.html`.
* LifeWiki pages on Run Length Encoded, Plaintext, Life 1.05 and Life 1.06,
  and the list of Life-like cellular automata (as summarised in
  `docs/research/life-engines-hashlife.md` §1 and §6).
* `docs/research/pattern-library.md`, which describes the asset library and
  catalog schema this loader reads.

---

## 8. How to test

```sh
CARGO_TARGET_DIR=target/agents/patterns cargo test -p patterns
CARGO_TARGET_DIR=target/agents/patterns cargo clippy -p patterns --all-targets -- -D warnings
# Real-world corpus (Golly's GPL pattern collection is read at run time, never copied):
GOLLY_DIR=/path/to/golly CARGO_TARGET_DIR=target/agents/patterns cargo test -p patterns --test golly_files -- --ignored
```

The tests check behaviour, not only that the code runs.

* **Unit tests** (in each module):
  * tables of rule strings with their canonical forms, and error messages with
    columns;
  * every example from Golly's bounded-grid help and its canonical form, plus
    all of Golly's error cases;
  * the Generations transition rule;
  * exact RLE output (header, `#CXRLE`, blank-row folding, multi-state
    letters, the 70-character limit);
  * RLE error cases with their line and column;
  * RLE placement: Golly's bounded-grid placement and its precedence,
    position lines after the data, rectangles spanning all of `i64`, and the
    writer's `#CXRLE Pos` decision;
  * line breaks inside names and comments never reaching the data, in all
    writers;
  * plaintext and Life 1.05/1.06 round trips;
  * transforms, including that four rotations give the identity, that
    rotations happen in place, and composition rules;
  * catalog parsing, unknown fields, malformed catalogs and path escapes;
  * format detection.
* **`tests/builtin_dynamics.rs`**: an independent sparse Life simulator, built
  on `LifeRule::next_state`, verifies every embedded pattern.
  * Still lifes do not change.
  * Each oscillator has its exact period, with no shorter one.
  * Each spaceship has its displacement per period.
  * The Gosper gun restores itself and emits one glider every 30 generations
    (populations 41, 46, 51 and 56).
  * Diehard dies at generation 130, the R-pentomino has 116 cells at
    generation 1103, and the acorn has 633 cells at generation 5206.
* **`tests/asset_catalog.rs`** checks the workspace library.
  * All 80 catalog entries load, and their population and size match the
    files.
  * Every RLE under `assets/patterns`, including the OTCA metapixel files,
    parses and survives a round trip.
  * The embedded patterns are the same objects as the asset versions, up to
    symmetry.
  * The test is skipped when the assets are absent.
* **`tests/properties.rs`**: seeded randomised cross-checks.
  * Names, authors, descriptions, comments and rules made of random text
    (line breaks, `#`, `!`, `$`, commas, digits...) never stop any writer's
    output from parsing back to the same cells.
  * Random birth/survival masks written in every notation (`B…/S…`,
    `b…s…`, `S…/B…`, `S/B`, `S/B/C`, `B/S/C`, `g…b…s…`) with shuffled digits
    parse back to those masks, and `next_state` agrees with them.
  * Random two-state and 255-state patterns survive every writer and reader,
    including `parse_any` with the format's extension, at their original
    position.
  * Rotations and reflections agree with an independent dense-grid
    reference and keep the top-left corner fixed.
  * Random hostile text never makes a reader or rule parser panic, and
    numbers at the `i64`/`u64` limits either fail cleanly or round-trip.
* **`tests/golly_files.rs`** (`#[ignore]`, needs `GOLLY_DIR`): all 482
  `.rle`/`.lif` files of Golly's `Patterns` folder parse and survive an RLE
  round trip (3.78 million cells in total).
  * The `Patterns/Life` rules and bounded-grid suffixes parse.
  * Every bounded-grid RLE whose header size fits its grid has all its cells
    inside the grid (Golly's placement; 23 files).
  * The populations of `acorn.lif` (7) and `rabbits.lif` (9) are checked.
  * `breeder.lif` is checked for `#N`, and the Klein-bottle and shifted-torus
    examples are checked.
  * Generations rules are checked against the states used in their files.

**Differential check against Golly** (done during review, not part of the
test suite because it links Golly's GPL C++ sources): a small driver built
from Golly 5.1's `gollybase` loaded every file of the `Patterns` folder with
`readpattern` and printed each non-zero cell as `x y state`. For all 417
files Golly could load without external rule tables (the others use
`LifeHistory` or `.rule` files), `parse_any` produced exactly the same set of
`(x, y, state)` triples.
