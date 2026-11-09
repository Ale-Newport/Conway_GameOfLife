//! Procedural trees with growth stages.
//!
//! Branching is recursive with seeded variation ([`SimRng`]); branch
//! azimuths follow the golden angle (137.5°, phyllotaxis); child radii obey
//! Leonardo's rule (`r² = Σ rᵢ²`, the pipe model), and trunk radius scales
//! with height as `r ∝ h^{3/2}` (McMahon's elastic similarity, 1973), so
//! young trees are slender. Foliage is built from noise-displaced lumps
//! merged into clusters (never plain spheres); its vertex colours carry only
//! light/dark variation so a shared material can apply the seasonal hue.

use std::f32::consts::{PI, TAU};

use bevy::math::{Quat, Vec3};
use simulation_core::SimRng;

use crate::procedural::mesh::{Cap, MeshData, Ring, loft, lump, tube};
use crate::procedural::palette::{shade, srgb};
use crate::procedural::parts::Detail;

/// Golden angle (rad) between successive branches.
pub const GOLDEN_ANGLE: f32 = 2.399_963;

/// Tree architecture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TreeStyle {
    /// Deciduous broadleaf (oak, beech): decurrent, round crown.
    Broadleaf,
    /// Conifer (spruce, fir): excurrent trunk, whorled tiers, conical.
    Conifer,
    /// Palm: unbranched curved trunk with a crown of fronds.
    Palm,
    /// Savanna acacia: flat umbrella crown on spreading limbs.
    Acacia,
    /// Baobab: massive bottle trunk with stubby branches.
    Baobab,
    /// Dead tree: bare, partly broken branches.
    Dead,
}

impl TreeStyle {
    /// Every style.
    pub const ALL: [TreeStyle; 6] =
        [Self::Broadleaf, Self::Conifer, Self::Palm, Self::Acacia, Self::Baobab, Self::Dead];

    /// Adult height (m).
    pub fn adult_height(self) -> f32 {
        match self {
            Self::Broadleaf => 14.0,
            Self::Conifer => 18.0,
            Self::Palm => 11.0,
            Self::Acacia => 8.0,
            Self::Baobab => 14.0,
            Self::Dead => 11.0,
        }
    }

    /// Whether the tree sheds its leaves in the unfavourable season.
    pub fn deciduous(self) -> bool {
        matches!(self, Self::Broadleaf | Self::Acacia | Self::Baobab)
    }

    /// Whether the style has foliage at all.
    pub fn has_foliage(self) -> bool {
        self != Self::Dead
    }
}

/// Generated tree geometry (tree base at the origin, +Y up).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TreeMeshes {
    /// Trunk and branches.
    pub wood: MeshData,
    /// Leaf clusters (luminance-only vertex colours).
    pub foliage: MeshData,
    /// Height of the tree (m).
    pub height: f32,
}

struct Builder {
    rng: SimRng,
    wood: MeshData,
    foliage: MeshData,
    around: usize,
    lump_level: u32,
    bark: [f32; 4],
    style: TreeStyle,
    /// Fraction of deciduous leaf clusters kept (seasonal leaf fall).
    cover: f32,
}

impl Builder {
    fn limb(&mut self, from: Vec3, dir: Vec3, len: f32, r0: f32, r1: f32) -> Vec3 {
        // Slightly curved limb: the midpoint is pushed sideways.
        let side = dir.any_orthonormal_vector();
        let kink =
            Quat::from_axis_angle(dir, self.rng.range_f32(0.0, TAU)) * side * len * self.rng.range_f32(0.0, 0.06);
        let mid = from + dir * (0.5 * len) + kink;
        let end = from + dir * len + kink * 0.5;
        let bark = self.bark;
        let seed = self.rng.next_u32();
        tube(
            &mut self.wood,
            &[from, mid, end],
            &[r0, 0.5 * (r0 + r1), r1],
            self.around,
            Vec3::Y,
            (Cap::Flat, Cap::Round),
            &move |sp| {
                let n = crate::procedural::noise::value3(sp.position * 7.0, seed);
                shade(bark, 0.85 + 0.2 * n + 0.08 * sp.dorsal)
            },
        );
        end
    }

    fn cluster(&mut self, at: Vec3, radius: f32, flatten: f32, lumps: usize) {
        let level = self.lump_level;
        for k in 0..lumps {
            let offset =
                if k == 0 { Vec3::ZERO } else { self.rng.unit_vec3() * radius * 0.55 * Vec3::new(1.0, flatten, 1.0) };
            let r = radius * self.rng.range_f32(0.55, 0.85);
            let seed = self.rng.next_u32();
            // Leaf fall drops whole clusters in a fixed per-cluster order
            // (the cluster's own random seed), so a thinning crown keeps the
            // remaining clusters exactly where they were. The random stream
            // is consumed identically, so the wood never changes.
            if (seed >> 8) as f32 / (1u32 << 24) as f32 >= self.cover {
                continue;
            }
            lump(&mut self.foliage, at + offset, Vec3::new(r, r * flatten, r), level, 0.32, 2.2, seed, &|dir, k| {
                let lit = 0.72 + 0.22 * dir.y + 0.18 * (k - 1.0);
                [lit, lit, lit, 1.0]
            });
        }
    }

    /// Recursive decurrent branching (broadleaf, acacia, dead).
    #[allow(clippy::too_many_arguments)]
    fn branch(
        &mut self,
        from: Vec3,
        dir: Vec3,
        len: f32,
        r: f32,
        depth: u32,
        spread: f32,
        leaf_radius: f32,
        phase: f32,
    ) {
        let tip_r = if depth == 0 { r * 0.35 } else { r * 0.72 };
        let end = self.limb(from, dir, len, r, tip_r);
        if depth == 0 {
            if self.style.has_foliage() {
                let flatten = if self.style == TreeStyle::Acacia { 0.35 } else { 0.75 };
                self.cluster(end, leaf_radius, flatten, 3);
            }
            return;
        }
        if depth == 1 && self.style == TreeStyle::Broadleaf {
            // Interior clusters fill the crown of decurrent trees.
            self.cluster(end, 0.7 * leaf_radius, 0.8, 2);
        }
        let n = if self.style == TreeStyle::Acacia { 3 } else { 2 + self.rng.index(2) };
        let child_r = tip_r / (n as f32).sqrt();
        for i in 0..n {
            // Dead trees lose some limbs.
            if self.style == TreeStyle::Dead && self.rng.chance(0.25) {
                continue;
            }
            let azimuth = phase + i as f32 * GOLDEN_ANGLE + self.rng.range_f32(-0.3, 0.3);
            let tilt = spread
                * self.rng.range_f32(0.75, 1.2)
                * if i == 0 && self.style == TreeStyle::Broadleaf { 0.45 } else { 1.0 };
            let side = dir.any_orthonormal_vector();
            let axis = Quat::from_axis_angle(dir, azimuth) * side;
            let mut d = (Quat::from_axis_angle(axis, tilt) * dir).normalize();
            if self.style == TreeStyle::Acacia {
                // Acacia limbs spread out flat to an umbrella crown.
                d.y = d.y.clamp(0.05, 0.4);
                d = d.normalize();
            } else {
                // Phototropism: branches turn slightly upwards.
                d = (d + Vec3::Y * 0.25).normalize();
            }
            let ratio = self.rng.range_f32(0.68, 0.82);
            self.branch(end, d, len * ratio, child_r, depth - 1, spread, leaf_radius, azimuth + PI * 0.3);
        }
    }
}

/// Generates a tree of `style` at growth stage `growth` (0 = sapling,
/// 1 = mature) from `seed`, in full leaf.
pub fn generate_tree(style: TreeStyle, growth: f32, seed: u64, detail: Detail) -> TreeMeshes {
    generate_tree_with_leaf_cover(style, growth, seed, detail, 1.0)
}

/// Same tree as [`generate_tree`] with only a fraction `cover` (0..1) of
/// its leaf clusters, for seasonal leaf fall of deciduous styles. The wood
/// and the position of every remaining cluster are identical to the
/// full-leaf tree. Evergreen styles (conifer, palm) ignore `cover`.
pub fn generate_tree_with_leaf_cover(
    style: TreeStyle,
    growth: f32,
    seed: u64,
    detail: Detail,
    cover: f32,
) -> TreeMeshes {
    let g = growth.clamp(0.0, 1.0);
    let height = style.adult_height() * (0.08 + 0.92 * g.powf(0.8));
    let h_ratio = height / style.adult_height();
    let mut b = Builder {
        rng: SimRng::derive(seed, style as u64),
        wood: MeshData::new(),
        foliage: MeshData::new(),
        around: detail.around(10),
        lump_level: if detail == Detail::Full { 2 } else { 1 },
        bark: match style {
            TreeStyle::Dead => srgb(0x8a8278),
            TreeStyle::Baobab => srgb(0x9a8a78),
            TreeStyle::Palm => srgb(0x8a7458),
            TreeStyle::Conifer => srgb(0x5a4030),
            _ => srgb(0x5e4834),
        },
        style,
        cover: if style.deciduous() { cover.clamp(0.0, 1.0) } else { 1.0 },
    };
    // Elastic similarity: base radius ∝ h^1.5.
    let base_r = match style {
        TreeStyle::Baobab => 1.6,
        TreeStyle::Palm => 0.22,
        TreeStyle::Conifer => 0.35,
        _ => 0.38,
    } * h_ratio.powf(1.5).max(0.02);
    let depth = match style {
        TreeStyle::Conifer | TreeStyle::Palm => 0,
        TreeStyle::Baobab => 1 + (g * 1.5) as u32,
        _ => 1 + (g * 3.2) as u32,
    }
    .min(if detail == Detail::Full { 4 } else { 2 });
    match style {
        TreeStyle::Broadleaf | TreeStyle::Dead => {
            let trunk_len = 0.3 * height;
            let top = b.limb(Vec3::ZERO, Vec3::Y, trunk_len, base_r, base_r * 0.75);
            let branch_len = (height - trunk_len) * 0.42;
            let leaf_r = if style == TreeStyle::Dead { 0.0 } else { 0.2 * height * (0.6 + 0.4 * g) };
            let phase = b.rng.range_f32(0.0, TAU);
            b.branch(top, Vec3::Y, branch_len, base_r * 0.75, depth, 0.72, leaf_r, phase);
        }
        TreeStyle::Acacia => {
            let trunk_len = 0.35 * height;
            let lean = (Vec3::Y + b.rng.unit_vec3() * 0.12).normalize();
            let top = b.limb(Vec3::ZERO, lean, trunk_len, base_r, base_r * 0.8);
            let phase = b.rng.range_f32(0.0, TAU);
            b.branch(top, Vec3::Y, 0.4 * height, base_r * 0.8, depth, 0.85, 0.2 * height * (0.6 + 0.4 * g), phase);
        }
        TreeStyle::Baobab => {
            let crown_y = 0.62 * height;
            let n = detail.rings(10);
            let rings: Vec<Ring> = (0..=n)
                .map(|i| {
                    let u = i as f32 / n as f32;
                    let r = base_r * (1.0 + 0.35 * (PI * u * 0.9).sin() - 0.45 * u);
                    Ring::circle(Vec3::new(0.0, u * crown_y, 0.0), r.max(0.3 * base_r)).with_exponent(2.4)
                })
                .collect();
            let bark = b.bark;
            loft(&mut b.wood, &rings, detail.around(14), Vec3::Z, Cap::Flat, Cap::Round, &move |sp| {
                shade(bark, 0.85 + 0.15 * (sp.around * TAU * 9.0).sin())
            });
            let limbs = 5 + b.rng.index(3);
            for k in 0..limbs {
                let a = k as f32 * GOLDEN_ANGLE;
                let d = Vec3::new(a.cos(), 0.9, a.sin()).normalize();
                let start = Vec3::new(0.0, crown_y * 0.97, 0.0) + Vec3::new(a.cos(), 0.0, a.sin()) * base_r * 0.35;
                let phase = a;
                b.branch(start, d, 0.22 * height, base_r * 0.28, depth, 0.7, 0.07 * height, phase);
            }
        }
        TreeStyle::Conifer => {
            let trunk_top = b.limb(Vec3::ZERO, Vec3::Y, height, base_r, base_r * 0.08);
            let tiers = (3.0 + 7.0 * g).round() as usize;
            let crown_base = 0.18 * height;
            let level = b.lump_level;
            for k in 0..tiers {
                let u = k as f32 / tiers as f32;
                let y = crown_base + u * (height - crown_base) * 0.93;
                let radius = (0.28 * height) * (1.0 - u).powf(0.95) * (0.7 + 0.3 * g) + 0.03 * height;
                let thick = (height - crown_base) / tiers as f32 * 1.25;
                let seed = b.rng.next_u32();
                lump(
                    &mut b.foliage,
                    Vec3::new(0.0, y + 0.3 * thick, 0.0),
                    Vec3::new(radius, 0.55 * thick, radius),
                    level,
                    0.28,
                    3.0,
                    seed,
                    &|dir, k| {
                        let lit = 0.62 + 0.25 * dir.y.max(-0.3) + 0.2 * (k - 1.0);
                        [lit, lit, lit, 1.0]
                    },
                );
            }
            let seed = b.rng.next_u32();
            lump(&mut b.foliage, trunk_top, Vec3::new(0.03, 0.08, 0.03) * height, 1, 0.1, 2.0, seed, &|_, _| {
                [0.8, 0.8, 0.8, 1.0]
            });
        }
        TreeStyle::Palm => {
            // Curved trunk with leaf-scar rings.
            let lean = b.rng.range_f32(0.15, 0.35);
            let az = b.rng.range_f32(0.0, TAU);
            let dir = Vec3::new(az.cos(), 0.0, az.sin());
            let n = detail.rings(10);
            let pts: Vec<Vec3> = (0..=n)
                .map(|i| {
                    let u = i as f32 / n as f32;
                    dir * (lean * height * u * u) + Vec3::Y * (height * u)
                })
                .collect();
            let radii: Vec<f32> = (0..=n).map(|i| base_r * (1.0 - 0.35 * i as f32 / n as f32)).collect();
            let bark = b.bark;
            tube(&mut b.wood, &pts, &radii, b.around, Vec3::Z, (Cap::Flat, Cap::Round), &move |sp| {
                shade(bark, if (sp.along * 40.0).rem_euclid(1.0) < 0.25 { 0.7 } else { 1.0 })
            });
            let crown = pts[n];
            let fronds = if detail == Detail::Full { 11 } else { 7 };
            let frond_len = 0.42 * height * (0.6 + 0.4 * g);
            for k in 0..fronds {
                let a = k as f32 * GOLDEN_ANGLE;
                let out = Vec3::new(a.cos(), 0.0, a.sin());
                let rise = b.rng.range_f32(0.2, 0.7);
                let m = detail.rings(8);
                let rings: Vec<Ring> = (0..=m)
                    .map(|i| {
                        let u = i as f32 / m as f32;
                        let droop = rise * u - 1.1 * u * u;
                        let c = crown + out * (frond_len * u) + Vec3::Y * (frond_len * 0.5 * droop);
                        let w = frond_len * 0.13 * (PI * u.powf(0.7)).sin().max(0.04);
                        Ring {
                            center: c,
                            half_width: w,
                            top: 0.01 * frond_len,
                            bottom: 0.01 * frond_len,
                            exponent: 2.0,
                        }
                    })
                    .collect();
                loft(&mut b.foliage, &rings, 6, Vec3::Y, Cap::Flat, Cap::Point(0.04 * frond_len), &|sp| {
                    let lit = 0.75 + 0.2 * sp.dorsal.abs() + 0.1 * (sp.lateral * 9.0).sin();
                    [lit, lit, lit, 1.0]
                });
            }
            if g > 0.6 {
                let nut = srgb(0x6a4a24);
                for k in 0..5 {
                    let a = k as f32 * GOLDEN_ANGLE;
                    let seed = b.rng.next_u32();
                    lump(
                        &mut b.wood,
                        crown + Vec3::new(a.cos() * 0.2, -0.25, a.sin() * 0.2),
                        Vec3::splat(0.13),
                        1,
                        0.08,
                        2.0,
                        seed,
                        &move |_, k| shade(nut, 0.8 + 0.3 * k),
                    );
                }
            }
        }
    }
    TreeMeshes { wood: b.wood, foliage: b.foliage, height }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::math::Vec2;

    #[test]
    fn growth_increases_height_and_complexity() {
        for style in TreeStyle::ALL {
            let young = generate_tree(style, 0.1, 7, Detail::Full);
            let old = generate_tree(style, 1.0, 7, Detail::Full);
            assert!(old.height > 3.0 * young.height, "{style:?}");
            assert!(old.wood.triangle_count() >= young.wood.triangle_count(), "{style:?}");
            let (_, hi) = old.wood.bounds().unwrap();
            assert!(hi.y > 0.5 * old.height && hi.y < 1.4 * old.height, "{style:?} top {}", hi.y);
            assert_eq!(old.foliage.is_empty(), !style.has_foliage(), "{style:?}");
        }
    }

    #[test]
    fn same_seed_same_tree_different_seed_different_tree() {
        let a = generate_tree(TreeStyle::Broadleaf, 0.8, 1, Detail::Full);
        let b = generate_tree(TreeStyle::Broadleaf, 0.8, 1, Detail::Full);
        let c = generate_tree(TreeStyle::Broadleaf, 0.8, 2, Detail::Full);
        assert_eq!(a, b);
        assert_ne!(a.wood.positions, c.wood.positions);
    }

    #[test]
    fn trunk_follows_elastic_similarity() {
        // r ∝ h^1.5: halving the height divides the trunk radius by 2^1.5.
        let r_at = |g: f32| {
            let t = generate_tree(TreeStyle::Conifer, g, 3, Detail::Full);
            // Widest point of the trunk near the ground.
            let base: Vec<f32> = t
                .wood
                .positions
                .iter()
                .filter(|p| p[1] < 0.01 * t.height)
                .map(|p| (p[0] * p[0] + p[2] * p[2]).sqrt())
                .collect();
            (base.iter().copied().fold(0.0, f32::max), t.height)
        };
        let (r1, h1) = r_at(1.0);
        let (r2, h2) = r_at(0.35);
        let predicted = (h2 / h1).powf(1.5);
        assert!(((r2 / r1) - predicted).abs() < 0.05 * predicted.max(0.05) + 0.01, "{} vs {}", r2 / r1, predicted);
    }

    #[test]
    fn foliage_is_lumpy_not_spherical() {
        let t = generate_tree(TreeStyle::Broadleaf, 1.0, 5, Detail::Full);
        assert!(t.foliage.triangle_count() > 300);
        // Every lump is an icosphere (level 2: 162 vertices) displaced by
        // noise. Around its equator (where vertical flattening plays no
        // role) a plain sphere would have a constant radius; the lumps'
        // radii vary by more than 10 %.
        let per_lump = 162;
        let mut checked = 0;
        for lump in t.foliage.positions.chunks_exact(per_lump) {
            let c = lump.iter().map(|p| Vec3::from(*p)).sum::<Vec3>() / per_lump as f32;
            let size = lump.iter().map(|p| Vec3::from(*p).distance(c)).fold(0.0f32, f32::max);
            let d: Vec<f32> = lump
                .iter()
                .map(|p| Vec3::from(*p) - c)
                .filter(|v| v.y.abs() < 0.1 * size)
                .map(|v| Vec2::new(v.x, v.z).length())
                .collect();
            let (lo, hi) = d.iter().fold((f32::MAX, 0.0f32), |(a, b), r| (a.min(*r), b.max(*r)));
            assert!(d.len() >= 6 && hi > 1.1 * lo, "a foliage lump is nearly spherical: {lo}..{hi}");
            checked += 1;
        }
        assert!(checked >= 6, "{checked}");
    }

    #[test]
    fn leaf_fall_thins_the_crown_in_place() {
        let full = generate_tree(TreeStyle::Broadleaf, 1.0, 9, Detail::Full);
        let half = generate_tree_with_leaf_cover(TreeStyle::Broadleaf, 1.0, 9, Detail::Full, 0.5);
        let bare = generate_tree_with_leaf_cover(TreeStyle::Broadleaf, 1.0, 9, Detail::Full, 0.0);
        assert_eq!(full.wood, half.wood, "leaf fall never changes the branches");
        assert_eq!(full.wood, bare.wood);
        assert!(bare.foliage.is_empty());
        let (f, h) = (full.foliage.triangle_count(), half.foliage.triangle_count());
        assert!(h > f / 5 && h < 4 * f / 5, "{h} of {f}");
        // Remaining clusters keep their positions: every vertex of the thinned
        // crown exists in the full crown.
        let all: std::collections::BTreeSet<[u32; 3]> =
            full.foliage.positions.iter().map(|p| p.map(f32::to_bits)).collect();
        assert!(half.foliage.positions.iter().all(|p| all.contains(&p.map(f32::to_bits))));
        // The crown keeps its height (no sinking towards the ground).
        let top = |m: &MeshData| m.bounds().map_or(0.0, |(_, hi)| hi.y);
        assert!(top(&half.foliage) > 0.85 * top(&full.foliage));
        // Evergreens ignore leaf cover.
        let pine = generate_tree(TreeStyle::Conifer, 1.0, 9, Detail::Full);
        assert_eq!(pine, generate_tree_with_leaf_cover(TreeStyle::Conifer, 1.0, 9, Detail::Full, 0.0));
    }
}
