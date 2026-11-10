//! Mesh data of the map: terrain, water surface, roads and the territory
//! overlay. Pure functions of the published view (unit tested); the scene
//! systems turn them into Bevy meshes.

use bevy::math::{Vec2, Vec3};
use civilization::Terrain;
use rendering::terrain::SurfaceMeshData;

use crate::coords::{HeightField, WATER_LEVEL, tile_hash};
use crate::palette::{SAND, SNOW, linear, mix_srgb, road_half_width, road_rgba, terrain_srgb};

/// Bottom of the map's earth skirt.
pub const SKIRT_BOTTOM: f32 = -0.9;
/// Lowest trail intensity drawn.
pub const TRAIL_MIN: f32 = 0.03;
/// Height of overlays above the terrain.
pub const OVERLAY_LIFT: f32 = 0.025;

/// Adds one tile as a fan of four triangles around its centre (counter-
/// clockwise seen from above). `vertex(local)` gives position, normal,
/// colour and UV of the centre (`(0.5, 0.5)`) and the corners.
fn push_fan(out: &mut SurfaceMeshData, mut vertex: impl FnMut(Vec2) -> ([f32; 3], [f32; 3], [f32; 4], [f32; 2])) {
    let base = out.positions.len() as u32;
    for local in [Vec2::splat(0.5), Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(1.0, 1.0), Vec2::new(0.0, 1.0)]
    {
        let (p, n, c, uv) = vertex(local);
        out.positions.push(p);
        out.normals.push(n);
        out.colors.push(c);
        out.uvs.push(uv);
    }
    // Corners: 1 = (0,0), 2 = (1,0), 3 = (1,1), 4 = (0,1).
    out.indices.extend_from_slice(&[base, base + 2, base + 1]);
    out.indices.extend_from_slice(&[base, base + 3, base + 2]);
    out.indices.extend_from_slice(&[base, base + 4, base + 3]);
    out.indices.extend_from_slice(&[base, base + 1, base + 4]);
}

/// Colour of a terrain tile with a per-tile shade jitter.
fn tile_color(terrain: Terrain, x: u32, y: u32) -> [u8; 3] {
    let base = terrain_srgb(terrain);
    let j = tile_hash(x, y, 3);
    match terrain {
        // Dry and lush patches.
        Terrain::Grassland => mix_srgb(base, [138, 152, 86], 0.4 * j),
        Terrain::Hills => mix_srgb(base, [118, 112, 104], 0.5 * j),
        _ => base,
    }
}

/// Terrain mesh: one fan per tile with crisp tile colours and smooth
/// normals, plus an earth skirt around the map.
pub fn terrain_mesh(terrain: &[Terrain], field: &HeightField) -> SurfaceMeshData {
    let (w, h) = (field.width(), field.height());
    let mut out = SurfaceMeshData::default();
    for y in 0..h {
        for x in 0..w {
            let t = terrain[(y * w + x) as usize];
            let srgb = tile_color(t, x, y);
            let shade = 0.97 + 0.06 * tile_hash(x, y, 11);
            push_fan(&mut out, |local| {
                let centre = local == Vec2::splat(0.5);
                let (cx, cy) = (x + local.x as u32, y + local.y as u32);
                let height = if centre { field.centre(x, y) } else { field.corner(cx, cy) };
                let normal = if centre { field.centre_normal(x, y) } else { field.corner_normal(cx, cy) };
                let mut colour = srgb;
                if t == Terrain::Mountain {
                    // Rock darkens towards the summits; snow caps the highest.
                    let rock = mix_srgb(srgb, [94, 90, 88], ((height - 0.4) / 0.8).clamp(0.0, 1.0));
                    let snow = ((height - 1.3) / 0.25).clamp(0.0, 1.0);
                    colour = mix_srgb(rock, SNOW, snow * snow * (3.0 - 2.0 * snow));
                } else if height < WATER_LEVEL - 0.02 {
                    // Shallow beds are sandy, deep ones dark.
                    let depth = (WATER_LEVEL - height).min(0.4) / 0.4;
                    colour = mix_srgb(SAND, [40, 76, 92], depth);
                }
                let p = [x as f32 + local.x, height, y as f32 + local.y];
                (p, normal.to_array(), linear(colour, shade), [p[0] / w as f32, p[2] / h as f32])
            });
        }
    }
    push_skirt(&mut out, terrain, field);
    out
}

/// Vertical earth walls along the map border (up to the water level where
/// the border tile is water).
fn push_skirt(out: &mut SurfaceMeshData, terrain: &[Terrain], field: &HeightField) {
    let (w, h) = (field.width(), field.height());
    let top = |cx: u32, cy: u32, tile: (u32, u32)| {
        let ground = field.corner(cx, cy);
        if terrain[(tile.1 * w + tile.0) as usize] == Terrain::Water { ground.max(WATER_LEVEL) } else { ground }
    };
    let soil = linear([96, 78, 60], 1.0);
    let dark = linear([58, 47, 38], 1.0);
    let mut wall = |a: Vec3, b: Vec3, normal: Vec3| {
        let base = out.positions.len() as u32;
        for (p, c) in
            [(Vec3::new(a.x, SKIRT_BOTTOM, a.z), dark), (Vec3::new(b.x, SKIRT_BOTTOM, b.z), dark), (b, soil), (a, soil)]
        {
            out.positions.push(p.to_array());
            out.normals.push(normal.to_array());
            out.colors.push(c);
            out.uvs.push([0.0, 0.0]);
        }
        out.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    };
    for x in 0..w {
        // North edge (z = 0), facing −Z; south edge (z = h), facing +Z.
        let (a, b) =
            (Vec3::new(x as f32, top(x, 0, (x, 0)), 0.0), Vec3::new(x as f32 + 1.0, top(x + 1, 0, (x, 0)), 0.0));
        wall(b, a, Vec3::NEG_Z);
        let row = h - 1;
        let (a, b) = (
            Vec3::new(x as f32, top(x, h, (x, row)), h as f32),
            Vec3::new(x as f32 + 1.0, top(x + 1, h, (x, row)), h as f32),
        );
        wall(a, b, Vec3::Z);
    }
    for y in 0..h {
        // West edge (x = 0), facing −X; east edge (x = w), facing +X.
        let (a, b) =
            (Vec3::new(0.0, top(0, y, (0, y)), y as f32), Vec3::new(0.0, top(0, y + 1, (0, y)), y as f32 + 1.0));
        wall(a, b, Vec3::NEG_X);
        let col = w - 1;
        let (a, b) = (
            Vec3::new(w as f32, top(w, y, (col, y)), y as f32),
            Vec3::new(w as f32, top(w, y + 1, (col, y)), y as f32 + 1.0),
        );
        wall(b, a, Vec3::X);
    }
}

/// Water surface over water tiles; UV.x carries the water depth (world
/// units) for the water shader's colour, opacity and shoreline fade.
pub fn water_mesh(terrain: &[Terrain], field: &HeightField) -> SurfaceMeshData {
    let (w, h) = (field.width(), field.height());
    let mut out = SurfaceMeshData::default();
    for y in 0..h {
        for x in 0..w {
            if terrain[(y * w + x) as usize] != Terrain::Water {
                continue;
            }
            push_fan(&mut out, |local| {
                let centre = local == Vec2::splat(0.5);
                let ground =
                    if centre { field.centre(x, y) } else { field.corner(x + local.x as u32, y + local.y as u32) };
                let p = [x as f32 + local.x, WATER_LEVEL, y as f32 + local.y];
                (p, [0.0, 1.0, 0.0], [1.0; 4], [WATER_LEVEL - ground, 0.0])
            });
        }
    }
    out
}

/// Road network: ribbons between neighbouring tiles that carry trails and a
/// disc on every trail tile. Tiles below [`TRAIL_MIN`] are not drawn; tiles
/// at or above `threshold` are roads (opaque, wider, lighter).
pub fn road_mesh(roads: &[f32], threshold: f32, field: &HeightField) -> SurfaceMeshData {
    let (w, h) = (field.width() as i64, field.height() as i64);
    let mut out = SurfaceMeshData::default();
    if roads.len() != (w * h) as usize {
        return out;
    }
    let at = |x: i64, y: i64| if x >= 0 && y >= 0 && x < w && y < h { roads[(y * w + x) as usize] } else { 0.0 };
    let lift = |p: Vec2, extra: f32| Vec3::new(p.x, field.height_at(p).max(WATER_LEVEL) + OVERLAY_LIFT + extra, p.y);
    for y in 0..h {
        for x in 0..w {
            let i = at(x, y);
            if i < TRAIL_MIN {
                continue;
            }
            let centre = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
            // Busier segments sit a hair higher so they win over faint ones.
            let extra = 0.002 * (i / threshold.max(1e-3)).min(2.0);
            disc(&mut out, centre, road_half_width(i, threshold), road_rgba(i, threshold), &|p| lift(p, extra));
            for (dx, dy) in [(1i64, 0i64), (0, 1), (1, 1), (-1, 1)] {
                let j = at(x + dx, y + dy);
                if j < TRAIL_MIN {
                    continue;
                }
                if dx != 0 && dy != 0 && (at(x + dx, y) >= TRAIL_MIN || at(x, y + dy) >= TRAIL_MIN) {
                    // A diagonal next to an orthogonal link would make a triangle.
                    continue;
                }
                let weaker = i.min(j);
                let to = Vec2::new((x + dx) as f32 + 0.5, (y + dy) as f32 + 0.5);
                ribbon(&mut out, centre, to, road_half_width(weaker, threshold), road_rgba(weaker, threshold), &|p| {
                    lift(p, extra)
                });
            }
        }
    }
    out
}

fn disc(out: &mut SurfaceMeshData, centre: Vec2, radius: f32, color: [f32; 4], lift: &dyn Fn(Vec2) -> Vec3) {
    const SIDES: usize = 8;
    let base = out.positions.len() as u32;
    let mut push = |p: Vec2| {
        out.positions.push(lift(p).to_array());
        out.normals.push([0.0, 1.0, 0.0]);
        out.colors.push(color);
        out.uvs.push([0.0, 0.0]);
    };
    push(centre);
    for k in 0..SIDES {
        let a = std::f32::consts::TAU * k as f32 / SIDES as f32;
        push(centre + Vec2::new(a.cos(), a.sin()) * radius);
    }
    for k in 0..SIDES as u32 {
        let (a, b) = (base + 1 + k, base + 1 + (k + 1) % SIDES as u32);
        // Angles grow from +X towards +Z, i.e. clockwise seen from above.
        out.indices.extend_from_slice(&[base, b, a]);
    }
}

fn ribbon(out: &mut SurfaceMeshData, from: Vec2, to: Vec2, half: f32, color: [f32; 4], lift: &dyn Fn(Vec2) -> Vec3) {
    const PIECES: usize = 4;
    let dir = (to - from).normalize_or_zero();
    let side = Vec2::new(-dir.y, dir.x) * half;
    let base = out.positions.len() as u32;
    for k in 0..=PIECES {
        let p = from.lerp(to, k as f32 / PIECES as f32);
        for q in [p - side, p + side] {
            out.positions.push(lift(q).to_array());
            out.normals.push([0.0, 1.0, 0.0]);
            out.colors.push(color);
            out.uvs.push([0.0, 0.0]);
        }
    }
    for k in 0..PIECES as u32 {
        let (l0, r0, l1, r1) = (base + 2 * k, base + 2 * k + 1, base + 2 * k + 2, base + 2 * k + 3);
        // Seen from above, `side` points to the left of travel when travel
        // is +X → +Z rotated; emit both windings' worth consistently upward.
        let up = (Vec3::from(out_pos(out, r0)) - Vec3::from(out_pos(out, l0)))
            .cross(Vec3::from(out_pos(out, l1)) - Vec3::from(out_pos(out, l0)));
        if up.y >= 0.0 {
            out.indices.extend_from_slice(&[l0, r0, l1, r0, r1, l1]);
        } else {
            out.indices.extend_from_slice(&[l0, l1, r0, r0, l1, r1]);
        }
    }
}

fn out_pos(out: &SurfaceMeshData, i: u32) -> [f32; 3] {
    out.positions[i as usize]
}

/// Territory overlay: translucent group colour on claimed tiles (stronger
/// on contested tiles) and opaque border strips where the owner changes.
/// `colors(id)` gives the linear RGB of a group id (`None` if unknown).
///
/// Water tiles are left untinted (a tint over the dark water reads as mud,
/// and nobody gathers there except by fishing from the shore); borders are
/// drawn on the land side only, so a claim that spans a lake is still
/// outlined along its shores where it meets another owner.
pub fn territory_mesh(
    owners: &[u32],
    contested: &[bool],
    terrain: &[Terrain],
    field: &HeightField,
    colors: &dyn Fn(u32) -> Option<[f32; 3]>,
) -> SurfaceMeshData {
    let (w, h) = (field.width(), field.height());
    let mut out = SurfaceMeshData::default();
    if owners.len() != (w * h) as usize || terrain.len() != owners.len() {
        return out;
    }
    let owner = |x: i64, y: i64| {
        if x < 0 || y < 0 || x >= i64::from(w) || y >= i64::from(h) {
            0
        } else {
            owners[(y * i64::from(w) + x) as usize]
        }
    };
    let lifted = |x: f32, z: f32| {
        let p = Vec2::new(x, z);
        Vec3::new(x, field.height_at(p).max(WATER_LEVEL) + OVERLAY_LIFT, z)
    };
    for y in 0..h {
        for x in 0..w {
            let id = owners[(y * w + x) as usize];
            if terrain[(y * w + x) as usize] == Terrain::Water {
                continue;
            }
            let Some(rgb) = (id != 0).then(|| colors(id)).flatten() else { continue };
            let disputed = contested.get((y * w + x) as usize).copied().unwrap_or(false);
            // A light wash inside, stronger on contested tiles; the claim's
            // outline is a crisp strip with a soft glow fading inwards, which
            // reads at every zoom without tinting the land too much.
            let fill = [rgb[0], rgb[1], rgb[2], if disputed { 0.34 } else { TERRITORY_FILL }];
            push_fan(&mut out, |local| {
                let p = lifted(x as f32 + local.x.clamp(0.0, 1.0), y as f32 + local.y.clamp(0.0, 1.0));
                (p.to_array(), [0.0, 1.0, 0.0], fill, [0.0, 0.0])
            });
            let border = [rgb[0], rgb[1], rgb[2], 0.95];
            let glow = [rgb[0], rgb[1], rgb[2], 0.42];
            let clear = [rgb[0], rgb[1], rgb[2], 0.0];
            let (fx, fy) = (x as f32, y as f32);
            let (xi, yi) = (i64::from(x), i64::from(y));
            // (neighbour offset, edge start, edge end, inward direction)
            let edges = [
                ((0, -1), (fx, fy), (fx + 1.0, fy), Vec2::Y),
                ((1, 0), (fx + 1.0, fy), (fx + 1.0, fy + 1.0), Vec2::NEG_X),
                ((0, 1), (fx + 1.0, fy + 1.0), (fx, fy + 1.0), Vec2::NEG_Y),
                ((-1, 0), (fx, fy + 1.0), (fx, fy), Vec2::X),
            ];
            for ((dx, dy), a, b, inward) in edges {
                if owner(xi + dx, yi + dy) == id {
                    continue;
                }
                let (a, b) = (Vec2::new(a.0, a.1), Vec2::new(b.0, b.1));
                let strip = (a + inward * BORDER_WIDTH, b + inward * BORDER_WIDTH);
                band(&mut out, [a, strip.0, strip.1, b], [border; 4], 0.004, &lifted);
                let inner = (a + inward * GLOW_WIDTH, b + inward * GLOW_WIDTH);
                band(&mut out, [strip.0, inner.0, inner.1, strip.1], [glow, clear, clear, glow], 0.003, &lifted);
            }
        }
    }
    out
}

/// Alpha of the wash on claimed, uncontested tiles.
pub const TERRITORY_FILL: f32 = 0.14;
/// Width (tiles) of the opaque territory outline.
pub const BORDER_WIDTH: f32 = 0.07;
/// Distance (tiles) from the edge at which the outline's glow has faded.
pub const GLOW_WIDTH: f32 = 0.45;

/// Quad `[outer start, inner start, inner end, outer end]` on the surface,
/// raised by `raise`, facing up.
fn band(
    out: &mut SurfaceMeshData,
    corners: [Vec2; 4],
    colors: [[f32; 4]; 4],
    raise: f32,
    lifted: &dyn Fn(f32, f32) -> Vec3,
) {
    let quad = corners.map(|c| lifted(c.x, c.y) + Vec3::Y * raise);
    let base = out.positions.len() as u32;
    for (p, c) in quad.iter().zip(colors) {
        out.positions.push(p.to_array());
        out.normals.push([0.0, 1.0, 0.0]);
        out.colors.push(c);
        out.uvs.push([0.0, 0.0]);
    }
    let up = (quad[1] - quad[0]).cross(quad[3] - quad[0]);
    if up.y >= 0.0 {
        out.indices.extend_from_slice(&[base, base + 1, base + 3, base + 1, base + 2, base + 3]);
    } else {
        out.indices.extend_from_slice(&[base, base + 3, base + 1, base + 1, base + 3, base + 2]);
    }
}

/// Positions (tile units) of the vegetation or rocks drawn on a tile:
/// `count` points jittered around the centre, deterministic per tile.
pub fn scatter_on_tile(x: u32, y: u32, count: usize, spread: f32, salt: u64) -> Vec<Vec2> {
    let centre = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
    (0..count)
        .map(|k| {
            let a = tile_hash(x, y, salt + 31 * k as u64) * std::f32::consts::TAU;
            let r = if count == 1 {
                0.5 * spread * tile_hash(x, y, salt + 7)
            } else {
                spread * (0.55 + 0.45 * tile_hash(x, y, salt + 13 * k as u64))
            };
            centre + Vec2::new(a.cos(), a.sin()) * r
        })
        .collect()
}

/// Triangles whose normal points downwards (should be none for top surfaces).
pub fn downward_triangles(mesh: &SurfaceMeshData) -> usize {
    mesh.indices
        .as_chunks::<3>()
        .0
        .iter()
        .filter(|t| {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| Vec3::from(mesh.positions[i as usize]));
            (b - a).cross(c - a).y < -1e-6
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(rows: &[&str]) -> (Vec<Terrain>, HeightField) {
        let terrain: Vec<Terrain> =
            rows.iter().flat_map(|r| r.chars().map(|c| Terrain::from_symbol(c).expect("symbol"))).collect();
        let field = HeightField::from_terrain(&terrain, rows[0].len() as u32, rows.len() as u32);
        (terrain, field)
    }

    #[test]
    fn terrain_mesh_covers_every_tile_and_faces_up() {
        let (terrain, field) = grid(&["..T~", "hh^~", "rbb."]);
        let mesh = terrain_mesh(&terrain, &field);
        let tiles = terrain.len();
        // 5 vertices per tile plus 4 per skirt segment (2·(w + h) segments).
        assert_eq!(mesh.positions.len(), tiles * 5 + 4 * 2 * (4 + 3));
        assert_eq!(mesh.uvs.len(), mesh.positions.len());
        assert_eq!(mesh.colors.len(), mesh.positions.len());
        // The top surface (first 4 triangles per tile) faces up.
        let top = SurfaceMeshData { indices: mesh.indices[..tiles * 12].to_vec(), ..mesh.clone() };
        assert_eq!(downward_triangles(&top), 0);
        assert!(mesh.indices.iter().all(|&i| (i as usize) < mesh.positions.len()));
    }

    #[test]
    fn water_mesh_only_on_water_with_depth_uvs() {
        let (terrain, field) = grid(&["..~~", "..~~", "...."]);
        let mesh = water_mesh(&terrain, &field);
        assert_eq!(mesh.positions.len(), 4 * 5);
        assert_eq!(downward_triangles(&mesh), 0);
        assert!(mesh.positions.iter().all(|p| p[1] == WATER_LEVEL));
        // Depth is positive over the bed centre, smaller towards the shore.
        assert!(mesh.uvs[0][0] > 0.2);
        assert!(mesh.uvs.iter().all(|uv| uv[0] < 0.6));
    }

    #[test]
    fn roads_link_neighbours_and_skip_faint_tiles() {
        let (_, field) = grid(&["....", "....", "...."]);
        let mut roads = vec![0.0; 12];
        assert!(road_mesh(&roads, 0.2, &field).positions.is_empty(), "no trails, no mesh");
        roads[4] = 0.5; // (0, 1)
        roads[5] = 0.5; // (1, 1)
        roads[6] = 0.01; // too faint
        let mesh = road_mesh(&roads, 0.2, &field);
        // Two discs (9 vertices each) and one ribbon (10 vertices).
        assert_eq!(mesh.positions.len(), 2 * 9 + 10);
        assert_eq!(downward_triangles(&mesh), 0);
        assert!(mesh.positions.iter().all(|p| p[1] > field.height_at(Vec2::new(p[0], p[2])) - 1e-4));
        // Diagonal next to an orthogonal link is skipped: an L shape gives 2 ribbons.
        roads[1] = 0.5; // (1, 0)
        let l = road_mesh(&roads, 0.2, &field);
        assert_eq!(l.positions.len(), 3 * 9 + 2 * 10);
        assert!(road_mesh(&roads[..5], 0.2, &field).positions.is_empty(), "wrong grid size");
    }

    #[test]
    fn territory_fills_owned_tiles_and_draws_borders() {
        let (terrain, field) = grid(&["...", "...", "..."]);
        let mut owners = vec![0u32; 9];
        owners[4] = 7; // the centre tile
        let colors = |id: u32| (id == 7).then_some([1.0, 0.0, 0.0]);
        let mesh = territory_mesh(&owners, &[false; 9], &terrain, &field, &colors);
        // One fan and four borders (strip and glow).
        assert_eq!(mesh.positions.len(), 5 + 4 * 8);
        assert_eq!(downward_triangles(&mesh), 0);
        owners[5] = 7;
        let two = territory_mesh(&owners, &[false; 9], &terrain, &field, &colors);
        // Two fans; the shared edge has no border: 3 + 3 borders.
        assert_eq!(two.positions.len(), 2 * 5 + 6 * 8);
        // The outline is opaque and its glow fades to nothing inside.
        assert!(two.colors[5][3] > 0.9 && two.colors.iter().any(|c| c[3] == 0.0));
        let disputed = [false, false, false, false, true, false, false, false, false];
        let contested = territory_mesh(&owners, &disputed, &terrain, &field, &colors);
        assert!(contested.colors[0][3] > two.colors[0][3], "contested tiles are more opaque");
        let unknown = territory_mesh(&[9u32; 9], &[false; 9], &terrain, &field, &colors);
        assert!(unknown.positions.is_empty(), "groups without a colour are skipped");
        // A claimed lake tile (centre) is not tinted; its land neighbours of
        // the same owner get no border towards it.
        let (lake, lake_field) = grid(&["...", ".~.", "..."]);
        let claimed = territory_mesh(&[7u32; 9], &[false; 9], &lake, &lake_field, &colors);
        // Eight land fans and the twelve outer borders.
        assert_eq!(claimed.positions.len(), 8 * 5 + 12 * 8);
        assert!(territory_mesh(&owners, &[false; 9], &terrain[..4], &field, &colors).positions.is_empty());
    }

    #[test]
    fn scatter_is_deterministic_and_stays_on_the_tile() {
        let a = scatter_on_tile(5, 9, 3, 0.3, 1);
        assert_eq!(a, scatter_on_tile(5, 9, 3, 0.3, 1));
        assert_eq!(a.len(), 3);
        for p in &a {
            assert!(p.x >= 5.0 && p.x < 6.0 && p.y >= 9.0 && p.y < 10.0, "{p:?}");
        }
        assert_ne!(a, scatter_on_tile(6, 9, 3, 0.3, 1));
    }
}
