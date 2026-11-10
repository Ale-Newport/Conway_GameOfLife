//! Tutorial and concepts of the Three-Dimensional Life mode.
//!
//! Action buttons use the ids understood by
//! [`crate::interact::handle_actions`] (`load:<scenario>`, `tab:<tab>`,
//! `cut:middle`, `color:<mode>`, …).

use interface::{Tutorial, TutorialPage};

/// The mode's tutorial.
pub fn tutorial() -> Tutorial {
    Tutorial {
        pages: vec![
            TutorialPage::new(
                "Three dimensions, new rules",
                "Each voxel of the cubic lattice has **26** neighbours in the *Moore* neighbourhood (every voxel \
                 touching it by a face, an edge or a corner) or **6** in the *von Neumann* neighbourhood (faces \
                 only). A rule says when an empty voxel is **born** and when an alive voxel **survives**, by \
                 counting its alive neighbours.\n\n\
                 **These are not Conway's rules.** Conway's B3/S23 was tuned for the 8 neighbours of a square \
                 grid. Typed here, `B3/S23` is a *different* automaton (birth on 3 of 26 neighbours, survival on \
                 2 or 3 of 26). Rules with Life-like behaviour in 3D had to be searched for anew: Carter Bays \
                 proposed **4555** and **5766** (1987).\n\n\
                 The button loads a 16³ random soup under 5766 inside a 64³ box. Press **Space** to run it.",
            )
            .with_action("load:life-5766-soup", "Load the 5766 soup"),
            TutorialPage::new(
                "Reading a rule",
                "The Softology notation `S/B/C/N` lists the **survival** counts, the **birth** counts, the number \
                 of **states** C and the **neighbourhood** (`M` Moore, `N` von Neumann). Bays wrote two-state \
                 Moore rules as four numbers E_l E_u F_l F_u: **5766** means *survive with 5 to 7 alive \
                 neighbours, be born with exactly 6*, the same rule as `5-7/6/2/M`, `B6/S567` and Golly's \
                 `3D5..7/6`.\n\n\
                 With **C > 2** states the rule is of the *Generations* type: an alive voxel that fails to \
                 survive enters dying state 2, then 3, … C−1, and then becomes empty, whatever its neighbours. \
                 Dying voxels are **not** counted as neighbours and cannot be reborn before they are empty.\n\n\
                 The rule editor in the Tools panel accepts every notation and validates it as you type. The \
                 Rules tab states the rule in effect.",
            )
            .with_action("tab:rules", "Open the Rules tab"),
            TutorialPage::new(
                "Look inside: orbit, cut, inspect",
                "Drag with the **left** mouse button to orbit, the **right** button to pan, scroll to zoom. \
                 Keys **1–4** switch camera presets and **F** frames the world.\n\n\
                 A solid world hides its interior, so **cut** it: press **K** (or *Cut* in the Tools panel) to \
                 keep only the layers between two planes; **[** and **]** move the upper plane. Voxels cut open \
                 show their inside faces.\n\n\
                 The **slice inspector** (Properties tab) draws the exposed layer as a 2D image; hover it to \
                 read states. With the **Inspect** tool (**I**), click a voxel to read its state, age, number of \
                 alive neighbours and what the rule will do with it in the next generation.",
            )
            .with_action("cut:middle", "Cut the world in half"),
            TutorialPage::new(
                "Simple example: the 5766 glider",
                "Bays' 10-voxel glider is Conway's glider copied into two adjacent planes. Under 5766 it has \
                 **period 4** and moves by **(1, 1, 0)** every period, with 10 voxels in every phase; on this \
                 24 × 24 × 8 torus it is back at its start after 96 generations.\n\n\
                 Press **Space**. The *phase* line of the overlay reports that the population counts repeat and \
                 that the centroid moves (1, 1, 0) every 4 generations. The centroid is an arithmetic mean, so \
                 the report pauses for a few generations each time the glider wraps around the torus.",
            )
            .with_action("load:life-5766-glider", "Load the 5766 glider"),
            TutorialPage::new(
                "Conway's Life hidden inside 5766",
                "Take a bounded world **two voxels deep** whose two planes are identical. An alive voxel with \
                 *n* alive neighbours in its own plane has **2n + 1** alive 3D neighbours (its twin counts); an \
                 empty one has **2n**. Then\n\n\
                 * 5766 survival, 5 ≤ 2n + 1 ≤ 7, holds exactly for n = 2 or 3;\n\
                 * 5766 birth, 2n = 6, holds exactly for n = 3.\n\n\
                 So each plane runs Conway's B3/S23 and the planes stay identical. This is a special \
                 construction (Bays, Theorem 7); in general a 3D rule does not reduce to 2D Life.",
            )
            .with_action("load:life-in-5766-two-planes", "Load the two-plane world"),
            TutorialPage::new(
                "Advanced: Builder 1, a 10-state rule",
                "`2,6,9/4,6,8-9/10/M`: survive with 2, 6 or 9 alive neighbours, born with 4, 6, 8 or 9, and \
                 **10 states**, so a voxel that fails to survive fades through the 8 dying states 2 … 9 before \
                 it vanishes.\n\n\
                 Colour by **state** (key **C**) to see alive voxels in gold against their fading trail, and \
                 press **X** for *Ghost dying* to see the alive core through the trail. With this seed the \
                 population peaks above 23,000 voxels around generation 60 and falls to about 14,500 by \
                 generation 150 (asserted by the model's tests).",
            )
            .with_action("load:builder-1", "Load Builder 1"),
            TutorialPage::new(
                "Reading the metrics",
                "* **Population**: non-empty voxels (every state except 0).\n\
                 * **Alive (state 1)**: the only voxels counted as neighbours. **Dying state k**: voxels in \
                 refractory state k.\n\
                 * **Births**: empty voxels that became alive in the last generation. **Deaths**: alive voxels \
                 that did not survive it (they became empty or started dying).\n\
                 * **Bounding box** min / max per axis and **centroid**: extent and mean position of the \
                 non-empty voxels (the centroid is an arithmetic mean, also on a torus).\n\n\
                 The Properties tab plots alive against dying voxels and births against deaths; the Metrics tab \
                 plots any metric and exports CSV or JSON.",
            )
            .with_action("tab:metrics", "Open the Metrics tab"),
            TutorialPage::new(
                "Experiments to try",
                "1. **Bounded growth.** Load the 5766 soup, change *Density* (Seed section of the Tools panel) to 0.15, 0.3 and \
                 0.45 and press *Seed now*: Bays required every random soup of a 3D Life rule to stay bounded.\n\
                 2. **Two gliders.** Load the 4555 and the 5766 glider and compare: (−1, −1, 0) against (1, 1, 0) \
                 every 4 generations.\n\
                 3. **Symmetry is conserved.** Every rule here treats the 26 (or 6) neighbours alike, so a \
                 mirror-symmetric world stays symmetric. Load Amoeba, choose mirror `xyz` and an even cube side, \
                 seed and run (an even cube in this even-sized box is centred, so the walls are symmetric too).\n\
                 4. **Moore against von Neumann.** Load Crystal Growth 1 (`0-6/1,3/2/N`), change the last letter \
                 to `M` in the rule editor, apply and reset: compare the shapes.\n\
                 5. **Region census.** Turn on the region box (**M**), fit it to the population and follow the \
                 share of dying voxels while Builder 1 grows and decays.",
            )
            .with_action("load:softology-amoeba", "Load Amoeba"),
        ],
        concepts: CONCEPTS.to_string(),
    }
}

/// The concepts page (Markdown).
pub const CONCEPTS: &str = "## Three-dimensional outer-totalistic automata\n\
The lattice is the cubic grid of voxels. Every voxel has a state 0 … C−1 (0 empty, 1 alive, 2 … C−1 dying). \
All voxels update together from the previous generation. Let N be the number of **alive** (state 1) \
neighbours:\n\n\
| State | Next state |\n\
|---|---|\n\
| empty | alive if N is in B, else empty |\n\
| alive | alive if N is in S, else dying state 2 (C > 2) or empty (C = 2) |\n\
| dying k | k + 1, or empty after state C − 1 |\n\n\
**Neighbourhoods.** Moore: the 26 voxels with max(|dx|, |dy|, |dz|) = 1. von Neumann: the 6 face neighbours.\n\n\
## Not Conway's Life\n\
Conway's B3/S23 counts 8 neighbours on a square grid. The same numbers on 26 neighbours define a different \
automaton with different behaviour, so 3D rules are their own family. Carter Bays (1987) called a two-state \
Moore rule E_l E_u F_l F_u (survive iff E_l ≤ N ≤ E_u, born iff F_l ≤ N ≤ F_u) a *Game of Life* when a glider \
exists and arises naturally from random soups, and when every random soup shows bounded growth. **4555** and \
**5766** were his candidates; 5655 and 6855 followed (1994).\n\n\
## Notations accepted by the rule editor\n\
| Notation | Example | Meaning |\n\
|---|---|---|\n\
| Softology S/B/C/N | `4/4/5/M` | survive 4, born 4, 5 states, Moore |\n\
| Letters | `B6/S567`, `B4/S4/C5/N` | birth and survival lists, optional states and neighbourhood |\n\
| Bays | `5766`, `(10 21 10 21)` | two states, Moore, survival and birth intervals |\n\
| Golly 3D | `3D5..7/6`, `3D4,5/5F` | survival first; `F` or `V` means von Neumann |\n\n\
Birth on 0 neighbours is rejected: every empty voxel of the world would be born at once.\n\n\
## Edges\n\
* **Bounded**: voxels outside the box are permanently empty.\n\
* **Torus**: opposite faces are joined; on an axis shorter than 3 a neighbour reached through several offsets \
counts once per offset.\n\
* **Unbounded** (sparse): no edges within ±2^30; the run halts if it would need more than 32,768 chunks of 16³.\n\n\
## What the view shows\n\
* Only voxels with at least one exposed face are drawn (the *surface view*); the cut planes expose the \
interior.\n\
* **Colours**: by state (alive gold, dying from orange to dark purple), by age (generations since birth, \
logarithmic) or by height.\n\
* **Depth shading** darkens voxels with many visible neighbours (crevices, concave regions). It is a display \
aid only, like the shadows and the fog.\n\
* The **slice inspector** shows one complete layer, interior included, as read from the simulation.\n\
* **Region** counts voxels per state inside a box, read from the simulation (not from the drawn surface).\n\n\
## References\n\
* C. Bays, *Candidates for the Game of Life in Three Dimensions*, Complex Systems 1 (1987) 373–400.\n\
* C. Bays, *Further Notes on the Game of Three-Dimensional Life*, Complex Systems 8 (1994) 67–73.\n\
* J. Rampe, *3D Cellular Automata*, Softology blog (2019).\n\
* Golly 5.1, Generations algorithm and `3D.lua`.\n";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_is_understood() {
        let tutorial = tutorial();
        assert!(tutorial.pages.len() >= 8);
        let scenarios = crate::builtin_scenarios();
        for page in &tutorial.pages {
            let Some(action) = &page.action else { continue };
            match action.id.strip_prefix("load:") {
                Some(id) => assert!(scenarios.iter().any(|s| s.id == id), "unknown scenario {id}"),
                None => assert!(
                    ["tab:rules", "tab:metrics", "cut:middle"].contains(&action.id.as_str()),
                    "unexpected action {}",
                    action.id
                ),
            }
        }
    }

    #[test]
    fn quoted_rules_parse_to_the_stated_forms() {
        use voxel_life::Rule3D;
        let r = |s: &str| Rule3D::parse(s).expect(s);
        assert_eq!(r("5766"), r("5-7/6/2/M"));
        assert_eq!(r("5766"), r("B6/S567"));
        assert_eq!(r("5766"), r("3D5..7/6"));
        assert_eq!(r("4/4/5/M").states(), 5);
        assert_eq!(r("B4/S4/C5/N").neighbourhood(), voxel_life::Neighbourhood::VonNeumann);
        assert_eq!(r("3D4,5/5F").neighbourhood(), voxel_life::Neighbourhood::VonNeumann);
        assert_eq!(r("(10 21 10 21)").survive().iter().collect::<Vec<_>>(), (10..=21).collect::<Vec<_>>());
        assert_eq!(r("0-6/1,3/2/M").neighbourhood(), voxel_life::Neighbourhood::Moore);
        // B3/S23 is accepted and is a 3D rule over 26 neighbours.
        assert_eq!(r("B3/S23").neighbourhood().size(), 26);
        assert!(Rule3D::parse("B0/S1").is_err(), "birth on 0 neighbours is rejected");
        // The two-plane argument of the tutorial.
        let life = r("5766");
        for n in 0..=8u32 {
            let survives = life.next_state(1, 2 * n + 1) == 1;
            let born = life.next_state(0, 2 * n) == 1;
            assert_eq!(survives, n == 2 || n == 3, "n = {n}");
            assert_eq!(born, n == 3, "n = {n}");
        }
    }
}
