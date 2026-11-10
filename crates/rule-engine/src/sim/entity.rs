//! Per-entity state.

use organism_engine::{AgentId, Kinematics};
use serde::{Deserialize, Serialize};
use simulation_core::glam::Vec2;

use crate::compile::CompiledType;
use crate::compile::ir::PropRef;

/// One entity of a custom life system.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entity {
    /// Entity type index.
    pub kind: u16,
    /// Position, velocity and heading.
    pub kin: Kinematics,
    /// Energy.
    pub energy: f64,
    /// Ticks since birth.
    pub age: u64,
    /// Type-local state index.
    pub state: u16,
    /// Tick at which the current type/state was entered.
    pub state_since: u64,
    /// Diameter.
    pub size: f32,
    /// Movement speed.
    pub speed: f32,
    /// User property values (type's property order).
    pub props: Vec<f64>,
    /// Carried resource amounts (type's `carries` order).
    pub carried: Vec<f64>,
    /// Per rule of the type (in the type's sorted rule order): first tick at
    /// which the rule may fire again (`u64::MAX` after a `once` rule fired).
    pub cooldowns: Vec<u64>,
    /// First tick at which the entity may reproduce again.
    pub next_reproduction: u64,
    /// 0 for initial entities, parent generation + 1 for offspring.
    pub generation: u32,
    /// Parent (initiating parent for sexual reproduction).
    pub parent: Option<AgentId>,
    /// `on_birth` rules still have to run.
    pub birth_pending: bool,
    /// `on_state_enter` rules still have to run.
    pub state_enter_pending: bool,
}

impl Entity {
    /// A fresh entity of type `ty` (index `kind`) at `position`.
    pub(crate) fn new(kind: u16, ty: &CompiledType, position: Vec2, heading: f32, tick: u64, energy: f64) -> Self {
        Self {
            kind,
            kin: Kinematics::new(position, heading),
            energy: energy.clamp(0.0, ty.def.energy.max),
            age: 0,
            state: 0,
            state_since: tick,
            size: ty.def.size,
            speed: ty.def.movement.speed(),
            props: ty.def.properties.iter().map(|p| p.default).collect(),
            carried: ty.def.carries.iter().map(|c| c.initial).collect(),
            cooldowns: vec![0; ty.rules.len()],
            next_reproduction: 0,
            generation: 0,
            parent: None,
            birth_pending: true,
            state_enter_pending: false,
        }
    }

    /// Reads a property (`id` must be supplied by the caller's store).
    pub(crate) fn prop(&self, p: PropRef, id: AgentId, tick: u64) -> f64 {
        match p {
            PropRef::Energy => self.energy,
            PropRef::Age => self.age as f64,
            PropRef::StateAge => tick.saturating_sub(self.state_since) as f64,
            PropRef::Size => f64::from(self.size),
            PropRef::Speed => f64::from(self.speed),
            PropRef::X => f64::from(self.kin.position.x),
            PropRef::Y => f64::from(self.kin.position.y),
            PropRef::Generation => f64::from(self.generation),
            PropRef::Id => id.get() as f64,
            PropRef::User(i) => self.props.get(usize::from(i)).copied().unwrap_or(0.0),
            PropRef::Carried(i) => self.carried.get(usize::from(i)).copied().unwrap_or(0.0),
        }
    }

    /// Writes a writable property, clamping to its valid range. Non-finite
    /// values are ignored.
    pub(crate) fn set_prop(&mut self, p: PropRef, value: f64, ty: &CompiledType) {
        if !value.is_finite() {
            return;
        }
        match p {
            PropRef::Energy => self.energy = value.clamp(f64::MIN, ty.def.energy.max),
            PropRef::Size => self.size = (value as f32).clamp(0.01, 100.0),
            PropRef::Speed => self.speed = (value as f32).clamp(0.0, 50.0),
            PropRef::User(i) => {
                if let (Some(slot), Some(def)) =
                    (self.props.get_mut(usize::from(i)), ty.def.properties.get(usize::from(i)))
                {
                    *slot = value.clamp(def.min, def.max);
                }
            }
            _ => {}
        }
    }

    /// Changes the entity's type, keeping position, energy (clamped), age,
    /// same-named property values and same-named carried resources; the
    /// state becomes the new type's initial state. Returns the carried
    /// amounts the new type cannot hold (per resource field index), which
    /// the caller releases into the world.
    pub(crate) fn transform(
        &mut self,
        kind: u16,
        old: &CompiledType,
        new: &CompiledType,
        tick: u64,
    ) -> Vec<(u16, f64)> {
        let mut released = Vec::new();
        let props: Vec<f64> = new
            .def
            .properties
            .iter()
            .map(|p| {
                old.def
                    .properties
                    .iter()
                    .position(|o| o.name == p.name)
                    .and_then(|i| self.props.get(i).copied())
                    .map_or(p.default, |v| v.clamp(p.min, p.max))
            })
            .collect();
        let mut carried = vec![0.0; new.carried.len()];
        for (&res, &amount) in old.carried.iter().zip(&self.carried) {
            match new.carried.iter().position(|r| *r == res) {
                Some(j) => {
                    let kept = amount.min(new.def.carries[j].capacity);
                    carried[j] = kept;
                    if amount > kept {
                        released.push((res, amount - kept));
                    }
                }
                None if amount > 0.0 => released.push((res, amount)),
                None => {}
            }
        }
        self.kind = kind;
        self.props = props;
        self.carried = carried;
        self.energy = self.energy.min(new.def.energy.max);
        self.state = 0;
        self.state_since = tick;
        self.size = new.def.size;
        self.speed = new.def.movement.speed();
        self.cooldowns = vec![0; new.rules.len()];
        self.state_enter_pending = false;
        released
    }
}
