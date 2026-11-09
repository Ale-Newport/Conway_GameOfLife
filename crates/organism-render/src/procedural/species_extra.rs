//! Built-in arthropods and fish for the species table (see
//! [`super::species`]).

use bevy::math::Vec3;

use super::aquatic::{Caudal, FishParams};
use super::arthropod::{Antennae, ArthropodParams, WingRest, Wings};
use super::palette::{Coat, Pattern};
use super::species::{BodyPlan, SpeciesSpec};

const A: ArthropodParams = ArthropodParams::BASE;
const F: FishParams = FishParams::BASE;

fn bug(id: &str, name: &str, aliases: &[&str], mass: f32, p: ArthropodParams, rest: WingRest) -> SpeciesSpec {
    SpeciesSpec::new(id, name, aliases, mass, BodyPlan::Arthropod(p, rest))
}

pub(super) fn arthropods() -> Vec<SpeciesSpec> {
    vec![
        bug(
            "bee",
            "Western honey bee",
            &["bee_proxy", "honeybee"],
            1e-4,
            ArthropodParams {
                head: Vec3::new(0.0018, 0.0016, 0.0013),
                thorax: Vec3::new(0.002, 0.002, 0.0022),
                abdomen: Vec3::new(0.0021, 0.002, 0.0038),
                leg_length: 0.007,
                antennae: Antennae::Elbowed(0.004),
                wings: Wings::Membrane { pairs: 2, length: 0.009, width: 0.0032, tint: 0xd8e0f0 },
                head_color: 0x2a2018,
                thorax_color: 0x6a4a20,
                abdomen_color: 0xd8a020,
                bands: Some((0x1a1410, 520.0)),
                fuzzy: true,
                leg_color: 0x2a2018,
                flap_hz: 24.0,
                ..A
            },
            WingRest::FoldedBack,
        ),
        bug(
            "bumblebee",
            "Buff-tailed bumblebee",
            &[],
            2e-4,
            ArthropodParams {
                head: Vec3::new(0.0025, 0.0023, 0.002),
                thorax: Vec3::new(0.0035, 0.0034, 0.0035),
                abdomen: Vec3::new(0.0038, 0.0034, 0.005),
                leg_length: 0.009,
                antennae: Antennae::Elbowed(0.005),
                wings: Wings::Membrane { pairs: 2, length: 0.011, width: 0.0038, tint: 0xc8c0b0 },
                head_color: 0x141414,
                thorax_color: 0xe0b020,
                abdomen_color: 0x141414,
                bands: Some((0xe8c020, 260.0)),
                fuzzy: true,
                leg_color: 0x141414,
                flap_hz: 20.0,
                ..A
            },
            WingRest::FoldedBack,
        ),
        bug(
            "wasp",
            "Common wasp",
            &["flying_insect"],
            1e-4,
            ArthropodParams {
                head: Vec3::new(0.0018, 0.0016, 0.0014),
                thorax: Vec3::new(0.0018, 0.0018, 0.0022),
                abdomen: Vec3::new(0.002, 0.0019, 0.0042),
                waist: 0.0012,
                leg_length: 0.008,
                antennae: Antennae::Elbowed(0.0045),
                wings: Wings::Membrane { pairs: 2, length: 0.009, width: 0.0028, tint: 0xc8b890 },
                head_color: 0xe0b818,
                thorax_color: 0x1a1612,
                abdomen_color: 0xf0c818,
                bands: Some((0x141210, 480.0)),
                leg_color: 0xd8b020,
                flap_hz: 24.0,
                ..A
            },
            WingRest::FoldedBack,
        ),
        bug(
            "ant",
            "Red wood ant",
            &["ants"],
            1e-5,
            ArthropodParams {
                head: Vec3::new(0.0011, 0.0009, 0.0011),
                thorax: Vec3::new(0.0007, 0.0008, 0.0016),
                abdomen: Vec3::new(0.0013, 0.0012, 0.0016),
                waist: 0.0008,
                leg_length: 0.0055,
                leg_radius: 0.00012,
                antennae: Antennae::Elbowed(0.0032),
                mandibles: 0.0007,
                head_color: 0x7a2a14,
                thorax_color: 0x9a3a18,
                abdomen_color: 0x1a1210,
                leg_color: 0x6a2410,
                ..A
            },
            WingRest::FoldedBack,
        ),
        bug(
            "beetle",
            "Ground beetle",
            &["beetles"],
            5e-4,
            ArthropodParams {
                head: Vec3::new(0.0022, 0.0016, 0.002),
                thorax: Vec3::new(0.0035, 0.0022, 0.0028),
                abdomen: Vec3::new(0.0045, 0.0028, 0.0075),
                leg_length: 0.011,
                leg_radius: 0.0004,
                antennae: Antennae::Straight(0.008),
                wings: Wings::Elytra { color: 0x1a2a44, spots: None },
                mandibles: 0.0012,
                head_color: 0x141820,
                thorax_color: 0x1a2236,
                abdomen_color: 0x141414,
                ..A
            },
            WingRest::FoldedBack,
        ),
        bug(
            "stag_beetle",
            "Stag beetle",
            &[],
            0.002,
            ArthropodParams {
                head: Vec3::new(0.004, 0.0025, 0.0035),
                thorax: Vec3::new(0.005, 0.003, 0.004),
                abdomen: Vec3::new(0.006, 0.0035, 0.01),
                leg_length: 0.016,
                leg_radius: 0.0006,
                antennae: Antennae::Elbowed(0.008),
                wings: Wings::Elytra { color: 0x5a2a14, spots: None },
                mandibles: 0.012,
                head_color: 0x3a1a10,
                thorax_color: 0x1a1410,
                abdomen_color: 0x1a1410,
                ..A
            },
            WingRest::FoldedBack,
        ),
        bug(
            "ladybird",
            "Seven-spot ladybird",
            &["ladybug"],
            3e-5,
            ArthropodParams {
                head: Vec3::new(0.0012, 0.0009, 0.0008),
                thorax: Vec3::new(0.0018, 0.0012, 0.0012),
                abdomen: Vec3::new(0.0028, 0.0022, 0.0032),
                leg_length: 0.004,
                antennae: Antennae::Straight(0.0015),
                wings: Wings::Elytra { color: 0xd81a10, spots: Some(0x101010) },
                head_color: 0x101010,
                thorax_color: 0x141414,
                abdomen_color: 0x141414,
                ..A
            },
            WingRest::FoldedBack,
        ),
        bug(
            "butterfly",
            "Monarch butterfly",
            &["monarch"],
            5e-4,
            ArthropodParams {
                head: Vec3::new(0.0012, 0.0012, 0.0012),
                thorax: Vec3::new(0.0014, 0.0015, 0.0035),
                abdomen: Vec3::new(0.0011, 0.0012, 0.008),
                leg_length: 0.008,
                antennae: Antennae::Clubbed(0.012),
                wings: Wings::Scaled { length: 0.05, base: 0xe8701a, border: 0x141010, spots: 0xf8f4ec },
                head_color: 0x141010,
                thorax_color: 0x141010,
                abdomen_color: 0x1a1414,
                flap_hz: 6.0,
                ..A
            },
            WingRest::Upright,
        ),
        bug(
            "morpho",
            "Blue morpho",
            &[],
            6e-4,
            ArthropodParams {
                head: Vec3::new(0.0014, 0.0014, 0.0014),
                thorax: Vec3::new(0.0016, 0.0017, 0.004),
                abdomen: Vec3::new(0.0012, 0.0013, 0.009),
                leg_length: 0.009,
                antennae: Antennae::Clubbed(0.014),
                wings: Wings::Scaled { length: 0.07, base: 0x1a7ae8, border: 0x0e0e14, spots: 0xe8f0f8 },
                head_color: 0x1a1414,
                thorax_color: 0x1a1414,
                abdomen_color: 0x2a2018,
                flap_hz: 4.0,
                ..A
            },
            WingRest::Upright,
        ),
        bug(
            "dragonfly",
            "Emperor dragonfly",
            &[],
            0.001,
            ArthropodParams {
                head: Vec3::new(0.004, 0.0032, 0.003),
                thorax: Vec3::new(0.003, 0.0035, 0.0055),
                abdomen: Vec3::new(0.0015, 0.0016, 0.025),
                leg_length: 0.01,
                antennae: Antennae::None,
                wings: Wings::Membrane { pairs: 2, length: 0.05, width: 0.011, tint: 0xd8e8f0 },
                head_color: 0x2a8a3a,
                thorax_color: 0x4a9a3a,
                abdomen_color: 0x1a5ac0,
                bands: Some((0x101420, 160.0)),
                eye_color: 0x2a7ad0,
                flap_hz: 15.0,
                ..A
            },
            WingRest::Spread,
        ),
        bug(
            "grasshopper",
            "Meadow grasshopper",
            &["locust"],
            3e-4,
            ArthropodParams {
                head: Vec3::new(0.0018, 0.0028, 0.0022),
                thorax: Vec3::new(0.0022, 0.0028, 0.0045),
                abdomen: Vec3::new(0.0019, 0.0022, 0.0085),
                leg_length: 0.01,
                hind_legs: 2.1,
                antennae: Antennae::Straight(0.008),
                wings: Wings::Membrane { pairs: 1, length: 0.015, width: 0.003, tint: 0x8aa860 },
                head_color: 0x6a9a3a,
                thorax_color: 0x5a8a30,
                abdomen_color: 0x7aa840,
                leg_color: 0x5a8a30,
                flap_hz: 0.0,
                ..A
            },
            WingRest::FoldedBack,
        ),
        bug(
            "spider",
            "Garden spider",
            &["arachnid"],
            5e-4,
            ArthropodParams {
                head: Vec3::new(0.0014, 0.0012, 0.001),
                thorax: Vec3::new(0.0025, 0.0018, 0.003),
                abdomen: Vec3::new(0.004, 0.0038, 0.005),
                leg_pairs: 4,
                leg_length: 0.016,
                leg_radius: 0.00035,
                antennae: Antennae::None,
                head_color: 0x4a3420,
                thorax_color: 0x5a4028,
                abdomen_color: 0x8a6438,
                bands: Some((0x3a2614, 300.0)),
                leg_color: 0x5a4028,
                eye_color: 0x0a0a0a,
                ..A
            },
            WingRest::FoldedBack,
        ),
    ]
}

fn fish(id: &str, name: &str, aliases: &[&str], mass: f32, p: FishParams) -> SpeciesSpec {
    SpeciesSpec::new(id, name, aliases, mass, BodyPlan::Fish(p))
}

pub(super) fn fishes() -> Vec<SpeciesSpec> {
    vec![
        fish(
            "trout",
            "Brown trout",
            &[],
            1.0,
            FishParams {
                length: 0.4,
                depth: 0.09,
                width: 0.05,
                coat: Coat {
                    base: 0x6a6a3a,
                    belly: 0xf0e8c8,
                    countershade: -0.3,
                    pattern: Pattern::Speckles { color: 0x3a2010, per_m: 40.0 },
                },
                fin: 0x8a7a50,
                ..F
            },
        ),
        fish(
            "salmon",
            "Atlantic salmon",
            &[],
            4.0,
            FishParams {
                length: 0.75,
                depth: 0.17,
                width: 0.09,
                coat: Coat {
                    base: 0x4a5a6a,
                    belly: 0xe8e8ec,
                    countershade: -0.1,
                    pattern: Pattern::Speckles { color: 0x1a1a24, per_m: 18.0 },
                },
                fin: 0x5a6070,
                ..F
            },
        ),
        fish(
            "perch",
            "European perch",
            &["fish_small", "trout_proxy"],
            0.3,
            FishParams {
                length: 0.25,
                depth: 0.075,
                width: 0.035,
                depth_at: 0.4,
                dorsal: 0.6,
                coat: Coat {
                    base: 0x6a7a30,
                    belly: 0xe8e0b0,
                    countershade: -0.3,
                    pattern: Pattern::Stripes { color: 0x1a2010, per_m: 26.0, width: 0.4, reach: -0.3 },
                },
                fin: 0xd86020,
                ..F
            },
        ),
        fish(
            "goldfish",
            "Goldfish",
            &[],
            0.1,
            FishParams {
                length: 0.15,
                depth: 0.065,
                width: 0.035,
                depth_at: 0.4,
                caudal: Caudal::Forked,
                caudal_size: 1.6,
                coat: Coat::countershaded(0xe86a10, 0xf8c060),
                fin: 0xf08a30,
                segments: 3,
                ..F
            },
        ),
        fish(
            "minnow",
            "Common minnow",
            &[],
            0.01,
            FishParams {
                length: 0.08,
                depth: 0.015,
                width: 0.01,
                coat: Coat {
                    base: 0x6a7058,
                    belly: 0xe8e0d0,
                    countershade: -0.2,
                    pattern: Pattern::Spots { color: 0x2a2a20, per_m: 120.0, radius: 0.25 },
                },
                fin: 0xa89870,
                segments: 3,
                ..F
            },
        ),
        fish(
            "catfish",
            "Wels catfish",
            &[],
            20.0,
            FishParams {
                length: 1.4,
                depth: 0.2,
                width: 0.2,
                depth_at: 0.25,
                caudal: Caudal::Rounded,
                caudal_size: 0.9,
                dorsal: 0.15,
                barbels: 0.25,
                coat: Coat {
                    base: 0x3a3a2e,
                    belly: 0xc8c0b0,
                    countershade: -0.3,
                    pattern: Pattern::Spots { color: 0x2a2620, per_m: 8.0, radius: 0.35 },
                },
                fin: 0x3a362e,
                segments: 5,
                uniform: 0.4,
                ..F
            },
        ),
        fish(
            "eel",
            "European eel",
            &[],
            1.0,
            FishParams {
                length: 0.8,
                depth: 0.04,
                width: 0.035,
                depth_at: 0.3,
                caudal: Caudal::Ribbon,
                caudal_size: 1.2,
                dorsal: 0.0,
                pectoral: 0.3,
                coat: Coat::countershaded(0x3a3a24, 0xd8c890),
                fin: 0x4a4a30,
                segments: 7,
                uniform: 1.0,
                ..F
            },
        ),
        fish(
            "pike",
            "Northern pike",
            &[],
            5.0,
            FishParams {
                length: 0.9,
                depth: 0.14,
                width: 0.09,
                depth_at: 0.5,
                dorsal: 0.4,
                coat: Coat {
                    base: 0x4a6030,
                    belly: 0xe8e4c0,
                    countershade: -0.25,
                    pattern: Pattern::Spots { color: 0xc8c890, per_m: 18.0, radius: 0.3 },
                },
                fin: 0x8a6030,
                segments: 5,
                ..F
            },
        ),
        fish(
            "clownfish",
            "Clown anemonefish",
            &["reef_fish"],
            0.02,
            FishParams {
                length: 0.09,
                depth: 0.04,
                width: 0.02,
                depth_at: 0.4,
                caudal: Caudal::Rounded,
                coat: Coat {
                    base: 0xf06a10,
                    belly: 0xf07a20,
                    countershade: 1.0,
                    pattern: Pattern::Stripes { color: 0xf8f8f4, per_m: 18.0, width: 0.22, reach: -1.2 },
                },
                fin: 0xf07018,
                segments: 3,
                ..F
            },
        ),
        fish(
            "bream",
            "Common bream",
            &["fish_medium", "carp_proxy", "carp"],
            2.0,
            FishParams {
                length: 0.45,
                depth: 0.16,
                width: 0.05,
                depth_at: 0.42,
                coat: Coat::countershaded(0x5a5a40, 0xd8c8a0),
                fin: 0x5a5040,
                ..F
            },
        ),
        fish(
            "shark",
            "Blacktip reef shark",
            &["predatory_fish"],
            30.0,
            FishParams {
                length: 1.6,
                depth: 0.25,
                width: 0.22,
                depth_at: 0.35,
                caudal: Caudal::Lunate,
                caudal_size: 1.6,
                dorsal: 0.9,
                pectoral: 1.3,
                coat: Coat::countershaded(0x6a7480, 0xf0f0f0),
                fin: 0x5a6470,
                segments: 4,
                ..F
            },
        ),
        fish(
            "dolphin",
            "Bottlenose dolphin",
            &[],
            200.0,
            FishParams {
                length: 2.6,
                depth: 0.45,
                width: 0.42,
                depth_at: 0.35,
                caudal: Caudal::Fluke,
                caudal_size: 1.8,
                dorsal: 0.6,
                pectoral: 0.8,
                coat: Coat::countershaded(0x6a7480, 0xd8dce0),
                fin: 0x6a7480,
                segments: 4,
                ..F
            },
        ),
        fish(
            "whale",
            "Humpback whale",
            &[],
            30000.0,
            FishParams {
                length: 13.0,
                depth: 2.6,
                width: 2.6,
                depth_at: 0.35,
                caudal: Caudal::Fluke,
                caudal_size: 1.8,
                dorsal: 0.15,
                pectoral: 1.8,
                coat: Coat::countershaded(0x2a3038, 0xd8dce0),
                fin: 0x2a3038,
                segments: 4,
                ..F
            },
        ),
        fish(
            "manta_ray",
            "Reef manta ray",
            &["ray"],
            1000.0,
            FishParams {
                length: 2.0,
                depth: 0.5,
                width: 0.9,
                depth_at: 0.3,
                caudal: Caudal::Whip,
                dorsal: 0.0,
                pectoral: 1.1,
                disc: true,
                coat: Coat::countershaded(0x1a1e24, 0xf0f0f0),
                fin: 0x1a1e24,
                segments: 2,
                ..F
            },
        ),
    ]
}
