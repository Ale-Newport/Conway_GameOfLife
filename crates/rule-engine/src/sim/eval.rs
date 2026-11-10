//! Evaluation of the runtime IR against a [`Frame`].
//!
//! Evaluation is total: numeric operations follow IEEE semantics except
//! that division by zero yields 0, comparisons involving NaN are false
//! (`!=` true), and entity expressions that find nothing yield `None`
//! (`distance` is then infinite and `exists` false).

use simulation_core::SimRng;

use crate::compile::ir::{ArithOp, RExpr, Subject};
use crate::sim::frame::{Frame, Target};

/// Evaluation context of one rule for one entity.
pub struct EvalCx<'a> {
    /// Frozen world.
    pub frame: &'a Frame,
    /// Acting entity's slot.
    pub me: usize,
    /// The touched entity of a collision rule.
    pub other: Option<Target>,
    /// Keyed random stream of this (tick, entity, rule).
    pub rng: SimRng,
}

impl EvalCx<'_> {
    fn subject_slot(&self, s: Subject) -> Option<usize> {
        match s {
            Subject::Me => Some(self.me),
            Subject::Other => self.other.map(|t| t.slot as usize),
        }
    }

    /// Numeric value.
    pub fn num(&mut self, e: &RExpr) -> f64 {
        match e {
            RExpr::Num(v) => *v,
            RExpr::Prop(s, p) => self.subject_slot(*s).map_or(0.0, |slot| self.frame.prop(slot, *p)),
            RExpr::Tick => self.frame.tick as f64,
            RExpr::Nearby(kind, r) => self.frame.nearby(self.me, *kind, *r) as f64,
            RExpr::Population(kind) => self.frame.population_of(self.me, *kind),
            RExpr::Field(f) => self.frame.field_at(*f, self.frame.agent(self.me).kin.position),
            RExpr::Random => self.rng.f64(),
            RExpr::Distance(t) => self.entity(t).map_or(f64::INFINITY, |t| f64::from(t.dist)),
            RExpr::Min(a, b) => self.num(a).min(self.num(b)),
            RExpr::Max(a, b) => self.num(a).max(self.num(b)),
            RExpr::Abs(a) => self.num(a).abs(),
            RExpr::Clamp(x, lo, hi) => {
                let (x, lo, hi) = (self.num(x), self.num(lo), self.num(hi));
                if lo <= hi { x.clamp(lo, hi) } else { x }
            }
            RExpr::Neg(a) => -self.num(a),
            RExpr::Arith(op, a, b) => {
                let (a, b) = (self.num(a), self.num(b));
                match op {
                    ArithOp::Add => a + b,
                    ArithOp::Sub => a - b,
                    ArithOp::Mul => a * b,
                    ArithOp::Div if b == 0.0 => 0.0,
                    ArithOp::Div => a / b,
                }
            }
            // Boolean or entity expressions in numeric position cannot pass
            // validation; treat them as 0 rather than panicking.
            _ => 0.0,
        }
    }

    /// Boolean value.
    pub fn boolean(&mut self, e: &RExpr) -> bool {
        match e {
            RExpr::Bool(b) => *b,
            RExpr::StateIs(s, name, negate) => {
                let is = self.subject_slot(*s).is_some_and(|slot| {
                    let a = self.frame.agent(slot);
                    let ty = &self.frame.compiled.types()[usize::from(a.kind)];
                    ty.state_ids.get(usize::from(a.state)) == Some(name)
                });
                is != *negate
            }
            RExpr::TypeIs(s, t, negate) => {
                let is = self.subject_slot(*s).is_some_and(|slot| self.frame.agent(slot).kind == *t);
                is != *negate
            }
            RExpr::Chance(p) => {
                let p = self.num(p);
                self.rng.chance(p)
            }
            RExpr::Exists(t) => self.entity(t).is_some(),
            RExpr::Not(a) => !self.boolean(a),
            RExpr::Cmp(op, a, b) => {
                let (a, b) = (self.num(a), self.num(b));
                op.apply(a, b)
            }
            RExpr::BoolEq(a, b, negate) => (self.boolean(a) == self.boolean(b)) != *negate,
            RExpr::And(a, b) => self.boolean(a) && self.boolean(b),
            RExpr::Or(a, b) => self.boolean(a) || self.boolean(b),
            _ => false,
        }
    }

    /// Entity value.
    pub fn entity(&mut self, e: &RExpr) -> Option<Target> {
        match e {
            RExpr::SelfEntity => {
                Some(Target { slot: self.me as u32, pos: self.frame.agent(self.me).kin.position, dist: 0.0 })
            }
            RExpr::OtherEntity => self.other,
            RExpr::Nearest(kind, r) => self.frame.nearest(self.me, *kind, *r),
            _ => None,
        }
    }
}
