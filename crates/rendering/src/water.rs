//! Animated water material (WESL, `assets/shaders/water.wesl`).
//!
//! Use it on meshes from [`crate::terrain::water_mesh`] (UV.x = water depth in
//! world units). Shading model, all evaluated in the fragment shader:
//!
//! * waves: four directional sine trains whose analytic slopes perturb the
//!   normal; each train moves with the deep-water dispersion relation
//!   `ω = √(g k)` (scaled by `speed`). Geometry is not displaced
//!   (documented approximation: no vertex waves, so silhouettes stay flat);
//! * body colour: shallow → deep colour by depth, opacity rising with depth
//!   and fading to zero at the shoreline;
//! * Schlick Fresnel (F0 = 0.02) blending to a sky colour, plus Blinn–Phong
//!   sun specular from a uniform sun direction.
//!
//! The material computes its own lighting (it does not read Bevy's light
//! list, shadows or fog); keep `sun_direction` in sync with the scene's
//! directional light, e.g. with [`WaterSettings::with_sun_from`].

use bevy::asset::io::embedded::EmbeddedAssetRegistry;
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;
use std::path::{Path, PathBuf};

/// Asset path of the embedded water shader.
pub const WATER_SHADER_PATH: &str = "embedded://rendering/shaders/water.wesl";

/// Registers [`WaterMaterial`].
pub struct WaterPlugin;

impl Plugin for WaterPlugin {
    fn build(&self, app: &mut App) {
        if let Some(registry) = app.world().get_resource::<EmbeddedAssetRegistry>() {
            registry.insert_asset(
                PathBuf::new(),
                Path::new("rendering/shaders/water.wesl"),
                include_bytes!("../../../assets/shaders/water.wesl").as_slice(),
            );
        } else {
            error!("WaterPlugin needs AssetPlugin (DefaultPlugins) to be added first; the water shader is unavailable");
        }
        app.add_plugins(MaterialPlugin::<WaterMaterial>::default());
    }
}

/// Uniform parameters of the water shader (layout mirrors `water.wesl`).
#[derive(ShaderType, Clone, Copy, Debug, PartialEq)]
pub struct WaterSettings {
    /// Colour of shallow water (linear RGB).
    pub shallow_color: Vec4,
    /// Colour of deep water (linear RGB).
    pub deep_color: Vec4,
    /// Reflected sky colour (linear RGB).
    pub sky_color: Vec4,
    /// Sun colour (linear RGB) and specular intensity (w).
    pub sun_color: Vec4,
    /// Unit vector pointing towards the sun (xyz).
    pub sun_direction: Vec4,
    /// x: wave slope amplitude, y: longest wavelength (world units),
    /// z: dispersion speed factor, w: depth (world units) of fully deep colour.
    pub waves: Vec4,
    /// x: opacity in the shallows, y: opacity when deep, z: specular exponent,
    /// w: Fresnel reflection strength.
    pub shading: Vec4,
}

impl Default for WaterSettings {
    fn default() -> Self {
        Self {
            shallow_color: Color::srgb(0.10, 0.56, 0.60).to_linear().to_vec4(),
            deep_color: Color::srgb(0.02, 0.13, 0.30).to_linear().to_vec4(),
            sky_color: Color::srgb(0.62, 0.76, 0.92).to_linear().to_vec4(),
            sun_color: Vec4::new(1.0, 0.95, 0.85, 1.6),
            sun_direction: Vec3::new(0.4, 0.8, 0.3).normalize().extend(0.0),
            waves: Vec4::new(0.08, 6.0, 1.0, 4.0),
            shading: Vec4::new(0.55, 0.94, 180.0, 0.9),
        }
    }
}

impl WaterSettings {
    /// Points `sun_direction` towards a directional light with the given
    /// transform (lights shine along their local −Z).
    pub fn with_sun_from(mut self, light_transform: &Transform) -> Self {
        self.sun_direction = light_transform.back().as_vec3().extend(0.0);
        self
    }
}

/// Alpha-blended animated water surface.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone, Default)]
pub struct WaterMaterial {
    /// Shader parameters.
    #[uniform(0)]
    pub settings: WaterSettings,
}

impl Material for WaterMaterial {
    fn fragment_shader() -> ShaderRef {
        WATER_SHADER_PATH.into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }

    /// A translucent surface must not shadow the bed it shows.
    fn enable_shadows() -> bool {
        false
    }

    /// No depth prepass: the surface is blended over the terrain.
    fn enable_prepass() -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sun_direction_points_against_light_travel() {
        let light = Transform::from_xyz(10.0, 20.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y);
        let s = WaterSettings::default().with_sun_from(&light);
        let towards = Vec3::new(10.0, 20.0, 5.0).normalize();
        assert!((s.sun_direction.truncate() - towards).length() < 1e-5);
    }

    #[test]
    fn defaults_are_physically_sensible() {
        let s = WaterSettings::default();
        assert!((s.sun_direction.truncate().length() - 1.0).abs() < 1e-5);
        assert!(s.shading.x < s.shading.y, "deep water is more opaque");
        assert!(s.deep_color.x < s.shallow_color.x);
    }
}
