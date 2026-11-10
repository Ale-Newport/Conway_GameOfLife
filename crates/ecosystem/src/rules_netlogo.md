### Wolves, sheep and grass (NetLogo Wolf Sheep Predation rules)

Re-implementation of the rules of Wilensky (1997), *NetLogo Wolf Sheep Predation* (no code copied).
The world is a torus of patches (one patch = one cell); the abiotic world steps but does not affect the animals.

**Setup.** Each patch is green with probability ½; green patches get `countdown = grass_regrowth_time`, brown ones
`countdown = random(grass_regrowth_time)`. Sheep and wolves start at uniform random positions with random integer
headings and energy `random(2 × gain_from_food)` (an integer in `[0, 2·gain)`).

**Each tick**, in this order:

1. **Sheep**, in a fresh seeded random order: turn right `random 50`°, left `random 50`°, move forward 1 patch.
   *Grass version:* energy −1; if the patch is green it turns brown and the sheep gains `sheep_gain_from_food`;
   the sheep dies if its energy is below 0. Then, with probability `sheep_reproduce_percent / 100`, the sheep halves
   its energy and hatches one lamb with that energy, which turns by a uniform random angle and moves forward 1.
   *No-grass version:* sheep never gain or lose energy.
2. **Wolves**, in a fresh random order: move like sheep; energy −1; if any sheep stands on the wolf's patch, one of
   them chosen uniformly at random is killed and the wolf gains `wolf_gain_from_food`; the wolf dies if its energy is
   below 0; then it reproduces like sheep with probability `wolf_reproduce_percent / 100` (not gated by energy).
3. **Grass**: a brown patch whose countdown is ≤ 0 turns green and its countdown resets to `grass_regrowth_time`;
   otherwise the countdown decreases by 1.

Animals born during a phase act from the next tick on but can be eaten in the same tick.
The run is finished when no animal is left, or when no wolf is left and sheep exceed `max_sheep`.
