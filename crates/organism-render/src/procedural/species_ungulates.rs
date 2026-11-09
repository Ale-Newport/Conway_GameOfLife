//! Built-in ungulates (deer, horses, cattle, camelids, giraffe, elephant,
//! rhinoceros, hippopotamus) for the species table.

use super::palette::{Coat, Pattern};
use super::parts::{EarStyle, TailStyle};
use super::quadruped::{Foot, Headgear, Mane, QuadrupedParams};
use super::rig::GallopStyle;
use super::species::SpeciesSpec;
use super::species_mammals::{Q, horse, pig, quad, ungulate};

/// Hoofed mammals and the large pachyderms.
pub(super) fn ungulates(v: &mut Vec<SpeciesSpec>) {
    v.push(quad(
        "deer",
        "Red deer (hind)",
        &[],
        90.0,
        QuadrupedParams {
            muzzle: Some(0x3a2a20),
            ..ungulate(
                1.25,
                0.95,
                Coat { base: 0x9a6a40, belly: 0xe8dcc8, countershade: -0.3, pattern: Pattern::Plain },
            )
        },
    ));
    v.push(quad(
        "stag",
        "Red deer (stag)",
        &["deer_male", "elk_proxy", "elk"],
        190.0,
        QuadrupedParams {
            headgear: Headgear::Antlers { length: 0.85, tines: 4 },
            neck_radius: 0.17,
            muzzle: Some(0x3a2a20),
            ..ungulate(
                1.45,
                1.05,
                Coat { base: 0x8a5a36, belly: 0xd8c8b0, countershade: -0.3, pattern: Pattern::Plain },
            )
        },
    ));
    v.push(quad(
        "moose",
        "Moose",
        &[],
        500.0,
        QuadrupedParams {
            hump: 0.18,
            hump_at: 0.85,
            fore_ratio: 1.08,
            neck_length: 0.55,
            neck_angle: 0.35,
            neck_radius: 0.26,
            head_length: 0.75,
            head_width: 0.28,
            head_depth: 0.34,
            snout_width: 0.75,
            head_pitch: 0.6,
            headgear: Headgear::Palmate { span: 0.75 },
            socks: Some(0xa8988a),
            ..ungulate(2.1, 1.2, Coat { base: 0x3a2a1e, belly: 0x2e221a, countershade: 0.0, pattern: Pattern::Plain })
        },
    ));
    v.push(quad(
        "horse",
        "Horse",
        &["wild_horse"],
        450.0,
        horse(
            Coat { base: 0x7a4422, belly: 0x6a3a1e, countershade: 0.0, pattern: Pattern::Plain },
            Some(0x1a1410),
            0x1a1410,
        ),
    ));
    v.push(quad(
        "horse_white",
        "Grey horse",
        &["horse_variant"],
        450.0,
        horse(Coat { base: 0xe4e0d8, belly: 0xd8d4cc, countershade: 0.0, pattern: Pattern::Plain }, None, 0xd8d4cc),
    ));
    v.push(quad(
        "donkey",
        "Donkey",
        &["wild_ass"],
        250.0,
        QuadrupedParams {
            ears: EarStyle::Long,
            ear_size: 0.28,
            muzzle: Some(0xece6dc),
            tail: TailStyle::Tufted,
            tail_tip: Some(0x2a2420),
            ..horse(
                Coat { base: 0x8a8278, belly: 0xd8d0c8, countershade: -0.4, pattern: Pattern::Plain },
                None,
                0x4a443e,
            )
        },
    ));
    v.push(quad(
        "zebra",
        "Plains zebra",
        &[],
        350.0,
        QuadrupedParams {
            tail: TailStyle::Tufted,
            tail_tip: Some(0x141414),
            muzzle: Some(0x1a1a1a),
            ..horse(
                Coat {
                    base: 0xf4f0e8,
                    belly: 0xf4f0e8,
                    countershade: 1.0,
                    pattern: Pattern::Stripes { color: 0x141414, per_m: 6.0, width: 0.45, reach: -1.2 },
                },
                None,
                0x141414,
            )
        },
    ));
    v.push(quad(
        "cow",
        "Cattle",
        &["cattle", "aurochs_proxy"],
        600.0,
        QuadrupedParams {
            body_depth: 0.8,
            belly: 0.22,
            neck_length: 0.45,
            neck_angle: 0.25,
            neck_radius: 0.24,
            head_length: 0.52,
            head_width: 0.26,
            head_depth: 0.26,
            snout: 0.4,
            snout_width: 0.75,
            head_pitch: 0.65,
            ears: EarStyle::Drooping,
            ear_size: 0.16,
            headgear: Headgear::Horns { length: 0.22, curl: 0.9, spread: 0.7 },
            tail: TailStyle::Tufted,
            tail_length: 0.8,
            tail_radius: 0.035,
            tail_droop: 1.45,
            tail_tip: Some(0x1a1a1a),
            muzzle: Some(0xe0a8a0),
            nose: 0xc08880,
            ..ungulate(
                1.75,
                0.8,
                Coat {
                    base: 0xf2eee6,
                    belly: 0xf2eee6,
                    countershade: 1.0,
                    pattern: Pattern::Spots { color: 0x1a1a1a, per_m: 1.7, radius: 0.42 },
                },
            )
        },
    ));
    v.push(quad(
        "bison",
        "American bison",
        &["bison_proxy", "buffalo_proxy", "buffalo"],
        800.0,
        QuadrupedParams {
            body_depth: 0.95,
            hump: 0.4,
            hump_at: 0.88,
            rump: 0.75,
            fore_ratio: 1.05,
            neck_length: 0.35,
            neck_angle: 0.0,
            neck_radius: 0.35,
            head_length: 0.6,
            head_width: 0.4,
            head_depth: 0.42,
            snout: 0.35,
            snout_width: 0.7,
            head_pitch: 1.05,
            ears: EarStyle::Round,
            ear_size: 0.08,
            mane: Mane::Ruff(0x3a2618),
            headgear: Headgear::Horns { length: 0.25, curl: 1.4, spread: 0.85 },
            tail: TailStyle::Tufted,
            tail_length: 0.5,
            tail_tip: Some(0x2a1a10),
            ..ungulate(2.2, 0.85, Coat { base: 0x5a3a24, belly: 0x4a3020, countershade: 0.0, pattern: Pattern::Plain })
        },
    ));
    v.push(quad(
        "llama",
        "Llama",
        &["alpaca", "guanaco"],
        120.0,
        QuadrupedParams {
            neck_length: 0.75,
            neck_angle: 1.35,
            neck_radius: 0.11,
            head_length: 0.3,
            head_pitch: 1.2,
            ears: EarStyle::Long,
            ear_size: 0.15,
            tail_droop: 0.2,
            foot: Foot::Paw,
            ..ungulate(
                1.15,
                0.85,
                Coat { base: 0xd8c8a8, belly: 0xece2cc, countershade: -0.3, pattern: Pattern::Plain },
            )
        },
    ));
    v.push(quad(
        "sheep",
        "Sheep",
        &["mountain_goat_proxy"],
        70.0,
        QuadrupedParams {
            body_width: 0.48,
            body_depth: 0.6,
            neck_length: 0.25,
            neck_radius: 0.17,
            neck_angle: 0.5,
            head_length: 0.28,
            head_width: 0.15,
            head_depth: 0.17,
            ears: EarStyle::Drooping,
            ear_size: 0.1,
            face: Some(0x2a2420),
            socks: Some(0x2a2420),
            tail_length: 0.15,
            ..ungulate(1.1, 0.5, Coat { base: 0xece6d6, belly: 0xe0d8c8, countershade: 0.0, pattern: Pattern::Plain })
        },
    ));
    v.push(quad(
        "pig",
        "Domestic pig",
        &[],
        150.0,
        pig(Coat { base: 0xe8b4a4, belly: 0xe0a898, countershade: 0.0, pattern: Pattern::Plain }, 0.0, Mane::None),
    ));
    v.push(quad(
        "boar",
        "Wild boar",
        &["boar_proxy", "wild_boar"],
        100.0,
        pig(
            Coat { base: 0x4a3a2c, belly: 0x3a2e24, countershade: 0.0, pattern: Pattern::Plain },
            0.08,
            Mane::Crest(0x2a2018),
        ),
    ));
    v.push(quad(
        "giraffe",
        "Giraffe",
        &[],
        1000.0,
        QuadrupedParams {
            body_depth: 0.85,
            rump: 0.75,
            fore_ratio: 1.25,
            neck_length: 2.0,
            neck_angle: 1.05,
            neck_radius: 0.15,
            head_length: 0.6,
            head_width: 0.2,
            head_depth: 0.24,
            head_pitch: 1.1,
            headgear: Headgear::Ossicones,
            ear_size: 0.17,
            mane: Mane::Crest(0x8a5a2a),
            tail: TailStyle::Tufted,
            tail_length: 0.95,
            tail_radius: 0.03,
            tail_droop: 1.45,
            tail_tip: Some(0x1a1410),
            gallop: GallopStyle::Rotary,
            // Dark polygonal patches separated by pale lines.
            ..ungulate(
                1.9,
                1.75,
                Coat {
                    base: 0xa8642c,
                    belly: 0xece2c8,
                    countershade: -0.6,
                    pattern: Pattern::Reticulated { line: 0xf2e8d0, per_m: 3.6 },
                },
            )
        },
    ));
    v.push(quad(
        "camel",
        "Dromedary camel",
        &[],
        500.0,
        QuadrupedParams {
            hump: 0.5,
            hump_at: 0.5,
            neck_length: 1.1,
            neck_angle: 0.05,
            neck_bend: -0.95,
            neck_radius: 0.13,
            head_length: 0.5,
            head_width: 0.2,
            head_depth: 0.22,
            head_pitch: -0.05,
            ears: EarStyle::Round,
            ear_size: 0.06,
            foot: Foot::Pad,
            tail: TailStyle::Tufted,
            tail_length: 0.5,
            tail_tip: Some(0x5a4024),
            ..ungulate(1.9, 1.25, Coat { base: 0xc8a070, belly: 0xd8b888, countershade: -0.3, pattern: Pattern::Plain })
        },
    ));
    v.push(quad(
        "elephant",
        "African elephant",
        &[],
        5000.0,
        QuadrupedParams {
            body_length: 3.2,
            body_width: 1.55,
            body_depth: 1.8,
            rump: 0.88,
            belly: 0.08,
            leg_length: 1.65,
            fore_ratio: 1.05,
            leg_radius: 0.28,
            foot_ratio: 1.05,
            foot: Foot::Pad,
            hoof: 0x8e8882,
            neck_length: 0.35,
            neck_radius: 0.6,
            neck_angle: 0.15,
            head_length: 1.0,
            head_width: 0.9,
            head_depth: 1.0,
            snout: 0.25,
            snout_width: 0.6,
            head_pitch: 0.35,
            eyes_forward: 0.0,
            ears: EarStyle::Fan,
            ear_size: 1.5,
            ear_inner: 0x8a7a76,
            trunk: 1.8,
            tusks: 0.85,
            tail: TailStyle::Tufted,
            tail_length: 1.0,
            tail_radius: 0.06,
            tail_droop: 1.35,
            tail_tip: Some(0x2a2826),
            coat: Coat { base: 0x7a7470, belly: 0x6a6460, countershade: 0.0, pattern: Pattern::Plain },
            nose: 0x5a5450,
            gallop: GallopStyle::Amble,
            ..Q
        },
    ));
    v.push(quad(
        "rhinoceros",
        "White rhinoceros",
        &["rhino"],
        2000.0,
        QuadrupedParams {
            body_length: 2.4,
            body_width: 1.05,
            body_depth: 1.15,
            hump: 0.12,
            hump_at: 0.92,
            leg_length: 0.8,
            leg_radius: 0.2,
            foot: Foot::Pad,
            hoof: 0x5a5450,
            neck_length: 0.45,
            neck_radius: 0.42,
            neck_angle: 0.05,
            head_length: 0.85,
            head_width: 0.5,
            head_depth: 0.5,
            snout: 0.45,
            snout_width: 0.85,
            head_pitch: 0.7,
            eyes_forward: 0.0,
            ears: EarStyle::Pointed,
            ear_size: 0.16,
            headgear: Headgear::NoseHorns { front: 0.6, rear: 0.28 },
            tail: TailStyle::Tufted,
            tail_length: 0.5,
            tail_radius: 0.04,
            tail_tip: Some(0x2a2826),
            coat: Coat::plain(0x8a8480),
            nose: 0x6a6460,
            gallop: GallopStyle::Rotary,
            ..Q
        },
    ));
    v.push(quad(
        "hippopotamus",
        "Hippopotamus",
        &["hippo"],
        1500.0,
        QuadrupedParams {
            body_length: 2.8,
            body_width: 1.3,
            body_depth: 1.3,
            belly: 0.2,
            leg_length: 0.55,
            leg_radius: 0.22,
            foot: Foot::Pad,
            neck_length: 0.3,
            neck_radius: 0.6,
            neck_angle: 0.05,
            head_length: 0.95,
            head_width: 0.75,
            head_depth: 0.6,
            snout: 0.45,
            snout_width: 1.0,
            head_pitch: 0.1,
            ears: EarStyle::Round,
            ear_size: 0.07,
            tail: TailStyle::Stub,
            tail_length: 0.3,
            tail_radius: 0.07,
            coat: Coat { base: 0x6e5c5a, belly: 0xb08080, countershade: -0.3, pattern: Pattern::Plain },
            muzzle: Some(0xb08080),
            gallop: GallopStyle::Amble,
            ..Q
        },
    ));
}
