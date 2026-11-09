// Emergence Lab grid renderer (Bevy 0.20 WESL, Material2d).
//
// Reads an RGBA8 data texture (R = state or encoded field value, G = age,
// B = activity / trail, A = density / auxiliary) with exact texel loads and
// colours it according to `params.style`. Embedded into the `rendering` crate
// at compile time (see crates/rendering/src/grid/mod.rs).

import bevy_sprite_render::mesh2d::vertex_output::VertexOutput;

struct GridParams {
    background: vec4<f32>,
    foreground: vec4<f32>,
    grid_color: vec4<f32>,
    style: u32,
    palette_mode: u32,
    palette_len: u32,
    ramp_len: u32,
    cells_per_texel: u32,
    flags: u32,
    major_every: u32,
    origin_mod_x: u32,
    origin_mod_y: u32,
    grid_min_px: f32,
    glow: f32,
}

struct ColorTable {
    colors: array<vec4<f32>, 32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> params: GridParams;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var<uniform> palette: ColorTable;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var<uniform> ramp: ColorTable;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var cells: texture_2d<f32>;

const STYLE_MONOCHROME: u32 = 0u;
const STYLE_PIXEL_ART: u32 = 1u;
const STYLE_NEON: u32 = 2u;
const STYLE_MINIMAL: u32 = 3u;
const STYLE_CIRCLES: u32 = 4u;
const STYLE_GRADIENT: u32 = 5u;
const STYLE_HEATMAP: u32 = 6u;
const STYLE_AGE: u32 = 7u;
const STYLE_DENSITY: u32 = 8u;
const STYLE_HISTORY: u32 = 9u;

const FLAG_GRID: u32 = 1u;
const FLAG_SMOOTH: u32 = 2u;

// Area fractions used when a shape is smaller than a few pixels, so zoomed-out
// views keep the right average brightness instead of aliasing.
const CIRCLE_RADIUS: f32 = 0.44;
const CIRCLE_AREA: f32 = 0.60821; // pi * 0.44^2
const BOX_HALF: f32 = 0.38;
const BOX_RADIUS: f32 = 0.12;
const BOX_AREA: f32 = 0.56524; // 0.76^2 - (4 - pi) * 0.12^2

fn load_texel(p: vec2<i32>) -> vec4<f32> {
    let dims = vec2<i32>(textureDimensions(cells));
    return textureLoad(cells, clamp(p, vec2<i32>(0), dims - vec2<i32>(1)), 0);
}

fn palette_lerp(v: f32) -> vec3<f32> {
    let n = clamp(params.palette_len, 1u, 32u);
    let x = clamp(v, 0.0, 1.0) * f32(n - 1u);
    let i = min(u32(floor(x)), n - 1u);
    let j = min(i + 1u, n - 1u);
    return mix(palette.colors[i].rgb, palette.colors[j].rgb, x - f32(i));
}

fn ramp_lerp(v: f32) -> vec3<f32> {
    let n = clamp(params.ramp_len, 1u, 32u);
    let x = clamp(v, 0.0, 1.0) * f32(n - 1u);
    let i = min(u32(floor(x)), n - 1u);
    let j = min(i + 1u, n - 1u);
    return mix(ramp.colors[i].rgb, ramp.colors[j].rgb, x - f32(i));
}

// Colour of a state (discrete palette) or field value (continuous palette).
fn state_color(state: u32, value: f32) -> vec3<f32> {
    if params.palette_mode == 0u {
        let n = clamp(params.palette_len, 1u, 32u);
        return palette.colors[min(state, n - 1u)].rgb;
    }
    return palette_lerp(value);
}

// Bilinear interpolation of the R channel between texel centres.
fn smooth_value(t: vec2<f32>) -> f32 {
    let p = t - vec2<f32>(0.5);
    let i = vec2<i32>(floor(p));
    let f = p - floor(p);
    let a = load_texel(i).r;
    let b = load_texel(i + vec2<i32>(1, 0)).r;
    let c = load_texel(i + vec2<i32>(0, 1)).r;
    let d = load_texel(i + vec2<i32>(1, 1)).r;
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

// Anti-aliased coverage from a signed distance in pixels (positive inside).
fn coverage(signed_px: f32) -> f32 {
    return clamp(signed_px + 0.5, 0.0, 1.0);
}

fn rounded_box_sdf(p: vec2<f32>, half_size: f32, radius: f32) -> f32 {
    let q = abs(p) - vec2<f32>(half_size - radius);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - radius;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(cells));
    let t = in.uv * dims;
    // Derivatives first (uniform control flow): texels per screen pixel.
    let fw = fwidth(t);
    let px_per_texel = 1.0 / max(max(fw.x, fw.y), 1e-6);
    let cpt = f32(max(params.cells_per_texel, 1u));
    let px_per_cell = px_per_texel / cpt;

    let texel_f = clamp(floor(t), vec2<f32>(0.0), dims - vec2<f32>(1.0));
    let texel = vec2<i32>(texel_f);
    let local = clamp(t - texel_f, vec2<f32>(0.0), vec2<f32>(1.0));
    let cell_local = fract(local * cpt);
    // Shapes (circles, bevels, rounded boxes) are drawn per *texel*: an LOD
    // texel aggregates cpt x cpt cells whose individual states are unknown, so
    // drawing one shape per cell would invent structure that is not there.
    let lod = params.cells_per_texel > 1u;
    let unit_local = select(cell_local, local, lod);
    let px_per_unit = select(px_per_cell, px_per_texel, lod);
    let p = unit_local - vec2<f32>(0.5);

    let raw = load_texel(texel);
    let state = u32(round(raw.r * 255.0));
    let continuous = params.palette_mode == 1u;
    var value = raw.r;
    if continuous && (params.flags & FLAG_SMOOTH) != 0u {
        value = smooth_value(t);
    }
    let alive = select(state > 0u, value > 0.0, continuous);
    let fill = state_color(state, value);
    let bg = params.background.rgb;
    let base = select(bg, fill, alive);
    // 0 when a shape unit covers ~1 px (draw flat), 1 when it covers >= 4 px (draw shapes).
    let detail = smoothstep(1.5, 4.0, px_per_unit);

    var color = base;
    let style = params.style;
    if style == STYLE_MONOCHROME {
        color = select(bg, params.foreground.rgb, alive);
    } else if style == STYLE_PIXEL_ART {
        if alive {
            let hi = max(1.0 - smoothstep(0.07, 0.13, unit_local.x), 1.0 - smoothstep(0.07, 0.13, unit_local.y));
            let lo = max(smoothstep(0.87, 0.93, unit_local.x), smoothstep(0.87, 0.93, unit_local.y));
            color = base * (1.0 + (0.30 * hi - 0.35 * lo) * detail);
        }
    } else if style == STYLE_NEON {
        if alive {
            let core = coverage(-rounded_box_sdf(p, 0.36, 0.14) * px_per_unit);
            let r2 = dot(p, p) * 2.0;
            let halo = exp(-r2 * 3.0);
            let shaped = core * (0.75 + 0.25 * halo) + (1.0 - core) * 0.22 * halo;
            color = bg + fill * params.glow * mix(0.8, shaped, detail);
        } else {
            // Faint afterglow from the trail channel (quadratic falloff) in the first live colour.
            color = bg + state_color(1u, 1.0) * (raw.b * raw.b) * 0.12 * params.glow;
        }
    } else if style == STYLE_MINIMAL {
        let shape = coverage(-rounded_box_sdf(p, BOX_HALF, BOX_RADIUS) * px_per_unit);
        let amount = select(0.0, mix(BOX_AREA, shape, detail), alive);
        color = mix(bg, fill, amount);
    } else if style == STYLE_CIRCLES {
        let shape = coverage((CIRCLE_RADIUS - length(p)) * px_per_unit);
        let amount = select(0.0, mix(CIRCLE_AREA, shape, detail), alive);
        color = mix(bg, fill, amount);
    } else if style == STYLE_GRADIENT {
        // Radial shading: lightened centre fading to a dark rim (r = 1 at the edge midpoints).
        let r = clamp(length(p) * 2.0, 0.0, 1.0);
        let centre = mix(fill, vec3<f32>(1.0), 0.3);
        let shaded = mix(centre, fill * 0.3, smoothstep(0.0, 1.0, r));
        color = select(bg, mix(fill * 0.8, shaded, detail), alive);
    } else if style == STYLE_HEATMAP {
        color = ramp_lerp(raw.b);
    } else if style == STYLE_AGE {
        color = select(bg, ramp_lerp(raw.g), alive);
    } else if style == STYLE_DENSITY {
        // A carries the live fraction of LOD texels; exact frames may leave it 0.
        let density = select(f32(alive), raw.a, raw.a > 0.0);
        color = select(bg, ramp_lerp(density), density > 0.0);
    } else if style == STYLE_HISTORY {
        let trail = raw.b;
        color = select(mix(bg, ramp_lerp(trail), trail), fill, alive);
    }

    // Grid lines: exact cells only, faded in above the zoom threshold.
    if (params.flags & FLAG_GRID) != 0u && params.cells_per_texel == 1u {
        // smoothstep needs low < high (grid_min_px may be 0 = always on).
        let fade_end = max(params.grid_min_px * 1.5, params.grid_min_px + 0.01);
        let fade = smoothstep(params.grid_min_px, fade_end, px_per_cell);
        let edge = min(min(cell_local.x, 1.0 - cell_local.x), min(cell_local.y, 1.0 - cell_local.y)) * px_per_cell;
        var line = (1.0 - smoothstep(0.0, 1.0, edge)) * 0.55;
        if params.major_every > 1u {
            let ix = (params.origin_mod_x + u32(texel.x)) % params.major_every;
            let iy = (params.origin_mod_y + u32(texel.y)) % params.major_every;
            var d = 1e9;
            if ix == 0u { d = min(d, cell_local.x); }
            if ix == params.major_every - 1u { d = min(d, 1.0 - cell_local.x); }
            if iy == 0u { d = min(d, cell_local.y); }
            if iy == params.major_every - 1u { d = min(d, 1.0 - cell_local.y); }
            line = max(line, 1.0 - smoothstep(0.5, 1.5, d * px_per_cell));
        }
        color = mix(color, params.grid_color.rgb, clamp(line * params.grid_color.a * fade, 0.0, 1.0));
    }

    return vec4<f32>(color, 1.0);
}
