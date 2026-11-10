//! Compilation of a [`LifeSystem`]: definition validation, relationship
//! compilation, rule validation/lowering and script compilation.
//!
//! [`compile`] either returns a [`CompiledSystem`] (with warnings) or every
//! error found, each with a code, the rule or definition it belongs to and
//! a line:column position inside that rule's text.

pub(crate) mod defs;
pub mod ir;
pub(crate) mod lower;
pub(crate) mod relations;
pub(crate) mod rules;
pub(crate) mod symbols;
pub mod vocab;

use serde::{Deserialize, Serialize};

use crate::def::{EntityType, LifeSystem, MovementDef, RelationKind};
use crate::dsl::ast::RuleAst;
use crate::error::{Diagnostic, ItemRef, RuleEngineError, item_index};
use crate::script::ScriptHost;
use ir::{CompiledRule, RuleOrigin};
use rules::{check_manual_rule, lower_rule};
use symbols::Symbols;

pub use defs::{MAX_ENTITIES_CEILING, MAX_LIST_LEN, MAX_TYPES, MAX_WORLD_SIDE, MIN_WORLD_SIDE};

/// An entity type after compilation.
#[derive(Clone, Debug, PartialEq)]
pub struct CompiledType {
    /// The definition.
    pub def: EntityType,
    /// State names (implicit `default` filled in).
    pub states: Vec<String>,
    /// Global state-name id of each state.
    pub state_ids: Vec<u32>,
    /// Resource field index of each carried slot.
    pub carried: Vec<u16>,
    /// Types this type may mate with (itself and `reproduces_with` objects).
    pub mates: Vec<u16>,
    /// Indices into [`CompiledSystem::rules`], sorted by priority
    /// (descending) then declaration order.
    pub rules: Vec<u32>,
    /// Indices into the script list, sorted the same way.
    pub scripts: Vec<u32>,
}

impl CompiledType {
    /// Turn used by `wander()` without argument and by the default movement.
    pub fn default_turn(&self) -> f32 {
        match self.def.movement {
            MovementDef::None => 0.0,
            MovementDef::RandomWalk { .. } => std::f32::consts::PI,
            MovementDef::Directed { turn, .. } => turn,
        }
    }
}

/// Node of the relationship graph.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GraphNode {
    /// Entity type or resource name.
    pub name: String,
    /// Whether the node is a resource field (otherwise an entity type).
    pub resource: bool,
    /// Colour.
    pub color: [f32; 4],
    /// Glyph (entity types).
    pub icon: String,
    /// Trophic level from the `consumes` edges: 1 for resources and for
    /// types that consume nothing, otherwise 1 + the mean level of what the
    /// node consumes (capped at the number of nodes when feeding loops).
    /// Suggested use: node colour or vertical position.
    pub trophic_level: f64,
}

/// Typed edge of the relationship graph.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GraphEdge {
    /// Source node index.
    pub from: usize,
    /// Target node index.
    pub to: usize,
    /// Relation label (`consumes`, `avoids`...).
    pub relation: String,
    /// Index of the relationship in the definition.
    pub relationship: usize,
    /// Names of the rules generated from it.
    pub rules: Vec<String>,
    /// Parameters as short text (see [`RelationKind::summary`]).
    pub summary: String,
    /// Whether the relation acts both ways (`competes_with`).
    pub symmetric: bool,
}

/// Relationship graph for the UI graph view: entity types and resources as
/// nodes (types first, in definition order, so node `i < types` is legend
/// entry `i`), one typed edge per relationship. Plain data, ready to be
/// mirrored into a graph widget (e.g. egui_graphs with a force-directed
/// layout seeded from `trophic_level`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RelationshipGraph {
    /// Nodes: all entity types, then all resources.
    pub nodes: Vec<GraphNode>,
    /// Edges in relationship order.
    pub edges: Vec<GraphEdge>,
}

/// A validated system ready to simulate.
#[derive(Debug)]
pub struct CompiledSystem {
    source: LifeSystem,
    sym: Symbols,
    types: Vec<CompiledType>,
    rules: Vec<CompiledRule>,
    scripts: ScriptHost,
    warnings: Vec<Diagnostic>,
    graph: RelationshipGraph,
    max_size: f32,
}

impl CompiledSystem {
    /// The source definition.
    pub fn source(&self) -> &LifeSystem {
        &self.source
    }

    /// Compiled entity types (definition order).
    pub fn types(&self) -> &[CompiledType] {
        &self.types
    }

    /// All rules: manual rules (definition order), then relationship rules,
    /// then rules generated from type reproduction settings.
    pub fn rules(&self) -> &[CompiledRule] {
        &self.rules
    }

    /// Compiled scripts.
    pub fn scripts(&self) -> &ScriptHost {
        &self.scripts
    }

    /// Non-blocking diagnostics.
    pub fn warnings(&self) -> &[Diagnostic] {
        &self.warnings
    }

    /// Relationship graph.
    pub fn graph(&self) -> &RelationshipGraph {
        &self.graph
    }

    /// Largest entity diameter (bounds contact queries).
    pub fn max_size(&self) -> f32 {
        self.max_size
    }

    /// Index of a type by name.
    pub fn type_index(&self, name: &str) -> Option<u16> {
        self.sym.type_idx(name)
    }

    pub(crate) fn symbols(&self) -> &Symbols {
        &self.sym
    }

    /// Every rule as DSL text, grouped by entity type in evaluation order.
    pub fn rules_text(&self) -> String {
        let mut out = String::new();
        for t in &self.types {
            for &r in &t.rules {
                let rule = &self.rules[r as usize];
                if !out.is_empty() {
                    out.push_str("\n\n");
                }
                if let RuleOrigin::Relationship { relation, .. } = &rule.origin {
                    out.push_str(&format!("# generated from a `{relation}` relationship\n"));
                } else if let RuleOrigin::EntityType { entity } = &rule.origin {
                    out.push_str(&format!("# generated from the reproduction settings of `{entity}`\n"));
                }
                out.push_str(&rule.text);
            }
        }
        out
    }
}

/// The definition item a rule's diagnostics belong to.
fn item_of(system: &LifeSystem, origin: &RuleOrigin) -> ItemRef {
    match origin {
        RuleOrigin::Manual(i) => ItemRef::Rule(item_index(*i)),
        RuleOrigin::Relationship { index, .. } => ItemRef::Relationship(item_index(*index)),
        RuleOrigin::EntityType { entity } => {
            system.type_index(entity).map_or(ItemRef::System, |t| ItemRef::EntityType(item_index(t)))
        }
    }
}

/// Validates a whole system and returns **every** diagnostic, errors and
/// warnings, each anchored to its item ([`Diagnostic::item`]) — the entry
/// point for an editor's problem list. Equivalent to [`compile`] without
/// keeping the result.
pub fn validate(system: &LifeSystem) -> Vec<Diagnostic> {
    match compile(system) {
        Ok(compiled) => compiled.warnings,
        Err(RuleEngineError::Invalid(diags)) => diags,
        Err(other) => vec![Diagnostic::error("E020", other.to_string()).for_item(ItemRef::System)],
    }
}

/// Checks one hand-written rule against the definitions of `system`
/// (entity types, states, properties, resources, fields) without compiling
/// the rest — for live feedback while a rule is edited. Returns its errors
/// and warnings (without an [`ItemRef`]; the caller knows which rule it
/// is). Definition errors of `system` are returned instead when there are
/// any, since rules cannot be checked against invalid definitions. Name
/// clashes with other rules are only reported by [`validate`].
pub fn check_rule(system: &LifeSystem, rule: &RuleAst) -> Vec<Diagnostic> {
    let defs = defs::validate_definitions(system);
    if defs.iter().any(Diagnostic::is_error) {
        return defs.into_iter().filter(Diagnostic::is_error).collect();
    }
    let sym = Symbols::new(system);
    check_manual_rule(&sym, rule, RuleOrigin::Manual(usize::MAX)).1
}

/// Parses rule text and checks it like [`check_rule`]. Returns the parsed
/// rule (when the text is syntactically valid, even if it has semantic
/// errors) and all diagnostics.
pub fn check_rule_text(system: &LifeSystem, text: &str) -> (Option<RuleAst>, Vec<Diagnostic>) {
    match crate::dsl::parse_rule(text) {
        Ok(rule) => {
            let diags = check_rule(system, &rule);
            (Some(rule), diags)
        }
        Err(diags) => (None, diags),
    }
}

/// Validates and compiles a system. On failure the error carries every
/// diagnostic (errors and warnings); on success the warnings are kept in
/// [`CompiledSystem::warnings`].
pub fn compile(system: &LifeSystem) -> Result<CompiledSystem, RuleEngineError> {
    let mut diags = defs::validate_definitions(system);
    if diags.iter().any(Diagnostic::is_error) {
        return Err(RuleEngineError::Invalid(diags));
    }
    let sym = Symbols::new(system);
    let (generated, mut rel_diags) = relations::generate(system);
    diags.append(&mut rel_diags);

    let manual = system.rules.iter().enumerate().map(|(i, r)| (r.clone(), RuleOrigin::Manual(i)));
    let mut rules = Vec::new();
    for (ast, origin) in manual.chain(generated) {
        let item = item_of(system, &origin);
        let (rule, d) = if matches!(origin, RuleOrigin::Manual(_)) {
            check_manual_rule(&sym, &ast, origin)
        } else {
            lower_rule(&sym, &ast, origin)
        };
        diags.extend(d.into_iter().map(|d| d.for_item(item)));
        if let Some(rule) = rule {
            if rules.iter().any(|r: &CompiledRule| r.entity == rule.entity && r.name == rule.name) {
                diags.push(
                    Diagnostic::error("E016", format!("duplicate rule name `{}` for `{}`", rule.name, ast.entity.text))
                        .context(format!("rule `{}` ({})", ast.name, ast.entity.text))
                        .at(ast.span)
                        .for_item(item),
                );
                continue;
            }
            rules.push(rule);
        }
    }
    let scripts = match ScriptHost::compile(system, &sym) {
        Ok(host) => Some(host),
        Err(mut d) => {
            diags.append(&mut d);
            None
        }
    };
    for (i, s) in system.scripts.iter().enumerate() {
        let context = format!("script `{}` ({})", s.name, s.entity);
        if s.name.trim().is_empty() {
            diags.push(
                Diagnostic::error("E020", "script names must not be empty")
                    .context(&context)
                    .for_item(ItemRef::Script(item_index(i))),
            );
        }
        let dup = system.scripts[..i].iter().any(|o| o.entity == s.entity && o.name == s.name)
            || rules.iter().any(|r| sym.types[usize::from(r.entity)] == s.entity && r.name == s.name);
        if dup {
            diags.push(
                Diagnostic::error("E016", format!("duplicate rule/script name `{}` for `{}`", s.name, s.entity))
                    .context(context)
                    .for_item(ItemRef::Script(item_index(i))),
            );
        }
    }
    if diags.iter().any(Diagnostic::is_error) {
        return Err(RuleEngineError::Invalid(diags));
    }
    let Some(scripts) = scripts else { return Err(RuleEngineError::Invalid(diags)) };

    let mut types: Vec<CompiledType> = system
        .entity_types
        .iter()
        .enumerate()
        .map(|(i, def)| CompiledType {
            def: def.clone(),
            states: sym.states[i].clone(),
            state_ids: sym.state_ids[i].clone(),
            carried: sym.carried[i].clone(),
            mates: std::iter::once(i as u16)
                .chain(system.relationships.iter().filter_map(|r| match r.kind {
                    RelationKind::ReproducesWith { .. } if r.subject == def.name => sym.type_idx(&r.object),
                    _ => None,
                }))
                .collect(),
            rules: Vec::new(),
            scripts: Vec::new(),
        })
        .collect();
    for (i, r) in rules.iter().enumerate() {
        types[usize::from(r.entity)].rules.push(i as u32);
    }
    for (i, s) in scripts.scripts().iter().enumerate() {
        types[usize::from(s.entity)].scripts.push(i as u32);
    }
    for t in &mut types {
        // Stable sort: equal priorities keep declaration order.
        t.rules.sort_by_key(|&r| std::cmp::Reverse(rules[r as usize].priority));
        t.scripts.sort_by_key(|&s| std::cmp::Reverse(scripts.scripts()[s as usize].priority));
    }
    let graph = build_graph(system, &rules);
    let max_size = system.entity_types.iter().map(|t| t.size).fold(0.0f32, f32::max);
    Ok(CompiledSystem { source: system.clone(), sym, types, rules, scripts, warnings: diags, graph, max_size })
}

fn build_graph(sys: &LifeSystem, rules: &[CompiledRule]) -> RelationshipGraph {
    let mut nodes: Vec<GraphNode> = sys
        .entity_types
        .iter()
        .map(|t| GraphNode {
            name: t.name.clone(),
            resource: false,
            color: t.color,
            icon: t.icon.clone(),
            trophic_level: 1.0,
        })
        .collect();
    nodes.extend(sys.world.resources.iter().map(|r| GraphNode {
        name: r.name.clone(),
        resource: true,
        color: r.color,
        icon: String::new(),
        trophic_level: 1.0,
    }));
    let index = |name: &str| nodes.iter().position(|n| n.name == name);
    // Trophic levels: Jacobi iteration of TL(i) = 1 + mean TL(food of i).
    let foods: Vec<Vec<usize>> = (0..nodes.len())
        .map(|i| {
            sys.relationships
                .iter()
                .filter(|r| matches!(r.kind, RelationKind::Consumes { .. }) && r.subject == nodes[i].name)
                .filter_map(|r| index(&r.object))
                .collect()
        })
        .collect();
    let cap = nodes.len().max(1) as f64;
    let mut level = vec![1.0f64; nodes.len()];
    for _ in 0..nodes.len() + 1 {
        let next: Vec<f64> = foods
            .iter()
            .map(|f| {
                if f.is_empty() {
                    1.0
                } else {
                    (1.0 + f.iter().map(|&j| level[j]).sum::<f64>() / f.len() as f64).min(cap)
                }
            })
            .collect();
        if next == level {
            break;
        }
        level = next;
    }
    for (n, l) in nodes.iter_mut().zip(level) {
        n.trophic_level = l;
    }
    let index = |name: &str| nodes.iter().position(|n| n.name == name);
    let edges = sys
        .relationships
        .iter()
        .enumerate()
        .filter_map(|(i, rel)| {
            Some(GraphEdge {
                from: index(&rel.subject)?,
                to: index(&rel.object)?,
                relation: rel.kind.label().to_string(),
                relationship: i,
                summary: rel.kind.summary(),
                symmetric: rel.kind.is_symmetric(),
                rules: rules
                    .iter()
                    .filter(|r| matches!(r.origin, RuleOrigin::Relationship { index, .. } if index == i))
                    .map(|r| r.name.clone())
                    .collect(),
            })
        })
        .collect();
    RelationshipGraph { nodes, edges }
}
