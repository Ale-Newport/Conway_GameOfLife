//! CPU particle simulation (pure, deterministic for a given seed and time
//! steps). Particles are visual only; they never feed back into a model.
//!
//! Per step of length `dt` (clamped to 0.1 s), for each particle:
//! 1. velocity under constant acceleration `g` and linear drag `k` towards the
//!    moving air (`air = wind × wind_influence`), integrated exactly:
//!    `v = v∞ + (v − v∞) e^(−k dt)` with terminal velocity `v∞ = air + g / k`
//!    (`v += g dt` when `k = 0`);
//! 2. random forcing as white-noise acceleration (Euler–Maruyama):
//!    `v += σ √(3 dt) ξ` with ξ uniform in `[−1, 1]³` (unit variance per
//!    component), so after one second of forcing each velocity component has
//!    changed by σ = `turbulence` (RMS) whatever the frame rate; with drag `k`
//!    the jitter settles at `σ / √(2k)` (Ornstein–Uhlenbeck);
//! 3. `x += v dt` (semi-implicit Euler); `age += dt`; particles past their
//!    lifetime are removed.
//!
//! Emission uses an accumulator so `rate × time` particles are produced
//! regardless of frame rate, capped at `max_particles`. Each new particle is
//! born at a uniformly random instant inside the step and integrated only for
//! the remainder of it, so a step never ages a newborn by more than `dt`.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// Largest time step integrated at once (longer frames are clamped).
pub const MAX_STEP: f32 = 0.1;

/// Where new particles appear, in the emitter's local space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EmitterShape {
    /// At the emitter origin.
    Point,
    /// Uniformly inside a sphere.
    Sphere {
        /// Radius.
        radius: f32,
    },
    /// Uniformly inside an axis-aligned box.
    Box {
        /// Half extents.
        half_extents: Vec3,
    },
    /// Uniformly on a horizontal disc (XZ plane).
    Disc {
        /// Radius.
        radius: f32,
    },
}

/// How particles are composited.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParticleBlend {
    /// Additive light (fire, embers): order independent.
    Additive,
    /// Alpha blended (smoke, rain, snow, dust): sorted back to front.
    Alpha,
}

/// Behaviour of one emitter.
#[derive(Clone, Debug, PartialEq)]
pub struct ParticleConfig {
    /// Particles per second while emitting.
    pub rate: f32,
    /// Upper bound on live particles.
    pub max_particles: usize,
    /// Lifetime range in seconds (min, max).
    pub lifetime: (f32, f32),
    /// Spawn region.
    pub shape: EmitterShape,
    /// Mean initial velocity (world space, m/s).
    pub velocity: Vec3,
    /// Uniform ± jitter of the initial velocity per axis.
    pub velocity_jitter: Vec3,
    /// Constant acceleration (gravity or buoyancy), m/s².
    pub gravity: Vec3,
    /// Linear drag rate towards the air velocity, 1/s.
    pub drag: f32,
    /// Fraction of [`super::ParticleWind`] the surrounding air moves with.
    pub wind_influence: f32,
    /// Random forcing strength σ in m/s per √s: the RMS change of each
    /// velocity component after one second of white-noise acceleration
    /// (frame-rate independent, see the module docs).
    pub turbulence: f32,
    /// Colour stops over normalised age `t ∈ [0, 1]` (sorted by `t`).
    pub color_over_life: Vec<(f32, LinearRgba)>,
    /// Size stops (world units) over normalised age.
    pub size_over_life: Vec<(f32, f32)>,
    /// Compositing mode.
    pub blend: ParticleBlend,
    /// Velocity stretching: quads elongate by `speed × stretch` along the
    /// motion (0 = round camera-facing quads).
    pub stretch: f32,
}

impl ParticleConfig {
    /// The same effect in a world whose length unit is `1 / factor` as large
    /// (e.g. `scaled(4.0)` makes a 4× larger fire). Lengths, velocities,
    /// accelerations, turbulence and sizes are multiplied by `factor`; times,
    /// rates and drag stay the same, so the motion is geometrically similar.
    /// Non-positive or non-finite factors return the configuration unchanged.
    pub fn scaled(&self, factor: f32) -> Self {
        if !(factor.is_finite() && factor > 0.0) {
            return self.clone();
        }
        let shape = match self.shape {
            EmitterShape::Point => EmitterShape::Point,
            EmitterShape::Sphere { radius } => EmitterShape::Sphere { radius: radius * factor },
            EmitterShape::Box { half_extents } => EmitterShape::Box { half_extents: half_extents * factor },
            EmitterShape::Disc { radius } => EmitterShape::Disc { radius: radius * factor },
        };
        Self {
            shape,
            velocity: self.velocity * factor,
            velocity_jitter: self.velocity_jitter * factor,
            gravity: self.gravity * factor,
            turbulence: self.turbulence * factor,
            size_over_life: self.size_over_life.iter().map(|&(t, s)| (t, s * factor)).collect(),
            ..self.clone()
        }
    }
}

/// One live particle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Particle {
    /// World position.
    pub position: Vec3,
    /// World velocity.
    pub velocity: Vec3,
    /// Seconds since spawn.
    pub age: f32,
    /// Seconds until removal.
    pub lifetime: f32,
    /// Per-particle size multiplier.
    pub size_scale: f32,
}

impl Particle {
    /// Normalised age in `[0, 1]`.
    pub fn life_fraction(&self) -> f32 {
        if self.lifetime > 0.0 { (self.age / self.lifetime).clamp(0.0, 1.0) } else { 1.0 }
    }
}

/// Small deterministic generator (SplitMix64) for visual randomness.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParticleRng(u64);

impl ParticleRng {
    /// Seeds the generator.
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    pub fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Uniform in `[-1, 1)`.
    pub fn signed(&mut self) -> f32 {
        self.unit() * 2.0 - 1.0
    }

    fn signed_vec3(&mut self) -> Vec3 {
        Vec3::new(self.signed(), self.signed(), self.signed())
    }
}

/// Piecewise-linear interpolation of colour stops (clamped at the ends).
pub fn sample_color(stops: &[(f32, LinearRgba)], t: f32) -> LinearRgba {
    match stops {
        [] => LinearRgba::WHITE,
        [only] => only.1,
        _ => {
            let t = t.clamp(0.0, 1.0);
            if t <= stops[0].0 {
                return stops[0].1;
            }
            for pair in stops.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                if t <= b.0 {
                    let f = if b.0 > a.0 { (t - a.0) / (b.0 - a.0) } else { 1.0 };
                    return a.1.mix(&b.1, f);
                }
            }
            stops[stops.len() - 1].1
        }
    }
}

/// Piecewise-linear interpolation of scalar stops (clamped at the ends).
pub fn sample_scalar(stops: &[(f32, f32)], t: f32) -> f32 {
    match stops {
        [] => 1.0,
        [only] => only.1,
        _ => {
            let t = t.clamp(0.0, 1.0);
            if t <= stops[0].0 {
                return stops[0].1;
            }
            for pair in stops.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                if t <= b.0 {
                    let f = if b.0 > a.0 { (t - a.0) / (b.0 - a.0) } else { 1.0 };
                    return a.1 + (b.1 - a.1) * f;
                }
            }
            stops[stops.len() - 1].1
        }
    }
}

/// Live particles of one emitter plus its emission state.
#[derive(Clone, Debug, PartialEq)]
pub struct ParticleState {
    particles: Vec<Particle>,
    accumulator: f32,
    rng: ParticleRng,
}

impl ParticleState {
    /// Empty state with a seed for the emitter's randomness.
    pub fn new(seed: u64) -> Self {
        Self { particles: Vec::new(), accumulator: 0.0, rng: ParticleRng::new(seed) }
    }

    /// Live particles.
    pub fn particles(&self) -> &[Particle] {
        &self.particles
    }

    /// Removes every particle.
    pub fn clear(&mut self) {
        self.particles.clear();
        self.accumulator = 0.0;
    }

    fn spawn_one(&mut self, config: &ParticleConfig, emitter: &GlobalTransform) -> Particle {
        let rng = &mut self.rng;
        let local = match config.shape {
            EmitterShape::Point => Vec3::ZERO,
            EmitterShape::Sphere { radius } => loop {
                let p = rng.signed_vec3();
                if p.length_squared() <= 1.0 {
                    break p * radius;
                }
            },
            EmitterShape::Box { half_extents } => rng.signed_vec3() * half_extents,
            EmitterShape::Disc { radius } => {
                let r = radius * rng.unit().sqrt();
                let a = rng.unit() * std::f32::consts::TAU;
                Vec3::new(r * a.cos(), 0.0, r * a.sin())
            }
        };
        let (lo, hi) = (config.lifetime.0.max(0.0), config.lifetime.1.max(config.lifetime.0.max(0.0)));
        Particle {
            position: emitter.transform_point(local),
            velocity: config.velocity + rng.signed_vec3() * config.velocity_jitter,
            age: 0.0,
            lifetime: lo + (hi - lo) * rng.unit(),
            size_scale: 0.8 + 0.4 * rng.unit(),
        }
    }

    /// Spawns up to `count` particles immediately (capped at `max_particles`).
    pub fn burst(&mut self, config: &ParticleConfig, emitter: &GlobalTransform, count: usize) {
        let room = config.max_particles.saturating_sub(self.particles.len());
        for _ in 0..count.min(room) {
            let p = self.spawn_one(config, emitter);
            self.particles.push(p);
        }
    }

    /// Advances the particles by `dt` seconds and emits new ones.
    pub fn step(&mut self, config: &ParticleConfig, emitter: &GlobalTransform, emitting: bool, wind: Vec3, dt: f32) {
        let dt = if dt.is_finite() { dt.clamp(0.0, MAX_STEP) } else { 0.0 };
        let motion = Motion::new(config, wind);
        for p in &mut self.particles {
            motion.advance(p, dt, &mut self.rng);
        }
        self.particles.retain(|p| p.age < p.lifetime);
        if emitting && config.rate > 0.0 {
            self.accumulator += config.rate * dt;
            let due = self.accumulator.floor();
            self.accumulator -= due;
            let room = config.max_particles.saturating_sub(self.particles.len());
            let count = (due as usize).min(room);
            for _ in 0..count {
                let mut p = self.spawn_one(config, emitter);
                // Born at a uniformly random instant of this step: integrate
                // only the remaining part so emission looks continuous.
                let remaining = dt * self.rng.unit();
                motion.advance(&mut p, remaining, &mut self.rng);
                if p.age < p.lifetime {
                    self.particles.push(p);
                }
            }
        }
    }
}

/// Per-step motion constants of one emitter (see the module docs).
struct Motion {
    gravity: Vec3,
    drag: f32,
    terminal: Vec3,
    turbulence: f32,
}

impl Motion {
    fn new(config: &ParticleConfig, wind: Vec3) -> Self {
        let air = wind * config.wind_influence;
        let drag = if config.drag.is_finite() { config.drag.max(0.0) } else { 0.0 };
        let terminal = if drag > 0.0 { air + config.gravity / drag } else { Vec3::ZERO };
        let turbulence = if config.turbulence.is_finite() { config.turbulence.max(0.0) } else { 0.0 };
        Self { gravity: config.gravity, drag, terminal, turbulence }
    }

    /// Integrates one particle over `dt` seconds.
    fn advance(&self, p: &mut Particle, dt: f32, rng: &mut ParticleRng) {
        if self.drag > 0.0 {
            p.velocity = self.terminal + (p.velocity - self.terminal) * (-self.drag * dt).exp();
        } else {
            p.velocity += self.gravity * dt;
        }
        if self.turbulence > 0.0 {
            // Uniform ξ in [-1, 1] has variance 1/3, hence √(3 dt).
            p.velocity += rng.signed_vec3() * (self.turbulence * (3.0 * dt).sqrt());
        }
        p.position += p.velocity * dt;
        p.age += dt;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> ParticleConfig {
        ParticleConfig {
            rate: 10.0,
            max_particles: 1000,
            lifetime: (100.0, 100.0),
            shape: EmitterShape::Point,
            velocity: Vec3::ZERO,
            velocity_jitter: Vec3::ZERO,
            gravity: Vec3::ZERO,
            drag: 0.0,
            wind_influence: 0.0,
            turbulence: 0.0,
            color_over_life: vec![(0.0, LinearRgba::WHITE)],
            size_over_life: vec![(0.0, 1.0)],
            blend: ParticleBlend::Alpha,
            stretch: 0.0,
        }
    }

    #[test]
    fn emission_rate_is_frame_rate_independent() {
        for dt in [0.05f32, 0.01, 1.0 / 60.0] {
            let mut s = ParticleState::new(1);
            let steps = (1.0 / dt).round() as usize;
            for _ in 0..steps {
                s.step(&config(), &GlobalTransform::IDENTITY, true, Vec3::ZERO, dt);
            }
            assert!((s.particles().len() as i64 - 10).abs() <= 1, "dt {dt}: {}", s.particles().len());
        }
    }

    #[test]
    fn not_emitting_spawns_nothing_and_cap_holds() {
        let mut s = ParticleState::new(1);
        s.step(&config(), &GlobalTransform::IDENTITY, false, Vec3::ZERO, 0.1);
        assert!(s.particles().is_empty());
        let capped = ParticleConfig { rate: 10_000.0, max_particles: 50, ..config() };
        for _ in 0..10 {
            s.step(&capped, &GlobalTransform::IDENTITY, true, Vec3::ZERO, 0.1);
        }
        assert_eq!(s.particles().len(), 50);
        s.burst(&capped, &GlobalTransform::IDENTITY, 10);
        assert_eq!(s.particles().len(), 50);
    }

    #[test]
    fn particles_expire_after_lifetime() {
        let cfg = ParticleConfig { lifetime: (0.5, 0.5), rate: 0.0, ..config() };
        let mut s = ParticleState::new(2);
        s.burst(&cfg, &GlobalTransform::IDENTITY, 5);
        for _ in 0..4 {
            s.step(&cfg, &GlobalTransform::IDENTITY, false, Vec3::ZERO, 0.1);
        }
        assert_eq!(s.particles().len(), 5);
        s.step(&cfg, &GlobalTransform::IDENTITY, false, Vec3::ZERO, 0.1);
        assert!(s.particles().is_empty());
    }

    #[test]
    fn ballistic_motion_without_drag() {
        let cfg = ParticleConfig {
            rate: 0.0,
            velocity: Vec3::new(2.0, 0.0, 0.0),
            gravity: Vec3::new(0.0, -10.0, 0.0),
            ..config()
        };
        let mut s = ParticleState::new(3);
        s.burst(&cfg, &GlobalTransform::from_translation(Vec3::new(0.0, 5.0, 0.0)), 1);
        let dt = 0.01;
        for _ in 0..100 {
            s.step(&cfg, &GlobalTransform::IDENTITY, false, Vec3::ZERO, dt);
        }
        let p = s.particles()[0];
        assert!((p.velocity - Vec3::new(2.0, -10.0, 0.0)).length() < 1e-4);
        // Semi-implicit Euler: x = x0 + v0 t + g t²/2 + g t dt/2.
        let expected_y = 5.0 - 0.5 * 10.0 - 0.5 * 10.0 * 1.0 * dt;
        assert!((p.position.x - 2.0).abs() < 1e-4);
        assert!((p.position.y - expected_y).abs() < 1e-3, "{}", p.position.y);
    }

    #[test]
    fn drag_reaches_terminal_and_wind_velocity() {
        let cfg = ParticleConfig {
            rate: 0.0,
            gravity: Vec3::new(0.0, -9.81, 0.0),
            drag: 9.0,
            wind_influence: 1.0,
            ..config()
        };
        let mut s = ParticleState::new(4);
        s.burst(&cfg, &GlobalTransform::IDENTITY, 1);
        for _ in 0..300 {
            s.step(&cfg, &GlobalTransform::IDENTITY, false, Vec3::new(3.0, 0.0, 0.0), 1.0 / 60.0);
        }
        let v = s.particles()[0].velocity;
        assert!((v.x - 3.0).abs() < 1e-3, "drifts with the wind: {v:?}");
        // Terminal velocity g/k (exact integration).
        assert!((v.y + 9.81 / 9.0).abs() < 1e-3, "terminal velocity: {v:?}");
    }

    /// Per-component velocity variance of all particles after `seconds` of
    /// forcing at time step `dt` (no drag, no gravity, particles at rest).
    fn turbulent_variance(turbulence: f32, dt: f32, seconds: f32) -> f32 {
        let cfg = ParticleConfig { rate: 0.0, max_particles: 10_000, turbulence, ..config() };
        let mut s = ParticleState::new(21);
        s.burst(&cfg, &GlobalTransform::IDENTITY, 4000);
        let steps = (seconds / dt).round() as usize;
        for _ in 0..steps {
            s.step(&cfg, &GlobalTransform::IDENTITY, false, Vec3::ZERO, dt);
        }
        let n = (s.particles().len() * 3) as f32;
        s.particles().iter().map(|p| p.velocity.length_squared()).sum::<f32>() / n
    }

    #[test]
    fn turbulence_is_frame_rate_independent_white_noise() {
        // Euler–Maruyama: Var(v) = σ² t per component, whatever the step.
        let sigma = 0.8;
        for dt in [1.0 / 30.0, 1.0 / 60.0, 1.0 / 240.0] {
            let var = turbulent_variance(sigma, dt, 1.0);
            assert!((var / (sigma * sigma) - 1.0).abs() < 0.08, "dt {dt}: variance {var}");
        }
        let var = turbulent_variance(sigma, 1.0 / 60.0, 2.0);
        assert!((var / (2.0 * sigma * sigma) - 1.0).abs() < 0.08, "variance grows linearly: {var}");
    }

    #[test]
    fn turbulent_jitter_settles_at_ornstein_uhlenbeck_level() {
        // With drag k the stationary per-component variance is σ² / (2k).
        let (sigma, k) = (1.2f32, 3.0f32);
        let cfg = ParticleConfig { rate: 0.0, max_particles: 10_000, turbulence: sigma, drag: k, ..config() };
        let mut s = ParticleState::new(5);
        s.burst(&cfg, &GlobalTransform::IDENTITY, 4000);
        for _ in 0..240 {
            s.step(&cfg, &GlobalTransform::IDENTITY, false, Vec3::ZERO, 1.0 / 60.0);
        }
        let var = s.particles().iter().map(|p| p.velocity.length_squared()).sum::<f32>() / (4000.0 * 3.0);
        let expected = sigma * sigma / (2.0 * k);
        assert!((var / expected - 1.0).abs() < 0.1, "variance {var}, expected {expected}");
    }

    #[test]
    fn newborn_particles_are_younger_than_one_step() {
        let cfg = ParticleConfig { rate: 1000.0, velocity: Vec3::new(0.0, 2.0, 0.0), ..config() };
        let mut s = ParticleState::new(8);
        let dt = 0.05;
        s.step(&cfg, &GlobalTransform::IDENTITY, true, Vec3::ZERO, dt);
        assert_eq!(s.particles().len(), 50);
        for p in s.particles() {
            assert!((0.0..=dt).contains(&p.age), "age {} exceeds the step", p.age);
            // Constant velocity: the distance travelled matches the age.
            assert!((p.position.y - 2.0 * p.age).abs() < 1e-5, "{p:?}");
        }
        // Particles emitted in later steps keep spreading over the step.
        s.step(&cfg, &GlobalTransform::IDENTITY, true, Vec3::ZERO, dt);
        let young = s.particles().iter().filter(|p| p.age < dt).count();
        assert_eq!(young, 50);
    }

    #[test]
    fn spawn_shapes_stay_inside() {
        let shapes = [
            EmitterShape::Sphere { radius: 2.0 },
            EmitterShape::Box { half_extents: Vec3::new(1.0, 0.5, 3.0) },
            EmitterShape::Disc { radius: 1.5 },
        ];
        for shape in shapes {
            let cfg = ParticleConfig { shape, rate: 0.0, ..config() };
            let mut s = ParticleState::new(5);
            s.burst(&cfg, &GlobalTransform::from_translation(Vec3::new(10.0, 0.0, 0.0)), 200);
            for p in s.particles() {
                let local = p.position - Vec3::new(10.0, 0.0, 0.0);
                let inside = match shape {
                    EmitterShape::Sphere { radius } => local.length() <= radius + 1e-5,
                    EmitterShape::Box { half_extents } => local.abs().cmple(half_extents + 1e-5).all(),
                    EmitterShape::Disc { radius } => local.y == 0.0 && local.xz().length() <= radius + 1e-5,
                    EmitterShape::Point => local == Vec3::ZERO,
                };
                assert!(inside, "{shape:?}: {local:?}");
            }
        }
    }

    #[test]
    fn same_seed_same_particles() {
        let cfg = ParticleConfig { velocity_jitter: Vec3::ONE, turbulence: 2.0, ..config() };
        let run = |seed| {
            let mut s = ParticleState::new(seed);
            for _ in 0..30 {
                s.step(&cfg, &GlobalTransform::IDENTITY, true, Vec3::X, 0.03);
            }
            s
        };
        assert_eq!(run(9), run(9));
        assert_ne!(run(9), run(10));
    }

    #[test]
    fn over_life_curves_interpolate() {
        let stops = [
            (0.0, LinearRgba::new(0.0, 0.0, 0.0, 0.0)),
            (0.5, LinearRgba::new(1.0, 1.0, 1.0, 1.0)),
            (1.0, LinearRgba::new(1.0, 0.0, 0.0, 0.0)),
        ];
        assert_eq!(sample_color(&stops, 0.25), LinearRgba::new(0.5, 0.5, 0.5, 0.5));
        assert_eq!(sample_color(&stops, 0.75), LinearRgba::new(1.0, 0.5, 0.5, 0.5));
        assert_eq!(sample_color(&stops, 2.0), stops[2].1);
        assert_eq!(sample_color(&[], 0.3), LinearRgba::WHITE);
        let sizes = [(0.2, 1.0), (1.0, 3.0)];
        assert_eq!(sample_scalar(&sizes, 0.0), 1.0);
        assert_eq!(sample_scalar(&sizes, 0.6), 2.0);
        assert_eq!(sample_scalar(&[(0.0, 4.0)], 0.9), 4.0);
    }

    #[test]
    fn scaled_config_is_geometrically_similar() {
        let base = ParticleConfig {
            rate: 0.0,
            shape: EmitterShape::Sphere { radius: 1.0 },
            velocity: Vec3::new(1.0, 2.0, 0.0),
            velocity_jitter: Vec3::splat(0.5),
            gravity: Vec3::new(0.0, -9.81, 0.0),
            drag: 0.7,
            size_over_life: vec![(0.0, 0.5), (1.0, 1.0)],
            ..config()
        };
        let big = base.scaled(3.0);
        assert_eq!(big.shape, EmitterShape::Sphere { radius: 3.0 });
        assert_eq!(big.size_over_life, vec![(0.0, 1.5), (1.0, 3.0)]);
        assert_eq!(big.drag, base.drag);
        assert_eq!(base.scaled(-1.0), base);
        // Same seed: every position is exactly 3× (emitter at the origin, no wind).
        let (mut a, mut b) = (ParticleState::new(11), ParticleState::new(11));
        a.burst(&base, &GlobalTransform::IDENTITY, 5);
        b.burst(&big, &GlobalTransform::IDENTITY, 5);
        for _ in 0..30 {
            a.step(&base, &GlobalTransform::IDENTITY, false, Vec3::ZERO, 1.0 / 30.0);
            b.step(&big, &GlobalTransform::IDENTITY, false, Vec3::ZERO, 1.0 / 30.0);
        }
        for (p, q) in a.particles().iter().zip(b.particles()) {
            assert!((p.position * 3.0 - q.position).length() < 1e-3, "{p:?} vs {q:?}");
        }
    }

    #[test]
    fn rng_ranges() {
        let mut rng = ParticleRng::new(42);
        for _ in 0..1000 {
            let u = rng.unit();
            assert!((0.0..1.0).contains(&u));
            let s = rng.signed();
            assert!((-1.0..1.0).contains(&s));
        }
    }
}
