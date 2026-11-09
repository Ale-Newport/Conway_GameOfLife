//! Shared materials. Procedural organisms carry their colours in vertex
//! colours, so all of them share a handful of white materials and batch
//! (GPU-instance) automatically when they also share a mesh.

use bevy::prelude::*;

use crate::procedural::rig::PartMaterial;

/// Material handles shared by every procedural organism and marker.
#[derive(Resource, Clone, Debug)]
pub struct OrganismMaterials {
    /// Fur, skin and feathers.
    pub matte: Handle<StandardMaterial>,
    /// Shells, elytra, horn.
    pub glossy: Handle<StandardMaterial>,
    /// Insect wings and thin membranes (alpha blended, double sided).
    pub translucent: Handle<StandardMaterial>,
    /// Eyes.
    pub eye: Handle<StandardMaterial>,
    /// Distant markers (unlit, vertex coloured).
    pub marker: Handle<StandardMaterial>,
}

impl FromWorld for OrganismMaterials {
    fn from_world(world: &mut World) -> Self {
        let mut materials = world.resource_mut::<Assets<StandardMaterial>>();
        let matte = materials.add(StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.82,
            metallic: 0.0,
            reflectance: 0.35,
            ..default()
        });
        let glossy = materials.add(StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.32,
            metallic: 0.0,
            reflectance: 0.55,
            ..default()
        });
        let translucent = materials.add(StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.3,
            alpha_mode: AlphaMode::Blend,
            double_sided: true,
            cull_mode: None,
            ..default()
        });
        let eye = materials.add(StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.08,
            reflectance: 0.8,
            ..default()
        });
        let marker = materials.add(StandardMaterial {
            base_color: Color::WHITE,
            unlit: true,
            double_sided: true,
            cull_mode: None,
            ..default()
        });
        Self { matte, glossy, translucent, eye, marker }
    }
}

impl OrganismMaterials {
    /// Handle for a part finish.
    pub fn for_part(&self, material: PartMaterial) -> Handle<StandardMaterial> {
        match material {
            PartMaterial::Matte => self.matte.clone(),
            PartMaterial::Glossy => self.glossy.clone(),
            PartMaterial::Translucent => self.translucent.clone(),
            PartMaterial::Eye => self.eye.clone(),
        }
    }
}
