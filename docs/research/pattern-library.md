# Life pattern library: sources, verification and licensing

Status: delivered 2026-11-09. Scope: the Conway's Life (B3/S23) pattern library in
`assets/patterns/life/` and its index `assets/patterns/catalog.toml`. The OTCA
metapixel files in `assets/patterns/otca/` were added separately and are **not**
covered by this document or listed in `catalog.toml`.

## 1. Summary

| | |
|---|---|
| Patterns delivered | **80** RLE files, every one listed in `catalog.toml` |
| Verified | **80 / 80** pass. Verification ran twice: once while building, then again on the files read back from disk |
| Dropped after failing | none. Every failure during development came from a verifier bug or a wrong expectation (section 5) and was fixed |
| Dropped by choice | `jam` (p3), to keep the library at 80 or fewer patterns |
| Rule | every file declares `rule = B3/S23` |
| Size on disk | about 340 KB of RLE plus an 84 KB catalog |

Per category (directory, then count): still-lifes 10, oscillators 15, spaceships 13, guns 7,
puffers 6, rakes 3, eaters-reflectors 6, methuselahs 11, growth 4, breeders 2,
misc 3.

Per `kind`: oscillator 15, spaceship 13, methuselah 11, still_life 10, gun 7,
puffer 6, eater 4, growth 4, rake 3, other 3, reflector 2, breeder 2.

Per source of the cell data: 32 transcribed by hand, 39 from Life Lexicon
diagrams, 9 from the Golly collection (8 RLE files copied unchanged, plus 1 Life 1.05
file converted to RLE).

Every item on the required list is present: Block, Beehive, Loaf, Boat, Tub, Pond, Ship;
Blinker, Toad, Beacon, Pulsar, Pentadecathlon, Clock, Figure eight, Queen bee
shuttle, Tumbler, Kok's galaxy, Twin bees shuttle; Glider, LWSS, MWSS, HWSS,
Copperhead, Loafer, Schick engine **and** Coe ship; Gosper and Simkin glider guns
plus p46 "new gun", p52, p59 (Snark), p165 MWSS and p210 loafer guns; Eater 1,
Eater 2, Eater 3, Eater 5, Snark and buckaroo; Puffer 1, Puffer train, block-laying
and glider-producing switch engines, Noah's ark, Space rake, Backrake 1; R-pentomino,
Diehard, Acorn, Pi- and B-heptomino, Rabbits, Thunderbird; Breeder 1, 10-cell
infinite growth, the 39x1 one-cell-thick line and Max.

## 2. Layout and format

```
assets/patterns/
  catalog.toml                         # [library] table + one [[pattern]] table per file
  life/<category>/<id>.rle             # categories: still-lifes oscillators spaceships guns
                                       #   puffers rakes eaters-reflectors methuselahs growth
                                       #   breeders misc
```

Each RLE file has these parts:

* `#N <name>` and `#O <discoverer>, <year>` (omitted when unknown). The Golly file
  `sawtooth177` already had its own `#N`/`#O`, which were kept.
* `#C` lines: an original description, a one-line summary of the verifier's
  measurement, the source and the licence note.
* For Golly-derived RLE files, a `#C ---- original Golly header follows ----` marker
  is followed by the untouched original `#C` lines. The data lines are copied verbatim.
* `x = W, y = H, rule = B3/S23`. In every file W and H equal the bounding box of the
  live cells (`check_assets.py` checked this).
* No `#` lines appear after the `x =` header, and nothing follows the terminating `!`.

The header of `catalog.toml` documents its schema. The fields asked for in the task are
`id, name, category, description, discoverer, year, period, dx, dy, kind, width,
height, population (initial), expected_behavior, complexity, file, source,
license, license_note`. Optional measured fields are added where they apply:
`speed`, `population_range`, `emits`, `emits_per_period`, `stabilizes_at`,
`final_population`, `dies_at`, `growth`, `growth_period`,
`growth_per_period`, `linear_from`, `object_population`, `demo_glider`,
`reflector_period`, `min_repeating_population`, `facts_source`, `verified`, `verification`.
Coordinates use the RLE convention, with x to the right and y downwards. `dx`/`dy` are the
displacement per period in the file's own orientation. For example, the LWSS file moves
(-2, 0) every 4 generations, which is leftwards.

Conventions that implementers of `crates/patterns::catalog` should know:

* `period = 0` for still lifes, methuselahs, growth and breeder patterns, eaters and the
  Snark (which is still apart from its demonstration glider).
* For guns, `period` is the period of the gun's own bounding box. For puffers and rakes,
  it is the period of the leading part (the engine plus its freshly emitted exhaust) in a
  co-moving frame. The puffer train's engine alone repeats every 20 generations,
  but engine plus exhaust repeats every 140, and the catalog stores 140.
* Eaters and reflectors ship with **one demonstration glider** (`demo_glider = true`).
  `population` includes it and `object_population` excludes it.
* `p210-loafer-gun` is the Golly file as published. Its loafer stream is absorbed by
  an eater at the right edge, so the whole file is periodic. The catalog still marks it
  as a gun emitting 1 loafer per 210 generations, which was measured by counting loafer births.

## 3. Sources

1. **Golly 5.1 pattern collection**, from the SourceForge git clone at commit
   `9a7079a84a24f4b5ef16b1c940660d75656e51be` ("updated version to 5.1", 2026-11-06),
   <https://sourceforge.net/projects/golly/>, git <https://git.code.sf.net/p/golly/code>.
   Files used, all under `Patterns/Life/`: `Guns/period-52-glider-gun.rle`,
   `Guns/p59-gun-with-Snark-reflectors.rle`, `Guns/gun-p165mwss.rle`,
   `Guns/loafer-gun-p210.rle`, `Rakes/2c5-engineless-rake-p185.rle` (11 trailing
   `showinviewer.lua` script comments after the `!` were dropped and the drop is noted in the
   file), `Methuselahs/blom.rle`, `Breeders/breeder.lif` (converted from Life 1.05 to
   RLE, with its `#D` text kept as `#C`), `Breeders/switch-engine-breeder.rle`, and
   `Miscellaneous/sawtooth177.rle`. CRLF line endings were normalised to LF.
2. **Life Lexicon, Release 29 (2 July 2018)**, as bundled with Golly at
   `Help/Lexicon/lex_*.htm`. Home page: <https://conwaylife.com/ref/lexicon/lex_home.htm>.
   Diagrams were extracted with a small HTML parser (`lexget.py`). Lexicon entries
   used as cell sources: figure-8, queen bee shuttle, tumbler, Kok's galaxy, twin bees
   shuttle, caterer, mold, pinwheel, octagon II, copperhead, loafer, Schick engine,
   Coe ship, weekender, turtle, Canada goose, Sir Robin, Cordership (Tooke's 3-engine),
   new gun, puffer (Puffer 1), puffer train, blinker puffer, Noah's ark, stabilized
   switch engine, time bomb, space rake, backrake, eater2, eater3, eater5, Snark,
   buckaroo, rabbits, bunnies, infinite growth (3 diagrams), spacefiller (Max) and
   LWSS (the synthesis diagram).
   * The Snark diagram is a loop of four Snark variants with one glider (period 232).
     The library uses Playle's original only. It was isolated by testing
     candidate component sets until the set reflected the loop's glider cleanly
     (49 cells). The demonstration glider is the loop's glider at generation 172.
   * Eater 5 is the "standard form" from the first eater5 diagram: the
     tub-with-tail plus its block, 12 cells.
3. **Hand transcription** of tiny classic objects: still lifes, blinker, toad,
   beacon, pulsar, pentadecathlon, clock, glider, xWSS, R-pentomino, diehard, acorn,
   pi/B-heptomino, thunderbird, Herschel, century, eater 1 and the switch engine. The
   Gosper and Simkin gun RLE bodies were also written by hand. Every hand transcription was
   compared with the Lexicon diagram. The comparison accepts any phase of the object and any of
   the 8 rotations or reflections (`xcheck`). All matched except the Simkin gun, where the Lexicon shows
   the snake-stabilised variant. Our file is the standard eater-stabilised form, which differs
   only in the bottom stabiliser and was confirmed as a p120, 36-cell gun functionally.
4. **Facts (discoverer, year)** come from the Lexicon text (`facts_source = "Life Lexicon
   Release 29 text"`), from Golly headers, or from LifeWiki infoboxes read through the mirror at
   <https://conwaylife.njohnston.ca/wiki/> (conwaylife.com itself serves a bot
   challenge to scripted clients, and we did not try to bypass it). The pages read were
   Space_rake, R-pentomino, Diehard, Pi-heptomino, B-heptomino, Thunderbird, Acorn,
   Puffer_1, Eater_2, Eater_3, Eater_5, Breeder_1, Max, Period-52_glider_gun, Block,
   Beehive, Loaf, Boat, Tub, Pond, Ship, Barge, Long_boat, Aircraft_carrier, Pulsar,
   Century, Herschel, Time_bomb (which redirects to Glider-producing switch engine),
   Bunnies, Backrake_1, Noah's_ark and Infinite_growth. The mirror may lag the live wiki.

Things not verified (marked UNVERIFIED or left blank):

* The year of the p13-assisted **period-52 gun** in Golly is left blank. The
  Golly header cites forum post p=65762 but gives no date. LifeWiki's page describes
  a *different* p52 gun, completed by Dave Greene in Sept 2018.
* The year of the **puffer train** is left blank. The Lexicon credits Bill Gosper
  and calls it the second known puffer but gives no year.
* No discoverer is known for **diehard**, **century** (1970) or **eater 5**, or for the
  **space rake**, which LifeWiki says was "probably Bill Gosper", no later than 1971.
* The stabilisation time and final population of the **unstable switch engine**
  (3911 generations, 842 cells) are only our measurement. None of the consulted sources
  states them.
* For still lifes, LifeWiki credits the "JHC group" (Conway's group). For the block it
  conflicts with itself: the infobox says 1969 and the body says 1970. We kept the infobox values.

## 4. Verification method

All verification code lives in the session scratchpad, on purpose **not in the repo**:
`/private/tmp/claude-501/-Users-alejandro-Projects-Conway-GameOfLife/d26793ef-30fe-4239-b1fb-77f42d831735/scratchpad/patterns-verify/`
(`lifelib.py`, `specs.py`, `build.py`, `check_assets.py`, `lexget.py`, `mktable.py`).
That directory is temporary, so section 7 recommends porting the checks to Rust. The simulator is a
from-scratch Python implementation: no Golly code was used or translated. It has two
backends:

* exact unbounded B3/S23 on a set of packed-integer cell coordinates, used for
  almost everything;
* a dense numpy grid that grows automatically, used only for quadratic-growth runs. On the
  R-pentomino it was checked generation-by-generation for 1200 generations
  against the set backend.

Checks by kind (the numbers in the catalog are the measured values):

| kind | method | pass criterion |
|---|---|---|
| still life | one step | next generation equals the pattern |
| oscillator / spaceship | hash each generation's state, translated to its bounding-box corner, until a state repeats (limit 2000 generations) | the cycle starts at generation 0 with the expected period. Oscillators: displacement (0,0). Ships: non-zero displacement, from which the speed string is derived |
| gun | restrict the state to the initial bounding box (+6 margin) and find the smallest P with identical restricted states over 3P generations after warm-up (4·max(W,H)+600). Classify every component outside the box against a library of all phases and orientations of glider, LWSS, MWSS, HWSS and loafer. Count escaped ships at generation t and t+L, with L = 2·lcm(P, ship period) | box period = expected; all escaped objects are the expected ship; integer ships per period ≥ 1. For guns whose output is absorbed (p210 loafer gun): count loafer births over one period |
| puffer / rake | co-moving "head" test: for each P, the candidate displacements come from the bounding-box edge shifts. Accept when the cells within a slab of depth max(W,H)+4 behind the leading edge at t+P equal the slab at t translated by d, over a window of 1-3 periods after warm-up. Free spaceships are ignored as head candidates and, where noted, removed before the comparison | P = expected period. Rakes: the debris must contain spaceships |
| eater | brute-force 4 glider directions × 29 lanes × 4 phases; keep a lane where eater+glider returns exactly to the eater after 90 generations; ship that glider as the demo | eater is a still life and file → eater after 90 generations |
| reflector | run (object+glider) and (object alone) for 240 generations | the difference is exactly one glider whose direction differs from the incoming one, and the object is unchanged (Snark still; buckaroo p30) |
| methuselah | (1) population-sequence test: the first T after which pop(t+p) = pop(t) for all t up to T+2500, with the smallest p ≤ 60 found on a 1500-generation tail. (2) spatial refinement: the first T for which the state with free spaceships deleted equals the same state 60 generations later, for all later t | T and final population equal the published values; dies_at for diehard |
| linear growth | pop(t+P) − pop(t) constant for every t in a window; also the first generation from which that holds | constant positive increment (and onset 1483 for the 39x1 line) |
| quadratic growth | populations at T/4, T/2, 3T/4, T (T a multiple of the pattern's natural period) | both equal-step second differences positive and within 25% of each other. In practice they were exactly equal: Max 20000/20000, Breeder 1 2400/2400, switch-engine breeder 2880/2880 |
| sawtooth | population at generations 0 and 48·(121−1) = 5760 | both 177, with every generation in between above 177 |
| synthesis | run 120 generations, classify components | exactly one LWSS |

Additional spot checks, not stored in the catalog:

* **Snark repeat time.** Two gliders 43 ticks apart are both reflected cleanly; at
  40, 41 or 42 ticks they are not. This confirms the Lexicon's repeat time of 43.
* **Re-check from disk.** `check_assets.py` parsed `catalog.toml` with Python
  `tomllib`. It checked that the catalog and `life/` contain exactly the same 80 files,
  re-parsed every RLE, compared the header x/y, the bounding box and the population with the
  catalog, and re-ran the verifier on the cells read back. Result: 80/80 OK, 0 mismatches.

Runtime on the M2 Max with 11 worker processes: about 2.7 minutes for the full build. The
slowest items were Blom (about 25,800 generations, 153 s) and Rabbits and Bunnies (about 20,000
generations, 82 s each).

## 5. Problems found during verification (all resolved)

| pattern | symptom | cause / fix |
|---|---|---|
| Thunderbird | the population method gave 242; the published value is 243 | the last reaction at 242→243 keeps the population the same. A spatial refinement was added (state minus free ships compared with the state 60 generations later), giving 243. The refinement left every other methuselah unchanged (1103, 5206, 173, 148, 128, 103, 17331, 17332) and moved Blom 23313→23314, which matches Golly's header |
| Puffer train | no period ≤ 40 | the engine is p20 but engine plus fresh exhaust is p140. The catalog stores 140, and the description mentions p20 |
| Noah's ark | no period ≤ 1400 | gliders thrown forward entered the comparison window. With free ships removed and a 3000-generation warm-up, the period is 1344 with displacement (-112,-112), matching LifeWiki |
| Backrake 1 | false period 4 at displacement (-1,1) | the "head" was a lone escaping glider. Head candidates made only of free spaceships are now rejected → p8, (0,-4) |
| Glider-producing switch engine | planned as a rake | its exhaust holds still lifes and blinkers as well as one glider per 384 generations (debris grows linearly), so it was moved to **puffers** |
| p165 MWSS gun | population increase per period not constant | MWSS populations differ by phase and 165 is odd. Fixed by counting escaped ships at two times L = 2·lcm(P,4) apart |
| p210 loafer gun | nothing escapes | the Golly file absorbs its own output. Births were counted instead (1 per 210) |
| Breeder 1 | log-ratio growth exponent 1.19 at generation 3000 | a large constant term (4060 initial cells) distorts that test. It was replaced with the equal-step second-difference test, which gives exactly 2400/2400 |
| Simkin gun | Lexicon cross-check failed | different stabiliser variant (see section 3). Verified functionally |

## 6. Licensing position

This is an engineering position for the project, not legal advice.

* **Golly-derived files (9)** stay under **GPL-2.0-or-later** as separately licensed
  *data* files, with their original headers and attribution kept unchanged and our provenance
  lines added. They are input data that the MIT/Apache-2.0 code loads at runtime. They are not
  compiled into or linked with the code, and no Golly *code* was copied. If the project ever
  needs a library that is entirely permissive, these 9 files are the ones to replace
  (`period-52-glider-gun`, `p59-snark-gun`, `p165-mwss-gun`, `p210-loafer-gun`,
  `engineless-rake-2c5-p185`, `blom`, `breeder-1`, `switch-engine-breeder`,
  `sawtooth-177`). A `LICENSE-GPL` notice, or a pointer to the GPL text, should ship next to
  `assets/patterns/` when the app is distributed. *Action for the packaging owner.*
* **Lexicon-derived files (39)**: the Life Lexicon is (C) Stephen Silver 1997-2018,
  updated by Dave Greene and David Bell, and released under **CC BY-SA 3.0**. A specific cell
  configuration is arguably a mathematical fact rather than protected expression, but we keep
  attribution in every file and in `license_note` anyway. We copied no Lexicon prose:
  all descriptions are original wording. If CC BY-SA attribution or share-alike is
  ever a concern, these objects can be re-entered from first principles, for example by
  re-running a published glider synthesis, without changing any cell.
* **Hand-transcribed files (32)** are tiny classic objects written for this project and
  marked "public-domain mathematical object (transcribed)" with the discoverer credited.
* The verifier scripts are our own original code. They are kept outside the repo.

## 7. Recommendations

* Port the verifier's checks into a Rust integration test in `crates/patterns`
  (or `simulation-cli validate`). The test should read `catalog.toml`, run each pattern
  through the project's engine and assert `period`, `dx`/`dy`, `stabilizes_at`/`final_population`
  and gun emission counts. The catalog values are the expected results. Once Hashlife is
  ready, long methuselahs (Blom, Rabbits) become cheap to check.
* Make the catalog loader tolerate the optional fields above, and treat
  `population` as the initial population.

## 8. Results table

All values were measured by the verifier. "published reference" lists the published values
we compared against: LW = LifeWiki infobox (mirror), LEX = Life Lexicon R29, GOLLY = Golly file
header. Every listed reference matches the measurement. pop = initial population. Oscillator
"pop range" is min..max over one period.

| # | id | category | kind | pop | WxH | period | displacement, speed | measured extra | published reference (matches) |
|---|---|---|---|---|---|---|---|---|---|
| 1 | `block` | still-lifes | still_life | 4 | 2x2 | - | - |  | - |
| 2 | `beehive` | still-lifes | still_life | 6 | 4x3 | - | - |  | - |
| 3 | `loaf` | still-lifes | still_life | 7 | 4x4 | - | - |  | - |
| 4 | `boat` | still-lifes | still_life | 5 | 3x3 | - | - |  | - |
| 5 | `tub` | still-lifes | still_life | 4 | 3x3 | - | - |  | - |
| 6 | `pond` | still-lifes | still_life | 8 | 4x4 | - | - |  | - |
| 7 | `ship` | still-lifes | still_life | 6 | 3x3 | - | - |  | - |
| 8 | `barge` | still-lifes | still_life | 6 | 4x4 | - | - |  | - |
| 9 | `long-boat` | still-lifes | still_life | 7 | 4x4 | - | - |  | - |
| 10 | `aircraft-carrier` | still-lifes | still_life | 6 | 4x3 | - | - |  | - |
| 11 | `blinker` | oscillators | oscillator | 3 | 3x1 | 2 | - | pop range [3, 3] | p2 (LEX) |
| 12 | `toad` | oscillators | oscillator | 6 | 4x2 | 2 | - | pop range [6, 6] | p2 (LEX) |
| 13 | `beacon` | oscillators | oscillator | 8 | 4x4 | 2 | - | pop range [6, 8] | p2 (LEX) |
| 14 | `pulsar` | oscillators | oscillator | 48 | 13x13 | 3 | - | pop range [48, 72] | p3, 48 cells (LW) |
| 15 | `pentadecathlon` | oscillators | oscillator | 12 | 10x3 | 15 | - | pop range [12, 40] | p15 (LEX) |
| 16 | `clock` | oscillators | oscillator | 6 | 4x4 | 2 | - | pop range [6, 6] | p2 (LEX) |
| 17 | `figure-eight` | oscillators | oscillator | 18 | 6x6 | 8 | - | pop range [12, 26] | p8 (LEX) |
| 18 | `queen-bee-shuttle` | oscillators | oscillator | 23 | 22x9 | 30 | - | pop range [22, 36] | p30 (LEX) |
| 19 | `tumbler` | oscillators | oscillator | 16 | 9x5 | 14 | - | pop range [16, 22] | p14 (LEX) |
| 20 | `koks-galaxy` | oscillators | oscillator | 48 | 9x9 | 8 | - | pop range [28, 64] | p8 (LEX) |
| 21 | `twin-bees-shuttle` | oscillators | oscillator | 38 | 27x17 | 46 | - | pop range [36, 94] | p46 (LEX) |
| 22 | `caterer` | oscillators | oscillator | 12 | 8x6 | 3 | - | pop range [12, 16] | p3 (LEX) |
| 23 | `mold` | oscillators | oscillator | 12 | 6x6 | 4 | - | pop range [12, 14] | p4 (LEX) |
| 24 | `pinwheel` | oscillators | oscillator | 35 | 12x12 | 4 | - | pop range [35, 35] | p4 (LEX) |
| 25 | `octagon-2` | oscillators | oscillator | 16 | 8x8 | 5 | - | pop range [16, 24] | p5 (LEX) |
| 26 | `glider` | spaceships | spaceship | 5 | 3x3 | 4 | (1,1) c/4 diagonal |  | c/4 diag, p4 (LEX) |
| 27 | `lwss` | spaceships | spaceship | 9 | 5x4 | 4 | (-2,0) c/2 orthogonal |  | c/2, p4 (LEX) |
| 28 | `mwss` | spaceships | spaceship | 11 | 6x5 | 4 | (-2,0) c/2 orthogonal |  | c/2, p4 (LEX) |
| 29 | `hwss` | spaceships | spaceship | 13 | 7x5 | 4 | (-2,0) c/2 orthogonal |  | c/2, p4 (LEX) |
| 30 | `copperhead` | spaceships | spaceship | 30 | 6x12 | 10 | (0,-1) c/10 orthogonal |  | c/10, p10 (LEX) |
| 31 | `loafer` | spaceships | spaceship | 20 | 9x9 | 7 | (-1,0) c/7 orthogonal |  | c/7, p7 (LEX) |
| 32 | `schick-engine` | spaceships | spaceship | 40 | 18x9 | 12 | (-6,0) c/2 orthogonal |  | c/2, p12 (LEX) |
| 33 | `coe-ship` | spaceships | spaceship | 28 | 10x9 | 16 | (8,0) c/2 orthogonal |  | c/2, p16 (LEX) |
| 34 | `weekender` | spaceships | spaceship | 36 | 16x11 | 7 | (0,-2) 2c/7 orthogonal |  | 2c/7, p7 (LEX) |
| 35 | `turtle` | spaceships | spaceship | 44 | 12x10 | 3 | (-1,0) c/3 orthogonal |  | c/3, p3 (LEX) |
| 36 | `canada-goose` | spaceships | spaceship | 36 | 13x12 | 4 | (-1,-1) c/4 diagonal |  | c/4 diag, p4 (LEX) |
| 37 | `sir-robin` | spaceships | spaceship | 282 | 31x79 | 6 | (-1,-2) (2,1)c/6 |  | (2,1)c/6, p6 (LEX) |
| 38 | `cordership-3-engine` | spaceships | spaceship | 221 | 63x58 | 96 | (-8,-8) c/12 diagonal |  | c/12 diag, p96 (LEX) |
| 39 | `gosper-glider-gun` | guns | gun | 36 | 36x9 | 30 | - | 1 glider per period | p30, 36 cells (LEX) |
| 40 | `simkin-glider-gun` | guns | gun | 36 | 33x21 | 120 | - | 1 glider per period | p120, 36 cells (LEX) |
| 41 | `new-gun-p46` | guns | gun | 56 | 34x34 | 46 | - | 1 glider per period | p46 (LEX) |
| 42 | `period-52-glider-gun` | guns | gun | 908 | 126x97 | 52 | - | 1 glider per period | p52 (GOLLY) |
| 43 | `p59-snark-gun` | guns | gun | 2738 | 299x239 | 59 | - | 1 glider per period | p59 (GOLLY) |
| 44 | `p165-mwss-gun` | guns | gun | 563 | 178x218 | 165 | - | 1 MWSS per period | p165 MWSS (GOLLY) |
| 45 | `p210-loafer-gun` | guns | gun | 1746 | 226x195 | 210 | - | 1 loafer per period (absorbed in file) | p210 loafer (GOLLY) |
| 46 | `puffer-1` | puffers | puffer | 44 | 27x7 | 128 | (0,-64) c/2 orthogonal |  | c/2, p128, 44 cells, 27x7 (LW) |
| 47 | `puffer-train` | puffers | puffer | 22 | 18x5 | 140 | (0,-70) c/2 orthogonal |  | c/2 (LEX); period not published in our sources |
| 48 | `blinker-puffer-1` | puffers | puffer | 37 | 9x18 | 8 | (-4,0) c/2 orthogonal |  | c/2, p8 (LEX) |
| 49 | `noahs-ark` | puffers | puffer | 16 | 15x15 | 1344 | (-112,-112) c/12 diagonal |  | c/12, p1344, 16 cells, 15x15 (LW) |
| 50 | `block-laying-switch-engine` | puffers | puffer | 104 | 60x60 | 288 | (-24,-24) c/12 diagonal |  | c/12, p288 (LEX) |
| 51 | `glider-producing-switch-engine` | puffers | puffer | 17 | 15x6 | 384 | (-32,-32) c/12 diagonal |  | p384 (LEX); time bomb 17 cells (LW) |
| 52 | `space-rake` | rakes | rake | 65 | 22x19 | 20 | (10,0) c/2 orthogonal | emits glider | c/2, p20, 65 cells, 22x19 (LW) |
| 53 | `backrake-1` | rakes | rake | 88 | 27x18 | 8 | (0,-4) c/2 orthogonal | emits glider | c/2, p8, 88 cells, 27x18 (LW) |
| 54 | `engineless-rake-2c5-p185` | rakes | rake | 2633 | 531x201 | 185 | (-74,0) 2c/5 orthogonal | emits glider | 2c/5, p185 (GOLLY) |
| 55 | `eater-1` | eaters-reflectors | eater | 12 | 13x13 | - | - | object 7 cells + demo glider | eats gliders (LEX) |
| 56 | `eater-2` | eaters-reflectors | eater | 24 | 17x15 | - | - | object 19 cells + demo glider | 19 cells, 7x7 (LW) |
| 57 | `eater-3` | eaters-reflectors | eater | 36 | 20x21 | - | - | object 31 cells + demo glider | 31 cells, 12x12 (LW) |
| 58 | `eater-5` | eaters-reflectors | eater | 17 | 17x18 | - | - | object 12 cells + demo glider | 12 cells, 9x6 (LW) |
| 59 | `snark` | eaters-reflectors | reflector | 54 | 19x24 | - | - | object 49 cells + demo glider | 90-degree reflector, repeat time 43 (LEX) |
| 60 | `buckaroo` | eaters-reflectors | reflector | 28 | 24x12 | 30 | - | object 23 cells + demo glider | p30 glider turner (LEX) |
| 61 | `r-pentomino` | methuselahs | methuselah | 5 | 3x3 | - | - | stable at 1103, final pop 116 | 1103 / 116 (LW, LEX) |
| 62 | `diehard` | methuselahs | methuselah | 7 | 8x3 | - | - | dies at 130 | dies at 130 (LW, LEX) |
| 63 | `acorn` | methuselahs | methuselah | 7 | 7x3 | - | - | stable at 5206, final pop 633 | 5206 / 633 (LW, LEX) |
| 64 | `pi-heptomino` | methuselahs | methuselah | 7 | 3x3 | - | - | stable at 173, final pop 55 | 173 / 55 (LW) |
| 65 | `b-heptomino` | methuselahs | methuselah | 7 | 4x3 | - | - | stable at 148, final pop 28 | 148 / 28 (LW) |
| 66 | `rabbits` | methuselahs | methuselah | 9 | 7x3 | - | - | stable at 17331, final pop 1744 | 17331 (LEX, GOLLY) |
| 67 | `thunderbird` | methuselahs | methuselah | 6 | 3x5 | - | - | stable at 243, final pop 46 | 243 / 46 (LW) |
| 68 | `herschel` | methuselahs | methuselah | 7 | 3x4 | - | - | stable at 128, final pop 24 | 128 / 24 (LW) |
| 69 | `century` | methuselahs | methuselah | 6 | 4x3 | - | - | stable at 103, final pop 15 | 103 / 15 (LW) |
| 70 | `bunnies` | methuselahs | methuselah | 9 | 8x4 | - | - | stable at 17332, final pop 1744 | 17332 / 1744 (LW) |
| 71 | `blom` | methuselahs | methuselah | 13 | 12x5 | - | - | stable at 23314, final pop 2740 | 23314 / 2740 (GOLLY) |
| 72 | `ten-cell-infinite-growth` | growth | growth | 10 | 8x6 | - | - | +16 cells / 144 gens from gen 564 | 10 cells, infinite growth (LEX) |
| 73 | `five-by-five-infinite-growth` | growth | growth | 13 | 5x5 | - | - | +16 cells / 144 gens from gen 660 | infinite growth (LEX) |
| 74 | `one-cell-thick-infinite-growth` | growth | growth | 28 | 39x1 | - | - | +32 cells / 144 gens from gen 1483 | 39x1, stable from gen 1483 (LEX) |
| 75 | `max-spacefiller` | growth | growth | 187 | 27x27 | - | - | second differences [20000, 20000] | 187 cells, 27x27 (LW) |
| 76 | `breeder-1` | breeders | breeder | 4060 | 749x338 | - | - | second differences [2400, 2400] | 4060 cells, 749x338, quadratic growth (LW) |
| 77 | `switch-engine-breeder` | breeders | breeder | 707 | 85x164 | - | - | second differences [2880, 2880] | quadratic growth (GOLLY) |
| 78 | `switch-engine` | misc | other | 8 | 6x4 | - | - | stable at 3911, final pop 842 | c/12 engine that self-destructs (LEX); end time not published in our sources |
| 79 | `sawtooth-177` | misc | other | 177 | 68x77 | - | - | pop 177 at gens 0 and 5760, peak 297 | pop 177 at gens 48(121^n - 1); n = 0, 1 checked (GOLLY) |
| 80 | `lwss-synthesis` | misc | other | 15 | 7x10 | - | - | result: exactly 1 LWSS | 3-glider LWSS synthesis (LEX) |
