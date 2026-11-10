//! The five model families of the laboratory: names, the scenario each
//! family opens by default, state names and state colours.
//!
//! Colours are sRGB bytes so they can be used both for the grid renderer
//! (`rendering::GridPalette`) and for egui legends. Every state colour of a
//! family is the colour the grid renderer draws for that state byte (the
//! palettes are built from the same tables).

use bevy::prelude::Color;
use bevy_egui::egui::Color32;
use cellular_automata::scenario::ScenarioModel;
use patterns::GenerationsRule;
use rendering::GridPalette;
use rendering::palette::MAX_PALETTE_COLORS;

/// A model family of the laboratory (one model of the `cellular-automata`
/// crate each).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Family {
    /// Two-state Life-like `B/S` rules (`ca.life_like`).
    #[default]
    LifeLike,
    /// Multi-state Generations rules (`ca.generations`).
    Generations,
    /// Wireworld (`ca.wireworld`).
    Wireworld,
    /// Langton's ant and multi-colour turmites (`ca.langtons_ant`).
    Ant,
    /// Elementary one-dimensional automata (`ca.wolfram`).
    Wolfram,
}

impl Family {
    /// Every family, in display order.
    pub const ALL: [Family; 5] =
        [Family::LifeLike, Family::Generations, Family::Wireworld, Family::Ant, Family::Wolfram];

    /// Display name.
    pub fn label(self) -> &'static str {
        match self {
            Family::LifeLike => "Life-like",
            Family::Generations => "Generations",
            Family::Wireworld => "Wireworld",
            Family::Ant => "Langton's ant",
            Family::Wolfram => "Elementary CA",
        }
    }

    /// Short name for compact buttons.
    pub fn short_label(self) -> &'static str {
        match self {
            Family::LifeLike => "Life-like",
            Family::Generations => "Generations",
            Family::Wireworld => "Wireworld",
            Family::Ant => "Ants",
            Family::Wolfram => "Wolfram",
        }
    }

    /// One-sentence description for tooltips.
    pub fn blurb(self) -> &'static str {
        match self {
            Family::LifeLike => "Two states and a birth/survival rule B/S on the 8-cell Moore neighbourhood.",
            Family::Generations => {
                "Life-like rules with extra 'dying' states: a cell that stops living decays step by step to dead."
            }
            Family::Wireworld => "Four states (empty, electron head, electron tail, conductor) for digital circuits.",
            Family::Ant => "Ants that turn by the colour under them and recolour it (turmite rules such as RL, LLRR).",
            Family::Wolfram => "One-dimensional rows of 0s and 1s; the 256 rules drawn as a space-time diagram.",
        }
    }

    /// Scenario opened when the family is picked in the tools panel.
    pub fn default_scenario(self) -> &'static str {
        match self {
            Family::LifeLike => "highlife-replicator",
            Family::Generations => "brians-brain",
            Family::Wireworld => "wireworld-circuits",
            Family::Ant => "langtons-ant-highway",
            Family::Wolfram => "wolfram-rule-30",
        }
    }

    /// Family of a scenario model (`None` for the Classic Conway scenarios,
    /// which belong to another mode).
    pub fn of_model(model: &ScenarioModel) -> Option<Family> {
        match model {
            ScenarioModel::Classic(_) => None,
            ScenarioModel::LifeLike(_) => Some(Family::LifeLike),
            ScenarioModel::Generations(_) => Some(Family::Generations),
            ScenarioModel::Wireworld(_) => Some(Family::Wireworld),
            ScenarioModel::Ant(_) => Some(Family::Ant),
            ScenarioModel::Wolfram(_) => Some(Family::Wolfram),
        }
    }

    /// Name of the tick counter shown in readouts.
    pub fn tick_name(self) -> &'static str {
        match self {
            Family::Ant => "Step",
            Family::Wolfram => "Row",
            _ => "Generation",
        }
    }

    /// Key of the metric shown as "population" in the overlay.
    pub fn headline_metric(self) -> (&'static str, &'static str) {
        match self {
            Family::LifeLike => ("population", "Population"),
            Family::Generations => ("alive", "Alive"),
            Family::Wireworld => ("heads", "Electron heads"),
            Family::Ant => ("colored", "Coloured cells"),
            Family::Wolfram => ("ones", "Ones in the current row"),
        }
    }
}

/// sRGB bytes.
pub type Rgb = [u8; 3];

/// Background (state 0) of the Life-like, Generations, ant and Wolfram palettes.
pub const DEAD: Rgb = [14, 16, 22];
/// Live cells of the two-state palettes (Life-like, Wolfram) and colour 1 of the ants.
pub const ALIVE: Rgb = [236, 232, 214];

/// Wireworld: empty cells (darker than Golly's grey so circuits stand out
/// against the dark lab theme), then Golly's head, tail and conductor colours.
pub const WIREWORLD: [Rgb; 4] = [[22, 25, 32], [0, 128, 255], [255, 255, 255], [255, 128, 0]];

/// Brian's Brain: ready (dark), firing (white), refractory (blue).
pub const BRIANS_BRAIN: [Rgb; 3] = [[6, 8, 14], [235, 242, 255], [40, 92, 220]];

/// Ant colours 2..16 (colours 0 and 1 are [`DEAD`] and [`ALIVE`]); no
/// teal, which is the mode's accent and the colour of the ant markers.
const ANT_COLORS: [Rgb; 14] = [
    [245, 120, 60],
    [199, 146, 234],
    [242, 184, 75],
    [90, 169, 255],
    [255, 122, 144],
    [108, 203, 95],
    [170, 180, 200],
    [160, 70, 30],
    [110, 70, 160],
    [150, 110, 30],
    [40, 90, 160],
    [160, 50, 80],
    [200, 230, 90],
    [90, 110, 130],
];

/// Colours of the Generations states `0..states` (state 0 dead, 1 alive,
/// then the dying states fading from orange-red to dark blue): the
/// renderer's Generations palette, or Brian's Brain colours for that rule.
pub fn generations_colors(rule: GenerationsRule) -> Vec<Rgb> {
    if rule.name() == Some("Brian's Brain") {
        return BRIANS_BRAIN.to_vec();
    }
    match GridPalette::generations(usize::from(rule.states)) {
        Ok(GridPalette::States(colors)) => colors.iter().map(srgb_bytes).collect(),
        _ => vec![DEAD, ALIVE],
    }
}

/// Colours of `n` ant colours (2..=16).
pub fn ant_colors(n: usize) -> Vec<Rgb> {
    let mut out = vec![DEAD, ALIVE];
    out.extend(ANT_COLORS.iter().copied().take(n.saturating_sub(2)));
    out.truncate(n.max(1));
    out
}

/// sRGB bytes of a Bevy colour.
pub fn srgb_bytes(color: &Color) -> Rgb {
    let c = color.to_srgba();
    [c.red, c.green, c.blue].map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// egui colour of sRGB bytes.
pub fn color32(rgb: Rgb) -> Color32 {
    Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}

/// Grid-renderer palette of state colours (at most 32 are uploaded; later
/// states use the last colour, as the renderer does).
pub fn grid_palette(colors: &[Rgb]) -> GridPalette {
    let list: Vec<Color> = colors.iter().take(MAX_PALETTE_COLORS).map(|c| Color::srgb_u8(c[0], c[1], c[2])).collect();
    GridPalette::States(if list.is_empty() { vec![Color::srgb_u8(DEAD[0], DEAD[1], DEAD[2])] } else { list })
}

/// Name of state `state` of a Generations rule with `states` states. The
/// names follow the model: 0 dead, 1 alive, 2..C−1 dying; Brian's Brain
/// uses its own words (ready, firing, refractory).
pub fn generations_state_name(state: u8, states: u8, brians_brain: bool) -> String {
    match (state, brians_brain) {
        (0, true) => "ready (dead)".to_string(),
        (1, true) => "firing (alive)".to_string(),
        (2, true) => "refractory (dying)".to_string(),
        (0, false) => "dead".to_string(),
        (1, false) => "alive".to_string(),
        (s, _) if states <= 3 => format!("dying (state {s})"),
        (s, _) => format!("dying {} of {}", s - 1, states - 2),
    }
}

/// Name of a Wireworld state.
pub fn wireworld_state_name(state: u8) -> &'static str {
    match state {
        0 => "empty",
        1 => "electron head",
        2 => "electron tail",
        _ => "conductor",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellular_automata::scenario::CA_LAB_SCENARIO_FILES;

    #[test]
    fn default_scenarios_belong_to_their_family() {
        for family in Family::ALL {
            let text = CA_LAB_SCENARIO_FILES
                .iter()
                .find(|(name, _)| name.trim_end_matches(".ron") == family.default_scenario())
                .map(|(_, t)| *t)
                .expect("default scenario is bundled");
            let s = cellular_automata::scenario::parse(text, "test").expect("parses");
            assert_eq!(Family::of_model(&s.model), Some(family), "{}", s.id);
        }
    }

    #[test]
    fn palettes_match_the_renderer() {
        // Brian's Brain colours for that rule, the renderer's palette otherwise.
        let bb = GenerationsRule::brians_brain();
        assert_eq!(generations_colors(bb), BRIANS_BRAIN.to_vec());
        let sw = GenerationsRule::star_wars();
        let colors = generations_colors(sw);
        assert_eq!(colors.len(), 4);
        let GridPalette::States(expected) = GridPalette::generations(4).expect("valid") else { panic!() };
        assert_eq!(colors, expected.iter().map(srgb_bytes).collect::<Vec<_>>());
        // Grid palettes reproduce the bytes.
        let p = grid_palette(&WIREWORLD);
        assert_eq!(srgb_bytes(&p.color(3)), WIREWORLD[3]);
        assert_eq!(srgb_bytes(&p.color(200)), WIREWORLD[3], "states beyond the table use the last colour");
        assert_eq!(ant_colors(2), vec![DEAD, ALIVE]);
        assert_eq!(ant_colors(16).len(), 16);
        let distinct: std::collections::HashSet<Rgb> = ant_colors(16).into_iter().collect();
        assert_eq!(distinct.len(), 16, "every ant colour is distinct");
    }

    #[test]
    fn state_names() {
        assert_eq!(generations_state_name(2, 3, true), "refractory (dying)");
        assert_eq!(generations_state_name(2, 3, false), "dying (state 2)");
        assert_eq!(generations_state_name(3, 4, false), "dying 2 of 2");
        assert_eq!(generations_state_name(1, 24, false), "alive");
        assert_eq!(wireworld_state_name(3), "conductor");
        assert_eq!(Family::Wolfram.tick_name(), "Row");
    }
}
