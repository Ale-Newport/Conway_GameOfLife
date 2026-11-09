//! Symmetry and direction properties of the three engines (research note §8,
//! property tests):
//!
//! * the rules commute with torus translations, quarter turns and mirror
//!   images (all kernels are radially symmetric, Sobel stencils and
//!   reintegration squares respect the square lattice), which catches
//!   off-centre kernels, transposed FFT layouts and inconsistent gradient
//!   axes;
//! * Flow-Lenia matter climbs the affinity gradient and descends the
//!   concentration gradient (the signs of `F = (1 − α)∇U − α∇A_Σ`).

use continuous::flow::FlowEngine;
use continuous::lenia::LeniaEngine;
use continuous::scenario::builtin_by_id;
use continuous::smoothlife::SmoothLifeEngine;
use continuous::{ContinuousSim, ConvolutionMethod, FlowKernel, FlowLeniaRule, GaussianRing, Rule};
use simulation_core::{Grid2, Simulation};

type Fields = Vec<Grid2<f32>>;

/// A lattice symmetry applied to one plane.
type Transform = fn(&Grid2<f32>) -> Grid2<f32>;

/// State of a built-in scenario after `ticks` ticks, with its rule.
fn state(id: &str, ticks: u64) -> (Rule, Fields) {
    let scenario = builtin_by_id(id).unwrap_or_else(|| panic!("missing scenario {id}"));
    let mut sim = ContinuousSim::from_scenario(scenario).unwrap_or_else(|e| panic!("{id}: {e}"));
    sim.advance(ticks);
    (sim.rule().clone(), sim.fields().to_vec())
}

/// Quarter turn on a square torus: new(x, y) = old(y, N − 1 − x).
fn rotate(g: &Grid2<f32>) -> Grid2<f32> {
    let n = g.width();
    Grid2::from_fn(n, n, |x, y| *g.get(y, n - 1 - x))
}

/// Mirror image across the vertical axis.
fn mirror(g: &Grid2<f32>) -> Grid2<f32> {
    let w = g.width();
    Grid2::from_fn(w, g.height(), |x, y| *g.get(w - 1 - x, y))
}

/// Torus translation by (37, 11).
fn shift(g: &Grid2<f32>) -> Grid2<f32> {
    let (w, h) = (g.width(), g.height());
    Grid2::from_fn(w, h, |x, y| *g.get((x + w - 37) % w, (y + h - 11) % h))
}

/// Runs `steps` ticks from `fields` and from `transform(fields)` and returns
/// the largest difference between `transform(evolved)` and the evolved
/// transformed state.
fn equivariance_error(
    fields: &Fields,
    transform: Transform,
    steps: usize,
    mut step_a: impl FnMut(&mut Fields),
    mut step_b: impl FnMut(&mut Fields),
) -> f32 {
    let mut a = fields.clone();
    let mut b: Fields = fields.iter().map(transform).collect();
    for _ in 0..steps {
        step_a(&mut a);
        step_b(&mut b);
    }
    a.iter()
        .map(transform)
        .zip(&b)
        .flat_map(|(x, y)| x.as_slice().iter().zip(y.as_slice()).map(|(p, q)| (p - q).abs()).collect::<Vec<_>>())
        .fold(0.0f32, f32::max)
}

const TRANSFORMS: [(&str, Transform); 3] = [("quarter turn", rotate), ("mirror", mirror), ("translation", shift)];

#[test]
fn expanded_lenia_commutes_with_lattice_symmetries() {
    // Three channels, 15 kernels with rings and cross-channel routing.
    let (rule, fields) = state("lenia.tessellatium_gyrans", 40);
    let Rule::Lenia(rule) = rule else { panic!("Lenia rule expected") };
    let (w, h) = (fields[0].width(), fields[0].height());
    for (name, transform) in TRANSFORMS {
        let mut ea = LeniaEngine::new(&rule, w, h, ConvolutionMethod::Fft).expect("engine");
        let mut eb = LeniaEngine::new(&rule, w, h, ConvolutionMethod::Fft).expect("engine");
        let err = equivariance_error(
            &fields,
            transform,
            20,
            |f| {
                ea.step(&rule, f);
            },
            |f| {
                eb.step(&rule, f);
            },
        );
        assert!(err < 1e-4, "{name}: max error {err}");
    }
}

#[test]
fn smoothlife_commutes_with_lattice_symmetries() {
    let (rule, fields) = state("smoothlife.glider", 5);
    let Rule::SmoothLife(rule) = rule else { panic!("SmoothLife rule expected") };
    let (w, h) = (fields[0].width(), fields[0].height());
    for (name, transform) in TRANSFORMS {
        let mut ea = SmoothLifeEngine::new(&rule, w, h, ConvolutionMethod::Fft).expect("engine");
        let mut eb = SmoothLifeEngine::new(&rule, w, h, ConvolutionMethod::Fft).expect("engine");
        let err = equivariance_error(
            &fields,
            transform,
            10,
            |f| {
                ea.step(&rule, &mut f[0]);
            },
            |f| {
                eb.step(&rule, &mut f[0]);
            },
        );
        assert!(err < 1e-3, "{name}: max error {err}");
    }
}

#[test]
fn flow_lenia_commutes_with_lattice_symmetries() {
    let (rule, fields) = state("flow_lenia.demo", 60);
    let Rule::FlowLenia(rule) = rule else { panic!("Flow-Lenia rule expected") };
    let (w, h) = (fields[0].width(), fields[0].height());
    for (name, transform) in TRANSFORMS {
        let mut ea = FlowEngine::new(&rule, w, h, ConvolutionMethod::Fft).expect("engine");
        let mut eb = FlowEngine::new(&rule, w, h, ConvolutionMethod::Fft).expect("engine");
        let err = equivariance_error(
            &fields,
            transform,
            20,
            |f| {
                ea.step(&rule, f);
            },
            |f| {
                eb.step(&rule, f);
            },
        );
        assert!(err < 1e-3, "{name}: max error {err}");
    }
}

/// A Flow-Lenia rule with one bump-shaped kernel (`a = 0`, wide ring) from
/// `source` to `destination` and Gaussian growth increasing on `[0, 1]`
/// (`μ = 1`, `σ = 1`), so the affinity grows with the potential.
fn bump_rule(channels: usize, source: usize, destination: usize, weight: f64, critical_mass: f64) -> FlowLeniaRule {
    FlowLeniaRule {
        radius: 10.0,
        dt: 1.0,
        channels,
        kernels: vec![FlowKernel {
            relative_radius: 1.0,
            rings: vec![GaussianRing { center: 0.0, height: 1.0, width: 0.5 }],
            mu: 1.0,
            sigma: 1.0,
            weight,
            source,
            destination,
        }],
        critical_mass,
        alpha_exponent: 2.0,
        // 0.5: a still cell keeps its mass exactly, so only the flow moves it.
        temperature: 0.5,
        window: 5,
    }
}

fn disc(w: usize, h: usize, cx: f32, cy: f32, r: f32, value: f32) -> Grid2<f32> {
    Grid2::from_fn(w, h, |x, y| {
        let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
        if dx * dx + dy * dy <= r * r { value } else { 0.0 }
    })
}

/// Mass-weighted mean x and mean squared distance from `(cx, cy)`.
fn moments(g: &Grid2<f32>, cx: f64, cy: f64) -> (f64, f64) {
    let (mut m, mut mx, mut m2) = (0.0, 0.0, 0.0);
    for (x, y, &v) in g.iter() {
        let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
        let v = f64::from(v);
        m += v;
        mx += v * px;
        m2 += v * ((px - cx).powi(2) + (py - cy).powi(2));
    }
    (mx / m, m2 / m)
}

#[test]
fn flow_lenia_matter_climbs_the_affinity_gradient() {
    // Channel 0: a dense disc at x = 20 that nothing moves (no kernel writes
    // to channel 0 and α ≈ 0 because θ_A = 100). Channel 1: a faint probe at
    // x = 28.5 whose affinity U_1 = G(K ∗ A_0) increases towards the disc.
    let (w, h) = (64, 64);
    let rule = bump_rule(2, 0, 1, 1.0, 100.0);
    let mut engine = FlowEngine::new(&rule, w, h, ConvolutionMethod::Fft).expect("engine");
    let mut fields = vec![disc(w, h, 20.0, 32.0, 4.0, 1.0), disc(w, h, 28.5, 32.0, 0.6, 0.01)];
    let blob_before = fields[0].clone();
    let (x0, _) = moments(&fields[1], 28.5, 32.0);
    engine.step(&rule, &mut fields);
    let (x1, _) = moments(&fields[1], 28.5, 32.0);
    assert!(x1 < x0 - 0.05, "probe moved from x = {x0} to {x1}; it must move towards the disc");
    let blob_drift: f32 =
        fields[0].as_slice().iter().zip(blob_before.as_slice()).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
    // α ≈ (1/100)² lets the disc diffuse by at most ~4·10⁻⁴ per tick.
    assert!(blob_drift < 2e-3, "the source disc should stay put (max change {blob_drift})");
}

#[test]
fn flow_lenia_matter_descends_the_concentration_gradient() {
    // θ_A = 0.01 makes α = 1 wherever there is matter, so the flow is pure
    // diffusion −∇A_Σ (weight h = 0 removes the affinity): the disc spreads.
    let (w, h) = (64, 64);
    let rule = bump_rule(1, 0, 0, 0.0, 0.01);
    let mut engine = FlowEngine::new(&rule, w, h, ConvolutionMethod::Fft).expect("engine");
    let mut fields = vec![disc(w, h, 32.0, 32.0, 6.0, 1.0)];
    let (_, spread0) = moments(&fields[0], 32.0, 32.0);
    let mass0 = fields[0].sum();
    for _ in 0..3 {
        engine.step(&rule, &mut fields);
    }
    let (_, spread1) = moments(&fields[0], 32.0, 32.0);
    assert!(spread1 > spread0 + 1.0, "second moment {spread0} → {spread1}: diffusion must spread the disc");
    assert!(((fields[0].sum() - mass0) / mass0).abs() < 1e-6);
}
