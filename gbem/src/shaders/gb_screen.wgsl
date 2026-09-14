// DMG LCD emulation shader.
//
// Inputs: current + previous frame as R8 textures holding shade * 85
// (0, 85, 170, 255 for the four Game Boy shades, 0 = lightest).
// Effects: classic green palette, per-pixel LCD grid, response-time
// ghosting (mix with previous frame), gentle vignette.

struct Uniforms {
    // x, y: screen size in GB pixels (160, 144)
    // z: ghosting amount (0..1)
    // w: grid strength (0..1)
    params: vec4<f32>,
};

@group(0) @binding(0) var t_cur: texture_2d<f32>;
@group(0) @binding(1) var t_prev: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;
@group(0) @binding(3) var<uniform> u: Uniforms;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    // Fullscreen triangle-pair covering the callback viewport.
    var positions = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    var out: VsOut;
    let p = positions[vi];
    out.pos = vec4<f32>(p, 0.0, 1.0);
    out.uv = vec2<f32>(p.x * 0.5 + 0.5, 0.5 - p.y * 0.5);
    return out;
}

// The classic DMG palette, lightest to darkest.
const P0: vec3<f32> = vec3<f32>(0.608, 0.737, 0.059);
const P1: vec3<f32> = vec3<f32>(0.545, 0.675, 0.059);
const P2: vec3<f32> = vec3<f32>(0.188, 0.384, 0.188);
const P3: vec3<f32> = vec3<f32>(0.059, 0.220, 0.059);

fn palette(shade: f32) -> vec3<f32> {
    // shade in [0,1] quantised to 4 levels; interpolate for ghosted values.
    let s = shade * 3.0;
    if s < 1.0 {
        return mix(P0, P1, fract(s));
    } else if s < 2.0 {
        return mix(P1, P2, fract(s));
    }
    return mix(P2, P3, fract(s));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let res = u.params.xy;
    let ghost = u.params.z;
    let grid_strength = u.params.w;

    // Sample with nearest-pixel snapping for crisp cells.
    let texel = (floor(in.uv * res) + 0.5) / res;
    let cur = textureSample(t_cur, samp, texel).r;
    let prev = textureSample(t_prev, samp, texel).r;

    // LCD response-time ghosting: the old image lingers.
    let shade = mix(cur, prev, ghost);

    var color = palette(shade);

    // Pixel grid: darken cell borders like the visible DMG pixel matrix.
    let cell = fract(in.uv * res);
    let border = smoothstep(0.0, 0.12, cell.x) * smoothstep(0.0, 0.12, cell.y)
        * smoothstep(0.0, 0.12, 1.0 - cell.x) * smoothstep(0.0, 0.12, 1.0 - cell.y);
    color *= mix(1.0, mix(0.72, 1.0, border), grid_strength);

    // Subtle vertical shading bands, a hint of the LCD segment structure.
    let band = 1.0 - 0.02 * sin(in.uv.y * res.y * 3.14159);
    color *= band;

    // Vignette.
    let d = in.uv - vec2<f32>(0.5, 0.5);
    color *= 1.0 - 0.35 * dot(d, d);

    return vec4<f32>(color, 1.0);
}
