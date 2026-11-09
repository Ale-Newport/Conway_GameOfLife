# OTCA metapixel pattern data: provenance and licensing

The **OTCA metapixel** (Outer Totalistic Cellular Automata Meta-Pixel, "OTCAMP")
was designed and built by **Brice Due** between autumn 2005 and spring 2006.
These files are third-party **data**. They are not code and are not covered by
the project's `MIT OR Apache-2.0` license. Keep this file next to the data.

Retrieved 2026-11-09. See `docs/research/otca-metapixel.md` for the full
specification, the coordinates, and the validation runs.

## Files

| File | What it is | Source | License |
|---|---|---|---|
| `OTCAmetapixel-OFF.rle` | Brice Due's original **OFF** base tile, **unprogrammed**: all 18 rule-table eaters are broken (rule `B/S`), and all 8 neighbourhood fuses are enabled (Moore). It is 2058×2058 with 23 920 live cells. | `http://www.cranemtn.com/life/weblog/OTCAmetapixel-OFF.rle.gz`, Brice Due's own site, linked from the 2006 posts. The original `.gz` has sha256 `e3584f12e38804aae020857308d8a325ba480fc1db0d998539b80ec242de5acf`. | See "Licensing" below. |
| `OTCAmetapixel-ON.rle` | Brice Due's original **ON** base tile, unprogrammed. It is 2058×2058 with 66 203 live cells. | `http://www.cranemtn.com/life/weblog/OTCAmetapixel-ON.rle.gz`. The original `.gz` has sha256 `f8375ab7673ad9549f5c0e353a3f502cd979d4f041235b375198ee1b16314285`. | See below. |
| `OTCAmetapixel-OFF-B3S23.rle` | The same OFF tile **programmed for B3/S23**. It adds 3 cells, which complete the B3, S2 and S3 eaters. It has 23 923 cells. | Derived by us. It is cell-for-cell identical to the B3/S23 tiles in Golly's `metapixel-galaxy.mc` once trimmed to 2048×2048 (corner tubs and corner blocks aside). | See below. |
| `OTCAmetapixel-ON-B3S23.rle` | The same ON tile programmed for B3/S23. It has 66 206 cells. | Derived as above. | See below. |
| `metapixel-galaxy.mc.gz` | Golly demo: a 15×15 metapixel tiling of Kok's galaxy (p8), rule B3/S23, Moore neighbourhood. It was produced by Brice Due's `tile-otcamp.pl` on 2006-06-08. | Golly source tree `Patterns/HashLife/Metacell/metapixel-galaxy.mc.gz`, git `9a7079a84a24f4b5ef16b1c940660d75656e51be` (https://sourceforge.net/projects/golly). The original has sha256 `b6f1c7aef5a8ec5d99a8c621c0dd5cb7c57da42af0b348874779bd63fe958f96`. | GPL-2.0-or-later (the Golly distribution) |
| `metapixel-p216-gun.mc.gz` | Golly demo: David Eppstein's p216 gun in **B35/S236**, as a 142×119 metapixel tiling (4 512 tiles, sparse). | Same tree, `metapixel-p216-gun.mc.gz`. The original has sha256 `0b7db171499b240f51bac0feb3f306a156471d50cd355128ef151da24d4fb6a7`. | GPL-2.0-or-later |
| `metapixel-parity64.mc.gz` | Golly demo: the parity rule **B135/S135** with the **von Neumann** neighbourhood, as a 64×64 tiling. The corner fuses are disabled by removing the corner LWSSs. | Same tree, `metapixel-parity64.mc.gz`. The original has sha256 `cc2a6f03359f3e2c9c7f2d9ae46a94a656c95682fe32dbbbd4e4559d3d39da1f`. | GPL-2.0-or-later |

There is one more file, `otca-spec.json`. It is not third-party data. We wrote
it as a machine-readable version of the programming coordinates, the readout
parameters and the golden test values from our own bgolly validation runs, and
it falls under the project license. It holds only coordinates and numbers taken
from the data above.

Edits we made:

- **Each file has gained one `#C Provenance: ...` comment line.** In the RLE
  files it sits immediately before the `x = ...` line. In the `.mc.gz` files it
  sits after the last original header comment. Every original header line is
  kept verbatim.
- **The `-B3S23` files have also been re-encoded.** Their RLE body was
  re-encoded after the 3 programming cells were added. Extra `#C` lines
  document the change.
- **No other cell has been modified.** Every file was re-read after writing. The
  cell sets match the sources exactly, and the `.mc` files load in `bgolly` with
  the same bounding box and population as the originals.

## Coordinate convention

The tile RLE files have no `#CXRLE Pos` line, so a loader places the top-left of
the 2058×2058 bounding box at (0,0).

- **The nominal 2048×2048 tile starts at file coordinate (5,5).** Subtract 5 to
  get *tile-nominal* coordinates, in which the tile occupies `[0,2047]²`. This
  is the convention of Golly's `metafier.lua`/`metafier.py`, which load these
  tiles with an offset of (-5,-5).
- **Tiles are placed with OR at pitch 2048.** Neighbouring tiles overlap by
  5 cells on every side. In the overlap, the corner tubs and corner blocks of
  adjacent tiles coincide or nest.

## Licensing

- **Golly demos.** The three `.mc.gz` files are part of the Golly distribution.
  Golly's `docs/License.html` states that Golly is under GPL-2.0-or-later
  (Copyright 2005-2026 The Golly Gang). We treat these pattern files as
  GPL-2.0-or-later data and redistribute them unmodified apart from the comment
  line.
- **Brice Due's base tiles.** The tiles `OTCAmetapixel-{ON,OFF}.rle` come from
  Brice Due's personal site.
  - **No explicit license is stated** on that site or in the file header.
    (UNVERIFIED whether any license was ever attached.)
  - **The same cell data is in Golly's GPL demos.** We verified that the
    2048×2048 interior of every tile in the GPL-licensed Golly demos above is
    *exactly* these tiles plus rule-programming cells and corner tubs. So the
    same information is distributed by Golly under GPL-2.0-or-later.
  - **We redistribute them under GPL-2.0-or-later.** Credit goes to Brice Due as
    the author, as the file headers ask.
  - **To avoid any doubt, the tiles can be rebuilt from Golly.** Extract any
    level-11 node of `metapixel-galaxy.mc.gz` and add the tub and corner-block
    cells listed in the research document.
- **Our code does not include GPL code.** No GPL *code* (Golly C++, `metafier`
  scripts, or `tile-otcamp.pl`) is copied into the project. Only pattern data
  is stored here. The Rust implementation must be written from the
  specification in `docs/research/otca-metapixel.md`, not by translating the
  GPL scripts.
- **Binaries that embed these files carry GPL obligations.** Shipping any of
  these files inside a binary or package, for example via `include_bytes!`,
  makes that artifact carry GPL-2.0-or-later data. Keep them as separately
  loaded asset files, or have legal review before embedding.

## Related material that is NOT included

These sources were consulted only, never copied into the project:

- **Brice Due's sample tilings.** Available at
  `http://www.cranemtn.com/life/weblog/` as `p46ticktock-META-B3-S23.mc.gz`,
  `o0138-META-B3-S23.mc.gz`, `meta-toad.mc.gz`, `sigmaguns-META-B3-S23.mc.gz`,
  `DblBoojum-META-B3-S23.mc.gz`, `mwssmaze-META-B3-S12345.mc.gz`,
  `H-trees-META-B1-S1234568.mc.gz` and `metablog.tar.gz`. The license is not
  stated.
- **Brice Due's tiling script `tile-otcamp-9.pl`.** It ships inside
  `tileotcamp9pl.zip`, which has sha256
  `beed81cedff2d51dcc05de177e4ccb089804c45eeb198e9cf01502e40f7bcf71`. It was
  read only and never executed.
- **Golly's `Scripts/Lua/metafier.lua` and `Scripts/Python/metafier.py`.** These
  are by Dave Greene (2006), GPL, and were read only.
  - Their tile definitions differ slightly from Brice Due's original tiles.
    Details are in the research document.
- **The "How does it work" slides 21–31 on `otcametapixel.blogspot.com`.**
  These are images, retrieved via the Internet Archive.
