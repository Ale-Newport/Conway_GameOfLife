# continuous — Continuous Artificial Life

`crates/continuous` implements continuous-state cellular automata on a periodic
grid: **Lenia** and **Expanded (multi-channel, multi-kernel) Lenia**,
**SmoothLife** and **Flow-Lenia**. The equations, parameters and preset data come
from the primary sources reviewed in
[`docs/research/continuous-life.md`](../research/continuous-life.md); this page
states what the code executes, how it was verified and where it deliberately
differs from a source.

The crate is pure Rust (dependencies: `simulation-core`, `rustfft`, `rayon`,
`serde`, `ron`, `thiserror`), renderer-independent and deterministic for a given
seed, platform and build.

## Contents

1. [Module map and public API](#module-map-and-public-api)
2. [Lenia and Expanded Lenia](#lenia-and-expanded-lenia)
3. [SmoothLife](#smoothlife)
4. [Flow-Lenia](#flow-lenia)
5. [Numerics: FFT, direct convolution, precision, parallelism](#numerics)
6. [The `continuous.*` models](#the-continuous-models)
7. [Scenarios and presets](#scenarios-and-presets)
8. [Approximations and limitations](#approximations-and-limitations)
9. [How it was verified](#how-it-was-verified)
10. [Performance](#performance)
11. [References](#references)

## Module map and public API

| Module | Content |
|---|---|
| `kernel` | `KernelCore` (exponential, polynomial, rectangular, staircase, Gaussian bump), `lenia_shell`, `antialiased_disc`, `GaussianRing`/`gaussian_rings`, `KernelImage::radial` (sampling of the non-zero taps, `f64` normalisation, fit check; dense wrapped image on demand) |
| `growth` | `GrowthFunction` (Gaussian, polynomial, rectangular) with `f64` reference and prepared `f32` evaluation |
| `fft` | `Fft2`: 2-D complex FFT from `rustfft` row/column passes, transposed spectral layout, tiled transpose |
| `convolve` | `ConvolutionPlan` (many kernels, real kernel spectra, paired inverse transforms, direct path, resource limits via `ConvolutionPlan::check`), `ConvolutionMethod {Auto, Fft, Direct}`, `convolve_direct` |
| `lenia` | `LeniaRule`, `LeniaKernel`, `GrowthNormalization`, `LeniaEngine` |
| `smoothlife` | `SmoothLifeRule`, `SmoothLifeMixing`, `SmoothLifeStepping`, `sigma1`, `SmoothLifeEngine` |
| `flow` | `FlowLeniaRule`, `FlowKernel`, `axis_weights`, `reintegrate`, `FlowEngine` |
| `rule` | `Rule` (one of the three), `ModelKind`, model ids |
| `edit` | `CellData`, `CreatureCells`, `place` (position + rotation + scale, bilinear), `Brush`/`apply_brush`, `Region`/`randomize`, `random_discs` |
| `rle` | Chan's 8-bit cell RLE (`decode`, `encode`) |
| `scenario` | `Scenario` RON files: `parse`, `to_ron`, `load_file`, `load_dir`, `builtin`, `builtin_by_id`, `creature_library` |
| `stats` | ordered sums, `ChannelStats`, `torus_centroid`, `CentroidTracker` |
| `parallel` | `for_row_blocks`, `for_row_blocks_with` (per-block scratch pool), `sum_rows` (deterministic row-block parallelism) |
| `sim` | `ContinuousSim` (`simulation_core::Simulation`), `ContinuousCommand`, `params`, `view::{ContinuousViewRequest, ContinuousView, ViewLayer}` |

Typical use:

```rust
use continuous::{ContinuousSim, ContinuousViewRequest, scenario};
use simulation_core::Simulation;

let mut sim = ContinuousSim::from_scenario(scenario::builtin_by_id("lenia.orbium").unwrap())?;
sim.advance(100);
let view = sim.view(&ContinuousViewRequest::default());   // planes + per-channel stats
let runner = simulation_core::Runner::spawn(sim, Default::default())
    .with_clear(|s| s.clear())
    .with_randomize(|s, seed| s.randomize(seed));
```

## Lenia and Expanded Lenia

Chan (2019) eqs. 7–16 and Chan (2020) eq. 2. State: `C` channels `A_j(x) ∈ [0, 1]`
on a `W × H` torus. Space resolution `R` (cells per kernel radius), time
resolution `T`, `dt = 1/T`. One tick:

```text
U_k  = K_k ∗ A_src(k)                                           (periodic convolution)
D_j  = Σ_{k: dst(k)=j} h_k · G_k(U_k)                           normalization = sum
D_j  = Σ_{k: dst(k)=j} h_k · G_k(U_k) / Σ_{k: dst(k)=j} h_k      normalization = per_destination
A_j ← clip(A_j + dt · D_j, 0, 1)
```

**Kernels.** Kernel `k` has radius `R·r_k` cells, ring peaks `β_k` (`B` rings) and a
core `K_C`. It is sampled at integer offsets `n` (cell centres):
`K_k(n) ∝ β[⌊Bρ⌋] · K_C(Bρ mod 1)` with `ρ = ‖n‖/(R·r_k) < 1` (Chan eq. 11 and the
`kernel_shell` code detail), normalised in `f64` so that `Σ_n K_k(n) = 1`
(eq. 12), and stored centred at index (0, 0) with wrap-around (no `fftshift`).

| core (`KernelCore`) | `K_C(ρ)` |
|---|---|
| `exponential` (bump4, α = 4) | `exp(4 − 1/(ρ(1−ρ)))`, 0 at the ends |
| `polynomial` (quad4, α = 4) | `(4ρ(1−ρ))⁴` |
| `rectangular` | `1` on `[¼, ¾]` |
| `staircase` | `1` on `[¼, ¾]`, `½` on `[0, ¼)` |
| `gaussian_bump` (Lenia tutorial) | `exp(−((ρ − ½)/0.15)²/2)` |

**Growth** (`GrowthFunction`, eq. 13): Gaussian `2·exp(−(u−μ)²/(2σ²)) − 1`;
polynomial `2·1[|u−μ| ≤ 3σ]·(1 − (u−μ)²/(9σ²))⁴ − 1`; rectangular `+1` if `|u−μ| ≤ σ`,
else `−1`.

**Expanded Lenia.** Every kernel carries its own `r, β, core, growth, μ, σ, h` and a
source and destination channel (up to 8 channels and 64 kernels). Chan's
`LeniaNDKC.py` divides by the weight sum per destination channel; the tutorial
uses the raw sum. Both are available (`GrowthNormalization`), chosen per
scenario.

**Preset fidelity.** `animals.json` stores `kn = 1, gn = 1`, which Chan's
`LeniaND.py` executes as the *polynomial* core and *polynomial* growth (its
tables are indexed with `kn − 1`), although its UI labels `kn = 1`
"exponential". Every preset therefore stores `core` and `growth` explicitly; the
`animals.json` presets use polynomial/polynomial.

## SmoothLife

Rafler (2011) eqs. 1–8. State `f ∈ [0, 1]`. Inner radius `rᵢ = rₐ/ratio`,
anti-aliasing rim `b`:

```text
w_d(u) = clip((rᵢ + b/2 − |u|)/b, 0, 1)                 disc weights
w_a(u) = clip((rₐ + b/2 − |u|)/b, 0, 1) − w_d(u)        annulus weights
m = Σ w_d f / Σ w_d,   n = Σ w_a f / Σ w_a              (two normalised kernels)
σ₁(x, a, α) = 1/(1 + exp(−4(x − a)/α))
σ₂(x, a, b) = σ₁(x, a, αₙ)·(1 − σ₁(x, b, αₙ))
σₘ(x, y, m) = x·(1 − σ₁(m, ½, αₘ)) + y·σ₁(m, ½, αₘ)
s(n, m) = σ₂(n, σₘ(b₁, d₁, m), σₘ(b₂, d₂, m))           mixing = bounds (paper eq. 6)
s(n, m) = σₘ(σ₂(n, b₁, b₂), σ₂(n, d₁, d₂), m)            mixing = indicators (Ready's SmoothLifeL)
```

Time stepping (`SmoothLifeStepping`, Rafler's modes, then clip to `[0, 1]`):
`discrete` (mode 0) `f ← s`; `smooth` (mode 1) `f ← f + dt·(2s − 1)`;
`relax` (mode 2) `f ← f + dt·(s − f)`. Modes 3 and 4 of a third-party port (which
integrate from `m` instead of `f`) are **not implemented**: they could not be
checked against Rafler's shader.

The two fillings are one forward FFT and one *paired* inverse FFT (see
[Numerics](#numerics)).

## Flow-Lenia

Plantec et al. (2023 eqs. 1–7; 2025). State: `C` channels of non-negative,
unbounded mass. One tick:

```text
U_j  = Σ_{k: dst(k)=j} h_k · G_k(K_k ∗ A_src(k))      affinity, Gaussian growth
A_Σ  = Σ_c A_c,   α = clip((A_Σ/θ_A)^n, 0, 1)
F_j  = (1 − α)·∇U_j − α·∇A_Σ                          flow
δ_j  = clip(dt·F_j, −(w − s), w − s)                   per axis, window w
A_j'(p) = Σ_{p'} A_j(p')·|Ω(p) ∩ Sq(p' + ½ + δ_j(p'), s)| / (2s)²   reintegration tracking
```

Kernels are sums of Gaussian rings `Σ b·exp(−(D − a)²/(2w²))`, `D = ‖n‖/(r·R) < 1`,
normalised to `Σ K = 1` (paper eq. 1, `2w²` convention). `Sq(c, s)` is the square
of side `2s` centred at `c` ("temperature" `s`).

**Reintegration as a scatter.** Each source cell spreads its mass over the at
most `⌈2s⌉ + 1` cells per axis that its displaced square overlaps. Per-axis
overlaps are computed in *source-local* coordinates and renormalised to sum to
exactly 1 (`flow::axis_weights`), and destinations accumulate in `f64`; the only
rounding left is the final conversion of each cell to `f32`. With `F = 0`,
`s = 0.5` is the identity and `s = 0.65` blurs with per-axis weights
`[0.11538, 0.76923, 0.11538]` (unit-tested). The `mass_drift` metric reports the
relative deviation from the mass after the last reset or edit.

**Conventions chosen** (the paper and its reference code disagree; see
the research note §6.6):

* gradients use the *unnormalised* 3×3 Sobel operator (≈ 8× the derivative), as
  the reference code does, so the published `dt = 0.2` and `s = 0.65` keep their
  meaning; unlike the reference code the Sobel stencil wraps periodically
  instead of zero-padding;
* `α` uses the summed mass `A_Σ` and `θ_A` as in the paper (the code's
  per-channel `θ_A = C` form is equivalent for one channel with `θ_A = 1`);
* the kernel radius is `r·R` (paper), not the code's `(R + 15)·r`; ring width uses
  the paper's `2w²`.

## Numerics

**FFT convolution** (`fft`, `convolve`). `rustfft` 6.4 only (no `realfft`): the
real field is loaded into a complex buffer, row FFTs run in place, a tiled
transpose produces the *spectral layout* (`W` rows of `H` bins) and the column
FFTs run as contiguous rows. Kernel spectra are precomputed once per rule and
grid size in the same transposed layout with `1/(W·H)` folded in, so a
convolution is: forward transform of each source channel, pointwise product,
inverse transform. Every kernel is sampled from a function of the distance
only, so it is exactly point-symmetric (`K(n) = K(−n)`) and its DFT is real;
the plan stores only the real part of each kernel spectrum (the imaginary part
is rounding noise, below 10⁻⁶ of the largest bin — unit-tested), which halves
spectrum memory and the work of the pointwise products. Kernels themselves are
stored as their non-zero taps; the dense wrapped image is only materialised
while a spectrum is computed. Because all fields and kernels are real, two kernels share
one inverse transform: `IFFT(S_a·K̂_a + i·S_b·K̂_b) = U_a + i·U_b` (exact by
linearity). `C` source channels and `K` kernels cost `C + ⌈K/2⌉` 2-D transforms
(Tessellatium: 3 + 8 instead of 3 + 15; SmoothLife: 1 + 1).

**Direct convolution** sums `U(x) = Σ_n K(n)·A(x + n)` over the kernel's non-zero
taps from a horizontally wrap-padded copy (contiguous inner loop). It is used for
tiny kernels and as the oracle of the FFT path. `ConvolutionMethod::Auto` picks
direct when `taps < 3·(transforms)·log₂(W·H)` (calibrated with the benchmark in
[Performance](#performance)); the `convolution` parameter can force either.

**Resource limits.** `ConvolutionPlan::check` validates kernels, resolves the
method and checks two limits before anything world-sized is allocated: an FFT
plan may use at most `MAX_PLAN_BYTES` = 4 GiB (e.g. 64 kernels on a 4096² world
are refused), and forced direct summation at most `MAX_DIRECT_WORK` = 10¹⁰
multiply-adds per tick (≈ 1 s; e.g. SmoothLife's 1 450 taps on 4096²). Both
are reported as `SimError::LimitExceeded` (or as an invalid parameter when a
size or method change would cross them) instead of exhausting memory or
freezing the simulation thread.

**Precision.** Fields are `f32`. Kernel sampling and normalisation, all sums
(mass, centroid, statistics) and Flow-Lenia's reintegration accumulator are
`f64`.

**Parallelism and determinism.** Every parallel loop goes through
`parallel::for_row_blocks`: blocks of whole rows (`rows / (2·threads)` per task,
as measured in the research note §7.5; grids of up to 256² cells run serially,
see [Performance](#performance)) computed by pure functions into separate output
buffers. FFT scratch buffers come from a per-block pool allocated once per plan. Each row is computed by the same code
whatever the block boundaries, so results are bitwise identical for any thread
count (tested with a one-thread pool). Reductions add per-row partial sums in row
order. Flow-Lenia's scatter runs serially in row-major order. Randomness comes
only from the `SimRng` in the state.

## The `continuous.*` models

`ContinuousSim` implements `Simulation` for three model ids: `continuous.lenia`,
`continuous.smoothlife` and `continuous.flow_lenia`. The id, name, rules text,
parameters and metrics depend on the scenario's rule. One tick advances
`dt` time units (`1/T` for Lenia, the `dt` parameter for Flow-Lenia and smooth
SmoothLife modes, 1 step for discrete SmoothLife).

### Parameters

All rule parameters apply at the next tick (`NextTick`): a change is validated —
including whether every kernel still fits the current *and* the pending world
size (`2·reach + 1 ≤ min(W, H)`) and the resource limits — and the kernel
spectra are rebuilt before `set_param` returns. World size and seed apply at the
next reset (`RequiresReset`); a pending size is accepted only if the next reset
can build it (kernels fit, limits hold, and the scenario's creatures fit the
world), so `reset()` never fails silently.

Every spec's `default` is the *scenario's* value (world size, seed, and each
rule parameter of the preset; kernels the preset does not have default to the
current values), so a "restore default" control in the property panel restores
the preset — e.g. `k0.core` defaults to `polynomial` for `lenia.orbium_chan`
and `k3.beta` to `0,1` for Tessellatium.

| Key | Models | Meaning |
|---|---|---|
| `width`, `height` | all | world size (16…4096), on reset |
| `seed` | all | random seed used by reset |
| `convolution` | all | `auto` / `fft` / `direct` |
| `radius`, `time_resolution`, `normalization` | Lenia | `R`, `T`, growth combination |
| `k{i}.mu`, `k{i}.sigma`, `k{i}.weight`, `k{i}.relative_radius` | Lenia, Flow | per-kernel `μ, σ, h, r` |
| `k{i}.beta`, `k{i}.core`, `k{i}.growth` | Lenia | rings (`"1/2,1,2/3"`), core, growth function |
| `k{i}.source`, `k{i}.destination` | multi-channel worlds | kernel routing |
| `outer_radius`, `radius_ratio`, `rim`, `b1`, `b2`, `d1`, `d2`, `alpha_n`, `alpha_m`, `mixing`, `stepping`, `dt` | SmoothLife | Rafler's parameters |
| `radius`, `dt`, `critical_mass`, `alpha_exponent`, `temperature`, `window` | Flow-Lenia | `R, dt, θ_A, n, s, w` |
| `k{i}.rings` | Flow-Lenia | rings `"a,b,w; a,b,w"` |

### Commands (`ContinuousCommand`)

* `Brush(Brush { x, y, radius, value, softness, mode: Paint | Erase, channel })` —
  `A ← A + w(value − A)` or `A ← A(1 − w)` with `w = 1` inside
  `(1 − softness)·radius` and a smoothstep to 0 at `radius`, evaluated at the
  shortest torus distance; every cell is edited once even when the brush is
  larger than the world.
* `PlaceCreature { cells, placement: Placement { x, y, rotation_deg, scale, blend } }` —
  exact copy when unrotated and unscaled (top-left snapped to a cell), otherwise
  bilinear resampling at the inverse-transformed cell centres (mass scales with
  `scale²`); `blend` replaces the covered box or adds. The rotated, scaled box
  must fit the world (otherwise it would overlap itself on the torus and `Add`
  would count cells twice), and any finite centre wraps. Creatures come from
  `scenario::creature_library()` or any decoded `CellData`.
* `Randomize { region: Rect | Disc, density, amplitude }` — each covered cell and
  channel becomes `amplitude·U(0, 1)` with probability `density`, else 0, drawn
  from the state's `SimRng` in row-major order.
* `Clear`.

Edits write values in `[0, 1]` for Lenia and SmoothLife and in
`[0, FLOW_EDIT_LIMIT]` = `[0, 100]` for Flow-Lenia, whose dynamics leave mass
unbounded but whose `f32` sums must stay finite (`Rule::edit_limit`): brush
values outside the range are errors, placements and random amplitudes are
capped. Every command validates its numbers (finite, ranges, channel indices)
and returns `SimError::InvalidCommand` otherwise. `clear()` and `randomize(seed)` back the runner's generic controls
(randomize: a centred disc of radius `min(W, H)/4` at density ½ for Lenia and
Flow-Lenia; `W·H/(2rₐ²)` random filled discs with radii in `[rᵢ, rₐ]` for
SmoothLife — such soups are chaotic, and with the paper's glider rule about
half of all seeds condense into gliders while the others die out; measured
7, 6 and 9 survivors out of 16 seeds for 74, 55 and 40 discs on 256²).

### Metrics

| Key | Meaning |
|---|---|
| `mass` | `Σ A` over all cells and channels |
| `active_fraction` | fraction of cells whose channel sum exceeds 0.01 |
| `centroid_x`, `centroid_y` | mass-weighted circular mean on the torus (per axis `atan2(Σ m sin θ, Σ m cos θ)`), world coordinates; 0 when undefined (empty or uniform world) |
| `speed` | length of the shortest wrapped centroid displacement in the last tick (cells/tick) |
| `mean_growth` | Lenia: mean `D`; SmoothLife: mean `2s − 1`; Flow-Lenia: mean affinity `U` |
| `mass_drift` | Flow-Lenia only: `(mass − reference)/reference` |

`ContinuousSim::centroid_track()` returns the unwrapped centroid trajectory,
which the tests use to measure glider speeds across the torus seams.

### Views

`ContinuousViewRequest { x0, y0, width, height, max_cols, max_rows, layer }`
selects a (wrapping) rectangle — `0` means the whole world — and an output
budget; the rectangle is box-filtered by an integer factor so that it fits.
`ViewLayer::State` returns one plane per channel, `Growth` the last tick's growth
or affinity per channel, `Potential` the potential of kernel 0 (SmoothLife: the
fillings `m` and `n`). Every view carries plane labels, a natural value range and
`ChannelStats { mass, mean, min, max, active_fraction }` per channel. The growth
range is `±max_j Σ_{k: dst(k)=j} h_k` for summed growths and affinities (e.g.
±2.0 for Tessellatium, whose channel-0 growth in empty space is about −1.9) and `±1` for
weighted means and SmoothLife.

### Snapshots and reset

Snapshots (`schema_version` 1, model id of the current rule) contain the
scenario, current rule, size, pending size, seed, convolution method, `SimRng`,
tick, simulated time, fields, centroid tracker and last metrics; kernel spectra are rebuilt on
restore. Restore checks model id, version and internal consistency (field
sizes, rule model, that the embedded scenario has the rule's model and channel
count and fits the pending size), so a corrupt or forged snapshot is refused
rather than breaking a later reset. `reset()`
rebuilds the scenario's initial state with the current rule and seed and applies
the pending size; scenario positions are fractions of the world size, so layouts
scale with it.

## Scenarios and presets

Scenario files live in `scenarios/continuous/*.ron` (embedded copies:
`scenario::builtin()`; a folder: `scenario::load_dir(path)`). Format (RON with the
`unwrap_variant_newtypes` and `implicit_some` extensions on by default):

```ron
(
    header: (id: "lenia.orbium", name: "Orbium unicaudatus", description: "...", seed: 1),
    width: 128,
    height: 128,
    rule: Lenia(radius: 13.0, time_resolution: 10.0, channels: 1, normalization: sum,
        kernels: [(relative_radius: 1.0, beta: [1.0], core: gaussian_bump, growth: gaussian,
                   mu: 0.15, sigma: 0.015, weight: 1.0, source: 0, destination: 0)]),
    init: [
        Creature(name: "Orbium", cells: Rows([[[0.0, 0.1, ...], ...]]), x: 0.5, y: 0.5, rotation: 0.0, scale: 1.0),
        Soup(x: 0.5, y: 0.5, radius: 48.0, shape: disc, density: 0.5, amplitude: 1.0),
        RandomDiscs(count: 74, min_radius: 7.0, max_radius: 21.0, value: 1.0),
    ],
    notes: ["provenance and licence notices"],
)
```

`cells` is `Rows` (explicit values, one array per channel) or `Rle` (Chan's 8-bit
RLE per channel). Positions are fractions of the world size; radii are cells.

| File | Id | Content | Source |
|---|---|---|---|
| `orbium.ron` | `lenia.orbium` | Orbium, tutorial configuration (Gaussian-bump kernel, Gaussian growth, σ = 0.015), 128² | Lenia tutorial, MIT |
| `orbium-chan.ron` | `lenia.orbium_chan` | Orbium `animals.json` cells, polynomial/polynomial, σ = 0.015 | Chan, MIT |
| `scutium-solidus.ron` | `lenia.scutium_solidus` | μ = 0.29, σ = 0.045, polynomial/polynomial | Chan, MIT |
| `hydrogeminium-natans.ron` | `lenia.hydrogeminium_natans` | R = 18, β = [1/2, 1, 2/3], μ = 0.26, σ = 0.036, 256² | Chan, MIT |
| `tessellatium-gyrans.ron` | `lenia.tessellatium_gyrans` | 3 channels, 15 kernels, R = 12, T = 2, summed growth | Lenia tutorial, MIT |
| `fish.ron` | `lenia.fish` | 1 channel, 3 kernels, averaged growth | Lenia tutorial, MIT |
| `lenia-soup.ron` | `lenia.soup` | Orbium rule, random disc soup (radius 48, density 0.5), 256² | — |
| `smoothlife-glider.ron` | `smoothlife.glider` | Rafler's smooth glider (rₐ = 21), cells extracted with this engine | Rafler 2011 parameters |
| `smoothlife-soup.ron` | `smoothlife.soup` | 74 random filled discs under the glider rule, 256², seed 7: blobs for ~150 steps, then smooth gliders | Rafler 2011 parameters |
| `flow-lenia-demo.ron` | `flow_lenia.demo` | one channel, 7 Gaussian-ring kernels, R = 33, 40×40 random patch, 128² | FL23 ranges, see below |

**Orbium kernel variants.** The tutorial variant (Gaussian-bump core, Gaussian
growth) is the canonical test fixture. Other published sets sustain the same
creature: exponential core with Gaussian growth and σ = 0.015, 0.016 (paper
figures) or 0.017 (JavaScript app), and `animals.json`'s polynomial/polynomial
(`orbium-chan.ron`). Switch with the `k0.core`, `k0.growth` and `k0.sigma`
parameters.

**SmoothLife glider fixture.** `examples/smoothlife_glider.rs` regenerates it:
for seeds 1, 2, … run the paper rule from the disc soup for 150 steps, split the
field into connected components, run each alone, keep the first whose mass
varies by < 2 % while moving > 2 cells/step, re-crop it after it settles,
quantise to 8-bit RLE and verify the quantised copy (1000 steps). With this
build the first glider comes from seed 4 (seeds 1–3 leave none; which seed works
depends on rounding, see below): a 46×45 crop (44×45 after RLE trims empty
columns), mass 799.5, speed 5.281 cells/step — the research reproduction
measured 799.4 ± 1.5 and 5.281 ± 0.008 with independent numpy code.

**SmoothLife soups are chaotic.** In discrete mode the fate of a disc soup
depends on the seed *and* on floating-point rounding: for 16 seeds the FFT and
direct paths disagreed on whether anything survives 600 steps in 2 cases, and
on the surviving structures in several more. The shipped `smoothlife.soup` uses
seed 7, whose outcome does not hinge on rounding: the soup churns for about 150
steps and then condenses into isolated smooth gliders — one glider with the FFT
and with direct summation, two for a translated copy of the soup (tested).

**Flow-Lenia demo.** No Flow-Lenia creature is published with complete
parameters in the paper convention, so the demo rule was found by a seeded
random search with this engine: kernels sampled uniformly from the [FL23] Table 1
ranges (`r ∈ [0.2, 1]`, `a, b ∈ [0, 1]³`, `w ∈ [0.01, 0.5]³`, `μ ∈ [0.05, 0.5]`,
`σ ∈ [0.001, 0.2]`, `h ∈ [0, 1]`, 2–10 kernels, `R ∈ [17, 40]` to match the
reference code's effective radii `(R + 15)·r`), a 40×40 uniform patch on 128²,
keeping rules whose matter stays localised (< 25 % active cells) and moves.
Among about 1000 trials most rules give stationary blobs; trial 1227 (shipped)
self-organises into a membrane-like structure covering about 10 % of the world
that keeps reorganising and drifts slowly. A compact fast glider was not found
by random sampling (the papers find them with evolutionary search), so the demo
is described as what it is. Its total mass (804.4022) stays constant to a
relative 2.2 × 10⁻⁸ over 1000 ticks.

## Approximations and limitations

* **Periodic boundaries only.** Every model runs on a torus; zero/wall
  boundaries (and Flow-Lenia's "wall" border) are not implemented.
* **`f32` fields.** Chaotic dynamics diverge from `f64` runs after ~1000 steps
  (statistics agree; see the research note §7.4). Results are reproducible on one
  platform and build, not bit-identical across CPU architectures (`rustfft`
  SIMD paths differ in the last ulp).
* **Lenia options not implemented:** state quantisation `P`, soft clip, the
  Adams–Bashforth integrator, the asymptotic ("target") update and 3-D/4-D
  worlds. The JavaScript app's names map onto the cores/growths above; the
  polynomial α is fixed at 4 as in Chan's presets.
* **SmoothLife modes 3–4** and higher-order integrators are not implemented.
  Discrete-mode soups are chaotic (see above): only statistical or
  rounding-robust outcomes are claimed for them.
* **Flow-Lenia:** no parameter localisation (multi-species genomes carried by
  matter), no food/decay, and the conventions listed in [Flow-Lenia](#flow-lenia)
  (unnormalised periodic Sobel, paper radius and ring width). Reintegration is a
  serial scatter.
* **Placement** with rotation or non-unit scale resamples bilinearly, which
  slightly blurs creatures (exact copies when unrotated and unscaled).
* **Centroid metrics** describe the summed mass of the whole world: with
  several creatures they describe the group, not an individual.
* `ConvolutionMethod::Auto`'s cost model is a single calibrated constant.

## How it was verified

Unit tests (`cargo test -p continuous`) check the rules with independently
computed values:

* kernel cores (`K_C(¼) = e^{−4/3}`, `0.75⁴`, bell 0.2493522), ring peaks at
  `(i + ½)/B`, `Σ K = 1` within 1e−6, 516/517 taps at R = 13, SmoothLife areas
  `M = 154.55`, `N = 1231.23`;
* growth values at `μ ± σ, 2σ, 3σ` and zero crossings for Gaussian and polynomial
  growth; the `f32` path against the `f64` reference;
* the 2-D FFT against a naive DFT, inverse round trip, FFT == direct convolution
  (max error < 1e−4) on random fields for odd, even and non-power-of-two sizes and
  for paired and single inverses; bitwise identity with one thread;
* SmoothLife sigmoid and the transition `s(n, m)` at six reference points;
  stepping modes on a uniform field;
* Flow-Lenia reintegration weights, identity at `s = 0.5`, conservation with
  random flows (relative error < 1e−12 in `f64`), translation, Sobel;
* kernel spectra are real (imaginary parts < 10⁻⁶ of the largest bin) and
  kernels are invariant under the eight lattice symmetries; resource limits are
  enforced without world-sized allocations;
* Chan RLE decoding (Orbium: 20×20, Σ = 19600/255), round trips and oversized
  counts; placement (rotation, scaling, fit on the torus, wrapping of far
  coordinates), brushes larger than the world (each cell edited once), soups;
  parameters, views, statistics.

Property tests (`tests/symmetry.rs`): Expanded Lenia (Tessellatium, 15
kernels, 3 channels), SmoothLife and Flow-Lenia commute with quarter turns,
mirror images and torus translations (max deviation after 10–20 ticks < 10⁻⁴,
10⁻³, 10⁻³; measured 5.5·10⁻⁶ to 7.6·10⁻⁵). Mutating the kernel centring by one
cell or flipping the sign of the Sobel `y` gradient makes these tests fail
(checked during review). Two direction tests fix the signs of Flow-Lenia's flow:
a faint probe moves *towards* a disc whose potential raises its affinity (`α ≈
0`), and a disc spreads under pure concentration flow (`α = 1`).

Integration tests (`tests/`; the long windows are the `#[ignore]` suite
`tests/verification.rs`, run with
`cargo test -p continuous --release --test verification -- --ignored --nocapture`):

| Test | Reference | Measured here |
|---|---|---|
| Test | Reference | Measured here (tested bound) |
|---|---|---|
| Orbium (tutorial) mass at t = 0/10/100/200/500 | 75.10 / 73.5506 / 73.0977 / 72.9739 / 72.9989 (independent Rust f32 run; numpy f64: 72.9991 at 500) | 75.1000 / 73.5506 / 73.0977 / 72.9739 / 72.9989 (±10⁻³) |
| Orbium direction, steps 0–1000 | displacement (rows, columns) ≈ (560.0, 222.2), 21.6° from +y | (560.4, 221.3), 602.5 cells, 21.55° from +y (tested: ±0.5°, distance ± 6) |
| Orbium mass band and speed, steps 100–500 / 500–1000 / 1000–5000 | mass ∈ [70, 76], [72.4, 73.3] for 500–1000; 0.602 ± 0.01 cells/step | [72.45, 73.31] / [72.43, 73.30] / [72.41, 73.34]; 0.6022 / 0.6022 / 0.6020 |
| Orbium (`animals.json`, poly/poly), steps 400–1000 | mass ≈ 73.6, speed 0.623 | mass [73.14, 74.04], speed 0.6233 |
| Scutium solidus (poly/poly), steps 500–2000 | mass 139.5, speed 0.36 | mass [138.95, 140.19], speed 0.3558 |
| Hydrogeminium natans (poly/poly), steps 400–2000 | mass 689 → 699 (2000 steps), speed ≈ 0.39–0.42 | initial 689.32, mass [689.14, 716.68], final 702.5, speed 0.410 |
| Tessellatium gyrans, steps 200–3000 | (no published number at scale 1) | mass [806.6, 813.7] (from 350.7 at t = 0), speed 0.596, 3.9 % active |
| K=3 Fish, steps 200–1000 / 200–3000 | (no published number) | mass [105.6, 113.3], net speed 0.217 / mass [103.6, 117.9], net speed 0.122 (it turns) |
| SmoothLife glider (r_a = 21), steps 50–300 / 300–1050 | mass 799.4 ± 1.5, speed 5.281 ± 0.008 | mass [799.05, 799.97] / [799.13, 799.70]; speed 5.2810 / 5.2811 |
| SmoothLife soup (seed 7, and a translated copy), 400 steps | — | isolated gliders: 1 (mass ≈ 799) / 2 (≈ 1599), < 2 % active per glider |
| Flow-Lenia demo mass, 500 / 1000 steps | exact conservation in exact arithmetic | worst relative error 2.2 × 10⁻⁸ / 3.2 × 10⁻⁸ (tested < 10⁻⁴ and < 10⁻⁶); per-tick drift < 10⁻⁶ |

Plus: FFT and direct paths agree on a running Orbium (< 1e−3 after 30 steps),
deterministic replay, thread-count independence, snapshot round trips (including
the generator state) for all three models, wrong-model/newer-version rejection,
forged snapshots (scenario with another channel count) refused, reset with a
pending size, pending sizes refused when a creature or the resource limits would
not fit, parameter policies, validation and scenario-valued defaults, Flow-Lenia
edit bounds, growth-view ranges, the threaded `Runner` with its generic clear /
randomize / parameter controls, commands, views, and that `scenario::builtin()`
equals the files in `scenarios/continuous/`.

## Performance

Criterion benchmark `benches/continuous.rs` (`cargo bench -p continuous`).
Apple M2 Max (8 performance + 4 efficiency cores), Rust 1.98.1, workspace
`release` profile (thin LTO, 4 codegen units), `f32` fields, rustfft 6.4.1
complex FFT with real kernel spectra. **The machine was heavily loaded by other
build jobs** (load average 10–20 on 12 cores during the run), so these are upper
bounds; parallel runs (512² and up) are the most affected. Median of the
Criterion estimate (low–high in brackets where the spread is large), measured
2026-11-09 during the review run:

| Benchmark | Grid | Time per tick |
|---|---|---|
| `lenia_step` (Orbium rule, R = 13, 1 kernel) | 128² | 0.167 ms |
| | 256² | 0.685 ms |
| | 512² (parallel) | 2.55 ms (1.92–3.29; 1.44 ms in an earlier, less loaded run) |
| | 1024² (parallel) | 6.63 ms (4.53 ms in an earlier run) |
| `tessellatium_step` (3 channels, 15 kernels: 3 + 8 transforms) | 256² | 5.35 ms |
| `smoothlife_step` (r_a = 21, 2 kernels: 1 + 1 transforms) | 256² | 1.70 ms |
| `flow_lenia_step` (demo rule, 7 kernels) | 128² | 1.00 ms |
| | 256² | 3.97 ms |

Single-kernel convolution at 256², serial, by method (used to calibrate
`ConvolutionMethod::Auto`):

| Kernel | Taps | FFT | Direct |
|---|---|---|---|
| R = 2 | 9 | 0.50 ms | 0.093 ms |
| R = 3 | 25 | 0.47 ms | 0.16 ms |
| R = 4 | 45 | 0.45 ms | 0.25 ms |
| R = 6 | 109 | 0.45 ms | 0.54 ms |
| R = 13 | 517 | 0.46 ms | 2.19 ms |

Direct summation runs at about 1.5·10¹⁰ multiply-adds per second here, so
`MAX_DIRECT_WORK` = 10¹⁰ is under a second per tick.

The crossover (≈ 100 taps, R ≈ 6) sets `DIRECT_TAPS_PER_FFT = 3`
(`taps < 3·transforms·log₂ cells`).

A first run with row-block parallelism already enabled at 256² measured
`lenia_step/256` at 1.16 ms (range 0.95–1.30 ms) and the 256² FFT convolution at
0.45–3.6 ms: on a loaded machine the five synchronisation points of a parallel
FFT convolution cost more than they save at this size. Parallelism therefore
starts above 256² (`parallel::PARALLEL_MIN_CELLS`); this does not change any
result, because row blocks are bitwise equivalent to the serial path.

For comparison, the research note's prototype measured (on an also loaded
machine) 0.38 ms/step single-threaded at 256² with a real-to-complex `realfft`
pipeline, about half the work of the complex transform used here; adopting
`realfft` would roughly halve the convolution cost but adds an MIT-only
dependency.

## References

* B. W.-C. Chan, "Lenia — Biology of Artificial Life", *Complex Systems* 28(3), 2019. arXiv:1812.05433.
* B. W.-C. Chan, "Lenia and Expanded Universe", ALIFE 2020, pp. 221–229. arXiv:2005.03742.
* S. Rafler, "Generalization of Conway's 'Game of Life' to a continuous domain — SmoothLife", 2011. arXiv:1111.1567.
* E. Plantec, G. Hamon, M. Etcheverry, P.-Y. Oudeyer, C. Moulin-Frier, B. W.-C. Chan, "Flow-Lenia: Towards open-ended evolution in cellular automata through mass conservation and parameter localization", ALIFE 2023. arXiv:2212.07906.
* E. Plantec et al., "Flow-Lenia: Emergent evolutionary dynamics in mass conservative continuous cellular automata", *Artificial Life* 31(2):228–248, 2025. arXiv:2506.08569.
* Chakazul/Lenia (`animals.json`, MIT, © 2018 Bert Chan) and lenia-org/Lenia-Tutorial (MIT, © 2021 Open Science Lenia): preset cells, with notices in each scenario file.
* Research note: [`docs/research/continuous-life.md`](../research/continuous-life.md).
