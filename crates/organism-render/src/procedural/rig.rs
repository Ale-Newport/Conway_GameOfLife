//! Articulated rigs: a tree of rigid parts (body, neck, head, jaw, leg
//! segments, tail segments, wings, fins...) with rest transforms and meshes,
//! plus the dimensions the procedural animator needs.
//!
//! Conventions (shared with glTF models): the organism faces local **+Z**,
//! **+Y** is up, **+X** is the animal's left side; the ground is `y = 0`.
//! Legs hang along their joint's local −Y; necks, heads and tails extend
//! along their joint's local +Z / −Z.

use bevy::prelude::Transform;

use super::mesh::{MeshData, Rgba};

/// Body side.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Side {
    /// Left (+X).
    Left,
    /// Right (−X).
    Right,
}

impl Side {
    /// +1 for left, −1 for right.
    pub fn sign(self) -> f32 {
        match self {
            Self::Left => 1.0,
            Self::Right => -1.0,
        }
    }

    /// Both sides, left first.
    pub const BOTH: [Side; 2] = [Side::Left, Side::Right];
}

/// Segment of a leg.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LegSegment {
    /// Thigh / upper arm (hip or shoulder joint); for arthropods the coxa+femur.
    Upper,
    /// Shank / forearm (knee or elbow joint); for arthropods the tibia+tarsus.
    Lower,
}

/// Segment of a wing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WingSegment {
    /// Arm wing (shoulder joint) or whole insect wing.
    Inner,
    /// Hand wing (wrist joint).
    Outer,
}

/// Animation role of a rig part.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PartRole {
    /// Body pivot; carries body bob, pitch, roll and lowering.
    Root,
    /// Neck joint `i` (0 = at the body).
    Neck(u8),
    /// Head.
    Head,
    /// Lower jaw / mandible / lower beak.
    Jaw,
    /// Ear.
    Ear(Side),
    /// Antenna.
    Antenna(Side),
    /// Leg segment; `pair` counts from the front (0 = fore legs).
    Leg {
        /// Leg pair from the front.
        pair: u8,
        /// Body side.
        side: Side,
        /// Segment.
        segment: LegSegment,
    },
    /// Tail joint `i` (0 = at the body).
    Tail(u8),
    /// Trunk joint `i` (0 = at the head).
    Trunk(u8),
    /// Wing segment; `pair` 0 = fore wings.
    Wing {
        /// Wing pair.
        pair: u8,
        /// Body side.
        side: Side,
        /// Segment.
        segment: WingSegment,
    },
    /// Body segment `i` behind the root for undulating bodies (fish, snakes,
    /// worms) and insect abdomens.
    Segment(u8),
    /// Paired fin or flipper (pectoral fins, turtle flippers).
    Fin(Side),
    /// Fixed decoration (eyes, horns, antlers, mane, shell...).
    Static,
}

/// Surface finish of a part, mapped to a shared material.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PartMaterial {
    /// Fur, skin, feathers.
    Matte,
    /// Shells, beetle elytra, wet skin, horn.
    Glossy,
    /// Insect wings and fins (alpha blended, double sided).
    Translucent,
    /// Eyes (dark, very glossy).
    Eye,
}

/// One rigid part.
#[derive(Clone, Debug, PartialEq)]
pub struct RigPart {
    /// Animation role.
    pub role: PartRole,
    /// Index of the parent part (`None` for the root).
    pub parent: Option<usize>,
    /// Rest transform relative to the parent.
    pub rest: Transform,
    /// Geometry in the part's local frame (may be empty for pure joints).
    pub mesh: MeshData,
    /// Surface finish.
    pub material: PartMaterial,
}

/// Locomotor body plan, selecting the procedural motion repertoire.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Locomotor {
    /// Four legs (mammals, lizards, crocodiles).
    Quadruped,
    /// Two legs plus wings (birds).
    Bird,
    /// Bat: membrane wings, crawling on the ground.
    Bat,
    /// Six legs (insects).
    Hexapod,
    /// Eight legs (spiders).
    Octopod,
    /// Body and caudal-fin undulation (fish, cetaceans, rays).
    Fish,
    /// Lateral undulation without limbs (snakes, eels).
    Serpentine,
    /// Peristaltic crawling (earthworms).
    Peristaltic,
    /// Saltatory hopping with synchronous hind legs (frogs, toads).
    Anuran,
    /// Slow walking with a rigid shell; flipper strokes in water.
    Chelonian,
    /// Gliding on a muscular foot (snails).
    Gastropod,
}

/// Asymmetric fast gait of a quadruped.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GallopStyle {
    /// Transverse gallop (horses, ungulates).
    Transverse,
    /// Rotary gallop (dogs, cats, cheetahs).
    Rotary,
    /// Bound / half-bound (rabbits, squirrels, mustelids).
    Bound,
    /// Fast lateral-sequence amble without suspension (elephants, tortoises).
    Amble,
}

/// Dimensions and capabilities used by the procedural animator (adult
/// metres; instances scale them by their age factor).
#[derive(Clone, Debug, PartialEq)]
pub struct RigMotion {
    /// Body plan.
    pub locomotor: Locomotor,
    /// Fast-gait style for quadrupeds.
    pub gallop: GallopStyle,
    /// Adult body mass (kg): breathing rate and wing loading.
    pub mass_kg: f32,
    /// Ground-to-hip height (m).
    pub hip_height: f32,
    /// Body length (m) for undulation.
    pub body_length: f32,
    /// How far (m) the root descends when the animal lies down so that the
    /// belly rests on the ground.
    pub rest_drop: f32,
    /// Leg segment lengths per leg pair: (upper, lower).
    pub leg_lengths: Vec<(f32, f32)>,
    /// Sprawling legs that swing about a vertical axis (arthropods): per
    /// pair, the lateral distance (m) of the foot from its leg's body joint
    /// in the rest pose, i.e. the lever arm that turns a yaw swing into
    /// fore-aft foot travel. Empty for upright legs, which swing about a
    /// horizontal axis with the full leg length as lever arm.
    pub leg_reach: Vec<f32>,
    /// Half the body width (m): resting height of a body lying on its side.
    pub half_width: f32,
    /// Wing half-span (m), 0 without wings.
    pub half_span: f32,
    /// Wing-beat frequency (Hz) for flyers, 0 otherwise.
    pub flap_hz: f32,
    /// Wing elevation amplitude (rad) during flapping.
    pub flap_amplitude: f32,
    /// Whether the species can fly.
    pub can_fly: bool,
    /// Whether the species lives in water (fish, cetaceans).
    pub aquatic: bool,
    /// Neck pitch (rad, downwards) that brings the mouth to the ground.
    pub graze_pitch: f32,
    /// Number of undulating body segments.
    pub segments: u8,
    /// Undulation wavelength in body lengths.
    pub wave_length: f32,
    /// Relative amplitude of the undulation at the tail.
    pub wave_amplitude: f32,
    /// Whether the body undulates up and down (cetaceans) instead of sideways.
    pub vertical_undulation: bool,
}

impl RigMotion {
    /// Hip height of a leg pair (upper + lower segment).
    pub fn leg_length(&self, pair: u8) -> f32 {
        self.leg_lengths.get(pair as usize).map_or(self.hip_height, |(a, b)| a + b)
    }

    /// Lever arm (m) converting a leg pair's swing angle into fore-aft foot
    /// travel: the recorded lateral reach for sprawling legs, else the
    /// leg length.
    pub fn swing_lever(&self, pair: u8) -> f32 {
        self.leg_reach.get(pair as usize).copied().unwrap_or_else(|| self.leg_length(pair))
    }
}

/// A complete articulated organism.
#[derive(Clone, Debug, PartialEq)]
pub struct Rig {
    /// Parts in parent-before-child order (index 0 is the root).
    pub parts: Vec<RigPart>,
    /// Animation dimensions.
    pub motion: RigMotion,
    /// Representative colour for distant markers.
    pub marker_color: Rgba,
}

impl Rig {
    /// Starts a rig with a root part at `root_rest`.
    pub fn new(root_rest: Transform, motion: RigMotion, marker_color: Rgba) -> Self {
        Self {
            parts: vec![RigPart {
                role: PartRole::Root,
                parent: None,
                rest: root_rest,
                mesh: MeshData::new(),
                material: PartMaterial::Matte,
            }],
            motion,
            marker_color,
        }
    }

    /// Adds a part and returns its index.
    pub fn add(
        &mut self,
        parent: usize,
        role: PartRole,
        rest: Transform,
        mesh: MeshData,
        material: PartMaterial,
    ) -> usize {
        self.parts.push(RigPart { role, parent: Some(parent), rest, mesh, material });
        self.parts.len() - 1
    }

    /// Index of the first part with `role`.
    pub fn find(&self, role: PartRole) -> Option<usize> {
        self.parts.iter().position(|p| p.role == role)
    }

    /// Rest transform of part `i` in rig space (composition up to the root,
    /// including the root's own rest transform).
    pub fn rest_model_transform(&self, i: usize) -> Transform {
        let mut chain = vec![i];
        let mut cur = i;
        while let Some(p) = self.parts[cur].parent {
            chain.push(p);
            cur = p;
        }
        chain.iter().rev().fold(Transform::IDENTITY, |acc, &k| acc * self.parts[k].rest)
    }

    /// All part meshes in their rest pose merged into one mesh per material
    /// (rig space), for static far-LOD rendering in a single draw per
    /// material.
    pub fn merged_by_material(&self) -> Vec<(PartMaterial, MeshData)> {
        let mut out: Vec<(PartMaterial, MeshData)> = Vec::new();
        for (i, part) in self.parts.iter().enumerate() {
            if part.mesh.is_empty() {
                continue;
            }
            let t = self.rest_model_transform(i);
            match out.iter_mut().find(|(m, _)| *m == part.material) {
                Some((_, mesh)) => mesh.append_transformed(&part.mesh, &t),
                None => {
                    let mut mesh = MeshData::new();
                    mesh.append_transformed(&part.mesh, &t);
                    out.push((part.material, mesh));
                }
            }
        }
        out.sort_by_key(|(m, _)| *m);
        out
    }

    /// Total triangle count.
    pub fn triangle_count(&self) -> usize {
        self.parts.iter().map(|p| p.mesh.triangle_count()).sum()
    }

    /// Rig-space bounds of all part meshes in the rest pose.
    pub fn rest_bounds(&self) -> Option<(bevy::math::Vec3, bevy::math::Vec3)> {
        let mut acc: Option<(bevy::math::Vec3, bevy::math::Vec3)> = None;
        for (i, part) in self.parts.iter().enumerate() {
            if part.mesh.is_empty() {
                continue;
            }
            let t = self.rest_model_transform(i);
            for p in &part.mesh.positions {
                let w = t.transform_point(bevy::math::Vec3::from(*p));
                acc = Some(match acc {
                    None => (w, w),
                    Some((lo, hi)) => (lo.min(w), hi.max(w)),
                });
            }
        }
        acc
    }

    /// Moves the root vertically so the lowest foot (or, for limbless
    /// bodies, the lowest point) touches `y = 0` in the rest pose, then lifts
    /// it further if any other part would sink below the ground.
    pub fn ground(&mut self) {
        let feet: Vec<usize> = self
            .parts
            .iter()
            .enumerate()
            .filter(|(_, p)| matches!(p.role, PartRole::Leg { segment: LegSegment::Lower, .. }) && !p.mesh.is_empty())
            .map(|(i, _)| i)
            .collect();
        let candidates: Vec<usize> = if feet.is_empty() { (0..self.parts.len()).collect() } else { feet };
        let mut lowest = f32::MAX;
        for i in candidates {
            let t = self.rest_model_transform(i);
            for p in &self.parts[i].mesh.positions {
                lowest = lowest.min(t.transform_point(bevy::math::Vec3::from(*p)).y);
            }
        }
        if lowest < f32::MAX {
            self.parts[0].rest.translation.y -= lowest;
        }
        // Short-legged bodies (ladybirds, tortoises) rest on their belly:
        // nothing may sink below the ground.
        let mut overall = f32::MAX;
        for i in 0..self.parts.len() {
            let t = self.rest_model_transform(i);
            for p in &self.parts[i].mesh.positions {
                overall = overall.min(t.transform_point(bevy::math::Vec3::from(*p)).y);
            }
        }
        if overall < 0.0 {
            self.parts[0].rest.translation.y -= overall;
        }
    }

    /// Checks structural invariants: one root, parents precede children.
    pub fn validate(&self) -> Result<(), String> {
        if self.parts.first().map(|p| p.role) != Some(PartRole::Root) {
            return Err("first part must be the root".into());
        }
        for (i, p) in self.parts.iter().enumerate().skip(1) {
            match p.parent {
                Some(parent) if parent < i => {}
                _ => return Err(format!("part {i} ({:?}) has an invalid parent", p.role)),
            }
            if !p.rest.translation.is_finite() || !p.rest.rotation.is_finite() {
                return Err(format!("part {i} has a non-finite rest transform"));
            }
        }
        Ok(())
    }
}
