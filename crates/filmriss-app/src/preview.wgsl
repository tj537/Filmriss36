// Filmriss live preview
//
// Per pixel: sample camera frame → 3D LUT (hardware trilinear) → intensity → vignette → grain.
// Must behave exactly like `filmriss_core::process::shade` (same formulas, same hash).

struct Uniforms {
    // camera frame size in pixels
    tex_size: vec2<f32>,
    intensity: f32,
    grain: f32,
    grain_size: f32,
    vignette: f32,
    // x < split → show original (0 = off, 1 = fully original)
    split: f32,
    // 1 = mirror horizontally (selfie view)
    mirror: f32,
    seed: u32,
    lut_size: f32,
    // 1 = target format is *Srgb, output must be linearized
    linear_output: f32,
    _pad: f32,
};

@group(0) @binding(0) var cam_tex: texture_2d<f32>;
@group(0) @binding(1) var cam_sampler: sampler;
@group(0) @binding(2) var lut_tex: texture_3d<f32>;
@group(0) @binding(3) var lut_sampler: sampler;
@group(0) @binding(4) var<uniform> u: Uniforms;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// One full-screen triangle; egui sets the viewport to the preview rectangle.
@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    var out: VsOut;
    out.pos = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    out.uv = vec2<f32>(x, y);
    return out;
}

fn hash(x: u32, y: u32, seed: u32) -> u32 {
    var h = (x * 0x8da6b343u) ^ (y * 0xd8163841u) ^ (seed * 0xcb1ab31fu);
    h = h ^ (h >> 16u);
    h = h * 0x7feb352du;
    h = h ^ (h >> 15u);
    h = h * 0x846ca68bu;
    h = h ^ (h >> 16u);
    return h;
}

fn hash_signed(x: u32, y: u32, seed: u32) -> f32 {
    return f32(hash(x, y, seed) >> 8u) * (2.0 / 16777216.0) - 1.0;
}

fn grain_noise(p: vec2<f32>, size: f32, seed: u32) -> f32 {
    let g = p / max(size, 0.5);
    let i = floor(g);
    let f = g - i;
    let s = f * f * (3.0 - 2.0 * f);
    let x0 = u32(i.x);
    let y0 = u32(i.y);
    let n00 = hash_signed(x0, y0, seed);
    let n10 = hash_signed(x0 + 1u, y0, seed);
    let n01 = hash_signed(x0, y0 + 1u, seed);
    let n11 = hash_signed(x0 + 1u, y0 + 1u, seed);
    let a = mix(n00, n10, s.x);
    let b = mix(n01, n11, s.x);
    return mix(a, b, s.y) * 1.6;
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    var uv = in.uv;
    if (u.mirror > 0.5) {
        uv.x = 1.0 - uv.x;
    }
    let src = textureSampleLevel(cam_tex, cam_sampler, uv, 0.0).rgb;

    // Half-texel offset: LUT grid points sit on texel centers.
    let n = u.lut_size;
    let lut_coord = clamp(src, vec3<f32>(0.0), vec3<f32>(1.0)) * ((n - 1.0) / n) + 0.5 / n;
    let graded = textureSampleLevel(lut_tex, lut_sampler, lut_coord, 0.0).rgb;
    var c = mix(src, graded, u.intensity);

    let px = uv * u.tex_size;

    if (u.vignette != 0.0) {
        let aspect = u.tex_size.x / u.tex_size.y;
        let d = length(vec2<f32>((px.x / u.tex_size.x - 0.5) * aspect, px.y / u.tex_size.y - 0.5))
            / sqrt(pow(aspect * 0.5, 2.0) + 0.25);
        c = c * (1.0 - u.vignette * 0.6 * smoothstep(0.35, 1.05, d));
    }

    if (u.grain > 0.0) {
        let l = clamp(dot(c, vec3<f32>(0.2126, 0.7152, 0.0722)), 0.0, 1.0);
        let amp = u.grain * 0.14 * (0.25 + 3.0 * l * (1.0 - l));
        c = c + vec3<f32>(grain_noise(px, u.grain_size, u.seed) * amp);
    }

    // Before/after comparison (in screen coordinates, not mirrored)
    if (in.uv.x < u.split) {
        c = src;
    }

    c = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    if (u.linear_output > 0.5) {
        c = srgb_to_linear(c);
    }
    return vec4<f32>(c, 1.0);
}
