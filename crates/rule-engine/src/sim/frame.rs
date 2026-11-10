//! The frozen world state read by the decision phase.
//!
//! A [`Frame`] is built at the start of every tick. It shares the entity
//! store and the resource fields with the simulation through `Arc`s (no
//! copy) and owns a freshly built spatial index. Every sensor — DSL and
//! Rhai alike — reads only the frame, so decisions are pure functions of
//! the state at the start of the tick and can run in parallel; the
//! simulation regains unique ownership of the store when the frame is
//! dropped after the decision phase.

use std::sync::Arc;

use organism_engine::{AgentId, AgentStore};
use simulation_core::glam::Vec2;
use simulation_core::{Grid2, SpatialGrid};

use crate::compile::CompiledSystem;
use crate::compile::ir::{FieldRef, KindSel, PropRef};
use crate::def::{Axis, EnvPattern, EnvironmentField};
use crate::sim::entity::Entity;
use crate::sim::space::Space;

/// An entity found by a sensor: its slot (index in id order), the position
/// of its image nearest to the observer and the distance to it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target {
    /// Index of the entity in the frame (id order).
    pub slot: u32,
    /// Nearest image of its position, in the observer's frame.
    pub pos: Vec2,
    /// Distance from the observer.
    pub dist: f32,
}

/// Read-only world state for one tick's decisions.
pub struct Frame {
    /// Tick being decided.
    pub tick: u64,
    /// Scenario seed (keys the per-decision random streams).
    pub seed: u64,
    /// Compiled rules and types.
    pub compiled: Arc<CompiledSystem>,
    /// Geometry.
    pub space: Space,
    /// Spatial index over entity positions (slot = store index).
    pub grid: SpatialGrid,
    /// Entities.
    pub store: Arc<AgentStore<Entity>>,
    /// Resource fields.
    pub fields: Arc<Vec<Grid2<f32>>>,
    /// Population per type at the start of the tick.
    pub population: Vec<u32>,
}

impl Frame {
    /// Entity in `slot`.
    pub fn agent(&self, slot: usize) -> &Entity {
        &self.store.as_slice()[slot]
    }

    /// Id of the entity in `slot`.
    pub fn id(&self, slot: usize) -> AgentId {
        self.store.ids()[slot]
    }

    /// Number of entities.
    pub fn len(&self) -> usize {
        self.store.len()
    }

    /// Property of the entity in `slot`.
    pub fn prop(&self, slot: usize, p: PropRef) -> f64 {
        self.agent(slot).prop(p, self.id(slot), self.tick)
    }

    /// Whether the entity in `other` matches `kind` from `me`'s point of view.
    pub fn matches(&self, me: usize, other: usize, kind: KindSel) -> bool {
        match kind {
            KindSel::Any => true,
            KindSel::Same => self.agent(other).kind == self.agent(me).kind,
            KindSel::Type(t) => self.agent(other).kind == t,
        }
    }

    /// Number of matching entities within `radius` of `me` (excluding `me`).
    pub fn nearby(&self, me: usize, kind: KindSel, radius: f32) -> usize {
        let origin = self.agent(me).kin.position;
        self.space.count(&self.grid, origin, radius, |s, _| s as usize != me && self.matches(me, s as usize, kind))
    }

    /// Nearest matching entity within `radius` of `me` (excluding `me`).
    pub fn nearest(&self, me: usize, kind: KindSel, radius: f32) -> Option<Target> {
        let origin = self.agent(me).kin.position;
        self.space
            .nearest(&self.grid, origin, radius, |s, _| s as usize != me && self.matches(me, s as usize, kind))
            .map(|p| Target { slot: p.slot, pos: p.position, dist: p.distance })
    }

    /// Distance at which two entities touch.
    pub fn contact_distance(&self, a: usize, b: usize) -> f32 {
        0.5 * (self.agent(a).size + self.agent(b).size)
    }

    /// Nearest entity touching `me` (optionally of type `with`).
    pub fn contact(&self, me: usize, with: Option<u16>) -> Option<Target> {
        let origin = self.agent(me).kin.position;
        let reach = 0.5 * (self.agent(me).size + self.compiled.max_size());
        self.space
            .nearest(&self.grid, origin, reach, |s, pos| {
                let s = s as usize;
                s != me
                    && with.is_none_or(|t| self.agent(s).kind == t)
                    && pos.distance(origin) <= self.contact_distance(me, s)
            })
            .map(|p| Target { slot: p.slot, pos: p.position, dist: p.distance })
    }

    /// Value of a field at a position.
    pub fn field_at(&self, field: FieldRef, pos: Vec2) -> f64 {
        match field {
            FieldRef::Resource(r) => {
                let (x, y) = self.space.cell(pos);
                self.fields.get(usize::from(r)).map_or(0.0, |g| f64::from(*g.get(x, y)))
            }
            FieldRef::Env(e) => self
                .compiled
                .source()
                .world
                .environment
                .get(usize::from(e))
                .map_or(0.0, |def| env_value(def, pos, self.tick, &self.space)),
        }
    }

    /// Population matching `kind` from `me`'s point of view.
    pub fn population_of(&self, me: usize, kind: KindSel) -> f64 {
        let n = match kind {
            KindSel::Any => self.population.iter().sum::<u32>(),
            KindSel::Same => self.population.get(usize::from(self.agent(me).kind)).copied().unwrap_or(0),
            KindSel::Type(t) => self.population.get(usize::from(t)).copied().unwrap_or(0),
        };
        f64::from(n)
    }
}

/// Value of an environment field at `pos` and `tick`.
pub fn env_value(def: &EnvironmentField, pos: Vec2, tick: u64, space: &Space) -> f64 {
    let base = match &def.pattern {
        EnvPattern::Constant(v) => *v,
        EnvPattern::Gradient { from, to, axis } => {
            let t = match axis {
                Axis::X => f64::from(pos.x / space.width),
                Axis::Y => f64::from(pos.y / space.height),
            };
            from + (to - from) * t.clamp(0.0, 1.0)
        }
        EnvPattern::Radial { x, y, radius, peak, edge } => {
            let d = f64::from(pos.distance(Vec2::new(*x as f32, *y as f32)));
            if d >= *radius { *edge } else { peak + (edge - peak) * d / radius }
        }
    };
    match &def.seasonal {
        Some(s) => base * (1.0 + s.amplitude * (std::f64::consts::TAU * tick as f64 / s.period).sin()),
        None => base,
    }
}
