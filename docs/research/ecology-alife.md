# Ecology and Artificial-Life Modeling Foundations

Research note for the Emergence Lab ecosystem, evolution and civilization modes.
Scope: models, formulas and default parameters that the headless, deterministic simulation crates can implement.
Bevy and rendering are out of scope here.

Date: 2026-11-09. Author: research subagent (Claude), for Alejandro Newport Diaz.

## Verification legend

Every claim carries one of these tags (or the section header carries it for the whole section).

| Tag | Meaning |
|---|---|
| **[V]** | Verified in this session against a primary source or a faithful secondary source that was fetched and read (URL given). |
| **[C]** | Computed in this session from downloaded data or from our own independent re-implementation. Method is given so it can be reproduced. |
| **[K]** | Standard textbook or literature result written from background knowledge. The bibliographic reference is given, but the paper was **not** re-fetched in this session. DOIs on [K] items were not re-checked. |
| **UNVERIFIED** | A specific number or detail that we could not confirm. Treat it as a placeholder to calibrate. |

Downloaded raw material (data and papers only, nothing executed) is in
`/private/tmp/claude-501/-Users-alejandro-Projects-Conway-GameOfLife/d26793ef-30fe-4239-b1fb-77f42d831735/scratchpad/downloads/ecology-*`.
The analysis scripts we wrote are in
`/private/tmp/claude-501/-Users-alejandro-Projects-Conway-GameOfLife/d26793ef-30fe-4239-b1fb-77f42d831735/scratchpad/scripts/`
(`wsp_sim.py`, `wsp_batch.py`, `fit_allometry.py`, `mte_norm.py`, `allo.py`, `species_tables.py`).

---

## 0. Executive summary: decisions for implementers

1. **Predator–prey baseline.** Re-implement the *rules* of NetLogo Wolf Sheep Predation from the description in section 1. Do not copy its code, because the model is CC BY-NC-SA 3.0.
   The grass version is the persistent one. In our re-implementation it ran 3,000 ticks in 8/8 seeds with noisy quasi-cycles.
   The no-grass version collapsed in 10/10 seeds within 194–605 ticks. Use both as regression scenarios.
2. **Stability knobs.** Prey density dependence (a regrowing resource), refuges and space, predator interference, and an energy-gated predator reproduction rule stabilize the system.
   Unlimited prey, saturating (Holling II) predation on an enriched prey (the paradox of enrichment), and small populations destabilize it.
   Expose these as explicit parameters.
3. **Derive species parameters from body mass M (kg)** with fitted allometries (section 3, Table 3.3). We measured them in this session from AnAge Build 15 and PanTHERIA 1.0 [C], plus Hirt et al. 2017 for speed [V] and Jetz et al. 2004 for home range [V].
   Hand-tune only multipliers per functional group.
4. **Energy is the universal currency.** Use a per-agent energy budget: intake × assimilation efficiency − Kleiber/MTE maintenance − locomotion − reproduction.
   Store conserved stocks (energy, biomass, water, N, P) as **integer fixed-point** for exact conservation and bit-for-bit determinism.
5. **Vegetation and soil as grid fields**, not agents: biomass per plant functional type, a water bucket, litter, soil organic matter, mineral N and P, fungal biomass.
   Growth = potential × Liebig minimum (or a multiplicative co-limitation) of light (Beer–Lambert), water, nutrients and temperature (beta function).
6. **Fire.** Use the Alexandridis et al. (2008) probabilistic CA [V] with fuel, moisture and wind/slope factors, followed by a succession state machine (section 5).
7. **Terrain and water.** fBm/ridged noise with domain warping, then Priority-Flood depression filling, D8 flow accumulation for rivers and lakes, and the virtual-pipe shallow-water model (Mei et al. 2007 [V]) for floods.
   Classify biomes with a Whittaker-style (T, P) lookup.
8. **Agent AI.** Use utility scoring for *what* to do, small behavior trees or FSMs for *how*, and Reynolds steering for *movement*. Use a uniform-grid spatial hash for neighbors.
   Use a **two-phase intent → sort → resolve** tick for all contested resources (section 7.7).
9. **Evolution.** Use a real-valued genome mapped through allometric trade-offs, Gaussian mutation with reflection (or logit-space mutation), and neutral marker loci to measure drift.
   Define species by NEAT-style distance-to-representative plus a mating-compatibility threshold. Keep a taxon-level phylogeny with pruning.
10. **Societies.** Implement Sugarscape rules G, M, R, S, K, C, I, T and L exactly as specified by Kehoe's formal spec [V], with explicit conflict resolution.
    Add stigmergic path reinforcement for emergent roads and trade routes, and XTENT/Voronoi territories.

---

## 1. Predator–prey: NetLogo Wolf Sheep Predation and Lotka–Volterra

### 1.1 Provenance and licence [V]

- Wilensky, U. (1997). *NetLogo Wolf Sheep Predation model.* CCL, Northwestern University. https://ccl.northwestern.edu/netlogo/models/WolfSheepPredation
- Source read: `Sample Models/Biology/Wolf Sheep Predation.nlogox` from https://github.com/NetLogo/models (raw file, NetLogo 7.1.0-alpha1 format).
- **Licence: Creative Commons BY-NC-SA 3.0** (stated in the model's Info tab). NetLogo itself is GPL.
  **Do not copy the code or the Info text into Emergence Lab (MIT OR Apache-2.0).** Re-implement the rules, which are described below in our own words, and credit Wilensky (1997) as inspiration.

### 1.2 World and default parameters [V]

| Parameter | Default (slider) | Range | Notes |
|---|---|---|---|
| world | 51 × 51 patches (pxcor, pycor ∈ [−25, 25]) | — | wraps in x and y (torus); 2,601 patches |
| `model-version` | `sheep-wolves` (chooser index 0) | `sheep-wolves`, `sheep-wolves-grass` | the default loads the **no-grass** version |
| `initial-number-sheep` | 100 | 0–250 | |
| `initial-number-wolves` | 50 | 0–250 | |
| `sheep-gain-from-food` | 4 | 0–50 | energy per grass patch eaten |
| `wolf-gain-from-food` | 20 | 0–100 | energy per sheep eaten |
| `sheep-reproduce` | 4 % | 1–20 % | probability per tick |
| `wolf-reproduce` | 5 % | 0–20 % | probability per tick |
| `grass-regrowth-time` | 30 ticks | 0–100 | grass version only |
| `show-energy?` | off | — | display only |
| `max-sheep` | 30,000 (desktop), 10,000 (NetLogo Web) | — | stop condition if no wolves remain |

Initialization [V]:
- Sheep energy = `random(2 × sheep-gain)`, which is uniform over the integers 0 … 7 by default.
- Wolf energy = `random(2 × wolf-gain)`, which is 0 … 39 by default.
- Agents start at uniform random continuous positions.
- Grass version: each patch is green or brown with p = 0.5. Green patches get `countdown = regrowth-time`. Brown patches get `countdown = random(regrowth-time)`, which staggers regrowth.

### 1.3 Per-tick rules (paraphrased from the source) [V]

Order within a tick: all sheep (in NetLogo's random `ask` order), then all wolves (random order), then grass, then `tick`.

- **Move** (both breeds): turn right by a random integer in [0, 50)°, turn left by a random integer in [0, 50)°, then move forward 1 patch length. This is a correlated random walk.
- **Sheep** (grass version only):
  - Energy −1.
  - If the current patch is green, make it brown and gain `sheep-gain-from-food`.
  - Die if energy < 0.
  - In the no-grass version sheep never gain or lose energy and never starve.
- **Sheep reproduction** (both versions): with probability `sheep-reproduce`/100, halve the parent's energy, and hatch one offspring with the same energy. The offspring turns randomly and moves 1.
- **Wolves:**
  - Energy −1.
  - Pick one random sheep on the same patch (if any), kill it and gain `wolf-gain-from-food`. At most 1 sheep per wolf per tick, and only on the same patch. This is an effectively linear (type I) functional response capped at 1 per tick.
  - Die if energy < 0.
  - Reproduce with probability `wolf-reproduce`/100, halving energy as for sheep.
  - **Reproduction is not gated by energy.** A starving wolf can still reproduce.
- **Grass** (patch procedure): if brown, then either turn green when countdown ≤ 0 (and reset countdown to `regrowth-time`), or decrement countdown. A grazed patch is therefore unavailable for about `regrowth-time + 1` ticks.
- **Stop conditions:** no turtles left, or no wolves and more than `max-sheep` sheep ("the sheep have inherited the earth").

### 1.4 Observed dynamics: our independent re-implementation [C]

We wrote `wsp_sim.py`/`wsp_batch.py`, which follow the rules above. Its RNG and floating-point details differ from NetLogo, so the numbers are indicative only.

| Version (defaults) | Seeds | Outcome |
|---|---|---|
| sheep-wolves-grass | 8 × 3,000 ticks | **8/8 persist.** After the 500-tick burn-in: sheep mean ≈ 156–162 (sd ≈ 22–31), wolves ≈ 75–77 (sd ≈ 13–15), green patches ≈ 969–987 of 2,601. These are **noisy quasi-cycles**: the autocorrelation peak sits at a lag of ≈ 146–178 ticks with low coherence (ACF 0.11–0.51), not clean LV orbits. |
| sheep-wolves (no grass) | 10 × 1,500 ticks | **10/10 collapse.** Wolves go extinct at t = 194–605. In 6/10 runs the wolves died first and the sheep then exploded to the 30,000 cap. In 4/10 runs the sheep went extinct first (t = 150–402) and the wolves then starved. |

This matches the model's own description: the no-grass version is "ultimately unstable", and the grass version is "generally stable" and closer to LV oscillations. The authors also note that real-valued LV models underestimate extinction in small populations (Wilensky & Rand 2015, ch. 4) [V].

### 1.5 Lotka–Volterra family [K]

**Classic LV** (Lotka 1925, Volterra 1926), with prey x and predator y:

```
dx/dt = α x − β x y
dy/dt = δ x y − γ y
```

- Equilibrium: (x*, y*) = (γ/δ, α/β). Linearized period T ≈ 2π / √(α γ).
- Conserved quantity: V(x, y) = δx − γ ln x + βy − α ln y.
- Orbits are **neutral closed cycles**: a perturbation moves the system onto a different orbit, and nothing pulls it back.
- With demographic noise the orbit amplitude random-walks until one population hits 0, so **stochastic, individual-based versions eventually go extinct.** This is the mechanism behind the no-grass collapse.
- Cycle averages equal (x*, y*) (Volterra's principle).

**Rosenzweig–MacArthur** (logistic prey plus Holling type II predation; Rosenzweig & MacArthur 1963; Rosenzweig 1971, *Science* 171:385, doi:10.1126/science.171.3969.385):

```
dN/dt = r N (1 − N/K) − a N P / (1 + a h N)
dP/dt = e a N P / (1 + a h N) − m P
```

- a = attack rate, h = handling time, e = conversion efficiency, m = predator death rate.
- Predator isocline: N* = m / (a (e − m h)). It requires e > m h and N* < K, otherwise the predator dies out.
- Prey isocline hump: N_hump = (K − 1/(a h)) / 2.
- **Stable focus iff N* > N_hump**, that is K < 2N* + 1/(a h). Otherwise a Hopf bifurcation produces a **limit cycle**.
- Raising K (enrichment) destabilizes the system: the *paradox of enrichment*. Large cycles bring populations close to 0, so a stochastic model goes extinct.

**Holling functional responses** (Holling 1959, *Can. Entomol.* 91:385) [K]:

| Type | Intake per predator f(N) | Effect on stability |
|---|---|---|
| I | a N (often capped) | neutral; this is the NetLogo case |
| II | a N / (1 + a h N) | saturating; destabilizing |
| III | a N² / (1 + a h N²) | sigmoid; prey refuge at low density; stabilizing |

**Predator interference** (Beddington–DeAngelis, f = a N / (1 + a h N + c P)) and ratio dependence both stabilize [K].

### 1.6 What produces oscillation versus collapse (design checklist)

| Ingredient | Effect | Emergence Lab knob |
|---|---|---|
| Prey resource limitation (regrowing grass, carrying capacity K) | damps cycles. The NetLogo grass version persists [V][C]. | plant regrowth rate and biomass cap |
| Unlimited prey food | neutral LV and drift to extinction [V][C] | — |
| Saturating predation with high K | limit cycles; with K too high, extinction | handling time h, digestion cooldown |
| Spatial structure, dispersal limits, refuges | persistence through asynchronous local cycles (Huffaker 1958, *Hilgardia* 27:343) [K] | terrain, burrows, cover, water |
| Small populations | demographic extinction [V] | minimum viable population warnings |
| Time delays (maturation, energy reserves, gestation) | cycles, overshoot | maturity age, reserves |
| Reproduction gated by energy (instead of NetLogo's fixed probability) | couples birth to food; usually damps cycles. Suggested in the model's "Things to try" [V]. | `reproduce_if energy > threshold` |
| Predator interference, territoriality | stabilizes [K] | territory model (section 9.4) |
| Allee effects in prey or predators | sudden collapse below a threshold [K] | mate-finding requirement |
| Multiple prey with predator switching (type III) | stabilizes [K] | diet preference matrix |

### 1.7 Implementation notes

- NetLogo's `ask` uses a fresh random order each tick, which is asynchronous updating.
- For determinism, either do a seeded shuffle per tick (simple and serial) or use the two-phase intent/resolve pattern (section 7.7).
  Each prey can be consumed once, so use intents "eat prey #k" sorted by (prey id, priority key).
- Keep a "NetLogo-compatible" scenario preset with exactly these defaults (51 × 51 torus, and so on), so the UI can show the classic plot.

---

## 2. What is deepmind.google/research/publications/106327? [V]

**It is Flow-Lenia** (the Google DeepMind publication listing is dated 28 Apr 2025):

- Plantec, E., Hamon, G., Etcheverry, M., Chan, B. W.-C., Oudeyer, P.-Y., Moulin-Frier, C. *Flow-Lenia: Emergent evolutionary dynamics in mass conservative continuous cellular automata.* **Artificial Life** (MIT Press), doi:10.1162/artl_a_00471.
- Preprint: arXiv:2212.07906, *Flow-Lenia: Towards open-ended evolution in cellular automata through mass conservation and parameter localization* (v1 14 Dec 2022, v2 24 Mar 2023). https://arxiv.org/abs/2212.07906
- The listing is on the DeepMind site presumably because co-author Bert Wang-Chak Chan, the creator of Lenia, is affiliated with Google DeepMind. That affiliation is UNVERIFIED in this session.

**What it does** (equations read from the ar5iv rendering of the preprint) [V]:

1. **Affinity map.** Compute U exactly like the Lenia growth field: U_j = Σ_i h_i · G_i(K_i ∗ A_{c0_i}) · 1[c1_i = j].
2. **Flow field.** F_i = (1 − α) ∇U_i − α ∇A_Σ, where α(p) = clip_[0,1]((A_Σ(p)/θ_A)^n) and A_Σ is the total mass per cell.
   Matter climbs the affinity gradient, but diffuses away when the local mass approaches a critical mass θ_A.
   The paper uses n = 2. θ_A is described only as a positive critical mass (its value is UNVERIFIED).
3. **Reintegration tracking.** Each cell's mass is moved to p'' = p + dt · F and spread as a uniform square of side 2s, then integrated back onto the grid cells.
   This **conserves total mass exactly** per channel. Defaults: dt = 0.2, s = 0.65.
4. **Parameter localization.** The rule parameters P(p) live *in each cell* and travel with the matter.
   Incoming parameters are mixed either by a mass-weighted average or by softmax sampling, with probability ∝ exp(incoming mass).
   **Mutation** = Gaussian noise added to the parameter map at some rate. Different "species" (rule parameter sets) can therefore coexist and compete in one world.
5. The paper analyses emergent evolution with the **evolutionary activity** framework (Bedau & Packard) and other metrics.
   The code is a notebook (link given as https://tinyurl.com/mr2ncy3h), JAX-based. Its licence is unknown, so do not copy it.

**Relevance to Emergence Lab:**
- **Lenia/SmoothLife mode.** Flow-Lenia is a drop-in "advanced" variant. Mass conservation stops the usual Lenia failure modes, where everything explodes or dies, and makes creatures behave like conserved matter that can collide, merge and compete.
  Implementation needs FFT convolution, Sobel gradients and a local reintegration kernel. Determinism holds if sums run in a fixed order.
- **Evolution lab.** This is the cleanest published example of *genome carried by matter*: heritable parameters, local mutation, and no explicit fitness function, with selection emerging from competition for mass.
  The same pattern applies to our Rhai and DSL custom rules: store a per-cell rule-parameter vector, advect it, and mutate it.
- **Metrics.** Adopt Bedau–Packard evolutionary activity statistics for the evolution dashboard (section 8.9).
- Related works [K]:
  - Chan 2019, *Lenia — Biology of Artificial Life*, Complex Systems 28(3), arXiv:1812.05433.
  - Chan 2020, multi-channel *Lenia and Expanded Universe* (ALIFE 2020), arXiv:2005.03742.

---

## 3. Energy flow and metabolic scaling

### 3.1 Trophic transfer

- **Lindeman (1942)**, *The trophic-dynamic aspect of ecology*, Ecology 23:399–418 (doi:10.2307/1930126).
  The ratio of energy flux at trophic level n+1 to level n (the "Lindeman efficiency") has been estimated empirically at **about 10 %**, as summarized by Brown et al. 2004 [V].
  Real values vary widely (roughly 2–25 % in marine meta-analyses, e.g. Pauly & Christensen 1995, *Nature* 374:255 [K]).
- Decomposition (Kozlovsky 1968; Begon, Townsend & Harper, *Ecology*) [K]:
  - Trophic transfer efficiency = consumption efficiency × assimilation efficiency × production efficiency.
  - Typical **assimilation** efficiency: carnivores about 80 %; herbivores about 20–60 % depending on food (seeds high, wood low); detritivores about 20–40 % (UNVERIFIED ranges).
  - **Production** efficiency (net production / assimilation) is low in endotherms (about 1–3 %) and high in ectotherms (about 10–40 %), per Humphreys 1979, *J. Anim. Ecol.* 48:427 (UNVERIFIED exact values).
  - This is why **ectotherm food chains are longer** and biomass pyramids differ.
- Agent energy budget, in per-agent fixed-point units:
  - Consumption C = assimilated A + egested F.
  - A = respiration R + production P (growth + reproduction).
  - Egested and dead matter go to the litter pool (section 4.6).
- Steady-state consequence (Brown et al. 2004, Eq. for Lindeman efficiency a) [V]:
  - Abundance ratio between levels: N1/N0 = a (M0/M1)^{3/4}.
  - Biomass ratio: W1/W0 = a (M0/M1)^{−1/4}.
  - Inverted *energy* pyramids are impossible. Inverted *abundance* or *biomass* pyramids are possible when size ratios are extreme, e.g. insects on trees, or whales on plankton.

### 3.2 Kleiber's law and the Metabolic Theory of Ecology (MTE) [V]

- **Kleiber (1932)**, *Body size and metabolism*, Hilgardia 6:315 (doi:10.3733/hilg.v06n11p315).
  Whole-organism metabolic rate scales as M^{3/4}. The classic mammal fit is about **70 kcal/day · M^{0.75}** (M in kg), which equals **3.39 W · M^{0.75}** (70 kcal/day = 70 × 4184 J / 86,400 s) [K for the coefficient].
- **Brown, Gillooly, Allen, Savage & West (2004)**, *Toward a metabolic theory of ecology*, Ecology 85:1771–1789, doi:10.1890/03-9000. PDF: https://people.clas.ufl.edu/gillooly/files/05_brownetal_2004.pdf
  - Individual metabolic rate: **I = i0 · M^{3/4} · e^{−E/kT}**, with k = 8.62 × 10⁻⁵ eV/K.
  - Activation energy: the fitted common slope is E ≈ 0.69 eV including hibernators, and ≈ 0.63 eV excluding them. Typical aerobic respiration is 0.60–0.70 eV. **They use E = 0.63 eV for temperature correction.** The normalization i0 varies about 20-fold across taxa.
  - Biological times (development, lifespan, generation time) scale as t ∝ M^{1/4} e^{E/kT}. Mass-specific rates scale as ∝ M^{−1/4} e^{−E/kT}.
  - r_max ∝ M^{−1/4} e^{−E/kT}.
  - Carrying capacity: K ∝ [R] · M^{−3/4} · e^{E/kT}, where [R] is resource supply. Equilibrium density of mammals scales as M^{−3/4}.
  - Production P ∝ M^{3/4} e^{−E/kT}.
- Our temperature-corrected check on AnAge [C]: B / M^{0.75} corrected to 20 °C with E = 0.63 eV (geometric means) gives:
  - Mammalia **0.78** (n = 378)
  - Amphibia **0.16** (n = 18)
  - Reptilia **0.125** (n = 16)

  So **at equal body temperature an ectotherm burns ≈ 0.16–0.2 × an endotherm of the same mass**.
  Use `ECTO_FACTOR = 0.16` and then apply the Boltzmann factor for the actual body temperature.

### 3.3 Allometric formulas usable to derive per-species parameters (M in kg)

Form: **y = a · M^b**. Fits marked [C] are OLS on log10 values, computed in this session (`fit_allometry.py`).
Data: AnAge Build 15 (excluding records with data quality "low" or "questionable"), and PanTHERIA 1.0 mammals (excluding bats and cetaceans).

| Quantity | Group | a | b | r² | n | Source |
|---|---|---|---|---|---|---|
| Basal metabolic rate (W) | mammals | 2.80 | 0.725 | 0.96 | 343 | AnAge [C] |
| Basal metabolic rate (W) | birds | 3.85 | 0.652 | 0.97 | 165 | AnAge [C] |
| Basal metabolic rate (W) | amphibians (at measurement T) | 0.098 | 0.746 | 0.79 | 15 | AnAge [C] |
| BMR, classic | mammals | 3.39 | 0.75 | — | — | Kleiber [K] |
| Field metabolic rate | mammals / birds / reptiles | — | 0.734 / 0.681 / 0.889 | — | 79/95/55 spp | Nagy, Girard & Brown 1999, *Annu. Rev. Nutr.* 19:247 [V exponents only; coefficients UNVERIFIED] |
| Max longevity (yr) | mammals | 14.1 | 0.159 | 0.50 | 997 | AnAge [C] |
| Max longevity (yr) | mammals excl. bats & cetaceans | 9.03 (= 108.4 months) | 0.239 | 0.62 | 893 | PanTHERIA [C] |
| Max longevity (yr) | birds | 23.6 | 0.214 | 0.45 | 1330 | AnAge [C] |
| Max longevity (yr) | reptiles | 19.8 | 0.138 | 0.29 | 355 | AnAge [C] |
| Female maturity (d) | mammals | 350 | 0.215 | 0.47 | 801 | AnAge [C] |
| Sexual maturity (d) | mammals | 316 | 0.269 | 0.54 | 912 | PanTHERIA [C] |
| Female maturity (d) | birds | 819 | 0.207 | 0.42 | 807 | AnAge [C] |
| Female maturity (d) | reptiles | 1554 | 0.190 | 0.30 | 103 | AnAge [C] |
| Gestation (d) | mammals | 61.8 | 0.251 | 0.64 | 1139 | PanTHERIA [C] |
| Incubation (d) | birds | 28.2 | 0.177 | 0.66 | 547 | AnAge [C] |
| Litter size | mammals | 2.12 | −0.138 | 0.37 | 1837 | PanTHERIA [C] |
| Clutch size | birds | 3.24 | −0.037 | 0.02 | 548 | AnAge [C] (essentially mass-independent) |
| Clutch size | reptiles | 9.9 | 0.320 | 0.56 | 34 | AnAge [C] |
| Home range per individual (km²) | mammal herbivores | 0.0211 | 1.112 | 0.77 | 192 | PanTHERIA [C] |
| Home range per individual (km²) | mammal omnivores | 0.122 | 1.122 | 0.71 | 251 | PanTHERIA [C] |
| Home range per individual (km²) | mammal carnivores | 0.436 | 1.311 | 0.78 | 85 | PanTHERIA [C] |
| Home range (ha) | all / herb / omni / carn mammals | 6.69 / 2.05 / 15.87 / 52.07 | 1.07 / 1.02 / 1.12 / 1.20 | — | 274/158/44/70 | Jetz et al. 2004 Table 1 [V] |
| Area per individual = 1/density (ha) | all / herb / omni / carn mammals | 2.06 / 1.01 / 3.62 / 34.43 | 0.76 / 0.76 / 0.73 / 0.86 | — | 563/327/176/38 | Jetz et al. 2004 [V] |
| Population density (ind/km²) | mammal herb / omni / carn | 109 / 30.0 / 7.73 | −0.807 / −0.808 / −1.016 | 0.66 / 0.61 / 0.68 | 304/338/98 | PanTHERIA [C] |
| Population density (ind/km²) | all mammals | 45.8 | −0.746 | 0.57 | 941 | PanTHERIA [C] (≈ Damuth's −3/4) |
| Max speed (km/h) | running | v = 25.5 M^0.26 (1 − e^{−22 M^{−0.6}}) | — | — | — | Hirt et al. 2017 [V] |
| Max speed (km/h) | flying | v = 142.8 M^0.24 (1 − e^{−2.4 M^{−0.72}}) | — | — | — | Hirt et al. 2017 [V] |
| Max speed (km/h) | swimming | v = 11.2 M^0.36 (1 − e^{−19.5 M^{−0.56}}) | — | — | — | Hirt et al. 2017 [V] |
| Fasting endurance | any | ∝ M^{0.25} (stores ∝ M, burn ∝ M^{0.75}) | — | — | — | Lindstedt & Boyce 1985, *Am. Nat.* 125:873 [K] |
| Max population growth r_max | any | ∝ M^{−1/4} e^{−E/kT} | — | — | — | Brown 2004 [V]; Savage et al. 2004 *Am. Nat.* [K] |
| Detection distance | animals | between M^0 and M^{1/2} | — | — | — | Jetz 2004, citing Kiltie 2000 and others [V] |

Notes:
- **Hirt et al. 2017**: *A general scaling law reveals why the largest animals are not the fastest*, Nature Ecology & Evolution 1:1116, doi:10.1038/s41559-017-0241-4.
  The parameters (a, b, h, i) come from Supplementary Table 4, with M in kg and v in km/h:
  - flying: a = 142.8 ± 16.7, b = 0.24 ± 0.01, h = 2.4 ± 1.4, i = −0.72 ± 0.26
  - running: a = 25.5 ± 0.84, b = 0.26 ± 0.006, h = 22 ± 7.6, i = −0.6 ± 0.05
  - swimming: a = 11.2 ± 0.91, b = 0.36 ± 0.02, h = 19.5 ± 13.6, i = −0.56 ± 0.07

  The hump-shaped form comes from finite acceleration time. Peak running speed lies near 50–100 kg.
  The model is fitted mostly on endotherms and fish. **It overpredicts slow-moving clades** (beavers, crocodiles on land, earthworms), so use observed or hand values for those (Table 10.3).
- **Jetz, Carbone, Fulford & Brown 2004**, *The scaling of animal space use*, Science 306:266–268. PDF: https://jetzlab.yale.edu/sites/default/files/files/Jetz%20et%20al%20Science%2004.pdf
  - Home range H scales about as M^{1.0–1.2}, but area per individual (1/N) scales as M^{0.76}.
  - The gap is **home-range overlap**: exclusivity falls about as M^{−1/4}. In their words, only 7 % is used exclusively at 100 kg.
  - For simulation this means: let large animals' ranges overlap. Do not assign exclusive territories scaled by M^{1}.
  - They also estimate the energy supply rate R ≈ 2188 (herbivores), 408 (omnivores) and 32 (carnivores) W/km².
- AnAge fits mix captive and wild records. Max longevity is a **record**, not a mean.
  For simulated mortality, use Gompertz hazard μ(t) = IMR · e^{γ t}, with γ = ln 2 / MRDT.
  AnAge gives IMR and MRDT for 43 species [C], for example:
  - *Mus musculus*: IMR 0.01 /yr, MRDT 0.3 yr
  - *Loxodonta africana*: IMR 0.002 /yr, MRDT 8 yr
  - *Equus caballus*: IMR 0.0002 /yr, MRDT 4 yr

  Add extrinsic mortality (predation, starvation) on top. Do not impose it.

### 3.4 Recipe: per-species parameters from M (pseudocode)

```text
inputs: M_kg, thermo ∈ {endo, ecto}, locomotion ∈ {run, fly, swim}, trophic_group, T_body_K
c_th        = 1.0 if endo else 0.16                         # [C] section 3.2
boltz       = exp(-E/k * (1/T_body - 1/293.15)), E = 0.63 eV  # MTE temperature correction
maint_W     = 0.78 * c_th * M^0.75 * boltz                  # W; 0.78 W/kg^0.75 at 20 C for mammals [C]
                                                             # (for endotherms at 37 C this is about 2.8-3.4 W M^0.75)
field_W     = maint_W * 2.5                                  # activity multiplier; FMR/BMR ratio UNVERIFIED (2-4)
v_max       = Hirt(locomotion, M)                            # km/h, override per clade where wrong
v_cruise    = 0.25..0.35 * v_max                             # designer default (UNVERIFIED)
locomotion_cost_per_m ∝ M^0.7                                # cost of transport; exponent UNVERIFIED ~0.6-0.7 [K]
store_max   = k_store * M                                    # J; fat reserve
fast_days   = store_max / field_W  (∝ M^0.25)
life_max_y  = group fit (Table 3.3); mortality via Gompertz + extrinsic
maturity_d  = group fit; gestation_d = group fit; litter = group fit
home_range  = Jetz/PanTHERIA fit by trophic group (overlapping)
K_density   = PanTHERIA fit by trophic group → initial population & sanity bounds
perception  = R0 * M^0.25 * modality_factor (R0 ≈ 30 m; designer)
```

Calibration rule of thumb: run a single-species meadow and check that the equilibrium density lands within a factor of about 3 of the PanTHERIA density fit for that mass and trophic group.

---

## 4. Plants, vegetation, fungi and nutrient cycling

### 4.1 Growth and grazing [K]

- **Logistic growth** (Verhulst 1838): dB/dt = r B (1 − B/K). This is the per-cell biomass of one plant functional type (PFT).
- **Grazing** (Noy-Meir 1975, *J. Ecol.* 63:459, doi:10.2307/2258730): dB/dt = r B (1 − B/K) − H · c_max B / (B + B_half).
  This produces **alternative stable states**: overgrazing collapse with hysteresis. It is good emergent behavior to keep.
- **Discrete NetLogo-style regrowth** (fixed countdown) is a special case. It is simple, but it gives a hard carrying capacity.

### 4.2 Light competition [K]

- **Beer–Lambert canopy attenuation** (Monsi & Saeki 1953): I(L) = I0 · e^{−k L}. Here L is the cumulative leaf area index (LAI) above, and the extinction coefficient k ≈ 0.5 (range about 0.3–0.8, UNVERIFIED).
- Per cell, sort PFT layers by height (trees > shrubs > herbs > moss) and compute absorbed light for each layer: APAR_i = I_above_i · (1 − e^{−k LAI_i}).
- This makes **succession emerge**. Fast, shade-intolerant PFTs win in open, post-fire cells. Slow, shade-tolerant PFTs win under a canopy.

### 4.3 Simplified photosynthesis and limitation [K]

- **Light-use efficiency** (Monteith 1972/1977): GPP = ε · APAR. NPP ≈ 0.45–0.5 × GPP (UNVERIFIED typical carbon-use efficiency).
  As one reference value, the CASA model (Potter et al. 1993, *Global Biogeochem. Cycles* 7:811) uses ε* ≈ 0.39 g C / MJ PAR (UNVERIFIED).
- Limitation, where each f ∈ [0, 1]:
  - **Liebig's law of the minimum** (Sprengel 1828, Liebig 1840): growth = G_max · min(f_light, f_water, f_N, f_P, f_T).
    It is cheap, and it shows the player a single "limiting factor" for a cell, which is good UI.
  - Multiplicative co-limitation: growth = G_max · f_light · f_water · f_N · f_T. It is smoother, but double-penalizes.
  - Recommendation: Liebig over the nutrients and water, multiplied by f_T and f_light.
- **Nutrient uptake** (Monod / Michaelis–Menten): f_N = N / (K_N + N).
- **Temperature**: use the beta function (Yan & Hunt 1999, *Ann. Bot.* 84:607, doi:10.1006/anbo.1999.0955):
  f_T = [(T_max − T)/(T_max − T_opt)] · [(T − T_min)/(T_opt − T_min)]^{(T_opt − T_min)/(T_max − T_opt)}, and 0 outside (T_min, T_max).
- **Water** (bucket model, Thornthwaite–Mather style):
  - W ← W + infiltration − ET − drainage.
  - f_water = min(1, W / (0.5 W_fc)), or f_water = AET/PET.
- **Primary productivity sanity targets by biome** (Whittaker & Likens 1975, mean NPP in g dry matter m⁻² yr⁻¹; commonly reproduced; UNVERIFIED in this session):

  | Biome | Mean NPP |
  |---|---|
  | tropical rainforest | ≈ 2200 |
  | temperate deciduous forest | ≈ 1200 |
  | boreal forest | ≈ 800 |
  | savanna | ≈ 900 |
  | temperate grassland | ≈ 600 |
  | tundra | ≈ 140 |
  | desert scrub | ≈ 90 |
  | swamp / marsh | ≈ 2000 |
  | open ocean | ≈ 125 |

  Use these to calibrate G_max per biome.
- **Phytoplankton** (aquatic cells): Eppley (1972, *Fish. Bull.* 70:1063) gives maximum growth ≈ 0.851 · 1.066^T doublings per day, which is μ ≈ 0.59 e^{0.064 T} d⁻¹ [K].

### 4.4 Seed dispersal kernels [K]

The 2D kernel k(r) is a probability density per unit area at distance r. To sample a distance, draw r from the radial density 2πr · k(r).

| Kernel | k(r) | Mean distance | Use |
|---|---|---|---|
| Negative exponential | (1/(2π α²)) e^{−r/α} | 2α | gravity- or ant-dispersed seeds |
| Gaussian | (1/(π α²)) e^{−r²/α²} | (√π/2) α | short diffusion |
| 2Dt (Clark et al. 1999, *Ecology* 80:1475) | p / (π u (1 + r²/u)^{p+1}) | finite for p > 1/2 | fat-tailed; wind and animal dispersal |
| Mixture | (1 − q) · short + q · long (q ≈ 0.01–0.05) | — | long-distance dispersal by birds and mammals (endozoochory, caching) |

Reference: Nathan & Muller-Landau 2000, *TREE* 15:278.

Implementation notes:
- Sample with a per-plant deterministic RNG stream.
- Draw seed *counts* per cell (Poisson) instead of individual seeds for performance.
- Let **animal agents carry seeds**: frugivores deposit seeds after a gut-passage delay, and jays and squirrels cache acorns. This gives emergent long-distance dispersal and links trophic levels.

### 4.5 Fungi [K]

Model fungi as grid fields (biomass per guild per cell) plus optional visible fruiting bodies.

- **Saprotrophs (decomposers).**
  - The decomposition rate k of litter and wood scales with saprotroph biomass and with temperature and moisture.
  - White-rot fungi degrade lignin, which sets the wood decay rate.
  - Fruiting bodies appear after moist warm spells. They are food for squirrels, boars, deer and insects.
  - Spores are wind-dispersed, so use a fat-tailed kernel.
- **Mycorrhizal symbionts.**
  - Most land plants are mycorrhizal: commonly cited as about 80–90 % of species (Smith & Read 2008, *Mycorrhizal Symbiosis*; Brundrett & Tedersoo 2018, *New Phytol.*). Exact fractions UNVERIFIED.
  - Model: the plant pays a carbon fraction (commonly cited as about 4–20 % of photosynthate, UNVERIFIED), and its N and P uptake is multiplied by (1 + m · fungal_biomass_norm).
  - Common mycorrhizal networks can be modeled as diffusion of nutrients between connected tree cells. Keep the effect weak. The strength of "wood-wide web" claims is debated in the literature (UNVERIFIED for magnitude).
- **Parasites and pathogens** (e.g. *Armillaria*): kill or weaken hosts and spread through rhizomorphs. They also act as saprotrophs on the dead wood they create.
- **Lichens** (fungus + alga): pioneers on bare rock and soil formation, with very slow growth (mm per year). Food for reindeer in tundra.

### 4.6 Nutrient cycling [K]

Pools per cell, as fixed-point integers:
- live biomass C, N, P per PFT
- litter (leaf / wood / dung / carcass) C and N
- soil organic matter (SOM) C and N
- mineral N (NH₄⁺ + NO₃⁻ lumped)
- mineral P (labile)
- saprotroph biomass
- water

Fluxes per day:
- **Litterfall** from turnover and mortality. Animal **dung, urine and carcasses** feed the litter pools. Carcasses also feed scavengers first.
- **Decomposition**: dC/dt = −k · C · f_T · f_W · f_Q · f_fungi. Over time this gives first-order (Olson 1963, *Ecology* 44:322) decay: C(t) = C₀ e^{−kt}.
  Typical leaf-litter k ranges from about 0.1–0.2 yr⁻¹ (boreal) to above 1–3 yr⁻¹ (wet tropics) (UNVERIFIED; global syntheses such as Zhang et al. 2008, *J. Plant Ecol.* 1:85).
  - Q10 form: f_T = Q10^{(T − T_ref)/10}, with Q10 ≈ 2 (typical range 1.5–3).
  - Or Lloyd & Taylor (1994, *Funct. Ecol.* 8:315): R = R₁₀ · exp[308.56 · (1/56.02 − 1/(T − 227.13))], with T in K.
  - Moisture f_W is hump-shaped, peaking near about 60 % water-filled pore space (Linn & Doran 1984, *SSSAJ* 48:1267).
  - Quality f_Q decreases with the litter C:N and lignin:N ratios.
- **Mineralization and immobilization**: microbes need N at their own C:N (≈ 8–12, UNVERIFIED).
  Net mineral N = (decomposed C / litter C:N) − (microbial growth C / microbial C:N). It can be negative, which models immobilization in high-C:N litter.
- **Uptake**: Monod-limited, plus the mycorrhizal multiplier, plus the Liebig constraint.
- **Inputs**: atmospheric N deposition, biological N fixation (legume PFT, or a cyanobacterial crust), and P weathering from bedrock (slow).
- **Losses**: leaching ∝ drainage × mineral N, denitrification in waterlogged cells, and fire volatilization (most N in burned biomass is lost, while most P stays in ash; exact fractions UNVERIFIED).
- **Stoichiometric indicators**:
  - Foliar N:P < 14 indicates N limitation; > 16 indicates P limitation (Koerselman & Meuleman 1996, *J. Appl. Ecol.* 33:1441).
  - The marine Redfield ratio is C:N:P = 106:16:1.
- Earthworms and other bioturbators raise the decomposition rate and mix litter into SOM, as an ecosystem-engineer flag.

---

## 5. Fire

### 5.1 Alexandridis et al. (2008) probabilistic CA [V]

- Paper: Alexandridis, Vakalis, Siettos, Bafas (2008). *A cellular automata model for forest fire spread prediction: the case of the wildfire that swept through Spetses Island in 1990.* Applied Mathematics and Computation 204:191–201, doi:10.1016/j.amc.2008.06.046.
- Equations and constants were verified via Freire & DaCamara (2019), *NHESS* 19:169, which reuses the model with the same constants: https://nhess.copernicus.org/articles/19/169/2019/

**States:**
1. no fuel (non-burnable)
2. unburned fuel
3. burning
4. burned

**Rules:**
- A burning cell becomes burned at the next step.
- A burned cell stays burned. A no-fuel cell never changes.
- A burning cell ignites each unburned neighbor (Moore neighborhood) independently with:

```
p_burn = p_h · (1 + p_veg) · (1 + p_den) · p_w · p_s
p_w    = exp(c1·V) · exp(V·c2·(cos θ − 1))       = exp(V·(c1 + c2·(cos θ − 1)))
p_s    = exp(a · θ_s),  θ_s = atan((E_target − E_source)/D)
```

Parameters:

| Parameter | Value | Notes |
|---|---|---|
| p_h | 0.58 | baseline probability |
| c1, c2 | 0.045, 0.131 | V = wind speed in m/s; θ = angle between wind direction and spread direction |
| a | 0.078 | θ_s is in degrees (implied by the magnitude; UNVERIFIED) |
| D | L for orthogonal neighbors, √2·L for diagonal | Alexandridis' diagonal convention is UNVERIFIED; the Freire & DaCamara text reads 2L |
| L (cell size) | 100 m | used in Freire & DaCamara |
| Δt | ≈ 20 min | used in Freire & DaCamara |
| p_veg | agricultural −0.3, thickets 0, *Pinus halepensis* forest +0.4 | as commonly reproduced; **UNVERIFIED** against the original table |
| p_den | sparse −0.4, normal 0, dense +0.3 | **UNVERIFIED** |
| Spotting (Freire & DaCamara extension) | jumps along the wind if V > 8 m/s and the angle < π/10, up to 10 cells | |
| Firebreaks | p_veg = p_den = −0.8 (primary roads), −0.7 (secondary), −0.4 (tertiary or waterlines) | Freire & DaCamara |

Worked example:
- With V = 10 m/s, downwind (θ = 0): p_w = e^{0.45} ≈ 1.57.
- Upwind (θ = 180°): p_w = e^{0.45 − 2.62} ≈ 0.11.
- A +10° upslope gives p_s = e^{0.78} ≈ 2.2.
- Clamp p_burn to [0, 1].

**Emergence Lab extensions** (our design):
- Fuel moisture: multiply by f_m = max(0, 1 − M_fuel / M_x), where M_x is the moisture of extinction (Rothermel concept; about 0.12–0.40 by fuel type, UNVERIFIED).
- Burn duration ∝ fuel load, instead of a single tick.
- Intensity feeds into tree mortality and into soil N loss.
- Ignition sources: lightning (rate ∝ storm events × dryness) and agents (civilization mode).
- Use a deterministic per-(cell, tick) hash RNG so the result is independent of iteration order.

### 5.2 Drossel–Schwabl forest-fire model [K]

Drossel & Schwabl 1992, *PRL* 69:1629, doi:10.1103/PhysRevLett.69.1629.

Rules:
- An empty cell becomes a tree with probability p.
- A tree ignites by lightning with probability f.
- Fire spreads to neighboring trees.

When f ≪ p the model is self-organized critical: fire sizes follow a power law. Use it as a cheap "fire regime" baseline and as a teaching scenario in the CA lab.

### 5.3 Rothermel (1972) concepts [K]

Rothermel, R. C. (1972). *A mathematical model for predicting fire spread in wildland fuels.* USDA Forest Service Research Paper INT-115.
Comprehensive modern explanation: Andrews, P. L. (2018), USDA FS RMRS-GTR-371.

**Steady surface rate of spread:**

```
R = I_R · ξ · (1 + φ_w + φ_s) / (ρ_b · ε · Q_ig)
```

| Term | Meaning | Classic (English-unit) form |
|---|---|---|
| I_R | reaction intensity (heat release per area per time) | depends on fuel load, heat content, moisture damping and mineral damping |
| ξ | propagating flux ratio (share of I_R that heats fuel ahead) | — |
| φ_w | wind factor | C · U^B · (β/β_op)^{−E}; C, B, E are functions of the surface-area-to-volume ratio σ |
| φ_s | slope factor | 5.275 · β^{−0.3} · tan²φ |
| ρ_b | bulk density | — |
| ε | effective heating number | exp(−138/σ) |
| Q_ig | heat of preignition | 250 + 1116 · M_f (Btu/lb) |

**How to use it in a grid:**
1. Pre-compute R per (fuel model, moisture class, wind class, slope class) offline into a table.
2. Per tick, convert R to an ignition probability for each neighbor: p = min(1, R_dir · Δt / D).
3. Use a Huygens/elliptical shape for direction dependence: max spread downwind, flanks at a reduced rate.

Related formulas:
- Byram fireline intensity: I = H · w · R.
- Flame length L_f ≈ 0.0775 · I^{0.46} (SI units). Use it for visuals and tree mortality.

### 5.4 Regeneration and succession after fire [K]

- **Mechanisms** (Connell & Slatyer 1977, *Am. Nat.* 111:1119, doi:10.1086/283241): facilitation, tolerance and inhibition. Clementsian succession towards a "climax" is the classical view.
- **Cheap model.** A Markov state-transition chain per cell (Horn 1975) has transition probabilities conditioned on seed sources within the dispersal kernel, light, moisture and time since disturbance:
  bare/ash → herbs and grasses (seasons) → shrubs (years) → pioneer trees, e.g. birch or pine (decades) → late-successional trees, e.g. oak or beech (decades to centuries).
- **Better model.** PFT biomass layers with Beer–Lambert light competition (section 4.2). Succession then *emerges*: fast-growing, shade-intolerant PFTs dominate early, and slow, shade-tolerant PFTs dominate late.
  Fire resets the canopy and gives a nutrient pulse (mineral P and some N to the soil; most N volatilizes).
- **Fire traits by PFT:**
  - resprouters (grasses, many shrubs, oaks) regrow from surviving roots
  - obligate seeders need a seed source
  - serotinous cones (some pines) release seed after fire
  - thick bark (pines, oaks) improves survival of surface fires
  - flammability (resinous conifers, dry grass) raises p_veg
- **Fire-return intervals** (order of magnitude, UNVERIFIED): grassland/savanna about 1–10 yr, Mediterranean shrubland about 10–40 yr, boreal forest about 50–200 yr.
  Calibrate lightning frequency against these.
- Post-fire **erosion risk** rises (bare soil): couple this to the hydraulic erosion model (section 6.2).

---

## 6. Hydrology and terrain

### 6.1 Heightmaps

- **fBm (fractional Brownian motion) noise**: h(p) = Σ_{o=0}^{O−1} g^o · noise(λ^o · f · p).
  Parameters: lacunarity λ, gain/persistence g (H = −log_λ g), O octaves.
  Musgrave, Kolb & Mace 1989, *SIGGRAPH* (doi:10.1145/74334.74337). Perlin 2002 improved noise (doi:10.1145/566654.566636) [K].
- The local `noise 0.9.0` crate (MIT/Apache-2.0) provides [V, read in `~/.cargo/registry`]:
  - generators: Perlin, OpenSimplex, SuperSimplex, Value, Worley
  - fractals: Fbm, Billow, BasicMulti, HybridMulti, RidgedMulti
  - transformers: Turbulence, Displace (domain distortion)
  - Defaults: Fbm has 6 octaves, frequency 1.0, **lacunarity 2π/3 ≈ 2.094** (not 2.0, which avoids lattice alignment artifacts) and persistence 0.5. RidgedMulti uses persistence 1.0.
- **Ridged multifractal** for mountain chains: 1 − |noise|, with octave weighting by the previous octave's value (Musgrave, in Ebert et al., *Texturing & Modeling*) [K].
- **Domain warping**: f(p + s · fbm(p + s · fbm(p))), after Inigo Quilez, https://iquilezles.org/articles/warp/ [K]. It gives organic, eroded-looking shapes cheaply.
- Recommended pipeline:
  1. continent mask (low-frequency noise plus a radial falloff, or a plate seed)
  2. warped fBm
  3. ridged noise weighted by a "mountain mask"
  4. thermal erosion (talus angle)
  5. hydraulic erosion
  6. depression fill / lake detection

### 6.2 Erosion

- **Thermal erosion** (Musgrave 1989) [K]: if the slope to a neighbor exceeds the talus angle, move c · (Δh − T) of material downhill.
- **Hydraulic erosion, grid version** (Mei, Decaudin & Hu 2007) [V], in five steps per iteration:
  1. rain: d ← d + Δt · r
  2. pipe flow (section 6.4)
  3. erosion/deposition with sediment capacity **C = K_c · sin(α) · |v|**. If C > s, dissolve K_s(C − s); otherwise deposit K_d(s − C). Use a minimum α so flat terrain still erodes.
  4. semi-Lagrangian sediment advection: s(x) ← s(x − v Δt)
  5. evaporation: d ← d · (1 − K_e Δt)

  Paper: *Fast Hydraulic Erosion Simulation and Visualization on GPU*, Pacific Graphics 2007, pp. 47–56, doi:10.1109/PG.2007.15. Open copy: https://inria.hal.science/inria-00402079
- **Particle ("droplet") erosion** (Beyer 2015, TU München BSc thesis; popularized in many game-dev implementations) [K]: simulate many droplets with inertia, capacity, erosion radius and evaporation.
  It parallelizes poorly but is deterministic if droplets are processed in a fixed order. For world generation it is fine single-threaded or with tiled batches.

### 6.3 Depressions, rivers and lakes [K]

- **Depression filling.**
  - Planchon & Darboux 2002, *Catena* 46:159–176 (doi:10.1016/S0341-8162(01)00164-3): iterative; floods the DEM then drains.
  - **Priority-Flood** (Barnes, Lehman & Mulla 2014, *Computers & Geosciences* 62:117–127, doi:10.1016/j.cageo.2013.04.024; arXiv:1511.04463; also Wang & Liu 2006): O(n log n).
    1. Seed a min-heap with all edge and ocean cells, marked closed.
    2. Pop the lowest cell c.
    3. For each unclosed neighbor n: set z_n ← max(z_n, z_c + ε) (ε > 0 yields a drainable surface), close it, and push it.

    Use a deterministic tie-break (z, then the linear index).
    **Lake depth** = z_filled − z_original (with ε = 0 for the water level).
- **Flow direction, D8** (O'Callaghan & Mark 1984, doi:10.1016/S0734-189X(84)80011-0): each cell drains to the neighbor with steepest descent (Δz / distance, using √2 for diagonals).
  Alternatives: D∞ (Tarboton 1997, *WRR* 33:309) and MFD (Freeman 1991) for smoother, divergent flow.
- **Flow accumulation.** Process cells in descending filled elevation (or in topological order by in-degree), and add each cell's area × runoff to its receiver.
  **River** where accumulation > threshold. Channel width ∝ Q^{0.5} (hydraulic geometry, Leopold & Maddock 1953) [K].
- **Lakes.** Filled basins become lakes when inflow ≥ evaporation × lake area. Otherwise lower the lake level until the water balance holds (an endorheic/salt lake or playa). The outlet is the spill point found by Priority-Flood.

### 6.4 Virtual-pipe shallow-water model (Mei et al. 2007, after O'Brien & Hodgins 1995) [V]

Per cell store: terrain b, water d, sediment s, outflow fluxes f = (f_L, f_R, f_T, f_B), and velocity v = (u, v).

```
Δh_L   = b + d1 − b(x−1,y) − d1(x−1,y)
f_L'   = max(0, f_L + Δt · A · g · Δh_L / l)            # likewise R, T, B
K      = min(1, d1 · l_X · l_Y / ((f_L'+f_R'+f_T'+f_B') · Δt))
f_i   ← K · f_i'                                         # prevents negative water
ΔV     = Δt · (Σ inflow − Σ outflow)
d2     = d1 + ΔV / (l_X l_Y)
ΔW_X   = (f_R(x−1,y) − f_L(x,y) + f_R(x,y) − f_L(x+1,y)) / 2
u      = ΔW_X / (l_Y · d̄),  d̄ = (d1 + d2)/2              # likewise v
```

- Boundaries: zero outflow across walls. Use open boundaries for sea outlets: our extension is to set ocean cells as sinks.
- Stability: CFL-like, with Δt · u ≤ l_X and Δt · v ≤ l_Y. Also keep Δt ≤ l / √(g d_max) [K]. Shrink Δt when cells get smaller.
- Four von Neumann pipes suffice according to the authors.
- **Use for:** flash floods, dam breaks (beaver dams!), lake filling and tides, and erosion.
- **Determinism:** double-buffer the fields, compute each cell from the previous buffer only, and use fixed-point water if exact mass conservation is required.

### 6.5 Rain, infiltration, evaporation [K]

- **Precipitation field.**
  - Start from a latitude baseline: wet near the equator (ITCZ), dry near ±30° (subtropical highs), wet in mid-latitude storm tracks (≈ 45–60°), dry at the poles.
  - Then advect moisture along the prevailing wind and rain it out when lifted:
    `m_{i+1} = m_i + E_ocean − P_i`, with `P_i = m_i · (k_base + k_oro · max(0, Δh/Δx))`.
    This gives **rain shadows** for free (our design).
- **Infiltration.**
  - Horton: f(t) = f_c + (f_0 − f_c) e^{−k t}.
  - Or the SCS curve number: S = 25400/CN − 254 (mm), and Q = (P − 0.2 S)² / (P + 0.8 S) for P > 0.2 S, else 0.
  - CN depends on soil and cover. Burned or bare soil has a higher CN, which gives more runoff.
- **Potential evapotranspiration.**
  - Hargreaves & Samani (1985): ET₀ = 0.0023 · R_a · (T_mean + 17.8) · √(T_max − T_min), in mm/day, with R_a expressed as an evaporation equivalent (MJ m⁻² d⁻¹ × 0.408).
  - Priestley–Taylor is an alternative.
  - Open water evaporates at about ET₀. Soil evaporates at ET₀ × f(moisture).
- **Snow.** Accumulate when T < 0 °C. Melt with a degree-day factor (about 2–6 mm °C⁻¹ d⁻¹, UNVERIFIED).

### 6.6 Climate fields [K]

- Lapse rate: T(z) = T_sea − Γ · z, with Γ ≈ 6.5 K/km (standard atmosphere).
- Annual mean T falls with latitude. Seasonal amplitude rises with latitude and with distance from the ocean (continentality).
  Lag the temperature behind insolation by about 1 month over land (more over ocean) (UNVERIFIED value).

### 6.7 Biome classification (Whittaker 1975 style)

Whittaker (1975), *Communities and Ecosystems*, plots biomes against mean annual temperature (T) and precipitation (P) [K].
The widely used digitized polygons (R package **plotbiomes**, MIT code, https://github.com/valentinitnelav/plotbiomes) come from Ricklefs' textbook figure, whose copyright is unclear [V]. So we **do not embed those polygons**.
Our own simplified, tunable thresholds (UNVERIFIED approximations, adjust visually):

| T (°C) | P < 30 cm | 30–60 | 60–100 | 100–150 | 150–250 | > 250 |
|---|---|---|---|---|---|---|
| < −5 | tundra / polar desert | tundra | tundra | tundra | tundra | tundra |
| −5 to 3 | cold steppe / tundra | boreal forest | boreal forest | boreal forest | boreal forest | boreal forest |
| 3 to 20 | temperate desert | temperate grassland | woodland / shrubland (Mediterranean if T > 12 with dry summers) | temperate seasonal forest | temperate seasonal forest; above 220 cm temperate rainforest | temperate rainforest |
| ≥ 20 | subtropical desert | savanna / thorn scrub | savanna | savanna / seasonal forest | tropical seasonal forest | tropical rainforest |

- Smooth the boundaries with blending weights for rendering.
- The **Holdridge life zones** (Holdridge 1947, *Science* 105:367) use biotemperature, precipitation and the PET ratio on a log scale. They are a deterministic alternative that also handles aridity.
- Altitudinal zonation follows automatically from the lapse rate.

### 6.8 Seasons and day–night insolation [K]

Day of year N. Latitude φ.

```
δ  = 23.44° · sin(2π (284 + N) / 365)                 # declination (Cooper 1969)
E0 = 1 + 0.033 · cos(2π N / 365)                       # Earth–Sun distance factor
h0 = arccos(clamp(−tan φ · tan δ, −1, 1))              # sunset hour angle (0 = polar night, π = polar day)
daylength_h = 24 · h0 / π
Q_day (W/m², TOA daily mean) = (S0/π) · E0 · (h0 sin φ sin δ + cos φ cos δ sin h0)
cos Z(t) = sin φ sin δ + cos φ cos δ cos h,   h = 2π (t_solar/24 − 0.5)
I(t) = S0 · E0 · max(0, cos Z) · τ_atm                  # τ_atm ≈ 0.7–0.75 clear sky (UNVERIFIED)
S0 ≈ 1361 W/m² (Kopp & Lean 2011, GRL, TSI ≈ 1360.8)
```

- Drive plant f_light, temperature cycles, animal activity cycles (nocturnal, diurnal, crepuscular), and snowmelt from these.
- For game worlds, allow a configurable axial tilt and year length.

---

## 7. Agent AI

### 7.1 Recommended layering

| Layer | Question | Technique | Rate |
|---|---|---|---|
| Needs | what do I lack? | homeostatic drives (hunger, thirst, fatigue, fear, temperature, reproduction) | every tick |
| Decision | what should I do? | **utility AI** (Mark) | every ~5–20 ticks, staggered by `id % k` |
| Execution | how do I do it? | small **behavior tree** or **HFSM** per action (e.g. Hunt: search → stalk → chase → kill → eat) | every tick |
| Locomotion | how do I move? | **Reynolds steering** with a blended or priority arbiter | every tick |
| Perception | what is near me? | spatial hash query within perception radius and FOV, with occlusion optional | every tick or decimated |

### 7.2 Utility AI [K]

Mark, D. (2009), *Behavioral Mathematics for Game AI* (Course Technology). Dill & Mark, *Improving AI Decision Modeling Through Utility Theory*, GDC 2010.
The **Infinite Axis Utility System** was presented at GDC AI Summit talks in 2013 and in *Building a Better Centaur: AI at Massive Scale* (2015). See https://gameai.com/iaus.php [V for existence and description].

- Each action has a list of **considerations**. Each one maps an input (normalized to [0, 1]) through a **response curve**:
  - linear
  - polynomial: y = m (x − c)^k + b
  - logistic: y = 1 / (1 + e^{−k(x − x0)})
  - logit
- The action score is the **product** of its consideration scores, times an action weight. Pick the argmax, or do weighted random among the top-N, using the agent's own RNG stream.
- **Compensation factor.** Products shrink as consideration count n grows. The commonly cited IAUS correction is:
  `mod = 1 − 1/n; make_up = (1 − s) · mod; s' = s + make_up · s`.
  This specific formula is from memory of the GDC 2015 talk and is **UNVERIFIED**.
- Add **momentum**: a bonus of about 10–25 % for the current action, which prevents dithering.
- Example (herbivore "Flee"):
  `score = logistic(threat_proximity) × logistic(predator_is_hunting) × (1 − 0.5 · exhaustion)`.
  Other herbivore actions: Graze, Drink, Rest, Seek mate, Follow herd, Seek shade, Hide.
- Genes can set response-curve parameters (x0, k), so **behavior evolves** (section 8).

### 7.3 Behavior trees, FSMs and GOAP [K]

- **Behavior trees.**
  - Origin in games: Isla 2005, *Handling Complexity in the Halo 2 AI*, GDC 2005. Formal treatment: Colledanchise & Ögren 2018, *Behavior Trees in Robotics and AI*, arXiv:1709.00084.
  - Nodes: Sequence, Selector/Fallback, Parallel, Decorators (Inverter, Repeat, Cooldown), Conditions, Actions.
  - Status values: Success, Failure, Running.
  - Keep a tick-based, data-driven representation (RON) so that species definitions can carry trees.
- **FSM / HFSM** (statecharts, Harel 1987): best for life-stage and physiological modes, e.g. egg → larva → adult, hibernating, rutting, nesting.
- **GOAP** (Orkin 2006, *Three States and a Plan: The AI of F.E.A.R.*, GDC 2006): planning over symbolic world state. It suits the civilization mode (settlers planning build chains), but is too costly for thousands of animals.

### 7.4 Steering and boids (Reynolds) [V]

- Reynolds 1987, *Flocks, herds and schools: a distributed behavioral model*, SIGGRAPH, Computer Graphics 21(4):25–34, doi:10.1145/37402.37406 [K for bibliographic details].
- Reynolds 1999, *Steering Behaviors for Autonomous Characters*, GDC 1999: https://www.red3d.com/cwr/steer/gdc99/ [V].
- **Vehicle model**: point mass with mass, position, velocity, max_force, max_speed and orientation. Each step:

  ```
  steering_force = truncate(steering, max_force)
  velocity       = truncate(velocity + steering_force / mass, max_speed)
  position      += velocity
  ```
- **Seek**: desired = normalize(target − position) · max_speed; steering = desired − velocity.
  The printed paper line has the subtraction reversed relative to its own prose, so check the sign.
- **Flee**: the opposite of seek.
- **Arrival**: ramped_speed = max_speed · (distance / slowing_distance), clipped to max_speed.
- **Pursuit**: seek the quarry's predicted position, position + velocity · T, with T = D · c (D = distance, c = turning parameter). Reduce T when the pursuer is aligned with and in front of the quarry.
- **Evasion**: flee from the pursuer's predicted position.
- **Wander**: a random displacement on a sphere (or circle) ahead of the agent. Radius sets strength; the displacement step sets rate.
- **Flocking** (neighbors within distance *and* angular field of view):
  - **separation**: Σ normalized offset / r
  - **cohesion**: seek the neighbors' centroid
  - **alignment**: average neighbor velocity − own velocity
- Also: obstacle avoidance, containment, path following, leader following, unaligned collision avoidance, flow-field following.
- **Combination**: either a weighted sum, or **prioritized dithering / allocation**. Rule: avoid collision first, then flee, then flock, then wander.

**Herds, flocks and schools:** tune separation, cohesion and alignment radii and weights by species.
- Fish schools: strong alignment, small separation radius.
- Ungulate herds: cohesion towards the herd centre, plus a "selfish herd" bias towards the centre when a threat is perceived (Hamilton 1971, *J. Theor. Biol.* 31:295) [K].
- Bird flocks in 3D: the same rules with banking.

### 7.5 Predator–prey pursuit and evasion details (design)

- **Detection**: the probability of detection per tick rises with proximity, prey movement and size, and falls with cover (vegetation density in the cell). Raptors get a vision multiplier; canids get a smell multiplier with wind direction.
- **Stalk → chase**: the predator approaches at low speed (lower detection) until within the attack radius, then sprints.
- **Endurance**: sprint stamina drains, so chases end after a stamina budget. Cheetah-like predators are fast with short endurance; wolves are moderate with long endurance (coursing).
- **Prey responses**:
  - vigilance (alarm propagates through the herd via neighbor queries)
  - flee using evasion of the predicted position
  - zig-zag when the predator is near, exploiting its turn radius
  - seek refuge (burrow, water, dense cover)
- **Capture success**: logistic in (predator speed − prey speed), plus a relative size term, plus a group-defense term (bison or musk oxen form defensive circles).
- **Holling handling time**: after a kill, the predator stays to eat for h ticks. Carcass biomass remains for scavengers (ravens, foxes, bears).

### 7.6 Spatial hashing for neighbor queries

- **Uniform grid / cell list** (as in SPH; Teschner et al. 2003, *Optimized Spatial Hashing for Collision Detection of Deformable Objects*, VMV) [K].
  - Cell size s ≈ the common query radius.
  - Key = (⌊x/s⌋, ⌊y/s⌋[, ⌊z/s⌋]).
  - Build each tick with a counting sort: count per cell → prefix sum → scatter agent indices in **id order**. This is deterministic, O(N), and cache-friendly.
  - A query visits ⌈r/s⌉ rings of cells: 3×3 in 2D, 3×3×3 in 3D for r ≤ s.
- Bounded world: use a dense array of cell starts. Unbounded world: use a hash table keyed by the cell coordinate, but iterate results by sorted agent id, never by hash order.
- For radii that differ widely (mouse versus elephant), use 2–3 grid levels, or query with ring count = ⌈r/s⌉.
- Keep separate grids per layer (terrestrial, aerial, aquatic, plants) if their densities differ by orders of magnitude.

### 7.7 Deterministic conflict resolution

The problem: Sugarscape's own authors note that a parallel version of movement rule M "would have to be supplemented with a conflict resolution rule", and the formal spec flags the same issue for mating (Kehoe 2016) [V].
Synchronous and asynchronous updating can produce different outcomes (Huberman & Glance 1993, *PNAS* 90:7716) [K]. Pick one, document it, and make it reproducible.

**Pattern A: seeded sequential order** (NetLogo and the original Sugarscape style).
Shuffle the agent list each tick with the world RNG, then act serially. It is simple and deterministic, but it cannot run in parallel and has first-mover advantages.

**Pattern B: intent → sort → resolve → apply** (recommended for parallel Rust with rayon).
1. **Sense/decide (parallel, read-only snapshot).** Each agent emits `Intent { kind, target, priority_key, agent_id, payload }`. rayon's `par_iter().map().collect::<Vec<_>>()` preserves index order.
2. **Sort** intents by `(kind, target, priority_key desc, tiebreak, agent_id)`. The key must be a **total order**, so the agent id makes it unique.
   For fairness use `tiebreak = hash64(world_seed, tick, agent_id, target)` (SplitMix64 or a ChaCha block). Plain ascending id would systematically favor old agents.
3. **Resolve per target group.**
   - *Indivisible resource* (prey, cell slot, mate): the first claimant wins. Losers get a fallback: stay, or take their second choice in a bounded number of extra rounds (like iterated matching).
   - *Divisible resource* (grass, water, carcass): **pro-rata sharing**, share_i = R · demand_i / Σ demand. This is order-free. Use integer arithmetic with deterministic remainder distribution sorted by key.
   - *Mutual pairing* (mating, trade, combat): greedy maximal matching over edges sorted by key, or Gale–Shapley deferred acceptance for stable matches. Each agent takes at most one partner per sub-step.
4. **Apply** effects. Run serially, or in parallel over disjoint partitions (e.g. grouped by cell), never with racing writes.

**Priority keys** should be ecologically meaningful, for example:
- hunger-weighted urgency
- dominance rank
- distance (closer wins)
- predators before scavengers at a carcass
- territory owner before intruder

**RNG.**
- Use one counter-based stream per agent. `rand_chacha 0.10` exposes `set_stream(u64)` and `set_word_pos(u128)` [V, local registry]: seed with the world seed, set `stream = agent_id`, and set the word position from the tick. Or derive a seed via `hash(world_seed, agent_id, tick)`.
- The draws then never depend on thread scheduling or on how many other agents drew.

**Containers.**
- Never iterate `std::collections::HashMap` / `HashSet` in simulation logic. Their `RandomState` order differs between runs.
- Use `Vec` plus sort, `BTreeMap`, or an index-ordered map.

**Floating point.**
- Parallel reductions (`rayon::reduce`/`sum`) over `f32`/`f64` can vary because work-stealing changes the association order.
- Use fixed-point integers for conserved stocks, or reduce per fixed chunk in index order.
- Transcendental functions (`exp`, `sin`) can differ across platforms. If cross-platform replay matters, use a pure-Rust `libm` implementation (UNVERIFIED crate version).

**Births and deaths.**
- Buffer them and apply at the end of the tick.
- Assign new ids from a monotonically increasing counter **in sorted parent order**, never from thread-local counters.

---

## 8. Evolution

### 8.1 Genotype → phenotype mapping (design)

- **Genome** = `Vec<Gene>`. A Gene is either:
  - real-valued in [0, 1], mapped to a trait range (log-scale for scale-like traits such as body mass, perception and litter size), or
  - a discrete allele (u8) at diploid loci, for Mendelian traits and neutral markers.
- **Trade-offs come through the allometry layer (section 3.4), not through free knobs.**
  - Bigger M gives higher top speed (up to the Hirt hump), larger fasting endurance and more prey options, but higher absolute food need, longer maturity and fewer offspring.
  - Speed cost per tick ∝ v² (UNVERIFIED exponent; designer).
  - Perception has a metabolic cost.
  - Offspring count n splits a fixed reproductive energy budget, so each offspring gets energy E_r/n. This is the r/K trade-off and must be explicit.
- **Pleiotropy** (optional): phenotype z = W · g. A sparse matrix W couples traits, giving correlated responses to selection.
- **Developmental noise**: z = f(g) + N(0, σ_env²). This makes heritability < 1 and is measurable.
- **Behavior genes**: utility response-curve parameters, diet preference weights, sociality (cohesion weight) and aggression.

### 8.2 Mutation models

- **Gaussian per-gene mutation**: with probability μ per gene (e.g. 0.01–0.1), g' = g + N(0, σ²), with σ ≈ 0.02–0.1 of the range (UNVERIFIED defaults).
- **Bounds**:
  - *Clamping* piles mass at the bounds (artifact).
  - *Reflection* (g' = −g' if < 0; 2 − g' if > 1) avoids pile-up.
  - *Logit-space* mutation (mutate x = logit(g)) never leaves bounds and gives finer resolution near the edges.
  - For mass, mutate ln M additively (log-normal).
- **Self-adaptive σ** (evolution strategies; Schwefel, Rechenberg) [K]: σ' = σ · exp(τ · N(0, 1)) with τ ∝ 1/√n.
- **Discrete loci**: allele flip with a per-locus rate μ_locus.
- **Structural mutations** for Rhai/DSL rule genomes: insertion, deletion, duplication (as in NEAT add-node/add-link; Stanley & Miikkulainen 2002) [K].
- **Determinism**: draw the mutation RNG from the offspring's own stream, keyed by (seed, child_id).

### 8.3 Recombination [K]

- Real genes:
  - uniform crossover (each gene from either parent with p = 0.5)
  - intermediate: g = w · g₁ + (1 − w) · g₂, w ~ U(0, 1) or fixed 0.5
  - BLX-α (Eshelman & Schaffer 1993): sample in the expanded interval
- Diploid loci: Mendelian segregation. Each parent passes one of its two alleles. Optionally link loci on "chromosomes" with a crossover rate per interval.
- Sugarscape style: the child takes each genetic attribute from one parent at random [V, Kehoe spec, mating rule S].
- Asexual species (Daphnia, many plants, fungi): clonal with mutation. Switch to sexual reproduction under stress (cyclical parthenogenesis) as an emergent strategy.

### 8.4 Selection (measure it, don't impose it) [K]

- Natural selection emerges from differential survival and reproduction. Measure:
  - **Selection differential** S = z̄_reproducing parents − z̄_population.
  - **Response** R = z̄_offspring − z̄_population.
  - **Breeder's equation**: R = h² S (Lush 1937).
- **Price equation** (Price 1970, *Nature* 227:520): Δz̄ = Cov(w, z) / w̄ + E(w Δz) / w̄. Log the covariance term per trait per generation window. It is a clean "selection pressure" readout.
- Modes: directional, stabilizing, disruptive. Disruptive selection plus assortative mating leads to sympatric speciation.
- Lab mode (optional, explicit): artificial selection (choose parents by a fitness function), and tournament or truncation selection, for the "evolution lab" experiments.

### 8.5 Genetic drift: Wright–Fisher [K]

- Wright–Fisher (Fisher 1930; Wright 1931): non-overlapping generations, population N diploid (2N gene copies).
  Next-generation allele count ~ Binomial(2N, p).
  - Expected heterozygosity decays as **H_t = H₀ (1 − 1/(2N_e))^t**.
  - A neutral allele fixes with probability p₀. Mean time to fixation of a new neutral mutant is ≈ 4N_e generations.
  - With selection: fixation probability u(p) = (1 − e^{−4N_e s p}) / (1 − e^{−4N_e s}) (Kimura 1962).
- **Moran model**: overlapping generations, one birth and one death per step. It is closer to a continuous agent sim.
- **Engine test**: add 2–4 **neutral marker loci**, run a closed population of size N, and check the H_t decay curve against the formula. This catches RNG or inheritance bugs.

### 8.6 Heritability [K]

- Narrow-sense h² = V_A / V_P.
- Estimate in the sim:
  - by regressing offspring on the mid-parent trait (slope = h²), or
  - on a single parent (slope = h²/2).
- Log (parent trait, offspring trait) pairs to do this.

### 8.7 Allele frequencies and diversity [K]

- Allele frequency at a locus: p_i = count_i / (2N). Bin real-valued genes into k bins for "allele" displays.
- Expected heterozygosity H_e = 1 − Σ p_i². Compare with the observed heterozygosity H_o (the share of heterozygous individuals).
- F_ST = (H_T − H_S) / H_T between subpopulations. Use it for geographic structure and incipient speciation.
- Species diversity:
  - richness S
  - **Shannon** H' = −Σ p ln p; Pielou evenness J = H'/ln S
  - **Simpson** λ = Σ p²; Gini–Simpson 1 − λ; inverse Simpson 1/λ
  - **Hill numbers** ^qD = (Σ p_i^q)^{1/(1−q)} (q = 0 richness, q → 1 exp(H'), q = 2 inverse Simpson) (Hill 1973, *Ecology* 54:427)
- Genetic diversity for continuous genomes: mean pairwise genetic distance (an analog of nucleotide diversity π), plus trait variance per species.

### 8.8 Observable speciation criteria (design)

1. **Genetic distance**:
   d(i, j) = √(Σ_k w_k (g_ik − g_jk)²) over normalized genes, plus Hamming distance on discrete loci, weighted.
   For rule genomes, use a NEAT-like compatibility distance: δ = c₁ E/N + c₂ D/N + c₃ W̄. NEAT's XOR defaults were c₁ = c₂ = 1.0, c₃ = 0.4, δ_t = 3.0 (Stanley & Miikkulainen 2002, *Evol. Comput.* 10:99) [K].
2. **Clustering**: assign each individual to the first species whose **representative** (a random member from the previous census) is within θ_species. Otherwise found a new species (NEAT-style, O(N·S), and IDs stay stable).
   Alternatives: connected components of the mating-compatibility graph, or DBSCAN on genomes.
3. **Reproductive isolation**:
   - Mating is allowed only if d(i, j) < θ_mate (assortative mating).
   - Optionally, hybrid fitness falls with d (Dobzhansky–Muller-like penalty).
   - Report "biological species" as groups with no successful interbreeding in the last W generations.
4. **Hysteresis**: declare a split only if two clusters stay separated (between-cluster distance > θ, gene flow < ε) for ≥ T generations. This prevents species-count flicker.
5. **Mode tag**: allopatric if the clusters' spatial ranges are disjoint (no migrants for T generations); otherwise sympatric or parapatric.

Prior art: EcoSim (Gras et al. 2009, *Artificial Life* 15:423) uses genetic-distance-based species in a predator–prey individual-based model. PolyWorld (Yaeger 1994) [K].

### 8.9 Phylogeny tracking

- Track **taxa** (species or genotype clusters), not every individual. Each node holds:
  `{id, parent_id, origin_tick, extinction_tick, representative_genome, peak_population, cumulative_births}`.
- **Prune** extinct taxa that have no living descendants (as Phylotrack does). This keeps memory proportional to the living lineages.
- **Phylotrack** (Dolson, Rodriguez-Papa & Moreno 2024, arXiv:2405.09389): a C++ header-only library with a Python wrapper, MIT-licensed [V].
  Reimplement the concepts in Rust: a systematics manager with add-org/remove-org callbacks, MRCA tracking and pruning. Do not depend on C++.
- For massive or distributed runs: **hereditary stratigraphy** (Moreno, Dolson & Ofria 2022, *hstrat*) [K] reconstructs phylogenies approximately from per-genome "stratum" annotations, with no central tracker.
- **Outputs**:
  - Muller plot (abundance of lineages over time)
  - phylogenetic tree (Newick export)
  - Bedau–Packard **evolutionary activity** (cumulative usage of components that persist, compared against a neutral shadow model; Bedau & Packard 1992, *Artificial Life II*) [K]. This connects directly to Flow-Lenia's analysis (section 2).

---

## 9. Agent-based societies

### 9.1 Sugarscape (Epstein & Axtell 1996, *Growing Artificial Societies*, Brookings/MIT Press)

The rules below are as formalized in Kehoe, J. (2016), *The Specification of Sugarscape*, arXiv:1505.06012 (Z notation) [V]. Mesa's G1MT example confirms the trade formulas [V]: https://mesa.readthedocs.io/stable/examples/advanced/sugarscape_g1mt.html

**Agent state:** position, sex, vision, metabolism (per resource), sugar (and spice), age, max age, culture tag bits (odd length), immunity bits, diseases, children, loans.

**Rules:**

- **G_α (growback)**: each site regrows α units per tick, up to its capacity (maxSugar).
- **S_{α,β,γ} (seasonal growback)**: the halves of the map alternate summer and winter every γ ticks. The winter rate is α/β.
- **M (movement)**:
  1. Look along the 4 lattice directions up to *vision*. The search order is random to break ties.
  2. Among unoccupied sites, pick the **nearest site with maximal welfare**.
  3. Move there and harvest everything.
  4. **Metabolism** is deducted each tick. The agent dies at 0 sugar.
- **Welfare**, two goods: Cobb–Douglas, W = w₁^{m₁/m_T} · w₂^{m₂/m_T}, with m_T = m₁ + m₂ (book p. 97 per Mesa).
  With pollution, the movement welfare becomes sugar / (1 + pollution).
- **P (pollution)**: p_t = p_{t−1} + α·s (gathered) + β·m (metabolized).
- **D (pollution diffusion)**: diffuse to the 4 neighbors.
- **R_[a,b] (replacement)**: when an agent dies, a new random agent appears with max age drawn from U[a, b]. New agents get initial sugar in [5, 25].
- **S (sex)**:
  - Pick a random von Neumann neighbor. Mate if opposite sex, both fertile, and at least one parent has an empty neighboring site.
  - The child inherits each genetic attribute from either parent and crosses over culture.
  - The book adds that **each parent must hold at least its initial endowment and gives half to the child**.
  - Fertility windows:
    - females start at 12–15 and end at 40–50
    - males start at 12–15 and end at 50–60
    - male end = female end + 10
- **I (inheritance)**: on death, wealth is split equally among living children.
- **K (culture)**:
  - Pick a random neighbor and a random tag position. If they differ, the **neighbor's** tag flips to match.
  - Tribe = majority bit: 0s mean Blue, 1s mean Red.
- **C_α (combat)**:
  1. Look along the 4 directions within vision.
  2. Discard sites occupied by your own tribe, and sites occupied by richer agents of another tribe.
  3. Reward = site resource + min(α, occupant wealth).
  4. Discard sites vulnerable to retaliation.
  5. Move to the nearest site with maximal reward and collect. The occupant is killed.
- **T (trade)**:
  1. MRS = (w₂/m₂) / (w₁/m₁), which is spice per sugar in "time-to-death" units.
  2. If MRS_A ≠ MRS_B, spice flows from the higher-MRS agent to the lower, and sugar the other way.
  3. **Price p = √(MRS_A · MRS_B)** (geometric mean).
  4. If p > 1, exchange p units of spice for 1 sugar; otherwise 1/p units of sugar for 1 spice. Mesa truncates to integers.
  5. Execute only if **both welfares increase and the MRSs do not cross**. Repeat until that fails.
- **L_{d,r} (credit)**:
  - Lenders are agents too old to bear children (who lend half their wealth), or fertile agents with surplus.
  - Borrowers are fertile agents lacking wealth but with positive income.
  - A loan runs d ticks at rate r. On default, the borrower pays half its wealth and the rest is re-lent.
- **E (disease)**:
  - A disease is a bit string. If it is a substring of the immune string, the agent is immune.
  - Otherwise, flip the first differing bit of the closest (minimum Hamming distance) immune substring.
  - Transmit one random disease to each neighbor.

**Defaults.**
- Mesa G1MT: 50 × 50 grid, 200 agents, vision 1–5, metabolism 1–5 per good, endowment 25–50 [V].
- The book's base chapter: 400 agents, vision 1–6, metabolism 1–4, max age 60–100 [K, UNVERIFIED in this session].

**Conflict resolution** is unspecified in the original. Use section 7.7.

### 9.2 Cooperation (Axelrod and successors) [K]

- Iterated Prisoner's Dilemma payoffs T = 5, R = 3, P = 1, S = 0 (T > R > P > S and 2R > T + S).
  Tit-for-Tat won Axelrod's tournaments (Axelrod & Hamilton 1981, *Science* 211:1390; Axelrod 1984, *The Evolution of Cooperation*).
- Spatial games (Nowak & May 1992, *Nature* 359:826): agents on a lattice play neighbors and copy the best-scoring neighbor. Cooperator clusters persist for temptation b in an intermediate range.
  This is a great CA-lab scenario and is sensitive to synchronous versus asynchronous updating (Huberman & Glance 1993).
- **Five rules for cooperation** (Nowak 2006, *Science* 314:1560), each with a threshold condition:

  | Rule | Condition |
  |---|---|
  | kin selection | r > c/b |
  | direct reciprocity | w > c/b |
  | indirect reciprocity (reputation / image scoring) | q > c/b |
  | network reciprocity | b/c > k (k = number of neighbors) |
  | group selection | b/c > 1 + n/m |

  These give a menu of mechanisms for social species and civilizations.
- **Axelrod culture model** (Axelrod 1997, *J. Conflict Resolut.* 41:203): F features × q traits. Neighbors interact with probability equal to their cultural similarity, and copy one differing feature. The result is stable cultural regions. Combine it with Sugarscape tags.

### 9.3 Emergent trade routes and roads (stigmergy)

- **Stigmergy** (Grassé 1959): agents modify the environment and others react to the modification [K].
- **Ant Colony Optimization** (Dorigo, Maniezzo & Colorni 1996, *IEEE SMC-B* 26:29) [K]: pheromone τ ← (1 − ρ) τ + Σ Δτ.
  Next-step choice ∝ τ^α · η^β, where η is a heuristic such as 1/cost.
  The same field drives *Lasius* ant trails in the ecosystem mode.
- **Active-walker trail model** (Helbing, Keltsch & Molnár 1997, *Nature* 388:47) [K]:
  - Ground potential: dG/dt = (G₀ − G)/T_decay + I · (1 − G/G_max) · (footfall).
  - Walkers steer toward the destination, biased by the gradient of trail attractiveness.
  - Human-like trail networks and shortcuts emerge.
- **Civilization roads** (our design):
  - Traders route with A* on cost c = terrain_cost(slope, biome, water) × (1 − k · u/(u + u₀)), where u is the decaying usage counter of the cell.
  - Usage above thresholds upgrades the cell: path → road → paved road. This gives emergent route networks between settlements and resources.
- **Trade volume between settlements**: gravity model, T_ij = k · P_i P_j / d_ij^β (Tinbergen 1962) [K]. Here d is the A* path cost. Prices are set by Sugarscape-style bilateral MRS bargaining at markets.

### 9.4 Territory models

- **Animal territories** [K]:
  - Scent-mark / mechanistic home-range models (Moorcroft & Lewis 2006, *Mechanistic Home Range Analysis*; Potts & Lewis 2014, *Proc. R. Soc. B* 281:20140231).
    Each agent deposits marks that decay. Agents avoid foreign marks and retreat toward their own. Territories emerge, with overlap consistent with Jetz 2004 (section 3.3).
  - Territorial random walkers: Giuggioli, Potts & Harris 2011, *PLoS Comput. Biol.* 7:e1002008.
- **Habitat choice**: the **ideal free distribution** (Fretwell & Lucas 1970), where individuals spread so that per-capita intake is equal across patches. Use it as a validation target. Under despotic dominance, the ideal despotic distribution applies instead.
- **Human / civilization territories**:
  - Weighted Voronoi (Thiessen) cells around settlements.
  - **XTENT** (Renfrew & Level 1979) [K]: cell → argmax_k (C_k^a − k_d · d), claimed only if positive. C_k is a settlement's size or power, and d is the path cost. Smaller centres fall inside large ones' influence, so hierarchies emerge.
  - Borders shift with population and conflict (Sugarscape C rule at group level).
- **Settlement grounding**: the Artificial Anasazi model (Axtell et al. 2002, *PNAS* 99:7275) [K] uses household agents, maize yield from paleoclimate, and fission/fusion of settlements. It is a good template for linking our ecosystem yields to civilization demography.

---

## 10. Species parameter table (suggested starting set)

### 10.1 Method

- Body mass, life history and max longevity come from **AnAge Build 15** (de Magalhães et al.; HAGR, https://genomics.senescence.info/species/; dataset downloaded this session) and **PanTHERIA 1.0** (Jones et al. 2009, *Ecology* 90:2648, Ecological Archives E090-184, https://esapubs.org/archive/ecol/E090/184/).
  Both are [C], extracted from the data files.
- Maximum speeds are **Hirt et al. 2017** dataset entries [V]. "m" means the Hirt-model estimate for the mass. "~" means a hand approximation (UNVERIFIED).
- T_pref for mammals = PanTHERIA mean monthly temperature across the species' geographic range (column `28-2_Temp_Mean_01degC` / 10) [C]. For other groups it is a qualitative thermal niche (UNVERIFIED).
- Max longevity is a record, often from captivity. Mean wild lifespan is much shorter.
- AnAge "adult weight" for fish is often a maximum, so typical masses are given instead (marked ~).
- Life-history duplicates (AnAge versus PanTHERIA) are shown as ranges.
- Simulation-scaled values (Table 10.4) are **derived** deterministically from Tables 10.2–10.3 with the formulas in 10.4. Implementers can regenerate them with `species_tables.py`.

**Trophic role flags** (multi-role allowed):

| Flag | Meaning |
|---|---|
| PRD | producer |
| GRZ | grazer |
| BRW | browser |
| GRN | granivore |
| FRG | frugivore |
| NEC | nectar / pollen (pollinator) |
| INS | insectivore |
| CAR | carnivore |
| PSC | piscivore |
| SCV | scavenger |
| DET | detritivore |
| DEC | decomposer |
| FIL | filter feeder |
| PAR | parasite |
| MUT | mutualist |
| ENG | ecosystem engineer |

### 10.2 Table A: ecology (33 animal species; plants and fungi are in 10.5–10.6)

| id | Species | Group | Roles | Social | Habitat / biomes | T_pref (°C) |
|---|---|---|---|---|---|---|
| mouse | House mouse *Mus musculus* | mammal | GRN, INS (omnivore) | demes, territorial males | grassland, farmland, settlements | commensal, wide (UNVERIFIED) |
| rabbit | European rabbit *Oryctolagus cuniculus* | mammal | GRZ, ENG (warrens) | colonial warrens | temperate grassland, scrub | 10.4 |
| squirrel | Red squirrel *Sciurus vulgaris* | mammal | GRN, FRG, fungivore, egg predation; seed cacher (MUT) | solitary, arboreal | boreal and temperate forest | −3.6 |
| bat | Little brown bat *Myotis lucifugus* | mammal | INS (aerial) | colonial roosts; hibernates | temperate forest near water | n/a |
| fox | Red fox *Vulpes vulpes* | mammal | CAR, SCV, FRG, INS | pairs / family groups | generalist: temperate, boreal, tundra edge, settlements | 3.4 |
| beaver | Eurasian beaver *Castor fiber* | mammal | BRW (bark), aquatic plants, **ENG (dams)** | family colonies | rivers and lakes in temperate and boreal forest | 2.3 |
| wolf | Gray wolf *Canis lupus* | mammal | CAR (large ungulates), SCV | territorial packs | boreal, temperate forest, tundra, grassland | 0.5 |
| boar | Wild boar *Sus scrofa* | mammal | omnivore: FRG (nuts), roots, INS, SCV, fungi; ENG (rooting) | matriarchal sounders | temperate broadleaf forest, scrub, wetland | 11.7 |
| deer | Red deer *Cervus elaphus* | mammal | GRZ + BRW (mixed feeder) | herds, sex-segregated, rut | temperate forest edge, grassland, moor | 2.8 |
| bear | Brown bear *Ursus arctos* | mammal | omnivore: FRG, roots, PSC, CAR, SCV, INS | solitary; hibernates | boreal forest, tundra, mountains | −4.7 |
| lion | Lion *Panthera leo* | mammal | CAR (cooperative), SCV / kleptoparasite | prides | savanna, grassland, open woodland | 23.4 |
| bison | American bison *Bison bison* | mammal | GRZ, ENG (wallows, grazing lawns) | herds (≈ 30 to 1000s) | temperate grassland, boreal meadows | −0.5 |
| elephant | African bush elephant *Loxodonta africana* | mammal | GRZ + BRW + FRG; **ENG** (tree felling, seed dispersal) | matriarchal family herds | savanna, tropical dry forest | 23.1 |
| sparrow | House sparrow *Passer domesticus* | bird | GRN, INS (nestlings) | colonial, flocks | settlements, farmland | wide (UNVERIFIED) |
| raven | Common raven *Corvus corax* | bird | SCV, omnivore, nest predator | pairs; juvenile flocks | tundra to desert, mountains | wide (UNVERIFIED) |
| mallard | Mallard *Anas platyrhynchos* | bird | omnivore: aquatic plants, GRN, INS; seed dispersal | flocks | wetlands, lakes, rivers | wide (UNVERIFIED) |
| peregrine | Peregrine falcon *Falco peregrinus* | bird | CAR (birds in flight) | territorial pairs | cliffs in all biomes | wide (UNVERIFIED) |
| barnowl | Barn owl *Tyto alba* | bird | CAR (rodents, nocturnal, hearing) | pairs | grassland, farmland | ≈ 5–30 (UNVERIFIED) |
| eagle | Golden eagle *Aquila chrysaetos* | bird | CAR (hares, marmots), SCV | territorial pairs | mountains, tundra, steppe | cool (UNVERIFIED) |
| herring | Atlantic herring *Clupea harengus* | fish | FIL / zooplanktivore | schools | temperate–boreal marine shelf | ≈ 2–15 water (UNVERIFIED) |
| trout | Brown trout *Salmo trutta* | fish | INS, PSC | territorial | cold streams and lakes | ≈ 4–19 water (UNVERIFIED) |
| pike | Northern pike *Esox lucius* | fish | PSC (also frogs, ducklings) | solitary ambush | vegetated lakes, slow rivers | ≈ 10–25 water (UNVERIFIED) |
| carp | Common carp *Cyprinus carpio* | fish | omnivore benthic: DET, INS, plants; ENG (turbidity) | loose schools | warm lakes, rivers | ≈ 20–28 water (UNVERIFIED) |
| salmon | Atlantic salmon *Salmo salar* | fish | INS (parr), PSC (sea); nutrient transport upstream | schools at sea | rivers (spawning) ↔ ocean | ≈ 4–18 water (UNVERIFIED) |
| frog | Common frog *Rana temporaria* | amphibian | tadpole: grazer/DET → adult: INS (ontogenetic shift) | solitary; mass spawning | ponds plus temperate forest and grassland; hibernates | ≈ 5–25 (UNVERIFIED) |
| lizard | Sand lizard *Lacerta agilis* | reptile | INS | solitary; basks | heath, dunes, grassland | body ≈ 30 when active (UNVERIFIED) |
| snake | Grass snake *Natrix natrix* | reptile | CAR (amphibians, fish) | solitary; communal egg sites | wetlands, temperate; hibernates | ≈ 10–30 (UNVERIFIED) |
| croc | Nile crocodile *Crocodylus niloticus* | reptile | CAR / PSC ambush, SCV | basking groups | tropical rivers, lakes, swamps | ≈ 25–35 (UNVERIFIED) |
| bee | Honey bee *Apis mellifera* (colony agent) | insect | NEC, **MUT (pollinator)** | eusocial colony | meadows, forest edge, temperate–tropical | brood ≈ 34–35 regulated (UNVERIFIED) |
| ant | Black garden ant *Lasius niger* (colony agent) | insect | omnivore, SCV, aphid farming (MUT), pheromone trails | eusocial colony | temperate grassland, settlements | ≈ 10–35 (UNVERIFIED) |
| earthworm | Earthworm *Lumbricus terrestris* | annelid | DET, **ENG (bioturbation)** | solitary | temperate soils | soil ≈ 5–20 (UNVERIFIED) |
| grasshopper | Grasshopper (Acrididae) | insect | GRZ; outbreak/swarm phase possible (locusts) | solitary ↔ gregarious | grassland, savanna | ≈ 20–40 (UNVERIFIED) |
| daphnia | Water flea *Daphnia* spp. | crustacean | FIL (phytoplankton) | swarms; cyclical parthenogenesis | ponds, lakes | ≈ 10–25 water (UNVERIFIED) |

### 10.3 Table B: literature life-history values

Sources: AnAge (A), PanTHERIA (P), Hirt et al. 2017 (H). "~" means UNVERIFIED approximation.

| id | Mass (kg) | v_max (km/h) | Max longevity (yr) | Maturity, female (yr) | Offspring per event | Events per yr | Gestation / incubation (d) |
|---|---|---|---|---|---|---|---|
| mouse | 0.019–0.021 A,P | 13 H | 4 A (6 P) | 0.12 A (0.18 P) | 5.5–7 | 4.5–5.4 | 19–20 |
| rabbit | 1.6–1.8 A,P | 54 H | 13 A (18 P) | 0.5 P (A lists 2.0) | 5–5.2 | 4.3–4.5 | 30 |
| squirrel | 0.33 P | 19.5 H | 14.8 A | 0.8–1.0 | 4.5–5 | 2 | 38 |
| bat | 0.008–0.010 A,P | 45 m | 34 A | 0.6–0.8 | 1 | 1 | 55–60 |
| fox | 4.1–4.8 A,P | 48 H | 21.3 A | 0.83–0.88 | 4.6–5 | 1 | 52 |
| beaver | 19–25 A,P | ≈ 8 ~ (slow on land) | ≈ 25 P | 1.75 A (2.8 P) | 3 | 1 | 105 |
| wolf | 26.6–31.8 A,P | 62–64 H | 20.6 A (29.5 P) | 1.8 | 5–6 | 0.8–1 | 62–63 |
| boar | 84–130 A,P | 56 H | 21–27 | 0.9–1.0 | 4.5–7 | 1–1.5 | 115 |
| deer | 85–240 (H, A, P; the P value probably includes wapiti) | 72 H | 31.5 A | 1.8–2.3 | 1 | 0.9–1 | 235–245 |
| bear | 196–278 A,P | 35–47 H | 40 A (50 P) | 3.6 | 2–2.2 | 0.4 | 103 A (P: 228, includes delayed implantation) |
| lion | 159–175 A,P | 55–80 H | 28–30 | 2.7–3.0 | 2.75–3 | ≈ 0.5–1 ~ | 109 |
| bison | 625–630 A,P | 52–54 H | 33.5 A | 2.5 | 1 | 1 | 274–283 |
| elephant | 3,800–4,800 A,P | 34–40 H | 65 A (80 P) | 11–13.6 | 1 | 0.2 | 660–670 |
| sparrow | 0.025 A | 59 m | 23 A | 0.42 (F) | 5 | 4 | 11 |
| raven | 1.04 A | 130 m | 69 A | 3 | 5 | 1 | 19 |
| mallard | 1.05–1.14 A,H | 105 H | 29.1 A | 1 | 9 | 1 | 26 |
| peregrine | 0.78–0.84 A,H | 110 H (level flight; stoop far faster, UNVERIFIED) | 25 A | 1 | 3 | 1 | 30 |
| barnowl | 0.5–0.58 A | 119 m (likely high) | 34 A | 1 | 6 | 1–2 | 33 |
| eagle | 4.3–4.8 A,H | 129 H | 48 A | 4 | 2 | 1 | 40 |
| herring | ≈ 0.2 ~ (H entry 0.156) | 6.1 H | 22 A | 3 A | ≈ 2–5 × 10⁴ eggs ~ | 1 | ≈ 10–20 ~ |
| trout | ≈ 0.5–2 ~ | 8–12.6 H | 38 A | ≈ 2–4 ~ | ≈ 2–8 × 10³ eggs ~ (A: 8000) | 1 | ≈ 40–100 ~ (cold) |
| pike | ≈ 1–5 ~ (A max 15.6) | 5–7.6 H | 30 A | 2.4–3.1 A | ≈ 10⁴–10⁵ eggs ~ | 1 | ≈ 10–15 ~ |
| carp | ≈ 2–5 ~ (A 10.6) | 2–6 H (small fish) | 47 A | 3 A | 3 × 10⁵ eggs A | 1 | 4 A |
| salmon | ≈ 3–6 ~ (A max 25.7) | 17.5–21.6 H | 13 A | 5 A (male; female not given) | ≈ 10³–10⁴ eggs ~ | 1 (often semelparous ~) | ≈ 60–120 ~ |
| frog | ≈ 0.02–0.03 ~ | ≈ 4 ~ | 27 A (captive?) | 3 A | 2,585 eggs A | 1 | ≈ 14–21 ~ (hatch) |
| lizard | 0.009 H | 6.0 H | ≈ 11 ~ | ≈ 2 ~ | ≈ 5–15 ~ | 1 | ≈ 40–60 ~ |
| snake | 0.094–0.096 A | ≈ 5 ~ | 20 A | 4 A | ≈ 10–40 ~ | 1 | ≈ 60–70 ~ |
| croc | 94 A (large males ≫ 200 ~) | ≈ 12–14 on land / ≈ 30 swimming ~ | 56 A | 19 A (often 10–12 ~) | ≈ 25–80 ~ | 1 | ≈ 80–90 ~ |
| bee | worker ≈ 1e-4 ~ | ≈ 25 ~ | queen 8 A; worker weeks ~ | ≈ 0.06 (21 d egg to adult) ~ | queen ≈ 1,000+ eggs/day ~ | continuous | 21 ~ |
| ant | worker ≈ 5e-6 ~ | ≈ 0.2 ~ (desert *Cataglyphis* 2.1 H) | queen 28 A | ≈ 0.06 ~ | ≈ 10³–10⁴/yr ~ | continuous | ≈ 40–60 ~ |
| earthworm | ≈ 0.005 ~ | ≈ 0.02 ~ | ≈ 6 ~ | ≈ 1 ~ | ≈ 10–40 cocoons/yr ~ | 1 | ≈ 90–150 ~ |
| grasshopper | ≈ 0.0015 ~ | hop; flight ≈ 13 ~ | ≈ 1 ~ (univoltine) | ≈ 0.2 ~ | ≈ 10–100 per pod ~ | 1 | overwinter ~ |
| daphnia | ≈ 1e-6 ~ | ≈ 0.01 ~ | 0.19 A | ≈ 0.02 ~ | ≈ 5–30 ~ | ≈ every 3 d ~ | ≈ 3 ~ |

### 10.4 Table C: simulation-scaled derived values [C]

Formulas (M in kg). These are designer defaults derived from the literature; tune the multipliers, not the exponents.

| Column | Formula |
|---|---|
| `log10 M` | log₁₀ M (size class) |
| `size_rel` | M^{1/3}: isometric body length relative to a 1-kg animal; use as the render scale |
| `speed_idx` | (v_max / 40 km/h)^{0.5}: compressed top-speed index. Sim top speed = speed_idx × global_speed. Cruise ≈ 0.3 × top (UNVERIFIED). |
| `perception_m` | 30 m × M^{0.25} × modality factor. The modality factor is designer-set: raptors 3, owls 2.5, ravens 2, canids, cats and bats 1.5, beaver 0.7, soil and colony invertebrates 0.1–0.5. The M^{0.25} exponent lies inside the M^0–M^{0.5} range cited by Jetz 2004 [V]. |
| `metab_rel` | c_th · M^{0.75}, with c_th = 1 (endotherm) or 0.16 (ectotherm at 20 °C, [C] section 3.2). Daily maintenance relative to a 1-kg mammal; multiply by the Boltzmann factor for other body temperatures. |
| `fast_rel` | M^{0.25} / c_th: days survivable without food, relative to a 1-kg endotherm |
| `HR_ha`, `K_km²` | PanTHERIA fits by trophic group (section 3.3); mammals only |

| id | log10 M | size_rel | v_max used | speed_idx | perception_m | metab_rel | fast_rel | HR_ha | K (ind/km²) |
|---|---|---|---|---|---|---|---|---|---|
| mouse | −1.70 | 0.27 | 13 | 0.57 | 11 | 0.053 | 0.38 | 0.15 | 707 |
| rabbit | 0.23 | 1.19 | 54 | 1.16 | 34 | 1.49 | 1.14 | 3.8 | 71 |
| squirrel | −0.48 | 0.69 | 20 | 0.70 | 23 | 0.44 | 0.76 | 3.5 | 73 |
| bat | −2.10 | 0.20 | 45m | 1.06 | 13 | 0.027 | 0.30 | (0.08) | (1040) |
| fox | 0.68 | 1.69 | 48 | 1.10 | 67 | 3.24 | 1.48 | 341 | 1.6 |
| beaver | 1.30 | 2.71 | 8~ | 0.45 | 44 | 9.46 | 2.11 | 59 | 9.7 |
| wolf | 1.48 | 3.11 | 63 | 1.25 | 105 | 12.8 | 2.34 | 3,770 | 0.24 |
| boar | 1.93 | 4.40 | 56 | 1.18 | 91 | 28.0 | 3.04 | 1,790 | 0.83 |
| deer | 2.18 | 5.31 | 72 | 1.34 | 126 | 42.9 | 3.50 | 553 | 1.9 |
| bear | 2.30 | 5.85 | 41 | 1.01 | 135 | 53.2 | 3.76 | 4,670 | 0.41 |
| lion | 2.20 | 5.43 | 64 | 1.26 | 160 | 45.0 | 3.56 | 33,800 | 0.045 |
| bison | 2.80 | 8.55 | 53 | 1.15 | 150 | 125 | 5.00 | 2,710 | 0.61 |
| elephant | 3.60 | 15.9 | 37 | 0.96 | 239 | 503 | 7.95 | 21,300 | 0.14 |
| sparrow | −1.60 | 0.29 | 59m | 1.21 | 14 | 0.063 | 0.40 | – | – |
| raven | 0.02 | 1.01 | 130m | 1.80 | 61 | 1.03 | 1.01 | – | – |
| mallard | 0.04 | 1.03 | 105 | 1.62 | 46 | 1.07 | 1.02 | – | – |
| peregrine | −0.09 | 0.94 | 110 | 1.66 | 86 | 0.86 | 0.95 | – | – |
| barnowl | −0.30 | 0.79 | 119m | 1.72 | 63 | 0.60 | 0.84 | – | – |
| eagle | 0.65 | 1.65 | 129 | 1.80 | 131 | 3.09 | 1.46 | – | – |
| herring | −0.70 | 0.58 | 6 | 0.39 | 20 | 0.048 | 4.18 | – | – |
| trout | 0.00 | 1.00 | 12 | 0.55 | 30 | 0.16 | 6.25 | – | – |
| pike | 0.48 | 1.44 | 8 | 0.44 | 47 | 0.37 | 8.23 | – | – |
| carp | 0.48 | 1.44 | 6 | 0.39 | 32 | 0.37 | 8.23 | – | – |
| salmon | 0.70 | 1.71 | 20 | 0.71 | 45 | 0.54 | 9.35 | – | – |
| frog | −1.60 | 0.29 | 4~ | 0.32 | 10 | 0.010 | 2.49 | – | – |
| lizard | −2.05 | 0.21 | 6 | 0.39 | 7 | 0.0047 | 1.93 | – | – |
| snake | −1.00 | 0.46 | 5~ | 0.35 | 13 | 0.029 | 3.51 | – | – |
| croc | 2.18 | 5.31 | 30~ (swim) | 0.87 | 105 | 6.86 | 21.9 | – | – |
| bee | −4.00 | 0.05 | 25~ | 0.79 | 3 (colony forages km; UNVERIFIED) | 1.6e-4 | 0.62 | – | – |
| ant | −5.30 | 0.02 | 0.2~ | 0.07 | 0.4 | 1.7e-5 | 0.30 | – | – |
| earthworm | −2.30 | 0.17 | 0.02~ | 0.02 | 0.8 | 0.0030 | 1.66 | – | – |
| grasshopper | −2.82 | 0.11 | 13~ | 0.57 | 3 | 0.0012 | 1.23 | – | – |
| daphnia | −6.00 | 0.01 | 0.01~ | 0.02 | 0.3 | 5e-6 | 0.20 | – | – |

Notes:
- Bat values in parentheses use the terrestrial mammal fit and are not meaningful for colonial roosting bats.
- Colony insects should be simulated as **colony super-agents** (queen + worker pool + forager agents only near the colony) for performance.
- **Ectotherms**: multiply metabolism by the Boltzmann factor, e^{−(0.63 eV/k)(1/T_body − 1/293.15)}, with T_body following the environment. They need basking or shade behaviour and become torpid below T_min.
- **Time scale.** Choose 1 sim-year = Y ticks (e.g. Y = 3,650 with 10 ticks per sim-day). Durations then keep real ratios (mouse maturity 42 d against elephant 11 yr is about 1:100).
  For gameplay, compress with d_sim = d_ref · (d/d_ref)^γ, where γ ≈ 0.5–0.7 (designer).

### 10.5 Plants (functional types; designer defaults, order-of-magnitude, UNVERIFIED unless marked)

| id | PFT / example | Height (m) | Growth | Maturity (yr) | Lifespan (yr) | Dispersal kernel | Shade tolerance | Water need | Fire response | Notes |
|---|---|---|---|---|---|---|---|---|---|---|
| grass | Grasses, Poaceae (C3 in cool climates, C4 in hot) | 0.3–1.5 | fast; regrows from basal meristems after grazing | < 1 | perennial tufts ~5–20 | exponential, α ~1 m, plus animal-borne tail | low | low–medium | resprouts within one season; high flammability when cured | core grazer food; NPP via section 4.3 |
| forb | Wildflowers (herbaceous) | 0.1–1 | fast | < 1–2 | 1–10 | ant- or gravity-dispersed, short | low–medium | medium | seeds bank | pollinator link (bees) |
| shrub | Bilberry *Vaccinium myrtillus* | 0.1–0.6 | moderate, clonal | 2–4 | clonal, decades | berries eaten by bears and birds; 2Dt fat tail | high (understory) | medium | resprouts from rhizomes | boreal / temperate understory |
| birch | Silver birch *Betula pendula* (pioneer tree) | 20–30 | fast | ~5–15 | ~80–100 | tiny wind-dispersed seeds; tens of m mean, long tail | low | medium | killed; recolonizes burns fast | early-successional |
| pine | Scots pine *Pinus sylvestris* | 25–35 | moderate | ~10–15 | ~300–700 | winged seeds by wind | low | low–medium | thick bark survives surface fire; resinous fuel | boreal / temperate conifer |
| oak | Pedunculate oak *Quercus robur* | 25–40 | slow | ~40 (acorns) | ~500–1000+ | heavy acorns: gravity plus scatter-hoarding by jays and squirrels (100s of m – km) | medium | medium | thick bark; resprouts when young | late-successional; mast years every ~2–5 yr |
| tropical | Generic tropical canopy tree | 30–50 | fast | 10–30 | 100–500 | frugivore-dispersed | medium | high | fire-sensitive | rainforest |
| cactus | Saguaro *Carnegiea gigantea* | up to ~12 | very slow (CAM) | ~35–70 | ~150–200 | bird- and bat-dispersed fruit | low | very low | fire-sensitive | desert |
| reed | Common reed *Phragmites australis* | 2–4 | fast, rhizomatous | 1–2 | clonal | wind plus clonal spread | low | wetland | resprouts | lake and river margins; bird habitat |
| phyto | Phytoplankton (aggregate field) | – | Eppley μ ≈ 0.59 e^{0.064T} d⁻¹ [K] | days | days | advected by water | light-limited with depth | aquatic | – | Daphnia and herring base |

Record longevities in AnAge include *Pinus longaeva* at 5,062 yr and *Adansonia digitata* at 2,500 yr [C]. Use these for "ancient tree" flavor.

### 10.6 Fungi (grid fields plus optional fruiting bodies; UNVERIFIED unless marked)

| id | Guild / example | Role | Spread | Interactions | Notes |
|---|---|---|---|---|---|
| sapro | Leaf-litter saprotrophs (e.g. *Agaricus*, *Marasmius*) | DEC (cellulose) | mycelial front ~mm/day; wind-dispersed spores | raises litter decay k; fruiting bodies feed squirrels, boar and insects | needs moisture; seasonal fruiting |
| whiterot | White-rot wood decomposers (e.g. *Trametes versicolor*) | DEC (lignin) | spores; colonizes dead wood | sets wood decay k (~0.02–0.1 /yr for logs) | releases nutrients locked in dead trees |
| ecto | Ectomycorrhizal fungi (e.g. *Amanita muscaria*) | MUT with birch, pine and oak | grows with root systems | plant N and P uptake bonus; carbon cost to the plant | toxic fruiting bodies (animals avoid them) |
| armillaria | Honey fungus *Armillaria ostoyae* | PAR + DEC | rhizomorphs (~1 m/yr) | kills or weakens trees, then decomposes them | one Oregon clone is reported to cover about 965 ha and be thousands of years old (Ferguson et al. 2003, *Can. J. For. Res.* 33:612; figures UNVERIFIED) |
| lichen | Lichens (fungus + alga, e.g. *Cladonia*) | PRD + MUT, pioneer | very slow (mm/yr) | reindeer winter forage; rock weathering and soil formation | tundra and boreal ground cover |

### 10.7 Data licences for the table sources

| Source | Licence | What it means for us |
|---|---|---|
| AnAge (HAGR) | database data sheets under **CC BY 3.0 Unported** [V] (https://genomics.senescence.info/legal.html) | Use derived values with attribution. Cite de Magalhães et al. 2024 (*NAR*), as HAGR requests. The Build 15 release notes cite Tacutu et al. 2013 (*NAR* 41:D1027). |
| PanTHERIA | metadata states "Copyright restrictions: None; Proprietary restrictions: None" [V] | The authors request citation of Jones et al. 2009. |
| Hirt et al. 2017 | supplementary data (Springer Nature) | Individual numeric facts are cited, not redistributed in bulk. |
| plotbiomes | code is MIT; the polygons derive from a copyrighted textbook figure | Not used. |

---

## 11. Licence summary for implementers

- **NetLogo Wolf Sheep Predation**: CC BY-NC-SA 3.0. **Do not copy code or text.** Re-implement from the rule description and credit Wilensky 1997.
- **Golly** (GPL): not used here.
- **Sugarscape**: the book is copyrighted. The rules are ideas, so re-implement from the description; Kehoe's spec is an arXiv preprint.
  Mesa is Apache-2.0 [K]. Reading it for reference is fine; do not paste its code without Apache attribution.
- **Phylotrack**: MIT [V]. A Rust re-implementation of its ideas is fine; attribution is courteous.
- **Flow-Lenia notebook**: licence unknown, so do not copy it. The equations are published.
- **noise 0.9.0**: "Apache-2.0/MIT" [V]. **rand_chacha 0.10.0**: MIT OR Apache-2.0 [V]. Both are compatible with the project's MIT OR Apache-2.0 licence.
- **Data**: AnAge is CC BY 3.0. PanTHERIA has no restrictions (cite it). Include a `DATA_SOURCES.md` with attributions in any release that ships species presets.

---

## 12. References (URLs; [V] = fetched and read this session)

**Predator–prey and NetLogo**
- Wilensky 1997, NetLogo Wolf Sheep Predation — https://ccl.northwestern.edu/netlogo/models/WolfSheepPredation [V]; source https://github.com/NetLogo/models (Sample Models/Biology) [V]
- Lotka 1925; Volterra 1926; Rosenzweig & MacArthur 1963 (*Am. Nat.* 97:209); Rosenzweig 1971, doi:10.1126/science.171.3969.385; Holling 1959; Huffaker 1958 [K]
- Wilensky & Rand 2015, *An Introduction to Agent-Based Modeling*, MIT Press [K]

**Flow-Lenia and Lenia**
- https://deepmind.google/research/publications/106327/ [V]; doi:10.1162/artl_a_00471; https://arxiv.org/abs/2212.07906 [V]; https://ar5iv.labs.arxiv.org/html/2212.07906 [V]
- Chan 2019, arXiv:1812.05433; Chan 2020, arXiv:2005.03742 [K]

**Energy and allometry**
- Brown et al. 2004, doi:10.1890/03-9000 — https://people.clas.ufl.edu/gillooly/files/05_brownetal_2004.pdf [V]
- Kleiber 1932, doi:10.3733/hilg.v06n11p315; Lindeman 1942, doi:10.2307/1930126; Gillooly et al. 2001, doi:10.1126/science.1061967 [K]
- Jetz et al. 2004, *Science* 306:266 — https://jetzlab.yale.edu/sites/default/files/files/Jetz%20et%20al%20Science%2004.pdf [V]
- Hirt et al. 2017, doi:10.1038/s41559-017-0241-4; supplement https://media.springernature.com/original/springer-static/esm/art%3A10.1038%2Fs41559-017-0241-4/MediaObjects/41559_2017_241_MOESM1_ESM.pdf [V]
- Nagy, Girard & Brown 1999, *Annu. Rev. Nutr.* 19:247 [V, exponents only]
- AnAge — https://genomics.senescence.info/species/ (dataset.zip, Build 15, 4,671 entries, released 3 Jul 2023) [V]
- PanTHERIA — https://esapubs.org/archive/ecol/E090/184/ (metadata.htm) [V]

**Vegetation and soil**
- Noy-Meir 1975; Monteith 1972; Yan & Hunt 1999; Clark et al. 1999 (*Ecology* 80:1475); Nathan & Muller-Landau 2000; Olson 1963; Lloyd & Taylor 1994; Linn & Doran 1984; Koerselman & Meuleman 1996; Eppley 1972; Whittaker & Likens 1975; Smith & Read 2008 [K]

**Fire**
- Alexandridis et al. 2008, doi:10.1016/j.amc.2008.06.046 [K for the paper, V for the constants via Freire & DaCamara 2019, https://nhess.copernicus.org/articles/19/169/2019/]
- Rothermel 1972, USDA FS INT-115; Andrews 2018, RMRS-GTR-371; Drossel & Schwabl 1992, doi:10.1103/PhysRevLett.69.1629; Connell & Slatyer 1977, doi:10.1086/283241; Horn 1975 [K]

**Terrain and hydrology**
- Mei, Decaudin & Hu 2007, doi:10.1109/PG.2007.15 — https://inria.hal.science/inria-00402079 [V]
- Musgrave et al. 1989; Perlin 2002; Quilez, https://iquilezles.org/articles/warp/; Planchon & Darboux 2002; Barnes et al. 2014 (arXiv:1511.04463); Wang & Liu 2006; O'Callaghan & Mark 1984; Tarboton 1997; Hargreaves & Samani 1985; Whittaker 1975; Holdridge 1947; Cooper 1969; Kopp & Lean 2011 [K]
- plotbiomes — https://github.com/valentinitnelav/plotbiomes [V, licence via search]
- noise 0.9.0 and rand_chacha 0.10.0 crate sources in the local cargo registry [V]

**Agent AI**
- Reynolds 1999 — https://www.red3d.com/cwr/steer/gdc99/ [V]; Reynolds 1987, doi:10.1145/37402.37406 [K]
- Dave Mark / IAUS — https://gameai.com/iaus.php [V existence]; Mark 2009 book [K]
- Isla 2005; Colledanchise & Ögren arXiv:1709.00084; Orkin 2006; Teschner et al. 2003; Hamilton 1971; Huberman & Glance 1993, doi:10.1073/pnas.90.16.7716 [K]

**Evolution**
- Stanley & Miikkulainen 2002 (NEAT), doi:10.1162/106365602320169811; Price 1970; Hill 1973; Kimura 1962; Bedau & Packard 1992; Gras et al. 2009 (EcoSim); Yaeger 1994 [K]
- Phylotrack, https://arxiv.org/abs/2405.09389 [V via search]

**Societies**
- Kehoe 2016, *The Specification of Sugarscape* — https://arxiv.org/abs/1505.06012 [V]
- Mesa Sugarscape G1MT — https://mesa.readthedocs.io/stable/examples/advanced/sugarscape_g1mt.html [V]
- Epstein & Axtell 1996; Axelrod & Hamilton 1981; Axelrod 1984, 1997; Nowak & May 1992; Nowak 2006; Dorigo et al. 1996; Helbing et al. 1997; Tinbergen 1962; Renfrew & Level 1979; Moorcroft & Lewis 2006; Potts & Lewis 2014; Giuggioli et al. 2011; Fretwell & Lucas 1970; Axtell et al. 2002 [K]
