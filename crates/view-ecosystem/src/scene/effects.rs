//! Weather and fire effects: rain and snow from the model's weather around
//! the camera, flames, smoke and embers over burning cells (the nearest to
//! the view first) with a warm glow at night.

use bevy::prelude::*;
use bevy_panorbit_camera::PanOrbitCamera;
use physics_world::{FireState, WeatherKind, WorldLayer};
use rendering::{ParticleEmitter, ParticlePreset};

use super::EcoCamera;
use super::terrain::MapState;
use crate::MODE;
use crate::state::{EcoData, EcoUi};

/// Burning cells drawn with flames at once.
pub const MAX_FIRES: usize = 36;
/// Fire glow lights.
pub const MAX_FIRE_LIGHTS: usize = 6;

/// The rain / snow emitter.
#[derive(Component, Debug, Clone, Copy)]
pub struct WeatherEmitter;

/// A flame, smoke or ember emitter of the fire pool.
#[derive(Component, Debug, Clone, Copy)]
pub struct FireEmitter;

/// A fire glow light.
#[derive(Component, Debug, Clone, Copy)]
pub struct FireGlow;

/// What the sky drops.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Precipitation {
    /// Nothing.
    None,
    /// Rain.
    Rain,
    /// Snow.
    Snow,
}

/// Precipitation drawn for a weather state and the air temperature at the
/// view (°C).
pub fn precipitation(kind: WeatherKind, rate_mm_h: f32, temperature_c: f32) -> Precipitation {
    match kind {
        WeatherKind::Rain | WeatherKind::Storm if rate_mm_h > 0.01 => {
            if temperature_c < 0.0 {
                Precipitation::Snow
            } else {
                Precipitation::Rain
            }
        }
        WeatherKind::Snow if rate_mm_h > 0.01 => Precipitation::Snow,
        _ => Precipitation::None,
    }
}

/// Pools of effect entities.
#[derive(Resource, Default)]
pub struct EffectPools {
    fires: Vec<(Entity, Entity, Entity)>,
    lights: Vec<Entity>,
    weather: Option<Entity>,
    precipitation: Option<Precipitation>,
    epoch: Option<u64>,
    seq: u64,
    /// Entities were spawned: place them once they exist (next frame).
    unplaced: bool,
}

/// Burning cells as scene points with their intensity, nearest to `focus` first.
pub fn burning_points(map: &MapState, fire: &[FireState], focus: Vec3, limit: usize) -> Vec<(Vec3, f32)> {
    let Some(g) = map.geometry.as_ref() else { return Vec::new() };
    let mut points: Vec<(f32, Vec3, f32)> = fire
        .iter()
        .enumerate()
        .filter_map(|(i, s)| {
            let intensity = match s {
                FireState::Burning { intensity, .. } => *intensity,
                FireState::Lava { .. } => 1.5,
                _ => return None,
            };
            let (x, z) = (((i % g.width) as f32 + 0.5) * g.spacing, ((i / g.width) as f32 + 0.5) * g.spacing);
            let p = Vec3::new(x, g.ground(x, z), z);
            Some((p.distance_squared(focus), p, intensity))
        })
        .collect();
    points.sort_by(|a, b| a.0.total_cmp(&b.0));
    points.into_iter().take(limit).map(|(_, p, i)| (p, i)).collect()
}

/// Positions the rain / snow emitter and the fire pool from new frames.
#[allow(clippy::too_many_arguments)]
pub fn sync_effects(
    mut commands: Commands,
    data: Res<EcoData>,
    map: Res<MapState>,
    ui: Res<EcoUi>,
    mut pools: ResMut<EffectPools>,
    camera: Option<Single<(&GlobalTransform, &PanOrbitCamera), With<EcoCamera>>>,
    mut emitters: Query<(&mut ParticleEmitter, &mut Transform), Without<FireGlow>>,
    mut lights: Query<(&mut PointLight, &mut Transform, &mut Visibility), With<FireGlow>>,
    scene: Res<super::SceneState>,
) {
    let (Some(frame), Some(geometry), Some(camera)) = (data.get(), map.geometry.as_ref(), camera) else { return };
    let (eye, pan) = (camera.0.translation(), camera.1);
    if pools.epoch != map.epoch {
        pools.epoch = map.epoch;
        pools.seq = 0;
    }
    // Rain and snow around the eye, between it and the focus.
    let weather = &frame.view.world.weather;
    let focus = pan.target_focus;
    let temperature = map
        .geometry
        .as_ref()
        .and_then(|g| g.sample_of_cell(g.plane_to_cell(focus.x, focus.z)))
        .and_then(|i| frame.view.world.layer(WorldLayer::Temperature).and_then(|t| t.as_slice().get(i).copied()))
        .unwrap_or(10.0);
    let wanted = if ui.layers.weather {
        precipitation(weather.kind, weather.precipitation_rate_mm_h, temperature)
    } else {
        Precipitation::None
    };
    let weather_entity = *pools.weather.get_or_insert_with(|| {
        commands
            .spawn((
                Name::new("Precipitation"),
                WeatherEmitter,
                ParticleEmitter::preset(ParticlePreset::Rain, 11),
                Transform::default(),
                DespawnOnExit(MODE),
            ))
            .id()
    });
    if let Ok((mut emitter, mut transform)) = emitters.get_mut(weather_entity) {
        if pools.precipitation != Some(wanted) {
            pools.precipitation = Some(wanted);
            emitter.config = match wanted {
                Precipitation::Snow => ParticlePreset::Snow.config(),
                _ => ParticlePreset::Rain.config(),
            };
        }
        let strength = (weather.precipitation_rate_mm_h / 2.0).clamp(0.3, 2.5);
        let base = match wanted {
            Precipitation::Snow => ParticlePreset::Snow.config().rate,
            _ => ParticlePreset::Rain.config().rate,
        };
        let rate = base * strength;
        if (emitter.config.rate - rate).abs() > 1.0 {
            emitter.config.rate = rate;
        }
        let on = wanted != Precipitation::None;
        if emitter.emitting != on {
            emitter.emitting = on;
        }
        let ahead = (focus - eye).normalize_or_zero() * (focus - eye).length().min(30.0);
        let centre = eye + ahead + Vec3::Y * 14.0;
        if transform.translation.distance(centre) > 0.5 {
            transform.translation = centre;
        }
    }
    if pools.seq == data.seq && !pools.unplaced {
        return;
    }
    pools.seq = data.seq;
    pools.unplaced = false;
    // Fires nearest to the focus.
    let fire = frame.view.world.fire.as_ref().map(|g| g.as_slice()).unwrap_or(&[]);
    let points = if ui.layers.weather { burning_points(&map, fire, focus, MAX_FIRES) } else { Vec::new() };
    while pools.fires.len() < points.len() {
        let k = pools.fires.len() as u64;
        let spawn = |commands: &mut Commands, preset: ParticlePreset, seed: u64| {
            commands
                .spawn((
                    Name::new(preset.label()),
                    FireEmitter,
                    ParticleEmitter::preset(preset, seed),
                    Transform::default(),
                    DespawnOnExit(MODE),
                ))
                .id()
        };
        let entry = (
            spawn(&mut commands, ParticlePreset::Fire, 100 + k),
            spawn(&mut commands, ParticlePreset::Smoke, 200 + k),
            spawn(&mut commands, ParticlePreset::Embers, 300 + k),
        );
        pools.fires.push(entry);
        pools.unplaced = true;
    }
    let cell = geometry.spacing;
    for (slot, &(flame, smoke, embers)) in pools.fires.iter().enumerate() {
        let target = points.get(slot).copied();
        for (entity, preset, extra) in [
            (flame, ParticlePreset::Fire, 0.0),
            (smoke, ParticlePreset::Smoke, 3.0),
            (embers, ParticlePreset::Embers, 1.0),
        ] {
            let Ok((mut emitter, mut transform)) = emitters.get_mut(entity) else { continue };
            match target {
                Some((p, intensity)) => {
                    // Flames fill a good part of the cell; stronger fires burn taller.
                    let factor = cell / 20.0 * (6.0 + 10.0 * intensity.min(1.5));
                    let config = preset.config().scaled(factor);
                    if emitter.config.shape != config.shape {
                        emitter.config = config;
                    }
                    emitter.emitting = true;
                    transform.translation = p + Vec3::Y * extra * factor * 0.2;
                }
                None => emitter.emitting = false,
            }
        }
    }
    // Warm light over the largest nearby fires (strongest at night).
    while pools.lights.len() < MAX_FIRE_LIGHTS.min(points.len()) {
        let e = commands
            .spawn((
                Name::new("Fire glow"),
                FireGlow,
                PointLight { color: Color::srgb(1.0, 0.55, 0.2), shadow_maps_enabled: false, ..default() },
                Transform::default(),
                Visibility::Hidden,
                DespawnOnExit(MODE),
            ))
            .id();
        pools.lights.push(e);
        pools.unplaced = true;
    }
    let night = 1.0 - scene.daylight.min(0.5) / 0.5;
    for (slot, &e) in pools.lights.iter().enumerate() {
        let Ok((mut light, mut transform, mut vis)) = lights.get_mut(e) else { continue };
        match points.get(slot * points.len().max(1) / MAX_FIRE_LIGHTS.max(1)) {
            Some((p, intensity)) if slot < points.len() => {
                light.intensity = (1.5e6 + 6.0e6 * night) * intensity.min(1.5) * (cell / 20.0).powi(2);
                light.range = cell * 5.0;
                transform.translation = *p + Vec3::Y * cell * 0.6;
                vis.set_if_neq(Visibility::Inherited);
            }
            _ => {
                vis.set_if_neq(Visibility::Hidden);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coords::MapGeometry;

    #[test]
    fn precipitation_follows_weather_and_temperature() {
        assert_eq!(precipitation(WeatherKind::Rain, 2.0, 12.0), Precipitation::Rain);
        assert_eq!(precipitation(WeatherKind::Storm, 6.0, -3.0), Precipitation::Snow);
        assert_eq!(precipitation(WeatherKind::Snow, 1.0, -8.0), Precipitation::Snow);
        assert_eq!(precipitation(WeatherKind::Clear, 0.0, 12.0), Precipitation::None);
        assert_eq!(precipitation(WeatherKind::Rain, 0.0, 12.0), Precipitation::None);
    }

    #[test]
    fn nearest_fires_first() {
        let mut map = MapState::default();
        map.geometry = Some(MapGeometry::new(3, 1, 1, 10.0, vec![0.0; 3]));
        let fire = [
            FireState::Burning { intensity: 1.0, steps: 1 },
            FireState::Fuel,
            FireState::Burning { intensity: 0.5, steps: 1 },
        ];
        let pts = burning_points(&map, &fire, Vec3::new(25.0, 0.0, 5.0), 10);
        assert_eq!(pts.len(), 2);
        assert_eq!(pts[0].0.x, 25.0, "the cell next to the focus first");
        assert_eq!(burning_points(&map, &fire, Vec3::ZERO, 1).len(), 1);
    }
}
