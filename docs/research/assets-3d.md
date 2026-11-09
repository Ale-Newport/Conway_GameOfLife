# 3D models and animations for the ecosystem: licence-safe sourcing

Research date: 2026-11-09. Scope: animals, plants, trees, fungi, rocks and water for the 3D ecosystem view
(`rendering` and `view-ecosystem` crates, Bevy 0.20.1). Everything marked **UNVERIFIED** was not confirmed
from a primary source.

## 0. Summary

* **Installed:** 125 GLB files (about 31 MB) are now in `assets/models/` and listed in `assets/models/manifest.toml`.
  * 30 animated animals: 28 under CC0 and 2 under CC-BY 3.0.
  * 47 stylised plants, trees, fungi and rocks (CC0) that share 15 textures.
  * 48 low-poly Kenney nature props (CC0).
  * Every file passes validation with the `gltf` 1.4.1 crate, which is the parser that `bevy_gltf` 0.20.1 uses.
* **Main source:** Quaternius, released as CC0. The Ultimate Animated Animal Pack has 12 species, each with
  12–13 clips: Idle, Walk, Gallop, Eating, Attack, Death, Jump, HitReact and others. Other Quaternius packs
  used: Farm Animals, Easy Enemy (rat, spider, wasp, snake, frog), Animated Monster (bat), Animated Fish
  (7 species) and Stylized Nature MegaKit.
  * The official Google Drive links returned *"Quota exceeded"* when downloaded without logging in.
  * The files were therefore taken from Quaternius's own uploads on Poly Pizza, from the direct
    `https://static.poly.pizza/<uuid>.glb` links.
* **Gaps:** there is no CC0 or CC-BY animated GLB, available without a login, for these:
  * small birds (songbird, duck, owl)
  * bear, big cats, boar (the pig model is used as a proxy), squirrel
  * butterfly, bee (the wasp is used as a proxy), ant, beetle, worm
  * turtle, lizard
  * We also have no clip for **swim on land animals**, **lie-down "rest"**, or **death for fish, snake and hawk**.
  * Birds have one CC-BY 3.0 asset, the "Hawk Lp Rigged" model by Sherkiz on Poly Pizza (Fly clip only).
  * The rabbit is also CC-BY 3.0 (Sherkiz, Idle and Walk only).
* **Recommendation: a hybrid approach.** See §7.
  * Use authored CC0 GLBs for the common, recognisable large species.
  * Layer **procedural bone animation** on top of them to fill missing actions.
  * Use **procedural meshes with vertex-shader animation** for small, numerous creatures (insects, small birds, fish schools).
  * Use **procedural meshes generated from the genome** for species created in the evolution lab and in custom-life.
  * Water is drawn with shaders, not meshes.
* **Legal and terms-of-service caveats** (§8):
  * **Poly Pizza's Terms of Service forbid scraping and downloading through automated means.** This work
    used scripted `curl` requests: about 80 HTML page fetches, 121 GLB downloads and 43 JPG previews. The CC0 and
    CC-BY licences of the content are unaffected. Even so, re-download the final set by hand from official
    sources before release (§8.1).
  * quaternius.com now also publishes a non-CC0 **Quaternius Asset License (QAL) v1.0** (2026-08-28). Every pack
    used here is still labelled CC0 on its own page, but this must be re-checked whenever a pack is added.

## 1. Requirements recap

| Requirement | How it is met |
|---|---|
| Animals recognisable without labels (no cubes or spheres) | Low-poly but anatomically proportioned Quaternius models. Contact-sheet render check in §3.4. "Cube World" and "blob" variants were rejected. |
| Animated glTF/GLB with locomotion, eat, attack, idle, death, fly and swim | 30 animated GLBs. The per-model action map is in `manifest.toml` `[model.animation_set]`; coverage is in §5. |
| Stylised-detailed or semi-realistic look | Quaternius animals use flat colours and faceted shading. The MegaKit flora is textured and "Ghibli-inspired" (pack description). |
| CC0 preferred, CC-BY acceptable | 123 files are CC0 and 2 are CC-BY 3.0 (credit lines in `polypizza-sherkiz/ATTRIBUTION.txt`). No NC, ND or unclear licences. |
| Assets stored locally, no runtime URLs | All files are under `assets/models/`. MegaKit textures are relative URIs (`textures/*.png`) next to the GLBs. |
| Total under ~60 MB | 30.9 MB (19.5 MB GLB + 11.4 MB PNG). |

## 2. What is in the repository

```
assets/models/
├── manifest.toml                      # 9 [[source]] + 125 [[model]] entries (TOML, parsed OK by tomllib)
├── quaternius/        LICENSE.txt (CC0 legal code + provenance)  ATTRIBUTION.txt (per-file URLs)
│   ├── animals/   fox wolf wild_dog_husky wild_dog_shiba deer stag horse horse_white donkey cow bull alpaca
│   │              sheep pig zebra                                  (15 files, 7.8 MB)
│   ├── critters/  rat spider wasp snake frog bat                   (6 files, 2.3 MB)
│   ├── fish/      fish_perch fish_clown fish_bream shark dolphin whale manta_ray   (7 files, 0.5 MB)
│   └── nature/    tree_broadleaf_[a-e] tree_pine_[a-e] tree_twisted_[a-e] tree_dead_[a-e] bush bush_flowering
│                  clover_[ab] fern flower_group_[ab] flower_single_[ab] grass grass_wispy_[ab] grass_tall
│                  mushroom_cluster fungus_bracket plant_small_[ab] plant_big_[ab] pebble_round_[ab]
│                  pebble_square_[ab] rock_medium_[a-c]             (47 files, 5.6 MB)
│       └── textures/  15 PNG, 1024², shared by all nature GLBs   (11.4 MB)
├── polypizza-sherkiz/ hawk.glb rabbit.glb  LICENSE.txt (CC BY 3.0 legal code)  ATTRIBUTION.txt (REQUIRED credits)
└── kenney/            LICENSE.txt (original Kenney License.txt + CC0 legal code)  ATTRIBUTION.txt
    └── nature_kit/    48 GLBs: palms, cacti, water lilies, stumps/logs, toadstools, crop growth stages (corn A–D,
                       wheat A–B, bamboo A–B, leafy A–B), boulders/rock spires, low-poly tree/bush/grass LOD proxies
```

### 2.1 Manifest schema (`assets/models/manifest.toml`)

Top-level keys: `schema_version = 1`, `units = "metre"`, `up_axis = "+Y"`. There is one `[[source]]` per pack:
`key`, `license` (SPDX), `author`, `page`, `obtained_from`, `retrieved`. Each `[[model]]` contains:

| field | meaning |
|---|---|
| `id`, `name`, `category`, `species[]` | Stable id. Category is one of `animal.mammal`, `animal.bird`, `animal.fish`, `animal.insect`, `animal.arachnid`, `animal.reptile`, `animal.amphibian`, `plant.tree`, `plant.shrub`, `plant.herb`, `plant.flower`, `plant.grass`, `plant.crop`, `plant.succulent`, `plant.aquatic`, `fungus`, `deadwood`, `rock`. `species` lists the ecosystem species ids the model can represent. |
| `source`, `license`, `author`, `origin_url` | Provenance. `origin_url` is set for MegaKit items; per-file URLs for everything else are in `ATTRIBUTION.txt`. |
| `model_path` | Path relative to `assets/`, for Bevy's `AssetServer`. |
| `scale` | Uniform factor for the scene root that brings the model to its real-world reference size in metres. It is computed from the **rest-pose AABB including skinning** (joint world matrix × inverse bind matrix × weights), measured by script. |
| `real_size_m` | AABB (x, y, z) after `scale`. |
| `forward`, `yaw_offset_deg` | Model-space facing axis, measured from head and tail joint positions. 26 animals face +Z (glTF convention). The wasp faces +X (yaw −90) and the hawk faces −Z (yaw 180). |
| `triangles`, `file_bytes` | Measured. |
| `animation_set` | Our actions (`idle walk run eat attack death fly swim jump rest`) mapped to the **exact clip names in the GLB**, ready for `Gltf::named_animations`. |
| `extra_clips` | Useful clips that are not in our action list: `idle_alt`, `attack_alt`, `hit_left/right`, `jump_in_place`, `walk_slow`, `hit`. |
| `anim_speed` | Playback-rate multipliers for when one clip serves several actions, for example `run = Walk × 2.0` for the spider. |
| `supported_actions`, `missing_actions` | The fallback plan for missing actions is in §7.3. |
| `lod_variants` | No source ships LODs. For MegaKit trees, bushes, grass, mushrooms and rocks the field points to a **stylistic proxy** in the Kenney kit: a far-LOD stand-in, not a true decimation. Otherwise it is `[]`. |
| `notes`, `modifications` | Free text. `modifications` records what our tool changed. The same text is also embedded in each GLB as `asset.extras.emergence_lab_processed`. |

The architecture defines one world unit as one terrain cell (`docs/ARCHITECTURE.md` §3.3). The renderer must
therefore convert with `metres_per_cell`. Small creatures (wasp 3 cm, spider 10 cm, frog 10 cm) will also need a
per-category **display exaggeration** factor at ecosystem zoom levels. This is a renderer setting and is not
stored in the asset.

### 2.2 Processing applied (our own pure-Python tool, no downloaded code executed)

The tool is `scratchpad/assets-work/glbtool.py`. It rewrites the JSON chunk, garbage-collects unused accessors and
buffer views, and compacts the BIN chunk. These problems were found while inspecting the downloads and were fixed
in our copies (allowed by CC0):

1. **Duplicate animation takes.** Every Poly Pizza GLB of the Ultimate Animated Animal Pack contains each clip
   twice: `Walk` and `AnimalArmature|Walk`, and so on. This is FBX2glTF exporting both the FBX takes and the
   actions. In some files the two copies differ:
   * The Bull's `Gallop_Jump` is 2.042 s in one copy and 1.458 s in the other.
   * The prefixed set for the Bull and the Cow lacks `Idle_HitReact_Right`.
   * The Shiba's prefixed `Attack` and `Eating` timings match the Husky's.

   This suggests the prefixed set is a stale copy from a template rig, so **the un-prefixed set is kept**. Every
   kept clip targets only skin joints with rotation and translation channels. File size fell by about 40%
   (Fox: 960 KB → 557 KB).
2. **`metallicFactor`.** Poly Pizza conversions set 0.4 on every material, and Kenney's originals set 1.0. Both
   render as greyish metal under Bevy's PBR, so it was set to **0.0** in the CC0 files. The CC-BY files were left
   byte-identical, so the engine must override the material at load time (§7.4).
3. **NaN skin weights in `wasp.glb`.** Three vertices in each of its two primitives (six in total) had non-finite
   `WEIGHTS_0`. They were replaced with the weights and joints of the nearest valid vertex. Without this fix,
   those vertices would be undefined after skinning.
4. **Foliage alpha.** MegaKit leaves and flowers use `alphaMode: BLEND`, are single-sided and have
   **near-binary alpha**: 0 or 255 for more than 97% of texels (measured). They were switched to `MASK`
   (cutoff 0.5) and `doubleSided`. Blended foliage needs back-to-front sorting, which causes problems with
   overlapping leaves. Bevy 0.20 also excludes `Blend`, `Premultiplied`, `Add` and `Multiply` materials from the
   depth/normal prepass (`bevy_pbr-0.20.1/src/prepass/mod.rs`). `Mask` works regardless of draw order and is
   included in the prepass. Grass was also made `doubleSided`.
5. **Duplicated embedded textures.** Each MegaKit GLB embedded its own copy of 1–2 MB PNGs; the downloads
   totalled 89 MB. The PNGs were moved to `nature/textures/` and are referenced by relative `uri`. Bevy resolves
   such URIs through the `AssetServer`, so each texture is loaded **once** for all trees. This was verified in
   `bevy_gltf-0.20.1/src/loader/mod.rs`, where `Source::Uri` becomes
   `ImageOrPath::Path { path: gltf_path.resolve_embed_str(uri) }`.

Validation uses `scratchpad/assets-work/gltfcheck`, a small Rust binary built offline against the `gltf = 1.4.1`
crate already in the cargo registry. It parses and validates each file, checks bufferView bounds, checks that every
external image exists, reads every position, index, joint and weight, and lists the clips. **Result: 125/125 OK.**
A NaN/weight-sum scan (`sanity.py`) is clean, and a flat-shaded software render of rest-pose skinned meshes
(`render_check.py`) looks correct (§3.4).

All animated clips are **in place**: the maximum per-joint translation delta during any clip is ≤ 0.05 local units,
so the clips contain no root motion. Locomotion is therefore done by moving the entity and matching stride to speed.

## 3. Source survey

### 3.1 Sources evaluated

| Source | Licence (where stated) | Download access without login or CAPTCHA | Formats | Verdict |
|---|---|---|---|---|
| **Quaternius**: [quaternius.com](https://quaternius.com) packs | **CC0** on every pack page used (each page links to the CC0 deed). See [license.html](https://quaternius.com/license.html) for the new QAL. | The "Just give me the Download" button opens a Google Drive folder (Ultimate Animals: `drive.google.com/drive/folders/1uJ3N5HfB7jKTseJUNQr3N4YaN0UuEtHk`). The folder listing works without login, but `uc?export=download` returned **"Quota exceeded"** on 2026-11-09. itch.io mirrors use a "name your price" flow. | FBX, OBJ, Blend, and glTF for some packs (Ultimate Animals and MegaKit list glTF; Farm, Fish, Easy Enemy list FBX/OBJ/Blend only). | **Primary source.** Best coverage of animated animals. |
| **Poly Pizza**: [poly.pizza/u/Quaternius](https://poly.pizza/u/Quaternius) (1,411 models; 1,395 CC0, 16 CC-BY 3.0) | Per model, from page metadata (`window.__SERVER_APP_STATE__.initialData.model.Licence`) | **Direct GLB at `https://static.poly.pizza/<ResourceID>.glb`** (and a `.jpg` preview). No login needed. **The ToS forbids automated downloading** (§8.1). | GLB produced by Poly Pizza (`FBX2glTF v0.9.7`) | Used as a mirror for the Quaternius packs and for the 2 Sherkiz models. |
| **Kenney** Nature Kit: [kenney.nl/assets/nature-kit](https://kenney.nl/assets/nature-kit) | CC0. The page shows "License: Creative Commons CC0", and the zip contains `License.txt` with CC0. | Clicking "Continue without donating" calls `https://kenney.nl/download-link/<hex id>`, which returns a signed URL. That URL served `kenney_nature-kit.zip` (10,537,521 B, v2.1, 2020-04-29). | GLB (329 in `Models/GLTF format/`), FBX, OBJ, DAE, STL | **Used** for 48 props. No animals; Kenney's 3D animals ("Cube Pets") are blocky. |
| **Khronos glTF-Sample-Assets Fox** | Model CC0 (PixelMannen). Rigging and animation **CC-BY 4.0** (tomkranis). glTF conversion **CC-BY 4.0** (@AsoboStudio, @scurest) ([README](https://github.com/KhronosGroup/glTF-Sample-Assets/tree/main/Models/Fox)). | `https://raw.githubusercontent.com/KhronosGroup/glTF-Sample-Assets/main/Models/Fox/glTF-Binary/Fox.glb` (162,852 B) | GLB: 576 tris, 24 joints, 1 PNG texture, clips `Survey` 3.42 s, `Walk` 0.71 s, `Run` 1.16 s. Native units are cm (155 units long). | **Not installed.** The Quaternius fox is better (12 clips, CC0). It is the same fox Bevy's own examples use (`models/animated/Fox.glb`, credited in Bevy's `CREDITS.md`). |
| **KayKit** Forest Nature Pack: [kaylousberg.itch.io/kaykit-forest](https://kaylousberg.itch.io/kaykit-forest) | The itch page says "Creative Commons Zero v1.0 Universal" (fetched 2026-11-09). | Only through itch.io's "name your own price" purchase flow (free tier 6.1 MB). There is no stable direct URL, and no GitHub mirror was found. | FBX, glTF, OBJ (page). GLB not stated. | **Not downloaded.** A good candidate for a *manual* download (trees, rocks, grass in a matching low-poly style). |
| **OpenGameArt** | [Low poly 3D Pigeon (rigged + animated)](https://opengameart.org/content/low-poly-3d-pigeon-model-rigged-animated-untextured) by mujtaba-io is **CC0**. [Bird - Animated](https://opengameart.org/content/bird-animated) by PantherOne is **CC-BY 3.0** (324 tris, flying loop). | Direct file links work, for example `opengameart.org/sites/default/files/genuinely-my-pigeon-extended-furthur.blend` (798,248 B). | **`.blend` only** | **Not converted.** Converting would mean opening third-party `.blend` files in Blender, which can carry embedded Python. If someone wants these birds, do it by hand with `blender --factory-startup --disable-autoexec` and the glTF exporter (§8.3). |
| **Poly Pizza, other uploaders** | Mostly CC-BY 3.0: "Poly by Google" migrations and community uploads. | Same CDN pattern | GLB | 24 non-Quaternius model pages were opened, chosen from searches for 24 animal terms. Only 2 have `Animated: true` (Sherkiz's *Hawk Lp Rigged* and *Rabbit*), and both were installed. Static CC-BY candidates found: madtrollstudio Bear, Bison, Duck, Deer, Sheep; marioba Morpho Butterfly and Turtle; Crispy_Prawn Long-Necked Turtle. Static CC0: Exceptional_3D Ladybird and MiniPoly Mice (both cartoon-styled). |
| **Sketchfab** (CC0/CC-BY) | Per model | Downloads require login | — | **Skipped**, as instructed. |
| **three.js example birds** (`Parrot.glb`, `Flamingo.glb`, `Stork.glb`) | **Unclear.** Credited to mirada / ro.me. No per-asset licence was found; the repository's MIT licence covers code. **UNVERIFIED.** | — | GLB with morph-target flight | **Excluded** because the licence is unclear. |
| **Poly Haven**: [api.polyhaven.com](https://api.polyhaven.com/assets?t=models) | CC0 | Public JSON API with direct glTF and texture URLs at `dl.polyhaven.org` (for example `.../Models/gltf/1k/rock_moss_set_01/rock_moss_set_01_1k.gltf`) | glTF + 1k–8k textures | **Not installed.** 521 models (110 nature, 57 plants, 37 rocks, 20 trees). The scans are photoreal and heavy (`rock_moss_set_01` has 63,127 polygons), which clashes with the low-poly animals. Candidate for a future "realistic" art mode. |
| **Bevy repository assets** | `CREDITS.md` lists the fox and others. `textures/water_normals.png`, used by `examples/3d/ssr.rs`, is **not listed**, so its licence is **UNVERIFIED**. | — | — | Do not copy `water_normals.png`; generate water normals ourselves (§6). |

### 3.2 Quaternius packs used: provenance and model lists

These are the Poly Pizza bundles, mapped to their quaternius.com pack pages. Every page showed **License CC0**
on 2026-11-09.

| Pack (quaternius.com page) | Poly Pizza bundle | Models present on Poly Pizza |
|---|---|---|
| [Ultimate Animated Animal Pack](https://quaternius.com/packs/ultimateanimatedanimals.html) (12 models, "Animated, Textured", FBX/OBJ/Blend/glTF) | [Animated-Animal-Pack-ILAPXeUYiS](https://poly.pizza/bundle/Animated-Animal-Pack-ILAPXeUYiS) | Alpaca, Bull, Cow, Deer, Donkey, Fox, Horse, White Horse, Husky, Shiba Inu, Stag, Wolf |
| [Farm Animal Pack](https://quaternius.com/packs/farmanimal.html) (7) | [Farm-Animal-Pack-1kUvRTPLzT](https://poly.pizza/bundle/Farm-Animal-Pack-1kUvRTPLzT) | Llama, Pig, Pug, Sheep, Horse, Cow, Zebra |
| [Easy Enemy Pack](https://quaternius.com/packs/easyenemy.html) (5) | individual uploads | Spider, Rat, Wasp, Snake, Frog. Confirmed from the pack preview image. |
| [Animated Monster Pack](https://quaternius.com/packs/animatedmonster.html) (4) | individual upload | Bat ([poly.pizza/m/hNO9XvjlKa](https://poly.pizza/m/hNO9XvjlKa)) |
| [Animated Fish Pack](https://quaternius.com/packs/animatedfish.html) (7) | [Animated-Fish-Bundle-ZkGbjS8m8g](https://poly.pizza/bundle/Animated-Fish-Bundle-ZkGbjS8m8g) | Fish ×3, Dolphin, Shark, Whale, Manta ray |
| [Animated Cute Fish Pack](https://quaternius.com/packs/cutefish.html) (52) | [Animated-Fish-Bundle-44zhHN1UbT](https://poly.pizza/bundle/Animated-Fish-Bundle-44zhHN1UbT) | 33 fish plus fishing props. **Not installed** (see below). |
| [Stylized Nature MegaKit](https://quaternius.com/packs/stylizednaturemegakit.html) ("110+ unique nature models") | [Stylized-Nature-MegaKit-T34GZFA0fm](https://poly.pizza/bundle/Stylized-Nature-MegaKit-T34GZFA0fm) (68 models) | Tree ×5, Pine ×5, Twisted Tree ×5, Dead Tree ×5, Bush, Bush with Flowers, Clover ×2, Fern, Flower Group ×2, Flower Single ×2, Flower Petal ×5, Grass, Grass Wispy ×2, Tall Grass, Mushroom, Mushroom Laetiporus, Plant ×2, Plant Big ×2, Pebble Round ×5, Pebble Square ×5, Rock Medium ×3, Rock Path ×10 |

Per-file Poly Pizza page URLs and direct GLB URLs for all 74 Quaternius files are listed in
`assets/models/quaternius/ATTRIBUTION.txt`.

**Animated Cute Fish Pack (CC0).** It has richer clips: `Fish_Armature|Attack`, `Death`, `Out_Of_Water`,
`Swimming_Fast`, `Swimming_Impulse` and `Swimming_Normal`. Inspected species: Koi, Tuna, Goldfish, Piranha, Red
Snapper and Shark, each with 1.3–1.7k tris, a 6-joint skeleton and 140–190 KB per file. They are *chibi* (big eyes,
round bodies), which clashes with the semi-realistic animals, so they were left out. They are the easiest upgrade if
fish need attack and death clips. The Poly Pizza page ids are in the bundle above.

### 3.3 Rejected candidates and reasons

* **Quaternius "Cube World" animals** (Cat, Chicken, Raccoon, Hedgehog: blocky voxel style) and **"blob"
  characters** (Pigeon, Chicken, Cat "Blob" meshes with 4-joint rigs). They are not recognisable as animals in a
  naturalistic scene, which breaks the "no cubes or spheres" requirement.
* **Anthropomorphic characters**: "Bunny" holding a carrot (Punch, Wave and Weapon clips), the upright "Frog"
  character, and the Panda character. The art style does not fit.
* **Crab Enemy and Bee Enemy**: cartoon monsters with big mouths and teeth.
* **Farm Cow, Horse, Llama and Pug**: duplicates of better Ultimate-pack models (fewer clips, lower poly).
* **Google Poly (CC-BY 3.0) static animals**: not animated. They remain possible as static far-LOD or "carcass"
  meshes, but each one adds an attribution line.

### 3.4 Visual check

Previews from `static.poly.pizza/<uuid>.jpg` were checked on contact sheets. A software render of our processed
files in their skinned rest pose, at a three-quarter view, confirmed these look right and that the conversion is
intact: fox, stag, sheep, wasp (after the weight fix), spider, shark, hawk and rabbit. The hawk renders white in that
check only because the checker ignores textures. It is not shipped in the repository; the scripts and sheets are in
`scratchpad/assets-work/` (`sheet1..4.jpg`, `render1.jpg`).

## 4. Installed animal assets (measured)

All clip names were obtained by parsing each GLB's JSON chunk (`glbinfo.py`). Clips shown without their armature
prefix: the GLB stores, for example, `RatArmature|Rat_Walk` and `Armature|Swim`. The manifest uses the full names.

| File | Name | Licence | Tris | KB | Clips | Supported actions | Missing (land/air) |
|---|---|---|---:|---:|---|---|---|
| `quaternius/animals/fox.glb` | Red fox | CC0 | 1,848 | 557 | 12: Attack, Death, Eating, Gallop, Gallop_Jump, Idle_HitReact_Left/Right, Jump_ToIdle, Walk, Idle_2_HeadLow, Idle_2, Idle | idle walk run eat attack death jump rest | — |
| `quaternius/animals/wolf.glb` | Grey wolf | CC0 | 1,962 | 575 | same 12 as fox | idle walk run eat attack death jump rest | — |
| `quaternius/animals/wild_dog_husky.glb` | Husky (wild dog / wolf variant) | CC0 | 1,920 | 545 | same 12 | idle walk run eat attack death jump rest | — |
| `quaternius/animals/wild_dog_shiba.glb` | Shiba Inu (dingo proxy) | CC0 | 1,950 | 511 | same 12 | idle walk run eat attack death jump rest | — |
| `quaternius/animals/deer.glb` | Deer (doe) | CC0 | 2,096 | 629 | 13: Attack_Headbutt, Attack_Kick, Death, Eating, Gallop, Gallop_Jump, Idle_HitReact_Left/Right, Jump_toIdle, Walk, Idle_Headlow, Idle_2, Idle | idle walk run eat attack death jump rest | — |
| `quaternius/animals/stag.glb` | Red deer stag (antlers = separate mesh `Stag_Horns`) | CC0 | 3,667 | 653 | same 13 | same | — |
| `quaternius/animals/horse.glb`, `horse_white.glb` | Horse | CC0 | 2,182 | 646 | same 13 | same | — |
| `quaternius/animals/donkey.glb` | Donkey / wild ass | CC0 | 2,000 | 629 | same 13 | same | — |
| `quaternius/animals/cow.glb` | Cow (aurochs proxy) | CC0 | 2,450 | 632 | same 13 | same | — |
| `quaternius/animals/bull.glb` | Bull (bison/buffalo proxy) | CC0 | 2,418 | 635 | same 13 | same | — |
| `quaternius/animals/alpaca.glb` | Alpaca / llama / guanaco | CC0 | 2,060 | 615 | same 13 | same | — |
| `quaternius/animals/sheep.glb` | Sheep | CC0 | 610 | 150 | 2: Idle, Jump | idle jump | walk run eat attack death rest |
| `quaternius/animals/pig.glb` | Pig / wild boar proxy | CC0 | 562 | 152 | 2: Idle, Jump | idle jump | walk run eat attack death rest |
| `quaternius/animals/zebra.glb` | Zebra | CC0 | 1,354 | 367 | 6: Death, Idle, Jump, Run, Walk, WalkSlow | idle walk run death jump | eat attack rest |
| `quaternius/critters/rat.glb` | Rat / mouse | CC0 | 4,004 | 529 | 6: Rat_Attack, Rat_Death, Rat_Idle, Rat_Jump, Rat_Run, Rat_Walk | idle walk run attack death jump | eat rest |
| `quaternius/critters/spider.glb` | Spider | CC0 | 2,712 | 429 | 5: Spider_Attack, Spider_Death, Spider_Idle, Spider_Jump, Spider_Walk | idle walk run(=walk×2) attack death jump | eat rest |
| `quaternius/critters/wasp.glb` | Wasp (bee proxy) | CC0 | 3,734 | 426 | 3: Wasp_Attack, Wasp_Death, Wasp_Flying | idle(hover) fly attack death | eat jump rest (land/walk) |
| `quaternius/critters/snake.glb` | Snake | CC0 | 1,618 | 207 | 4: Snake_Attack, Snake_Idle, Snake_Jump, Snake_Walk | idle walk run attack jump | eat death rest |
| `quaternius/critters/frog.glb` | Tree frog | CC0 | 4,920 | 568 | 4: Frog_Attack, Frog_Death, Frog_Idle, Frog_Jump | idle walk/run/swim(=hop) attack death jump | eat rest |
| `quaternius/critters/bat.glb` | Bat | CC0 | 1,046 | 220 | 5: Bat_Attack, Bat_Attack2, Bat_Death, Bat_Flying, Bat_Hit | idle(hover) fly attack death | eat jump rest (roost) |
| `quaternius/fish/fish_perch.glb`, `fish_clown.glb`, `fish_bream.glb`, `shark.glb`, `dolphin.glb`, `whale.glb`, `manta_ray.glb` | Fish ×3, shark, dolphin, whale, manta | CC0 | 440–696 | 57–98 | 1: `Swim` (`Swim.001` for bream) | swim, idle/walk/run (same clip ×0.4/0.8/1.8) | eat attack death |
| `polypizza-sherkiz/hawk.glb` | Hawk (eagle/raptor proxy) | **CC-BY 3.0** | 9,956 | 845 | 1: `metarig\|Fly` (1.29 s) | fly, idle(glide = paused clip) | eat attack death perch/walk |
| `polypizza-sherkiz/rabbit.glb` | Rabbit / hare | **CC-BY 3.0** | 1,560 | 221 | 2: `Bunny\|Bunny_idle`, `Bunny\|Bunny_walk` | idle walk run(=walk×2.2) eat/rest(=idle) | attack death jump |

Skeleton sizes:
* Ultimate-pack animals: 38–51 joints.
* Farm-pack animals: 24–28 joints. The sheep and pig rigs have 24 joints, so leg bones exist and the procedural gait (§7.3) can drive them.
* Rat 31, spider 39, wasp 29, snake 15, frog 28, bat 23.
* Fish 6–13.
* Hawk 58 (including feather bones), rabbit 45.

## 5. Coverage matrix: ecosystem species to asset

There is no species catalogue in the repository yet: `assets/species/` (RON, planned in `ARCHITECTURE.md`) does not
exist. The species list below is the planned ecosystem roster, inferred from the product brief: animals, plants,
fungi, fire, water and climate.

Legend:
* **A** = authored animated GLB covers it.
* **A~** = covered by a proxy model plus recolouring.
* **P** = partial clip set; the procedural fallback fills the rest.
* **S** = only static meshes exist (CC-BY Poly Pizza, not installed).
* **—** = not covered, so procedural.

| Group | Species (ecosystem id) | Asset (`manifest id`) | Status | Missing actions → fallback |
|---|---|---|---|---|
| Herbivore, large | deer | `deer` | A | swim → procedural |
| | deer male / elk | `stag` (elk = recolour + scale 1.3) | A / A~ | antler drop: hide `Stag_Horns` mesh |
| | horse / wild horse | `horse`, `horse_white` | A | — |
| | zebra | `zebra` | P | eat, attack, rest → neck-bend overlay, kick reuse |
| | donkey / wild ass | `donkey` | A | — |
| | cattle / aurochs | `cow` | A | — |
| | bison / buffalo | `bull` | A~ | no hump: accept, or use a procedural mesh |
| | llama / alpaca / guanaco | `alpaca` | A | — |
| | sheep / mountain goat | `sheep` | P | walk, run, eat, death, rest → procedural leg gait on 24-joint rig |
| | moose, giraffe, elephant, camel | — | — | procedural body plan |
| Herbivore, small | rabbit / hare | `rabbit` (CC-BY) | P | attack, death, jump → procedural |
| | mouse / rat / vole | `rat` | A | eat → idle with head bob |
| | squirrel, beaver, hedgehog | — | — | procedural (the Quaternius hedgehog is Cube World style) |
| Omnivore | wild boar / pig | `pig` (dark tint) | P | as sheep |
| | bear | — (S: madtrollstudio "Bear", CC-BY, 946 tris, static) | — | procedural quadruped |
| | raccoon | — (Quaternius raccoon is Cube World) | — | procedural |
| Carnivore | fox | `fox` | A | — |
| | wolf | `wolf` (`wild_dog_husky` as variant) | A | — |
| | wild dog / dingo / jackal | `wild_dog_shiba`, `wild_dog_husky` | A~ | — |
| | big cat (lynx, puma, lion) | — | — | procedural quadruped (derive from canine rig proportions) |
| Birds | raptor (hawk, eagle) | `hawk` (CC-BY) | P | glide = paused Fly; perch, eat, death → procedural |
| | songbird, sparrow, finch | — (S: several Poly-by-Google CC-BY) | — | **procedural mesh + vertex-shader flap**, boids |
| | duck / waterfowl | — (S: madtrollstudio Duck CC-BY) | — | procedural; swim = float + paddle bob |
| | pigeon, crow | — (OGA CC0 pigeon is .blend only) | — | procedural, or manual Blender conversion (§8.3) |
| | chicken / ground bird | — (only blob or Cube World) | — | procedural |
| Bats | bat | `bat` | A | roost → hang pose (procedural) |
| Fish | small fish (trout, perch) | `fish_perch` | P | death → belly-up + sink |
| | medium fish (carp, bream) | `fish_bream` | P | as above |
| | reef fish | `fish_clown` | P | as above |
| | shark / predatory fish | `shark` | P | attack = fast swim + lunge (procedural) |
| | dolphin, whale, ray | `dolphin`, `whale`, `manta_ray` | P | as above |
| Amphibians and reptiles | frog / toad | `frog` | A | — |
| | snake | `snake` | P | death → straighten + roll |
| | turtle, lizard | — (S: marioba Turtle, Crispy_Prawn Long-necked turtle, both CC-BY) | — | procedural |
| Invertebrates | wasp | `wasp` | A | landing / walk → procedural |
| | bee | `wasp` recoloured (A~) | A~ | — |
| | butterfly / moth | — (S: marioba Morpho Butterfly CC-BY, static) | — | **procedural 2-quad wings + vertex shader** |
| | ant, beetle, ladybird | — (S: Exceptional_3D Ladybird CC0 cartoon) | — | procedural instanced, 6-leg tripod gait |
| | spider | `spider` | A | — |
| | worm / earthworm | — (Quaternius bait "Worm", CC0, 240 tris, static, not installed) | — | procedural tube + sine wave |
| Trees | broadleaf (oak, beech) | `tree_broadleaf_[a-e]` (LOD `kenney_tree_oak`) | static | wind sway shader; growth = scale |
| | conifer | `tree_pine_[a-e]` (LOD `kenney_tree_pineTallA`) | static | — |
| | autumn / old tree | `tree_twisted_[a-e]`, `kenney_tree_fat_fall` | static | seasonal tint |
| | dead / burnt tree | `tree_dead_[a-e]` | static | burnt = darken material |
| | palm | `kenney_palm_*` | static | — |
| Shrubs and herbs | shrub, berry bush | `bush`, `bush_flowering`, `kenney_plant_bush` | static | — |
| | grass, tall grass, reeds | `grass*`, `grass_wispy_*`, `grass_tall`, `kenney_grass*` | static | GPU-instanced; wind |
| | flowers | `flower_group_*`, `flower_single_*` | static | — |
| | fern, clover, herbs | `fern`, `clover_*`, `plant_small_*`, `plant_big_*` | static | — |
| | cactus / succulent | `kenney_cactus_*` | static | — |
| | crops / wild grain (growth stages) | `kenney_crop_corn_[a-d]`, `kenney_crop_wheat_[ab]`, `kenney_crop_bamboo_[ab]`, `kenney_crop_leafy_[ab]` | static | stage = model swap |
| | water lily | `kenney_lily_large/small` | static | — |
| Fungi | mushrooms | `mushroom_cluster`, `kenney_mushroom_tan*` | static | — |
| | toadstool (poisonous) | `kenney_mushroom_red*` | static | — |
| | bracket fungus / decomposer | `fungus_bracket` | static | attach to logs and dead trees |
| | mycelium network | — | — | procedural decal or line overlay |
| Dead wood | stumps, logs | `kenney_stump_*`, `kenney_log*` | static | fire aftermath |
| Rocks | boulders, spires, pebbles | `rock_medium_[a-c]`, `pebble_*`, `kenney_stone_*`, `kenney_rock_large*` | static | — |
| Water | lakes, rivers, sea | — (no mesh; see §6) | — | shader water |
| Fire | fire, smoke, embers | — | — | particles (out of scope for meshes) |

## 6. Water, fire and terrain

There are no mesh assets for these, by design.

* **Water.** Bevy 0.20.1 `examples/3d/ssr.rs` shows a water surface built as an `ExtendedMaterial` with a
  `water_material.wesl` shader that scrolls normal maps, combined with screen-space reflections. The normal-map
  texture in that example (`textures/water_normals.png`) **has no licence entry in Bevy's CREDITS.md**
  (**UNVERIFIED**). Generate our own tileable normal map offline with the `noise` 0.9 crate (fBm/Perlin → height →
  Sobel → normals) and ship it under our own licence. Lily pads (`kenney_lily_*`), reeds (`grass_tall`) and fish
  give the water life.
* **Fire.** Use particles plus tinting burnt meshes (`tree_dead_*`, `kenney_stump_*`) instead of dedicated models.
* **Terrain.** Generate it procedurally. Kenney cliff and waterfall tiles exist in the zip but were not installed.

## 7. Procedural animals in Rust, and the recommended hybrid strategy

### 7.1 What it takes to make procedural animals recognisable

From the perception literature and shipped games:

* **Silhouette.** Species identity at game distances comes mainly from proportions: leg length relative to body,
  neck length, head/snout shape, ear shape and size, tail shape (bushy, thin, fan, fluke), and appendages such as
  antlers, horns or wings. The mesh generator therefore needs per-species parameters for: spine curve, a
  cross-section radius profile along the spine, head primitive with snout length and ear template, limb count with
  segment lengths and thickness, and tail type.
* **Colour pattern.** Signature palettes carry a lot of the recognition (orange fox with black "socks" and a white
  tail tip, zebra stripes, countershading). Vertex colours or a small palette texture driven by procedural masks
  (`noise` crate) are enough in a flat-shaded style.
* **Motion.** Gait is itself an identity cue. Point-light studies show that people recognise biological motion from
  movement alone (G. Johansson, "Visual perception of biological motion and a model for its analysis",
  *Perception & Psychophysics* 14(2), 1973). Examples: the rabbit's half-bound hop, the deer's long stride and
  pronking, the snake's lateral undulation, the fish's body wave, the flapping-versus-soaring split for birds.
* **Prior art for procedural animation on arbitrary bodies:**
  * Spore's creature animation retargeting. C. Hecker et al., "Real-time motion retargeting to highly varied
    user-created morphologies", *ACM Trans. Graph.* 27(3), SIGGRAPH 2008.
  * D. Rosen, "An Indie Approach to Procedural Animation", GDC 2014 (Overgrowth).

  Both show that **IK-driven procedural gaits on a generated skeleton look convincing when the body plan is right**.

**Engineering components and estimated effort.** These estimates are for one engineer familiar with Bevy and are
**UNVERIFIED**.

| Component | Technique | Effort |
|---|---|---|
| Body mesh | Loft rings along the spine with rotation-minimising frames, using superellipse cross-sections. Alternative: an SDF union of capsules and ellipsoids meshed with surface nets or dual contouring, which gives smooth blends at limb joints. Faceted normals give the low-poly look. | 1–2 weeks |
| Limbs, wings, fins | Tapered lofted tubes (legs). Membrane or feather quads (wings). Planar fan meshes (fins, tail flukes). | 1 week |
| Skinning | Bones generated along spine and limbs. Weights from distance to bone segments with smooth falloff. Bevy `SkinnedMesh` + `SkinnedMeshInverseBindposes` + `Mesh::ATTRIBUTE_JOINT_INDEX/WEIGHT` (pattern in `bevy-0.20.1/examples/animation/custom_skinned_mesh.rs`) with `DynamicSkinnedMeshBounds` for culling. | 1 week |
| Gait | Per-leg phase offsets (walk: lateral sequence; trot: diagonal pairs; gallop: rotary or transverse; bound: front and back pairs; hexapod: alternating tripod). Foot trajectories: stance follows the ground, swing is an arc. Analytic 2-bone IK. Body bob and pitch from foot heights. Head stabilisation. | 2 weeks |
| Undulation | Fish and snakes: travelling wave θ(s,t) = A(s)·sin(k·s − ω·t) applied to spine bones, or in the vertex shader for instanced schools. | 2–3 days |
| Flight | Shoulder rotation with an asymmetric down- and upstroke. Soaring = fixed spread pose with banking. For small birds and insects: vertex-shader flap keyed on a per-vertex wing mask. | 3–5 days |
| Death, eat, rest | Pose blends: tip over or settle with a damped spring on the root; neck bend; lie down = fold legs by IK to the ground plane. | 3–5 days |
| Per-species tuning | Parameter sets and palette, checked against references. | 0.5–1 day per species |

**Assessment.**
* Procedural animals can be made reliably recognisable for **body plans with strong silhouettes**: snakes, fish,
  worms, insects, spiders, small birds in flight, rabbits, and generic quadrupeds such as bear or cat if ears, tail
  and palette are tuned.
* They are weaker than authored models for **close-up hero shots of mammals** (faces, fur volume) and for nuanced
  clips (eating, fighting).
* Their decisive advantages here:
  * unlimited morphological variation, which the evolution lab needs because a genome maps directly to body-plan
    parameters
  * no licensing exposure
  * cheap GPU instancing when skinning is replaced by vertex-shader animation
  * consistent art direction

### 7.2 Recommended hybrid

1. **Authored GLBs** (installed set) for the ~25 common, recognisable species: ungulates, canids, rat, frog, snake,
   spider, wasp/bee, fish, hawk, rabbit.
   * Play the clips through `AnimationGraph` and cross-fade with `AnimationTransitions`.
   * Bevy 0.20.1 API checked in the registry sources:
     * `WorldAssetRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(path)))`
     * an observer on `On<WorldInstanceReady>`. Both types live in `bevy_world_serialization` 0.20.1. Earlier
       Bevy releases called them `SceneRoot` / `SceneInstanceReady`; no type named `SceneRoot` exists in the
       0.20.1 sources.
     * `AnimationGraph::from_clip(s)`, `AnimationGraphHandle`, `AnimationPlayer::play(i).repeat()`
     * `AnimationTransitions::play(&mut player, node, Duration)`
     * `Gltf::named_animations: HashMap<Box<str>, Handle<AnimationClip>>`

     Files: `examples/animation/animated_mesh.rs`, `animated_mesh_control.rs`, `animation_masks.rs`;
     `bevy_gltf-0.20.1/src/assets.rs`.
   * Resolve `animation_set` names from the manifest **once per model type** and build one `AnimationGraph` per
     model type, shared by every instance.
2. **Procedural bone overlay** on authored rigs to fill missing actions (§7.3). Write joint `Transform`s after the
   animation system and before transform propagation, or use `AnimationMask` groups (`animation_masks.rs`) so
   procedural legs and authored head/neck can coexist.
3. **Procedural instanced creatures** for small and numerous species: songbirds, butterflies, bees in swarms, ants,
   beetles, worms, fish schools beyond about 50 m.
   * Static mesh plus vertex-shader animation in one instanced draw (flap, undulation, leg-wiggle masks stored in
     vertex colours).
   * Boids or flow fields from the simulation drive the positions.
4. **Genome-driven procedural meshes** for evolution-lab and custom-life organisms, in the same flat-shaded style
   and with the same gait system as item 3. This is the "procedural organism and plant meshes" path already
   foreseen in `ARCHITECTURE.md` for the `rendering` crate.
5. **LOD and crowd scaling.**
   * On native backends Bevy 0.20 can batch skinned meshes. Batching is turned off only when skins use uniform
     buffers, as on WebGL 2 (`no_automatic_skin_batching` in `bevy_pbr-0.20.1/src/render/skin.rs`).
   * The cost that grows with the number of animals is on the CPU. Each animal is one `AnimationPlayer` plus
     38–58 joint entities, all evaluated and transform-propagated every frame.
   * Plan three tiers:
     * near: skinned GLB, full clips
     * mid (beyond about 40 m): baked **vertex-animation textures** (VAT) of the main loops (idle, walk, run) on an
       instanced static mesh with phase offsets
     * far: impostor billboard or a coloured dot from `density_grid`
   * Bake VATs offline with a small Rust tool that samples the clips.
   * For flora, `lod_variants` already points at the Kenney proxies; add `meshopt`-style simplification later if
     needed (**UNVERIFIED**: no crate was evaluated).
6. **Plants.** Authored GLBs plus a wind-sway vertex shader (height-weighted). Growth is a scale tween; crops use
   stage swaps. Procedural L-system trees are reserved for evolved plant species.

### 7.3 Procedural fallbacks for missing clips on authored rigs

| Missing action | Fallback |
|---|---|
| walk / run (sheep, pig) | Find leg chains by joint-name heuristics (the Farm rig has 24 joints). Oscillate hip/shoulder and knee rotations with phase offsets (walk 0, 0.5, 0.25, 0.75) and add a body bob. Speed comes from stride length. |
| eat (zebra, rat, rabbit, fish) | Blend a neck/head pitch-down pose over Idle (additive), plus a small jaw/head bob at 2–3 Hz. |
| rest | Sit or lie: lower the root by about 40% of leg length and fold legs (IK to the ground); breathing via spine scale at 0.3 Hz. Ungulates already have `Idle_Headlow`. |
| death (fish, snake, hawk) | Fish: roll 180° and sink with drag. Snake: freeze the last frame and straighten the spine bones. Hawk: tumble with gravity. Then fade or decay to a "carcass" state for the detritus cycle. |
| swim (land mammals) | Lower the root to the waterline, play `Walk` at ×0.6, and add bow-wave particles. |
| fly idle / glide (hawk) | Pause `metarig\|Fly` at the wings-spread frame (anim_speed 0), bank by yaw rate. |
| attack (fish, shark) | Fast swim (×2.5) plus a lunge root offset and a jaw-open bone if present. |

### 7.4 Integration notes for Bevy 0.20.1

* **Material override at load** for all models, including the byte-identical CC-BY ones. Iterate the scene's
  `MeshMaterial3d<StandardMaterial>` and set `metallic = 0.0` and `perceptual_roughness = max(r, 0.6)`, in one
  observer on `WorldInstanceReady`. For flora, `AlphaMode::Mask(0.5)` is already in the files.
  `AlphaMode::AlphaToCoverage` with MSAA gives softer leaf edges.
* **Orientation.** Apply `yaw_offset_deg` and `scale` on a child "model root" entity, not on the simulation-driven
  entity. If you steer with Bevy's `Transform::forward()` (−Z), add 180°.
* **Unsupported glTF extensions** in `bevy_gltf` 0.20.1 (support table in `src/lib.rs`): Draco, meshopt,
  `KHR_mesh_quantization`, `EXT_mesh_gpu_instancing`, `KHR_texture_basisu`/`EXT_texture_webp` syntax. None of the
  installed files use any extension: `extensionsUsed` is empty, except that Kenney files declare
  `KHR_materials_unlit` without using it. **Do not run** compression tools that emit these extensions on our assets.
* **Texture memory.** The 15 MegaKit PNGs are 1024² RGBA (`Flowers.png` is 1024×997). Bark textures carry an unused,
  fully opaque alpha channel. Optional later step: convert to KTX2/BasisU **files** with `uri` references, since
  Bevy supports the KTX2 format with a feature flag but not the `KHR_texture_basisu` syntax.
* **Hawk tri count** (9,956) is about 5 times that of the other animals. Use it only for near-LOD, and use the
  procedural bird for flocks.

## 8. Risks and open items

### 8.1 Poly Pizza Terms of Service

[poly.pizza/docs/tos](https://poly.pizza/docs/tos) forbids scraping and "automated means, including spiders,
robots, crawlers, data mining tools, or the like to download data from the Services". Its TL;DR targets AI-training
scrapers. Our retrieval was scripted with `curl`, low volume, and did not bypass any protection. The CC0 and CC-BY
licences of the files are unaffected, because the ToS is a contract term between the downloader and Poly Pizza and
does not change the content licence.

Even so, **before release, a human should re-obtain the packs from the official sources.** Options:
* quaternius.com download buttons (Google Drive or itch.io) through a browser
* Poly Pizza's official API with a key ([docs/api/v1.1](https://poly.pizza/docs/api/v1.1); not tested)
* the download buttons on the Poly Pizza model pages

The official Quaternius downloads also include `.blend` and native glTF exports. These probably do not have the
FBX2glTF artefacts (duplicate takes, metallic 0.4), and Farm Animals may contain more clips (Poly Pizza's sheep and
pig contain only Idle and Jump; **UNVERIFIED** whether the original FBX has more).

### 8.2 Licence drift at Quaternius

The site-wide **QAL v1.0** (last updated 2026-08-28) forbids redistributing assets as assets, even for free. A public
repository containing raw GLBs could be argued to be such redistribution. Every pack used here is labelled **CC0** on
its own page. CC0 1.0 §2 is an irrevocable waiver, and QAL §7 says changes do not apply retroactively. Even so:
* **check each new pack's own page** before adding it
* keep `quaternius/LICENSE.txt` with the evidence and date
* never add a pack labelled QAL to `assets/models/`

### 8.3 Other risks

* **CC-BY provenance.** The Sherkiz account (hawk, rabbit) has no linked socials, so authorship is **UNVERIFIED**.
  If someone else's work had been re-uploaded, the licence would be void. Mitigation: they are isolated in their own
  folder and easy to drop. Replace them when a CC0 alternative appears, for example the OGA CC0 pigeon converted by
  hand.
* **Converting third-party `.blend` files.** A `.blend` can embed Python (drivers, scripts). If a human converts
  OGA birds, use `Blender --factory-startup --disable-autoexec -b file.blend --python-expr "<export only>"` on a
  machine where that is acceptable. This was not done here.
* **Mixed art styles.** Kenney props are flat-coloured and untextured, while the MegaKit is textured. Use Kenney
  mainly as far-LOD proxies and for categories the MegaKit lacks: palms, cacti, lilies, crops, stumps, logs.
* **Scale values are judgement calls.** They come from real-world reference sizes in `build_assets.py`, for example
  fox 1.0 m nose-to-tail tip, horse 2.4 m, whale 12 m. Tune them in game.
* **Tiny creatures** need display exaggeration or they will be sub-pixel. Wasp: 3 cm, spider: 10 cm leg span.
* **No LODs from sources.** Skinned crowd performance must be benchmarked before committing to the authored-GLB-only
  path for herds of more than 200 animals.

### 8.4 Not done (deliberately)

* No executables or downloaded code were run.
* No login, CAPTCHA or bot-check was bypassed.
* No NC, ND or unclear-licence asset was included.
* Google Drive links that returned "Quota exceeded" were not retried through workarounds.

## 9. Reproduction

The scratchpad scripts, all our own code, are in
`/private/tmp/claude-501/-Users-alejandro-Projects-Conway-GameOfLife/d26793ef-30fe-4239-b1fb-77f42d831735/scratchpad/assets-work/`:

| Script | Purpose |
|---|---|
| `glbinfo.py` | Lists meshes, triangles, skins, materials, images, extensions and animations with durations. |
| `glbtool.py` | GLB rewriter: duplicate-take removal, NaN-weight repair, material fixes, texture externalisation, buffer garbage collection. `--bounds` gives the skinned rest-pose AABB. |
| `build_assets.py` | The selection table, target sizes and action maps. Generates `assets/models/**` and `manifest.toml`. Run with `python3 -I build_assets.py <repo>`. |
| `gltfcheck/` | Rust validator built offline against `gltf = 1.4.1`. |
| `sanity.py`, `facing.py`, `render_check.py`, `contact.py` | Data checks, forward-axis detection and visual checks. |

Raw downloads (untrusted data, each in its own directory) are in `scratchpad/downloads/assets3d/`:
`polypizza_quaternius/`, `polypizza_quaternius_megakit/`, `polypizza_other/`, `kenney_nature_kit/`, `khronos_fox/`,
`licenses/` (CC0 and CC-BY 3.0 legal code from creativecommons.org, Quaternius license page) and `previews/`.

## 10. Sources

* Quaternius pack pages and licence:
  * https://quaternius.com/packs/ultimateanimatedanimals.html
  * https://quaternius.com/packs/farmanimal.html
  * https://quaternius.com/packs/easyenemy.html
  * https://quaternius.com/packs/animatedmonster.html
  * https://quaternius.com/packs/animatedfish.html
  * https://quaternius.com/packs/cutefish.html
  * https://quaternius.com/packs/stylizednaturemegakit.html
  * https://quaternius.com/license.html
* Poly Pizza:
  * https://poly.pizza/u/Quaternius
  * bundles linked in §3.2
  * https://poly.pizza/m/RkN6MEbP6g
  * https://poly.pizza/m/Km89u3twcR
  * https://poly.pizza/docs/tos
* Kenney: https://kenney.nl/assets/nature-kit
* Khronos Fox: https://github.com/KhronosGroup/glTF-Sample-Assets/tree/main/Models/Fox
* KayKit Forest: https://kaylousberg.itch.io/kaykit-forest
* OpenGameArt:
  * https://opengameart.org/content/low-poly-3d-pigeon-model-rigged-animated-untextured
  * https://opengameart.org/content/bird-animated
* Poly Haven API: https://api.polyhaven.com/assets?t=models
* Creative Commons legal code:
  * https://creativecommons.org/publicdomain/zero/1.0/legalcode.txt
  * https://creativecommons.org/licenses/by/3.0/legalcode.txt
* Bevy 0.20.1 and bevy_gltf 0.20.1 sources (local cargo registry):
  * `examples/animation/*.rs`
  * `examples/3d/ssr.rs`
  * `CREDITS.md`
  * `bevy_gltf/src/lib.rs` (extension table)
  * `bevy_gltf/src/assets.rs`
  * `bevy_gltf/src/loader/mod.rs`
* G. Johansson (1973), *Perception & Psychophysics* 14(2):201–211.
* C. Hecker et al. (2008), "Real-time motion retargeting to highly varied user-created morphologies", ACM TOG 27(3).
* D. Rosen (2014), "An Indie Approach to Procedural Animation", GDC.
