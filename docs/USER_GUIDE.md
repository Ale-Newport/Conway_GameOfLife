# User guide

Emergence Lab is a desktop laboratory for artificial life. It opens on a
**dashboard** with one card per laboratory. Clicking a card, or starting with
`--mode <slug>` (see [`INSTALLATION.md`](INSTALLATION.md)), opens that
laboratory. All laboratories share one application shell; each adds its own
tools, panels and overlay.

The shell reference is [`UI_SHELL.md`](UI_SHELL.md) §3. The model behind each
laboratory is described in [`SIMULATION_MODELS.md`](SIMULATION_MODELS.md).

## Contents

1. [The shell](#1-the-shell)
2. [Classic Conway](#2-classic-conway)
3. [Ecosystem Simulator](#3-ecosystem-simulator)
4. [Custom Life Creator](#4-custom-life-creator)
5. [Conway within Conway](#5-conway-within-conway)
6. [Cellular Automata Lab](#6-cellular-automata-lab)
7. [Evolution Lab](#7-evolution-lab)
8. [Continuous Life](#8-continuous-life)
9. [3D Life](#9-3d-life)
10. [Emergent Civilizations](#10-emergent-civilizations)

---

## 1. The shell

### 1.1 Layout

```text
┌ top bar: logo · mode title · scenario · panels · Save · Load · Export · Fullscreen · Tutorial · Help · Theme · About ┐
├ toolbar strip (some modes) ───────────────────────────────────────────────────────────────────────────────────────────┤
│ Tools (left) │                         world viewport (+ overlay, top left)                    │ Properties │ Parameters │ Rules │ Metrics │
├ bottom bar: Play/Pause · Step · Step × N · Reset · Clear · Randomize · Undo · Redo · Speed · timeline · readouts ─────┤
```

**Top bar**

| Control | What it does |
|---|---|
| Logo / "Emergence Lab" | Returns to the dashboard. |
| Model name | Hover over it to see the model's summary. |
| Scenario | Lists the mode's built-in scenarios. Picking one replaces the running simulation. |
| Panel toggles | Show or hide the Tools panel and the right panel. |
| Save, Load, Export | See §1.4. |
| Fullscreen | Toggles fullscreen (F11). |
| Tutorial | Opens the mode's tutorial (§1.6). |
| Help | Lists the shortcuts (F1). |
| Theme | Switches between Dark lab, Light and High contrast. |
| About | Shows the version, the data folder, libraries and licences, model attributions, pattern licence counts and the OTCA provenance. |

**Left panel, "Tools".** Mode-specific tools. It is collapsible and
resizable. Modes without tools do not show it.

**Right panel.** It has four tabs:

| Tab | Contents |
|---|---|
| Properties | Mode content. |
| Parameters | Controls generated from the model's parameter specs. Each control has a badge saying when a change applies: *live*, *next tick* or *on reset*. Each control has a reset-to-default button. *Reset now* appears for reset-only changes. |
| Rules | The model's identity, its time step and determinism, and a formal statement of the rules being executed. |
| Metrics | Time-series plots with a metric picker, a time window (all or the last N ticks), a latest/min/max/mean table and an optional histogram. |

**Bottom bar**

| Control or readout | What it does or shows |
|---|---|
| Play / Pause | Runs or pauses the simulation. |
| Step | Runs one tick. |
| Step × N | Runs N ticks (N is editable). |
| Reset | Returns to the scenario's initial state. You can undo it. |
| Clear, Randomize + seed | Shown only when the mode supports them. |
| Undo / Redo | Undo or redo the last change. |
| Speed | Logarithmic slider from 1 to 10 000 ticks/s, plus **Max**. |
| Timeline | Shows stored history snapshots. Click a marker to restore it. |
| State | RUNNING, PAUSED or FINISHED. |
| Generation / tick | The current generation or tick. |
| Simulated time | Shown only when one tick is not one unit, for example 6 minutes per tick in the ecosystem. |
| Population | Shown if the model publishes one. |
| Ticks/s | Measured rate and target. |
| Generations/s | Shown when it differs from ticks/s, as with Hashlife jumps. |
| Tick duration | Time taken by one tick. |
| FPS | Frame rate. |
| Memory | Process memory, the model's estimate and stored snapshots. |

The simulation runs on its own thread. A slow frame never changes the
results. If the model is slower than the requested speed, the bar shows the
measured rate.

### 1.2 Keyboard shortcuts

These shortcuts are ignored while a text field or other widget has keyboard
focus. Press Esc, or click the world, to give the keyboard back to the shell.

| Keys | Action |
|---|---|
| Space | Play / pause |
| N | Step one tick |
| Shift+N | Step the configured number of ticks |
| R | Reset (undoable) |
| Ctrl/Cmd+Z | Undo |
| Shift+Ctrl/Cmd+Z, Ctrl/Cmd+Y | Redo |
| `=` or `+` (Shift+`=`), Numpad + | Faster: 1, 2, 5, 10 … 10 000 ticks/s, then Max |
| `-`, Numpad - | Slower |
| Home | Dashboard |
| Tab | Hide or show the side panels, toolbar and bottom bar |
| F11 | Fullscreen |
| F12 | Screenshot |
| F1 | Help |

Mode shortcuts are listed in each section below. They never use a shell key,
and they are ignored while Ctrl, Cmd or Alt is held.

### 1.3 Undo and history

* These actions are undoable:
  * edits (one step per gesture);
  * Reset;
  * loading a save;
  * timeline jumps.
* The timeline markers are stored snapshots. Clicking a marker pauses the run
  and restores that snapshot; the restore is itself undoable.
* If the history budget is full, the oldest snapshot is evicted, and a click
  may land on the neighbouring snapshot.

### 1.4 Save, load and export

**Save** names the current state and writes `<data>/saves/<mode>/<name>.elsnap`:

* The file holds the complete state, including the random generator, so a
  loaded run continues exactly as the original would have.
* Tick *Overwrite* to replace an existing save.
* The file is written in the background. A notification reports the result.

**Load** lists the saves of this mode with their tick, date and size:

* Saves made by another model, and unreadable files, are listed but disabled,
  with the reason.
* Loading pauses the run, can be undone, and clears the plots.
* *Delete…* asks for confirmation first.

**Export** offers:

| Export | Output |
|---|---|
| Metrics as CSV or JSON | `<data>/exports/<mode>-<stamp>-metrics.csv` (or `.json`) |
| Screenshot | One PNG |
| PNG sequence of N rendered frames | `<data>/screenshots/<mode>-<stamp>-frames/` |

Screenshots capture the whole window, including the interface. Press Tab
first to hide the panels. A sequence saves one PNG per rendered frame, not per
simulation tick.

Some modes have their own files:

* Classic Conway: pattern import and export.
* Custom Life Creator: a system library.

See their sections. The data folder location is in
[`INSTALLATION.md`](INSTALLATION.md#user-data-folder).

### 1.5 Themes

| Theme | Description |
|---|---|
| Dark lab | The default. |
| Light | The mode accent is darkened to at least 3:1 contrast on panels. |
| High contrast | Black surfaces, white text, thicker outlines. The accent is brightened to at least 7:1. |

Each mode's accent colour tints the shell while that mode is open.

### 1.6 Tutorials

**Tutorial** opens a window with two parts:

* **Steps** (previous / next). Many steps have an action button that loads a
  pattern or scenario, or sets up the view.
* **Concepts**: background reading in Markdown.

The dashboard has its own tutorial.

---

## 2. Classic Conway

Conway's Game of Life, exactly B3/S23 (model `life.classic`).

**Worlds**

* Topologies: bounded, torus, Klein bottle, cross-surface and infinite plane.
  World size is set in Parameters.
* Nine scenarios, from the top-bar picker.
* The **Engine** card selects the engine: dense, bit-packed, sparse, Hashlife
  or automatic. In automatic mode it shows the reason for its choice.

**Tools**, selected with keys 1–7:

| Tool | Use |
|---|---|
| Draw / Erase | Paint or erase cells. |
| Line | Draw a line of cells. |
| Rectangle | Draw a rectangle, optionally filled or erasing. |
| Fill | Random fill. |
| Selection | Copy, cut, paste, clear, move, rotate or flip a region. |
| Pattern stamp | Place a pattern, with rotation (Q / E), mirror (M) and a ghost preview. |

Each gesture is one undo step.

**Patterns window** (P to open; I opens import):

* Catalog browser with categories, search, thumbnails, metadata, licence and
  provenance.
* Import of RLE, plaintext and Life 1.05 / 1.06, with line/column error
  messages.
* RLE export to the clipboard or the exports folder.

**Time**

* Generations per tick.
* Hashlife jumps of 2^k generations (k ≤ 40); J opens the jump.
* Generation numbers are always shown exactly.

**Rendering**

* Eight styles: classic, flat, circles, neon, age, activity trails, heat and
  density. V and Shift+V cycle through them.
* Five colour schemes.
* G toggles the cell grid. F frames the pattern.
* When you zoom out, one texel stands for 2^k × 2^k cells.

**Properties and metrics**

* Population, births and deaths, bounding box, density and generation rate.
* Hashlife memo statistics.
* A period analysis, run on a copy of the world, classifies the pattern as a
  still life, an oscillator or a spaceship with its displacement.

**Other shortcuts**

| Keys | Action |
|---|---|
| `[` / `]` | Brush size |
| Ctrl/Cmd+C / X / V | Copy / cut / paste |
| Delete | Clear the selection |
| Esc | Cancel |

**Tutorial:** seventeen steps that place catalog patterns, plus a concepts
page.

## 3. Ecosystem Simulator

A 3D view of individual plants and animals on generated terrain, with weather,
hydrology, fire, disease and nutrient accounting. One tick is 6 simulated
minutes. There are 14 scenarios, for example `wolf_sheep_grass`,
`balanced_meadow`, `forest_fire` and `invasive_species`.

**Tools.** Inspect is the default; the others act on a left click:

| Tool | Effect |
|---|---|
| Inspect | Select animals and trees. |
| Spawn | Add animals of a chosen species. |
| Cull | Kill animals within a radius. |
| Plant tree | Plant a tree seedling. |
| Ignite | Set a cell on fire. |
| Flood | Flood around a point. |
| Earthquake | Earthquake around a point. |
| Landslide | Landslide around a point. |
| Volcano | Volcano at a point. |
| Tornado | Tornado from a first click to a second click. |

**Cameras:** orbit, top-down, ground level, cinematic, and follow the selected
animal.

**Overlays:** temperature, moisture, water, nutrients, biomass, fire,
territories, disease, pheromone and pollination.

**Properties:**

* scenario;
* inspector for the selected organism;
* species;
* populations;
* food web;
* energy;
* climate;
* deaths.

**Shortcuts**

| Key | Action |
|---|---|
| Esc | Clear the selection and the tool |
| F | Follow the selection |
| C | Cinematic camera |
| G | Ground-level camera |
| T | Top-down camera |
| O | Orbit camera |
| M | Frame the map |

## 4. Custom Life Creator

Design a life system without writing code, then watch the population it
produces (model `custom_life`). Seven presets are in the scenario picker:

* bacterial growth;
* virus and cells;
* competitive colonies;
* alien ecosystem;
* producers–consumers;
* artificial chemistry;
* robot swarm.

**System editor** (E opens and closes it). Changes are validated live. Apply
sends them to the running simulation; each change takes effect at the next
tick or the next reset, depending on its kind. The editor has these tabs:

| Tab | Contents |
|---|---|
| Entities | Entity types. |
| Rules | A visual rule builder, or DSL text with highlighting and `line:col` diagnostics. |
| Relationships | A relationship graph with force-directed layout. |
| World | Size, topology and fields. |
| Scripts | Sandboxed Rhai scripts. |
| Files | The system library and file transfers. |

* The library is `<data>/scenarios/custom-life/`.
* Export writes RON or JSON to `<data>/exports/`.
* Import accepts plain systems and preset files.
* A draft with errors still loads and shows its diagnostics.

The rule language is documented in [`RULE_ENGINE.md`](RULE_ENGINE.md).

**World tools** (keys 1–4): select, spawn, remove and paint.

**Viewing**

* C cycles entity colouring: type, energy, state.
* V cycles field overlays.
* G toggles glyphs.
* L follows the selection.
* F fits the world in view.
* Arrow keys pan.
* Delete or Backspace removes the selected entity. Esc clears the selection.

**Behaviour to know**

* Leaving the mode keeps the world and the editor draft. Coming back continues
  from them.
* Changes made in the Parameters tab, by undo/redo or by loading a save also
  update the draft.

## 5. Conway within Conway

Brice Due's OTCA metapixel: every tile is a 2048 × 2048 region of a single
B3/S23 Life pattern, run cell by cell with Hashlife (model `life.metapixel`).

* The tiles are read from `assets/patterns/otca/` at start-up. They are
  GPL-2.0-or-later data.
* Seven scenarios include a blinker, a block, a glider, a galaxy, a soup, the
  von Neumann parity rule and a single B0 tile.

**Two linked layers** (L switches the layer):

| Layer | What it shows |
|---|---|
| Physical cells | The Life cells themselves. |
| Macro layer | One texel per tile. M switches its source: exact readout, live core counts, or the accelerated prediction (clearly labelled). |

In Auto, the macro layer fades in as you zoom out. A badge always names the
layer on screen.

**Tools** (keys 1–3):

* Inspect tiles.
* Draw macro cells.
* Stamp library patterns as macro patterns.

Every edit rebuilds the construction at generation 0. A whole drag is one
undo step.

**Rule and timing**

* Program the macro rule with B/S digits or presets. C shows the rule table.
* Set the neighbourhood with a compass control.
* Set the grid size, seed and random soups.
* `[` / `]` change the Hashlife step size.
* J jumps to the next readout. Readouts happen every 35 328 generations.
* A progress display shows the position in the cycle and the cost of each
  tick.

**Properties**

* Tile inspector: states, core count, rule bits, neighbour channels and
  parts, with a mini map.
* Validation: each exact readout is compared with the prediction, and
  populations and core counts are compared with Golly's golden table
  (`otca-spec.json`).
* Construction and Hashlife statistics, and credits.

**Other shortcuts**

| Key | Action |
|---|---|
| F | Frame the grid |
| T | Frame the selected tile |
| G | Tile grid |
| O | Tile parts |
| Esc | Clear the selection |

**Tutorial:** twelve steps and a concepts page.

## 6. Cellular Automata Lab

One session at a time, from five families:

| Family | Model | Highlights |
|---|---|---|
| Life-like | `ca.life_like` | Presets (HighLife, Seeds, Day & Night, Life without Death, …), any typed B/S rule with live validation, B/S toggles |
| Generations | `ca.generations` | Multi-state colours and legend; Brian's Brain, Star Wars, typed B/S/C rules |
| Wireworld | `ca.wireworld` | Paint head, tail or conductor; draw wires; stamp the bundled diode, clocks and gates |
| Langton's ant | `ca.langtons_ant` | Turmite rules (RL, LLRR, …), ant markers with heading, follow mode, add and remove ants |
| Elementary CA | `ca.wolfram` | Rule number and a clickable 8-case table, equivalent rules, measured behaviour class, space-time diagram growing downward |

There are twenty scenarios. The available tools depend on the family: inspect,
draw, erase, line, fill, stamp, ant, remove and toggle.

**Shortcuts**

| Keys | Action |
|---|---|
| 1–6 | Tools |
| `[` / `]` | Brush size |
| `,` / `.` | Paint state |
| Q / E | Turn the circuit stamp or the new ant |
| F | Frame the world |
| G | Grid lines |
| T | Colouring: states, trail, age, activity |
| V | Follow the ant or the newest row |

The McIntosh AND gate is GPL-2.0-or-later data. It is read from
`assets/patterns/wireworld/` at run time.

## 7. Evolution Lab

Prey and optional predators with heritable genomes live on a 3D diorama of
the model grid (model `evolution`). There is no fitness function: selection
emerges from survival and reproduction. Six scenarios:

* speed selection;
* thermal adaptation;
* arms race;
* food scarcity;
* island evolution;
* r vs K.

**Tools panel**

* Replicates.
* World tools (keys 1–5): select, add food, cull, spawn a genome, predators.
* Climate and barriers.
* A custom genome editor.

**Properties**

* Inspector for the selected organism.
* Gene charts over time.
* Genealogy tree.
* Regions.

**Overlay:** status, legend, scale bar and a hover card.

Body shape reflects genes: size, speed, thermal optimum and perception.

**Shortcuts**

| Key | Action |
|---|---|
| Esc | Back to Select, clear the selection |
| F | Follow the selection |
| G | Genealogy of the selection |
| V | Next ground tint: temperature, food, regions, plain |
| C | Next camera preset |
| `[` / `]` | Brush size |

## 8. Continuous Life

Lenia, Expanded Lenia, SmoothLife and Flow-Lenia (models
`continuous.lenia`, `continuous.smoothlife`, `continuous.flow_lenia`).

* Scenarios are grouped by model in the picker:
  * Orbium (two variants), Scutium, Hydrogeminium, Tessellatium, a K = 3 fish
    and a Lenia soup;
  * a SmoothLife glider and a SmoothLife soup;
  * a Flow-Lenia demo.
* The Tools panel also has model buttons.

**World**

* Only the visible region is requested from the model.
* Colormaps: Lenia blue, viridis, inferno, magma, plasma, turbo-like and
  grayscale, with optional bilinear smoothing. C cycles them.
* Multi-channel worlds are shown as an RGB composite.
* Layers (L cycles them):

  | Layer | Note |
  |---|---|
  | State | — |
  | Potential | SmoothLife: the fillings |
  | Growth | Flow-Lenia: the affinity |

**Tools** (keys 1–4):

| Tool | Use |
|---|---|
| Inspect | Read values. |
| Brush / Eraser | Soft brush with value, radius, softness and channel. One stroke is one undo step. |
| Creature stamp | Place a creature, with rotation (Q / E), scale and blend previews. |

`[` / `]` resize the brush or the stamp.

**Properties**

* Inspector.
* Kernel profile and growth-function plots that follow the live rule.
* Live rule and kernel parameters.
* Mass and speed series.
* Channel statistics.

**Overlay:** legend, time step, regime, scale bar and hover readout.

**Other shortcuts**

| Key | Action |
|---|---|
| T | Centroid trail |
| G | Cell grid |
| F | Frame the world |

**Tutorial:** twelve steps and a concepts page.

## 9. 3D Life

Outer-totalistic cellular automata in three dimensions (model `voxel.life`):

* Bays' 4555 and 5766;
* Softology multi-state rules;
* Moore-26 and von Neumann-6 neighbourhoods.

These are **not** Conway's 2D rules, and the mode says so. There are twelve
scenarios, for example the 5766 glider, Rokicki's ship, Life embedded in 5766
on two planes, and Clouds.

**Tools panel**

* Tool selection.
* Rule editor with live validation and presets.
* Seeding.
* Cut planes.
* Region box.
* Display options.
* Camera presets.
* A list of shortcuts.

**Properties**

* Scenario, rule and population.
* Probe.
* Region counts.
* 2D slice inspector.
* Plots.
* Render statistics.

**Shortcuts**

| Keys | Action |
|---|---|
| 1–4 | Camera: aerial, isometric, free orbit, strategic |
| F / G | Frame the world / the population |
| C | Colour by state, age or height |
| X | Opaque, ghost dying cells, X-ray |
| K | Cut on / off |
| L | Cut axis |
| `[` / `]` | Upper cut plane down / up |
| Shift+`[` / Shift+`]` | Lower cut plane down / up |
| B | World box |
| T | Turntable rotation |
| M | Region box |
| I | Inspect tool |
| P | Place tool (Shift+click erases) |
| O | Erase tool |

The orbit camera also uses W A S D or the arrow keys to pan, and Q / E to
rotate.

## 10. Emergent Civilizations

People with needs, skills, inventories and relationships on a tile world
(model `civilization`). Roads, groups, territories and markets emerge from
their decisions. Six scenarios:

* abundant;
* scarcity;
* cooperation vs competition;
* two strategies;
* territorial expansion;
* trade routes.

**World view**

* Terrain, water and trees.
* Bushes and rocks whose look follows the resource stocks.
* Buildings: huts, group stores, and farm plots with crop stages.
* Roads from the trail grid.
* A territory overlay in group colours.
* Animated people with activity badges.

**Tools panel**

* Intervention tools: spawn people, add or remove resources, disaster, and
  regeneration.
* Layers.
* Camera presets: strategic, isometric, aerial, free.

**Properties**

When a person is selected, the inspector shows:

* needs;
* goal and route;
* inventory;
* exchange rates;
* skills;
* preferences;
* relationships;
* recent actions.

Buildings can also be inspected. The tab also has society charts, a
trade-network graph, groups and activities.

**Overlay:** clock, legend and scale bar.

**Shortcuts**

| Keys | Action |
|---|---|
| 1–5 | Inspect, Spawn, Add resources, Remove resources, Disaster |
| T | Territory |
| P | Trails and roads |
| I | Activity badges |
| V | Vegetation |
| F | Follow the selected person |
| C | Next camera preset |
| G | Frame the map |
| Esc | Deselect / inspect tool |
