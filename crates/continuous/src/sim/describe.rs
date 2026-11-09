//! Formal Markdown statements of the executed rules (shown in the rule
//! panel). The equations mirror `lenia.rs`, `smoothlife.rs` and `flow.rs`;
//! the tables are generated from the current rule values.

use std::fmt::Write;

use crate::flow::FlowLeniaRule;
use crate::lenia::{GrowthNormalization, LeniaRule};
use crate::rule::Rule;
use crate::sim::params::format_list;
use crate::smoothlife::{SmoothLifeMixing, SmoothLifeRule, SmoothLifeStepping};

/// Rule text for the descriptor.
pub fn rules_markdown(rule: &Rule) -> String {
    match rule {
        Rule::Lenia(r) => lenia(r),
        Rule::SmoothLife(r) => smoothlife(r),
        Rule::FlowLenia(r) => flow(r),
    }
}

const GRID: &str = "The world is a periodic grid (torus) of cells; `x` grows to the right and `y` downward. \
Convolutions are periodic and computed by FFT or by direct summation (identical up to ~10⁻⁶).";

fn lenia(r: &LeniaRule) -> String {
    let combine = match r.normalization {
        GrowthNormalization::Sum => "`D_j = Σ_{k: dst(k)=j} h_k · G_k(U_k)`",
        GrowthNormalization::PerDestination => "`D_j = Σ_{k: dst(k)=j} h_k · G_k(U_k) / Σ_{k: dst(k)=j} h_k`",
    };
    let mut s = format!(
        "### Lenia (Chan 2019; Expanded Lenia, Chan 2020)\n\n{GRID}\n\n\
         State: {c} channel(s) `A_j(x) ∈ [0, 1]`. Space resolution `R = {R}` cells, time resolution `T = {T}` \
         (`dt = 1/T = {dt}`).\n\n\
         Each tick:\n\n\
         1. Potentials `U_k = K_k ∗ A_src(k)` for every kernel `k`.\n\
         2. Growth {combine}.\n\
         3. Update `A_j ← clip(A_j + dt · D_j, 0, 1)`.\n\n\
         Kernel `k`: `K_k(n) ∝ K_S(‖n‖ / (R·r_k); β_k)` for `‖n‖ < R·r_k`, normalised so that `Σ_n K_k(n) = 1`, \
         with shell `K_S(ρ; β) = β_⌊Bρ⌋ · K_C(Bρ mod 1)` (`B = |β|` rings).\n\n\
         Cores `K_C(ρ)`: exponential `exp(4 − 1/(ρ(1−ρ)))`; polynomial `(4ρ(1−ρ))⁴`; rectangular `1[¼ ≤ ρ ≤ ¾]`; \
         staircase rectangular `+ ½·1[ρ < ¼]`; Gaussian bump `exp(−((ρ − ½)/0.15)²/2)`.\n\n\
         Growth `G(u)`: Gaussian `2·exp(−(u−μ)²/(2σ²)) − 1`; polynomial `2·max(0, 1 − (u−μ)²/(9σ²))⁴ − 1`; \
         rectangular `+1` if `|u − μ| ≤ σ` else `−1`.\n\n\
         | k | src → dst | r | β | core | growth | μ | σ | h |\n|---|---|---|---|---|---|---|---|---|\n",
        c = r.channels,
        R = r.radius,
        T = r.time_resolution,
        dt = r.dt(),
    );
    for (i, k) in r.kernels.iter().enumerate() {
        let _ = writeln!(
            s,
            "| {i} | {} → {} | {} | {} | {} | {} | {} | {} | {} |",
            k.source,
            k.destination,
            k.relative_radius,
            format_list(&k.beta),
            k.core.label(),
            k.growth.label(),
            k.mu,
            k.sigma,
            k.weight
        );
    }
    s
}

fn smoothlife(r: &SmoothLifeRule) -> String {
    let transition = match r.mixing {
        SmoothLifeMixing::Bounds => "`s(n, m) = σ₂(n, σₘ(b₁, d₁, m), σₘ(b₂, d₂, m))` (interval bounds, Rafler eq. 6)",
        SmoothLifeMixing::Indicators => "`s(n, m) = σₘ(σ₂(n, b₁, b₂), σ₂(n, d₁, d₂), m)` (interval indicators)",
    };
    let update = match r.stepping {
        SmoothLifeStepping::Discrete => "mode 0: `f ← s(n, m)`".to_string(),
        SmoothLifeStepping::Smooth => format!("mode 1: `f ← clip(f + dt·(2s(n, m) − 1), 0, 1)` with `dt = {}`", r.dt),
        SmoothLifeStepping::Relax => format!("mode 2: `f ← clip(f + dt·(s(n, m) − f), 0, 1)` with `dt = {}`", r.dt),
    };
    format!(
        "### SmoothLife (Rafler 2011)\n\n{GRID}\n\n\
         State `f(x) ∈ [0, 1]`. Outer radius `rₐ = {ra}`, inner radius `rᵢ = rₐ/{ratio} = {ri:.4}`, \
         anti-aliasing rim `b = {b}`.\n\n\
         Each tick:\n\n\
         1. Inner filling `m = Σ w_d(u) f(x+u) / Σ w_d` with disc weights `w_d(u) = clip((rᵢ + b/2 − |u|)/b, 0, 1)`.\n\
         2. Outer filling `n = Σ w_a(u) f(x+u) / Σ w_a` with annulus weights \
            `w_a(u) = clip((rₐ + b/2 − |u|)/b, 0, 1) − w_d(u)`; with `b = 0` the edges are hard \
            (weight 1 for `|u| ≤ r`, else 0).\n\
         3. Transition {transition}, where `σ₁(x, a, α) = 1/(1 + exp(−4(x − a)/α))`, \
            `σ₂(x, a, b) = σ₁(x, a, αₙ)(1 − σ₁(x, b, αₙ))`, `σₘ(x, y, m) = x(1 − σ₁(m, ½, αₘ)) + y·σ₁(m, ½, αₘ)`.\n\
         4. Update {update}.\n\n\
         Birth `[b₁, b₂] = [{b1}, {b2}]`, survival `[d₁, d₂] = [{d1}, {d2}]`, `αₙ = {an}`, `αₘ = {am}`.\n",
        ra = r.outer_radius,
        ratio = r.radius_ratio,
        ri = r.inner_radius(),
        b = r.rim,
        b1 = r.birth[0],
        b2 = r.birth[1],
        d1 = r.survival[0],
        d2 = r.survival[1],
        an = r.alpha_n,
        am = r.alpha_m,
    )
}

fn flow(r: &FlowLeniaRule) -> String {
    let mut s = format!(
        "### Flow-Lenia (Plantec et al. 2023, 2025)\n\n{GRID}\n\n\
         State: {c} channel(s) of non-negative mass `A_j(x) ≥ 0`; total mass per channel is conserved. \
         `R = {R}`, `dt = {dt}`, critical mass `θ_A = {theta}`, exponent `n = {n}`, temperature `s = {s}`, \
         window `{w}`.\n\n\
         Each tick:\n\n\
         1. Affinity `U_j = Σ_{{k: dst(k)=j}} h_k · G_k(K_k ∗ A_src(k))`, Gaussian growth \
            `G_k(u) = 2·exp(−(u − μ_k)²/(2σ_k²)) − 1`.\n\
         2. `A_Σ = Σ_j A_j`, `α = clip((A_Σ/θ_A)^n, 0, 1)`.\n\
         3. Flow `F_j = (1 − α)·∇U_j − α·∇A_Σ`, gradients by the unnormalised periodic 3×3 Sobel operator.\n\
         4. Displacement `δ_j = clip(dt·F_j, −({w} − s), {w} − s)` per axis.\n\
         5. Reintegration: each cell's mass moves to the square of side `2s` centred at its centre plus `δ_j`, \
            and every cell receives the fraction of that square it overlaps.\n\n\
         Kernel `k`: `K_k(n) ∝ Σ_i b_i·exp(−(D − a_i)²/(2w_i²))` with `D = ‖n‖/(R·r_k) < 1`, normalised to `Σ K_k = 1`.\n\n\
         | k | src → dst | r | rings (a, b, w) | μ | σ | h |\n|---|---|---|---|---|---|---|\n",
        c = r.channels,
        R = r.radius,
        dt = r.dt,
        theta = r.critical_mass,
        n = r.alpha_exponent,
        s = r.temperature,
        w = r.window,
    );
    for (i, k) in r.kernels.iter().enumerate() {
        let rings: Vec<String> = k.rings.iter().map(|g| format!("({}, {}, {})", g.center, g.height, g.width)).collect();
        let _ = writeln!(
            s,
            "| {i} | {} → {} | {} | {} | {} | {} | {} |",
            k.source,
            k.destination,
            k.relative_radius,
            rings.join(" "),
            k.mu,
            k.sigma,
            k.weight
        );
    }
    s
}
