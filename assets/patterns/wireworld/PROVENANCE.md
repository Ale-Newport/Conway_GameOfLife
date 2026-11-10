# Wireworld circuits: provenance and licences

These RLE files (Golly extended RLE, `rule = WireWorld`; states `.` empty,
`A` electron head, `B` electron tail, `C` conductor) are used by the
`cellular-automata` crate (`WireworldSim`, the `scenarios/ca-lab/` Wireworld
scenarios and `tests/wireworld.rs`). Every behaviour quoted in a file's `#C`
lines is asserted by `crates/cellular-automata/tests/wireworld.rs`. The
permissive files are embedded in the crate; the GPL AND gate is read from
this directory at run time.

| File | Origin | Licence |
|---|---|---|
| `diode.rle` | Own layout, written for Emergence Lab (`docs/research/automata-catalog.md` §4.1) | MIT OR Apache-2.0 |
| `clock-8.rle` | Own layout (§4.1) | MIT OR Apache-2.0 |
| `clock-12.rle` | Own layout (§4.1) | MIT OR Apache-2.0 |
| `or-gate.rle` | Own layout (§4.1) | MIT OR Apache-2.0 |
| `xor-gate.rle` | Own layout of the well-known ring XOR (the same structure as the McIntosh/Dewdney gate), §4.1 | MIT OR Apache-2.0 |
| `and-gate-mcintosh.rle` | Golly 5.1 `Patterns/WireWorld/gate-AND.mcl` by Harold V. McIntosh: the first of the five gate instances in that file, converted from MCell `#L` lines to RLE. Cells unchanged; the original `#D` text is kept as `#C` lines after a marker | **GPL-2.0-or-later** (Golly distribution) |

## Notes

* Source of the Golly file: the Golly 5.1 source tree, SourceForge git
  `https://git.code.sf.net/p/golly/code`, commit `9a7079a84a24f4b5ef16b1c940660d75656e51be`
  ("updated version to 5.1", 2026-11-06). The file carries no individual
  licence grant, so it is redistributed under the licence of the Golly
  distribution, GPL-2.0-or-later, as a separately licensed data file. It is
  read from this directory at run time and never compiled into a binary;
  no Golly code is used.
* An own AND layout was not designed: the obvious construction
  `A ∧ B = (A ∨ B) ⊕ (A ⊕ B)` needs a wire crossing (its connection graph
  contains K3,3 once the inputs and the output are on the outside), so the
  published McIntosh gate is used instead. Replace this file to make the
  folder entirely permissive.
* Golly's `primes.mc` (Wireworld computer, "Copyright 1992, 2008 David Moore
  and Mark Owen") and the other `.mcl` files are deliberately **not** bundled.
