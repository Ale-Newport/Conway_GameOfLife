//! What an organism is visibly doing, and the biomechanics used to animate it.
//!
//! [`OrganismAction`] is the vocabulary shared by simulations and renderers.
//! The renderer maps each action to an authored glTF clip when the model has
//! one, or to procedural motion otherwise (see [`crate::library`]).
//!
//! Locomotion is matched to the simulated ground speed with dynamic
//! similarity (Alexander 1976; Alexander & Jayes 1983): the Froude number
//! `Fr = v² / (g·h)` (with hip height `h`) selects the gait, and relative
//! stride length follows `λ/h = 2.3·Fr^0.3`, so stride frequency is `v/λ`.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Standard gravity used for Froude numbers, m/s².
pub const GRAVITY: f32 = 9.81;

/// Froude number below which quadrupeds walk (Alexander & Jayes 1983).
pub const FROUDE_WALK_TROT: f32 = 0.5;
/// Froude number above which cursorial quadrupeds gallop (Alexander & Jayes 1983).
pub const FROUDE_TROT_GALLOP: f32 = 2.5;
/// Froude number used as the default speed of a walking animal when the
/// simulation does not provide a speed.
pub const FROUDE_DEFAULT_WALK: f32 = 0.25;
/// Froude number used as the default speed of a running animal when the
/// simulation does not provide a speed.
pub const FROUDE_DEFAULT_RUN: f32 = 3.0;

/// Visible behaviour of an organism.
///
/// `Flee` is rendered exactly like `Run`; `Rest` covers resting and sleeping
/// (the parser accepts `"sleep"`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrganismAction {
    /// Standing still, breathing.
    #[default]
    Idle,
    /// Slow locomotion.
    Walk,
    /// Fast locomotion.
    Run,
    /// Feeding (grazing, browsing, tearing, pecking).
    Eat,
    /// Attacking (bite, kick, lunge, strike, sting).
    Attack,
    /// Escaping; rendered as [`OrganismAction::Run`].
    Flee,
    /// Resting or sleeping (lying down where the body plan allows).
    Rest,
    /// Drinking at water (head down to the ground).
    Drink,
    /// Dying / dead.
    Death,
    /// Powered flight.
    Fly,
    /// Soaring or gliding flight with spread, still wings.
    Glide,
    /// Swimming.
    Swim,
    /// A single jump or hop.
    Jump,
}

/// Error returned when parsing an unknown action name.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error(
    "unknown organism action `{0}` (expected one of: idle, walk, run, eat, attack, flee, rest, sleep, drink, death, fly, glide, swim, jump)"
)]
pub struct ActionParseError(pub String);

impl OrganismAction {
    /// Every action, in declaration order.
    pub const ALL: [OrganismAction; 13] = [
        Self::Idle,
        Self::Walk,
        Self::Run,
        Self::Eat,
        Self::Attack,
        Self::Flee,
        Self::Rest,
        Self::Drink,
        Self::Death,
        Self::Fly,
        Self::Glide,
        Self::Swim,
        Self::Jump,
    ];

    /// Lower-case name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Walk => "walk",
            Self::Run => "run",
            Self::Eat => "eat",
            Self::Attack => "attack",
            Self::Flee => "flee",
            Self::Rest => "rest",
            Self::Drink => "drink",
            Self::Death => "death",
            Self::Fly => "fly",
            Self::Glide => "glide",
            Self::Swim => "swim",
            Self::Jump => "jump",
        }
    }

    /// The action this one is rendered as: `Flee` is drawn as `Run`; every
    /// other action is drawn as itself.
    pub fn canonical(self) -> Self {
        match self {
            Self::Flee => Self::Run,
            other => other,
        }
    }

    /// Key of the clip in a manifest `animation_set` that natively represents
    /// this action, if the manifest vocabulary has one. `Drink` and `Glide`
    /// have no key of their own (they reuse `eat` / `fly` through the
    /// fallback rules in [`crate::library`]).
    pub fn manifest_key(self) -> Option<&'static str> {
        match self.canonical() {
            Self::Idle => Some("idle"),
            Self::Walk => Some("walk"),
            Self::Run => Some("run"),
            Self::Eat => Some("eat"),
            Self::Attack => Some("attack"),
            Self::Death => Some("death"),
            Self::Fly => Some("fly"),
            Self::Swim => Some("swim"),
            Self::Jump => Some("jump"),
            Self::Rest => Some("rest"),
            Self::Drink | Self::Glide | Self::Flee => None,
        }
    }

    /// Whether the action moves the organism through the world, so that the
    /// animation rate must follow the movement speed.
    pub fn is_locomotion(self) -> bool {
        matches!(self.canonical(), Self::Walk | Self::Run | Self::Swim | Self::Fly)
    }

    /// Whether the action is a one-shot that should not loop (the last
    /// frame is held).
    pub fn is_one_shot(self) -> bool {
        matches!(self, Self::Death)
    }
}

impl fmt::Display for OrganismAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for OrganismAction {
    type Err = ActionParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let lower = s.trim().to_ascii_lowercase();
        if lower == "sleep" {
            return Ok(Self::Rest);
        }
        Self::ALL.into_iter().find(|a| a.name() == lower).ok_or(ActionParseError(s.to_string()))
    }
}

/// Manifest action keys accepted in `animation_set`, `anim_speed`,
/// `supported_actions` and `missing_actions`.
pub const MANIFEST_ACTION_KEYS: [&str; 10] =
    ["idle", "walk", "run", "eat", "attack", "death", "fly", "swim", "jump", "rest"];

/// Froude number `v² / (g·h)` of a movement at speed `v` (m/s) for hip
/// height `h` (m). Returns 0 for non-positive heights.
pub fn froude_number(speed: f32, hip_height: f32) -> f32 {
    if hip_height <= 0.0 {
        return 0.0;
    }
    speed * speed / (GRAVITY * hip_height)
}

/// Speed (m/s) at which an animal of hip height `h` moves at Froude number `fr`.
pub fn speed_for_froude(fr: f32, hip_height: f32) -> f32 {
    (fr.max(0.0) * GRAVITY * hip_height.max(0.0)).sqrt()
}

/// Stride length (m) predicted by Alexander's dynamic-similarity relation
/// `λ = 2.3·h·Fr^0.3` (Alexander 1976, *Nature* 261:129).
pub fn stride_length(speed: f32, hip_height: f32) -> f32 {
    let fr = froude_number(speed, hip_height);
    2.3 * hip_height * fr.powf(0.3)
}

/// Stride frequency (strides per second) `v / λ` for legged locomotion.
/// Returns 0 when the animal is not moving.
pub fn stride_frequency(speed: f32, hip_height: f32) -> f32 {
    let lambda = stride_length(speed, hip_height);
    if speed <= 0.0 || lambda <= f32::EPSILON { 0.0 } else { speed / lambda }
}

/// Tail-beat frequency (Hz) of a fish of body length `l` swimming at `u`
/// m/s for steady carangiform swimming. Bainbridge (1958, *J. Exp. Biol.*
/// 35:109) measured `U = L·(3f − 4)/4` for dace, trout and goldfish above
/// about 5 Hz, i.e. a stride of ≈ 0.6–0.75 body lengths per beat; we use
/// the constant-stride approximation `U ≈ 0.7·L·f`, which stays valid at
/// the low beat frequencies the formula's offset would make negative.
pub fn tailbeat_frequency(speed: f32, body_length: f32) -> f32 {
    if speed <= 0.0 || body_length <= 0.0 { 0.0 } else { speed / (0.7 * body_length) }
}

/// Resting breathing frequency (Hz) for body mass `m` kg from Stahl's
/// allometry, respiratory rate `≈ 53.5·M^-0.26` breaths per minute (Stahl
/// 1967, *J. Appl. Physiol.* 22:453).
pub fn breathing_frequency(mass_kg: f32) -> f32 {
    53.5 * mass_kg.max(1e-5).powf(-0.26) / 60.0
}

/// Wing-beat frequency (Hz) of a flapping bird from Pennycuick's
/// dimensional relation `f = m^(3/8) g^(1/2) b^(-23/24) S^(-1/3) ρ^(-3/8)`
/// (Pennycuick 2008, *Modelling the Flying Bird*), with mass `m` (kg), span
/// `b` (m), wing area `S` (m²) and sea-level air density.
pub fn bird_wingbeat_frequency(mass_kg: f32, span_m: f32, wing_area_m2: f32) -> f32 {
    const RHO: f32 = 1.225;
    if mass_kg <= 0.0 || span_m <= 0.0 || wing_area_m2 <= 0.0 {
        return 0.0;
    }
    mass_kg.powf(3.0 / 8.0)
        * GRAVITY.sqrt()
        * span_m.powf(-23.0 / 24.0)
        * wing_area_m2.powf(-1.0 / 3.0)
        * RHO.powf(-3.0 / 8.0)
}

/// Locomotor gait chosen from the Froude number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gait {
    /// Lateral-sequence walk, duty factor > 0.5.
    Walk,
    /// Diagonal-pair trot.
    Trot,
    /// Asymmetric gallop (transverse or rotary) or bound.
    Gallop,
}

/// Selects the gait for a Froude number using the Alexander & Jayes (1983)
/// transitions (walk → trot at Fr ≈ 0.5, trot → gallop at Fr ≈ 2.5).
pub fn gait_for_froude(fr: f32) -> Gait {
    if fr < FROUDE_WALK_TROT {
        Gait::Walk
    } else if fr < FROUDE_TROT_GALLOP {
        Gait::Trot
    } else {
        Gait::Gallop
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_round_trips_and_accepts_sleep() {
        for a in OrganismAction::ALL {
            assert_eq!(a.name().parse::<OrganismAction>(), Ok(a));
        }
        assert_eq!("Sleep".parse::<OrganismAction>(), Ok(OrganismAction::Rest));
        let err = "dance".parse::<OrganismAction>().unwrap_err();
        assert!(err.to_string().contains("dance"));
    }

    #[test]
    fn flee_is_rendered_as_run() {
        assert_eq!(OrganismAction::Flee.canonical(), OrganismAction::Run);
        assert_eq!(OrganismAction::Flee.manifest_key(), Some("run"));
        assert!(OrganismAction::Flee.is_locomotion());
        assert!(!OrganismAction::Eat.is_locomotion());
    }

    #[test]
    fn froude_gait_transitions() {
        // A horse (hip height ~1 m) walks at 1.5 m/s, trots at 3.5, gallops at 6.
        assert_eq!(gait_for_froude(froude_number(1.5, 1.0)), Gait::Walk);
        assert_eq!(gait_for_froude(froude_number(3.5, 1.0)), Gait::Trot);
        assert_eq!(gait_for_froude(froude_number(6.0, 1.0)), Gait::Gallop);
        let v = speed_for_froude(0.5, 1.0);
        assert!((froude_number(v, 1.0) - 0.5).abs() < 1e-5);
    }

    #[test]
    fn stride_scaling_follows_alexander() {
        // λ/h = 2.3 Fr^0.3: at Fr = 1, λ = 2.3 h.
        let h = 0.8;
        let v = speed_for_froude(1.0, h);
        assert!((stride_length(v, h) - 2.3 * h).abs() < 1e-4);
        // Faster animals take longer and more frequent strides.
        assert!(stride_length(4.0, h) > stride_length(2.0, h));
        assert!(stride_frequency(4.0, h) > stride_frequency(2.0, h));
        // Smaller animals step faster at the same speed.
        assert!(stride_frequency(1.0, 0.2) > stride_frequency(1.0, 1.0));
        assert_eq!(stride_frequency(0.0, h), 0.0);
    }

    #[test]
    fn physiological_frequencies_are_plausible() {
        // Human-sized mammal breathes ~15-20 /min, mouse ~150 /min.
        let human = breathing_frequency(70.0) * 60.0;
        let mouse = breathing_frequency(0.02) * 60.0;
        assert!((12.0..20.0).contains(&human), "{human}");
        assert!((120.0..180.0).contains(&mouse), "{mouse}");
        // House sparrow (30 g, 0.24 m span, 0.0094 m²) flaps ~10-16 Hz;
        // a 4 kg eagle with 2 m span flaps ~2-4 Hz.
        let sparrow = bird_wingbeat_frequency(0.03, 0.24, 0.0094);
        let eagle = bird_wingbeat_frequency(4.5, 2.0, 0.65);
        assert!((10.0..17.0).contains(&sparrow), "{sparrow}");
        assert!((1.5..4.0).contains(&eagle), "{eagle}");
        // Bainbridge: a 0.3 m trout at 0.42 m/s beats its tail at 2 Hz.
        assert!((tailbeat_frequency(0.42, 0.3) - 2.0).abs() < 1e-4);
    }
}
