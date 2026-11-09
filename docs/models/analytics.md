# analytics and persistence: measurement, analysis and files

This page covers two foundation crates. Neither is a simulation model, so
neither implements `Simulation` or has parameters or metrics of its own.

* `crates/analytics` turns the `Metrics` that every model publishes into time
  series, statistics, ecological indices, survival curves, run records,
  multi-run comparisons and CSV/JSON exports.
* `crates/persistence` gives every saved file a versioned envelope. It
  encodes files as RON, JSON, TOML or the binary `.elsnap` container, upgrades
  old files through schema migrations, stores snapshots and snapshot
  histories, writes files atomically and finds the per-user data folder.

Both depend on `simulation-core`, `serde`, `serde_json` and `thiserror`.
`persistence` also uses `ron`, `toml` and `postcard`. All of these are
workspace dependencies. The only extra is the `float_roundtrip` feature of
`serde_json` in `analytics`, explained in §2.7. Neither crate reads the
clock or a random source. Timestamps (`started_at`, `created`) are supplied
by the caller, so the outputs are pure functions of their inputs.

---

## 1. analytics: modules

| Module | Contents |
|---|---|
| `series` | `Point`, `Series`, `StoreConfig`; `TimeSeriesStore` (in `series/store.rs`) |
| `downsample` | `min_max`, `m4`, `lttb`, `downsample(points, n, DownsampleMethod)` |
| `stats` | `RunningStats` (Welford), `summarize` → `Summary`, `mean`, `variance`, `sample_variance`, `std_dev`, `min_max`, `quantile(s)`, `median`, `pearson`, `spearman`, `linear_regression` → `LinearFit` |
| `histogram` | `Histogram::{with_range, fixed, auto}`, `BinRule` |
| `diversity` | `Diversity` (richness, Shannon, Simpson, inverse Simpson, Pielou), `gini`, `lorenz_curve` |
| `survival` | `Lifespan`, `SurvivalCurve::kaplan_meier`, `SurvivalStep` |
| `dynamics` | `exponential_growth` → `GrowthFit`, `growth_rates`, `autocorrelation` (direct or FFT), `estimate_period`, `estimate_series_period`, `extinction_tick`, `extinctions` |
| `run` | `RunRecord`, `MetricStats`, `compare_runs` → `ComparisonTable`, `compare_runs_with` → `Result<ComparisonTable>`, `ComparisonTable::validate`, `align_series`, `align_store`, `align_runs` → `AlignedSeries` |
| `export` | `store_to_csv` / `write_store_csv`, `run_to_csv` / `write_run_csv`, `AlignedSeries::to_csv`, `ComparisonTable::to_csv`, `escape_csv_field`, `format_number`, versioned JSON `*_to_json` / `*_from_json` |

## 2. analytics: algorithms

### 2.1 Time series store

`TimeSeriesStore::record(&Metrics)` appends every value to the `Series` of
its key. Each series is created the first time its key appears, and
`keys()` returns the keys in that first-seen order, which is also the CSV
column order. The store uses a `BTreeMap` index for lookups and never
iterates over a hash map.

Rules applied by `record`:

* **Ticks.** A measurement whose tick is earlier than the last recorded tick
  is rejected with `TickOutOfOrder`. The store is left unchanged, because the
  tick is checked before any series is modified. A measurement for the *same*
  tick replaces that tick's values. This lets a paused simulation that was
  edited re-publish its metrics. To rewind after restoring an earlier
  snapshot, call `truncate_after(tick)` first.
* **Non-finite values.** A NaN or infinite value is not stored. It is counted
  in `skipped_non_finite()`, and the series has no sample at that tick, which
  shows up as an empty CSV cell. When the measurement replaces the most
  recent tick, a non-finite value also *removes* the key's earlier sample at
  that tick (otherwise a stale value would survive). Keys absent from a
  replacing measurement keep their samples, so values added with
  `record_value` are not lost. `Series::push` and `record_value` return an
  error for non-finite values instead of skipping them.

`Series::window(start, end)` returns the samples with
`start ≤ tick ≤ end` as a slice, found by binary search.
`TimeSeriesStore::window` builds a new store restricted to that range, which
is useful for exporting part of a run.

### 2.2 Downsampling for plots

All series keep their samples at full resolution. For plots, every series
also keeps an **incremental M4 summary** (Jugel et al., 2014). M4 is defined
on the time axis: bucket `k` holds the samples with ticks in
`[t₀ + k·w, t₀ + (k+1)·w)`, where `t₀` is the tick of the first sample and
the width `w` is a power of two. Each bucket remembers its first, minimum,
maximum and last sample. When a sample falls beyond bucket
`display_points / 4 − 1`, `w` doubles and neighbouring buckets are merged
pairwise (bucket `k` becomes `⌊k/2⌋`). Hence:

* a push costs O(1) amortised (a merge pass happens only when the time span
  doubles);
* `Series::display()` returns at most `display_points` points in time
  proportional to `display_points`, however long the series is;
* the first and last samples and the global minimum and maximum are always
  among the displayed points, because each bucket keeps its own extremes and
  endpoints;
* irregular sampling (metrics every few ticks, gaps, rewinds, compacted
  data) keeps a correct time axis, because buckets cover equal tick spans,
  not equal sample counts;
* while a series has at most `display_points` samples (default 2048),
  `display()` returns every sample, i.e. full resolution.

For decimation on demand, for example over a zoomed window
(`Series::downsample_window`), three algorithms are available. `min_max` and
`m4` split `[first tick, last tick]` into equal-width buckets (accepting
input in any order without exceeding the budget); `lttb` uses equal-count
buckets as in the original algorithm.

| Method | Keeps per bucket | Guarantees |
|---|---|---|
| `min_max` | minimum and maximum | global extremes, ≤ `n` points |
| `m4` | first, min, max, last | global extremes, endpoints, ≤ `n` points; pixel-exact lines with one bucket per pixel column |
| `lttb` | the point that maximises the triangle area with the previously selected point and the next bucket's mean (Steinarsson, 2013) | exactly `n` points, endpoints. Extremes are **not** guaranteed |

**Optional memory cap.** If `StoreConfig::max_raw_points = Some(cap)`
(which must be ≥ 2 × `display_points`), a series that grows beyond `cap`
samples is replaced by its M4 summary with at most `cap / 2` points, using
the same power-of-two grid anchored at the first sample. Because M4 of whole
buckets composes (the M4 of merged buckets equals the M4 of their M4
points) and successive grids nest, the stored samples right after every
compaction are *exactly* the M4 summary of the original full-resolution
data: the first and the latest sample, every bucket's extremes and an even
time resolution over the whole run are kept, however many compactions
happen. Intermediate samples are lost. `Series::is_compacted()` then reports
`true`, and exports reflect the reduced resolution. By default there is no
cap: memory is 16 bytes per sample per metric.

### 2.3 Descriptive statistics

* **Welford's algorithm** (`RunningStats`): for each new value `x`, with
  `n ← n + 1`, `δ = x − x̄`, `x̄ ← x̄ + δ/n`, `M₂ ← M₂ + δ(x − x̄)`.
  Population variance is `M₂/n` and sample variance is `M₂/(n − 1)`. Two
  accumulators combine exactly (Chan, Golub & LeVeque, 1979):
  `M₂ = M₂ᵃ + M₂ᵇ + δ² nₐ n_b / n`. Welford stays accurate for data with a
  large offset, where the naive `Σx² − (Σx)²/n` formula cancels
  catastrophically. A test checks this with values near 10⁹.
* **Quantiles** use Hyndman & Fan type 7, the default in R and NumPy. With
  sorted values `x₀ … xₙ₋₁` and `h = (n − 1)q`,
  `Q(q) = x⌊h⌋ + (h − ⌊h⌋)(x⌊h⌋₊₁ − x⌊h⌋)`. Values are sorted with
  `f64::total_cmp`. Non-finite inputs are rejected with their position.
* **Correlation.** Pearson's `r = Sxy / √(Sxx Syy)` is computed from centred
  sums in two passes. Spearman's ρ is Pearson's r applied to the ranks, with
  tied values given the average of their ranks. Both are reported as
  `Undefined` when one variable is constant.
* **Regression.** `linear_regression` fits ordinary least squares and
  returns the slope, intercept and `r²`. It is defined as 1 when `y` is
  constant, because the line then fits exactly.

### 2.4 Histograms

The bins have equal width over `[lo, hi]`. Bin `i` is `[lo + iw, lo + (i+1)w)`,
except that a value equal to `hi` goes into the last bin (the NumPy
convention). Values outside the range are counted as underflow and overflow.
When all values are equal, the range becomes `[x − ½, x + ½]`.

Automatic bin rules for `n` values:

* Sturges: `k = ⌈log₂ n⌉ + 1`
* square root: `k = ⌈√n⌉`
* Scott: bin width `h = 3.49 σ n^(−1/3)`
* Freedman–Diaconis: bin width `h = 2 IQR n^(−1/3)` (this is the default)

For the width-based rules, `k = ⌈range / h⌉`. If the spread estimate is zero,
these rules fall back to Sturges. The number of bins is capped at
`MAX_AUTO_BINS = 1000`, so one extreme outlier cannot allocate millions of
empty bins. `density()` normalises the counts so they integrate to 1 over
the range.

Every histogram has between 1 and `MAX_BINS = 1 000 000` bins and a finite
range with a finite width; `with_range` and JSON import reject anything
else, so a damaged file cannot make `bin_of` index out of bounds. When all
values equal `x`, the range is `[x − d, x + d]` with `d = max(½, |x|·10⁻⁹)`,
so the bounds stay distinct for large `|x|`.

### 2.5 Diversity and inequality

The diversity indices take abundances `nᵢ ≥ 0` and use the proportions
`pᵢ = nᵢ/N`.

| Index | Formula | Notes |
|---|---|---|
| richness `S` | `#{i : nᵢ > 0}` | |
| Shannon `H'` | `−Σ pᵢ ln pᵢ` | natural log (nats) |
| Gini–Simpson | `1 − Σ pᵢ²` | probability that two individuals drawn *with replacement* belong to different categories. It uses proportions, not the unbiased `n(n−1)/N(N−1)` form |
| inverse Simpson | `1 / Σ pᵢ²` | effective number of equally common categories |
| Pielou `J` | `H' / ln S` | `None` when `S ≤ 1` |

An all-zero (extinct) community is defined to have richness 0 and zero
Shannon and Simpson indices. `Diversity::from_labels` counts one label per
individual (for example a species id per organism) in sorted order, so the
result does not depend on iteration order.

The **Gini coefficient** is `G = Σᵢ Σⱼ |xᵢ − xⱼ| / (2 n² x̄)`. It is computed
in O(n log n) from the sorted values as `G = Σᵢ (2i − n − 1)·x₍ᵢ₎ / (n Σx)`
(1-based ranks), which avoids the cancellation of the algebraically equal
`2 Σᵢ i·x₍ᵢ₎ / (n Σx) − (n+1)/n`. Equal amounts (including all zeros) return
exactly `G = 0`; one member holding everything gives `(n − 1)/n`.
`lorenz_curve` returns the cumulative shares, poorest first.

### 2.6 Survival, growth, periods and extinction

* **Kaplan–Meier.** For each distinct time `tᵢ` with `dᵢ` deaths among the
  `nᵢ` individuals still at risk, `S(t) = Π_{tᵢ ≤ t} (1 − dᵢ/nᵢ)`. The
  standard error comes from Greenwood's formula,
  `Var S = S² Σ dᵢ / (nᵢ(nᵢ − dᵢ))`, and is `None` once `S` reaches 0.
  Individuals censored at a death time count as at risk at that time. The
  median is the smallest `t` with `S(t) ≤ ½`. The table lists censoring-only
  times too, with zero deaths, so plots can mark them.
* **Exponential growth.** `exponential_growth` fits `ln N = ln N₀ + r t` by
  least squares over the samples with `N > 0`. From `r` it derives the
  finite rate `λ = eʳ`, the doubling time `ln 2 / r` and the halving time
  `ln 2 / −r`. `growth_rates` gives the rate over each interval,
  `ln(N₁/N₀)/(t₁ − t₀)`.
* **Oscillation period.** The sample autocorrelation is
  `r(k) = Σ (xₜ − x̄)(xₜ₊ₖ − x̄) / Σ (xₜ − x̄)²`, the biased estimator with
  `r(0) = 1`. The sums are computed directly when `n·(max_lag + 1)` is small
  and otherwise with a zero-padded radix-2 FFT (`|FFT(x)|²` followed by an
  inverse FFT, padding to at least `n + max_lag + 1` so the circular
  correlation equals the linear one). This makes the default lag range
  (`n/2` lags) O(n log n) instead of O(n²): 200 000 samples take
  milliseconds. The candidates are the local maxima of `r(k)` in
  `[min_lag, max_lag]` (by default `[2, n/2]`) with `r(k) ≥ min_strength`
  (default 0.3). The function returns the **smallest** candidate lag whose
  peak reaches `harmonic_tolerance` (default 0.85) times the strongest peak.
  Because the biased estimator decays with lag, the fundamental period is
  chosen over its multiples. Parabolic interpolation of the three ACF values
  around the peak gives a sub-sample `refined` period.
  `estimate_series_period` converts the period to ticks and requires samples
  that are evenly spaced in ticks. A constant series has no period.
  `min_strength` must lie in `[−1, 1]` and `harmonic_tolerance` in `(0, 1]`;
  other values (including NaN) are rejected.
* **Extinction.** `extinction_tick(points, threshold)` is the tick of the
  first sample in the *final* run of samples with `value ≤ threshold`. If the
  last sample is above the threshold, there is no extinction, so a temporary
  dip followed by recovery is not counted. A NaN threshold declares nothing
  extinct. `extinctions(store, threshold)` lists every extinct series of a
  store.

### 2.7 Run records, comparison and export

`RunRecord { run_id, model_id, scenario, seed, params: ParamSet,
started_at, ticks, series, summary: BTreeMap<String, f64>, events:
Vec<SimEvent> }`. `record` advances `ticks`, and `set_summary` rejects
non-finite numbers. `validate` checks that summary values are finite and
that `ticks` covers the series.

`compare_runs(&[RunRecord]) -> ComparisonTable` produces:

* `runs`: one `RunHeader` per run, in input order.
* `metrics`: one row per metric key, in order of first appearance over the
  runs. Each row has one `MetricStats` per run, or `None` if the run lacks
  the metric. `MetricStats` holds the sample count, first and final tick,
  final value, mean, population standard deviation, min and max (each with
  the first tick at which it was reached), and `extinction_tick`. Each row
  also has `final_across_runs`, a `Summary` of the final values over the
  runs, which is useful for replicate seeds.
* `summaries`: the summary keys in sorted order, with values per run and
  their summary across runs. Non-finite values are treated as missing.

`compare_runs_with` sets the extinction threshold (default 0) and returns an
error for a non-finite threshold. Extinction ticks are computed for every
metric but only mean something for populations and other gauges.
`ComparisonTable::validate` checks that every row has one entry per run and
that every number is finite; JSON export and import call it, because JSON
cannot represent NaN or infinity.

`align_series` (union or intersection of tick axes), `align_store` and
`align_runs` (one metric, one column per run) put series on a common tick
axis. Missing cells are `None`. Imported `AlignedSeries` documents are
validated (one column per label, one cell per tick, increasing ticks), and
`row` / `cell` read a missing cell of a hand-built value as `None` instead
of panicking.

**CSV** (RFC 4180 quoting):

* A field is quoted when it contains `,`, `"`, CR or LF, or when it starts
  or ends with a space. Embedded quotes are doubled.
* Lines end with `\n`.
* `store_to_csv` writes `tick` followed by one column per metric, in
  first-seen order, with one row per tick present in any series.
  `write_store_csv` and `write_run_csv` stream the same text to any
  `io::Write` (buffered internally) through a k-way merge of the series, so
  memory does not grow with the run length; I/O failures are returned as
  `AnalyticsError::Io`.
* Numbers are formatted by `format_number`. Integers below 10¹⁵ are written
  without a decimal point, moderate magnitudes in shortest round-trip
  decimal, and very large or very small magnitudes in shortest round-trip
  scientific notation. Parsing a cell therefore gives back exactly the
  stored `f64`.
* `run_to_csv` adds `# key: value` comment lines with the run identity and
  parameters before the table. Line breaks inside labels and values
  (including parameter keys) are replaced by spaces, so every comment line
  starts with `#`.
* `ComparisonTable::to_csv` is long format with a `source` column: one row
  per (metric, run) with `source = metric`, then one row per (summary key,
  run) with `source = summary` and the value in the `final` column, so a
  summary key equal to a metric key stays unambiguous.

**JSON:** `{"schema_version": 1, "kind": "time_series" | "run" | "runs" |
"comparison", "data": …}`.

* Series are columnar, with a `ticks` array and a `values` array.
* Import checks the kind, rejects newer schema versions, and re-validates
  every series: equal lengths, strictly increasing ticks, finite values.
* The default float parser of `serde_json` can be off by one unit in the
  last place. The `float_roundtrip` feature (enabled in
  `crates/analytics/Cargo.toml`) makes JSON round-trips bit-exact. A test
  covers this.

## 3. persistence

### 3.1 Envelope

```ron
#![enable(unwrap_newtypes)]
(
    format: "emergence-lab",
    kind: scenario,            // scenario | snapshot | snapshot_history | rule_set | experiment | settings | pattern
    schema_version: 3,
    app_version: "0.1.0",
    model_id: Some("life.conway"),
    created: "2026-11-09T10:00:00Z",
    payload: ( ... ),
)
```

* The encoding is chosen from the file extension, ignoring case: `.ron`,
  `.json`, `.toml` or `.elsnap`. Any other extension is refused with
  `UnsupportedExtension`.
* `Encoding::preferred_for(kind)` recommends TOML for settings, binary for
  snapshots and RON for everything else.
* RON is written pretty-printed with the `unwrap_newtypes` extension, so
  newtype structs read as their inner value.

`Codec` performs all file operations. `save`, `load`, `encode`, `decode` and
`read_header` take an explicit `DocumentSpec { kind, schema_version }`. The
typed helpers `save_document` and `load_document` work with any type that
implements `Document`, which declares `KIND` and `SCHEMA_VERSION` as
constants.

When a file is loaded, the codec checks the following in order:

1. **Size.** Files above `max_file_bytes` (default 1 GiB) are refused with
   `TooLarge` before any memory is allocated.
2. **Syntax.** Errors carry the parser's line and column.
3. **Format tag.** A file without `format: "emergence-lab"` is refused with
   `NotEmergenceLab`.
4. **Kind.** A different kind gives `WrongKind`.
5. **Model.** When an expected model is given, a mismatch gives
   `WrongModel`.
6. **Version.** A version newer than the current one gives `NewerVersion`.
   An older version is migrated (§3.2).
7. **Payload decode.** A payload that does not match the type gives
   `InvalidPayload`, which names the kind and schema version.

Every error raised during a file operation is wrapped in
`PersistenceError::AtPath`, so its message starts with the file path.
`root()` and `path()` give access to the underlying error and the path.

### 3.2 Migrations

`MigrationRegistry::register(kind, from_version, f)` registers a step
`f: Fn(serde_json::Value) -> Result<serde_json::Value, String>` that
converts the payload from `from_version` to `from_version + 1`. Registering
the same step twice is an error. To load a text file with version `v` when
the current version is `c > v`:

1. The whole chain `v → … → c` is checked first. If a step is missing, the
   load fails with `MissingMigration`, naming the first missing version.
   `missing_steps` lets an application validate its registry at start-up.
2. The document is parsed into the JSON data model. RON is mapped with
   serde's externally tagged enum convention: `Variant(x)` becomes
   `{"Variant": x}`, a unit variant becomes `"Variant"`, and `Some(x)`
   becomes `x`. This is the same representation `serde_json` uses, so
   migration code is identical for every text format.
3. Each step runs in order. A step that fails gives `MigrationFailed`, which
   names the step (`v → v + 1`) and gives its reason.
4. The result is decoded into the current type. `Loaded::migrated_from`
   records the original version, and the envelope's `schema_version` is
   updated to the current one. `created` and `app_version` keep the values
   from the file.

The tests use a fake chain of three versions. Version 1 → 2 splits `size`
into `width` and `height`. Version 2 → 3 adds `seed` and replaces
`wrap: bool` with a `Topology` enum. The chain is exercised in JSON, RON and
TOML, from version 1 and from version 2, together with missing, failing and
duplicate steps.

### 3.3 Binary `.elsnap` container

```text
magic "\x89ELSNAP\n" | u16 container version (1) | u16 flags (0) | u32 header length H
| u64 payload length P | u32 CRC-32 of header+payload | header (postcard) | payload (postcard)
```

All integers are little-endian.

* The magic starts with a non-ASCII byte and ends with LF, so files damaged
  by a text-mode transfer are detected.
* Lengths must match the file size exactly. Shorter files are reported as
  truncated, and extra bytes as trailing data.
* The CRC-32 is the IEEE polynomial, implemented in the crate. Its check
  value for `"123456789"` is `0xCBF43926`, and a test verifies it. A
  corrupted file is reported before any decoding.
* `binary::read_header` reads only the prefix and the header, so saves can
  be listed without reading large payloads. It does not verify the checksum.
* The header and the payload must each be consumed entirely by their
  decoders. postcard happily decodes a prefix of the bytes, so without this
  check a payload of another type could decode "successfully" as a shorter
  type; leftover bytes are reported as `InvalidPayload`.

### 3.4 Snapshot files

* `save_snapshot` and `load_snapshot` store one `simulation_core::Snapshot`
  in an envelope of kind `snapshot`. The envelope's `model_id` must equal
  the snapshot's `model_id`, and `load_snapshot` re-checks that they agree.
  The envelope's `schema_version` (`SNAPSHOT_SCHEMA_VERSION`) versions the
  container. The model's state version travels inside the `Snapshot`, and
  the model checks it when it restores the snapshot.
* `save_history` and `load_history` store a `Vec<Snapshot>`, which must be:
  * non-empty;
  * from a single model;
  * in non-decreasing tick order;
  * within `HistoryLimits { max_snapshots (10 000), max_total_bytes (512 MiB) }`.

  These rules are enforced both on export and on import.

### 3.5 Atomic writes

`atomic_write(path, bytes)` writes the data and replaces the destination in
these steps:

1. Create `.<name>.<pid>.<n>.tmp` in the destination folder with
   `create_new`.
2. Write the data and call `sync_all`.
3. Rename the temporary file over the destination. Renaming within one file
   system is atomic.
4. On Unix, call `fsync` on the parent directory so the rename survives a
   power loss.

If any step fails, the temporary file is removed and the destination is left
untouched. At most the first 120 bytes of the destination name (cut on a
character boundary) are copied into the temporary name, so destinations
whose names are close to the 255-byte limit can still be written. A test runs 8 concurrent writers while a reader checks that it
only ever sees complete versions. `read_limited` checks the file size before
allocating memory and stops reading at the limit, in case the file grows
while it is being read (the limit arithmetic saturates, so `u64::MAX` means
"unlimited" instead of overflowing).

### 3.6 User data folders

`UserDataDirs::resolve()` uses only `std::env`:

| Condition | Root |
|---|---|
| `EMERGENCE_LAB_DATA` set and non-empty | that path |
| macOS | `$HOME/Library/Application Support/EmergenceLab` |
| Linux / other Unix | `$XDG_DATA_HOME/emergence-lab` if `XDG_DATA_HOME` is set and absolute, otherwise `$HOME/.local/share/emergence-lab` |
| Windows | `%APPDATA%\EmergenceLab`, otherwise `%USERPROFILE%\AppData\Roaming\EmergenceLab` |

The subfolders are `saves/`, `scenarios/`, `exports/` and `screenshots/`, and
the settings file is `settings.toml` at the root. `ensure()` creates them.
`DataFolder::for_kind` gives the default folder for each kind of file.
`resolve_with(platform, var)` accepts an injected variable lookup, so all
platforms are tested on any host. Tests do not mutate the process
environment, which would require `unsafe` in edition 2024.

## 4. Approximations and limitations

* **Display buckets** span power-of-two numbers of ticks, so between
  one quarter and all of the `display_points` budget is used once a series
  exceeds it (the width only doubles). After a series is truncated (rewind)
  or compacted, its display summary is rebuilt from the stored samples.
* **Raw-sample cap.** Compaction by `max_raw_points` permanently drops
  intermediate samples, keeping each bucket's first, last, min and max.
  Statistics computed afterwards (mean, autocorrelation, CSV) use the
  decimated data, and the stored samples are no longer evenly spaced (the
  period estimator then refuses `estimate_series_period`).
* **Extinction** ticks are reported for every metric of a comparison,
  including per-tick counters, where "stays at or below the threshold" does
  not mean extinction.
* **FFT autocorrelation** agrees with the direct sums to about `10⁻¹²`
  relative error; on exactly periodic data a tie between neighbouring lags
  could in principle resolve differently than with the direct sums.
* **The period estimator** assumes roughly stationary, evenly sampled data.
  The caller should remove trends, for example by analysing growth rates
  instead of raw populations, and skip initial transients by passing a
  window. The harmonic rule is a heuristic, and for very noisy signals the
  chosen lag can be a multiple of the true period.
* **Extinction** is judged on the recorded data only. A population that
  would recover after the end of the run still counts as extinct.
* **Simpson** uses proportions, which is the large-sample form. Shannon is
  in nats.
* **Migrations through the JSON data model.** The following RON features
  have no JSON equivalent and cannot be migrated:
  * non-string map keys;
  * NaN and infinity;
  * byte strings;
  * newtype structs written as `(x)` without `unwrap_newtypes`;
  * named structs written as `Name(...)`, which become enum-like maps.

  Files at the current version are decoded directly from their own format
  and do not have these restrictions. Files written by this crate use
  `unwrap_newtypes` and no struct names.
* **Binary files cannot be migrated.** postcard is not self-describing, so a
  binary file with an older envelope version is refused with
  `BinaryMigration`. Model state inside snapshots is versioned separately,
  by `Snapshot::schema_version`. Binary payloads also cannot contain types
  that need `deserialize_any`, such as untagged enums (including
  `ParamValue`) and `serde_json::Value`. Scenarios and settings belong in
  text files.
* **TOML** has no null and no unit values, and its integers are limited to
  `i64`. A value that TOML cannot represent produces an `Encode` error.
* **CSV** files contain no formula-injection protection. A metric key
  starting with `=` is written as-is.
* **Atomic writes** are only atomic within one file system, which holds
  because the temporary file is created next to the destination. A crash
  can leave a `.<name>.<pid>.<n>.tmp` file behind. Such files are
  recognisable but not cleaned up automatically. The new file gets default
  permissions (the old file's mode is not copied), and a destination that is
  a symbolic link is replaced by a regular file rather than written through.

## 5. References

* Welford, B. P. (1962). Note on a method for calculating corrected sums of squares and products. *Technometrics* 4(3).
* Chan, T. F., Golub, G. H., LeVeque, R. J. (1979). Updating formulae and a pairwise algorithm for computing sample variances. Stanford CS report STAN-CS-79-773.
* Hyndman, R. J., Fan, Y. (1996). Sample quantiles in statistical packages. *The American Statistician* 50(4).
* Sturges, H. A. (1926); Scott, D. W. (1979); Freedman, D., Diaconis, P. (1981): histogram bin-width rules.
* Shannon, C. E. (1948); Simpson, E. H. (1949); Pielou, E. C. (1966): diversity and evenness indices.
* Gini, C. (1912). *Variabilità e mutabilità*.
* Kaplan, E. L., Meier, P. (1958). Nonparametric estimation from incomplete observations. *JASA* 53(282). Greenwood, M. (1926).
* Freireich, E. J. et al. (1963), 6-MP remission data, as tabulated in Kleinbaum, D. G., Klein, M., *Survival Analysis: A Self-Learning Text*, ch. 2.
* Jugel, U., Jerzak, Z., Hackenbroich, G., Markl, V. (2014). M4: a visualization-oriented time series data aggregation. *PVLDB* 7(10).
* Steinarsson, S. (2013). *Downsampling time series for visual representation* (LTTB). MSc thesis, University of Iceland.
* RFC 4180 (CSV); XDG Base Directory Specification; CRC-32/ISO-HDLC (IEEE 802.3).

## 6. How to test

```sh
CARGO_TARGET_DIR=target/agents/analytics cargo test -p analytics
CARGO_TARGET_DIR=target/agents/analytics cargo test -p persistence
CARGO_TARGET_DIR=target/agents/analytics cargo clippy -p analytics -p persistence --all-targets -- -D warnings
```

The tests check rules and known values, not only that the code runs.

analytics:

* **Statistics:**
  * Welford against the textbook data set `2,4,4,4,5,5,7,9` (mean 5, σ 2);
  * numerical stability with values near 10⁹;
  * `merge` equals a sequential pass;
  * type-7 quantiles;
  * Pearson against a known value (0.7745966692);
  * Spearman equals 1 on monotone data;
  * average ranks for ties.
* **Histograms:**
  * bin counts, including a value equal to `hi`;
  * under- and overflow;
  * a brute-force assignment of 2000 random values to half-open bins;
  * the bin counts of all four automatic rules on 0..99;
  * the bin cap, `MAX_BINS`, non-finite or too-wide ranges, and rejection
    of damaged imports (no bins, reversed range);
  * constant data with a large magnitude.
* **Diversity:** the indices of even and uneven communities computed by
  hand, and degenerate communities.
* **Gini:**
  * exactly 0 for equal amounts (any value, 1 to 59 members), all zeros and
    a single value;
  * 0.75 for (0, 0, 0, 10);
  * 0.25 for 1..4;
  * random samples against the O(n²) mean-absolute-difference definition.
* **Kaplan–Meier:** the Freireich 6-MP table. Survival is 0.857, 0.807,
  0.753, 0.690, 0.627, 0.538 and 0.448 at t = 6, 7, 10, 13, 16, 22 and 23.
  The test also checks the risk sets, the Greenwood standard errors (0.0764
  and 0.0869) and the median of 23.
* **Downsampling:**
  * `min_max` and M4 keep the global extremes, including injected spikes,
    and stay within budget with ticks in order, also for unordered input;
  * buckets cover equal tick spans (dense start plus sparse tail);
  * LTTB returns exactly `n` points with both endpoints and picks an
    isolated peak;
  * the incremental M4 equals a brute-force M4 on its grid, for regular and
    irregular ticks, and after replacing or removing the last samples;
  * the store displays full resolution up to N samples and decimates beyond
    that while keeping extremes, and irregular sampling is not crowded out;
  * the raw cap keeps extremes and the latest sample after every push, and
    right after each of more than 100 compactions the stored samples equal
    the M4 summary of all original samples.
* **Dynamics:**
  * recovery of the growth rate and doubling time;
  * the FFT (impulse, cosine, inverse) and FFT autocorrelation against the
    direct sums; a 200 000-sample noisy sine gives period 137 ± 0.5;
  * pulsar populations (48, 56, 72) give period 3, and a period-15 profile
    gives 15;
  * invalid period options are rejected;
  * a noisy sine with period 25 gives 25 ± 1, and 100 ticks at a spacing
    of 4;
  * constant and ramp series have no period;
  * extinction is detected for a permanent drop but not for a temporary
    dip, and not at all with a NaN threshold.
* **Store, runs and export:**
  * ordering errors leave the store unchanged;
  * same-tick replacement, including a NaN removing the replaced sample;
  * comparison tables, including missing metrics and summary keys, the
    `source` column, a NaN threshold and inconsistent tables;
  * union and intersection alignment against a brute-force lookup, and
    validation of imported alignments;
  * the streaming CSV writer equals the aligned table, write errors are
    reported, and run comments stay on one line;
  * CSV escaping of commas, quotes, CR, LF and leading spaces;
  * exported CSV parsed back with an RFC 4180 reader, checking consistent
    columns and empty gap cells;
  * bit-exact number round-trip;
  * JSON round-trip of runs, stores and comparisons, and rejection of
    out-of-order, mismatched, wrong-kind and newer-version documents.

persistence:

* **Round-trips:**
  * every encoding (RON, JSON, TOML, `.elsnap`);
  * header-only reads;
  * typed TOML settings.
* **Migrations:**
  * the v1 → v2 → v3 chain from v1 and v2 files in JSON, RON and TOML;
  * RON enum syntax converted to serde's JSON form during migration;
  * missing, failing and duplicate steps, and newer files.
* **Validation:**
  * hand-written RON with comments and struct names at the current version;
  * wrong kind, wrong model and foreign files;
  * syntax errors with line and column, and non-UTF-8 input;
  * payload mismatch;
  * file errors carry the path;
  * size limits.
* **Binary container:**
  * the CRC-32 check value;
  * detection of truncation, trailing data, bit flips, wrong magic, newer
    container versions and unknown flags;
  * a payload that a shorter type would only partly consume is rejected.
* **Snapshots:**
  * the snapshot round-trip restores the RNG stream;
  * header/data model mismatch is detected;
  * corruption is detected;
  * history limits and the consistency rules.
* **Atomic writes:**
  * replacement leaves no temporary files;
  * a failed rename keeps the destination and removes the temporary file;
  * concurrent writers and a reader never observe a partial file;
  * destination names near the length limit (multi-byte characters) work;
  * `read_limited` with an unlimited (`u64::MAX`) limit.
* **Data folders:** the defaults for each platform, the override, the XDG
  rules and folder creation, with an error that names the blocking path.
