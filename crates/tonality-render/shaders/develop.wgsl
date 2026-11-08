// tonality develop pipeline — the single source of truth for how an edit looks.
// Runs natively via wgpu (export) and in the webview via naga → GLSL ES 3.00 (preview).
//
// Input texture: camera RGB, linear, black-subtracted, 0..1, *no* white balance.
// Output: display-referred sRGB (encoded).

struct Params {
    wb: vec4<f32>,      // xyz: camera-space multipliers (min = 1), w: clip level
    cam0: vec4<f32>,    // rows of camera → linear Rec.2020
    cam1: vec4<f32>,
    cam2: vec4<f32>,
    light0: vec4<f32>,  // x: exposure gain, y: contrast, z: highlights, w: shadows  (-1..1)
    light1: vec4<f32>,  // x: whites, y: blacks (-1..1)
};

struct View {
    xform: vec4<f32>,   // xy: scale, zw: offset (clip space)
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<uniform> view: View;
@group(0) @binding(2) var image: texture_2d<f32>;
@group(0) @binding(3) var image_sampler: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    // Two triangles covering the image quad.
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0),
        vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
    );
    let p = corners[i];
    var out: VsOut;
    out.pos = vec4<f32>(p * view.xform.xy + view.xform.zw, 0.0, 1.0);
    out.uv = vec2<f32>(p.x * 0.5 + 0.5, 0.5 - p.y * 0.5);
    return out;
}

const REC2020_LUMA = vec3<f32>(0.2627, 0.6780, 0.0593);
const MIDDLE_GREY: f32 = 0.18;

// ---- Tone ------------------------------------------------------------------------------------
// Operates on log2 luminance relative to middle grey and rescales RGB by the luminance ratio,
// which preserves hue and saturation.
fn tone(rgb: vec3<f32>) -> vec3<f32> {
    let y = max(dot(rgb, REC2020_LUMA), 1e-7);
    var ev = log2(y / MIDDLE_GREY);

    let contrast = params.light0.y;
    let highlights = params.light0.z;
    let shadows = params.light0.w;
    let whites = params.light1.x;
    let blacks = params.light1.y;

    ev = ev * (1.0 + 0.6 * contrast);
    ev = ev + 1.5 * highlights * smoothstep(-0.5, 3.5, ev);
    ev = ev + 1.5 * shadows * (1.0 - smoothstep(-6.0, -0.5, ev));
    ev = ev + 1.0 * whites * smoothstep(1.0, 4.5, ev);
    ev = ev + 1.5 * blacks * (1.0 - smoothstep(-10.0, -4.0, ev));

    let y2 = MIDDLE_GREY * exp2(ev);
    return rgb * (y2 / y);
}

// ---- Display transform: AgX (Rec.2020 variant) -----------------------------------------------
fn agx_contrast(x: vec3<f32>) -> vec3<f32> {
    let x2 = x * x;
    let x4 = x2 * x2;
    return 15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2
        + 0.1191 * x - 0.00232;
}

fn agx(rgb: vec3<f32>) -> vec3<f32> {
    let inset = mat3x3<f32>(
        vec3<f32>(0.856627153315983, 0.137318972929847, 0.11189821299995),
        vec3<f32>(0.0951212405381588, 0.761241990602591, 0.0767994186031903),
        vec3<f32>(0.0482516061458583, 0.101439036467562, 0.811302368396859),
    );
    let outset = mat3x3<f32>(
        vec3<f32>(1.1271005818144368, -0.1413297634984383, -0.14132976349843826),
        vec3<f32>(-0.11060664309660323, 1.157823702216272, -0.11060664309660294),
        vec3<f32>(-0.016493938717834573, -0.016493938717834257, 1.2519364065950405),
    );
    let rec2020_to_srgb = mat3x3<f32>(
        vec3<f32>(1.6605, -0.1246, -0.0182),
        vec3<f32>(-0.5876, 1.1329, -0.1006),
        vec3<f32>(-0.0728, -0.0083, 1.1187),
    );
    let min_ev = -12.47393;
    let max_ev = 4.026069;

    var c = inset * rgb;
    c = max(c, vec3<f32>(1e-10));
    c = clamp((log2(c) - min_ev) / (max_ev - min_ev), vec3<f32>(0.0), vec3<f32>(1.0));
    c = agx_contrast(c);
    c = outset * c;
    c = pow(max(c, vec3<f32>(0.0)), vec3<f32>(2.2));
    c = rec2020_to_srgb * c;
    return clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
}

fn srgb_encode(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

fn develop(cam_in: vec3<f32>) -> vec3<f32> {
    // White balance in camera space, then clip so blown highlights stay neutral.
    let cam = clamp(cam_in * params.wb.xyz, vec3<f32>(0.0), vec3<f32>(params.wb.w));
    var rgb = vec3<f32>(dot(params.cam0.xyz, cam), dot(params.cam1.xyz, cam), dot(params.cam2.xyz, cam));
    rgb = max(rgb, vec3<f32>(0.0)) * params.light0.x;
    rgb = tone(rgb);
    return srgb_encode(agx(rgb));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let cam = textureSample(image, image_sampler, in.uv).rgb;
    return vec4<f32>(develop(cam), 1.0);
}
