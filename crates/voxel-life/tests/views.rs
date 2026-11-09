//! Surface extraction against its definition, evaluated voxel by voxel with
//! the public `state_at` accessor, on every world kind and slice axis.

mod common;

use common::*;
use simulation_core::{SimRng, Simulation};
use voxel_life::{Axis, Pos, SliceRange, SurfaceVoxel, VoxelView, VoxelViewRequest, WorldKind, faces};

const FACES: [(Pos, u8); 6] = [
    ([-1, 0, 0], faces::NEG_X),
    ([1, 0, 0], faces::POS_X),
    ([0, -1, 0], faces::NEG_Y),
    ([0, 1, 0], faces::POS_Y),
    ([0, 0, -1], faces::NEG_Z),
    ([0, 0, 1], faces::POS_Z),
];

/// The module-level surface rule: a non-empty voxel inside the range is
/// listed iff a face neighbour is empty, outside the world box (no wrapping,
/// also on a torus) or outside the range.
fn reference_surface(life: &voxel_life::VoxelLife, range: &SliceRange) -> Vec<SurfaceVoxel> {
    let world = life.bounds();
    let mut out = Vec::new();
    for voxel in life.voxels() {
        if !range.contains(voxel.pos) {
            continue;
        }
        let mut mask = 0;
        for (d, bit) in FACES {
            let n = [voxel.pos[0] + d[0], voxel.pos[1] + d[1], voxel.pos[2] + d[2]];
            if !world.contains(n) || !range.contains(n) || life.state_at(n) == 0 {
                mask |= bit;
            }
        }
        if mask != 0 {
            out.push(SurfaceVoxel { pos: voxel.pos, state: voxel.state, age: voxel.age, faces: mask });
        }
    }
    out
}

fn sorted(mut voxels: Vec<SurfaceVoxel>) -> Vec<SurfaceVoxel> {
    voxels.sort_by_key(|v| (v.pos[2], v.pos[1], v.pos[0]));
    voxels
}

#[test]
fn surface_matches_its_definition_on_every_world_kind() {
    let mut rng = SimRng::seed_from_u64(31);
    let setups: [(WorldKind, [u32; 3], Pos); 4] = [
        (WorldKind::DenseBounded, [21, 18, 19], [0, 0, 0]),
        (WorldKind::DenseTorus, [17, 20, 16], [0, 0, 0]),
        (WorldKind::SparseBounded, [37, 20, 34], [0, 0, 0]),
        // Crosses chunk borders at 0 and ±16 on every axis.
        (WorldKind::SparseUnbounded, [8, 8, 8], [-20, -17, -3]),
    ];
    for (kind, size, lo) in setups {
        let side = if kind == WorldKind::SparseUnbounded { [40, 36, 38] } else { size.map(|s| s as i32) };
        // A dense soup (so most voxels are enclosed) with ages, then decay.
        let mut life = world("9-26/5-7,12-13,15/5/M", kind, size);
        load(&mut life, &soup(9, lo, side, 0.7, 5));
        life.advance(2);
        let mut ranges = vec![SliceRange::all()];
        for axis in Axis::ALL {
            let a = axis.index();
            for _ in 0..3 {
                let min = lo[a] - 2 + rng.below(side[a] as u64 + 2) as i32;
                ranges.push(SliceRange::new(axis, min, min + rng.below(12) as i32));
            }
            ranges.push(SliceRange::new(axis, lo[a] + 3, lo[a] + 3));
            ranges.push(SliceRange::new(axis, i32::MIN, lo[a] + 5));
        }
        for range in ranges {
            let request = VoxelViewRequest::Surface { range, max_voxels: usize::MAX };
            let VoxelView::Surface(view) = life.view(&request) else { panic!("surface view") };
            let expected = reference_surface(&life, &range);
            assert!(!expected.is_empty(), "{kind:?} {range:?}: the soup must be visible");
            assert_eq!(view.visible, expected.len() as u64, "{kind:?} {range:?}");
            assert_eq!(sorted(view.voxels), expected, "{kind:?} {range:?}");
        }
    }
}
