// Emergence Lab water surface (Bevy 0.20 WESL, 3D Material, alpha blended).
//
// Expects UV.x = water depth in world units (see rendering::terrain::water_mesh).
// Waves are normal-only (no vertex displacement): four directional sine trains
// with analytic slopes, advancing with the deep-water dispersion relation
// omega = sqrt(g k). Embedded into the `rendering` crate at compile time.

import bevy_pbr::render::{
    forward_io::VertexOutput,
    mesh_view_bindings::{globals, view},
};

struct WaterSettings {
    shallow_color: vec4<f32>,
    deep_color: vec4<f32>,
    sky_color: vec4<f32>,
    sun_color: vec4<f32>,
    sun_direction: vec4<f32>,
    waves: vec4<f32>,
    shading: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> water: WaterSettings;

const GRAVITY: f32 = 9.81;
const TAU: f32 = 6.28318530718;

// Surface normal from the summed slopes of four wave trains.
fn wave_normal(p: vec2<f32>, t: f32) -> vec3<f32> {
    let amplitude = water.waves.x;
    let base_k = TAU / max(water.waves.y, 0.01);
    let speed = water.waves.z;
    var dirs = array<vec2<f32>, 4>(
        vec2<f32>(1.0, 0.2),
        vec2<f32>(-0.6, 1.0),
        vec2<f32>(0.35, -1.0),
        vec2<f32>(-1.0, -0.45),
    );
    var scales = array<f32, 4>(1.0, 1.73, 2.91, 4.37);
    var slope = vec2<f32>(0.0);
    for (var i = 0u; i < 4u; i = i + 1u) {
        let d = normalize(dirs[i]);
        let k = base_k * scales[i];
        let omega = sqrt(GRAVITY * k) * speed;
        let phase = dot(d, p) * k - omega * t;
        // Height a_i sin(phase) with a_i = amplitude / (k_i s_i): shorter trains are gentler.
        slope += d * (amplitude * cos(phase)) / scales[i];
    }
    return normalize(vec3<f32>(-slope.x, 1.0, -slope.y));
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let depth = in.uv.x;
    let n = wave_normal(in.world_position.xz, globals.time);
    let v = normalize(view.world_position - in.world_position.xyz);
    let l = normalize(water.sun_direction.xyz);

    let deep_t = clamp(depth / max(water.waves.w, 1e-3), 0.0, 1.0);
    let body = mix(water.shallow_color.rgb, water.deep_color.rgb, sqrt(deep_t));
    let lambert = 0.45 + 0.55 * max(dot(n, l), 0.0);

    let cos_v = clamp(dot(n, v), 0.0, 1.0);
    let fresnel = 0.02 + 0.98 * pow(1.0 - cos_v, 5.0);
    let h = normalize(l + v);
    let spec = pow(max(dot(n, h), 0.0), water.shading.z) * water.sun_color.w;

    let color = mix(body * lambert, water.sky_color.rgb, fresnel * water.shading.w) + water.sun_color.rgb * spec;
    let opacity = mix(water.shading.x, water.shading.y, deep_t) + fresnel * 0.25 + spec;
    let shore = smoothstep(0.0, 0.12, depth);
    return vec4<f32>(color, clamp(opacity, 0.0, 1.0) * shore);
}
