//! Built-in species table assembly, shared quadruped presets (canids,
//! felids, ungulates, horses, pigs, lizards) and the carnivores. Other
//! groups live in `species_ungulates`, `species_small`, `species_birds`
//! and `species_extra`.

use super::palette::{Coat, Pattern};
use super::parts::{EarStyle, TailStyle};
use super::quadruped::{Foot, Mane, QuadrupedParams};
use super::rig::GallopStyle;
use super::species::{BodyPlan, SpeciesSpec};

pub(super) const Q: QuadrupedParams = QuadrupedParams::BASE;

pub(super) fn quad(id: &str, name: &str, aliases: &[&str], mass: f32, p: QuadrupedParams) -> SpeciesSpec {
    SpeciesSpec::new(id, name, aliases, mass, BodyPlan::Quadruped(p))
}

pub(super) fn canid(body: f32, leg: f32, coat: Coat) -> QuadrupedParams {
    QuadrupedParams {
        body_length: body,
        body_width: 0.3 * body,
        body_depth: 0.36 * body,
        rump: 0.85,
        leg_length: leg,
        leg_radius: 0.055 * body,
        neck_length: 0.28 * body,
        neck_radius: 0.12 * body,
        neck_angle: 0.65,
        head_length: 0.3 * body,
        head_width: 0.17 * body,
        head_depth: 0.15 * body,
        snout: 0.5,
        snout_width: 0.42,
        head_pitch: 0.1,
        eyes_forward: 0.55,
        ears: EarStyle::Pointed,
        ear_size: 0.13 * body,
        tail: TailStyle::Bushy,
        tail_length: 0.55 * body,
        tail_radius: 0.08 * body,
        tail_droop: 1.0,
        coat,
        gallop: GallopStyle::Rotary,
        ..Q
    }
}

pub(super) fn felid(body: f32, leg: f32, coat: Coat) -> QuadrupedParams {
    QuadrupedParams {
        body_length: body,
        body_width: 0.3 * body,
        body_depth: 0.36 * body,
        rump: 0.92,
        leg_length: leg,
        leg_radius: 0.07 * body,
        foot_ratio: 0.75,
        neck_length: 0.22 * body,
        neck_radius: 0.13 * body,
        neck_angle: 0.4,
        head_length: 0.24 * body,
        head_width: 0.2 * body,
        head_depth: 0.18 * body,
        snout: 0.32,
        snout_width: 0.6,
        head_pitch: 0.15,
        eyes_forward: 0.9,
        ears: EarStyle::Round,
        ear_size: 0.055 * body,
        tail: TailStyle::Thin,
        tail_length: 0.6 * body,
        tail_radius: 0.03 * body,
        tail_droop: 1.1,
        coat,
        muzzle: Some(0xf0e6d4),
        nose: 0x6a4a40,
        iris: 0xb08a20,
        gallop: GallopStyle::Rotary,
        ..Q
    }
}

pub(super) fn ungulate(body: f32, leg: f32, coat: Coat) -> QuadrupedParams {
    QuadrupedParams {
        body_length: body,
        body_width: 0.3 * body,
        body_depth: 0.42 * body,
        rump: 0.95,
        leg_length: leg,
        leg_radius: 0.045 * body,
        foot_ratio: 0.5,
        foot: Foot::Hoof,
        neck_length: 0.42 * body,
        neck_radius: 0.1 * body,
        neck_angle: 0.85,
        head_length: 0.32 * body,
        head_width: 0.13 * body,
        head_depth: 0.15 * body,
        snout: 0.55,
        snout_width: 0.5,
        head_pitch: 0.55,
        eyes_forward: 0.0,
        ears: EarStyle::Pointed,
        ear_size: 0.12 * body,
        tail: TailStyle::Stub,
        tail_length: 0.12 * body,
        tail_radius: 0.04 * body,
        tail_droop: 1.2,
        coat,
        gallop: GallopStyle::Transverse,
        ..Q
    }
}
pub(super) fn builtin_species() -> Vec<SpeciesSpec> {
    let mut v = Vec::new();
    carnivores(&mut v);
    super::species_ungulates::ungulates(&mut v);
    super::species_small::small_mammals(&mut v);
    super::species_small::reptiles_and_amphibians(&mut v);
    v.extend(super::species_birds::birds());
    v.extend(super::species_extra::arthropods());
    v.extend(super::species_extra::fishes());
    v
}

/// Canids, bears, felids and the raccoon.
fn carnivores(v: &mut Vec<SpeciesSpec>) {
    v.push(quad(
        "fox",
        "Red fox",
        &[],
        6.0,
        QuadrupedParams {
            socks: Some(0x1e1814),
            muzzle: Some(0xf4eee4),
            ear_tip: Some(0x1e1814),
            tail_tip: Some(0xf4f0e8),
            ..canid(0.6, 0.32, Coat { base: 0xc8541e, belly: 0xf2ebe0, countershade: -0.15, pattern: Pattern::Plain })
        },
    ));
    v.push(quad(
        "wolf",
        "Grey wolf",
        &["wolf_variant", "wild_dog"],
        40.0,
        QuadrupedParams {
            muzzle: Some(0xd8d0c4),
            ..canid(
                1.0,
                0.62,
                Coat {
                    base: 0x8a8278,
                    belly: 0xd8d0c4,
                    countershade: -0.1,
                    pattern: Pattern::Saddle { color: 0x4a443e },
                },
            )
        },
    ));
    v.push(quad(
        "dingo",
        "Dingo",
        &["jackal_proxy", "jackal"],
        15.0,
        QuadrupedParams {
            muzzle: Some(0xf0e4cc),
            tail_tip: Some(0xf0e8d8),
            ..canid(0.85, 0.5, Coat { base: 0xc88a48, belly: 0xf0e4cc, countershade: -0.05, pattern: Pattern::Plain })
        },
    ));
    let bear = QuadrupedParams {
        body_length: 1.6,
        body_width: 0.72,
        body_depth: 0.85,
        rump: 0.9,
        hump: 0.1,
        hump_at: 0.8,
        leg_length: 0.62,
        fore_ratio: 1.08,
        leg_radius: 0.13,
        foot_ratio: 0.8,
        neck_length: 0.42,
        neck_radius: 0.26,
        neck_angle: 0.4,
        head_length: 0.5,
        head_width: 0.38,
        head_depth: 0.33,
        snout: 0.42,
        snout_width: 0.45,
        head_pitch: 0.3,
        eyes_forward: 0.7,
        ears: EarStyle::Round,
        ear_size: 0.13,
        tail: TailStyle::Stub,
        tail_length: 0.1,
        tail_radius: 0.06,
        coat: Coat { base: 0x5a3a22, belly: 0x4a3020, countershade: 0.0, pattern: Pattern::Plain },
        muzzle: Some(0x8a6a4a),
        gallop: GallopStyle::Rotary,
        ..Q
    };
    v.push(quad("brown_bear", "Brown bear", &["bear"], 300.0, bear));
    v.push(quad(
        "polar_bear",
        "Polar bear",
        &[],
        450.0,
        QuadrupedParams { neck_length: 0.45, neck_angle: 0.15, coat: Coat::plain(0xf0ead8), muzzle: None, ..bear },
    ));
    v.push(quad(
        "lion",
        "Lion (male)",
        &["big_cat"],
        190.0,
        QuadrupedParams {
            mane: Mane::Ruff(0x7a4a20),
            tail: TailStyle::Tufted,
            tail_tip: Some(0x3a2414),
            ..felid(1.6, 0.75, Coat { base: 0xc8963c, belly: 0xe8d0a0, countershade: -0.1, pattern: Pattern::Plain })
        },
    ));
    v.push(quad(
        "lioness",
        "Lioness",
        &[],
        130.0,
        QuadrupedParams {
            tail: TailStyle::Tufted,
            tail_tip: Some(0x3a2414),
            ..felid(1.45, 0.68, Coat { base: 0xc89a48, belly: 0xeedcb0, countershade: -0.1, pattern: Pattern::Plain })
        },
    ));
    v.push(quad(
        "tiger",
        "Tiger",
        &[],
        200.0,
        felid(
            1.75,
            0.8,
            Coat {
                base: 0xd86a20,
                belly: 0xf4ece0,
                countershade: -0.35,
                pattern: Pattern::Stripes { color: 0x141010, per_m: 5.0, width: 0.32, reach: -0.7 },
            },
        ),
    ));
    v.push(quad(
        "leopard",
        "Leopard",
        &[],
        60.0,
        felid(
            1.2,
            0.55,
            Coat {
                base: 0xd8a850,
                belly: 0xf2e8d0,
                countershade: -0.3,
                pattern: Pattern::Rosettes { color: 0x2a1a10, per_m: 14.0 },
            },
        ),
    ));
    v.push(quad(
        "cheetah",
        "Cheetah",
        &[],
        50.0,
        QuadrupedParams {
            leg_radius: 0.05,
            head_length: 0.24,
            head_width: 0.2,
            tail_tip: Some(0x1a1410),
            ..felid(
                1.2,
                0.78,
                Coat {
                    base: 0xd8b060,
                    belly: 0xf4ead4,
                    countershade: -0.35,
                    pattern: Pattern::Spots { color: 0x1a1410, per_m: 16.0, radius: 0.28 },
                },
            )
        },
    ));
    v.push(quad(
        "lynx",
        "Eurasian lynx",
        &[],
        20.0,
        QuadrupedParams {
            ears: EarStyle::Tufted,
            ear_size: 0.07,
            ear_tip: Some(0x1a1410),
            tail: TailStyle::Stub,
            tail_length: 0.15,
            tail_radius: 0.035,
            tail_tip: Some(0x1a1410),
            foot_ratio: 1.0,
            ..felid(
                0.95,
                0.58,
                Coat {
                    base: 0xa8906c,
                    belly: 0xece2d0,
                    countershade: -0.2,
                    pattern: Pattern::Spots { color: 0x5a4430, per_m: 14.0, radius: 0.22 },
                },
            )
        },
    ));
    v.push(quad(
        "raccoon",
        "Raccoon",
        &[],
        7.0,
        QuadrupedParams {
            mask: Some(0x1a1a1a),
            muzzle: Some(0xece8e0),
            tail_rings: Some(0x2a2622),
            ears: EarStyle::Round,
            ear_size: 0.05,
            eyes_forward: 0.6,
            tail_radius: 0.05,
            foot: Foot::Paw,
            gallop: GallopStyle::Bound,
            ..canid(0.5, 0.2, Coat { base: 0x7a7068, belly: 0x9a928a, countershade: -0.2, pattern: Pattern::Plain })
        },
    ));
}

pub(super) fn horse(coat: Coat, socks: Option<u32>, mane: u32) -> QuadrupedParams {
    QuadrupedParams {
        body_width: 0.36,
        body_depth: 0.45,
        neck_length: 0.55,
        neck_radius: 0.13,
        neck_angle: 0.95,
        head_length: 0.42,
        head_width: 0.15,
        head_depth: 0.19,
        snout: 0.6,
        snout_width: 0.65,
        head_pitch: 0.95,
        ear_size: 0.1,
        mane: Mane::Crest(mane),
        tail: TailStyle::Flowing,
        tail_length: 0.25,
        tail_radius: 0.04,
        tail_droop: 1.2,
        tail_tip: Some(mane),
        socks,
        ..ungulate(1.45, 0.95, coat)
    }
}

pub(super) fn pig(coat: Coat, tusks: f32, mane: Mane) -> QuadrupedParams {
    QuadrupedParams {
        body_length: 1.2,
        body_width: 0.48,
        body_depth: 0.58,
        rump: 0.95,
        leg_length: 0.38,
        leg_radius: 0.06,
        foot: Foot::Hoof,
        neck_length: 0.12,
        neck_radius: 0.24,
        neck_angle: 0.0,
        head_length: 0.45,
        head_width: 0.26,
        head_depth: 0.26,
        snout: 0.55,
        snout_width: 0.5,
        head_pitch: 0.25,
        ears: if tusks > 0.0 { EarStyle::Pointed } else { EarStyle::Drooping },
        ear_size: 0.12,
        tusks,
        mane,
        tail: TailStyle::Thin,
        tail_length: 0.25,
        tail_radius: 0.015,
        tail_droop: 0.9,
        coat,
        nose: if tusks > 0.0 { 0x2a2420 } else { 0xd08a80 },
        gallop: GallopStyle::Transverse,
        ..Q
    }
}

pub(super) fn lizard(body: f32, tail: f32, coat: Coat, ridge: Option<u32>) -> QuadrupedParams {
    QuadrupedParams {
        body_length: body,
        body_width: 0.3 * body,
        body_depth: 0.22 * body,
        rump: 0.9,
        leg_length: 0.22 * body,
        leg_radius: 0.05 * body,
        foot: Foot::Claw,
        sprawl: true,
        neck_length: 0.08 * body,
        neck_radius: 0.12 * body,
        neck_angle: 0.05,
        head_length: 0.32 * body,
        head_width: 0.22 * body,
        head_depth: 0.14 * body,
        snout: 0.5,
        snout_width: 0.6,
        head_pitch: 0.0,
        ears: EarStyle::None,
        tail: TailStyle::Heavy,
        tail_length: tail,
        tail_radius: 0.12 * body,
        tail_droop: 0.12,
        coat,
        ridge,
        iris: 0xc8a020,
        gallop: GallopStyle::Amble,
        ..Q
    }
}
