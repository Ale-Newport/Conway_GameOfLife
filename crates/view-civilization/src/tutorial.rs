//! Tutorial pages, concepts and the actions their buttons trigger.
//!
//! Every statement here describes the executed rules of the model
//! (`crates/civilization/src/rules.md`, `docs/models/civilization.md`).

use interface::{Tutorial, TutorialPage};

/// Actions of tutorial buttons (`TutorialAction` ids).
#[derive(Clone, Debug, PartialEq)]
pub enum TutorialStep {
    /// Load a built-in scenario.
    Load(String),
    /// Select the neediest person, open the inspector and follow them.
    InspectNeediest,
    /// Show roads only (territory off) and run `n` ticks.
    ShowRoads(u64),
    /// Show the territory overlay.
    ShowTerritory,
    /// Turn barter on or off (the `trade_enabled` parameter).
    Barter(bool),
    /// Run `n` ticks now.
    Run(u64),
    /// Pick a tool.
    Tool(crate::state::Tool),
    /// Open the right panel on the Metrics tab.
    Metrics,
}

/// Parses a `TutorialAction` id.
pub fn parse_action(id: &str) -> Option<TutorialStep> {
    let (verb, arg) = id.split_once(':').unwrap_or((id, ""));
    match verb {
        "load" if !arg.is_empty() => Some(TutorialStep::Load(arg.to_string())),
        "inspect" => Some(TutorialStep::InspectNeediest),
        "roads" => arg.parse().ok().map(TutorialStep::ShowRoads),
        "territory" => Some(TutorialStep::ShowTerritory),
        "barter" => match arg {
            "on" => Some(TutorialStep::Barter(true)),
            "off" => Some(TutorialStep::Barter(false)),
            _ => None,
        },
        "run" => arg.parse().ok().map(TutorialStep::Run),
        "tool" => match arg {
            "disaster" => Some(TutorialStep::Tool(crate::state::Tool::Disaster)),
            "spawn" => Some(TutorialStep::Tool(crate::state::Tool::Spawn)),
            "add" => Some(TutorialStep::Tool(crate::state::Tool::AddResource)),
            _ => None,
        },
        "metrics" => Some(TutorialStep::Metrics),
        _ => None,
    }
}

/// The mode's tutorial.
pub fn tutorial() -> Tutorial {
    Tutorial {
        pages: vec![
            TutorialPage::new(
                "Emergent civilizations",
                "People live on a regenerating tile world: grassland, forest (wood), hills (stone), lakes \
                 (fish, drinkable from the shore), fertile riverbanks, berry bushes and impassable mountains. \
                 They have needs, skills, an inventory, preferences and relationships, and they gather, farm, \
                 craft tools, build huts, group stores and farm plots, barter, share, steal and fight.\n\n\
                 **Nothing is scripted.** Roads, groups, territories, markets and specialisation are not \
                 placed by the program: they emerge from many individual decisions. One tick is one hour.\n\n\
                 Start with the *Abundant resources* scenario and press **Space** to run it.",
            )
            .with_action("load:abundant", "Load Abundant resources"),
            TutorialPage::new(
                "Needs drive decisions",
                "Each person has energy (hunger is `1 − energy`), thirst, fatigue, loneliness and a felt danger. \
                 Hunger, thirst, fatigue and loneliness grow every tick; felt danger follows recent fights nearby \
                 and foreign territory. Health falls at zero energy or saturated thirst.\n\n\
                 A **utility AI** decides: every feasible action (eat, drink, rest, forage, log, quarry, farm, \
                 craft, build, trade, socialise, share, store, steal, court, explore) is scored as the product of \
                 response curves of its considerations — the need it serves, distance, skill, holdings — times \
                 an action weight. The best score wins, with ±5 % noise and a 15 % bonus for the current goal. \
                 People re-decide when a goal ends or fails, when thirst or hunger reach 0.8, and otherwise 8 \
                 ticks after the expected arrival.\n\n\
                 The button selects the neediest person and opens the inspector: compare the need bars with \
                 the goal and the route drawn on the map.",
            )
            .with_action("inspect", "Inspect the neediest person"),
            TutorialPage::new(
                "Barter at marginal rates of substitution",
                "Each person values food, wood, stone and tools with a Cobb–Douglas welfare \
                 `W = Π (h_g + 1)^(m_g / m_T)`, where the demand weights `m` come from metabolism, building \
                 plans and worn tools. The **marginal rate of substitution** \
                 `MRS = (m_a / (h_a + 1)) / (m_b / (h_b + 1))` is how much of good *b* someone would give for one \
                 more unit of *a* (shown in the inspector).\n\n\
                 When two people's MRSs differ, the one with the higher MRS buys *a* at the price \
                 `p = √(MRS_A · MRS_B)` in steps of 0.5 units, while **both** welfares rise and the MRSs do not \
                 cross (Sugarscape rule T). Nobody is told to trade: meetings happen because both gain.\n\n\
                 *Spontaneous trade routes* separates timber and stone (west) from berries (east).",
            )
            .with_action("load:trade_routes", "Load Spontaneous trade routes"),
            TutorialPage::new(
                "Roads emerge from footfall",
                "Every time someone enters a tile it adds `1 − G/100` to the tile's trail `G`; every tick the \
                 trail is multiplied by `1 − 1/T` (*T*: trail decay time). The intensity `I = G/100` lowers the walking cost, `c·(1 − k·I)`, and route \
                 planning (A*) prefers cheap tiles, so busy paths get busier: an active-walker model (Helbing et \
                 al. 1997). Tiles at or above the road threshold are drawn as roads.\n\n\
                 The button hides the territory, shows trails and runs ten days (240 ticks). Watch the network \
                 connect the places people actually visit — shores, patches, markets, homes.",
            )
            .with_action("roads:240", "Show trails and run 10 days"),
            TutorialPage::new(
                "Specialisation from learning by doing",
                "Skills improve with every tick of practice, `s' = s + λ(1 − s)`, and fade otherwise, `s' = s(1 − φ)`. \
                 Productivity is `(0.5 + skill)·(1 + tool bonus)·…`, so whoever happens to gather wood a few \
                 times becomes better at it, and with barter it pays to keep doing it and buy the rest.\n\n\
                 The **specialisation index** (Society charts) is the mean of `1 − H/ln K` over each adult's \
                 production shares and normalised skills: 0 is a generalist, 1 a pure specialist.\n\n\
                 Experiment: switch barter off, run, and compare specialisation and population with barter on.",
            )
            .with_action("barter:off", "Switch barter off"),
            TutorialPage::new(
                "Groups and territory",
                "Affinity grows through proximity, talk, trade and gifts and falls through theft and fights. \
                 People linked by **mutual** affinity at or above the bond threshold form connected components; \
                 components of three or more are groups, which persist, split and merge.\n\n\
                 Each day every finished building of a group projects influence \
                 `4·w·(1 + 0.25·ln members) − falloff·distance` (hut 1, store 1.6, farm 0.6); a tile belongs to \
                 the strongest positive influence and is contested when a rival comes within 1. Members confront \
                 people gathering on their land.",
            )
            .with_action("territory", "Show the territory overlay"),
            TutorialPage::new(
                "Simple example: abundance",
                "On a rich, mixed landscape settlers rarely go hungry: the population grows (births stop at the \
                 population cap), huts and farm plots spread over grassland and riverbanks, and groups form \
                 around shared neighbourhoods. Compare it with *Food scarcity*, which differs only in its \
                 resources.\n\n\
                 Try the **Disaster** tool (5) on a settlement and watch people rebuild: the interventions are \
                 undoable with Ctrl/Cmd+Z.",
            )
            .with_action("load:abundant", "Load Abundant resources"),
            TutorialPage::new(
                "Advanced example: cooperation vs competition",
                "Two populations with different preference distributions share one moderately rich valley. \
                 Cooperators (high cooperation, low risk) bond quickly, share food with hungry group members and \
                 pool surplus in group stores; competitors (low cooperation, high risk) rarely bond and turn to \
                 theft when hungry. Compare the groups' sizes and territories, the \
                 conflicts and thefts counters (Metrics tab) and the wealth Gini over a few hundred days.",
            )
            .with_action("load:cooperation_vs_competition", "Load Cooperation vs competition"),
            TutorialPage::new(
                "Why nothing is scripted",
                "The model never says \"build a road here\", \"form a village\" or \"become a farmer\". It only \
                 updates individuals by local rules: needs, utility scores, movement costs, barter that both \
                 sides accept, affinity changes. Roads, markets, groups, territories, inequality and the \
                 division of labour are **macroscopic patterns** of those micro-decisions — change a rule \
                 parameter (Parameters tab) and different patterns appear from the same individuals.",
            )
            .with_action("metrics", "Open the Metrics tab"),
        ],
        concepts: CONCEPTS.into(),
    }
}

const CONCEPTS: &str = "## Reading the view\n\
- **Tunics** show a person's group colour (grey without a group); children are smaller.\n\
- The **bundle on the back** is the bulk good carried most (basket of food, logs, stones); an axe on the hip means tools.\n\
- **Badges** above heads show the activity of the last tick.\n\
- People who share one spot in the model (a field, a shore) are drawn up to 0.2 tiles apart so that each stays visible.\n\
- **Trails** are faint; tiles above the road threshold are drawn as wider, lighter roads.\n\
- **Territory** washes claimed land in the group colour (lakes stay clear) and outlines it where the owner changes; contested tiles are tinted more strongly.\n\
- Trees thin to saplings when forests are logged, bushes lose their berries and rocks shrink as stocks fall; they recover as stocks regrow.\n\n\
## Metrics\n\
| Metric | Meaning |\n\
|---|---|\n\
| Population | Living people. Births need fertile adults with energy ≥ 0.6, 3 food each and mutual affinity ≥ 0.2. |\n\
| Groups | Connected components (≥ 3 people) of mutual affinity at or above the bond threshold. |\n\
| Wealth Gini | Inequality of carried food + wood + stone + 2 × tools plus food in owned huts: 0 equal, 1 one person owns all. |\n\
| Trade volume | Units of goods that changed hands in barter during the tick (both directions). |\n\
| Specialisation | Mean over adults of 1 − normalised Shannon entropy of production shares and skills. |\n\
| Territory per group | Mean number of tiles claimed per group. |\n\
| Road length | Tiles whose trail intensity reaches the road threshold. |\n\n\
## The trade network\n\
The model counts successful barter meetings but does not publish who met whom. The view attributes meetings to \
pairs from the rise of the partners' learned selling stocks, who initiated the meeting and how the goods flows \
match, never attributing more meetings than the model reported; the caption shows the share attributed. In \
*Groups* mode node size grows with members, links with meetings between groups and rings with meetings inside a \
group; *People* mode lays traders out by force (click a node to inspect).\n\n\
## Suggested experiments\n\
1. **Barter off.** In *Spontaneous trade routes*, set *Barter* off (Parameters) and run 30 days: specialisation \
falls and people must walk to both regions.\n\
2. **Scarcity.** Load *Food scarcity*: watch thefts, conflicts and starvation deaths, and whether farming takes hold.\n\
3. **Trail memory.** Raise *Trail decay time* (Roads group) and compare road length: longer memory, stronger roads.\n\
4. **Bonding.** Lower the *Bond threshold*: groups form earlier and become larger; territories merge.\n\
5. **Regrowth.** Set berry regeneration to 0 with the Regeneration sliders and observe migration towards fish and farms.\n\
6. **Disaster.** Destroy a settlement's store: territory is recomputed at once and the group must rebuild.\n";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_page_action_is_understood() {
        let t = tutorial();
        assert!(t.pages.len() >= 8);
        for page in &t.pages {
            if let Some(action) = &page.action {
                assert!(parse_action(&action.id).is_some(), "{}", action.id);
            }
        }
        assert_eq!(parse_action("run:24"), Some(TutorialStep::Run(24)));
        assert_eq!(parse_action("barter:on"), Some(TutorialStep::Barter(true)));
        assert_eq!(parse_action("load:"), None);
        assert_eq!(parse_action("nonsense"), None);
    }

    #[test]
    fn loaded_scenarios_exist() {
        let ids: Vec<String> = civilization::scenarios::builtin().unwrap().into_iter().map(|s| s.id).collect();
        for page in tutorial().pages {
            if let Some(TutorialStep::Load(id)) = page.action.and_then(|a| parse_action(&a.id)) {
                assert!(ids.contains(&id), "{id}");
            }
        }
    }

    #[test]
    fn concepts_cover_the_metrics_shown() {
        for metric in ["Population", "Groups", "Wealth Gini", "Trade volume", "Specialisation", "Territory per group"] {
            assert!(CONCEPTS.contains(metric), "{metric}");
        }
    }
}
