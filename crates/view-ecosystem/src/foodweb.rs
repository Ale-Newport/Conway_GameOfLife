//! The food web: species and basal food nodes linked by their diets, with
//! the flows the model exposes (kills per predator and prey species, energy
//! eaten while grazing or browsing), laid out in rows by the model's trophic
//! levels.

use ecosystem::metrics::TrophicLevel;
use ecosystem::species::Food;
use ecosystem::units::TISSUE_KJ_PER_KG;

use crate::session::{FeedingMode, Observed, StaticInfo};

/// Food that is not a species.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Basal {
    /// Vegetation layers and tree foliage.
    Plants,
    /// Flower nectar.
    Nectar,
    /// Litter and dung.
    Detritus,
    /// Mushrooms.
    Fungi,
    /// Carcasses.
    Carrion,
    /// Other species' eggs.
    Eggs,
}

impl Basal {
    /// Display label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Plants => "Plants",
            Self::Nectar => "Nectar",
            Self::Detritus => "Detritus",
            Self::Fungi => "Fungi",
            Self::Carrion => "Carrion",
            Self::Eggs => "Eggs",
        }
    }
}

/// A node of the web.
#[derive(Clone, Debug, PartialEq)]
pub struct WebNode {
    /// What it stands for.
    pub kind: NodeKind,
    /// Label.
    pub label: String,
    /// Row: 0 basal, 1 primary consumers, 2 secondary, 3 apex.
    pub level: usize,
    /// Horizontal position in `[0, 1]`.
    pub x: f32,
    /// Current population (species; field and colony species count
    /// individuals and workers as the model does).
    pub population: f64,
    /// Present now (population > 0).
    pub alive: bool,
}

/// Node identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NodeKind {
    /// Basal food.
    Basal(Basal),
    /// Animal species index.
    Species(u16),
}

/// How an edge's flow was observed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Flow {
    /// Kills: count and body mass (kg).
    Kills {
        /// Animals killed.
        count: u64,
        /// Body mass killed (kg).
        kg: f64,
    },
    /// Energy eaten in a feeding mode (kJ).
    Eaten(f64),
    /// The model does not separate this flow.
    NotExposed,
}

impl Flow {
    /// Energy equivalent (kJ) used for line widths; kills count their
    /// tissue energy (7 kJ/g).
    pub fn energy_kj(self) -> f64 {
        match self {
            Self::Kills { kg, .. } => kg * TISSUE_KJ_PER_KG,
            Self::Eaten(kj) => kj,
            Self::NotExposed => 0.0,
        }
    }
}

/// A diet link: `to` eats `from`.
#[derive(Clone, Debug, PartialEq)]
pub struct WebEdge {
    /// Food node.
    pub from: usize,
    /// Consumer node.
    pub to: usize,
    /// Diet preference of the consumer for this food.
    pub preference: f32,
    /// Observed flow.
    pub flow: Flow,
}

/// The laid-out web.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FoodWeb {
    /// Nodes.
    pub nodes: Vec<WebNode>,
    /// Edges.
    pub edges: Vec<WebEdge>,
    /// Ticks the flows were observed over.
    pub observed_ticks: u64,
}

fn level_of(t: TrophicLevel) -> usize {
    match t {
        TrophicLevel::Primary => 1,
        TrophicLevel::Secondary => 2,
        TrophicLevel::Apex => 3,
    }
}

/// Builds the web of the species taking part in the run: those alive now
/// (`populations`) or that were born or died during the run (`ever`).
pub fn build(info: &StaticInfo, populations: &[f64], ever: &[bool], observed: &Observed) -> FoodWeb {
    let animals = info.catalog.animals();
    let included: Vec<u16> = (0..animals.len())
        .filter(|&i| populations.get(i).is_some_and(|p| *p > 0.0) || ever.get(i).copied().unwrap_or(false))
        .map(|i| i as u16)
        .collect();
    let mut nodes: Vec<WebNode> = Vec::new();
    let index = |nodes: &mut Vec<WebNode>, kind: NodeKind| -> usize {
        if let Some(i) = nodes.iter().position(|n| n.kind == kind) {
            return i;
        }
        let (label, level, population) = match kind {
            NodeKind::Basal(b) => (b.label().to_string(), 0, 0.0),
            NodeKind::Species(s) => (
                animals[usize::from(s)].name.clone(),
                level_of(info.trophic[usize::from(s)]),
                populations.get(usize::from(s)).copied().unwrap_or(0.0),
            ),
        };
        nodes.push(WebNode { kind, label, level, x: 0.5, population, alive: population > 0.0 });
        nodes.len() - 1
    };
    for &s in &included {
        index(&mut nodes, NodeKind::Species(s));
    }
    let mut edges = Vec::new();
    for &s in &included {
        let to = index(&mut nodes, NodeKind::Species(s));
        let species = &animals[usize::from(s)];
        // Merge the plant layers and browse into one link.
        let mut plants = 0.0f32;
        for entry in &species.diet {
            let (food, flow) = match &entry.food {
                Food::Plant(_) | Food::Browse => {
                    plants += entry.preference;
                    continue;
                }
                Food::Prey(prey) => {
                    let Some(p) = info.catalog.animal_index(prey).ok().filter(|p| included.contains(p)) else {
                        continue;
                    };
                    let tally = observed.kills_of(s, p);
                    let individual = animals[usize::from(p)].is_individual() || info.netlogo.is_some();
                    let flow =
                        if individual { Flow::Kills { count: tally.count, kg: tally.kg } } else { Flow::NotExposed };
                    (NodeKind::Species(p), flow)
                }
                Food::Carrion => (NodeKind::Basal(Basal::Carrion), Flow::NotExposed),
                Food::Nectar => (NodeKind::Basal(Basal::Nectar), Flow::NotExposed),
                Food::Detritus => (NodeKind::Basal(Basal::Detritus), Flow::NotExposed),
                Food::Fungi => (NodeKind::Basal(Basal::Fungi), Flow::NotExposed),
                Food::Eggs => (NodeKind::Basal(Basal::Eggs), Flow::NotExposed),
            };
            let from = index(&mut nodes, food);
            edges.push(WebEdge { from, to, preference: entry.preference, flow });
        }
        if plants > 0.0 {
            let from = index(&mut nodes, NodeKind::Basal(Basal::Plants));
            let eaten = observed.eaten_by(s, FeedingMode::Grazing) + observed.eaten_by(s, FeedingMode::Browsing);
            let flow =
                if species.is_individual() && info.netlogo.is_none() { Flow::Eaten(eaten) } else { Flow::NotExposed };
            edges.push(WebEdge { from, to, preference: plants, flow });
        }
    }
    let mut web = FoodWeb { nodes, edges, observed_ticks: observed.ticks };
    layout(&mut web);
    web
}

/// Places nodes in rows by level and orders each row by the barycentre of
/// the foods it eats (fewer crossings), basal foods in a fixed order.
pub fn layout(web: &mut FoodWeb) {
    let rows = web.nodes.iter().map(|n| n.level).max().map_or(0, |m| m + 1);
    // Basal row: fixed order by kind.
    let mut order: Vec<Vec<usize>> = vec![Vec::new(); rows];
    for (i, n) in web.nodes.iter().enumerate() {
        order[n.level].push(i);
    }
    if let Some(basal) = order.first_mut() {
        basal.sort_by_key(|&i| web.nodes[i].kind);
    }
    let place = |web: &mut FoodWeb, row: &[usize]| {
        let n = row.len();
        for (k, &i) in row.iter().enumerate() {
            web.nodes[i].x = (k as f32 + 0.5) / n as f32;
        }
    };
    for row in &order {
        place(web, row);
    }
    // Two sweeps upwards: sort each row by the preference-weighted mean x
    // of its foods (ties by name for determinism).
    for _ in 0..2 {
        #[allow(clippy::needless_range_loop)]
        for level in 1..rows {
            let mut keyed: Vec<(f32, usize)> = order[level]
                .iter()
                .map(|&i| {
                    let (sum, weight) =
                        web.edges.iter().filter(|e| e.to == i).fold((0.0f32, 0.0f32), |(s, w), e| {
                            (s + web.nodes[e.from].x * e.preference, w + e.preference)
                        });
                    (if weight > 0.0 { sum / weight } else { web.nodes[i].x }, i)
                })
                .collect();
            keyed.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| web.nodes[a.1].label.cmp(&web.nodes[b.1].label)));
            order[level] = keyed.into_iter().map(|(_, i)| i).collect();
            let row = order[level].clone();
            place(web, &row);
        }
    }
}

impl FoodWeb {
    /// Largest observed flow (kJ) for scaling line widths.
    pub fn max_flow_kj(&self) -> f64 {
        self.edges.iter().map(|e| e.flow.energy_kj()).fold(0.0, f64::max)
    }

    /// Line width (points) of an edge: thin for diet links without an
    /// observed flow, up to `max_width` for the largest flow (square root).
    pub fn edge_width(&self, edge: &WebEdge, max_width: f32) -> f32 {
        let max = self.max_flow_kj();
        let kj = edge.flow.energy_kj();
        if max <= 0.0 || kj <= 0.0 { 1.0 } else { 1.0 + (max_width - 1.0) * (kj / max).sqrt() as f32 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{EcoSession, FrameRequest};
    use simulation_core::Simulation;

    fn web_of(id: &str, ticks: usize) -> (FoodWeb, EcoSession) {
        let mut s = EcoSession::new(&ecosystem::scenario::find_builtin(id).unwrap()).unwrap();
        for _ in 0..ticks {
            s.step();
        }
        let frame = s.view(&FrameRequest::default());
        let pops: Vec<f64> = frame.view.species.iter().map(|p| f64::from(p.population)).collect();
        let ever: Vec<bool> =
            frame.counters.total_births.iter().zip(&frame.counters.total_deaths).map(|(b, d)| b + d > 0).collect();
        (build(&frame.info, &pops, &ever, &frame.observed), s)
    }

    #[test]
    fn wolf_sheep_web_has_the_classic_chain() {
        let (web, s) = web_of("wolf_sheep_grass", 300);
        let cat = s.sim().catalog();
        let sheep = cat.animal_index("sheep").unwrap();
        let wolf = cat.animal_index("wolf").unwrap();
        let node = |k: NodeKind| web.nodes.iter().position(|n| n.kind == k).expect("node");
        let (p, sh, w) =
            (node(NodeKind::Basal(Basal::Plants)), node(NodeKind::Species(sheep)), node(NodeKind::Species(wolf)));
        assert!(web.nodes[p].level < web.nodes[sh].level && web.nodes[sh].level < web.nodes[w].level);
        let predation = web.edges.iter().find(|e| e.from == sh && e.to == w).expect("wolves eat sheep");
        assert!(matches!(predation.flow, Flow::Kills { count, .. } if count > 0), "{:?}", predation.flow);
        assert!(web.edges.iter().any(|e| e.from == p && e.to == sh));
        assert!(web.nodes.iter().all(|n| (0.0..=1.0).contains(&n.x)));
    }

    #[test]
    fn meadow_web_is_layered_and_weighted() {
        let (web, _) = web_of("balanced_meadow", 240);
        assert!(web.nodes.len() > 8, "{}", web.nodes.len());
        for e in &web.edges {
            assert!(e.from != e.to, "no self loops");
            assert!(web.nodes[e.from].level <= web.nodes[e.to].level || web.nodes[e.to].level >= 1);
        }
        assert!(web.max_flow_kj() > 0.0, "grazing is observed within a day");
        let widths: Vec<f32> = web.edges.iter().map(|e| web.edge_width(e, 6.0)).collect();
        assert!(widths.iter().all(|w| (1.0..=6.0).contains(w)));
        // Rows hold distinct x positions.
        for level in 0..4 {
            let mut xs: Vec<f32> = web.nodes.iter().filter(|n| n.level == level).map(|n| n.x).collect();
            xs.sort_by(f32::total_cmp);
            assert!(xs.windows(2).all(|w| w[1] > w[0]));
        }
    }
}
