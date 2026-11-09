//! Ready-made particle configurations. Values are chosen to be physically
//! plausible where a physical quantity exists (raindrops fall at about
//! 9.8 m/s, snowflakes at about 1 m/s, both reached through drag) and are
//! otherwise artistic. Additive presets use colour values above 1.0 so they
//! bloom on HDR cameras. Turbulence is a velocity diffusion (m/s per √s, see
//! [`super::sim`]); with drag `k` it gives a steady velocity jitter of
//! `turbulence / √(2k)` per axis (for example about 0.07 m/s for snow).

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use super::sim::{EmitterShape, ParticleBlend, ParticleConfig};

/// Built-in particle effects.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParticlePreset {
    /// Flames: buoyant, short-lived, additive.
    Fire,
    /// Smoke: slow rising, expanding, wind-driven.
    Smoke,
    /// Embers: sparse sparks with turbulent arcs, velocity-stretched.
    Embers,
    /// Rain: streaks at terminal velocity over a 50 × 50 area.
    Rain,
    /// Snow: slow fluttering flakes over a 50 × 50 area.
    Snow,
    /// Splash: droplets thrown up and falling back (use bursts for impacts).
    Splash,
    /// Dust: low drifting haze.
    Dust,
}

fn rgba(r: f32, g: f32, b: f32, a: f32) -> LinearRgba {
    LinearRgba::new(r, g, b, a)
}

impl ParticlePreset {
    /// Every preset, in display order.
    pub const ALL: [ParticlePreset; 7] =
        [Self::Fire, Self::Smoke, Self::Embers, Self::Rain, Self::Snow, Self::Splash, Self::Dust];

    /// Display label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Fire => "Fire",
            Self::Smoke => "Smoke",
            Self::Embers => "Embers",
            Self::Rain => "Rain",
            Self::Snow => "Snow",
            Self::Splash => "Splash",
            Self::Dust => "Dust",
        }
    }

    /// The preset's configuration.
    pub fn config(self) -> ParticleConfig {
        match self {
            Self::Fire => ParticleConfig {
                rate: 70.0,
                max_particles: 400,
                lifetime: (0.45, 1.0),
                shape: EmitterShape::Disc { radius: 0.45 },
                velocity: Vec3::new(0.0, 1.6, 0.0),
                velocity_jitter: Vec3::new(0.35, 0.5, 0.35),
                gravity: Vec3::new(0.0, 2.5, 0.0),
                drag: 1.2,
                wind_influence: 0.6,
                turbulence: 0.19,
                color_over_life: vec![
                    (0.0, rgba(3.0, 2.2, 0.8, 0.0)),
                    (0.12, rgba(3.2, 1.5, 0.3, 0.9)),
                    (0.55, rgba(1.8, 0.45, 0.06, 0.55)),
                    (1.0, rgba(0.25, 0.05, 0.02, 0.0)),
                ],
                size_over_life: vec![(0.0, 0.45), (0.3, 0.75), (1.0, 0.2)],
                blend: ParticleBlend::Additive,
                stretch: 0.0,
            },
            Self::Smoke => ParticleConfig {
                rate: 12.0,
                max_particles: 160,
                lifetime: (3.0, 5.0),
                shape: EmitterShape::Disc { radius: 0.35 },
                velocity: Vec3::new(0.0, 1.3, 0.0),
                velocity_jitter: Vec3::new(0.25, 0.3, 0.25),
                gravity: Vec3::new(0.0, 0.5, 0.0),
                drag: 0.5,
                wind_influence: 1.0,
                turbulence: 0.045,
                color_over_life: vec![
                    (0.0, rgba(0.08, 0.08, 0.08, 0.0)),
                    (0.15, rgba(0.12, 0.12, 0.12, 0.55)),
                    (0.6, rgba(0.22, 0.22, 0.23, 0.3)),
                    (1.0, rgba(0.35, 0.35, 0.37, 0.0)),
                ],
                size_over_life: vec![(0.0, 0.6), (1.0, 3.2)],
                blend: ParticleBlend::Alpha,
                stretch: 0.0,
            },
            Self::Embers => ParticleConfig {
                rate: 10.0,
                max_particles: 120,
                lifetime: (1.5, 3.0),
                shape: EmitterShape::Disc { radius: 0.4 },
                velocity: Vec3::new(0.0, 3.2, 0.0),
                velocity_jitter: Vec3::new(0.8, 1.0, 0.8),
                gravity: Vec3::new(0.0, -1.2, 0.0),
                drag: 0.7,
                wind_influence: 0.9,
                turbulence: 0.22,
                color_over_life: vec![
                    (0.0, rgba(4.0, 1.8, 0.4, 1.0)),
                    (0.7, rgba(2.0, 0.5, 0.05, 0.8)),
                    (1.0, rgba(0.6, 0.1, 0.0, 0.0)),
                ],
                size_over_life: vec![(0.0, 0.07), (1.0, 0.035)],
                blend: ParticleBlend::Additive,
                stretch: 0.04,
            },
            Self::Rain => ParticleConfig {
                rate: 1500.0,
                max_particles: 3000,
                lifetime: (1.4, 1.6),
                shape: EmitterShape::Box { half_extents: Vec3::new(25.0, 0.5, 25.0) },
                velocity: Vec3::new(0.0, -9.5, 0.0),
                velocity_jitter: Vec3::new(0.2, 0.4, 0.2),
                gravity: Vec3::new(0.0, -9.81, 0.0),
                drag: 1.0,
                wind_influence: 1.0,
                turbulence: 0.0,
                color_over_life: vec![(0.0, rgba(0.55, 0.6, 0.7, 0.45)), (1.0, rgba(0.55, 0.6, 0.7, 0.45))],
                size_over_life: vec![(0.0, 0.025)],
                blend: ParticleBlend::Alpha,
                stretch: 0.035,
            },
            Self::Snow => ParticleConfig {
                rate: 300.0,
                max_particles: 2500,
                lifetime: (6.0, 8.0),
                shape: EmitterShape::Box { half_extents: Vec3::new(25.0, 0.5, 25.0) },
                velocity: Vec3::new(0.0, -1.0, 0.0),
                velocity_jitter: Vec3::new(0.3, 0.2, 0.3),
                gravity: Vec3::new(0.0, -9.81, 0.0),
                drag: 9.0,
                wind_influence: 1.0,
                turbulence: 0.30,
                color_over_life: vec![
                    (0.0, rgba(0.95, 0.97, 1.0, 0.0)),
                    (0.05, rgba(0.95, 0.97, 1.0, 0.95)),
                    (0.9, rgba(0.95, 0.97, 1.0, 0.95)),
                    (1.0, rgba(0.95, 0.97, 1.0, 0.0)),
                ],
                size_over_life: vec![(0.0, 0.09)],
                blend: ParticleBlend::Alpha,
                stretch: 0.0,
            },
            Self::Splash => ParticleConfig {
                rate: 40.0,
                max_particles: 300,
                lifetime: (0.4, 0.8),
                shape: EmitterShape::Disc { radius: 0.2 },
                velocity: Vec3::new(0.0, 3.5, 0.0),
                velocity_jitter: Vec3::new(1.8, 1.2, 1.8),
                gravity: Vec3::new(0.0, -9.81, 0.0),
                drag: 0.4,
                wind_influence: 0.3,
                turbulence: 0.0,
                color_over_life: vec![(0.0, rgba(0.7, 0.82, 0.95, 0.85)), (1.0, rgba(0.7, 0.82, 0.95, 0.0))],
                size_over_life: vec![(0.0, 0.07), (1.0, 0.03)],
                blend: ParticleBlend::Alpha,
                stretch: 0.03,
            },
            Self::Dust => ParticleConfig {
                rate: 18.0,
                max_particles: 200,
                lifetime: (3.0, 6.0),
                shape: EmitterShape::Box { half_extents: Vec3::new(2.5, 0.15, 2.5) },
                velocity: Vec3::new(0.4, 0.3, 0.0),
                velocity_jitter: Vec3::new(0.4, 0.25, 0.4),
                gravity: Vec3::new(0.0, -0.3, 0.0),
                drag: 1.2,
                wind_influence: 1.0,
                turbulence: 0.06,
                color_over_life: vec![
                    (0.0, rgba(0.42, 0.33, 0.22, 0.0)),
                    (0.2, rgba(0.48, 0.38, 0.26, 0.35)),
                    (1.0, rgba(0.55, 0.45, 0.32, 0.0)),
                ],
                size_over_life: vec![(0.0, 0.4), (1.0, 1.8)],
                blend: ParticleBlend::Alpha,
                stretch: 0.0,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::particles::sim::ParticleState;

    #[test]
    fn presets_are_well_formed() {
        for preset in ParticlePreset::ALL {
            let c = preset.config();
            assert!(c.lifetime.0 > 0.0 && c.lifetime.0 <= c.lifetime.1, "{preset:?}");
            assert!(c.max_particles > 0);
            assert!(c.color_over_life.windows(2).all(|w| w[0].0 <= w[1].0), "{preset:?} colour stops sorted");
            assert!(c.size_over_life.windows(2).all(|w| w[0].0 <= w[1].0), "{preset:?} size stops sorted");
            assert!(!preset.label().is_empty());
        }
    }

    #[test]
    fn rain_and_snow_fall_at_physical_terminal_speeds() {
        for (preset, speed) in [(ParticlePreset::Rain, 9.81), (ParticlePreset::Snow, 9.81 / 9.0)] {
            let cfg = ParticleConfig { turbulence: 0.0, velocity_jitter: Vec3::ZERO, rate: 0.0, ..preset.config() };
            let mut s = ParticleState::new(1);
            s.burst(&cfg, &GlobalTransform::IDENTITY, 1);
            for _ in 0..60 {
                s.step(&cfg, &GlobalTransform::IDENTITY, false, Vec3::ZERO, 1.0 / 60.0);
            }
            let v = s.particles()[0].velocity;
            assert!(v.y < 0.0, "{preset:?} falls");
            assert!((v.y.abs() - speed).abs() < 0.4, "{preset:?}: {v:?}");
        }
    }

    #[test]
    fn fire_rises() {
        let cfg = ParticleConfig { turbulence: 0.0, rate: 0.0, ..ParticlePreset::Fire.config() };
        let mut s = ParticleState::new(2);
        s.burst(&cfg, &GlobalTransform::IDENTITY, 20);
        for _ in 0..20 {
            s.step(&cfg, &GlobalTransform::IDENTITY, false, Vec3::ZERO, 1.0 / 60.0);
        }
        assert!(s.particles().iter().all(|p| p.position.y > 0.0));
    }
}
