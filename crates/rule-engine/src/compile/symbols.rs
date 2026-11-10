//! Symbol table: the names visible to rules, resolved to indices.

use crate::compile::ir::PropRef;
use crate::def::{LifeSystem, ReproductionMode};

/// Names visible to rules, resolved to indices.
#[derive(Clone, Debug)]
pub(crate) struct Symbols {
    pub types: Vec<String>,
    pub states: Vec<Vec<String>>,
    pub state_ids: Vec<Vec<u32>>,
    pub state_names: Vec<String>,
    pub props: Vec<Vec<String>>,
    pub carried: Vec<Vec<u16>>,
    pub resources: Vec<String>,
    pub env: Vec<String>,
    pub mobile: Vec<bool>,
    pub reproduction: Vec<ReproductionMode>,
    pub perception: Vec<f32>,
}

impl Symbols {
    /// Builds the symbol table (definitions must already be valid).
    pub fn new(sys: &LifeSystem) -> Self {
        let resources: Vec<String> = sys.world.resources.iter().map(|r| r.name.clone()).collect();
        let mut state_names: Vec<String> = Vec::new();
        let mut states = Vec::new();
        let mut state_ids = Vec::new();
        for t in &sys.entity_types {
            let local = t.state_names();
            let ids = local
                .iter()
                .map(|s| match state_names.iter().position(|g| g == s) {
                    Some(i) => i as u32,
                    None => {
                        state_names.push(s.clone());
                        (state_names.len() - 1) as u32
                    }
                })
                .collect();
            states.push(local);
            state_ids.push(ids);
        }
        Self {
            types: sys.entity_types.iter().map(|t| t.name.clone()).collect(),
            states,
            state_ids,
            state_names,
            props: sys.entity_types.iter().map(|t| t.properties.iter().map(|p| p.name.clone()).collect()).collect(),
            carried: sys
                .entity_types
                .iter()
                .map(|t| {
                    t.carries
                        .iter()
                        .filter_map(|c| resources.iter().position(|r| *r == c.resource).map(|i| i as u16))
                        .collect()
                })
                .collect(),
            resources,
            env: sys.world.environment.iter().map(|e| e.name.clone()).collect(),
            mobile: sys.entity_types.iter().map(|t| t.movement.is_mobile()).collect(),
            reproduction: sys.entity_types.iter().map(|t| t.reproduction.mode).collect(),
            perception: sys.entity_types.iter().map(|t| t.perception).collect(),
        }
    }

    pub fn type_idx(&self, name: &str) -> Option<u16> {
        self.types.iter().position(|t| t == name).map(|i| i as u16)
    }

    pub fn resource_idx(&self, name: &str) -> Option<u16> {
        self.resources.iter().position(|t| t == name).map(|i| i as u16)
    }

    pub fn env_idx(&self, name: &str) -> Option<u16> {
        self.env.iter().position(|t| t == name).map(|i| i as u16)
    }

    pub fn local_state(&self, ty: u16, name: &str) -> Option<u16> {
        self.states[usize::from(ty)].iter().position(|s| s == name).map(|i| i as u16)
    }

    /// Property of a type by name (user properties, then carried resources).
    pub fn prop_of(&self, ty: u16, name: &str) -> Option<PropRef> {
        if let Some(p) = PropRef::builtin(name) {
            return Some(p);
        }
        let t = usize::from(ty);
        if let Some(i) = self.props[t].iter().position(|p| p == name) {
            return Some(PropRef::User(i as u16));
        }
        self.carried[t].iter().position(|r| self.resources[usize::from(*r)] == name).map(|i| PropRef::Carried(i as u16))
    }

    pub fn prop_names(&self, ty: u16) -> Vec<&str> {
        let t = usize::from(ty);
        let mut names: Vec<&str> = PropRef::BUILTIN.iter().map(|(n, _)| *n).collect();
        names.extend(self.props[t].iter().map(String::as_str));
        names.extend(self.carried[t].iter().map(|r| self.resources[usize::from(*r)].as_str()));
        names
    }
}
