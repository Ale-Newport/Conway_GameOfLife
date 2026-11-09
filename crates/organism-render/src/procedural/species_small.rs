//! Built-in small mammals, reptiles, amphibians and molluscs for the
//! species table.

use super::aquatic::SerpentParams;
use super::herp::{FrogParams, SnailParams, TurtleParams};
use super::palette::{Coat, Pattern};
use super::parts::{EarStyle, TailStyle};
use super::quadruped::QuadrupedParams;
use super::rig::GallopStyle;
use super::species::{BodyPlan, SpeciesSpec};
use super::species_mammals::{Q, lizard, quad};

/// Rodents, lagomorphs, hedgehog and beaver.
pub(super) fn small_mammals(v: &mut Vec<SpeciesSpec>) {
    v.push(quad(
        "rat",
        "Brown rat",
        &["mouse_proxy", "small_rodent", "mouse"],
        0.3,
        QuadrupedParams {
            body_length: 0.2,
            body_width: 0.07,
            body_depth: 0.075,
            leg_length: 0.045,
            leg_radius: 0.01,
            neck_length: 0.03,
            neck_radius: 0.025,
            neck_angle: 0.2,
            head_length: 0.06,
            head_width: 0.035,
            head_depth: 0.032,
            snout: 0.55,
            snout_width: 0.4,
            ears: EarStyle::Round,
            ear_size: 0.02,
            ear_inner: 0xd8a8a0,
            tail: TailStyle::Thin,
            tail_length: 0.2,
            tail_radius: 0.005,
            tail_droop: 0.4,
            coat: Coat { base: 0x6a5a4a, belly: 0xb8aea0, countershade: -0.2, pattern: Pattern::Plain },
            nose: 0xd8a0a0,
            gallop: GallopStyle::Bound,
            ..Q
        },
    ));
    v.push(quad(
        "rabbit",
        "European rabbit",
        &["hare"],
        2.0,
        QuadrupedParams {
            body_length: 0.32,
            body_width: 0.15,
            body_depth: 0.18,
            rump: 1.1,
            leg_length: 0.13,
            fore_ratio: 0.62,
            leg_radius: 0.022,
            neck_length: 0.04,
            neck_radius: 0.05,
            neck_angle: 0.6,
            head_length: 0.1,
            head_width: 0.075,
            head_depth: 0.075,
            snout: 0.35,
            snout_width: 0.6,
            eyes_forward: 0.0,
            ears: EarStyle::Long,
            ear_size: 0.1,
            ear_tip: Some(0x3a2a20),
            tail: TailStyle::Stub,
            tail_length: 0.05,
            tail_radius: 0.025,
            tail_droop: -0.6,
            tail_tip: Some(0xf4f0ea),
            coat: Coat { base: 0x8a7058, belly: 0xece4d8, countershade: -0.1, pattern: Pattern::Plain },
            gallop: GallopStyle::Bound,
            ..Q
        },
    ));
    v.push(quad(
        "squirrel",
        "Red squirrel",
        &[],
        0.3,
        QuadrupedParams {
            body_length: 0.2,
            body_width: 0.07,
            body_depth: 0.08,
            leg_length: 0.06,
            fore_ratio: 0.8,
            leg_radius: 0.011,
            neck_length: 0.03,
            neck_radius: 0.028,
            neck_angle: 0.8,
            head_length: 0.06,
            head_width: 0.04,
            head_depth: 0.04,
            snout: 0.35,
            ears: EarStyle::Tufted,
            ear_size: 0.025,
            ear_tip: Some(0x8a3a1a),
            tail: TailStyle::Bushy,
            tail_length: 0.19,
            tail_radius: 0.03,
            tail_droop: -1.35,
            coat: Coat { base: 0xa0522d, belly: 0xf4ece0, countershade: -0.2, pattern: Pattern::Plain },
            gallop: GallopStyle::Bound,
            ..Q
        },
    ));
    v.push(quad(
        "hedgehog",
        "European hedgehog",
        &[],
        0.8,
        QuadrupedParams {
            body_length: 0.2,
            body_width: 0.13,
            body_depth: 0.11,
            leg_length: 0.03,
            leg_radius: 0.008,
            neck_length: 0.02,
            neck_radius: 0.03,
            neck_angle: 0.0,
            head_length: 0.06,
            head_width: 0.05,
            head_depth: 0.045,
            snout: 0.55,
            snout_width: 0.35,
            ears: EarStyle::Round,
            ear_size: 0.012,
            spines: Some(0x5a4430),
            tail: TailStyle::None,
            coat: Coat { base: 0x9a8670, belly: 0xb8a890, countershade: 0.0, pattern: Pattern::Plain },
            gallop: GallopStyle::Bound,
            ..Q
        },
    ));
    v.push(quad(
        "beaver",
        "Eurasian beaver",
        &[],
        20.0,
        QuadrupedParams {
            body_length: 0.7,
            body_width: 0.32,
            body_depth: 0.3,
            rump: 1.1,
            leg_length: 0.12,
            leg_radius: 0.03,
            neck_length: 0.06,
            neck_radius: 0.11,
            neck_angle: 0.2,
            head_length: 0.16,
            head_width: 0.12,
            head_depth: 0.11,
            snout: 0.35,
            snout_width: 0.65,
            ears: EarStyle::Round,
            ear_size: 0.02,
            tail: TailStyle::Paddle,
            tail_length: 0.3,
            tail_radius: 0.05,
            tail_droop: 0.2,
            coat: Coat { base: 0x5a3a24, belly: 0x6a4a30, countershade: 0.0, pattern: Pattern::Plain },
            gallop: GallopStyle::Bound,
            ..Q
        },
    ));
}

/// Lizards, crocodilians, snakes, worms, frogs, turtles and snails.
pub(super) fn reptiles_and_amphibians(v: &mut Vec<SpeciesSpec>) {
    v.push(quad(
        "lizard",
        "Green lizard",
        &[],
        0.03,
        lizard(
            0.1,
            0.25,
            Coat {
                base: 0x4a9a30,
                belly: 0xd8e0a0,
                countershade: -0.2,
                pattern: Pattern::Speckles { color: 0x2a5a1a, per_m: 120.0 },
            },
            None,
        ),
    ));
    v.push(quad(
        "crocodile",
        "Nile crocodile",
        &["crocodilian"],
        400.0,
        QuadrupedParams {
            head_length: 0.75,
            head_width: 0.32,
            head_depth: 0.2,
            snout: 0.7,
            snout_width: 0.55,
            ..lizard(
                1.6,
                2.2,
                Coat { base: 0x4a5430, belly: 0xc8c090, countershade: -0.3, pattern: Pattern::Plain },
                Some(0x2a3018),
            )
        },
    ));
    v.push(quad(
        "salamander",
        "Fire salamander",
        &[],
        0.03,
        QuadrupedParams {
            head_width: 0.022,
            ..lizard(
                0.08,
                0.1,
                Coat {
                    base: 0x141414,
                    belly: 0x1a1a1a,
                    countershade: 1.0,
                    pattern: Pattern::Spots { color: 0xf0c010, per_m: 70.0, radius: 0.32 },
                },
                None,
            )
        },
    ));
    v.push(SpeciesSpec::new(
        "snake",
        "Grass snake",
        &[],
        0.3,
        BodyPlan::Serpent(SerpentParams {
            length: 1.0,
            radius: 0.018,
            head_length: 0.04,
            head_width: 1.4,
            segments: 12,
            coat: Coat {
                base: 0x4e6a3a,
                belly: 0xd8d4b0,
                countershade: -0.35,
                pattern: Pattern::Speckles { color: 0x1a2010, per_m: 40.0 },
            },
            worm: false,
            accent: 0x202020,
            iris: 0xc8a020,
        }),
    ));
    v.push(SpeciesSpec::new(
        "viper",
        "Adder",
        &[],
        0.15,
        BodyPlan::Serpent(SerpentParams {
            length: 0.6,
            radius: 0.018,
            head_length: 0.035,
            head_width: 1.7,
            segments: 10,
            coat: Coat {
                base: 0x9a8a6a,
                belly: 0x3a3430,
                countershade: -0.4,
                pattern: Pattern::Bands { color: 0x1a1612, per_m: 22.0 },
            },
            worm: false,
            accent: 0x202020,
            iris: 0xc04020,
        }),
    ));
    v.push(SpeciesSpec::new(
        "earthworm",
        "Earthworm",
        &["worm"],
        0.005,
        BodyPlan::Serpent(SerpentParams {
            length: 0.15,
            radius: 0.0035,
            head_length: 0.012,
            head_width: 1.0,
            segments: 9,
            coat: Coat::plain(0xc07a7a),
            worm: true,
            accent: 0xd89a8a,
            iris: 0,
        }),
    ));
    v.push(SpeciesSpec::new(
        "frog",
        "Tree frog",
        &["toad_proxy"],
        0.01,
        BodyPlan::Frog(FrogParams {
            length: 0.05,
            width: 0.03,
            depth: 0.018,
            hind_leg: 0.075,
            coat: Coat { base: 0x4aa030, belly: 0xf0f0d0, countershade: -0.2, pattern: Pattern::Plain },
            warts: false,
            iris: 0xd03010,
        }),
    ));
    v.push(SpeciesSpec::new(
        "toad",
        "Common toad",
        &[],
        0.06,
        BodyPlan::Frog(FrogParams {
            length: 0.1,
            width: 0.07,
            depth: 0.04,
            hind_leg: 0.1,
            coat: Coat { base: 0x7a6040, belly: 0xc8b890, countershade: -0.2, pattern: Pattern::Plain },
            warts: true,
            iris: 0xc87020,
        }),
    ));
    v.push(SpeciesSpec::new(
        "poison_dart_frog",
        "Blue poison dart frog",
        &[],
        0.008,
        BodyPlan::Frog(FrogParams {
            length: 0.04,
            width: 0.024,
            depth: 0.016,
            hind_leg: 0.05,
            coat: Coat {
                base: 0x1a50d0,
                belly: 0x1a3aa0,
                countershade: 0.0,
                pattern: Pattern::Spots { color: 0x0a0a14, per_m: 220.0, radius: 0.3 },
            },
            warts: false,
            iris: 0x0a0a0a,
        }),
    ));
    v.push(SpeciesSpec::new(
        "turtle",
        "Pond turtle",
        &[],
        1.0,
        BodyPlan::Turtle(TurtleParams {
            shell_length: 0.22,
            shell_width: 0.17,
            shell_height: 0.06,
            head: 0.022,
            neck: 0.05,
            leg: 0.04,
            flippers: false,
            shell: 0x3a4028,
            seams: 0xc8b870,
            skin: 0x4a5a38,
            plastron: 0xd0b060,
        }),
    ));
    v.push(SpeciesSpec::new(
        "tortoise",
        "Tortoise",
        &[],
        8.0,
        BodyPlan::Turtle(TurtleParams {
            shell_length: 0.4,
            shell_width: 0.3,
            shell_height: 0.22,
            head: 0.035,
            neck: 0.07,
            leg: 0.09,
            flippers: false,
            shell: 0x8a6a30,
            seams: 0x3a2a14,
            skin: 0x8a7a60,
            plastron: 0xb09060,
        }),
    ));
    v.push(SpeciesSpec::new(
        "sea_turtle",
        "Green sea turtle",
        &[],
        150.0,
        BodyPlan::Turtle(TurtleParams {
            shell_length: 1.0,
            shell_width: 0.8,
            shell_height: 0.25,
            head: 0.08,
            neck: 0.12,
            leg: 0.3,
            flippers: true,
            shell: 0x5a4a2a,
            seams: 0xc8b080,
            skin: 0x6a6a4a,
            plastron: 0xe0d0a0,
        }),
    ));
    v.push(SpeciesSpec::new(
        "snail",
        "Garden snail",
        &[],
        0.01,
        BodyPlan::Snail(SnailParams {
            foot_length: 0.05,
            shell_radius: 0.016,
            whorls: 3.5,
            shell: 0x9a6a3a,
            band: 0x3a2414,
            body: 0xa89878,
        }),
    ));
}
