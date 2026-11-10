//! Gene → body mapping: how a genome becomes a visible procedural creature.
//!
//! Prey are drawn as lagomorph-like quadrupeds and predators as canids, both
//! built from `organism-render` body plans (never blobs). Visible traits
//! follow the heritable genes:
//!
//! | Gene | Visible trait |
//! |---|---|
//! | `size` | overall scale, `∝ size^(1/3)` (mass → length) |
//! | `speed` | leg length and posture (long-legged, upright, lean when fast) |
//! | `temp_optimum` | coat hue on the thermal ramp shared with the ground tint (prey) |
//! | `perception` | ear length and head/eye size |
//!
//! Each body plan needs its own meshes, so the continuous genes are
//! quantised into a small number of *morphs* (bins); the exact values are
//! always shown in the inspector. Predators keep a fixed dark species colour
//! so they stay distinct from every prey hue.

use evolution::Species;
use evolution::genes::{GENES, PERCEPTION, SPEED, TEMP_OPTIMUM};
use organism_render::procedural::palette::{Coat, Pattern};
use organism_render::procedural::species::{BodyPlan, SpeciesSpec};

/// Speed bins (leg length / posture).
pub const SPEED_BINS: u8 = 4;
/// Perception bins (ear and head size).
pub const PERCEPTION_BINS: u8 = 3;
/// Thermal-optimum bins (coat hue), 4.5 °C wide over −5 … 40 °C.
pub const HUE_BINS: u8 = 10;

/// Base procedural species of prey morphs.
pub const PREY_BASE: &str = "rabbit";
/// Base procedural species of predator morphs.
pub const PREDATOR_BASE: &str = "fox";

/// Quantised visible phenotype of one organism.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Morph {
    /// Prey or predator.
    pub species: Species,
    /// Speed bin.
    pub speed: u8,
    /// Perception bin.
    pub perception: u8,
    /// Thermal-optimum bin (always 0 for predators).
    pub hue: u8,
}

/// Bin of a normalised value `t ∈ [0, 1]` among `bins` equal bins.
pub fn bin_of(t: f32, bins: u8) -> u8 {
    let bins = bins.max(1);
    let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.5 };
    ((t * f32::from(bins)) as u8).min(bins - 1)
}

/// Centre of `bin` in normalised units.
pub fn bin_center(bin: u8, bins: u8) -> f32 {
    (f32::from(bin.min(bins.max(1) - 1)) + 0.5) / f32::from(bins.max(1))
}

impl Morph {
    /// Morph of an organism from its normalised gene values.
    pub fn from_genes(species: Species, speed: f32, perception: f32, temp_optimum: f32) -> Self {
        Self {
            species,
            speed: bin_of(speed, SPEED_BINS),
            perception: bin_of(perception, PERCEPTION_BINS),
            hue: if species == Species::Prey { bin_of(temp_optimum, HUE_BINS) } else { 0 },
        }
    }

    /// Morph from a full normalised genome (slots in `evolution::genes` order).
    pub fn from_genome(species: Species, normalised: &[f32]) -> Self {
        let g = |i: usize| normalised.get(i).copied().unwrap_or(0.5);
        Self::from_genes(species, g(SPEED), g(PERCEPTION), g(TEMP_OPTIMUM))
    }

    /// Procedural species id registered for this morph.
    pub fn species_id(&self) -> String {
        match self.species {
            Species::Prey => format!("evolab_prey_s{}_p{}_h{}", self.speed, self.perception, self.hue),
            Species::Predator => format!("evolab_predator_s{}_p{}", self.speed, self.perception),
        }
    }

    /// Every morph of a species.
    pub fn all(species: Species) -> Vec<Morph> {
        let hues = if species == Species::Prey { HUE_BINS } else { 1 };
        let mut out = Vec::new();
        for speed in 0..SPEED_BINS {
            for perception in 0..PERCEPTION_BINS {
                for hue in 0..hues {
                    out.push(Morph { species, speed, perception, hue });
                }
            }
        }
        out
    }

    /// Distance between two morphs in bin steps (for fallbacks while a new
    /// morph is being built).
    pub fn distance(&self, other: &Morph) -> u32 {
        if self.species != other.species {
            return u32::MAX;
        }
        u32::from(self.speed.abs_diff(other.speed)) * 3
            + u32::from(self.perception.abs_diff(other.perception)) * 2
            + u32::from(self.hue.abs_diff(other.hue))
    }
}

/// Stops of the thermal ramp (normalised temperature over −5 … 40 °C, sRGB).
const THERMAL_STOPS: [(f32, [u8; 3]); 8] = [
    (0.0, [232, 240, 250]),
    (0.15, [150, 196, 236]),
    (0.3, [76, 150, 214]),
    (0.45, [58, 178, 156]),
    (0.58, [140, 192, 74]),
    (0.72, [226, 186, 58]),
    (0.86, [228, 122, 46]),
    (1.0, [196, 58, 46]),
];

/// Lowest temperature of the colour scale (°C) — the gene's minimum.
pub fn thermal_min() -> f32 {
    GENES[TEMP_OPTIMUM].min
}

/// Highest temperature of the colour scale (°C) — the gene's maximum.
pub fn thermal_max() -> f32 {
    GENES[TEMP_OPTIMUM].max
}

/// Normalised position of a temperature on the colour scale (clamped).
pub fn thermal_t(celsius: f32) -> f32 {
    let t = (celsius - thermal_min()) / (thermal_max() - thermal_min());
    if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.5 }
}

/// Vivid thermal ramp colour (sRGB, 0..1) at `t ∈ [0, 1]`.
pub fn thermal_rgb(t: f32) -> [f32; 3] {
    let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.5 };
    let i = THERMAL_STOPS.iter().rposition(|(s, _)| *s <= t).unwrap_or(0).min(THERMAL_STOPS.len() - 2);
    let (t0, c0) = THERMAL_STOPS[i];
    let (t1, c1) = THERMAL_STOPS[i + 1];
    let f = ((t - t0) / (t1 - t0)).clamp(0.0, 1.0);
    std::array::from_fn(|k| (f32::from(c0[k]) + (f32::from(c1[k]) - f32::from(c0[k])) * f) / 255.0)
}

/// Muted version of the ramp used to tint the ground by temperature (the
/// same hue a creature with that thermal optimum wears, mixed with soil).
pub fn ground_rgb(t: f32) -> [f32; 3] {
    const SOIL: [f32; 3] = [0.36, 0.33, 0.27];
    let c = thermal_rgb(t);
    std::array::from_fn(|k| (c[k] * 0.5 + SOIL[k] * 0.5) * 0.82)
}

/// `[r, g, b]` in 0..1 → `0xRRGGBB`.
pub fn hex_of(rgb: [f32; 3]) -> u32 {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    (b(rgb[0]) << 16) | (b(rgb[1]) << 8) | b(rgb[2])
}

fn mix_rgb(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t)
}

/// Display scale (the renderer's age factor) of an organism of body size
/// `size` (mass units): length grows with the cube root of mass. Organisms
/// are born at adult size in the model, so this is the only scaling.
pub fn display_scale(species: Species, size: f32) -> f32 {
    let size = if size.is_finite() { size.max(0.05) } else { 1.0 };
    let base = match species {
        Species::Prey => 2.0,
        Species::Predator => 1.3,
    };
    base * size.cbrt()
}

/// Builds the procedural species of a morph from the base species spec
/// (`rabbit` for prey, `fox` for predators). Returns `None` if the base is
/// not a quadruped.
pub fn morph_spec(base: &SpeciesSpec, morph: &Morph) -> Option<SpeciesSpec> {
    let BodyPlan::Quadruped(mut p) = base.plan else { return None };
    let speed = bin_center(morph.speed, SPEED_BINS);
    let perception = bin_center(morph.perception, PERCEPTION_BINS);
    match morph.species {
        Species::Prey => {
            // Speed: short, hunched legs → long legs and a more upright, leaner build.
            p.leg_length *= 0.8 + 0.75 * speed;
            p.fore_ratio = 0.56 + 0.32 * speed;
            p.body_depth *= 1.08 - 0.2 * speed;
            p.neck_angle = 0.45 + 0.35 * speed;
            // Perception: ear length and head (eye) size.
            p.ear_size *= 0.55 + 1.15 * perception;
            p.head_width *= 0.88 + 0.32 * perception;
            p.head_depth *= 0.88 + 0.32 * perception;
            // Thermal optimum: coat hue.
            let coat = thermal_rgb(bin_center(morph.hue, HUE_BINS));
            let belly = mix_rgb(coat, [0.96, 0.94, 0.9], 0.62);
            let tip = mix_rgb(coat, [0.1, 0.09, 0.08], 0.55);
            p.coat = Coat { base: hex_of(coat), belly: hex_of(belly), countershade: -0.1, pattern: Pattern::Plain };
            p.ear_tip = Some(hex_of(tip));
            p.tail_tip = Some(hex_of(belly));
        }
        Species::Predator => {
            p.leg_length *= 0.82 + 0.5 * speed;
            p.body_depth *= 1.06 - 0.14 * speed;
            p.ear_size *= 0.65 + 0.9 * perception;
            p.head_width *= 0.9 + 0.25 * perception;
            p.head_depth *= 0.9 + 0.25 * perception;
            p.coat = Coat {
                base: 0x3a3e47,
                belly: 0x8d929c,
                countershade: -0.15,
                pattern: Pattern::Saddle { color: 0x1c1e23 },
            };
            p.socks = Some(0x15161a);
            p.ear_tip = Some(0x15161a);
            p.muzzle = Some(0xb9bcc4);
            p.tail_tip = Some(0xe8e8ea);
            p.iris = 0xe8a030;
        }
    }
    let name = match morph.species {
        Species::Prey => "Evolution Lab prey",
        Species::Predator => "Evolution Lab predator",
    };
    Some(SpeciesSpec {
        id: morph.species_id(),
        name: name.into(),
        aliases: Vec::new(),
        mass_kg: base.mass_kg,
        plan: BodyPlan::Quadruped(p),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use organism_render::procedural::species::SpeciesTable;

    #[test]
    fn bins_cover_the_range() {
        assert_eq!(bin_of(0.0, 4), 0);
        assert_eq!(bin_of(0.249, 4), 0);
        assert_eq!(bin_of(0.25, 4), 1);
        assert_eq!(bin_of(1.0, 4), 3, "the upper edge belongs to the last bin");
        assert_eq!(bin_of(-3.0, 4), 0);
        assert_eq!(bin_of(f32::NAN, 4), 2, "non-finite values fall in the middle");
        assert!((bin_center(0, 4) - 0.125).abs() < 1e-6);
        assert!((bin_center(3, 4) - 0.875).abs() < 1e-6);
        for b in 0..HUE_BINS {
            assert_eq!(bin_of(bin_center(b, HUE_BINS), HUE_BINS), b);
        }
    }

    #[test]
    fn morphs_follow_the_genes() {
        let slow_cold = Morph::from_genes(Species::Prey, 0.05, 0.1, 0.0);
        let fast_hot = Morph::from_genes(Species::Prey, 0.95, 0.9, 1.0);
        assert_eq!((slow_cold.speed, slow_cold.perception, slow_cold.hue), (0, 0, 0));
        assert_eq!((fast_hot.speed, fast_hot.perception, fast_hot.hue), (3, 2, HUE_BINS - 1));
        let predator = Morph::from_genes(Species::Predator, 0.5, 0.5, 1.0);
        assert_eq!(predator.hue, 0, "predators keep their species colour");
        assert_ne!(slow_cold.species_id(), fast_hot.species_id());
        assert!(predator.species_id().starts_with("evolab_predator"));
        assert_eq!(Morph::all(Species::Prey).len(), usize::from(SPEED_BINS * PERCEPTION_BINS * HUE_BINS));
        assert_eq!(Morph::all(Species::Predator).len(), usize::from(SPEED_BINS * PERCEPTION_BINS));
        assert_eq!(slow_cold.distance(&slow_cold), 0);
        assert_eq!(slow_cold.distance(&predator), u32::MAX);
        let mut genome = vec![0.5; evolution::genes::GENE_COUNT];
        genome[SPEED] = 1.0;
        assert_eq!(Morph::from_genome(Species::Prey, &genome).speed, SPEED_BINS - 1);
    }

    #[test]
    fn thermal_ramp_is_continuous_and_matches_the_gene_range() {
        assert_eq!(thermal_t(thermal_min()), 0.0);
        assert_eq!(thermal_t(thermal_max()), 1.0);
        assert_eq!(thermal_t(-100.0), 0.0);
        assert!((thermal_t(17.5) - 0.5).abs() < 1e-6);
        let mut prev = thermal_rgb(0.0);
        for i in 1..=200 {
            let c = thermal_rgb(i as f32 / 200.0);
            for k in 0..3 {
                assert!((c[k] - prev[k]).abs() < 0.08, "jump at {i}");
                assert!((0.0..=1.0).contains(&c[k]));
            }
            prev = c;
        }
        // Cold is blue-white, hot is red.
        let (cold, hot) = (thermal_rgb(0.25), thermal_rgb(1.0));
        assert!(cold[2] > cold[0] && hot[0] > hot[2]);
        // The ground tint keeps the hue order but is darker.
        let g = ground_rgb(1.0);
        assert!(g[0] > g[2] && g[0] < hot[0]);
        assert_eq!(hex_of([1.0, 0.0, 0.5]), 0xff0080);
    }

    #[test]
    fn display_scale_grows_with_the_cube_root_of_size() {
        let a = display_scale(Species::Prey, 1.0);
        let b = display_scale(Species::Prey, 8.0);
        assert!((b / a - 2.0).abs() < 1e-5);
        assert!(display_scale(Species::Predator, 1.0) < a, "fox base is larger than the rabbit base");
        assert!(display_scale(Species::Prey, f32::NAN).is_finite());
    }

    #[test]
    fn every_morph_is_a_valid_procedural_species() {
        let table = SpeciesTable::builtin();
        let prey = table.get(PREY_BASE).expect("rabbit exists");
        let predator = table.get(PREDATOR_BASE).expect("fox exists");
        for morph in Morph::all(Species::Prey).iter().chain(Morph::all(Species::Predator).iter()) {
            let base = if morph.species == Species::Prey { prey } else { predator };
            let spec = morph_spec(base, morph).expect("quadruped base");
            assert_eq!(spec.id, morph.species_id());
            spec.validate().unwrap_or_else(|e| panic!("{morph:?}: {e}"));
        }
    }

    #[test]
    fn faster_morphs_have_longer_legs_and_keener_morphs_longer_ears() {
        let table = SpeciesTable::builtin();
        let base = table.get(PREY_BASE).unwrap();
        let params = |m: Morph| match morph_spec(base, &m).unwrap().plan {
            BodyPlan::Quadruped(p) => p,
            _ => unreachable!(),
        };
        let slow = params(Morph { species: Species::Prey, speed: 0, perception: 0, hue: 3 });
        let fast = params(Morph { species: Species::Prey, speed: 3, perception: 2, hue: 3 });
        assert!(fast.leg_length > slow.leg_length * 1.4);
        assert!(fast.fore_ratio > slow.fore_ratio);
        assert!(fast.ear_size > slow.ear_size * 1.8);
        assert!(fast.head_width > slow.head_width);
        let hot = params(Morph { species: Species::Prey, speed: 0, perception: 0, hue: HUE_BINS - 1 });
        assert_ne!(hot.coat.base, slow.coat.base, "hue follows the thermal optimum");
    }
}
