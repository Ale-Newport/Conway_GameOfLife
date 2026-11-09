//! Parameter schemas of the three models and their application to rules.
//!
//! Rule parameters take effect at the next tick (`NextTick`): the change is
//! validated (including whether every kernel still fits the current and the
//! pending world size) and the derived kernels are rebuilt before the call
//! returns. World size and seed take effect at the next reset.

use simulation_core::{ApplyPolicy, ParamSet, ParamSpec, ParamValue, SimError};

use crate::convolve::ConvolutionMethod;
use crate::flow::FlowLeniaRule;
use crate::growth::GrowthFunction;
use crate::kernel::{GaussianRing, KernelCore};
use crate::lenia::{GrowthNormalization, LeniaRule};
use crate::rule::Rule;
use crate::scenario::{MAX_SIDE, MIN_SIDE, Scenario};
use crate::smoothlife::{SmoothLifeMixing, SmoothLifeRule, SmoothLifeStepping};

/// Largest length of list-valued text parameters.
const LIST_MAX_LEN: usize = 256;

/// Every parameter of a world running `rule`.
pub fn specs(rule: &Rule) -> Vec<ParamSpec> {
    let mut specs = vec![
        ParamSpec::int("width", "World width", MIN_SIDE as i64, MAX_SIDE as i64, 256)
            .describe("Number of columns of the toroidal world (cells).")
            .group("World")
            .apply(ApplyPolicy::RequiresReset),
        ParamSpec::int("height", "World height", MIN_SIDE as i64, MAX_SIDE as i64, 256)
            .describe("Number of rows of the toroidal world (cells).")
            .group("World")
            .apply(ApplyPolicy::RequiresReset),
        ParamSpec::int("seed", "Seed", 0, i64::MAX, 1)
            .describe("Seed of the random generator used by soups and randomisation when the world is reset.")
            .group("World")
            .apply(ApplyPolicy::RequiresReset),
        ParamSpec::choice(
            "convolution",
            "Convolution",
            &[("auto", "Automatic"), ("fft", "FFT"), ("direct", "Direct summation")],
            "auto",
        )
        .describe("FFT (cost independent of the kernel radius) or direct summation over kernel taps (faster for tiny kernels). Results agree to about 1e-6.")
        .group("Performance"),
    ];
    match rule {
        Rule::Lenia(r) => lenia_specs(r, &mut specs),
        Rule::SmoothLife(_) => smoothlife_specs(&mut specs),
        Rule::FlowLenia(r) => flow_specs(r, &mut specs),
    }
    specs
}

/// [`specs`] with defaults taken from `scenario`: world size, seed and every
/// rule parameter default to the scenario's own values (kernels the scenario
/// does not have default to the current rule's values), so that restoring a
/// default restores the preset. Defaults that would not validate keep the
/// generic value.
pub fn specs_with_defaults(rule: &Rule, scenario: &Scenario) -> Vec<ParamSpec> {
    let preset = values(&scenario.rule, scenario.width, scenario.height, scenario.header.seed, ConvolutionMethod::Auto);
    let current = values(rule, scenario.width, scenario.height, scenario.header.seed, ConvolutionMethod::Auto);
    let mut specs = specs(rule);
    for spec in &mut specs {
        let candidate = if scenario.rule.kind() == rule.kind() { preset.get(&spec.key) } else { None };
        if let Some(value) = candidate.or_else(|| current.get(&spec.key))
            && let Ok(valid) = spec.validate(value)
        {
            spec.default = valid;
        }
    }
    specs
}

fn kernel_group(i: usize, src: usize, dst: usize, channels: usize) -> String {
    if channels > 1 { format!("Kernel {} (channel {src} → {dst})", i + 1) } else { format!("Kernel {}", i + 1) }
}

fn channel_specs(i: usize, channels: usize, group: &str, specs: &mut Vec<ParamSpec>) {
    if channels > 1 {
        let max = channels as i64 - 1;
        specs.push(
            ParamSpec::int(&format!("k{i}.source"), "Source channel", 0, max, 0)
                .describe("Channel the kernel reads.")
                .group(group),
        );
        specs.push(
            ParamSpec::int(&format!("k{i}.destination"), "Destination channel", 0, max, 0)
                .describe("Channel that receives the kernel's growth.")
                .group(group),
        );
    }
}

fn lenia_specs(rule: &LeniaRule, specs: &mut Vec<ParamSpec>) {
    let cores: Vec<(&str, &str)> = KernelCore::ALL.iter().map(|c| (c.key(), c.label())).collect();
    let growths: Vec<(&str, &str)> = GrowthFunction::ALL.iter().map(|g| (g.key(), g.label())).collect();
    specs.push(
        ParamSpec::float("radius", "Kernel radius R", 0.5, 512.0, 13.0)
            .step(0.5)
            .describe("Space resolution: kernels span R·r cells. Creatures scale with R.")
            .group("Rule"),
    );
    specs.push(
        ParamSpec::float("time_resolution", "Time resolution T", 1.0, 10_000.0, 10.0)
            .step(1.0)
            .logarithmic()
            .describe("Each tick advances dt = 1/T; larger T approaches continuous time.")
            .group("Rule"),
    );
    specs.push(
        ParamSpec::choice(
            "normalization",
            "Growth combination",
            &[("sum", "Sum Σh·G"), ("per_destination", "Weighted mean Σh·G / Σh")],
            "sum",
        )
        .describe("How the growths of several kernels reaching one channel are combined.")
        .group("Rule"),
    );
    for (i, k) in rule.kernels.iter().enumerate() {
        let g = kernel_group(i, k.source, k.destination, rule.channels);
        let key = |s: &str| format!("k{i}.{s}");
        specs.push(
            ParamSpec::float(&key("mu"), "Growth centre μ", 0.0, 1.0, 0.15)
                .step(0.001)
                .describe("Potential at which growth is maximal.")
                .group(&g),
        );
        specs.push(
            ParamSpec::float(&key("sigma"), "Growth width σ", 0.0001, 1.0, 0.015)
                .step(0.0001)
                .logarithmic()
                .describe("Width of the growth mapping.")
                .group(&g),
        );
        specs.push(
            ParamSpec::float(&key("weight"), "Weight h", 0.0, 10.0, 1.0)
                .step(0.001)
                .describe("Weight of this kernel's growth.")
                .group(&g),
        );
        specs.push(
            ParamSpec::float(&key("relative_radius"), "Relative radius r", 0.01, 4.0, 1.0)
                .step(0.01)
                .describe("Kernel radius as a fraction of R.")
                .group(&g),
        );
        specs.push(
            ParamSpec::text(&key("beta"), "Ring peaks β", LIST_MAX_LEN, "1")
                .describe(
                    "Comma-separated peaks of the concentric rings, each in [0, 1]; fractions like 2/3 are accepted.",
                )
                .group(&g),
        );
        specs.push(
            ParamSpec::choice(&key("core"), "Kernel core", &cores, "gaussian_bump")
                .describe("Radial profile of each ring.")
                .group(&g),
        );
        specs.push(
            ParamSpec::choice(&key("growth"), "Growth function", &growths, "gaussian")
                .describe("Shape of the growth mapping G.")
                .group(&g),
        );
        channel_specs(i, rule.channels, &g, specs);
    }
}

fn smoothlife_specs(specs: &mut Vec<ParamSpec>) {
    let rule = "Rule";
    specs.extend([
        ParamSpec::float("outer_radius", "Outer radius rₐ", 1.0, 512.0, 21.0)
            .step(0.5)
            .describe("Outer radius of the neighbourhood annulus (cells).")
            .group(rule),
        ParamSpec::float("radius_ratio", "Radius ratio rₐ/rᵢ", 1.01, 16.0, 3.0)
            .step(0.01)
            .describe("Outer radius divided by the inner (cell) radius.")
            .group(rule),
        ParamSpec::float("rim", "Anti-aliasing rim b", 0.0, 4.0, 1.0)
            .step(0.1)
            .describe("Width of the linear edge of the disc and annulus (cells).")
            .group(rule),
        ParamSpec::float("b1", "Birth lower b₁", 0.0, 1.0, 0.278)
            .step(0.001)
            .describe("Lower bound of the birth interval.")
            .group(rule),
        ParamSpec::float("b2", "Birth upper b₂", 0.0, 1.0, 0.365)
            .step(0.001)
            .describe("Upper bound of the birth interval.")
            .group(rule),
        ParamSpec::float("d1", "Survival lower d₁", 0.0, 1.0, 0.267)
            .step(0.001)
            .describe("Lower bound of the survival interval.")
            .group(rule),
        ParamSpec::float("d2", "Survival upper d₂", 0.0, 1.0, 0.445)
            .step(0.001)
            .describe("Upper bound of the survival interval.")
            .group(rule),
        ParamSpec::float("alpha_n", "Sigmoid width αₙ", 0.0001, 1.0, 0.028)
            .step(0.001)
            .describe("Smoothness of the interval edges in the outer filling.")
            .group(rule),
        ParamSpec::float("alpha_m", "Sigmoid width αₘ", 0.0001, 1.0, 0.147)
            .step(0.001)
            .describe("Smoothness of the alive/dead transition in the inner filling.")
            .group(rule),
        ParamSpec::choice(
            "mixing",
            "Mixing",
            &[("bounds", "Interval bounds (paper)"), ("indicators", "Interval indicators (Ready)")],
            "bounds",
        )
        .describe("How the inner filling interpolates between birth and survival.")
        .group(rule),
        ParamSpec::choice(
            "stepping",
            "Time stepping",
            &[
                ("discrete", "Mode 0: f ← s"),
                ("smooth", "Mode 1: f ← f + dt(2s − 1)"),
                ("relax", "Mode 2: f ← f + dt(s − f)"),
            ],
            "discrete",
        )
        .describe("Rafler's time-stepping modes.")
        .group(rule),
        ParamSpec::float("dt", "Time step dt", 0.001, 1.0, 0.1)
            .step(0.001)
            .describe("Time step of modes 1 and 2 (ignored by mode 0).")
            .group(rule),
    ]);
}

fn flow_specs(rule: &FlowLeniaRule, specs: &mut Vec<ParamSpec>) {
    let g = "Rule";
    specs.extend([
        ParamSpec::float("radius", "Kernel radius R", 1.0, 512.0, 15.0)
            .step(0.5)
            .describe("Kernels span R·r cells.")
            .group(g),
        ParamSpec::float("dt", "Time step dt", 0.001, 1.0, 0.2)
            .step(0.001)
            .describe("Scales the flow displacement per tick.")
            .group(g),
        ParamSpec::float("critical_mass", "Critical mass θ_A", 0.01, 100.0, 1.0)
            .step(0.01)
            .describe("Summed mass at which diffusion fully replaces the affinity flow.")
            .group(g),
        ParamSpec::float("alpha_exponent", "Exponent n", 0.1, 10.0, 2.0)
            .step(0.1)
            .describe("Sharpness of the switch α = (A_Σ/θ_A)^n.")
            .group(g),
        ParamSpec::float("temperature", "Temperature s", 0.05, 3.0, 0.65)
            .step(0.01)
            .describe("Half side of the square each cell's mass is spread over (0.5 = no spreading for a still field).")
            .group(g),
        ParamSpec::int("window", "Window", 1, 16, 5)
            .describe("Displacements are clipped to ±(window − s) cells per axis.")
            .group(g),
    ]);
    for (i, k) in rule.kernels.iter().enumerate() {
        let gr = kernel_group(i, k.source, k.destination, rule.channels);
        let key = |s: &str| format!("k{i}.{s}");
        specs.push(
            ParamSpec::float(&key("mu"), "Growth centre μ", 0.0, 1.0, 0.15)
                .step(0.001)
                .describe("Potential at which growth is maximal.")
                .group(&gr),
        );
        specs.push(
            ParamSpec::float(&key("sigma"), "Growth width σ", 0.0001, 1.0, 0.015)
                .step(0.0001)
                .logarithmic()
                .describe("Width of the Gaussian growth.")
                .group(&gr),
        );
        specs.push(
            ParamSpec::float(&key("weight"), "Weight h", 0.0, 10.0, 1.0)
                .step(0.001)
                .describe("Weight of this kernel in the affinity.")
                .group(&gr),
        );
        specs.push(
            ParamSpec::float(&key("relative_radius"), "Relative radius r", 0.01, 4.0, 1.0)
                .step(0.01)
                .describe("Kernel radius as a fraction of R.")
                .group(&gr),
        );
        specs.push(
            ParamSpec::text(&key("rings"), "Rings (a, b, w)", LIST_MAX_LEN, "0.5,1,0.15")
                .describe("Semicolon-separated rings `a,b,w`: centre and width in kernel radii, height in [0, 1].")
                .group(&gr),
        );
        channel_specs(i, rule.channels, &gr, specs);
    }
}

/// Formats a list of numbers compactly (`0.5,1,0.666667`).
pub fn format_list(values: &[f64]) -> String {
    values.iter().map(|v| format_number(*v)).collect::<Vec<_>>().join(",")
}

fn format_number(v: f64) -> String {
    let s = format!("{v:.6}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-0" { "0".to_string() } else { s.to_string() }
}

/// Parses `1/2, 1, 0.667`-style lists.
pub fn parse_list(text: &str) -> Result<Vec<f64>, String> {
    text.split(',')
        .map(|item| {
            let item = item.trim();
            let parsed = match item.split_once('/') {
                Some((n, d)) => match (n.trim().parse::<f64>(), d.trim().parse::<f64>()) {
                    (Ok(n), Ok(d)) if d != 0.0 => Ok(n / d),
                    _ => Err(()),
                },
                None => item.parse::<f64>().map_err(|_| ()),
            };
            match parsed {
                Ok(v) if v.is_finite() => Ok(v),
                _ => Err(format!("`{item}` is not a number or fraction")),
            }
        })
        .collect()
}

fn format_rings(rings: &[GaussianRing]) -> String {
    rings.iter().map(|g| format_list(&[g.center, g.height, g.width])).collect::<Vec<_>>().join("; ")
}

fn parse_rings(text: &str) -> Result<Vec<GaussianRing>, String> {
    text.split(';')
        .map(|group| match parse_list(group)?.as_slice() {
            &[center, height, width] => Ok(GaussianRing { center, height, width }),
            other => Err(format!("ring `{}` needs exactly three numbers a,b,w (got {})", group.trim(), other.len())),
        })
        .collect()
}

/// Current values of every parameter.
pub fn values(rule: &Rule, width: usize, height: usize, seed: u64, method: ConvolutionMethod) -> ParamSet {
    let mut set = ParamSet::new()
        .with("width", ParamValue::Int(width as i64))
        .with("height", ParamValue::Int(height as i64))
        .with("seed", ParamValue::Int(i64::try_from(seed).unwrap_or(i64::MAX)))
        .with("convolution", ParamValue::Text(method.key().into()));
    let f = ParamValue::Float;
    let t = |s: &str| ParamValue::Text(s.to_string());
    match rule {
        Rule::Lenia(r) => {
            set.set("radius", f(r.radius));
            set.set("time_resolution", f(r.time_resolution));
            set.set("normalization", t(r.normalization.key()));
            for (i, k) in r.kernels.iter().enumerate() {
                set.set(format!("k{i}.mu"), f(k.mu));
                set.set(format!("k{i}.sigma"), f(k.sigma));
                set.set(format!("k{i}.weight"), f(k.weight));
                set.set(format!("k{i}.relative_radius"), f(k.relative_radius));
                set.set(format!("k{i}.beta"), t(&format_list(&k.beta)));
                set.set(format!("k{i}.core"), t(k.core.key()));
                set.set(format!("k{i}.growth"), t(k.growth.key()));
                if r.channels > 1 {
                    set.set(format!("k{i}.source"), ParamValue::Int(k.source as i64));
                    set.set(format!("k{i}.destination"), ParamValue::Int(k.destination as i64));
                }
            }
        }
        Rule::SmoothLife(r) => {
            for (key, v) in [
                ("outer_radius", r.outer_radius),
                ("radius_ratio", r.radius_ratio),
                ("rim", r.rim),
                ("b1", r.birth[0]),
                ("b2", r.birth[1]),
                ("d1", r.survival[0]),
                ("d2", r.survival[1]),
                ("alpha_n", r.alpha_n),
                ("alpha_m", r.alpha_m),
                ("dt", r.dt),
            ] {
                set.set(key, f(v));
            }
            set.set("mixing", t(r.mixing.key()));
            set.set("stepping", t(r.stepping.key()));
        }
        Rule::FlowLenia(r) => {
            set.set("radius", f(r.radius));
            set.set("dt", f(r.dt));
            set.set("critical_mass", f(r.critical_mass));
            set.set("alpha_exponent", f(r.alpha_exponent));
            set.set("temperature", f(r.temperature));
            set.set("window", ParamValue::Int(i64::from(r.window)));
            for (i, k) in r.kernels.iter().enumerate() {
                set.set(format!("k{i}.mu"), f(k.mu));
                set.set(format!("k{i}.sigma"), f(k.sigma));
                set.set(format!("k{i}.weight"), f(k.weight));
                set.set(format!("k{i}.relative_radius"), f(k.relative_radius));
                set.set(format!("k{i}.rings"), t(&format_rings(&k.rings)));
                if r.channels > 1 {
                    set.set(format!("k{i}.source"), ParamValue::Int(k.source as i64));
                    set.set(format!("k{i}.destination"), ParamValue::Int(k.destination as i64));
                }
            }
        }
    }
    set
}

/// A validated parameter change.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    /// New world width (next reset).
    Width(usize),
    /// New world height (next reset).
    Height(usize),
    /// New seed (next reset).
    Seed(u64),
    /// New convolution method.
    Method(ConvolutionMethod),
    /// New rule (not yet checked against the world size).
    Rule(Rule),
}

/// Validates `value` for `key` against the specs of `rule` and returns the
/// resulting change.
pub fn change(rule: &Rule, key: &str, value: &ParamValue) -> Result<Change, SimError> {
    let spec = specs(rule).into_iter().find(|s| s.key == key).ok_or_else(|| SimError::UnknownParameter(key.into()))?;
    let value = spec.validate(value)?;
    let invalid = |reason: String| SimError::invalid_param(key, reason);
    let num = value.as_f64().unwrap_or_default();
    let int = value.as_i64().unwrap_or_default();
    let text = value.as_str().unwrap_or_default().to_string();
    match key {
        "width" => return Ok(Change::Width(int as usize)),
        "height" => return Ok(Change::Height(int as usize)),
        "seed" => return Ok(Change::Seed(int as u64)),
        "convolution" => {
            return ConvolutionMethod::from_key(&text)
                .map(Change::Method)
                .ok_or_else(|| invalid("unknown method".into()));
        }
        _ => {}
    }
    let mut rule = rule.clone();
    match &mut rule {
        Rule::Lenia(r) => apply_lenia(r, key, num, int, &text).map_err(invalid)?,
        Rule::SmoothLife(r) => apply_smoothlife(r, key, num, &text).map_err(invalid)?,
        Rule::FlowLenia(r) => apply_flow(r, key, num, int, &text).map_err(invalid)?,
    }
    rule.validate().map_err(|e| invalid(e.to_string()))?;
    Ok(Change::Rule(rule))
}

/// Splits `k{i}.{field}` keys.
fn kernel_key(key: &str) -> Option<(usize, &str)> {
    let rest = key.strip_prefix('k')?;
    let (index, field) = rest.split_once('.')?;
    Some((index.parse().ok()?, field))
}

fn apply_lenia(r: &mut LeniaRule, key: &str, num: f64, int: i64, text: &str) -> Result<(), String> {
    match key {
        "radius" => r.radius = num,
        "time_resolution" => r.time_resolution = num,
        "normalization" => r.normalization = GrowthNormalization::from_key(text).ok_or("unknown normalisation")?,
        _ => {
            let (i, field) = kernel_key(key).ok_or("unknown parameter")?;
            let k = r.kernels.get_mut(i).ok_or("no such kernel")?;
            match field {
                "mu" => k.mu = num,
                "sigma" => k.sigma = num,
                "weight" => k.weight = num,
                "relative_radius" => k.relative_radius = num,
                "beta" => k.beta = parse_list(text)?,
                "core" => k.core = KernelCore::from_key(text).ok_or("unknown kernel core")?,
                "growth" => k.growth = GrowthFunction::from_key(text).ok_or("unknown growth function")?,
                "source" => k.source = int as usize,
                "destination" => k.destination = int as usize,
                _ => return Err("unknown kernel parameter".into()),
            }
        }
    }
    Ok(())
}

fn apply_smoothlife(r: &mut SmoothLifeRule, key: &str, num: f64, text: &str) -> Result<(), String> {
    match key {
        "outer_radius" => r.outer_radius = num,
        "radius_ratio" => r.radius_ratio = num,
        "rim" => r.rim = num,
        "b1" => r.birth[0] = num,
        "b2" => r.birth[1] = num,
        "d1" => r.survival[0] = num,
        "d2" => r.survival[1] = num,
        "alpha_n" => r.alpha_n = num,
        "alpha_m" => r.alpha_m = num,
        "dt" => r.dt = num,
        "mixing" => r.mixing = SmoothLifeMixing::from_key(text).ok_or("unknown mixing")?,
        "stepping" => r.stepping = SmoothLifeStepping::from_key(text).ok_or("unknown stepping mode")?,
        _ => return Err("unknown parameter".into()),
    }
    Ok(())
}

fn apply_flow(r: &mut FlowLeniaRule, key: &str, num: f64, int: i64, text: &str) -> Result<(), String> {
    match key {
        "radius" => r.radius = num,
        "dt" => r.dt = num,
        "critical_mass" => r.critical_mass = num,
        "alpha_exponent" => r.alpha_exponent = num,
        "temperature" => r.temperature = num,
        "window" => r.window = u32::try_from(int).map_err(|_| "window out of range")?,
        _ => {
            let (i, field) = kernel_key(key).ok_or("unknown parameter")?;
            let k = r.kernels.get_mut(i).ok_or("no such kernel")?;
            match field {
                "mu" => k.mu = num,
                "sigma" => k.sigma = num,
                "weight" => k.weight = num,
                "relative_radius" => k.relative_radius = num,
                "rings" => k.rings = parse_rings(text)?,
                "source" => k.source = int as usize,
                "destination" => k.destination = int as usize,
                _ => return Err("unknown kernel parameter".into()),
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_round_trip() {
        assert_eq!(parse_list("1/2, 1 ,2/3").map(|v| format_list(&v)), Ok("0.5,1,0.666667".to_string()));
        assert!(parse_list("1, x").is_err());
        assert!(parse_list("1/0").is_err());
        let rings = parse_rings("0.5,1,0.15; 0.2,0.5,0.1").expect("rings");
        assert_eq!(rings.len(), 2);
        assert_eq!(format_rings(&rings), "0.5,1,0.15; 0.2,0.5,0.1");
        assert!(parse_rings("0.5,1").is_err());
    }

    #[test]
    fn every_spec_default_is_valid_and_every_value_has_a_spec() {
        for rule in [Rule::Lenia(LeniaRule::orbium()), Rule::SmoothLife(SmoothLifeRule::paper_glider())] {
            let specs = specs(&rule);
            for s in &specs {
                assert!(s.validate(&s.default).is_ok(), "{}", s.key);
            }
            let values = values(&rule, 128, 128, 7, ConvolutionMethod::Auto);
            for (k, v) in values.iter() {
                let spec = specs.iter().find(|s| s.key == k).unwrap_or_else(|| panic!("no spec for {k}"));
                assert!(spec.validate(v).is_ok(), "{k} = {v}");
                // Re-applying the current value is a valid change.
                assert!(change(&rule, k, v).is_ok(), "{k}");
            }
            assert_eq!(values.len(), specs.len());
        }
    }

    #[test]
    fn changes_are_validated() {
        let rule = Rule::Lenia(LeniaRule::orbium());
        assert!(matches!(change(&rule, "k0.mu", &ParamValue::Float(0.2)), Ok(Change::Rule(_))));
        assert!(matches!(
            change(&rule, "k0.beta", &ParamValue::Text("0,0".into())),
            Err(SimError::InvalidParameter { .. })
        ));
        assert!(matches!(change(&rule, "k1.mu", &ParamValue::Float(0.2)), Err(SimError::UnknownParameter(_))));
        assert!(matches!(
            change(&rule, "k0.core", &ParamValue::Text("cubic".into())),
            Err(SimError::InvalidParameter { .. })
        ));
        assert_eq!(change(&rule, "width", &ParamValue::Int(64)), Ok(Change::Width(64)));
        assert!(change(&rule, "width", &ParamValue::Int(8)).is_err());
    }
}
