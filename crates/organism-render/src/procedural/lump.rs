//! Noise-displaced icospheres ("lumps") used for foliage clusters, shrub
//! crowns and small round details. Re-exported as [`super::mesh::lump`].

use bevy::math::Vec3;

use super::mesh::{MeshData, Rgba};
use super::noise::fbm3;

/// Unit icosahedron subdivided `level` times (positions on the unit sphere).
fn icosphere(level: u32) -> (Vec<Vec3>, Vec<[u32; 3]>) {
    let t = (1.0 + 5.0f32.sqrt()) / 2.0;
    let mut v: Vec<Vec3> = [
        (-1.0, t, 0.0),
        (1.0, t, 0.0),
        (-1.0, -t, 0.0),
        (1.0, -t, 0.0),
        (0.0, -1.0, t),
        (0.0, 1.0, t),
        (0.0, -1.0, -t),
        (0.0, 1.0, -t),
        (t, 0.0, -1.0),
        (t, 0.0, 1.0),
        (-t, 0.0, -1.0),
        (-t, 0.0, 1.0),
    ]
    .iter()
    .map(|&(x, y, z)| Vec3::new(x, y, z).normalize())
    .collect();
    let mut f: Vec<[u32; 3]> = vec![
        [0, 11, 5],
        [0, 5, 1],
        [0, 1, 7],
        [0, 7, 10],
        [0, 10, 11],
        [1, 5, 9],
        [5, 11, 4],
        [11, 10, 2],
        [10, 7, 6],
        [7, 1, 8],
        [3, 9, 4],
        [3, 4, 2],
        [3, 2, 6],
        [3, 6, 8],
        [3, 8, 9],
        [4, 9, 5],
        [2, 4, 11],
        [6, 2, 10],
        [8, 6, 7],
        [9, 8, 1],
    ];
    for _ in 0..level {
        let mut cache = std::collections::BTreeMap::new();
        let mut mid = |a: u32, b: u32, v: &mut Vec<Vec3>| -> u32 {
            let key = (a.min(b), a.max(b));
            *cache.entry(key).or_insert_with(|| {
                v.push(((v[a as usize] + v[b as usize]) * 0.5).normalize());
                (v.len() - 1) as u32
            })
        };
        let mut next = Vec::with_capacity(f.len() * 4);
        for [a, b, c] in f {
            let ab = mid(a, b, &mut v);
            let bc = mid(b, c, &mut v);
            let ca = mid(c, a, &mut v);
            next.extend_from_slice(&[[a, ab, ca], [b, bc, ab], [c, ca, bc], [ab, bc, ca]]);
        }
        f = next;
    }
    (v, f)
}

/// Noise-displaced, squashed icosphere ("lump"). `amplitude` is the relative
/// radial displacement; `seed` selects the noise pattern. The colour
/// function receives the unit direction and the displaced radius factor.
#[allow(clippy::too_many_arguments)]
pub fn lump(
    out: &mut MeshData,
    center: Vec3,
    radii: Vec3,
    level: u32,
    amplitude: f32,
    frequency: f32,
    seed: u32,
    color: &dyn Fn(Vec3, f32) -> Rgba,
) {
    let (dirs, faces) = icosphere(level.min(3));
    let mut piece = MeshData::new();
    for d in &dirs {
        let k = 1.0 + amplitude * fbm3(*d * frequency, seed, 3);
        let p = center + *d * radii * k;
        piece.vertex(p, *d, color(*d, k));
    }
    for [a, b, c] in faces {
        piece.triangle(a, b, c);
    }
    piece.recompute_normals();
    out.append(&piece);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Signed volume via the divergence theorem; positive for outward winding.
    fn signed_volume(m: &MeshData) -> f32 {
        m.indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|t| {
                let [a, b, c] = [t[0], t[1], t[2]].map(|i| Vec3::from(m.positions[i as usize]));
                a.dot(b.cross(c)) / 6.0
            })
            .sum()
    }

    #[test]
    fn icosphere_levels_have_euler_counts() {
        for level in 0..3 {
            let (v, f) = icosphere(level);
            let faces = 20 * 4usize.pow(level);
            assert_eq!(f.len(), faces);
            assert_eq!(v.len(), faces / 2 + 2, "V - E + F = 2 with E = 3F/2");
            assert!(v.iter().all(|p| (p.length() - 1.0).abs() < 1e-5));
        }
    }

    #[test]
    fn lump_is_closed_and_displaced() {
        let mut m = MeshData::new();
        lump(&mut m, Vec3::ZERO, Vec3::ONE, 2, 0.25, 2.0, 7, &|_, _| [1.0; 4]);
        assert!(m.normals.iter().all(|n| (Vec3::from(*n).length() - 1.0).abs() < 1e-3));
        let radii: Vec<f32> = m.positions.iter().map(|p| Vec3::from(*p).length()).collect();
        let (lo, hi) = radii.iter().fold((f32::MAX, 0.0f32), |(a, b), r| (a.min(*r), b.max(*r)));
        assert!(hi - lo > 0.05, "displacement visible");
        assert!(signed_volume(&m) > 0.0);
    }
}
