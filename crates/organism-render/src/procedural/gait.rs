//! Gait and cyclic-motion kinematics (pure functions, unit tested).
//!
//! * Quadruped footfall timings follow Hildebrand's gait descriptions
//!   (Hildebrand 1965, *Science* 150:701; 1977, *Am. Zool.* 17:211): the
//!   lateral-sequence walk (LH 0, LF 0.25, RH 0.5, RF 0.75), the diagonal
//!   trot, transverse (ungulate) and rotary (carnivore) gallops, the bound,
//!   and the elephant amble. Phases are fractions of a stride relative to
//!   the left hind foot touching down.
//! * Hexapods use the alternating tripod (L1-R2-L3 vs R1-L2-R3) and spiders
//!   the alternating tetrapod (L1-R2-L3-R4 vs R1-L2-R3-L4) (Wilson 1966,
//!   *Annu. Rev. Entomol.* 11:103).
//! * The swing amplitude reproduces the stride: during stance the body moves
//!   `λ·β` over the planted foot, so a leg of length `L` sweeps
//!   `2·asin(λβ / 2L)`.
//! * Undulation is a travelling wave `θ(s,t) = A(s)·sin(2π(s/λ − f t))`
//!   whose amplitude grows towards the tail (carangiform) or is nearly
//!   uniform (anguilliform), as in Lighthill's elongated-body theory.

use std::f32::consts::{PI, TAU};

use crate::action::Gait;

use super::rig::{GallopStyle, Side};

/// Fraction of the stride a foot spends on the ground.
pub fn duty_factor(gait: Gait, style: GallopStyle) -> f32 {
    match (gait, style) {
        (Gait::Walk, _) => 0.65,
        (_, GallopStyle::Amble) => 0.45,
        (Gait::Trot, _) => 0.45,
        (Gait::Gallop, GallopStyle::Bound) => 0.35,
        (Gait::Gallop, _) => 0.3,
    }
}

/// Footfall phase of a quadruped leg (`pair` 0 = fore, 1 = hind).
pub fn quadruped_phase(gait: Gait, style: GallopStyle, pair: u8, side: Side) -> f32 {
    let hind = pair >= 1;
    let left = side == Side::Left;
    match (gait, style) {
        (Gait::Walk, _) | (_, GallopStyle::Amble) => match (hind, left) {
            (true, true) => 0.0,
            (false, true) => 0.25,
            (true, false) => 0.5,
            (false, false) => 0.75,
        },
        (Gait::Trot, _) => match (hind, left) {
            (true, true) | (false, false) => 0.0,
            (true, false) | (false, true) => 0.5,
        },
        (Gait::Gallop, GallopStyle::Transverse) => match (hind, left) {
            (true, true) => 0.0,
            (true, false) => 0.12,
            (false, true) => 0.42,
            (false, false) => 0.54,
        },
        (Gait::Gallop, GallopStyle::Rotary) => match (hind, left) {
            (true, true) => 0.0,
            (true, false) => 0.1,
            (false, false) => 0.45,
            (false, true) => 0.57,
        },
        (Gait::Gallop, GallopStyle::Bound) => match (hind, left) {
            (true, true) => 0.0,
            (true, false) => 0.03,
            (false, true) => 0.5,
            (false, false) => 0.55,
        },
    }
}

/// Alternating-tripod phase of an insect leg (`pair` 0..3 from the front).
pub fn hexapod_phase(pair: u8, side: Side) -> f32 {
    let group_a = pair.is_multiple_of(2) == (side == Side::Left);
    if group_a { 0.0 } else { 0.5 }
}

/// Alternating-tetrapod phase of a spider leg (`pair` 0..4 from the front).
pub fn octopod_phase(pair: u8, side: Side) -> f32 {
    hexapod_phase(pair, side)
}

/// Leg angle and lift at a point of the stride cycle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LegPose {
    /// Protraction angle (rad): positive = foot ahead of the joint.
    pub swing: f32,
    /// Foot lift during the swing phase, 0..1.
    pub lift: f32,
    /// Whether the foot is on the ground.
    pub stance: bool,
}

/// Leg pose at `phase` (0..1, 0 = touch-down) for duty factor `duty` and
/// sweep half-amplitude `amplitude` (rad). During stance the leg retracts
/// linearly (foot fixed while the body passes over it); during swing it
/// protracts with a smooth profile and lifts the foot.
pub fn leg_pose(phase: f32, duty: f32, amplitude: f32) -> LegPose {
    let p = phase.rem_euclid(1.0);
    let duty = duty.clamp(0.05, 0.95);
    if p < duty {
        let s = p / duty;
        LegPose { swing: amplitude * (1.0 - 2.0 * s), lift: 0.0, stance: true }
    } else {
        let s = (p - duty) / (1.0 - duty);
        let smooth = s * s * (3.0 - 2.0 * s);
        LegPose { swing: amplitude * (2.0 * smooth - 1.0), lift: (PI * s).sin(), stance: false }
    }
}

/// Largest leg sweep half-angle (rad) drawn; beyond it limbs look
/// dislocated. Strides longer than the sweep allows are shortened (and the
/// stride frequency raised) by [`super::pose::stride_at`].
pub const MAX_SWEEP: f32 = 0.75;

/// Half-amplitude (rad) of the leg sweep that produces stride length
/// `stride` with duty factor `duty` for a leg whose swing lever arm is
/// `leg`, capped at [`MAX_SWEEP`].
pub fn sweep_amplitude(stride: f32, duty: f32, leg: f32) -> f32 {
    if leg <= 0.0 {
        return 0.0;
    }
    (stride * duty / (2.0 * leg)).clamp(0.0, MAX_SWEEP.sin()).asin()
}

/// Phase of a wing-beat cycle split into a downstroke lasting `down`
/// fraction of the cycle (birds ≈ 0.55-0.6). Returns the wing elevation in
/// −1 (bottom) .. +1 (top) and whether the wing is in its downstroke.
pub fn wingbeat(phase: f32, down: f32) -> (f32, bool) {
    let p = phase.rem_euclid(1.0);
    let down = down.clamp(0.2, 0.8);
    if p < down { ((PI * p / down).cos(), true) } else { (-(PI * (p - down) / (1.0 - down)).cos(), false) }
}

/// Lateral angle (rad) of body segment at arc position `s` (0 = head,
/// 1 = tail tip) for a travelling wave of `wavelength` body lengths at
/// cycle phase `phase` (cycles). `amplitude` is the tail-tip angle;
/// `uniform` blends from carangiform (0: amplitude grows ∝ s²) to
/// anguilliform (1: nearly uniform).
pub fn undulation_angle(s: f32, phase: f32, wavelength: f32, amplitude: f32, uniform: f32) -> f32 {
    let envelope = uniform * (0.3 + 0.7 * s) + (1.0 - uniform) * (0.08 + 0.92 * s * s);
    amplitude * envelope * (TAU * (s / wavelength.max(0.05) - phase)).sin()
}

/// Ballistic hop height (fraction of peak) at `phase` 0..1 of a hop whose
/// airborne part lasts `air` of the cycle: a parabola during flight, zero on
/// the ground.
pub fn hop_height(phase: f32, air: f32) -> f32 {
    let p = phase.rem_euclid(1.0);
    let air = air.clamp(0.05, 0.95);
    if p < air {
        let s = p / air;
        4.0 * s * (1.0 - s)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walk_is_lateral_sequence() {
        let s = GallopStyle::Transverse;
        let lh = quadruped_phase(Gait::Walk, s, 1, Side::Left);
        let lf = quadruped_phase(Gait::Walk, s, 0, Side::Left);
        let rh = quadruped_phase(Gait::Walk, s, 1, Side::Right);
        let rf = quadruped_phase(Gait::Walk, s, 0, Side::Right);
        // Hind foot is followed by the fore foot of the same side a quarter later.
        assert_eq!((lh, lf, rh, rf), (0.0, 0.25, 0.5, 0.75));
    }

    #[test]
    fn trot_moves_diagonal_pairs_together() {
        let s = GallopStyle::Rotary;
        assert_eq!(quadruped_phase(Gait::Trot, s, 1, Side::Left), quadruped_phase(Gait::Trot, s, 0, Side::Right));
        assert_eq!(quadruped_phase(Gait::Trot, s, 1, Side::Right), quadruped_phase(Gait::Trot, s, 0, Side::Left));
        assert_eq!(quadruped_phase(Gait::Trot, s, 1, Side::Left) + 0.5, quadruped_phase(Gait::Trot, s, 1, Side::Right));
    }

    #[test]
    fn gallops_pair_hinds_then_fores() {
        for style in [GallopStyle::Transverse, GallopStyle::Rotary, GallopStyle::Bound] {
            let hinds = [
                quadruped_phase(Gait::Gallop, style, 1, Side::Left),
                quadruped_phase(Gait::Gallop, style, 1, Side::Right),
            ];
            let fores = [
                quadruped_phase(Gait::Gallop, style, 0, Side::Left),
                quadruped_phase(Gait::Gallop, style, 0, Side::Right),
            ];
            assert!((hinds[0] - hinds[1]).abs() < 0.2, "{style:?}");
            assert!((fores[0] - fores[1]).abs() < 0.2, "{style:?}");
            assert!(fores.iter().all(|f| *f > 0.3), "{style:?}");
        }
        // Transverse: same lead on both ends (trailing hind L → trailing fore L
        // first); rotary: the lead switches (R hind then R fore first).
        assert!(
            quadruped_phase(Gait::Gallop, GallopStyle::Transverse, 0, Side::Left)
                < quadruped_phase(Gait::Gallop, GallopStyle::Transverse, 0, Side::Right)
        );
        assert!(
            quadruped_phase(Gait::Gallop, GallopStyle::Rotary, 0, Side::Right)
                < quadruped_phase(Gait::Gallop, GallopStyle::Rotary, 0, Side::Left)
        );
    }

    #[test]
    fn tripod_groups() {
        let a = [hexapod_phase(0, Side::Left), hexapod_phase(1, Side::Right), hexapod_phase(2, Side::Left)];
        let b = [hexapod_phase(0, Side::Right), hexapod_phase(1, Side::Left), hexapod_phase(2, Side::Right)];
        assert!(a.iter().all(|p| *p == 0.0));
        assert!(b.iter().all(|p| *p == 0.5));
        // Spiders: alternating tetrapods.
        assert_eq!(octopod_phase(3, Side::Right), 0.0);
        assert_eq!(octopod_phase(3, Side::Left), 0.5);
    }

    #[test]
    fn leg_cycle_is_continuous_and_retracts_in_stance() {
        let duty = 0.6;
        let amp = 0.4;
        let mut prev = leg_pose(0.0, duty, amp);
        assert!((prev.swing - amp).abs() < 1e-6, "touch-down with the foot forward");
        for i in 1..=1000 {
            let p = leg_pose(i as f32 / 1000.0, duty, amp);
            assert!((p.swing - prev.swing).abs() < 0.02, "continuous at {i}");
            if p.stance && prev.stance {
                assert!(p.swing <= prev.swing + 1e-6, "retracts during stance");
            }
            assert!(p.stance || p.lift >= 0.0);
            prev = p;
        }
        let stance = (0..1000).filter(|i| leg_pose(*i as f32 / 1000.0, duty, amp).stance).count();
        assert!((stance as f32 / 1000.0 - duty).abs() < 0.01);
    }

    #[test]
    fn sweep_reproduces_stride() {
        let (stride, duty, leg) = (1.2, 0.6, 0.8);
        let a = sweep_amplitude(stride, duty, leg);
        let foot_travel = 2.0 * leg * a.sin();
        assert!((foot_travel - stride * duty).abs() < 1e-4);
    }

    #[test]
    fn wingbeat_has_asymmetric_downstroke() {
        let (top, down) = wingbeat(0.0, 0.6);
        assert!((top - 1.0).abs() < 1e-6 && down);
        let (bottom, _) = wingbeat(0.6, 0.6);
        assert!((bottom + 1.0).abs() < 1e-5);
        assert!(!wingbeat(0.8, 0.6).1);
    }

    #[test]
    fn undulation_grows_towards_tail_and_travels_backwards() {
        let head = undulation_angle(0.0, 0.25, 1.0, 0.5, 0.0).abs();
        let tail_max =
            (0..100).map(|i| undulation_angle(1.0, i as f32 / 100.0, 1.0, 0.5, 0.0).abs()).fold(0.0, f32::max);
        assert!(tail_max > 4.0 * head);
        // A crest at s at phase p is found at s + δ·λ at phase p + δ: the wave moves tailwards.
        let a = undulation_angle(0.3, 0.1, 0.8, 0.5, 1.0) / (0.3 + 0.7 * 0.3);
        let b = undulation_angle(0.3 + 0.08, 0.2, 0.8, 0.5, 1.0) / (0.3 + 0.7 * 0.38);
        assert!((a - b).abs() < 1e-4);
    }

    #[test]
    fn hop_is_a_parabola() {
        assert_eq!(hop_height(0.0, 0.5), 0.0);
        assert!((hop_height(0.25, 0.5) - 1.0).abs() < 1e-6);
        assert_eq!(hop_height(0.7, 0.5), 0.0);
    }
}
