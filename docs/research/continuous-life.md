# Continuous cellular automata: Lenia, SmoothLife, Flow-Lenia, and how to build them in Rust

Research notes for the Emergence Lab continuous-CA module, written 2026-11-09.

Scope: exact model equations and parameters from the primary sources, published presets with their cell data, and an implementation plan for a pure-Rust, Bevy-free, deterministic simulation crate. The plan covers FFT convolution, rayon, CPU costs measured on this M2 Max, when to move to the GPU, and a test oracle list.

Status tags used below:

- **VERIFIED (source)**: read directly in the primary paper or reference code at the commit given.
- **VERIFIED (repro)**: reproduced numerically with our own independent code. The scratch code is in `/private/tmp/claude-501/-Users-alejandro-Projects-Conway-GameOfLife/d26793ef-30fe-4239-b1fb-77f42d831735/scratchpad/scripts/` (numpy, f64) and `/private/tmp/claude-501/-Users-alejandro-Projects-Conway-GameOfLife/d26793ef-30fe-4239-b1fb-77f42d831735/scratchpad/fftbench/` (Rust, f32). Neither is project code, and no third-party code was copied into it.
- **UNVERIFIED**: secondary source, memory, or inference. Re-check before relying on it.

---

## 0. TL;DR (decisions for implementers)

1. **Model**: implement a single generalized "Extended Lenia" engine: C channels, K kernels, each kernel `(R·r_k, β_k, core, μ_k, σ_k, h_k, src, dst)`, Euler step plus clip. Original Lenia, multi-ring Lenia, SmoothLife and Flow-Lenia's affinity map are all special cases or small add-ons:
   - SmoothLife needs two kernels (disk and annulus) and a 2D transition function `s(n, m)`.
   - Flow-Lenia replaces "add growth and clip" with "flow plus reintegration tracking".
2. **Convolution**: FFT on a periodic torus, using `realfft 3.5.1` (MIT) over `rustfft 6.4.1` (MIT OR Apache-2.0).
   - Real-to-complex row FFTs, then a transpose, then complex column FFTs over the `N/2+1` columns.
   - Precompute each kernel's spectrum once, with the 1/N² scale folded in.
   - Measured on this M2 Max: a full Lenia step takes **0.38 ms at 256²** and **1.6 ms at 512²** single-threaded f32. With coarse-grained rayon it takes **0.33 ms at 256²** and **0.67 ms at 512²**.
   - Direct convolution with R=13 (517 taps) is about 9–10× slower single-threaded and 2–3× slower with rayon. Use it only for tiny radii, spatially varying kernels, or as a test oracle.
3. **Precision**: use `f32` for simulation. Our Rust f32 Orbium run matches a numpy f64 run to 4 decimals of total mass for 500 steps (§8). Keep the kernel normalization and the statistics sums in `f64`.
4. **Preset fidelity issue (important)**: Chan's Python app maps `kn=1, gn=1` (as stored in `animals.json`) to the **polynomial (quad4)** kernel core and **polynomial** growth, even though its UI labels `kn=1` as "Exponential".
   - In our survey, 227 of 239 `animals.json` species survived 600 steps under polynomial/polynomial, against 181 of 239 under exponential/gaussian. All 46 disagreements favoured polynomial.
   - Store `core` and `growth` explicitly in every preset. Import `animals.json` with polynomial/polynomial.
5. **GPU**: not needed for single-channel Lenia up to about 1024² (1.9 ms/step on the CPU with rayon). Move to wgpu compute in the rendering crate for:
   - multi-channel / multi-kernel worlds of 512² and up (3 channels × 15 kernels, convolutions only: 4.5 ms at 512², 14 ms at 1024²; growth adds about 15 × the per-kernel growth cost),
   - 3D Lenia,
   - large Flow-Lenia worlds,
   - batched evolutionary search.

   Keep the CPU engine as the deterministic reference.

---

## 1. Sources

| Ref | What | Where | License / notes |
|---|---|---|---|
| [L19] | B. W.-C. Chan, "Lenia — Biology of Artificial Life", *Complex Systems* 28(3):251–286, 2019 | arXiv:1812.05433v3 (4 May 2019), https://arxiv.org/abs/1812.05433 ; journal: https://www.complex-systems.com/abstracts/v28_i03_a01/ | paper (all rights reserved by authors/publisher); equations are facts |
| [L20] | B. W.-C. Chan, "Lenia and Expanded Universe", ALIFE 2020, pp. 221–229 | arXiv:2005.03742v1 (7 May 2020), https://arxiv.org/abs/2005.03742 | paper |
| [SL] | S. Rafler, "Generalization of Conway's 'Game of Life' to a continuous domain — SmoothLife" | arXiv:1111.1567v2 (7 Dec 2011), https://arxiv.org/abs/1111.1567 | paper |
| [SLsf] | Rafler's SmoothLife SourceForge project: readme, blog post on time-stepping modes, FAQ | https://sourceforge.net/projects/smoothlife/ ; https://sourceforge.net/p/smoothlife/blog/2012/03/new-smooth-time-stepping-mode/ ; https://sourceforge.net/p/smoothlife/wiki/fnaq | code GPLv2 (per SourceForge search listing; UNVERIFIED in-file) — do not copy |
| [FL23] | E. Plantec, G. Hamon, M. Etcheverry, P.-Y. Oudeyer, C. Moulin-Frier, B. W.-C. Chan, "Flow-Lenia: Towards open-ended evolution in cellular automata through mass conservation and parameter localization", ALIFE 2023 (Best Paper) | arXiv:2212.07906v2 (24 Mar 2023), https://arxiv.org/abs/2212.07906 | paper |
| [FL25] | Plantec, Hamon, Etcheverry, Chan, Oudeyer, Moulin-Frier, "Flow-Lenia: Emergent evolutionary dynamics in mass conservative continuous cellular automata", *Artificial Life* 31(2):228–248, 2025, doi:10.1162/artl_a_00471 | arXiv:2506.08569v1, https://arxiv.org/abs/2506.08569 | paper |
| [LeniaGH] | Chan's reference code and data: `Python/LeniaND.py`, `LeniaNDK.py`, `LeniaNDKC.py`, `Python/animals.json`, `JavaScript/Lenia-LifeForms.js` | https://github.com/Chakazul/Lenia @ `adfc542939266de7f4bb7ebb552e8499701ee107` (2022-03-15) | **MIT**, "Copyright (c) 2018 Bert Chan" (`LICENSE.md`) |
| [Tut] | "From Conway to Lenia" Colab tutorial (Orbium/Hydrogeminium/multi-channel patterns) | https://github.com/lenia-org/Lenia-Tutorial @ `2642cf995b90f4f37788aba440450098e36aa798` (formerly OpenLenia/Lenia-Tutorial) | **MIT**, "Copyright (c) 2021 Open Science Lenia" |
| [FLGH] | Flow-Lenia JAX reference implementation | https://github.com/erwanplantec/FlowLenia @ `dce428c6b0c5079a06e5606fb7b5ac1fe1323bc5` (2024-02-08) | **No license file** (GitHub API `license: null`) ⇒ all rights reserved; read only, do not copy |
| [Ready] | GollyGang/Ready SmoothLife patterns `Patterns/SmoothLife2011/{smoothglider,smoothlifeL}.vti` | https://github.com/GollyGang/ready @ `7814c952dc93e75b87a08c69bb69b378ccd8ed91` | GPL-3.0; parameters are facts, do not copy code |
| [Moroz] | M. Moroz, "Reintegration tracking" (2020) | https://michaelmoroz.github.io/Reintegration-Tracking/ | cited by [FL23]; not read here |

Local copies of the downloaded data are kept for reproducibility and are not project files:

- `/private/tmp/claude-501/-Users-alejandro-Projects-Conway-GameOfLife/d26793ef-30fe-4239-b1fb-77f42d831735/scratchpad/downloads/continuous-life-papers/`: PDFs and extracted text.
- `/private/tmp/claude-501/-Users-alejandro-Projects-Conway-GameOfLife/d26793ef-30fe-4239-b1fb-77f42d831735/scratchpad/downloads/lenia-animals/`: `animals.json` and `LeniaNDKC.py`.
- `/private/tmp/claude-501/-Users-alejandro-Projects-Conway-GameOfLife/d26793ef-30fe-4239-b1fb-77f42d831735/scratchpad/downloads/lenia-tutorial/`: the tutorial notebook and its LICENSE.
- `/private/tmp/claude-501/-Users-alejandro-Projects-Conway-GameOfLife/d26793ef-30fe-4239-b1fb-77f42d831735/scratchpad/downloads/smoothlife-readme/readme.txt`

---

## 2. Lenia (Chan 2019) — VERIFIED (source) unless noted

### 2.1 Dimensions and resolutions

- **Discrete Lenia (DL)**:
  - State set `S = {0, 1, …, P}`.
  - Neighbourhood: Euclidean ball `N = B_R[0] = {x : ‖x‖₂ ≤ R}`.
  - **Space, time and state resolutions**: `R, T, P ∈ ℤ`.
  - **Steps**: `Δx = 1/R` (site distance), `Δt = 1/T` (time step), `Δp = 1/P` (state precision).
  - Normalized form [L19 eq. 5]: `L = Δx ℤ², 𝒯 = Δt ℤ, S = Δp{0…P}, N = B₁[0]`.
- **Continuous Lenia (CL)** is the hypothesized limit `R, T, P → ∞`: `L = ℝ², 𝒯 = ℝ, S = [0, 1], N = B₁[0]` [eq. 6].
- In practice P is just the float precision. Chan used double precision, and P > 10¹⁵ for f64 [§2.2]. The Python app can also quantize states to P levels: `A ← round(A·P)/P`.

### 2.2 Local rule

1. **Potential** [eq. 7]: `U^t(x) = K ∗ A^t(x) = Σ_{n∈N} K(n) A^t(x+n) Δx²` in DL, or the integral in CL.
2. **Growth** [eq. 8]: `G^t(x) = G(U^t(x))`, with `G : [0, 1] → [−1, 1]`.
3. **Update** [eq. 9]: `A^{t+Δt}(x) = [ A^t(x) + Δt · G^t(x) ]₀¹`. Here `[n]_a^b = min(max(n, a), b)` is the clip, and `Δt = 1/T`.
4. **Summary** [eqs. 15–16]: `A^{t+dt} ↦ [A^t + dt · G_{μ,σ}(K_β ∗ A^t)]₀¹`.
5. **ODE view** [eqs. 19–20]:
   - DL is the forward-Euler discretization of `dA/dt = [G(K ∗ A)]_{−A/dt}^{(1−A)/dt}`.
   - Smaller Δt (larger T) approaches the continuum limit.
   - In Chan's experiments, structural measures (mass, gyradius) decrease and dynamical measures (growth, speed) increase toward asymptotes as T grows [§3.1.2, Fig. 7].
6. Because K ≥ 0 and ΣK = 1, `U ∈ [0, 1]` whenever `A ∈ [0, 1]`.

### 2.3 Kernel = shell ∘ core [eqs. 10–12]

- **Kernel core** `K_C : [0, 1] → [0, 1]`: unimodal, with `K_C(0) = K_C(1) = 0` and usually `K_C(½) = 1`.

  | name | formula | α |
  |---|---|---|
  | exponential ("bump4") | `K_C(r) = exp(α − α / (4r(1−r)))` | α = 4 |
  | polynomial ("quad4") | `K_C(r) = (4r(1−r))^α` | α = 4 |
  | rectangular ("stpz1/4") | `K_C(r) = 1_{[¼, ¾]}(r)` | – |
  | staircase / GoL [eq. 14] | `K_C(r) = 1_{[¼, ¾]}(r) + ½·1_{[0, ¼)}(r)` | – |

  - With α = 4 the exponential core is `exp(4 − 1/(r(1−r)))`, which is the form in the Python code.
  - Tutorial "Gaussian bump" (not in the paper) [Tut]: `K_C(r) = exp(−((r − 0.5)/0.15)²/2)`. It is not zero at r = 0 or r = 1, so it is masked by `r < 1`.
  - Flow-Lenia instead uses sums of Gaussian bumps (§4.3).
- **Kernel shell, rank B, peaks** `β = (β₁ … β_B) ∈ [0, 1]^B` [eq. 11]: `K_S(r; β) = β_{⌊Br⌋} · K_C(Br mod 1)`, with r = ‖n‖₂/R (polar distance in kernel-radius units). This copies the core into B equidistant concentric rings.
  - Code detail [LeniaGH `kernel_shell`]: `(r < 1) · K_C(min(Br mod 1, 1)) · β[min(⌊Br⌋, B − 1)]`.
- **Normalization** [eq. 12]: `K(n) = K_S(‖n‖₂) / |K_S|`, where `|K_S| = Σ_N K_S Δx²`. In code this is `K / sum(K)`, so that ΣK = 1 on the grid. Notes on β [§2.1.4]:
  - `β` with n trailing zeros is equivalent to the same β with R scaled by (B + n)/B.
  - Because of normalization, `β ≡ β / max βᵢ`, so a stored β always has a 1 somewhere.

### 2.4 Growth mapping [eq. 13]

`G : [0, 1] → [−1, 1]` is unimodal and nonmonotonic, with growth centre μ, growth width σ, and `G(μ) = 1`.

| name | formula |
|---|---|
| exponential / gaussian ("gaus") | `G(u) = 2·exp(−(u−μ)²/(2σ²)) − 1` |
| polynomial ("quad4"), α = 4 | `G(u) = 2·1_{[μ±3σ]}(u)·(1 − (u−μ)²/(9σ²))^α − 1` |
| rectangular ("stpz") | `G(u) = 2·1_{[μ±σ]}(u) − 1` |

- **GoL inside Lenia** [eq. 14]: `R = T = P = 1`, staircase core, rectangular growth with `μ = 0.35, σ = 0.07`.
- **SmoothLife resemblance**: Chan notes that SmoothLife is close to Lenia with rectangular core functions [§3.1.3].

### 2.5 Measures (for analysis panels and tests) [L19 §2.4.2]

- **Units**: "mm" = kernel radius (R cells), "s" = unit time (T steps), "mg" = unit state × mm².
- **Formulas**:
  - **Mass**: `m = ∫A dx`. On the grid this is `ΣA / R²`; the Python analyzer uses `mass / R^DIM`.
  - **Volume**: `V = ∫_{A>0} dx`.
  - **Growth**: `g = ∫_{G>0} G dx`.
  - **Centroid**: `x̄_m = ∫xA dx / m`.
  - **Growth centre**: `x̄_g`.
  - **Growth–centroid distance**: `d_gm = |x̄_g − x̄_m|`.
  - **Linear speed**: `s_m = |dx̄_m/dt|`.
  - **Angular speed**: `ω_m = d/dt arg(dx̄_m/dt)`.
  - **Mass asymmetry**: `m_Δ`.
  - **Angular mass**: `I_m = ∫A (x − x̄_m)² dx`.
  - **Gyradius**: `r_m = √(I_m/m)`.
- **Periodic centroids**: on a torus, compute centroids with circular means (per axis, `atan2(Σ A·sin θ, Σ A·cos θ)`). Linear means jump when the creature wraps. Our test code does this.

### 2.6 Reference-implementation facts (LeniaND.py, [LeniaGH]) — VERIFIED (source)

- **Update** (`calc_once`):
  - `A_new = clip(A + dt·G(fftshift(ifft(fft(A)·K̂))), 0, 1)`, with `dt = 1/T`.
  - Optional **soft clip**: `log(1/(e^{kx} + e^{k·0}) + e^{−k·1}) / −k` with `k = 1/dt`.
  - Optional **2-step Adams–Bashforth**: `D = ½(3G_t − G_{t−1})`.
  - Optional **"arita" asymptotic mode** (LeniaNDKC): `D += dt·h·((G+1)/2 − A)`, i.e. the target-state update. The tutorial uses `A ← clip(A + dt·(bell(U; μ, σ) − A))`.
  - **Noise / async masks** exist; these are UI experiments.
- **Function tables are 0-indexed while params are 1-based**. The code calls `kernel_core[kn−1]` and `growth_func[gn−1]`:
  - `kernel_core = {0: polynomial quad4, 1: exponential bump4, 2: step, 3: staircase}`
  - `growth_func = {0: polynomial quad4, 1: gaussian, 2: step}`

  So `kn=1, gn=1` (every `animals.json` entry we inspected) **executes polynomial/polynomial**. The same mapping holds in `LeniaNDK.py` and `LeniaNDKC.py`, and `LeniaF.py` uses 1-based keys with `1 = polynomial`. The UI string table in `LeniaND.py` labels kn=1 "Exponential", which contradicts the code.
  - The older JavaScript app stores functions by name. Its Orbium entry is `"R=13;k=bump4;d=gaus(0.15,0.017)*0.1"`, i.e. exponential core, gaussian growth, μ = 0.15, σ = 0.017, dt = 0.1 (`JavaScript/Lenia-LifeForms.js` line 7).
- **Kernel FFT**: Chan builds the kernel centred in the array, so the code needs an `fftshift` after the inverse FFT. **Better**: build the kernel centred at index (0, 0) with wrap-around (`ifftshift`). Then no shift is needed. This is what our code does.
- **Storage quantization**: `arr2rle` stores `rint(A·255)`, so presets carry 8-bit states (§3.1).

---

## 3. Extended (multi-kernel, multi-channel) Lenia [L20] — VERIFIED (source)

- **Extensions in [L20]**:
  - **d dimensions** (3D/4D species exist in `animals3D.json` and `animals4D.json`).
  - **Multiple kernels** `K_k` with **relative radius** `r_k·R`, their own `β_k, μ_k, σ_k`, and weight `h_k`.
  - **Multiple channels** `A_i`, each kernel having a source channel `i` and a destination channel `j`. Kernel count: `n_k = k_s·c + k_x·c(c−1)`, where k_s is the number of self-kernels per channel and k_x the number of cross kernels per channel pair.
- **Update** [L20 eq. 2]: `A_j^{t+dt} = [ A_j^t + dt · Σ_{i,k} (h_k/h) · G_k(K_k ∗ A_i^t) ]₀¹`, where `h = Σ h_k`.
  - In code (`LeniaNDKC.py calc_once`) the normalizer is **per destination channel**: `D[c1] += dt·h_k·G_k(...)`, `Dn[c1] += h_k`, and `A_new = A + D[c1]/Dn[c1]`.
  - The tutorial instead uses raw `Σ h_k·G_k` without dividing.
  - Make the normalization a per-preset flag.
- **Kernel radius per kernel**: `D = ‖n‖ / (R·r_k)` and `K = (D < 1)·β[⌊B·D⌋]·core(B·D mod 1)`, each kernel normalized to sum 1.
- **Genotype size**: for a 3D, 3-channel, 3-self-kernel world, a genotype is `(r, h, β³, μ, σ)¹⁵` = 105 numbers.
- **Search methods in [L20]**: global random search, depth-first mutation, genetic algorithm, and an interactive user interface. They fit Emergence Lab's evolution lab.

### 3.1 Gaussian-bump kernels (Hamon et al. 2022/2024, used by Flow-Lenia) — VERIFIED (source)

[FL23 eq. 1] / [FL25 eq. 1]:

```
K_i(x) = Σ_{j=1..k} b_{i,j} · exp( −( |x|/(r_i·R) − a_{i,j} )² / (2·w_{i,j}²) )
```

There are k = 3 rings per kernel, giving `3k + 1` parameters per kernel (a, b, w, r), with R shared. Each kernel is normalized to ∫K = 1.

[FLGH] code differs from the paper text. These are observations, UNVERIFIED as to intent:

- **Ring width**: the code uses `exp(−(D − a)²/w)` (not `2w²`).
- **Radius scale**: the code uses `D = |x| / ((R + 15)·r)`.
- **Soft cutoff**: the code multiplies by a sigmoid `sigmoid(−10(D − 1))`.

Match whichever source a preset came from.

---

## 4. Presets with exact parameters and cells

### 4.1 Cell-data format used by Chan (`animals.json`) — VERIFIED (source, re-implemented independently)

Pattern RLE, row-major:

- **Run count**: an optional decimal count precedes a symbol.
- **Symbols**:
  - `.` or `b` = 0
  - `o` = 255
  - `A`…`X` = 1…24
  - two-character `p`…`y` followed by `A`…`X` = `(c₀−'p')·24 + (c₁−'A') + 25`, so `yO` = 255
- **Delimiters**: `$` ends a row (a count before `$` appends count−1 empty rows). `!` ends the pattern. 3D uses `%` between planes and 4D uses `#`.
- **Padding**: rows are right-padded with zeros to the maximum width.
- **State value**: `value/255`.

Our decoder reproduces the decoded shapes (Orbium: 20×20, Σ = 76.863 after /255).

### 4.2 Orbium unicaudatus (code O2u) — the reference glider

Published parameter sets (all VERIFIED (source)):

| source | R | T | β | μ | σ | core / growth |
|---|---|---|---|---|---|---|
| `animals.json` entry 4 [LeniaGH] | 13 | 10 | [1] | 0.15 | 0.015 | `kn=1, gn=1` → **polynomial / polynomial** as executed by LeniaND.py (§2.6) |
| JS `Lenia-LifeForms.js` line 7 [LeniaGH] | 13 | 10 (`*0.1`) | [1] | 0.15 | **0.017** | bump4 (exponential) / gaus |
| paper figures 6–7 [L19] | 13 (Fig. 7 sweeps R ∈ 9…55; Fig. 6 uses R=185) | 10 | [1] | 0.15 | **0.016** | exponential / exponential |
| tutorial `pattern["orbium"]` [Tut] | 13 | 10 | [1] | 0.15 | 0.015 | **Gaussian-bump kernel** `bell(r, 0.5, 0.15)` / gaussian growth |

**VERIFIED (repro)**: all of these combinations sustain Orbium. We ran a 128² torus for 2000 steps with the pattern centred. Results:

| configuration | mass ΣA, steps 500–1000 | speed, steps 500–1000 (cells/step) |
|---|---|---|
| tutorial (bell kernel, gauss growth, σ = 0.015) | 72.4–73.3 | 0.602 |
| animals.json cells, exponential/gaussian, σ = 0.015 | ≈ 71.1 | 0.615 |
| animals.json cells, polynomial/polynomial | ≈ 73.6 | 0.623 |
| animals.json cells, exponential/gaussian, σ = 0.016 | 73.3–73.9 | 0.627 |
| animals.json cells, exponential/gaussian, σ = 0.017 | ≈ 75.8 | 0.642 |

In Chan's units, 0.60 cells/step × T/R = 0.46 mm/s.

**Recommendation**: ship the tutorial configuration as the canonical "Orbium" test fixture (fully specified, MIT). Ship the `animals.json` cells as a second preset with polynomial/polynomial.

**Orbium cells, tutorial version** (20×20, row-major, values in [0, 1]).

Source: [Tut] `Tutorial_From_Conway_to_Lenia_(w_o_results).ipynb`, cell `pattern["orbium"]`, MIT License, Copyright (c) 2021 Open Science Lenia. Keep the MIT notice with any file that embeds this.

```json
[[0,0,0,0,0,0,0.1,0.14,0.1,0,0,0.03,0.03,0,0,0.3,0,0,0,0],
 [0,0,0,0,0,0.08,0.24,0.3,0.3,0.18,0.14,0.15,0.16,0.15,0.09,0.2,0,0,0,0],
 [0,0,0,0,0,0.15,0.34,0.44,0.46,0.38,0.18,0.14,0.11,0.13,0.19,0.18,0.45,0,0,0],
 [0,0,0,0,0.06,0.13,0.39,0.5,0.5,0.37,0.06,0,0,0,0.02,0.16,0.68,0,0,0],
 [0,0,0,0.11,0.17,0.17,0.33,0.4,0.38,0.28,0.14,0,0,0,0,0,0.18,0.42,0,0],
 [0,0,0.09,0.18,0.13,0.06,0.08,0.26,0.32,0.32,0.27,0,0,0,0,0,0,0.82,0,0],
 [0.27,0,0.16,0.12,0,0,0,0.25,0.38,0.44,0.45,0.34,0,0,0,0,0,0.22,0.17,0],
 [0,0.07,0.2,0.02,0,0,0,0.31,0.48,0.57,0.6,0.57,0,0,0,0,0,0,0.49,0],
 [0,0.59,0.19,0,0,0,0,0.2,0.57,0.69,0.76,0.76,0.49,0,0,0,0,0,0.36,0],
 [0,0.58,0.19,0,0,0,0,0,0.67,0.83,0.9,0.92,0.87,0.12,0,0,0,0,0.22,0.07],
 [0,0,0.46,0,0,0,0,0,0.7,0.93,1,1,1,0.61,0,0,0,0,0.18,0.11],
 [0,0,0.82,0,0,0,0,0,0.47,1,1,0.98,1,0.96,0.27,0,0,0,0.19,0.1],
 [0,0,0.46,0,0,0,0,0,0.25,1,1,0.84,0.92,0.97,0.54,0.14,0.04,0.1,0.21,0.05],
 [0,0,0,0.4,0,0,0,0,0.09,0.8,1,0.82,0.8,0.85,0.63,0.31,0.18,0.19,0.2,0.01],
 [0,0,0,0.36,0.1,0,0,0,0.05,0.54,0.86,0.79,0.74,0.72,0.6,0.39,0.28,0.24,0.13,0],
 [0,0,0,0.01,0.3,0.07,0,0,0.08,0.36,0.64,0.7,0.64,0.6,0.51,0.39,0.29,0.19,0.04,0],
 [0,0,0,0,0.1,0.24,0.14,0.1,0.15,0.29,0.45,0.53,0.52,0.46,0.4,0.31,0.21,0.08,0,0],
 [0,0,0,0,0,0.08,0.21,0.21,0.22,0.29,0.36,0.39,0.37,0.33,0.26,0.18,0.09,0,0,0],
 [0,0,0,0,0,0,0.03,0.13,0.19,0.22,0.24,0.24,0.23,0.18,0.13,0.05,0,0,0,0],
 [0,0,0,0,0,0,0,0,0.02,0.06,0.08,0.09,0.07,0.05,0.01,0,0,0,0,0]]
```

**Orbium cells, `animals.json` version** (20×20, 8-bit). Source: [LeniaGH] `Python/animals.json` index 4, MIT License, Copyright (c) 2018 Bert Chan.

```
7.MD6.qL$6.pKqEqFURpApBRAqQ$5.VqTrSsBrOpXpWpTpWpUpCrQ$4.CQrQsTsWsApITNPpGqGvL$3.IpIpWrOsGsBqXpJ4.LsFrL$A.DpKpSpJpDqOqUqSqE5.ExD$qL.pBpTT2.qCrGrVrWqM5.sTpP$.pGpWpD3.qUsMtItQtJ6.tL$.uFqGH3.pXtOuR2vFsK5.sM$.tUqL4.GuNwAwVxBwNpC4.qXpA$2.uH5.vBxGyEyMyHtW4.qIpL$2.wV5.tIyG3yOxQqW2.FqHpJ$2.tUS4.rM2yOyJyOyHtVpPMpFqNV$2.HsR4.pUxAyOxLxDxEuVrMqBqGqKJ$3.sLpE3.pEuNxHwRwGvUuLsHrCqTpR$3.TrMS2.pFsLvDvPvEuPtNsGrGqIP$4.pRqRpNpFpTrNtGtVtStGsMrNqNpF$5.pMqKqLqRrIsCsLsIrTrFqJpHE$6.RpSqJqPqVqWqRqKpRXE$8.OpBpIpJpFTK!
```

### 4.3 Scutium (shield) — VERIFIED (source); survival VERIFIED (repro)

- **Scutium solidus** (`S1s`, `animals.json` index 28): R=13, T=10, β=[1], μ=0.29, σ=0.045, kn=gn=1. Cells are 20×18. It survived under both function sets: at 2000 steps the mass was 142.4 (exponential/gaussian) and 139.5 (polynomial/polynomial), with speed 0.40 and 0.36 cells/step respectively. The paper identifies Scutium as the Lenia rediscovery of SmoothLife's "smooth glider" [L19 §3.2.1]. Cells (MIT, Bert Chan):

  ```
  5.pGQ$6.sUsDqRR$4.VpXrJwKvNtXrW$2.ApVrGrWsIwKyOyDwTuNO$2.qArOrIqIpPpTxH2yOyIvKqG$rVpIrNrJpH4.xP3yOvFqWA$sKvNsKqE5.pI4yOuHqPC$sNxBuMpD5.JuN3yOwXsUpN$sGxFyOT5.pCtIyF3yOuKqUH$rPwXyOxT5.qKtSxS3yOvIrQT$pQwK2yOtWXJXqHsGuWxW3yOvPsApC$.vQ3yOuVsLsGsXuMwOyK3yOvHrVpC$.rKyK3yOxEwBwCwXyE3yOxQuMrJU$.WvC5yOyM5yOwFtJqOL$2.rVwO9yOwWuMsApOC$2.pLtBwK6yOyJwQuTsQqLP$3.qHsWvEwUxTyCxRwWvRuGsNqSpA$3.HqBrUtHuGuPuLtVsXrSqIXB$4.DpDqEqW2rIqWqGpJN$6.ENTUPHA!
  ```

- **Scutium valvatus** (`S1v`, index 29): R=13, μ=0.283, σ=0.0461. It **dies under exponential/gaussian and survives under polynomial/polynomial** in our test. This is the clearest single example for §2.6.
- **Multi-ring Scutium serratus**: `2S1*`, indices 30–36, with β such as `1/2,1`, `1/6,1/2,1`, `1/12,1/6,1/2,1`.

### 4.4 Geminium family (rank-3 "twins"), incl. Hydrogeminium — VERIFIED (source)

From `animals.json`:

| idx | code | name | R | β | μ | σ |
|---|---|---|---|---|---|---|
| 231 | 3GH2s | Hydrogeminium saliens | 27 | 11/12, 11/12, 1 | 0.28 | 0.042 |
| 232 | 3GH2l | Hydrogeminium limus | 18 | 1, 1, 1 | 0.26 | 0.036 |
| 233 | 3GH2n | **Hydrogeminium natans** | 18 | 1/2, 1, 2/3 | 0.26 | 0.036 |
| 234 | 3GH2l | Hydrogeminium labens | 18 | 1/2, 1, 2/3 | 0.31 | 0.053 |
| 225–230 | 2GA2/3GA2 | Aerogeminium volitans/ambiguus/immotus/vagus/repens | 18–21 | various | 0.195–0.36 | 0.0198–0.072 |
| 235–242 | 3GT2 | Terrageminium (several) | 27 | e.g. 1, 2/3, 7/12 | 0.26–0.32 | 0.026–0.049 |
| 243–246 | 2GG2/3GG2 | Gyrogeminium gyrans/revolvens/serratus | 18 | e.g. 1, 1, 1 | 0.25–0.32 | 0.031–0.05 |

All entries have T=10 and kn=gn=1.

- The tutorial's `pattern["geminium"]` is "Hydrogeminium" with R=18, T=10, μ=0.26, σ=0.036, `b=[0.5,1,0.667]` (51×55 cells, MIT, Open Science Lenia), i.e. the natans parameters with 2/3 rounded to 0.667.
- **VERIFIED (repro)**: Hydrogeminium natans survives 2000 steps on a 256² grid under both function sets, with mass 689 → 714 (exp/gauss) and 689 → 699 (poly/poly) and speed about 0.39–0.42 cells/step.
- Hydrogeminium limus grew without bound in both of our runs (the paper notes a mirrored limus was "metastable"; UNVERIFIED whether our setup matches).
- Use index 233 as the shipped "Hydrogeminium" preset. Its RLE is in `animals.json`; copy it with the MIT notice.

### 4.5 Multi-channel presets (tutorial; MIT) — parameters VERIFIED (source)

- **"K=3 Fish"**:
  - R=10, T=5, one channel, three kernels `{b, m, s, h, c0, c1}`:
    - `{[1,5/12,2/3], 0.156, 0.0118, 1, 0, 0}`
    - `{[1/12,1], 0.193, 0.049, 1, 0, 0}`
    - `{[1], 0.342, 0.0891, 1, 0, 0}`
  - The growths are averaged (`np.mean`).
- **"Tessellatium gyrans"** (`pattern["aquarium"]`):
  - R=12, T=2, three channels, 15 kernels, each `{b, m, s, h, r, c0, c1}`.
  - Example: kernel 0 is `{b:[1], m:0.272, s:0.0595, h:0.138, r:0.91, c0:0, c1:0}`; the full list is in the notebook.
  - Growth is `Σ h·G` per destination channel without normalization.
- Also in the notebook: "Smooth glider gun" (`emitter`, three channels, target-function update in channel 2), "Pacman" (soft clip), and "Smooth wanderer" (asymptotic/target update, R=13, T=10, μ=0.1, σ=0.005, β=[3/4,1]).

**Survey evidence for §2.6 (VERIFIED (repro))**: we took all `animals.json` 2D species with kn=gn=1, R ≤ 20 and cells ≤ 80 px (239 species), ran 600 steps on 128², and counted survival as final mass within [0.3, 3]× initial.

- exponential/gaussian: **181/239** survive.
- polynomial/polynomial: **227/239** survive.
- 46 species disagree, and every one of them survives only under polynomial/polynomial. Examples: Orbium unicaudatus ignis, Scutium valvatus, Paraptera serratus arcus saliens, Heptageminium natans.

---

## 5. SmoothLife (Rafler 2011)

### 5.1 Model — VERIFIED (source, [SL] eqs. 1–6)

- **State**: `f(x, t) ∈ [0, 1]` on ℝ², simulated as a float array.
- **Inner filling** (cell, disk of radius rᵢ): `m = (1/M) ∫_{|u| < rᵢ} f(x + u, t) du`.
- **Outer filling** (annulus rᵢ < |u| < rₐ): `n = (1/N) ∫_{rᵢ < |u| < rₐ} f(x + u, t) du`. M and N are the disk and annulus areas, so m, n ∈ [0, 1].
- **Radius ratio**: typically `rₐ = 3rᵢ`, chosen so the neighbourhood is three cell-diameters across, as in GoL.
- **Anti-aliasing**: rim width b, typically b = 1. With `l = |u|`:
  - weight 1 if `l < r − b/2`
  - weight 0 if `l > r + b/2`
  - weight `(r + b/2 − l)/b` in between

  This applies to the inner disk edge and to both annulus rims. In the discrete normalization, M and N are the sums of these weights. For rₐ = 21, rᵢ = 7, b = 1 we get M = 154.55 and N = 1231.23 (π·49 = 153.94; π·392 = 1231.50), VERIFIED (repro).
- **Sigmoids** (smooth steps, width α):
  - `σ₁(x, a) = 1/(1 + exp(−(x − a)·4/α))` (eq. 3)
  - `σ₂(x, a, b) = σ₁(x, a)·(1 − σ₁(x, b))` (eq. 4)
  - `σ_m(x, y, m) = x·(1 − σ₁(m, 0.5)) + y·σ₁(m, 0.5)` (eq. 5)
- **Transition**: `s(n, m) = σ₂(n, σ_m(b₁, d₁, m), σ_m(b₂, d₂, m))` (eq. 6).
  - Birth interval `[b₁, b₂]`, death (survival) interval `[d₁, d₂]`.
  - σ₂ uses `α_n` and σ_m uses `α_m`.
  - So the living-ness `σ₁(m, 0.5)` interpolates the interval *bounds*.
- **Variant in [Ready] `smoothlifeL.vti`**: `s = σ_m(σ₂(n, b₁, b₂), σ₂(n, d₁, d₂), m)`, which interpolates the two interval *indicators*. This is probably Rafler's `mixtype`/`sigmode` option; his FAQ names `mixmode`, `sigmode` and `sigtype` without defining them (UNVERIFIED). Implement both as an enum.
- **Paper's smooth glider (Fig. 1 caption), VERIFIED (source)**: `rₐ = 21, b₁ = 0.278, b₂ = 0.365, d₁ = 0.267, d₂ = 0.445, α_n = 0.028, α_m = 0.147`, implied `rᵢ = 7`, discrete time stepping.

### 5.2 Time stepping

- **[SL] §4** (VERIFIED (source)):
  - **Discrete**: `f(x, t+1) = S[s(n, m)] f(x, t)` (eq. 7), i.e. the new value is `s(n, m)`.
  - **Smooth**: `f(x, t+dt) = f(x, t) + dt·S[s(n, m)] f(x, t)` with `s = 2s_d − 1 ∈ [−1, 1]` (eq. 8). This is the forward-Euler scheme of `∂_t f = S[s(n, m)] f` (eq. 9).
  - **Different life**: smooth stepping changes the dynamics. The discrete-mode gliders are not found at the same birth/death values, but gliders, oscillators and stable structures still exist.
- **Rafler's blog** (2012-03-11, [SLsf], VERIFIED (source)):
  - **mode 0**: discrete.
  - **mode 1**: `f ← f + dt·(2s − 1)`.
  - **mode 2**: `f ← (1 − c)·f + c·s`, which → `∂_t f = s − f` as c → 0 and recovers the discrete mode at c = 1.
  - The readme describes mode 0 as discrete and modes 1 and 2 as smooth, and says dt is ignored in discrete mode.
- **Modes 3 and 4**: a third-party port cites `SmoothLifeAll/SmoothLifeSDL/shaders/snm2D.frag` as defining five modes. UNVERIFIED against the original shader; source: https://gist.github.com/hamilton/60ba4ecacda9bcce6ae1cce2b3678585 (no license stated).

  | mode | update (then clip to [0, 1]) |
  |---|---|
  | 0 | `f ← s(n, m)` |
  | 1 | `f ← f + dt·(2s − 1)` |
  | 2 | `f ← f + dt·(s − f)` |
  | 3 | `f ← m + dt·(2s − 1)` |
  | 4 | `f ← m + dt·(s − m)` |

  Modes 3 and 4 integrate from the inner filling m instead of the point value f.
- **Other integrators**: Rafler's later releases compared Euler, improved Euler, Adams–Bashforth (3rd order) and RK4 (readme, SmoothLifeMatlab). Optional for us.

### 5.3 Other published parameter sets (secondary sources)

| name | rₐ | rₐ/rᵢ | b₁, b₂ | d₁, d₂ | α_n, α_m | stepping | source |
|---|---|---|---|---|---|---|---|
| smooth glider (paper) | 21 | 3 | 0.278, 0.365 | 0.267, 0.445 | 0.028, 0.147 | discrete | [SL] Fig. 1 (primary) |
| smoothglider (Ready) | 12 | 3 | same | same | same | discrete | [Ready] `smoothglider.vti` (init: white noise in [0, 0.75], "very sensitive") |
| SmoothLifeL (Ready) | 10 | 3 | 0.257, 0.336 | 0.365, 0.549 | 0.028, 0.147 | mode 1, dt = 0.1, alternative mixing (§5.1) | [Ready] `smoothlifeL.vti` |
| SmoothLifeL (UMBC page) | 20 | 3 | 0.257, 0.336 | 0.365, 0.549 | 0.028, 0.147 | Euler, dt = 0.05 | https://my3.my.umbc.edu/groups/csee/media/4227 (secondary; UNVERIFIED) |

### 5.4 Our reproduction of the paper glider — VERIFIED (repro)

- **Setup**: numpy FFT, 256² torus, discrete mode 0, eqs. 1–6 with b = 1 anti-aliasing. The initial state is random filled disks (radius U(7, 21), about N²/(2rₐ²) of them, seed 5).
- **Result**: after 100 steps one component converged to a steadily translating crescent, the smooth glider. Isolated on an empty torus and run for 1500 more steps:
  - mass Σf = **799.4 ± 1.5**
  - speed **5.281 ± 0.008 cells/step** (≈ 0.251 rₐ per step), constant over steps 200–1500
- Sparse initial disks die out, and denser soups produce gliders plus irregular blobs.
- The isolated glider state is saved at `/private/tmp/claude-501/-Users-alejandro-Projects-Conway-GameOfLife/d26793ef-30fe-4239-b1fb-77f42d831735/scratchpad/smoothglider_ra21_state.npy` (256² f64). Regenerate it from the project's own engine as a fixture rather than vendoring this file.

### 5.5 Relation to Lenia

SmoothLife is two-kernel Lenia:

- **Kernels**: a disk kernel `K_m` and an annulus kernel `K_n`, both rectangular cores with anti-aliased rims.
- **Transition**: a 2D function `s(n, m)` instead of a 1D growth G(u).
- **Update**: mode 1 is exactly Lenia's `A + dt·(2s − 1)` with clip.

The engine should accept a "transition over (U₁, U₂)" variant, which also covers Flow-Lenia's two-gradient flow.

---

## 6. Flow-Lenia [FL23, FL25] — VERIFIED (source) unless noted

### 6.1 Lenia part

- **Kernels and growth**: Gaussian-bump kernels [eq. 1]; gaussian growth `G_i(x) = 2·exp(−(μᵢ − x)²/(2σᵢ²)) − 1` [eq. 2].
- **Connectivity**: an adjacency matrix `M_{ij}` gives the number of kernels from channel i to channel j.
- **Affinity map** (Lenia's "growth" renamed) [eq. 3]: `U_j^t = Σ_i h_i·G_i(K_i ∗ A_{c0_i}^t)·[c1_i = j]`.
- **Plain Lenia** would continue with `A_i^{t+dt} = [A_i^t + dt·U_i^t]₀¹` [eq. 4].

### 6.2 Flow and mass conservation [eq. 5]

```
F_i^t = (1 − α^t)·∇U_i^t − α^t·∇A_Σ^t
α^t(p) = [ (A_Σ^t(p) / θ_A)^n ]₀¹,      A_Σ = Σ_i A_i
```

- **Critical mass** θ_A ∈ ℝ_{>0} (renamed β_A in [FL25]); typically n > 1.
- **Interpretation**: near the critical mass the concentration gradient `−∇A_Σ` dominates and acts as diffusion, replacing Lenia's clip. Elsewhere matter climbs the affinity gradient.
- **Gradients**: estimated with Sobel filters.
- **States**: unbounded, `S = ℝ_{≥0}^C`. Total mass per channel is conserved: `Σ_x A_c^t = Σ_x A_c^{t+dt}`.

### 6.3 Reintegration tracking (RT) [eqs. 6–7]

```
A_i^{t+dt}(p) = Σ_{p'} A_i^t(p') · I_i(p', p)
I_i(p', p)    = ∫_{Ω(p)} D(p''_i, s),     p''_i = p' + dt·F_i^t(p')
```

- **Domains**: Ω(p) is the unit square of cell p.
- **Distribution**: D(m, s) is a uniform square of side **2s** centred at m, integrating to 1, so each source sends out exactly its mass.
- **Temperature**: s is the "temperature" hyperparameter.
- **Gather window**: for efficiency, [FL25] considers only source cells whose Chebyshev distance to the target is "less than 5". The reference code loops over offsets in `[−dd, dd]²` with dd = 5, i.e. distance ≤ 5 and an 11 × 11 window.
- **Displacement clip**: the code clips the displacement `dt·F` to `±(dd − s)` per axis so mass never leaves the window. This is what makes the gather exactly conservative.
- **Borders**: the code offers `"wall"` (target centres clipped to `[s, N − s]`) and `"torus"`.

### 6.4 Parameters [FL23/FL25 Table 1]

- **Neighbourhood**: R ∈ [2, 25]; r ∈ [0.2, 1] per kernel.
- **Kernel weights and rings**: h ∈ [0, 1]; a ∈ [0, 1]³; b ∈ [0, 1]³; w ∈ [0.01, 0.5]³.
- **Growth**: μ ∈ [0.05, 0.5]; σ ∈ [0.001, 0.2].
- **Flow**: **s = 0.65, n = 2, dt = 0.2**.
- **Initial state**: a 40 × 40 patch of uniform random matter.
- **Reported performance**: 255 µs ± 3.11 µs per step on a Tesla T4 (JAX) with 1 channel, 10 kernels and 128².
- **Searched worlds**:
  - Multi-species runs: 1024² initialized with 144 creatures for 200k steps [FL23].
  - Evolutionary-activity experiments: 3 channels with 5 kernels per channel pair, i.e. 45 kernels [FL25].

### 6.5 Parameter localization (multi-species)

- **Parameter map**: `P : L → Θ`. In [FL25] only the kernel weights h are embedded, `Θ ≡ ℝ^{|K|}`. Then `U_j^t(x) = Σ_i P_i^t(x)·G_i(K_i ∗ A_{c0_i}^t)(x)·[c1_i = j]` [FL25 eq. 7]. Kernels stay global, so FFT still applies.
- **Transport**: parameters move with matter during RT. Mixing rules:
  - **Average** [FL23 eq. 9]: `P^{t+dt}(p) = Σ A(p')I(p', p)P(p') / Σ A(p')I(p', p)`.
  - **Softmax sampling** [FL23 eq. 10; FL25 eq. 8, the default in [FL25]]: `Pr[P^{t+dt}(x) = P^t(x')] = exp(A(x')I(x', x)) / Σ_{x''} exp(A(x'')I(x'', x))`. A deterministic argmax variant is also used.
- **Mutations**: Gaussian noise added to P in random zones. In [FL25], mutations are 10 × 10 "beams".
- **Food** [FL23 eq. 11]:
  - matter decays at `ρ_decay`
  - a food map Ψ is converted to matter at `ρ_digest`: `A ← … + [A·ρ_digest]_0^Ψ − A·ρ_decay` and `Ψ ← Ψ − [A·ρ_digest]_0^Ψ`
  - kernels from Ψ let creatures sense food

### 6.6 Reference-code conventions that matter for reproduction ([FLGH], read only, no license)

- **Sobel**: unnormalized 3 × 3 (`[[1,0,−1],[2,0,−2],[1,0,−1]]`), about 8× the central-difference derivative. It is applied with `convolve2d(..., mode='same')`, i.e. **zero padding even when `border="torus"`**.
- **α**: the code computes `clip((A_i / C)², 0, 1)` per channel, i.e. θ_A = C and n = 2. The paper defines α on A_Σ; the two agree only for C = 1.
- **Kernel geometry**: see §3.1.
- **Stochastic mixing**: uses a fixed `PRNGKey(42)` inside the RT step.

### 6.7 Our implementation findings — VERIFIED (repro)

- **Scatter form**: RT is best written as a scatter on the CPU. With s = 0.65 the square has side 1.3, so each source touches at most 3 × 3 cells, instead of the 121-offset gather.
  - Per axis, the overlap of `[c − s, c + s]` with cell `[t, t+1]` is `max(0, min(t+1, c+s) − max(t, c−s))`, and the area is the product of the two axes over (2s)².
  - **Unit test**: with F = 0 and s = 0.65, the per-axis weights are exactly [0.11538, 0.76923, 0.11538] (= [0.15, 1.0, 0.15]/1.3). With F = 0 and s = 0.5, RT is the identity.
- **f32 pitfall**: if overlaps are computed in absolute grid coordinates, f32 rounding of positions up to about 256 leaks about −7 × 10⁻⁶ of total mass per step (we measured −3.3 × 10⁻³ after 500 steps).
  - **Fix**: compute weights in source-local coordinates and renormalize each axis' weights to sum to 1. Drift then fell to about −2 × 10⁻⁸ per step (−1.1 × 10⁻⁵ after 500 steps, i.e. f32 accumulation noise).
  - For exact conservation, accumulate in f64 or rescale by `M₀/M` once per step and record the correction as a diagnostic.
- **Cost**: a 256², 1-kernel Flow-Lenia step (FFT + growth + 2 Sobels + flow + scatter RT) takes **1.02 ms** single-threaded on the M2 Max, of which RT is 0.48 ms.

---

## 7. Implementation strategy in Rust

### 7.1 Crate placement and data model (proposal)

- **Crate**: the workspace already has a `crates/continuous` skeleton (package `continuous`; dependencies `simulation-core`, `serde`, `rayon`, `rustfft` with workspace `rustfft = "6.4"`; no `realfft` yet). Add `realfft = "3.5"` to the workspace if the r2c path is adopted. No Bevy dependency. Seeded RNG via `rand_chacha 0.10`. Fixed ticks. CLI-runnable. Presets in RON or TOML with a `provenance { source_url, commit, license, copyright }` block.
- **Types**:
  - `enum KernelCore { Exponential { alpha: f32 }, Polynomial { alpha: f32 }, Rectangular, Staircase, GaussianBump { mu: f32, sigma: f32 }, Rings { a: [f32; 3], b: [f32; 3], w: [f32; 3] } }`
  - `enum Growth { Gaussian, Polynomial { alpha: f32 }, Rectangular, Target /* asymptotic */ }`
  - `struct KernelSpec { r_rel: f32, beta: SmallVec<f32>, core: KernelCore, mu: f32, sigma: f32, h: f32, src: u8, dst: u8 }`
  - `struct LeniaRule { r: f32 /*R*/, t: f32 /*T, dt = 1/T*/, kernels: Vec<KernelSpec>, h_norm: HNorm /* None | PerDst */, clip: Clip /* Hard | Soft{k} */, integrator: Euler | AB2, quantize_p: Option<u32> }`
  - `struct SmoothLifeRule { ra, ratio, aa_width: b, b1, b2, d1, d2, alpha_n, alpha_m, mix: Bounds | Indicators, mode: 0..=4, dt }`
  - `struct FlowRule { lenia: LeniaRule, theta_a, n, s /*temperature*/, dd: u8, border: Torus | Wall, mixing: Avg | Softmax | Argmax }`
- **Field storage**: `Vec<f32>` row-major, `width × height`, one Vec per channel. Use 64-byte alignment if SIMD helps.

### 7.2 FFT convolution

- **Crates** (checked with `cargo info` on 2026-11-09):

  | crate | version | license | MSRV | notes |
  |---|---|---|---|---|
  | `rustfft` | 6.4.1 (published 2025-09-18) | MIT OR Apache-2.0 | 1.61 | features `avx`, `sse`, `neon` (default on); `FftPlanner` picks the NEON planner at runtime on aarch64 |
  | `realfft` | 3.5.1 (published 2026-10-26; previous 3.5.0, 2025-06-12) | **MIT** (only) | 1.61 | requires `rustfft ^6.4.0`; r2c gives N/2+1 complex values; c2r **zeroes** non-zero imaginary parts of the DC/Nyquist bins and returns `FftError::InputValues` (the transform still runs) |

  - **Transitive dependencies** (`transpose 0.2.3`, `strength_reduce 0.2.4`, `primal-check 0.3.4`, `num-complex 0.4.6`, `num-traits 0.2.19`, `num-integer 0.1.47`) are all MIT OR Apache-2.0.
  - **Licence compatibility**: realfft's MIT-only licence is compatible with an MIT OR Apache-2.0 project; keep its notice in the third-party licence bundle.
- **2D r2c layout** (what we benchmarked):
  1. For each of the N rows, r2c (N reals → W = N/2+1 complex) into `S[N×W]`.
  2. Transpose to `Sᵀ[W×N]`.
  3. Run a complex FFT of length N on each of the W rows.
  4. Multiply by the precomputed kernel spectrum, stored in the same transposed layout and pre-scaled by 1/N².
  5. Run the inverse complex FFT on the W rows.
  6. Transpose back.
  7. For each row, zero `im` of bin 0 (and bin N/2 when N is even), then c2r.
- **Cost**: this is about half the work of a full complex 2D FFT. We measured c2c at about 1.9–6× slower than rayon realfft (§7.6).
- **Kernel spectrum**: build each kernel centred at (0, 0) with wrap-around, normalize in f64, then FFT it once. The spectrum is real up to rounding for a radially symmetric kernel, so storing only the real part halves the multiply bandwidth (optional).
- **Many kernels**:
  - Forward-FFT each *source channel* once.
  - Then per kernel do one complex multiply plus one inverse FFT. Cost ≈ `(C + K)` 2D FFTs + `K·N²` multiply-adds.
  - Two real outputs from the same input can share one inverse c2c via `IFFT(F·(K₁ + i·K₂)) = U₁ + i·U₂`, which is exact by linearity. This needs the full complex spectrum, so it only pays off in a c2c pipeline; SmoothLife's m and n are the canonical use.
- **Sizes**: rustfft handles any N (Bluestein/Rader for awkward primes), but prefer `2^a·3^b·5^c` sizes; we only benchmarked powers of two. Require `N ≥ 2R + 1` (for multi-kernel `2·max(r_k)·R + 1`), or the kernel wraps onto itself. In practice use N ≥ 4R so creatures do not self-interact across the torus.
- **Boundaries**:
  - **Periodic**: the FFT gives this for free [L19 §2.2].
  - **Zero or wall boundaries**: zero-pad the field to at least `N + 2R` (linear convolution) and mask.
  - **Flow-Lenia walls**: RT clips target centres; separately pick Sobel padding (see §6.6).

### 7.3 Direct convolution

- **Cost**: `O(N² · T_taps)` with `T_taps ≈ πR²`. R = 13 gives 517 non-zero taps for the bell kernel (516 for the exponential core, which is 0 at r = 0).
- **Implementation**: loop over taps, and for each tap add a scaled, shifted row slice from a wrap-padded copy. This vectorizes well.
- **Measured**: 2.27 ms at 256² single-threaded versus 0.23 ms for realfft, about 10×.
- **Crossover**: with FFT cost roughly constant in R, direct only wins for very small kernels (UNVERIFIED estimate: R ≲ 4).
- **Still useful for**: (a) a test oracle against FFT; (b) spatially varying kernels, where FFT is impossible [FL25 §3.1]; (c) GPU shared-memory tiling for moderate R.

### 7.4 Precision

- **f32 is sufficient for simulation.**
  - Our Rust f32 FFT Orbium agrees with numpy f64 on total mass to ±0.0003 for 500 steps: 73.5506 / 73.0977 / 72.9739 / 72.9989 at steps 10, 100, 200 and 500, against f64's 73.5506 / 73.0977 / 72.9739 / 72.9991.
  - By step 1000 the two have diverged slightly (72.77 vs 73.17; displacement (560.3, 221.5) vs (560.0, 222.2)) because the dynamics are chaotic. Both remain healthy Orbium.
  - Chan's GPU path used complex64 (`reikna`).
- **Use f64 for**: kernel normalization sums, mass/centroid statistics, Flow-Lenia mass bookkeeping, and the analysis/plot pipeline.
- **Value range**: Lenia states stay in [0, 1], where f32 has about 7 decimal digits. Flow-Lenia states are unbounded but typically ≤ 2 (max A ≈ 1.5 in our run).

### 7.5 Parallelism (rayon 1.12)

- **Measured trap**: per-row tasks (`par_chunks_mut(N)` with `for_each_init` per row) were **slower than single-threaded** at 256² and 512² (0.7 ms vs 0.24 ms conv at 256²).
- **What works**:
  - Group rows into blocks of `rows / (2·threads)` per task.
  - Allocate FFT scratch once per task.
  - Parallelize the transposes with 32-row tiles.
  - With that, rayon breaks even at 256² and gives about 2.4× at 512², 3.6× at 1024² and 4.7× at 2048². This is on 8 P-cores + 4 E-cores; rayon uses 12 threads.
- **Small grids**: for 256² and below, run single-threaded or parallelize across *worlds* (batch evolution) instead of within one world.
- **Determinism**: FFT rows are independent, so results are bitwise identical regardless of thread count. Keep reductions (mass, centroid) in a fixed order: per-row partial sums, then a sequential sum.
- **Cross-platform**: x86 AVX and aarch64 NEON paths may differ in the last ulp. With chaotic dynamics, cross-platform golden tests must therefore compare statistics with tolerances, not hashes.

### 7.6 Measured cost per step on this machine

Setup: Apple M2 Max (8P + 4E cores), Rust 1.98.1, `--release` (thin LTO, codegen-units 1), f32, single-kernel Lenia with a gaussian growth step and clip. Each value is the best of 5 × k iterations, and the minimum over 3 runs. The machine was **heavily loaded by other jobs** (load average 99–255), so treat these as upper bounds. Benchmark source: `/private/tmp/claude-501/-Users-alejandro-Projects-Conway-GameOfLife/d26793ef-30fe-4239-b1fb-77f42d831735/scratchpad/fftbench/src/main.rs`.

| grid | realfft conv, 1 thread | growth+clip, 1 thread | **full step, 1 thread** | realfft conv, rayon | **full step, rayon** | c2c rustfft conv, rayon | direct R=13 conv, 1 thread / rayon | 3 ch × 15 kernels conv, rayon |
|---|---|---|---|---|---|---|---|---|
| 256² | 0.23 ms | 0.14 ms | **0.38 ms** | 0.25 ms | **0.33 ms** | 0.48 ms | 2.27 / 0.49 ms | 2.5 ms |
| 512² | 1.01 ms | 0.56 ms | **1.57 ms** | 0.53 ms | **0.67 ms** | 1.43 ms | 8.6 / 1.6 ms | 4.5 ms |
| 1024² | 4.56 ms | 2.23 ms | **6.8 ms** | 1.56 ms | **1.88 ms** | 4.8 ms | – | 13.7 ms |
| 2048² | 22.4 ms | 8.9 ms | **31.8 ms** | 5.1 ms | **6.7 ms** | 31.6 ms | – | 46.6 ms |

Implications:

- 512² single-kernel Lenia runs at about 1500 steps/s with rayon, far above 60 FPS even at 10 steps/frame.
- Tessellatium-like 3-channel / 15-kernel rules cost about 4.5 ms/step at 512². That is fine for real time at 1–3 steps/frame, but tight for fast-forward or for evolutionary batches.
- The growth step (an `exp` per cell) is about a third of single-threaded step time. A lookup table or a polynomial `exp` approximation is a cheap win if needed (UNVERIFIED gain).

### 7.7 When a GPU (wgpu compute via Bevy) is worth it

- **Bevy 0.20.1 facts** (VERIFIED (source), from `bevy-0.20.1/examples/shader/compute_shader_game_of_life.rs` in the local cargo registry):
  - Compute work is a **system in the `RenderGraph` schedule**: `render_app.add_systems(RenderGraph, game_of_life.before(camera_driver))`. The system takes a `RenderContext` and records `begin_compute_pass` / `dispatch_workgroups`.
  - Pipelines are queued with `PipelineCache::queue_compute_pipeline` in `RenderStartup`.
  - Bind groups are built in `RenderSystems::PrepareBindGroups`.
  - The shader is WESL (`shaders/game_of_life.wesl`). Workgroup size is 8 × 8, with ping-pong storage textures selected by bind-group index.
  - `bevy_render 0.20.1` depends on `wgpu = "30"`; `wgpu 30.0.1` is in the registry.
  - Sibling examples: `gpu_readback.rs` and `storage_buffer.rs`.
- **The architecture rule (sim crates are Bevy-free) means a GPU backend lives in the rendering crate.** It can be a WGSL implementation of the same rule spec, validated against the CPU engine with statistical tolerances (never bitwise).
- **No WGSL FFT ships with wgpu or Bevy** (UNVERIFIED that no mature wgpu FFT crate exists). Options:
  - (a) Direct convolution in a compute shader with workgroup-shared-memory tiles. For R ≤ 20 this is simple and fast: at 1024² and 517 taps it is about 0.54 G multiply-adds per step, well within an M2 Max GPU budget. Rough GPU throughput figures (UNVERIFIED): about 10–13 TFLOPS FP32 and about 400 GB/s.
  - (b) A Stockham radix-4/8 FFT in WGSL, for large R, 3D or many kernels.
- **Choose the GPU when**:
  1. Grids are ≥ 1024² with multi-channel/multi-kernel rules (CPU: 14 ms at 1024² for 3 ch × 15 kernels).
  2. **3D Lenia**: 64³ = 0.26 M cells is fine on the CPU. 128³ = 2.1 M cells is half the cell count of 2048² 2D (4.2 M cells, 6.7 ms/step with rayon), so expect ≲ 7 ms/step (estimate, UNVERIFIED).
  3. Large Flow-Lenia worlds. For reference, [FL23] reports 255 µs/step on a Tesla T4 at 128² with 10 kernels, and its multi-species runs used 1024² worlds for 200k steps.
  4. Batched parameter search, i.e. many small worlds stepped together.
- **Stay on the CPU when**:
  - Grids are ≤ 512² single-channel. Uploading the field each frame is trivial (512² f32 = 1 MiB/frame).
  - Determinism and headless reproducibility are required.
  - Analysis needs the data on the CPU anyway. GPU readback adds latency, though `gpu_readback.rs` shows the pattern.

---

## 8. Testing strategy

### Unit tests (exact values; computed independently)

- **Kernel cores**:
  - Exponential core: `K_C(0.5) = 1`, `K_C(0.25) = K_C(0.75) = e^{−4/3} = 0.2635971`, `K_C(0.1) = 8.15988e−4`.
  - Polynomial core: `K_C(0.25) = 0.75⁴ = 0.31640625`.
  - Bell kernel: `bell(0.25) = 0.2493522`.
- **Kernel normalization**: `Σ K = 1` within 1e−6 in f32, after normalizing in f64. The kernel spectrum DC bin = 1, and the imaginary part is ≈ 0 for a symmetric kernel. Rings peak at `r = (i + ½)/B` in kernel units.
- **Growth with μ = 0.15, σ = 0.015**:
  - Gaussian: `G(μ) = 1`, `G(μ ± σ) = 2e^{−½} − 1 = 0.2130613`, `G(μ ± 2σ) = −0.7293294`, `G(μ ± 3σ) = −0.9777820`, `G(0) = −1`. Zero crossing at `|u − μ| = σ√(2 ln 2) = 1.17741σ`.
  - Polynomial: `G(μ ± σ) = 2(8/9)⁴ − 1 = 0.2485902`, `G(μ ± 2σ) = −0.8094803`, `G(μ ± 3σ) = −1` exactly. Zero crossing at `1.19663σ`.
  - Rectangular: ±1 at the edges per eq. 13.
- **SmoothLife**:
  - `σ₁(a, a) = 0.5`; `σ₁(a + α/2, a) = 1/(1 + e^{−2}) = 0.8807971`.
  - With the paper parameters: `s(0.3, 0) = 0.958538`, `s(0.3, 1) = 0.991113`, `s(0.4, 1) = 0.998388`, `s(0.4, 0) = 0.006693`, `s(0.2, 1) = 7.0e−5`, `s(0.5, 1) = 3.87e−4`.
  - Kernel areas for rₐ = 21, rᵢ = 7, b = 1: M = 154.55, N = 1231.23.
- **RLE codec**: round-trip `decode(encode(A)) == round(A·255)/255`. Decoding `animals.json` Orbium gives a 20 × 20 array with Σ = 76.8627 (= 19600/255) and max = 1.0.
- **FFT vs direct**: on random fields and on Orbium, `max |U_fft − U_direct| ≤ 1e−6` in f32. We measured 1.34e−7.
- **Flow-Lenia RT**: identity when F = 0 and s = 0.5; blur weights [0.11538, 0.76923, 0.11538] per axis when F = 0 and s = 0.65; mass conservation with random F (relative error ≤ 1e−6 per step in f32 with local-coordinate weights); non-negativity.

### Property tests

- **Translation**: shifting the initial state by (dx, dy) on the torus shifts the result by the same amount (≤ 1e−5 max abs after 50 steps).
- **Rotation and reflection**: 90° rotation or reflection of the initial state commutes with the step (≤ 1e−5), since the kernels are radially symmetric.
- **Determinism**: same seed and same thread count give bitwise-identical fields. Different thread counts must also be bitwise identical, because FFT rows are independent and reductions are ordered.
- **Lenia bounds**: `0 ≤ A ≤ 1` always, and `0 ≤ U ≤ 1 + 1e−6`.

### Regression oracles (VERIFIED (repro); implementation-defined, so use tolerances)

- **Orbium, tutorial config**:
  - **Setup**: 128² torus, cells placed at offset (54, 54), bell kernel, gaussian growth, R=13, T=10, μ=0.15, σ=0.015.
  - **Mass ΣA**: 75.10 (t=0) → 73.55 (10) → 73.10 (100) → 72.97 (200) → 73.00 (500).
  - **Steady state**: over steps 500–1000, mass stays in [72.4, 73.3], a relative range of 1.2%.
  - **Speed**: 0.602 ± 0.01 cells/step, i.e. 0.463 R-units per unit time.
  - **Direction**: constant, `atan2(222.2, 560.0)` ≈ 21.6° from the +row axis for the cells exactly as given (row 0 at the top).
  - **Suggested assertions**: mass ∈ [70, 76] for steps 100–5000; speed ∈ [0.58, 0.63]; heading drift < 2°.
- **Orbium, `animals.json` cells, polynomial/polynomial**: mass ≈ 73.6 (rel. range 1.1%), speed 0.623 cells/step.
- **Survival smoke test**: Scutium solidus, Hydrogeminium natans and Orbium all survive 2000 steps (mass within [0.3, 3]× initial). The 227/239 species survey (§4.5) can be a nightly job, taking about 90 s in numpy and presumably much less in Rust.
- **SmoothLife paper glider** (discrete): mass 799.4 ± 2 and speed 5.28 ± 0.02 cells/step over 1000 steps. Regenerate the fixture with our engine from the documented soup procedure, then store it in RON/binary.
- **Flow-Lenia**: total mass constant to ≤ 1e−5 relative over 1000 steps (f32), or exact with f64 accumulation or renormalization. The creature stays spatially localized: the bounding box stays below 50% of the world for the [FL23] Table 1 defaults with a random 40 × 40 patch (UNVERIFIED for arbitrary random parameters).

---

## 9. Licensing summary for what we may ship

- **Equations and parameter values** from the papers are facts. Implement them freely and cite the papers.
- **Chan's `animals.json` patterns and Lenia code**: MIT, Copyright (c) 2018 Bert Chan. Data may be vendored with the MIT notice. We did not copy code; the RLE codec is re-implemented from the format description.
- **Lenia tutorial patterns** (Orbium, Hydrogeminium, Fish, Tessellatium, emitter, pacman, wanderer): MIT, Copyright (c) 2021 Open Science Lenia. Vendor with notice.
- **Flow-Lenia reference code**: no license, so do not copy or translate it. Implement from the paper. Our RT scatter was written from the paper's description.
- **SmoothLife original code**: GPLv2 per SourceForge (UNVERIFIED in-file). **Ready**: GPL-3.0. Do not copy code from either; the parameter sets are facts.
- **Crates**: rustfft (MIT OR Apache-2.0), realfft (MIT), transpose/num-* (MIT OR Apache-2.0), rayon (MIT OR Apache-2.0).

---

## 10. Open questions / UNVERIFIED items

1. **SmoothLife modes 3 and 4** (`m + dt(2s−1)`, `m + dt(s−m)`) come from a third-party port quoting Rafler's `snm2D.frag`. Not checked against the original shader. Reading the SourceForge zip would settle it; it is GPL, so read only.
2. **Rafler's `sigtype`/`mixtype` options** are undocumented. The Ready SmoothLifeL "mix indicators" transition is an assumption about one of them.
3. **Hydrogeminium limus** exploded in our runs. It may need the JS function set or differs by design (the paper calls it metastable).
4. **wgpu FFT**: whether a maintained wgpu/WGSL FFT crate exists in 2026 was not checked.
5. **M2 Max GPU throughput figures** quoted in §7.7 are from memory.
6. **Larger-R presets**: performance numbers are for R=13 kernels. FFT cost is R-independent, but direct-convolution and GPU-tiling numbers scale with R².
