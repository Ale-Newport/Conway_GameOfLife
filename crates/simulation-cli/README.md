# emergence-cli

Headless command-line runner for every Emergence Lab model: list models and
scenarios, describe them, run experiments, compare replicates, validate
determinism, benchmark and convert pattern files. It uses
[`model-registry`](../model-registry) for models, scenarios and factories,
`analytics` for statistics and CSV/JSON export and `persistence` for snapshot
files and the user data folder.

```text
cargo run --release -p simulation-cli -- <command> [options]
cargo build --release -p simulation-cli   # target/release/emergence-cli
```

Every example below was run with the release build on an Apple M2 Max
(12 cores, 64 GiB, macOS 27.0.1) with `--builtin-only` (built-in scenarios
only); long outputs are cut where marked `...`.

## Targets, data and exit status

* A **target** is a model id (its default scenario, e.g. `life.classic`), a
  scenario id qualified by folder (`conway/glider-gun-showcase`) or bare when
  unique (`seeds`), or the path of a scenario file inside a folder named
  after its format (`my-scenarios/conway/foo.ron`). Bare ids that exist in
  two folders (`island_evolution`) are rejected with the qualified choices.
* **Assets** (pattern catalog, OTCA metapixel tiles, species): `--assets`,
  else `$EMERGENCE_LAB_ASSETS`, else `<exe dir>/assets`, else the workspace
  `assets/` this binary was built from. The metapixel model needs the OTCA
  tiles; the other models fall back to data embedded in their crates.
* **Scenarios**: the built-in scenarios of every model crate, then the
  workspace `scenarios/` folder, the user folder `<data>/scenarios/`
  (`$EMERGENCE_LAB_DATA` or the platform location, see `persistence`) and
  every `--scenario-dir`. A file with the id of an earlier scenario replaces
  it; invalid files are skipped and listed by `list scenarios` with the model
  crate's message. `custom-life/` folders also hold the plain systems the
  Custom Life editor saves in the user library (`<data>/scenarios/custom-life/`):
  they get the file name as id and the model's default seed (0), since the
  editor stores none. `--builtin-only` ignores the workspace and user folders.
* **Exit status**: `0` success, `1` error (printed as `error: ...` with
  context), `2` invalid command line, `3` a validation check failed.

## list

```text
$ emergence-cli list models
ID                     NAME                                   FAMILY                      CRATE              SCENARIOS  DEFAULT SCENARIO
life.classic           Classic Conway                         Cellular automaton          cellular-automata          9  conway/glider-gun-showcase
ca.life_like           Life-like rules                        Cellular automaton          cellular-automata         10  ca-lab/highlife-replicator
ca.generations         Generations rules                      Cellular automaton          cellular-automata          2  ca-lab/brians-brain
ca.wireworld           Wireworld                              Cellular automaton          cellular-automata          1  ca-lab/wireworld-circuits
ca.langtons_ant        Langton's ant & turmites               Agent-based model           cellular-automata          3  ca-lab/langtons-ant-highway
ca.wolfram             Elementary cellular automata           Cellular automaton          cellular-automata          4  ca-lab/wolfram-rule-110
life.hashlife          Hashlife                               Cellular automaton          hashlife                  18  hashlife/gosper-glider-gun
life.metapixel         Conway within Conway (OTCA metapixel)  Meta-cellular construction  metapixel                  7  metapixel/metapixel.blinker
continuous.lenia       Lenia                                  Continuous field            continuous                 7  continuous/lenia.orbium
continuous.smoothlife  SmoothLife                             Continuous field            continuous                 2  continuous/smoothlife.glider
continuous.flow_lenia  Flow-Lenia                             Continuous field            continuous                 1  continuous/flow_lenia.demo
voxel.life             Three-Dimensional Life                 3D cellular automaton       voxel-life                12  voxel/life-5766-glider
environment.world      Environment (abiotic world)            Continuous field            physics-world             10  environment/temperate
ecosystem              Ecosystem                              Hybrid agent / field model  ecosystem                 14  ecosystem/wolf_sheep_grass
custom_life            Custom Life Creator                    Hybrid agent / field model  rule-engine                7  custom-life/bacterial-growth
evolution              Evolution Laboratory                   Agent-based model           evolution                  6  evolution/speed_selection
civilization           Emergent Civilizations                 Agent-based model           civilization               6  civilization/abundant
17 models

$ emergence-cli list scenarios --model continuous.lenia
ID                                     MODEL             SEED  SOURCE   TITLE
continuous/lenia.orbium                continuous.lenia     1  builtin  Orbium unicaudatus
continuous/lenia.orbium_chan           continuous.lenia     1  builtin  Orbium unicaudatus (animals.json)
continuous/lenia.scutium_solidus       continuous.lenia     1  builtin  Scutium solidus
continuous/lenia.hydrogeminium_natans  continuous.lenia     1  builtin  Hydrogeminium natans
continuous/lenia.tessellatium_gyrans   continuous.lenia     1  builtin  Tessellatium gyrans
continuous/lenia.fish                  continuous.lenia     1  builtin  K=3 Fish
continuous/lenia.soup                  continuous.lenia     7  builtin  Lenia random soup
7 scenarios; scenario directories: none
```

Names, families and summaries come from each model's own descriptor (the
model is built with its default scenario). `--json` prints machine-readable
lists. `hashlife/*` are the 18 patterns embedded in the `patterns` crate
loaded into a Hashlife universe; `environment/*` are the 10 biome presets of
`physics-world`.

## info

```text
$ emergence-cli info ca.wolfram
Model      ca.wolfram — Elementary cellular automata (Cellular automaton)
Crate      cellular-automata (documentation: docs/models/cellular-automata.md)
Scenario   ca-lab/wolfram-rule-110 — Rule 110 (seed 1, builtin (cellular-automata))
Time       one tick = 1 step; deterministic: yes

Wolfram's 256 one-dimensional rules drawn as space-time diagrams (rule 30, 90, 110, 184...).

Elementary rule 110, proved Turing-complete by Matthew Cook (2004). From a single cell the right edge stays fixed while the left edge advances one cell per step, leaving a triangle of interacting gliders.

PARAMETERS (6)
KEY       GROUP    TYPE                           DEFAULT      VALUE        APPLIES
rule      Rules    int [0, 255]                   30           110          applies next tick
boundary  Rules    one of fixed0|fixed1|periodic  fixed0       fixed0       applies next tick
width     World    int [3, 1048576]               1025         1025         requires reset
start     World    one of single_cell|random      single_cell  single_cell  requires reset
density   World    float [0, 1]                   0.5          0.5          requires reset
history   Display  int [1, 1048576]               4096         4096         requires reset

METRICS (3)
KEY         KIND   UNIT   DESCRIPTION
ones        gauge  cells  Cells equal to 1 in the current row.
density     ratio         Ones divided by the row width.
generation  gauge  steps  Index of the current row.

RULES
## Elementary rule 110
...
SCENARIOS OF ca.wolfram (4): ca-lab/wolfram-rule-110, ca-lab/wolfram-rule-184, ca-lab/wolfram-rule-30, ca-lab/wolfram-rule-90
```

`VALUE` is the scenario's value, `APPLIES` the parameter's apply policy.
`--json` prints the scenario, the full descriptor and the values.

## run

One tick is one call to the model's `step()`, exactly as the desktop
application's runner advances it: `generations_per_tick` generations for
Life, `2^step_exponent` generations for Hashlife, the `step` parameter's
generations for the metapixel (one metageneration, 35 328 generations, by
default), one time step for the other models. Step-size parameters given
with `--param` therefore act as they do in the application. Metrics are
recorded before the first tick and every `--metrics-every K` ticks (and
after the last one); counter metrics (births, deaths...) hold the value of
the sampled tick. The summary line gives the model's tick counter and its
simulated time (`tick × dt` in the descriptor's unit).

`--param key=value` overrides are converted to the parameter's type,
validated against the model's descriptor and applied before the first tick;
parameters that require a reset are put into effect by one reset, after
which numeric, switch, choice and colour values are checked to be in effect
and text values (rule notations) are recorded as the model normalised them.
For models that publish their seed as a `seed` parameter, `--param seed=S`
is the same as `--seed S` (giving both with different values is an error),
and seeds outside that parameter's range (above 2^63 − 1) are refused.

```text
$ emergence-cli run conway/random-soup-torus --ticks 500 --metrics-every 50 --seed 42 \
    --param width=128 --param height=128 --param rule=B36/S23 --out soup.csv --snapshot-out soup.elsnap
model     life.classic (Classic Conway)
scenario  conway/random-soup-torus — Random soup on a torus (seed 42)
overrides width=128 (requires reset), height=128 (requires reset), rule=B36/S23 (applies next tick)
tick 500 | 500/500 ticks (100%) | 0.1 s | 8857.61 ticks/s
ran       500 ticks in 0.057 s (8770.4 ticks/s): tick counter 0 -> 500, simulated time 0 -> 500 generation

METRIC           FINAL     MEAN        MIN       MAX  UNIT
population         697  1638.73        697      5705  cells
density      0.0425415  0.10002  0.0425415  0.348206
bbox_width         128      128        128       128  cells
bbox_height        128      128        128       128  cells
generation         500      250          0       500  generations
engine               0        0          0         0
births             211    484.9        211      1033  per tick
deaths             219    501.8        219      1088  per tick
metrics written to soup.csv (86 values)
snapshot of tick 500 written to soup.elsnap

$ head -16 soup.csv
# run_id: conway/random-soup-torus@42
# model_id: life.classic
# scenario: conway/random-soup-torus
# seed: 42
# started_at: 2026-11-10T05:34:48Z
# ticks: 500
# param engine: auto
# param generations_per_tick: 1
# param height: 128
# param preset: highlife
# param rule: B36/S23
# param topology: torus
# param track_activity: true
# param track_ages: true
# param width: 128
tick,population,density,bbox_width,bbox_height,generation,engine,births,deaths
```

`--out x.json` writes the versioned `analytics` run document instead
(`run_from_json` reads it back). Progress goes to stderr once per second;
`--quiet` prints only the final table. A closed standard output
(`emergence-cli run ... --out soup.csv | head`) only drops the rest of the
printed text: the run completes and its files are written.

`--resume` restores a snapshot file (checked against the model id) and
continues (the record names the target's scenario); `--param` overrides are
then applied to the restored state, and parameters that require a reset are
refused because a reset would discard it.
Snapshot files do not record the seed: models that report it as a parameter
give it back, for the others (Life, for example) pass the seed of the saved
run with `--seed` so that the record carries it:

```text
$ emergence-cli run conway/random-soup-torus --ticks 100 --seed 42 --resume soup.elsnap --quiet
METRIC           FINAL       MEAN        MIN        MAX  UNIT
population         583    709.307        577        812  cells
density      0.0355835  0.0432927  0.0352173  0.0495605
bbox_width         128        128        128        128  cells
bbox_height        128        128        128        128  cells
generation         600        550        500        600  generations
engine               0          0          0          0
births             164        221        158        280  per tick
deaths             187    222.208        139        310  per tick
```

With the metapixel's default step, three ticks are three metagenerations,
each read from the exact B3/S23 pattern:

```text
$ emergence-cli run life.metapixel --ticks 3 --metrics-every 1
model     life.metapixel (Conway within Conway (OTCA metapixel))
scenario  metapixel/metapixel.blinker — Meta-blinker (seed 1)
overrides none
tick 105984 | 3/3 ticks (100%) | 0.2 s | 18.0856 ticks/s
ran       3 ticks in 0.166 s (18.0723 ticks/s): tick counter 0 -> 105984, simulated time 0 -> 105984 generation

METRIC                   FINAL      MEAN     MIN       MAX  UNIT
population              330244    333208  330244    342076  cells
generation              105984     52992       0    105984  generations
metagen                      2      0.75       0         2  metagens
phase                        0         0       0         0  generations
readouts                     3       1.5       0         3  metagens
exact_on_tiles               3      2.25       0         3  tiles
accelerated_on_tiles         3         3       3         3  tiles
mismatches                   0         0       0         0  tiles
ambiguous_tiles              0         0       0         0  tiles
total_mismatches             0         0       0         0  tiles
node_count              531415    381196   11006    531415  nodes
memory_bytes          50334088  25364360  786824  50334088  bytes
```

## compare

Runs every combination of `--seeds` (`a..b` excludes `b`, `a..=b`, or
`1,5,9`) and `--param key=v1,v2,...` sweeps in parallel (rayon; `--jobs N`
limits the threads). Each run is independent and seeded, so results do not
depend on the thread count (checked: `--jobs 1` and 12 threads write
identical summaries for `continuous/lenia.soup`, `environment/temperate` and
`ecosystem/balanced_meadow`). If a run fails (for example a rule its model
rejects), runs not yet started are skipped and the error names the failing
run. The summary file is the `analytics` comparison
table: `.csv` has one row per metric and run (final, mean, standard
deviation, min/max with their ticks, extinction tick), `.json` the full
table. `--metric` restricts the printed table, not the file.

```text
$ emergence-cli compare conway/random-soup-torus --seeds 1..=3 --param rule=B3/S23,B36/S23 \
    --param width=128 --param height=128 --ticks 500 --metrics-every 10 \
    --metric population --metric births --out compare.csv
6 runs of 500 ticks on 12 threads
run 1/6 done: seed=2 rule=B3/S23 width=128 height=128 (0.04 s)
...
RUN                                       SCENARIO                  END TICK  TIME S  NOTE
seed=1 rule=B3/S23 width=128 height=128   conway/random-soup-torus       500   0.044
seed=1 rule=B36/S23 width=128 height=128  conway/random-soup-torus       500   0.047
seed=2 rule=B3/S23 width=128 height=128   conway/random-soup-torus       500   0.043
seed=2 rule=B36/S23 width=128 height=128  conway/random-soup-torus       500   0.048
seed=3 rule=B3/S23 width=128 height=128   conway/random-soup-torus       500   0.044
seed=3 rule=B36/S23 width=128 height=128  conway/random-soup-torus       500   0.047

METRIC      RUNS  FINAL MEAN  FINAL SD  FINAL MIN  FINAL MEDIAN  FINAL MAX  MEAN OVER TIME
population     6     801.167    177.64        539         766.5       1120         1493.26
births         6       245.5   86.3573        132         234.5        408         549.667
summary of 6 runs written to compare.csv

$ head -2 compare.csv
source,key,run_id,model_id,scenario,seed,samples,first_tick,final_tick,final,mean,std_dev,min,min_tick,max,max_tick,extinction_tick
metric,population,seed=1 rule=B3/S23 width=128 height=128,life.classic,conway/random-soup-torus,1,51,0,500,891,1346.549019607843,831.4678094702846,795,450,5698,0,
```

## validate

For every model (or the given model/scenario ids): two runs from the same
seed must report bit-identical metrics after every tick; a snapshot must
survive encoding and, restored into the same instance and into a fresh one
built with another seed, replay the same metrics (Hashlife's node-store
metrics — node count, arena bytes, root level — describe its cache and are
not compared); plus the checks the model crates' APIs allow: Life-like rules
— every engine that supports the world (dense, bit-packed, sparse, Hashlife)
reaches the model's cells; Hashlife — the library's still lifes, oscillators
and spaceships repeat with their catalogued period and displacement;
Flow-Lenia — relative mass change per step below 1e-6; ecosystem
(ecological rules) — energy and nutrient books balance within 1e-5;
metapixel — the macro states read from the exact B3/S23 pattern after two
metagenerations equal the accelerated prediction. Exit status 3 if anything
fails. Each phase tick here is one `advance(1)` call (one generation for
Hashlife and the metapixel, whatever their step size), which keeps the
checks short; the macro readout check runs whole metagenerations.

```text
$ emergence-cli validate civilization/scarcity ecosystem/balanced_meadow life.metapixel
civilization  (civilization/scarcity, seed 11, 20 ticks per phase)
  PASS  determinism          two runs, 20 ticks, 500 metric values identical
  PASS  snapshot round trip  snapshot at tick 10 (95207 bytes) restored into the same and a fresh instance; 10 ticks replayed identically
ecosystem  (ecosystem/balanced_meadow, seed 7, 20 ticks per phase)
  PASS  determinism                two runs, 20 ticks, 3120 metric values identical
  PASS  snapshot round trip        snapshot at tick 10 (2661322 bytes) restored into the same and a fresh instance; 10 ticks replayed identically
  PASS  energy and nutrient books  after 20 ticks: energy error 6.721e-10, nutrient error 4.181e-9 (limit 1e-5)
life.metapixel  (metapixel/metapixel.blinker, seed 1, 20 ticks per phase)
  PASS  determinism          two runs, 20 ticks, 240 metric values identical
  PASS  snapshot round trip  snapshot at tick 10 (120476 bytes) restored into the same and a fresh instance; 10 ticks replayed identically (cache metrics node_count, memory_bytes not compared)
  PASS  macro readout        2 readout(s) up to generation 70656: all 18 tile readings match the accelerated prediction

3 target(s) validated: 3 passed, 0 failed
```

`emergence-cli validate` with no arguments checks the 17 default scenarios;
all pass:

```text
$ emergence-cli validate --ticks 20
...
civilization  (civilization/abundant, seed 11, 20 ticks per phase)
  PASS  determinism          two runs, 20 ticks, 500 metric values identical
  PASS  snapshot round trip  snapshot at tick 10 (94724 bytes) restored into the same and a fresh instance; 10 ticks replayed identically

17 target(s) validated: 17 passed, 0 failed
```

## bench

Runs `--warmup` ticks (not timed), then times each of `--ticks` ticks on
its own: one `step()` call, as for `run`, measured with the monotonic clock.
Ticks per second is measured ticks divided by their summed time; memory is
the model's own `memory_bytes()` estimate; `--json` writes every number,
including the tick counter and simulated time ranges and the machine.

```text
$ emergence-cli bench continuous/lenia.orbium --ticks 500 --warmup 50 --json bench.json
machine   Apple M2 Max | 12 logical cores (12 physical) | 64.0 GiB RAM | macOS 27.0.1 | aarch64 | debug assertions off
rayon     12 threads
target    continuous/lenia.orbium (continuous.lenia), seed 1
ticks     50 warm-up + 500 measured (tick counter 50 -> 550, simulated time 5 -> 55 time unit (T ticks))

MEASURE                         VALUE  UNIT
ticks per second              6342.07  ticks/s
time per tick, mean          0.157677  ms
time per tick, median        0.156834  ms
time per tick, p95           0.162752  ms
time per tick, min           0.156541  ms
time per tick, max           0.173167  ms
model memory after warm-up  774.1 KiB
model memory at the end     774.1 KiB
results written to bench.json
```

For the metapixel one tick is one metageneration (its default step), so
these are times per metageneration; Hashlife memoises the periodic
blinker, so later metagenerations are much faster than the first ones:

```text
$ emergence-cli bench life.metapixel --ticks 10 --warmup 2
machine   Apple M2 Max | 12 logical cores (12 physical) | 64.0 GiB RAM | macOS 27.0.1 | aarch64 | debug assertions off
rayon     12 threads
target    metapixel/metapixel.blinker (life.metapixel), seed 1
ticks     2 warm-up + 10 measured (tick counter 70656 -> 423936, simulated time 70656 -> 423936 generation)

MEASURE                        VALUE  UNIT
ticks per second             1033.01  ticks/s
time per tick, mean         0.968046  ms
time per tick, median        0.17675  ms
time per tick, p95           4.40762  ms
time per tick, min          0.097042  ms
time per tick, max           7.13304  ms
model memory after warm-up  24.0 MiB
model memory at the end     48.0 MiB
```

A debug build says `debug assertions on (build with --release for
representative numbers)`.

## convert

Reads RLE, plaintext, Life 1.05 and Life 1.06 through the `patterns` crate
(format from extension and content) and Golly macrocell files through
`hashlife::macrocell`; writes the format given by `--format` or the output
extension (`.rle`, `.cells`, `.lif` = Life 1.06, `.mc`). Plaintext and the
Life formats store no rule; macrocell needs a two-state Life-like rule
without B0.

```text
$ emergence-cli convert gosper-glider-gun.rle gun.mc
gosper-glider-gun.rle (RLE) -> gun.mc (macrocell): 36 live cells, bounding box 36×9, rule B3/S23
$ emergence-cli convert gun.mc gun.cells
gun.mc (macrocell) -> gun.cells (plaintext): 36 live cells, bounding box 36×9, rule B3/S23
```

## Limitations

* The CLI drives models through the object-safe `DynSimulation` interface:
  it can set parameters but not send model commands (edits, interventions).
* A tick is one `step()` of the model (see `run`); a sample every K ticks
  holds counter metrics of its own tick only, not sums over the K ticks.
* When resuming a model that does not report its seed as a parameter (Life,
  for example), the run record uses `--seed` or the scenario's seed, and the
  CLI says so.
* Benchmarks time single `step()` calls; for models whose ticks take
  microseconds the clock's own cost (tens of nanoseconds) is included.
* `validate` checks the default scenarios of the models (or the targets
  given) without parameter overrides.
