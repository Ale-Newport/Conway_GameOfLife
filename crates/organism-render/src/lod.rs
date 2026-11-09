//! Visual level of detail for organisms, chosen from the camera distance
//! relative to the organism's size. It only changes how an organism is
//! drawn and never feeds back into the simulation.
//!
//! | level | drawn as |
//! |---|---|
//! | `Near` | full animated model (GLB clips or procedural rig) |
//! | `Mid` | same model, animation pose updated every `mid_update_interval` frames |
//! | `Far` | static, low-segment procedural mesh merged into one draw per material |
//! | `Marker` | camera-facing coloured disc of roughly constant screen size |

use bevy::diagnostic::FrameCount;
use bevy::prelude::*;

use crate::catalog::OrganismCatalog;
use crate::organism::AnimatedOrganism;
use rendering::WorldCamera;

/// Level of detail.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LodLevel {
    /// Full animation.
    #[default]
    Near,
    /// Animation at reduced update rate.
    Mid,
    /// Static simplified mesh.
    Far,
    /// Billboard marker.
    Marker,
}

/// Per-organism LOD state, written by [`update_organism_lod`].
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct OrganismLod {
    /// Current level.
    pub level: LodLevel,
    /// Camera distance (m) at the last update.
    pub distance: f32,
    /// Whether a `Mid` organism refreshes its pose this frame.
    pub update_this_frame: bool,
    /// Fixed level overriding the distance rule (galleries, inspectors).
    pub pinned: Option<LodLevel>,
}

/// Distance thresholds (m) for a 1 m organism; they scale with
/// `extent^size_exponent` so large animals keep detail further away.
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct LodSettings {
    /// Near → mid distance.
    pub near: f32,
    /// Mid → far distance.
    pub mid: f32,
    /// Far → marker distance.
    pub far: f32,
    /// Exponent of the size scaling.
    pub size_exponent: f32,
    /// Fractional hysteresis band preventing flicker at boundaries.
    pub hysteresis: f32,
    /// Frames between pose updates at `Mid`.
    pub mid_update_interval: u32,
    /// Marker diameter as a fraction of the camera distance.
    pub marker_screen_fraction: f32,
}

impl Default for LodSettings {
    fn default() -> Self {
        Self {
            near: 35.0,
            mid: 80.0,
            far: 220.0,
            size_exponent: 0.5,
            hysteresis: 0.1,
            mid_update_interval: 3,
            marker_screen_fraction: 0.008,
        }
    }
}

impl LodSettings {
    /// Level for an organism of size `extent` (m) at `distance` (m), given
    /// its current level.
    ///
    /// Hysteresis is applied per boundary: a boundary the current level lies
    /// below is pushed out by `hysteresis` (the organism must move clearly
    /// past it to coarsen), one it lies above is pulled in (it must come
    /// clearly closer to refine). A jump across several bands (camera
    /// teleport or fast zoom) therefore lands on the right level at once.
    pub fn classify(&self, distance: f32, extent: f32, current: LodLevel) -> LodLevel {
        let k = extent.max(0.005).powf(self.size_exponent);
        let levels = [LodLevel::Near, LodLevel::Mid, LodLevel::Far, LodLevel::Marker];
        let h = self.hysteresis.clamp(0.0, 0.9);
        let crossed = [self.near, self.mid, self.far]
            .iter()
            .enumerate()
            .filter(|&(i, b)| {
                // Boundary i separates levels[i] (inside) from levels[i + 1].
                let shift = if (current as usize) <= i { 1.0 + h } else { 1.0 - h };
                distance >= b * k * shift
            })
            .count();
        levels[crossed]
    }
}

/// Camera-facing marker drawn at `LodLevel::Marker`.
#[derive(Component, Clone, Copy, Debug)]
pub struct LodMarker {
    /// Organism the marker stands for.
    pub owner: Entity,
    /// Adult organism extent (m); scaled by the age factor when drawn.
    pub extent: f32,
}

/// Assigns LOD levels from the distance to the first [`WorldCamera`].
/// Without a world camera every organism stays at `Near`.
pub fn update_organism_lod(
    settings: Res<LodSettings>,
    catalog: Res<OrganismCatalog>,
    frame: Res<FrameCount>,
    cameras: Query<&GlobalTransform, With<WorldCamera>>,
    mut organisms: Query<(Entity, &GlobalTransform, &AnimatedOrganism, &mut OrganismLod)>,
) {
    let camera = cameras.iter().next().map(GlobalTransform::translation);
    let interval = settings.mid_update_interval.max(1);
    for (entity, gt, org, mut lod) in &mut organisms {
        let distance = camera.map_or(0.0, |c| c.distance(gt.translation()));
        let level = match lod.pinned {
            Some(l) => l,
            None if camera.is_none() => LodLevel::Near,
            None => settings.classify(distance, catalog.extent(&org.species) * org.effective_age_scale(), lod.level),
        };
        // Stagger mid-LOD updates across frames by entity index.
        let update = level != LodLevel::Mid || (frame.0.wrapping_add(entity.index_u32())) % interval == 0;
        let new = OrganismLod { level, distance, update_this_frame: update, pinned: lod.pinned };
        if *lod != new {
            *lod = new;
        }
    }
}

/// Turns markers towards the camera and keeps their screen size roughly
/// constant (never smaller than the organism itself).
pub fn orient_markers(
    settings: Res<LodSettings>,
    cameras: Query<&GlobalTransform, With<WorldCamera>>,
    owners: Query<(&GlobalTransform, &AnimatedOrganism)>,
    mut markers: Query<(&LodMarker, &mut Transform)>,
) {
    let Some(camera) = cameras.iter().next() else { return };
    let cam = camera.translation();
    for (marker, mut transform) in &mut markers {
        let Ok((owner, org)) = owners.get(marker.owner) else { continue };
        let (_, owner_rot, owner_pos) = owner.to_scale_rotation_translation();
        let age = org.effective_age_scale();
        // World-space size: the organism's own size, or a minimum screen size.
        let extent = marker.extent * age;
        let size = extent.max(cam.distance(owner_pos) * settings.marker_screen_fraction);
        let lift = Vec3::Y * 0.5 * extent;
        let world = Transform::from_translation(owner_pos + lift).looking_at(cam, Vec3::Y).rotation;
        transform.rotation = owner_rot.inverse() * world;
        transform.translation = owner_rot.inverse() * lift / age;
        transform.scale = Vec3::splat(size / age);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_bands_scale_with_size() {
        let s = LodSettings::default();
        assert_eq!(s.classify(10.0, 1.0, LodLevel::Near), LodLevel::Near);
        assert_eq!(s.classify(50.0, 1.0, LodLevel::Near), LodLevel::Mid);
        assert_eq!(s.classify(150.0, 1.0, LodLevel::Mid), LodLevel::Far);
        assert_eq!(s.classify(400.0, 1.0, LodLevel::Far), LodLevel::Marker);
        // An elephant (4 m) is still near at 50 m; a bee is a marker at 30 m.
        assert_eq!(s.classify(50.0, 4.0, LodLevel::Near), LodLevel::Near);
        assert_eq!(s.classify(30.0, 0.015, LodLevel::Near), LodLevel::Marker);
    }

    #[test]
    fn hysteresis_prevents_flicker() {
        let s = LodSettings::default();
        // Just past the near/mid boundary: stay near.
        assert_eq!(s.classify(s.near * 1.05, 1.0, LodLevel::Near), LodLevel::Near);
        // Clearly past: switch.
        assert_eq!(s.classify(s.near * 1.2, 1.0, LodLevel::Near), LodLevel::Mid);
        // Coming back just inside: stay mid.
        assert_eq!(s.classify(s.near * 0.95, 1.0, LodLevel::Mid), LodLevel::Mid);
        assert_eq!(s.classify(s.near * 0.85, 1.0, LodLevel::Mid), LodLevel::Near);
    }

    #[test]
    fn jumps_across_several_bands_land_on_the_right_level() {
        let s = LodSettings::default();
        // A camera that teleports from close up to just past the far
        // boundary must not leave the organism at full detail.
        assert_eq!(s.classify(s.far * 1.05, 1.0, LodLevel::Near), LodLevel::Far);
        assert_eq!(s.classify(s.far * 1.2, 1.0, LodLevel::Near), LodLevel::Marker);
        // Zooming back in from a marker straight to close range.
        assert_eq!(s.classify(s.near * 0.95, 1.0, LodLevel::Marker), LodLevel::Mid);
        assert_eq!(s.classify(s.near * 0.5, 1.0, LodLevel::Marker), LodLevel::Near);
        // Classification is monotone in distance for every current level.
        for current in [LodLevel::Near, LodLevel::Mid, LodLevel::Far, LodLevel::Marker] {
            let mut prev = LodLevel::Near;
            for i in 0..600 {
                let level = s.classify(i as f32, 1.0, current);
                assert!(level >= prev, "{current:?} at {i} m");
                prev = level;
            }
        }
    }
}
