# Third-party assets and data: licences and provenance

This document lists every third-party asset and data source in the repository:
its licence, author, source, where it is used, and any known issues. It is an
engineering record, not legal advice.

The project's own code and original data are licensed `MIT OR Apache-2.0`
(`LICENSE`, `LICENSE-MIT`, `LICENSE-APACHE`).

Primary records kept next to the files:

* `assets/models/manifest.toml`
* `assets/models/*/LICENSE.txt` and `ATTRIBUTION.txt`
* `assets/patterns/catalog.toml` (the `license` and `license_note` fields)
* `assets/patterns/otca/PROVENANCE.md`
* `assets/patterns/wireworld/PROVENANCE.md`
* `docs/RESEARCH.md` §8 (inventory and policy)

The in-app **About** window shows the 3D-model attributions, the
pattern-library licence counts and the OTCA provenance.

## 1. Summary

| Asset / data | Licence | Files | Embedded in binaries? |
|---|---|---|---|
| Quaternius 3D models | CC0 1.0 (see caveat 3.3) | 74 GLB + 15 PNG textures in `assets/models/quaternius/` | No (loaded at run time) |
| Kenney Nature Kit 3D models | CC0 1.0 | 49 GLB in `assets/models/kenney/nature_kit/` | No |
| Sherkiz hawk and rabbit | CC BY 3.0 (see caveat 3.2) | `assets/models/polypizza-sherkiz/{hawk,rabbit}.glb` | No |
| OTCA metapixel tiles (Brice Due) | Treated as GPL-2.0-or-later (see 3.4) | 4 RLE files in `assets/patterns/otca/` | No |
| Golly OTCA demo tilings | GPL-2.0-or-later | 3 `.mc.gz` in `assets/patterns/otca/` | No |
| Golly-derived Life patterns | GPL-2.0-or-later | 9 RLE files in `assets/patterns/life/` | No |
| McIntosh Wireworld AND gate (via Golly) | GPL-2.0-or-later | `assets/patterns/wireworld/and-gate-mcintosh.rle` | No |
| Life Lexicon-derived Life patterns | CC BY-SA 3.0 | 39 RLE files in `assets/patterns/life/` | No |
| Hand-transcribed classic Life objects | "Public-domain mathematical object (transcribed)" | 32 RLE files in `assets/patterns/life/`, plus 18 in `crates/patterns/src/catalog/builtin.rs` | The 18 built-ins only |
| Lenia creatures (Bert Chan, Open Science Lenia) | MIT | 6 scenario files in `scenarios/continuous/` | Yes (`include_str!`); MIT notice included in each file |
| Species parameters (AnAge, PanTHERIA, Hirt et al. 2017) | Facts / derived values (see §2.5) | `assets/species/**/*.ron` | Yes (`include_str!`) |

Original project data is under the project licence:

* `otca-spec.json`;
* `catalog.toml`;
* the own-layout Wireworld circuits;
* the scenario files;
* the shaders `assets/shaders/*.wesl`.

## 2. Details

### 2.1 3D models (`assets/models/`)

Each model's source, licence, author, retrieval date and modifications are in
`assets/models/manifest.toml`, which has 125 `[[model]]` entries.

| Source key | Author | Licence | Pack page | Obtained from | Models | Where used |
|---|---|---|---|---|---|---|
| `quaternius/ultimate-animated-animal-pack` | Quaternius | CC0 1.0 | https://quaternius.com/packs/ultimateanimatedanimals.html | https://poly.pizza/bundle/Animated-Animal-Pack-ILAPXeUYiS | 12 | `organism-render` (animals) |
| `quaternius/farm-animal-pack` | Quaternius | CC0 1.0 | https://quaternius.com/packs/farmanimal.html | https://poly.pizza/bundle/Farm-Animal-Pack-1kUvRTPLzT | 3 | `organism-render` |
| `quaternius/easy-enemy-pack` | Quaternius | CC0 1.0 | https://quaternius.com/packs/easyenemy.html | https://poly.pizza/u/Quaternius | 5 | `organism-render` (critters) |
| `quaternius/animated-monster-pack` | Quaternius | CC0 1.0 | https://quaternius.com/packs/animatedmonster.html | https://poly.pizza/m/hNO9XvjlKa | 1 (bat) | `organism-render` |
| `quaternius/animated-fish-pack` | Quaternius | CC0 1.0 | https://quaternius.com/packs/animatedfish.html | https://poly.pizza/bundle/Animated-Fish-Bundle-ZkGbjS8m8g | 7 | `organism-render` (fish) |
| `quaternius/stylized-nature-megakit` | Quaternius | CC0 1.0 | https://quaternius.com/packs/stylizednaturemegakit.html | https://poly.pizza/bundle/Stylized-Nature-MegaKit-T34GZFA0fm | 46 | `organism-render` (flora, fungi, rocks) |
| `kenney/nature-kit` | Kenney (www.kenney.nl) | CC0 1.0 | https://kenney.nl/assets/nature-kit | `kenney_nature-kit.zip` v2.1 (2020-04-29) | 49 | `organism-render` (flora, rocks) |
| `polypizza/sherkiz-hawk` | Sherkiz (Poly Pizza user) | CC BY 3.0 | https://poly.pizza/m/RkN6MEbP6g | https://static.poly.pizza/2a7aca61-a8f6-45f6-950e-003d4bfcab45.glb | 1 | `organism-render` |
| `polypizza/sherkiz-rabbit` | Sherkiz (Poly Pizza user) | CC BY 3.0 | https://poly.pizza/m/Km89u3twcR | https://static.poly.pizza/8a01980e-f9cd-47c2-97b0-bad9b131ef6f.glb | 1 | `organism-render` |

All models were retrieved on 2026-11-09.

**Modifications.** Each folder's `ATTRIBUTION.txt` records the changes:

| Models | Changes |
|---|---|
| Quaternius | Duplicate animation takes removed; `metallicFactor` 0.4 → 0.0; NaN skin weights repaired in `wasp.glb`; nature textures moved to shared PNGs; foliage alpha mode changed |
| Kenney | `metallicFactor` 1.0 → 0.0; unused buffer data compacted |
| Sherkiz | Byte-identical to the downloads (no changes) |

**Required credit lines** (CC BY 3.0, from
`assets/models/polypizza-sherkiz/ATTRIBUTION.txt`):

* "Hawk Lp Rigged" by Sherkiz, https://poly.pizza/m/RkN6MEbP6g, licensed under
  CC BY 3.0 (https://creativecommons.org/licenses/by/3.0/). Used unmodified.
* "Rabbit" by Sherkiz, https://poly.pizza/m/Km89u3twcR, licensed under CC BY
  3.0. Used unmodified.

### 2.2 Life pattern library (`assets/patterns/life/`, `catalog.toml`)

There are 80 patterns. Every `[[pattern]]` entry has `license`, `license_note`,
`source` and `facts_source` fields.

| Licence field | Count | Notes |
|---|---|---|
| `CC-BY-SA-3.0 (Life Lexicon) attribution kept; cell data is a mathematical object` | 39 | Cell layouts transcribed from the **Life Lexicon, Release 29**. © Stephen Silver 1997–2018, with later updates by Dave Greene and David Bell. CC BY-SA 3.0. Home page: https://conwaylife.com/ref/lexicon/lex_home.htm. 37 files say "transcribed from"; 2 were extracted from a Lexicon diagram with a demonstration glider positioned by the project's verifier. |
| `GPL-2.0-or-later (Golly pattern collection)` | 9 | Copied from Golly 5.1 (SourceForge git `9a7079a84a24`), keeping the original headers. One file (Breeder 1) was converted from Life 1.05 to RLE with its cells unchanged. |
| `Public-domain mathematical object (transcribed)` | 32 | Transcribed by hand and cross-checked against Lexicon R29 diagrams. Whether such small configurations can be copyrighted has not been resolved (`docs/RESEARCH.md` §8.2 item 5). |

The 9 GPL patterns, with discoverers as recorded in `catalog.toml`:

| Id | File | Discoverer(s) |
|---|---|---|
| `period-52-glider-gun` | `life/guns/period-52-glider-gun.rle` | Entity Valkyrie, Chris Cain, Matthias Merzenich, Adam P. Goucher, Dave Greene |
| `p59-snark-gun` | `life/guns/p59-snark-gun.rle` | Dave Greene (Snark by Mike Playle) |
| `p165-mwss-gun` | `life/guns/p165-mwss-gun.rle` | David Bell and Jason Summers (reaction by Dieter Leithner) |
| `p210-loafer-gun` | `life/guns/p210-loafer-gun.rle` | Shannon Omick |
| `engineless-rake-2c5-p185` | `life/rakes/engineless-rake-2c5-p185.rle` | Paul Tooke (updates by Jason Summers) |
| `blom` | `life/methuselahs/blom.rle` | Dean Hickerson |
| `breeder-1` | `life/breeders/breeder-1.rle` | Bill Gosper's group (MIT) |
| `switch-engine-breeder` | `life/breeders/switch-engine-breeder.rle` | Helmut Postl |
| `sawtooth-177` | `life/misc/sawtooth-177.rle` | 'thunk' (conwaylife.com forum user) |

Where the library is used:

* Classic Conway: pattern browser and stamps.
* Conway within Conway: macro stamps.
* `cellular-automata` scenarios. For example, `scenarios/conway/breeder-hashlife.ron`
  refers to `breeder-1` by id and needs the asset catalog at run time.
* Tests in `crates/cellular-automata/tests/catalog.rs` and
  `crates/patterns/tests/asset_catalog.rs`.

### 2.3 OTCA metapixel (`assets/patterns/otca/`)

Full details are in `PROVENANCE.md`. Summary:

| File | Author / origin | Source | Licence |
|---|---|---|---|
| `OTCAmetapixel-OFF.rle`, `OTCAmetapixel-ON.rle` | Brice Due (2005–2006) | http://www.cranemtn.com/life/weblog/ (sha256 of the original `.gz` recorded) | No explicit licence; redistributed as GPL-2.0-or-later (see 3.4) |
| `OTCAmetapixel-OFF-B3S23.rle`, `OTCAmetapixel-ON-B3S23.rle` | Derived by the project from the above (3 programming cells added) | — | As above |
| `metapixel-galaxy.mc.gz`, `metapixel-p216-gun.mc.gz`, `metapixel-parity64.mc.gz` | Golly demos (galaxy tiled by Brice Due's `tile-otcamp.pl`; p216 gun by David Eppstein) | Golly source tree `Patterns/HashLife/Metacell/`, git `9a7079a84a24…` | GPL-2.0-or-later |
| `otca-spec.json` | Project (coordinates and golden numbers from our own validation runs) | — | Project licence |

The only edit to the original files is one added `#C Provenance:` line. The
`-B3S23` files were also re-encoded after the cells were added.

Where used:

* `metapixel` reads the tiles with `TileSet::from_dir`.
* `view-metapixel` loads them on a background thread.
* The `metapixel` tests (`construction.rs`, `exact.rs`) read the Golly demos
  from this folder.
* The `hashlife` Golly tests read them from a separate Golly checkout given
  by `GOLLY_PATTERNS`.

### 2.4 Wireworld circuits (`assets/patterns/wireworld/`)

| File | Origin | Licence | How used |
|---|---|---|---|
| `diode.rle`, `clock-8.rle`, `clock-12.rle`, `or-gate.rle`, `xor-gate.rle` | Own layouts | MIT OR Apache-2.0 | Embedded with `include_str!` in `crates/cellular-automata/src/library.rs` |
| `and-gate-mcintosh.rle` | Harold V. McIntosh, from Golly 5.1 `Patterns/WireWorld/gate-AND.mcl` (first gate instance, converted to RLE) | GPL-2.0-or-later | Read from the asset folder at run time (`view-ca-lab` `ASSET_CIRCUITS`) |

Golly's `primes.mc` and the other `.mcl` files are deliberately not bundled.

### 2.5 Other data sources

| Source | Licence | Author / citation | URL | Where used |
|---|---|---|---|---|
| Lenia `animals.json` (Orbium, Scutium solidus, Hydrogeminium natans) | MIT | © 2018 Bert Chan (Chakazul/Lenia @ `adfc5429`) | https://github.com/Chakazul/Lenia | `scenarios/continuous/{orbium-chan,scutium-solidus,hydrogeminium-natans}.ron` (MIT notice in the file) |
| Lenia tutorial (Orbium tutorial configuration, Tessellatium gyrans, K = 3 fish) | MIT | © 2021 Open Science Lenia (lenia-org/Lenia-Tutorial @ `2642cf99`) | https://github.com/lenia-org/Lenia-Tutorial | `scenarios/continuous/{orbium,tessellatium-gyrans,fish}.ron` (MIT notice in the file) |
| AnAge (Human Ageing Genomic Resources) | CC BY 3.0 (per `docs/RESEARCH.md` §8.1) | de Magalhães et al. | https://genomics.senescence.info/species/ | Lifespan, maturity and gestation values ("order of magnitude") in `assets/species/animals/*.ron` comments and fields |
| PanTHERIA 1.0 | "no restrictions" (per `docs/RESEARCH.md` §8.1) | Jones et al. 2009 | https://esapubs.org/archive/ecol/E090/184/ | Derived mammal parameters in species files |
| Hirt et al. 2017 supplement | Springer Nature supplementary data | Hirt et al. 2017 | — | Individual sprint-speed facts cited in species files; no bulk redistribution |
| NetLogo Wolf Sheep Predation | CC BY-NC-SA 3.0 | Wilensky (1997) | https://ccl.northwestern.edu/netlogo/models/WolfSheepPredation | Rules re-implemented in `ecosystem` (`netlogo` rule set). No code or text copied. |

`docs/RESEARCH.md` §8.2 proposes a `DATA_SOURCES.md` for the species
parameters. That file does not exist yet.

## 3. Caveats

### 3.1 Poly Pizza: scripted retrieval against the terms of service

All Quaternius models and both Sherkiz models were downloaded from the Poly
Pizza CDN (`static.poly.pizza`) with scripted requests. `docs/RESEARCH.md` §8.1
records about 80 pages, 121 GLBs and 43 previews. The Quaternius site's own
download buttons point to Google Drive, which returned "Quota exceeded" on
2026-11-09.

Poly Pizza's terms of service forbid automated downloading. This does not
change the licence of the models: the Quaternius files are CC0 and the Sherkiz
files are CC BY 3.0. It is, however, a breach of the site's terms in how the
copies were obtained. Redistributors may prefer to re-download the same models
manually, either from Poly Pizza or from the official Quaternius and Kenney
pages, before a release. The Kenney Nature Kit was downloaded through the
normal public flow on kenney.nl.

### 3.2 Sherkiz CC BY 3.0 models: unverified uploader

The hawk and the rabbit were uploaded to Poly Pizza by the account "Sherkiz"
on 2025-06-28. The account has no linked portfolio or social profile, so the
uploader's identity, and therefore their authority to license the models,
could not be verified (UNVERIFIED).

To keep this contained:

* The two files are isolated in `assets/models/polypizza-sherkiz/` with their
  own `LICENSE.txt` and `ATTRIBUTION.txt`.
* The credit lines are shown in the About window.
* The files are byte-identical to the downloads.

`ATTRIBUTION.txt` recommends replacing them with CC0 models if suitable ones
become available.

### 3.3 Quaternius: CC0 pages versus the Quaternius Asset License

`quaternius.com/license.html` now publishes a "Quaternius Asset License (QAL)
v1.0", last updated 2026-08-28. It forbids redistributing assets as assets.

* The six packs used here were each labelled "License: CC0" on their own pack
  pages at retrieval time (2026-11-09).
* Each Poly Pizza model page states "Licence: CC0 1.0".
* A CC0 waiver is irrevocable (CC0 1.0 §2), so these copies are treated as
  CC0.

The QAL applies to assets released under it. Rule from
`assets/models/quaternius/LICENSE.txt`: do not add a Quaternius pack whose page
shows the QAL instead of CC0.

### 3.4 Golly-derived GPL-2.0-or-later data

These files are GPL-2.0-or-later **data**:

* the OTCA tiles and the three Golly demo tilings;
* the 9 Golly-derived catalog patterns;
* the McIntosh Wireworld AND gate.

For Brice Due's tiles, no licence is stated at the source. The same cell data
is distributed inside Golly's GPL demos, so they are treated as
GPL-2.0-or-later, with credit to Brice Due.

The files are kept as separately licensed data under `assets/`, read at run
time, and are never compiled into a binary. No Golly *code* is used: Golly
C++, `metafier` scripts and `tile-otcamp.pl` were read only.

**Verification.** A grep of every `include_str!` and `include_bytes!` in
`crates/*/src` and `apps/` (on 2026-11-10) found that the embedded files are
only:

| Crate | Embedded files |
|---|---|
| `cellular-automata` | the five own-layout Wireworld circuits; the `scenarios/conway/` and `scenarios/ca-lab/` RON files |
| `ecosystem` | `assets/species/**/*.ron` and `scenarios/ecosystem/*.ron` |
| `continuous`, `voxel-life`, `rule-engine`, `metapixel`, `evolution`, `civilization` | their `scenarios/<mode>/*.ron` files |
| `rendering` | the two shaders |
| `ecosystem`, `civilization` | their own Markdown rules text |
| `view-*` | their own source files (used for tests) |

**No** OTCA file, Golly demo, `assets/patterns/life/` RLE file or
`and-gate-mcintosh.rle` is embedded.

* The 18 patterns embedded in `crates/patterns/src/catalog/builtin.rs` are the
  hand-transcribed classics. None of them is a GPL pattern.
* `scenarios/conway/breeder-hashlife.ron`, which is embedded, contains no
  cells. It refers to the GPL `breeder-1` by catalog id, and the catalog is
  loaded at run time.
* `scenarios/metapixel/*.ron` describe macro patterns, not tile data.

A package that ships the `assets/` folder distributes those GPL files. The
recipient must get the GPL-2.0-or-later terms and access to the source of
those files. `docs/RESEARCH.md` §8.2 item 3 leaves open a choice between two
options:

1. keep the files with a GPL notice next to them;
2. ship only permissive data and import Golly content from a user's own Golly
   folder.

### 3.5 Life Lexicon (CC BY-SA 3.0) attributions

The 39 Lexicon-derived patterns credit the Life Lexicon Release 29 (Stephen
Silver, with updates by Dave Greene and David Bell) in their `license_note`
field. Each of the 39 RLE files also carries `#C Source:` and `#C License:`
header lines naming the Lexicon entry and CC BY-SA 3.0 (checked by grep: 39
files mention CC BY-SA).

Discoverer and year facts in `catalog.toml` come from the Lexicon text, as
recorded in `facts_source`.

Share-alike applies to these data files. If share-alike becomes a problem,
`docs/RESEARCH.md` §8.2 item 4 proposes re-entering the patterns from first
principles.

## 4. Libraries

Rust dependencies are linked, not bundled as assets. Their licences are listed
in `docs/RESEARCH.md` §8.1. Most are MIT OR Apache-2.0; a few entries there are
marked UNVERIFIED. A third-party notice bundle for binary releases has not
been produced yet.
