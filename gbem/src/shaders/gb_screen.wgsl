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
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let res = u.params.xy;
    let ghost = u.params.z;
    let grid_strength = u.params.w;

    // Sample with nearest-pixel snapping for crisp cells. The PPU already
    // produces final RGB (DMG green or CGB colour), so we work in colour space.
    let texel = (floor(in.uv * res) + 0.5) / res;
    let cur = textureSample(t_cur, samp, texel).rgb;
    let prev = textureSample(t_prev, samp, texel).rgb;

    // LCD response-time ghosting: the old image lingers.
    var color = mix(cur, prev, ghost);

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
