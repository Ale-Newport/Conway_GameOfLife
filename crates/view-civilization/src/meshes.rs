//! Procedural meshes of the mode (built once, shared by every instance so
//! Bevy batches them): huts, group stores, farm plots and crops at five
//! growth stages, construction scaffolds, rocks, berry bushes, carried
//! goods and the articulated parts of the people.
//!
//! Units: one tile is one world unit; an adult is about 0.5 units tall.
//! Every mesh carries linear vertex colours that multiply its material:
//! parts drawn with the shared white material keep their own colours, parts
//! drawn with a group material (tunic, sleeves, pennants) are white or grey
//! so the group colour shows through.

use bevy::math::{Quat, Vec3};
use bevy::prelude::Transform;
use organism_render::procedural::mesh::{Cap, MeshData, Rgba, lump, superellipsoid, tube};

use crate::coords::tile_hash;
use crate::palette::linear;

fn rgb(srgb: [u8; 3]) -> Rgba {
    linear(srgb, 1.0)
}

fn shade(c: Rgba, k: f32) -> Rgba {
    [c[0] * k, c[1] * k, c[2] * k, c[3]]
}

/// Axis-aligned box with flat normals.
pub fn cuboid(out: &mut MeshData, center: Vec3, half: Vec3, color: Rgba) {
    let faces: [(Vec3, Vec3, Vec3); 6] = [
        (Vec3::X, Vec3::Y, Vec3::Z),
        (Vec3::NEG_X, Vec3::Y, Vec3::NEG_Z),
        (Vec3::Y, Vec3::Z, Vec3::X),
        (Vec3::NEG_Y, Vec3::X, Vec3::Z),
        (Vec3::Z, Vec3::Y, Vec3::NEG_X),
        (Vec3::NEG_Z, Vec3::Y, Vec3::X),
    ];
    for (n, u, v) in faces {
        // Top faces a touch lighter, bottoms darker: cheap ambient occlusion.
        let k = 1.0 + 0.08 * n.y;
        let c = shade(color, k);
        let o = center + n * half;
        let (hu, hv) = ((u * half).length(), (v * half).length());
        let p = [o - u * hu - v * hv, o + u * hu - v * hv, o + u * hu + v * hv, o - u * hu + v * hv];
        let base = out.vertex(p[0], n, c);
        out.vertex(p[1], n, c);
        out.vertex(p[2], n, c);
        out.vertex(p[3], n, c);
        // (u × v) = n for every face above, so this winding faces outwards.
        out.triangle(base, base + 1, base + 2);
        out.triangle(base, base + 2, base + 3);
    }
}

/// Upright cylinder (side and top), `sides` facets.
pub fn cylinder(
    out: &mut MeshData,
    base: Vec3,
    radius: f32,
    height: f32,
    sides: usize,
    color: &dyn Fn(f32, f32) -> Rgba,
) {
    let top = base + Vec3::Y * height;
    for k in 0..sides {
        let a0 = std::f32::consts::TAU * k as f32 / sides as f32;
        let a1 = std::f32::consts::TAU * (k + 1) as f32 / sides as f32;
        let (d0, d1) = (Vec3::new(a0.cos(), 0.0, a0.sin()), Vec3::new(a1.cos(), 0.0, a1.sin()));
        let i0 = out.vertex(base + d0 * radius, d0, color(a0, 0.0));
        let i1 = out.vertex(base + d1 * radius, d1, color(a1, 0.0));
        let i2 = out.vertex(top + d1 * radius, d1, color(a1, 1.0));
        let i3 = out.vertex(top + d0 * radius, d0, color(a0, 1.0));
        out.triangle(i0, i2, i1);
        out.triangle(i0, i3, i2);
        let c = out.vertex(top, Vec3::Y, color(0.0, 1.0));
        let t0 = out.vertex(top + d0 * radius, Vec3::Y, color(a0, 1.0));
        let t1 = out.vertex(top + d1 * radius, Vec3::Y, color(a1, 1.0));
        out.triangle(c, t1, t0);
    }
}

/// Cone from a base ring to an apex, with an underside.
pub fn cone(out: &mut MeshData, base: Vec3, radius: f32, apex: Vec3, sides: usize, color: &dyn Fn(f32, f32) -> Rgba) {
    let slope = radius / (apex - base).length().max(1e-4);
    for k in 0..sides {
        let a0 = std::f32::consts::TAU * k as f32 / sides as f32;
        let a1 = std::f32::consts::TAU * (k + 1) as f32 / sides as f32;
        let am = 0.5 * (a0 + a1);
        let (d0, d1, dm) = (
            Vec3::new(a0.cos(), 0.0, a0.sin()),
            Vec3::new(a1.cos(), 0.0, a1.sin()),
            Vec3::new(am.cos(), 0.0, am.sin()),
        );
        let n = |d: Vec3| (d + Vec3::Y * slope).normalize();
        let i0 = out.vertex(base + d0 * radius, n(d0), color(a0, 0.0));
        let i1 = out.vertex(base + d1 * radius, n(d1), color(a1, 0.0));
        let i2 = out.vertex(apex, n(dm), color(am, 1.0));
        out.triangle(i0, i2, i1);
        let u0 = out.vertex(base + d0 * radius, Vec3::NEG_Y, shade(color(a0, 0.0), 0.5));
        let u1 = out.vertex(base + d1 * radius, Vec3::NEG_Y, shade(color(a1, 0.0), 0.5));
        let uc = out.vertex(base, Vec3::NEG_Y, shade(color(a0, 0.0), 0.5));
        out.triangle(uc, u0, u1);
    }
}

/// Gable roof: a prism with the ridge along X.
fn gable_roof(out: &mut MeshData, center: Vec3, half_x: f32, half_z: f32, rise: f32, roof: Rgba, gable: Rgba) {
    let (y0, y1) = (center.y, center.y + rise);
    let ridge_a = Vec3::new(center.x - half_x, y1, center.z);
    let ridge_b = Vec3::new(center.x + half_x, y1, center.z);
    for side in [-1.0f32, 1.0] {
        let eave_a = Vec3::new(center.x - half_x, y0, center.z + side * half_z);
        let eave_b = Vec3::new(center.x + half_x, y0, center.z + side * half_z);
        let n = Vec3::new(0.0, half_z, side * rise).normalize();
        let c = shade(roof, if side > 0.0 { 1.0 } else { 0.92 });
        let a = out.vertex(eave_a, n, c);
        let b = out.vertex(eave_b, n, c);
        let r1 = out.vertex(ridge_b, n, shade(c, 1.08));
        let r0 = out.vertex(ridge_a, n, shade(c, 1.08));
        if side > 0.0 {
            out.triangle(a, b, r1);
            out.triangle(a, r1, r0);
        } else {
            out.triangle(a, r1, b);
            out.triangle(a, r0, r1);
        }
        // Underside so the roof is solid from below.
        let ua = out.vertex(eave_a, Vec3::NEG_Y, shade(roof, 0.4));
        let ub = out.vertex(eave_b, Vec3::NEG_Y, shade(roof, 0.4));
        let ur1 = out.vertex(ridge_b, Vec3::NEG_Y, shade(roof, 0.4));
        let ur0 = out.vertex(ridge_a, Vec3::NEG_Y, shade(roof, 0.4));
        if side > 0.0 {
            out.triangle(ua, ur1, ub);
            out.triangle(ua, ur0, ur1);
        } else {
            out.triangle(ua, ub, ur1);
            out.triangle(ua, ur1, ur0);
        }
    }
    for end in [-1.0f32, 1.0] {
        let x = center.x + end * (half_x - 0.02);
        let n = Vec3::X * end;
        let a = out.vertex(Vec3::new(x, y0, center.z - half_z * 0.86), n, gable);
        let b = out.vertex(Vec3::new(x, y0, center.z + half_z * 0.86), n, gable);
        let t = out.vertex(Vec3::new(x, y1 - 0.02, center.z), n, gable);
        if end > 0.0 {
            out.triangle(a, t, b);
        } else {
            out.triangle(a, b, t);
        }
    }
}

const WATTLE: [u8; 3] = [196, 166, 118];
const THATCH: [u8; 3] = [206, 170, 92];
const TIMBER: [u8; 3] = [140, 96, 58];
const DARK_WOOD: [u8; 3] = [82, 58, 38];
const STONE: [u8; 3] = [146, 142, 136];
const SOIL: [u8; 3] = [104, 76, 50];

/// Height of the pennant pole top above a building's base.
pub const HUT_FLAG_HEIGHT: f32 = 0.64;
/// Height of the pennant pole top of a group store.
pub const STORE_FLAG_HEIGHT: f32 = 0.62;

/// Round wattle hut with a thatched cone roof, door towards +Z.
pub fn hut_mesh() -> MeshData {
    let mut m = MeshData::new();
    let wall = rgb(WATTLE);
    cylinder(&mut m, Vec3::ZERO, 0.27, 0.035, 14, &|_, _| rgb(STONE));
    // Walls tall enough (and the eaves short enough) to show the wattle and
    // the door from the default tilted view.
    cylinder(&mut m, Vec3::Y * 0.035, 0.255, 0.2, 14, &|a, h| shade(wall, 0.92 + 0.08 * (a * 3.0).sin() + 0.06 * h));
    cuboid(&mut m, Vec3::new(0.0, 0.1, 0.255), Vec3::new(0.05, 0.075, 0.012), rgb(DARK_WOOD));
    let thatch = rgb(THATCH);
    cone(&mut m, Vec3::Y * 0.225, 0.31, Vec3::Y * 0.54, 18, &|a, h| {
        shade(thatch, 0.82 + 0.18 * h + 0.05 * (a * 9.0).sin())
    });
    cylinder(&mut m, Vec3::Y * 0.5, 0.008, HUT_FLAG_HEIGHT - 0.5, 5, &|_, _| rgb(DARK_WOOD));
    m
}

/// Raised timber granary on a stone plinth with a gable roof.
pub fn storage_mesh() -> MeshData {
    let mut m = MeshData::new();
    cuboid(&mut m, Vec3::new(0.0, 0.04, 0.0), Vec3::new(0.32, 0.04, 0.26), rgb(STONE));
    cuboid(&mut m, Vec3::new(0.0, 0.19, 0.0), Vec3::new(0.29, 0.11, 0.22), rgb(TIMBER));
    for (x, z) in [(-0.29f32, -0.22f32), (0.29, -0.22), (-0.29, 0.22), (0.29, 0.22)] {
        cuboid(&mut m, Vec3::new(x, 0.19, z), Vec3::new(0.022, 0.115, 0.022), rgb(DARK_WOOD));
    }
    // Planks: darker bands on the long walls.
    for k in -2..=2 {
        let x = k as f32 * 0.11;
        for z in [-0.222f32, 0.222] {
            cuboid(&mut m, Vec3::new(x, 0.19, z), Vec3::new(0.008, 0.1, 0.004), shade(rgb(TIMBER), 0.75));
        }
    }
    cuboid(&mut m, Vec3::new(0.0, 0.14, 0.226), Vec3::new(0.06, 0.07, 0.008), rgb(DARK_WOOD));
    gable_roof(&mut m, Vec3::new(0.0, 0.3, 0.0), 0.34, 0.29, 0.2, rgb([120, 86, 62]), rgb(TIMBER));
    cylinder(&mut m, Vec3::new(0.3, 0.46, 0.0), 0.008, STORE_FLAG_HEIGHT - 0.46, 5, &|_, _| rgb(DARK_WOOD));
    m
}

/// Pennant (white, tinted by the group material) hanging from a pole top
/// at the origin, pointing towards +X.
pub fn pennant_mesh() -> MeshData {
    let mut m = MeshData::new();
    let c = [1.0, 1.0, 1.0, 1.0];
    let (a, b, tip) = (Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, -0.075, 0.0), Vec3::new(0.12, -0.03, 0.0));
    let f0 = m.vertex(a, Vec3::Z, c);
    let f1 = m.vertex(b, Vec3::Z, c);
    let f2 = m.vertex(tip, Vec3::Z, c);
    m.triangle(f0, f1, f2);
    let k0 = m.vertex(a, Vec3::NEG_Z, shade(c, 0.85));
    let k1 = m.vertex(b, Vec3::NEG_Z, shade(c, 0.85));
    let k2 = m.vertex(tip, Vec3::NEG_Z, shade(c, 0.85));
    m.triangle(k0, k2, k1);
    m
}

/// Tilled field with furrows and a low fence (crops are separate).
pub fn farm_mesh() -> MeshData {
    let mut m = MeshData::new();
    cuboid(&mut m, Vec3::new(0.0, 0.014, 0.0), Vec3::new(0.44, 0.014, 0.44), rgb(SOIL));
    for k in 0..5 {
        let z = -0.32 + 0.16 * k as f32;
        cuboid(&mut m, Vec3::new(0.0, 0.03, z), Vec3::new(0.4, 0.012, 0.035), shade(rgb(SOIL), 1.22));
    }
    let wood = rgb(TIMBER);
    for (x, z) in [(-0.45f32, -0.45f32), (0.45, -0.45), (-0.45, 0.45), (0.45, 0.45), (0.0, -0.45), (0.0, 0.45)] {
        cuboid(&mut m, Vec3::new(x, 0.05, z), Vec3::new(0.012, 0.05, 0.012), shade(wood, 0.9));
    }
    for z in [-0.45f32, 0.45] {
        cuboid(&mut m, Vec3::new(0.0, 0.07, z), Vec3::new(0.45, 0.006, 0.006), wood);
    }
    for x in [-0.45f32, 0.45] {
        cuboid(&mut m, Vec3::new(x, 0.07, 0.0), Vec3::new(0.006, 0.006, 0.45), wood);
    }
    m
}

/// Number of crop stages ([`crop_stage`]).
pub const CROP_STAGES: usize = 5;

/// Visual stage of a crop: 0 sprouts … 3 ripening, 4 ripe (growth ≥ 1).
pub fn crop_stage(growth: f32) -> usize {
    if growth >= 1.0 { 4 } else { ((growth.max(0.0) * 4.0) as usize).min(3) }
}

/// Crops on the furrows of a farm plot at a growth stage.
pub fn crop_mesh(stage: usize) -> MeshData {
    let mut m = MeshData::new();
    let stage = stage.min(CROP_STAGES - 1);
    let height = [0.035, 0.07, 0.11, 0.14, 0.15][stage];
    let green = rgb([96, 156, 64]);
    let gold = rgb([222, 186, 84]);
    let stalk = match stage {
        0..=2 => green,
        3 => rgb([168, 172, 76]),
        _ => gold,
    };
    for row in 0..5 {
        let z = -0.32 + 0.16 * row as f32;
        for k in 0..7 {
            let x = -0.33 + 0.11 * k as f32 + 0.02 * tile_hash(k, row, 5);
            let h = height * (0.85 + 0.3 * tile_hash(k, row, 9));
            let base = Vec3::new(x, 0.04, z);
            // A tuft of three leaning blades.
            for b in 0..3 {
                let a = b as f32 * 2.1 + 0.5 * tile_hash(k, row, b);
                let lean = Vec3::new(a.cos(), 0.0, a.sin()) * h * 0.25;
                tube(
                    &mut m,
                    &[base, base + Vec3::Y * h + lean],
                    &[0.008, 0.003],
                    4,
                    Vec3::Z,
                    (Cap::Flat, Cap::Point(0.004)),
                    &|s| shade(stalk, 0.85 + 0.25 * s.along),
                );
            }
            if stage >= 3 {
                let ear = base + Vec3::Y * (h + 0.012);
                superellipsoid(&mut m, ear, Vec3::new(0.012, 0.025, 0.012), 1.0, 1.0, 4, 6, &|_| {
                    shade(if stage == 4 { gold } else { rgb([190, 180, 90]) }, 1.05)
                });
            }
        }
    }
    m
}

/// Construction scaffold: corner posts, beams and a pile of materials.
pub fn scaffold_mesh() -> MeshData {
    let mut m = MeshData::new();
    let wood = rgb([168, 124, 76]);
    for (x, z) in [(-0.26f32, -0.26f32), (0.26, -0.26), (-0.26, 0.26), (0.26, 0.26)] {
        cuboid(&mut m, Vec3::new(x, 0.2, z), Vec3::new(0.012, 0.2, 0.012), wood);
    }
    for z in [-0.26f32, 0.26] {
        cuboid(&mut m, Vec3::new(0.0, 0.36, z), Vec3::new(0.27, 0.01, 0.01), wood);
        cuboid(&mut m, Vec3::new(0.0, 0.18, z), Vec3::new(0.27, 0.008, 0.008), shade(wood, 0.9));
    }
    for x in [-0.26f32, 0.26] {
        cuboid(&mut m, Vec3::new(x, 0.36, 0.0), Vec3::new(0.01, 0.01, 0.27), wood);
    }
    // Pile of logs and stones beside the site.
    for k in 0..3 {
        let y = 0.02 + 0.03 * k as f32;
        let z = 0.36 + 0.015 * (k % 2) as f32;
        tube(
            &mut m,
            &[Vec3::new(-0.16, y, z), Vec3::new(0.12, y, z)],
            &[0.016, 0.016],
            6,
            Vec3::Y,
            (Cap::Flat, Cap::Flat),
            &|_| rgb([150, 104, 60]),
        );
    }
    lump(&mut m, Vec3::new(0.28, 0.045, 0.36), Vec3::new(0.05, 0.035, 0.045), 1, 0.2, 2.0, 3, &|_, k| {
        shade(rgb(STONE), 0.8 + 0.2 * k)
    });
    m
}

/// Rock outcrop (four shape variants).
pub fn rock_mesh(variant: u32) -> MeshData {
    let mut m = MeshData::new();
    let base = rgb([118, 114, 108]);
    let r = Vec3::new(0.16, 0.11, 0.13) * (0.85 + 0.1 * variant as f32);
    lump(&mut m, Vec3::Y * r.y * 0.45, r, 2, 0.28, 2.2, 17 + variant, &|d, k| {
        shade(base, (0.72 + 0.3 * (k - 0.8) + 0.15 * d.y).clamp(0.5, 1.15))
    });
    if variant.is_multiple_of(2) {
        lump(&mut m, Vec3::new(0.11, 0.035, 0.06), r * 0.45, 1, 0.25, 2.0, 5 + variant, &|_, k| {
            shade(base, 0.8 + 0.2 * k)
        });
    }
    m
}

/// Number of berry levels of a bush ([`berry_level`]).
pub const BERRY_LEVELS: usize = 3;

/// Berry level of a bush from the tile's berry stock fraction.
pub fn berry_level(stock: f32) -> usize {
    if stock < 0.15 {
        0
    } else if stock < 0.6 {
        1
    } else {
        2
    }
}

/// Berry bush with `level` (0 bare, 1 some, 2 laden) berries.
pub fn bush_mesh(level: usize, variant: u32) -> MeshData {
    let mut m = MeshData::new();
    let leaf = rgb([62, 112, 52]);
    lump(&mut m, Vec3::Y * 0.1, Vec3::new(0.15, 0.12, 0.14), 2, 0.22, 3.0, 40 + variant, &|d, k| {
        shade(leaf, (0.7 + 0.25 * d.y + 0.2 * (k - 1.0)).clamp(0.45, 1.2))
    });
    let berries = [0, 7, 16][level.min(BERRY_LEVELS - 1)];
    for i in 0..berries {
        let a = i as f32 * 2.399_963 + variant as f32;
        let up = 0.15 + 0.7 * tile_hash(i, variant, 3);
        let d = Vec3::new(a.cos() * (1.0 - up * up).sqrt(), up, a.sin() * (1.0 - up * up).sqrt());
        let p = Vec3::Y * 0.1 + d * Vec3::new(0.15, 0.12, 0.14) * 1.02;
        lump(&mut m, p, Vec3::splat(0.024), 1, 0.0, 1.0, i, &|_, _| rgb([196, 34, 64]));
    }
    m
}

/// Skin tones of the people.
pub const SKIN: [[u8; 3]; 4] = [[236, 196, 164], [206, 150, 110], [160, 108, 74], [110, 72, 50]];
/// Hair colours.
pub const HAIR: [[u8; 3]; 3] = [[52, 36, 26], [122, 78, 40], [196, 160, 98]];

/// Hip height of an adult (legs hang from it).
pub const HIP: f32 = 0.2;
/// Shoulder height above the hips.
pub const SHOULDER: f32 = 0.145;
/// Shoulder half span.
pub const SHOULDER_X: f32 = 0.072;
/// Hip half span.
pub const HIP_X: f32 = 0.034;
/// Head centre above the hips.
pub const HEAD: f32 = 0.215;

/// Tunic (white and grey, tinted by the group material); origin at the hips.
pub fn torso_mesh() -> MeshData {
    let mut m = MeshData::new();
    superellipsoid(&mut m, Vec3::Y * 0.085, Vec3::new(0.066, 0.092, 0.045), 0.6, 0.9, 10, 12, &|s| {
        // A darker belt around the waist.
        let y = s.position.y;
        if (0.035..0.05).contains(&y) { [0.32, 0.3, 0.28, 1.0] } else { [1.0, 1.0, 1.0, 1.0] }
    });
    m
}

/// Sleeve hanging from the shoulder (origin), tinted by the group material.
pub fn sleeve_mesh() -> MeshData {
    let mut m = MeshData::new();
    tube(
        &mut m,
        &[Vec3::ZERO, Vec3::new(0.0, -0.075, 0.0), Vec3::new(0.0, -0.125, 0.0)],
        &[0.022, 0.019, 0.016],
        8,
        Vec3::Z,
        (Cap::Round, Cap::Flat),
        &|_| [0.9, 0.9, 0.9, 1.0],
    );
    m
}

/// Hand at the end of a sleeve (origin at the shoulder).
pub fn hand_mesh(skin: usize) -> MeshData {
    let mut m = MeshData::new();
    superellipsoid(&mut m, Vec3::new(0.0, -0.14, 0.0), Vec3::new(0.016, 0.02, 0.016), 1.0, 1.0, 5, 8, &|_| {
        rgb(SKIN[skin % SKIN.len()])
    });
    m
}

/// Head with hair and eyes facing +Z; origin at the hips.
pub fn head_mesh(skin: usize, hair: usize) -> MeshData {
    let mut m = MeshData::new();
    let skin_c = rgb(SKIN[skin % SKIN.len()]);
    let hair_c = rgb(HAIR[hair % HAIR.len()]);
    // Neck.
    tube(&mut m, &[Vec3::Y * 0.16, Vec3::Y * 0.19], &[0.018, 0.017], 8, Vec3::Z, (Cap::Flat, Cap::Flat), &|_| {
        shade(skin_c, 0.9)
    });
    superellipsoid(&mut m, Vec3::Y * HEAD, Vec3::new(0.047, 0.055, 0.05), 1.0, 1.0, 10, 14, &|_| skin_c);
    superellipsoid(
        &mut m,
        Vec3::new(0.0, HEAD + 0.02, -0.007),
        Vec3::new(0.052, 0.042, 0.053),
        0.9,
        1.0,
        8,
        14,
        &|s| {
            if s.position.z > 0.02 && s.position.y < HEAD + 0.02 { skin_c } else { hair_c }
        },
    );
    for x in [-0.017f32, 0.017] {
        superellipsoid(
            &mut m,
            Vec3::new(x, HEAD + 0.004, 0.046),
            Vec3::new(0.007, 0.008, 0.005),
            1.0,
            1.0,
            4,
            6,
            &|_| [0.03, 0.03, 0.04, 1.0],
        );
    }
    m
}

/// Leg hanging from the hip (origin), trousers and boot.
pub fn leg_mesh() -> MeshData {
    let mut m = MeshData::new();
    let cloth = rgb([92, 78, 64]);
    tube(
        &mut m,
        &[Vec3::ZERO, Vec3::new(0.0, -0.1, 0.0), Vec3::new(0.0, -0.185, 0.0)],
        &[0.026, 0.022, 0.019],
        8,
        Vec3::Z,
        (Cap::Round, Cap::Flat),
        &|_| cloth,
    );
    cuboid(&mut m, Vec3::new(0.0, -0.19, 0.012), Vec3::new(0.019, 0.012, 0.032), rgb([54, 40, 30]));
    m
}

/// Kinds of carried bundle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Carry {
    /// Basket of food.
    Food,
    /// Bundle of logs.
    Wood,
    /// Sack of stones.
    Stone,
}

/// Carried bundle on the back (origin at the hips, back is −Z).
pub fn carry_mesh(kind: Carry) -> MeshData {
    let mut m = MeshData::new();
    let back = Vec3::new(0.0, 0.1, -0.075);
    match kind {
        Carry::Food => {
            cylinder(&mut m, back - Vec3::Y * 0.04, 0.045, 0.07, 10, &|a, h| {
                shade(rgb([168, 120, 64]), 0.85 + 0.12 * (a * 6.0).sin().abs() + 0.05 * h)
            });
            for (i, c) in [[214u8, 74, 58], [230, 150, 52], [204, 60, 80], [120, 170, 70]].iter().enumerate() {
                let a = i as f32 * 1.7;
                lump(
                    &mut m,
                    back + Vec3::new(0.02 * a.cos(), 0.04, 0.02 * a.sin()),
                    Vec3::splat(0.022),
                    1,
                    0.1,
                    2.0,
                    i as u32,
                    &|_, _| rgb(*c),
                );
            }
        }
        Carry::Wood => {
            for (k, (dy, dz)) in [(0.0f32, 0.0f32), (0.032, 0.0), (0.016, -0.03)].iter().enumerate() {
                let p = back + Vec3::new(0.0, *dy, *dz);
                tube(
                    &mut m,
                    &[p - Vec3::X * 0.08, p + Vec3::X * 0.08],
                    &[0.017, 0.017],
                    7,
                    Vec3::Y,
                    (Cap::Flat, Cap::Flat),
                    &|s| {
                        if s.along < 0.02 || s.along > 0.98 {
                            rgb([214, 182, 128])
                        } else {
                            shade(rgb([132, 90, 52]), 0.9 + 0.05 * k as f32)
                        }
                    },
                );
            }
        }
        Carry::Stone => {
            lump(&mut m, back, Vec3::new(0.05, 0.045, 0.045), 1, 0.2, 2.5, 9, &|_, k| {
                shade(rgb([154, 152, 146]), 0.75 + 0.25 * k)
            });
        }
    }
    m
}

/// Small hand axe worn on the right hip when carrying tools.
pub fn tool_mesh() -> MeshData {
    let mut m = MeshData::new();
    let rot = Transform::from_rotation(Quat::from_rotation_z(0.35));
    let mut piece = MeshData::new();
    tube(
        &mut piece,
        &[Vec3::new(0.0, -0.05, 0.0), Vec3::new(0.0, 0.05, 0.0)],
        &[0.006, 0.006],
        5,
        Vec3::Z,
        (Cap::Flat, Cap::Flat),
        &|_| rgb([120, 84, 50]),
    );
    cuboid(&mut piece, Vec3::new(0.012, 0.045, 0.0), Vec3::new(0.016, 0.012, 0.004), rgb([170, 176, 186]));
    m.append_transformed(&piece, &rot);
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(mesh: &MeshData) {
        assert!(!mesh.is_empty());
        assert!(mesh.indices.iter().all(|&i| (i as usize) < mesh.vertex_count()));
        assert!(mesh.positions.iter().flatten().all(|v| v.is_finite()));
        assert_eq!(mesh.colors.len(), mesh.vertex_count());
    }

    /// Fraction of triangles whose normal points away from the mesh centre
    /// (closed convex-ish parts should be mostly outward).
    fn outward_fraction(mesh: &MeshData) -> f32 {
        let (lo, hi) = mesh.bounds().expect("non-empty");
        let c = 0.5 * (lo + hi);
        let n = mesh.indices.len() / 3;
        let out = mesh
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .filter(|t| {
                let [a, b, d] = [t[0], t[1], t[2]].map(|i| Vec3::from(mesh.positions[i as usize]));
                (b - a).cross(d - a).dot((a + b + d) / 3.0 - c) >= 0.0
            })
            .count();
        out as f32 / n as f32
    }

    #[test]
    fn building_meshes_are_valid_and_sized_to_a_tile() {
        for mesh in [hut_mesh(), storage_mesh(), farm_mesh(), scaffold_mesh()] {
            check(&mesh);
            let (lo, hi) = mesh.bounds().unwrap();
            assert!(lo.x >= -0.5 && hi.x <= 0.5 && lo.z >= -0.5 && hi.z <= 0.5, "fits its tile: {lo:?} {hi:?}");
            assert!(lo.y >= -1e-4, "nothing below the ground");
        }
        let (_, hut_top) = hut_mesh().bounds().unwrap();
        assert!((hut_top.y - HUT_FLAG_HEIGHT).abs() < 1e-4, "the pole carries the pennant");
        let (_, store_top) = storage_mesh().bounds().unwrap();
        assert!((store_top.y - STORE_FLAG_HEIGHT).abs() < 1e-4);
        check(&pennant_mesh());
        let cube = {
            let mut m = MeshData::new();
            cuboid(&mut m, Vec3::ZERO, Vec3::splat(0.5), [1.0; 4]);
            m
        };
        assert_eq!(outward_fraction(&cube), 1.0, "box faces point outwards");
    }

    #[test]
    fn crops_grow_through_their_stages() {
        assert_eq!(crop_stage(0.0), 0);
        assert_eq!(crop_stage(0.3), 1);
        assert_eq!(crop_stage(0.99), 3);
        assert_eq!(crop_stage(1.0), 4);
        assert_eq!(crop_stage(f32::NAN), 0);
        let heights: Vec<f32> = (0..CROP_STAGES).map(|s| crop_mesh(s).bounds().unwrap().1.y).collect();
        assert!(heights.windows(2).all(|w| w[1] >= w[0]), "{heights:?}");
        for s in 0..CROP_STAGES {
            check(&crop_mesh(s));
        }
    }

    #[test]
    fn people_parts_assemble_to_a_person() {
        for mesh in [torso_mesh(), sleeve_mesh(), hand_mesh(1), head_mesh(2, 1), leg_mesh(), tool_mesh()] {
            check(&mesh);
        }
        for kind in [Carry::Food, Carry::Wood, Carry::Stone] {
            check(&carry_mesh(kind));
            let (_, hi) = carry_mesh(kind).bounds().unwrap();
            assert!(hi.z < 0.0, "{kind:?} is carried on the back (-Z)");
        }
        // Feet reach the ground when the hips are at HIP.
        let (leg_lo, _) = leg_mesh().bounds().unwrap();
        assert!((HIP + leg_lo.y).abs() < 0.01, "{}", HIP + leg_lo.y);
        let (_, head_hi) = head_mesh(0, 0).bounds().unwrap();
        let height = HIP + head_hi.y;
        assert!((0.45..0.55).contains(&height), "adult height {height}");
        // Eyes face +Z (the model's forward).
        let (_, hi) = head_mesh(0, 0).bounds().unwrap();
        assert!(hi.z > 0.045);
    }

    #[test]
    fn vegetation_meshes_and_levels() {
        assert_eq!(berry_level(0.0), 0);
        assert_eq!(berry_level(0.3), 1);
        assert_eq!(berry_level(0.9), 2);
        let bare = bush_mesh(0, 1);
        let laden = bush_mesh(2, 1);
        check(&bare);
        assert!(laden.vertex_count() > bare.vertex_count(), "berries add geometry");
        for v in 0..4 {
            check(&rock_mesh(v));
        }
    }
}
