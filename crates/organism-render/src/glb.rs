//! Authored glTF organisms: loading, one shared `AnimationGraph` per model
//! built from the clips named in the manifest, scene-ready setup (animation
//! player, material fix-up, joint discovery), clip switching with
//! `AnimationTransitions` crossfades, stride-matched playback rates,
//! reduced-rate updates at mid LOD, and procedural overlays for actions the
//! model has no clip for.

use std::collections::BTreeMap;
use std::time::Duration;

use bevy::animation::graph::AnimationNodeIndex;
use bevy::gltf::{Gltf, GltfAssetLabel};
use bevy::platform::collections::HashSet;
use bevy::prelude::*;
use bevy::world_serialization::{WorldAsset, WorldAssetRoot, WorldInstanceReady};

use crate::action::{OrganismAction, speed_for_froude, stride_frequency};
use crate::catalog::OrganismCatalog;
use crate::glb_overlay::{
    GlbRig, JointRef, LegJoints, OverlayDims, bone_overlay, forward_axis, leg_joint_name, root_overlay,
};
use crate::library::{ActionPlan, LocomotionScale, MotionOverlay, locomotion_scale, plan_action, playback_rate};
use crate::lod::{LodLevel, OrganismLod};
use crate::manifest::ModelEntry;
use crate::organism::AnimatedOrganism;
use crate::procedural::pose::CROSSFADE_SECONDS;
use crate::procedural::rig::Side;

/// Animation graph shared by every instance of one model.
#[derive(Clone, Debug)]
pub struct GlbGraph {
    /// Graph asset.
    pub handle: Handle<AnimationGraph>,
    /// Clip name → graph node.
    pub nodes: BTreeMap<String, AnimationNodeIndex>,
    /// Clips named in the manifest but absent from the file.
    pub missing: Vec<String>,
}

/// Loading state of one model.
#[derive(Clone, Debug)]
pub enum GlbStatus {
    /// Still loading.
    Loading,
    /// Loaded; graph built.
    Ready(GlbGraph),
    /// Could not be loaded.
    Failed(String),
}

/// Handles and status of one model.
#[derive(Clone, Debug)]
pub struct GlbModelState {
    /// Whole-file handle (for named clips).
    pub gltf: Handle<Gltf>,
    /// First scene.
    pub scene: Handle<WorldAsset>,
    /// Status.
    pub status: GlbStatus,
}

/// All requested GLB models.
#[derive(Resource, Default)]
pub struct GlbModelAssets {
    models: BTreeMap<String, GlbModelState>,
    empty_graph: Option<Handle<AnimationGraph>>,
    fixed_materials: HashSet<AssetId<StandardMaterial>>,
}

impl GlbModelAssets {
    /// Starts loading `model` (idempotent) and returns its state.
    pub fn request(&mut self, model: &ModelEntry, server: &AssetServer) -> &GlbModelState {
        self.models.entry(model.id.clone()).or_insert_with(|| GlbModelState {
            gltf: server.load(model.model_path.clone()),
            scene: server.load(GltfAssetLabel::Scene(0).from_asset(model.model_path.clone())),
            status: GlbStatus::Loading,
        })
    }

    /// State of a requested model.
    pub fn state(&self, id: &str) -> Option<&GlbModelState> {
        self.models.get(id)
    }

    /// Models that are still loading.
    pub fn pending(&self) -> usize {
        self.models.values().filter(|m| matches!(m.status, GlbStatus::Loading)).count()
    }

    /// Ids and messages of models that failed to load.
    pub fn failures(&self) -> Vec<(String, String)> {
        self.models
            .iter()
            .filter_map(
                |(id, m)| if let GlbStatus::Failed(e) = &m.status { Some((id.clone(), e.clone())) } else { None },
            )
            .collect()
    }
}

/// Root-motion and clip state of one GLB organism (on the scene root).
#[derive(Component, Clone, Debug)]
pub struct GlbInstance {
    /// Manifest model id.
    pub model: String,
    /// Organism entity.
    pub owner: Entity,
    /// Yaw and scale from the manifest.
    pub base: Transform,
    /// Entity with the `AnimationPlayer`, once found.
    pub player: Option<Entity>,
    /// Joints for bone overlays.
    pub rig: GlbRig,
    /// Current plan.
    pub plan: Option<ActionPlan>,
    /// Seconds in the current action.
    pub action_t: f32,
    /// Locomotion cycle for overlays.
    pub cycle: f32,
    /// Time accumulated since the last mid-LOD update.
    pub accumulated: f32,
    /// Whether bone overlays wrote joints (to restore their rest pose).
    pub bones_touched: bool,
    /// Setup failed (no player / model failed): stop retrying.
    pub broken: bool,
}

/// Inserted when the scene instance has been spawned.
#[derive(Component, Clone, Copy, Debug)]
pub struct GlbSceneReady;

/// Spawns a model's scene under `parent` (the organism's visual entity).
pub fn spawn_glb_scene(
    commands: &mut Commands,
    parent: Entity,
    owner: Entity,
    model: &ModelEntry,
    assets: &mut GlbModelAssets,
    server: &AssetServer,
) -> Entity {
    let scene = assets.request(model, server).scene.clone();
    let base = Transform::from_rotation(Quat::from_rotation_y(model.yaw_offset_deg.to_radians()))
        .with_scale(Vec3::splat(model.scale));
    let instance = GlbInstance {
        model: model.id.clone(),
        owner,
        base,
        player: None,
        rig: GlbRig::default(),
        plan: None,
        action_t: 0.0,
        cycle: 0.0,
        accumulated: 0.0,
        bones_touched: false,
        broken: false,
    };
    commands.spawn((WorldAssetRoot(scene), base, instance, ChildOf(parent))).id()
}

/// Observer: marks GLB organism scenes whose instance is ready.
pub fn on_world_instance_ready(
    ready: On<WorldInstanceReady>,
    instances: Query<(), With<GlbInstance>>,
    mut commands: Commands,
) {
    if instances.contains(ready.entity) {
        commands.entity(ready.entity).insert(GlbSceneReady);
    }
}

/// Builds each loaded model's animation graph from its named clips.
pub fn prepare_glb_models(
    mut assets: ResMut<GlbModelAssets>,
    catalog: Res<OrganismCatalog>,
    server: Res<AssetServer>,
    gltfs: Res<Assets<Gltf>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
) {
    if assets.empty_graph.is_none() {
        assets.empty_graph = Some(graphs.add(AnimationGraph::new()));
    }
    for (id, state) in assets.models.iter_mut() {
        if !matches!(state.status, GlbStatus::Loading) {
            continue;
        }
        if let bevy::asset::LoadState::Failed(e) = server.load_state(&state.gltf) {
            error!("organism-render: cannot load model `{id}`: {e}");
            state.status = GlbStatus::Failed(e.to_string());
            continue;
        }
        let (Some(gltf), Some(model)) = (gltfs.get(&state.gltf), catalog.library.model(id)) else { continue };
        let mut graph = AnimationGraph::new();
        let root = graph.root;
        let mut nodes = BTreeMap::new();
        let mut missing = Vec::new();
        for name in model.clip_names() {
            match gltf.named_animations.get(name) {
                Some(clip) => {
                    nodes.insert(name.to_string(), graph.add_clip(clip.clone(), 1.0, root));
                }
                None => missing.push(name.to_string()),
            }
        }
        if !missing.is_empty() {
            warn!("organism-render: model `{id}` lacks clips {missing:?}; those actions use procedural fallbacks");
        }
        state.status = GlbStatus::Ready(GlbGraph { handle: graphs.add(graph), nodes, missing });
    }
}

/// Composes the rest rotations from the scene root down to (and including)
/// `entity`.
fn model_rotation(entity: Entity, root: Entity, parents: &Query<&ChildOf>, transforms: &Query<&Transform>) -> Quat {
    let mut chain = Vec::new();
    let mut cur = entity;
    while cur != root {
        chain.push(cur);
        match parents.get(cur) {
            Ok(p) => cur = p.parent(),
            Err(_) => break,
        }
    }
    chain.iter().rev().fold(Quat::IDENTITY, |acc, e| acc * transforms.get(*e).map_or(Quat::IDENTITY, |t| t.rotation))
}

/// Once a scene is ready: finds its animation player and named joints,
/// fixes the converted materials (metallic 0, roughness ≥ 0.6, as noted in
/// `docs/research/assets-3d.md` §7.4) and attaches the shared graph.
#[allow(clippy::too_many_arguments)]
pub fn attach_glb_players(
    mut commands: Commands,
    mut assets: ResMut<GlbModelAssets>,
    catalog: Res<OrganismCatalog>,
    mut instances: Query<(Entity, &mut GlbInstance), With<GlbSceneReady>>,
    children: Query<&Children>,
    parents: Query<&ChildOf>,
    transforms: Query<&Transform>,
    names: Query<&Name>,
    players: Query<(), With<AnimationPlayer>>,
    mesh_materials: Query<&MeshMaterial3d<StandardMaterial>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (root, mut inst) in &mut instances {
        if inst.player.is_some() || inst.broken {
            continue;
        }
        let graph = match assets.state(&inst.model).map(|s| &s.status) {
            Some(GlbStatus::Ready(g)) => g.handle.clone(),
            Some(GlbStatus::Failed(_)) | None => {
                inst.broken = true;
                continue;
            }
            Some(GlbStatus::Loading) => continue,
        };
        let mut player = None;
        let mut by_name: BTreeMap<String, Entity> = BTreeMap::new();
        for d in children.iter_descendants(root) {
            if player.is_none() && players.contains(d) {
                player = Some(d);
            }
            if let Ok(name) = names.get(d) {
                by_name.entry(name.as_str().to_string()).or_insert(d);
            }
            if let Ok(mat) = mesh_materials.get(d)
                && assets.fixed_materials.insert(mat.id())
                && let Some(mut m) = materials.get_mut(&mat.0)
            {
                m.metallic = 0.0;
                m.perceptual_roughness = m.perceptual_roughness.max(0.6);
            }
        }
        let Some(player) = player else {
            warn!("organism-render: model `{}` has no animation player", inst.model);
            inst.broken = true;
            continue;
        };
        commands.entity(player).insert((AnimationGraphHandle(graph), AnimationTransitions::new()));
        inst.player = Some(player);
        let joint = |name: &str| -> Option<JointRef> {
            let e = *by_name.get(name)?;
            let parent = parents.get(e).ok()?.parent();
            Some(JointRef {
                entity: e,
                rest: transforms.get(e).ok()?.rotation,
                parent_model: model_rotation(parent, root, &parents, &transforms),
            })
        };
        let mut legs = Vec::new();
        for pair in 0..2u8 {
            for side in Side::BOTH {
                if let Some(upper) = joint(&leg_joint_name(pair, side, true)) {
                    legs.push(LegJoints { pair, side, upper, lower: joint(&leg_joint_name(pair, side, false)) });
                }
            }
        }
        let forward = catalog.library.model(&inst.model).map_or(Vec3::Z, |m| forward_axis(&m.forward));
        inst.rig = GlbRig {
            legs,
            neck: joint("Neck"),
            head: joint("Head"),
            lateral: Vec3::Y.cross(forward).normalize_or(Vec3::X),
        };
    }
}

/// Speed used for overlay cycles when the simulation gives none.
fn overlay_speed(org: &AnimatedOrganism, scale: LocomotionScale) -> f32 {
    if org.effective_speed() > 0.0 {
        org.effective_speed()
    } else if matches!(org.action.canonical(), OrganismAction::Run) {
        speed_for_froude(crate::action::FROUDE_DEFAULT_RUN, scale.hip_height)
    } else {
        speed_for_froude(crate::action::FROUDE_DEFAULT_WALK, scale.hip_height)
    }
}

/// Switches clips on action changes (crossfade), sets playback rates from
/// the movement speed, throttles mid-LOD evaluation and applies whole-body
/// overlays.
pub fn drive_glb_animation(
    time: Res<Time>,
    assets: Res<GlbModelAssets>,
    catalog: Res<OrganismCatalog>,
    organisms: Query<(&AnimatedOrganism, Option<&OrganismLod>, &GlobalTransform)>,
    mut instances: Query<(&mut GlbInstance, &mut Transform)>,
    mut players: Query<(&mut AnimationPlayer, &mut AnimationTransitions, &mut AnimationGraphHandle)>,
) {
    let dt = time.delta_secs();
    for (mut inst, mut transform) in &mut instances {
        let Ok((org, lod, owner_gt)) = organisms.get(inst.owner) else { continue };
        let Some(model) = catalog.library.model(&inst.model) else { continue };
        let scale = locomotion_scale(model).scaled(org.effective_age_scale());
        let changed = inst.plan.as_ref().is_none_or(|p| p.action != org.action);
        if changed {
            inst.plan = Some(plan_action(model, org.action));
            inst.action_t = 0.0;
        }
        inst.action_t += dt;
        let Some(plan) = inst.plan.clone() else { continue };
        let speed = overlay_speed(org, scale);
        if plan.has(MotionOverlay::LegGait) {
            inst.cycle += stride_frequency(speed, scale.hip_height) * dt;
        } else {
            inst.cycle += dt;
        }
        // Whole-body overlay on top of the manifest yaw/scale.
        let dims = OverlayDims::from_model(model);
        let bones = inst.rig.neck.is_some();
        let overlay = root_overlay(&plan.overlays, inst.action_t, inst.cycle, &dims, owner_gt.translation().y, bones);
        let posed = Transform {
            translation: overlay.translation,
            rotation: overlay.rotation * inst.base.rotation,
            scale: inst.base.scale,
        };
        // Avoid re-propagating the whole skeleton when nothing moved.
        if *transform != posed {
            *transform = posed;
        }

        let (Some(player_entity), Some(GlbStatus::Ready(graph))) =
            (inst.player, assets.state(&inst.model).map(|s| &s.status))
        else {
            continue;
        };
        let Ok((mut player, mut transitions, mut graph_handle)) = players.get_mut(player_entity) else { continue };
        let Some(clip) = &plan.clip else { continue };
        let Some(&node) = graph.nodes.get(&clip.name) else { continue };
        if changed || transitions.get_main_animation() != Some(node) {
            let fade = if transitions.get_main_animation().is_none() {
                Duration::ZERO
            } else {
                Duration::from_secs_f32(CROSSFADE_SECONDS)
            };
            let active = transitions.play(&mut player, node, fade);
            if clip.repeat {
                active.repeat();
            }
        }
        let rate = if plan.has(MotionOverlay::LegGait) {
            clip.multiplier
        } else {
            playback_rate(&plan, scale, org.effective_speed())
        };
        // Mid LOD: evaluate the clip only on scheduled frames, catching up
        // the skipped time; in between, an empty graph disables evaluation.
        let mid = lod.is_some_and(|l| l.level == LodLevel::Mid);
        let catch_up = if mid {
            inst.accumulated += dt;
            if lod.is_some_and(|l| l.update_this_frame) {
                let k = inst.accumulated / dt.max(1e-6);
                inst.accumulated = 0.0;
                k
            } else {
                0.0
            }
        } else {
            inst.accumulated = 0.0;
            1.0
        };
        let wanted = match (&assets.empty_graph, catch_up > 0.0) {
            (Some(empty), false) => empty.clone(),
            _ => graph.handle.clone(),
        };
        if graph_handle.0 != wanted {
            graph_handle.0 = wanted;
        }
        if let Some(a) = player.animation_mut(node) {
            a.set_speed(rate * catch_up.max(1.0));
        }
    }
}

/// Writes bone overlays (leg gait, lying down, head dip) after clip
/// evaluation and before transform propagation; restores the rest pose
/// when an overlay ends.
pub fn apply_glb_bone_overlays(
    organisms: Query<&AnimatedOrganism>,
    catalog: Res<OrganismCatalog>,
    mut instances: Query<&mut GlbInstance>,
    mut joints: Query<&mut Transform, Without<GlbInstance>>,
) {
    for mut inst in &mut instances {
        let Ok(org) = organisms.get(inst.owner) else { continue };
        let Some(model) = catalog.library.model(&inst.model) else { continue };
        let Some(plan) = &inst.plan else { continue };
        let scale = locomotion_scale(model).scaled(org.effective_age_scale());
        let dims = OverlayDims::from_model(model);
        let rotations =
            bone_overlay(&inst.rig, &plan.overlays, inst.action_t, inst.cycle, overlay_speed(org, scale), &dims);
        if rotations.is_empty() {
            if inst.bones_touched {
                let rest: Vec<JointRef> = inst
                    .rig
                    .legs
                    .iter()
                    .flat_map(|l| std::iter::once(l.upper).chain(l.lower))
                    .chain(inst.rig.neck)
                    .chain(inst.rig.head)
                    .collect();
                for j in rest {
                    if let Ok(mut t) = joints.get_mut(j.entity) {
                        t.rotation = j.rest;
                    }
                }
                inst.bones_touched = false;
            }
            continue;
        }
        for (entity, rotation) in rotations {
            if let Ok(mut t) = joints.get_mut(entity) {
                t.rotation = rotation;
            }
        }
        inst.bones_touched = true;
    }
}
