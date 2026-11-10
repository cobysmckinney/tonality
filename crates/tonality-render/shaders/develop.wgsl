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
    color0: vec4<f32>,  // x: saturation, y: vibrance (-1..1)
    mix_hue0: vec4<f32>, // colour mixer, bands red..green / aqua..magenta (-1..1)
    mix_hue1: vec4<f32>,
    mix_sat0: vec4<f32>,
    mix_sat1: vec4<f32>,
    mix_lum0: vec4<f32>,
    mix_lum1: vec4<f32>,
    vignette: vec4<f32>, // x: amount (-1..1), y: midpoint (0..1), z: feather (0..1)
    grain: vec4<f32>,    // x: amount (0..1), y: size (0..1)
    detail0: vec4<f32>,  // x: sharpen amount, y: sharpen radius (px), z: masking (0..1), w: luminance NR (0..1)
    detail1: vec4<f32>,  // x: colour NR (0..1)
    // Framing (crop / rotate): source uv = [geo0.xyz, geo1.xyz] · (frame position, 1).
    geo0: vec4<f32>,     // w: output width (source px)
    geo1: vec4<f32>,     // w: output height (source px)
    // Masks (local adjustments): x: count, yz: source size / its long edge.
    masks_head: vec4<f32>,
    masks: array<MaskParams, 16>,
    // Lens correction (see `tonality_core::lens`): x: active, y: tile margin, zw: source size / diagonal.
    lens_head: vec4<f32>,
    // Per radius knot (0..LENS_RMAX half-diagonals): rgb: source ÷ output radius, w: vignetting gain.
    lens: array<vec4<f32>, 32>,
};

// One mask; see `tonality_core::mask`. Positions are in "long-edge" units: source uv × masks_head.yz.
struct MaskParams {
    shape0: vec4<f32>, // linear: from.xy, to.xy · radial: centre.xy, radius.xy
    shape1: vec4<f32>, // radial: cos(angle), sin(angle), feather
    info: vec4<f32>,   // x: kind (1 linear, 2 radial, 3 brush), y: invert, z: amount (0..1), w: brush layer
    range: vec4<f32>,  // luminance range: low, high, smoothness (0..1), w: enabled
    adj0: vec4<f32>,   // exposure (stops), contrast, highlights, shadows (-1..1)
    adj1: vec4<f32>,   // whites, blacks, temperature, tint
    adj2: vec4<f32>,   // saturation
};

// Where to draw. Rects are (x, y, w, h). The *frame* is the framed (cropped, rotated) output,
// 0..1 across; the *source* is the full-resolution image, 0..1 across. So the preview, zoomed
// tiles and export tiles all address the same image the same way.
struct View {
    xform: vec4<f32>,   // xy: scale, zw: offset — places the quad in clip space
    quad: vec4<f32>,    // the part of the frame the quad shows
    tex: vec4<f32>,     // the part of the source the bound texture holds
    frame: vec4<f32>,   // xy: full-resolution source size in pixels; z: mask overlay (preview)
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<uniform> view: View;
@group(0) @binding(2) var image: texture_2d<f32>;
@group(0) @binding(3) var image_sampler: sampler;
// Tone curve lookup table: 1024×1, rgb = per-channel curve applied after the master curve.
@group(0) @binding(4) var curve: texture_2d<f32>;
@group(0) @binding(5) var curve_sampler: sampler;
// Brush masks: one 8-bit layer per brush mask, covering the whole source.
@group(0) @binding(6) var brush: texture_2d_array<f32>;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) f: vec2<f32>,                       // frame coordinates
    @location(1) @interpolate(flat) tex: vec4<f32>,  // view.tex
    @location(2) @interpolate(flat) px: vec4<f32>,   // xy: texture units per source pixel; zw: source uv per pixel
    @location(3) @interpolate(flat) overlay: f32,    // preview: 1 + index of the mask to show, or 0
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    // Two triangles covering the image quad.
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0),
        vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
    );
    let p = corners[i];
    let local = vec2<f32>(p.x * 0.5 + 0.5, 0.5 - p.y * 0.5);
    var out: VsOut;
    out.pos = vec4<f32>(p * view.xform.xy + view.xform.zw, 0.0, 1.0);
    out.f = view.quad.xy + local * view.quad.zw;
    out.tex = view.tex;
    out.px = vec4<f32>(1.0 / (view.frame.xy * view.tex.zw), 1.0 / view.frame.xy);
    out.overlay = view.frame.z;
    return out;
}

// ---- Detail: noise reduction and sharpening ----------------------------------------------------
// Runs on white-balanced camera RGB. Luminance is handled as sqrt(L), where photon (shot) noise
// is roughly uniform, so one strength works across shadows and highlights. Kernel sizes are in
// full-resolution pixels: on the downscaled preview the taps fall within a texel and the effect
// shrinks accordingly, just as it would in a downscaled export.

const CAM_LUMA = vec3<f32>(0.25, 0.5, 0.25);

fn cam_at(tc: vec2<f32>) -> vec3<f32> {
    let c = textureSampleLevel(image, image_sampler, tc, 0.0).rgb;
    // White balance, then clip so blown highlights stay neutral.
    return clamp(c * params.wb.xyz, vec3<f32>(0.0), vec3<f32>(params.wb.w));
}

fn root_luma(c: vec3<f32>) -> f32 {
    return sqrt(max(dot(c, CAM_LUMA), 0.0));
}

fn detail(tc: vec2<f32>, px: vec2<f32>) -> vec3<f32> {
    let c0 = cam_at(tc);
    let amount = params.detail0.x;
    let lnr = params.detail0.w;
    let cnr = params.detail1.x;
    if (amount == 0.0 && lnr == 0.0 && cnr == 0.0) {
        return c0;
    }
    let s0 = root_luma(c0);
    var s = s0;
    var chroma = c0 / max(s0 * s0, 1e-6);

    // Luminance: 5×5 bilateral filter.
    let sigma_r = 0.06 * lnr + 1e-4;
    if (lnr > 0.0) {
        var acc = 0.0;
        var wsum = 0.0;
        for (var j = -2; j <= 2; j = j + 1) {
            for (var i = -2; i <= 2; i = i + 1) {
                let o = vec2<f32>(f32(i), f32(j));
                let v = root_luma(cam_at(tc + o * px));
                let d = (v - s0) / sigma_r;
                let w = exp(-dot(o, o) / 4.5 - 0.5 * d * d);
                acc = acc + v * w;
                wsum = wsum + w;
            }
        }
        s = mix(s0, acc / wsum, min(1.0, lnr * 2.0));
    }

    // Colour: average the chroma ratio over a wide window, stopping at luminance edges.
    if (cnr > 0.0) {
        let step = 1.5 + 2.5 * cnr;
        var acc = vec3<f32>(0.0);
        var wsum = 0.0;
        for (var j = -2; j <= 2; j = j + 1) {
            for (var i = -2; i <= 2; i = i + 1) {
                let o = vec2<f32>(f32(i), f32(j));
                let c = cam_at(tc + o * step * px);
                let v = root_luma(c);
                let d = (v - s0) / 0.05;
                let w = exp(-dot(o, o) / 8.0 - 0.5 * d * d);
                acc = acc + w * c / max(v * v, 1e-6);
                wsum = wsum + w;
            }
        }
        chroma = mix(chroma, acc / wsum, min(1.0, cnr * 1.5));
    }

    // Sharpening: unsharp mask on sqrt luminance, with coring (ignores noise-sized detail when
    // noise reduction is on), edge masking and overshoot limiting (no halos).
    if (amount > 0.0) {
        let r = params.detail0.y;
        var n: array<f32, 9>;
        var mn = s0;
        var mx = s0;
        for (var k = 0; k < 9; k = k + 1) {
            let o = vec2<f32>(f32(k % 3 - 1), f32(k / 3 - 1));
            n[k] = root_luma(cam_at(tc + o * r * px));
            mn = min(mn, n[k]);
            mx = max(mx, n[k]);
        }
        let blur = (4.0 * n[4] + 2.0 * (n[1] + n[3] + n[5] + n[7]) + n[0] + n[2] + n[6] + n[8]) / 16.0;
        var d = s0 - blur;
        let t = 0.5 * sigma_r * step(1e-3, lnr);
        d = sign(d) * max(abs(d) - t, 0.0);
        let gx = (n[2] + 2.0 * n[5] + n[8]) - (n[0] + 2.0 * n[3] + n[6]);
        let gy = (n[6] + 2.0 * n[7] + n[8]) - (n[0] + 2.0 * n[1] + n[2]);
        let edge = abs(gx) + abs(gy);
        let m = params.detail0.z * 0.12;
        let mask = select(1.0, smoothstep(m * 0.5, m, edge), m > 0.0);
        let margin = 0.02 + 0.1 * (mx - mn);
        s = clamp(s + amount * d * mask, mn - margin, mx + margin);
        s = max(s, 0.0);
    }

    return max(chroma * s * s, vec3<f32>(0.0));
}

const REC2020_LUMA = vec3<f32>(0.2627, 0.6780, 0.0593);
const MIDDLE_GREY: f32 = 0.18;

// ---- Tone ------------------------------------------------------------------------------------
// Operates on log2 luminance relative to middle grey and rescales RGB by the luminance ratio,
// which preserves hue and saturation.
// `t`: contrast, highlights, shadows, whites; `blacks` (-1..1, larger where masks add up).
fn tone(rgb: vec3<f32>, t: vec4<f32>, blacks: f32) -> vec3<f32> {
    let y = max(dot(rgb, REC2020_LUMA), 1e-7);
    var ev = log2(y / MIDDLE_GREY);

    let contrast = t.x;
    let highlights = t.y;
    let shadows = t.z;
    let whites = t.w;

    ev = ev * (1.0 + 0.6 * contrast);
    ev = ev + 1.5 * highlights * smoothstep(-0.5, 3.5, ev);
    ev = ev + 1.5 * shadows * (1.0 - smoothstep(-6.0, -0.5, ev));
    ev = ev + 1.0 * whites * smoothstep(1.0, 4.5, ev);
    ev = ev + 1.5 * blacks * (1.0 - smoothstep(-10.0, -4.0, ev));

    let y2 = MIDDLE_GREY * exp2(ev);
    return rgb * (y2 / y);
}

// ---- Colour (in OkLCh) -----------------------------------------------------------------------
// Rec.2020 linear ↔ OkLab, with the XYZ step folded into the LMS matrices.
fn rows(r0: vec3<f32>, r1: vec3<f32>, r2: vec3<f32>, v: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(dot(r0, v), dot(r1, v), dot(r2, v));
}

fn cbrt(v: vec3<f32>) -> vec3<f32> {
    return sign(v) * pow(abs(v), vec3<f32>(1.0 / 3.0));
}

fn to_oklab(rgb: vec3<f32>) -> vec3<f32> {
    let lms = rows(
        vec3<f32>(0.6166884, 0.36015908, 0.0230433),
        vec3<f32>(0.26514018, 0.63585651, 0.09903022),
        vec3<f32>(0.10015064, 0.20400434, 0.6963247),
        rgb,
    );
    return rows(
        vec3<f32>(0.2104542553, 0.7936177850, -0.0040720468),
        vec3<f32>(1.9779984951, -2.4285922050, 0.4505937099),
        vec3<f32>(0.0259040371, 0.7827717662, -0.8086757660),
        cbrt(lms),
    );
}

fn from_oklab(lab: vec3<f32>) -> vec3<f32> {
    let l = rows(
        vec3<f32>(1.0, 0.39633779, 0.21580376),
        vec3<f32>(1.0, -0.10556134, -0.06385417),
        vec3<f32>(1.0, -0.08948418, -1.29148554),
        lab,
    );
    return rows(
        vec3<f32>(2.14014057, -1.24635599, 0.10643167),
        vec3<f32>(-0.88483244, 2.16317261, -0.27836152),
        vec3<f32>(-0.04857905, -0.4544909, 1.50235623),
        l * l * l,
    );
}

// Colour mixer band centres (OkLCh hue, degrees): red, orange, yellow, green, aqua, blue,
// purple, magenta. A pixel's adjustment blends smoothly between the two bands around its hue,
// so the bands always sum to exactly one (no gaps, no double-counting).
fn band_value(lo: vec4<f32>, hi: vec4<f32>, h: f32) -> f32 {
    var centres = array<f32, 8>(29.0, 55.0, 105.0, 142.0, 195.0, 264.0, 294.0, 328.0);
    var vals = array<f32, 8>(lo.x, lo.y, lo.z, lo.w, hi.x, hi.y, hi.z, hi.w);
    var out = 0.0;
    for (var i = 0u; i < 8u; i = i + 1u) {
        let j = (i + 1u) % 8u;
        let span = (centres[j] - centres[i] + 360.0) % 360.0;
        let d = (h - centres[i] + 360.0) % 360.0;
        if (d < span) {
            out = mix(vals[i], vals[j], smoothstep(0.0, 1.0, d / span));
        }
    }
    return out;
}

fn color(rgb: vec3<f32>, saturation: f32) -> vec3<f32> {
    let lab = to_oklab(max(rgb, vec3<f32>(0.0)));
    var l = lab.x;
    var c = length(lab.yz);
    var h = degrees(atan2(lab.z, lab.y));
    h = (h + 360.0) % 360.0;

    // Colour mixer: fades out towards neutral, where hue is meaningless.
    let colourful = smoothstep(0.0, 0.05, c);
    let dh = band_value(params.mix_hue0, params.mix_hue1, h);
    let ds = band_value(params.mix_sat0, params.mix_sat1, h);
    let dl = band_value(params.mix_lum0, params.mix_lum1, h);
    h = h + 30.0 * dh * colourful;
    c = c * max(0.0, 1.0 + ds * colourful);
    l = l * exp2(0.5 * dl * colourful);

    // Vibrance favours muted colours; saturation is uniform.
    let vib = params.color0.y;
    c = c * max(0.0, 1.0 + vib * (1.0 - smoothstep(0.0, 0.2, c)));
    c = c * max(0.0, 1.0 + saturation);

    let hr = radians(h);
    return max(from_oklab(vec3<f32>(l, c * cos(hr), c * sin(hr))), vec3<f32>(0.0));
}

// ---- Effects ---------------------------------------------------------------------------------
fn vignette(rgb: vec3<f32>, uv: vec2<f32>) -> vec3<f32> {
    let amount = params.vignette.x;
    if (amount == 0.0) {
        return rgb;
    }
    // Follows the frame's shape: 0 at the centre, 1 in the corners.
    let r = length((uv - 0.5) * 2.0) / 1.41421356;
    let mid = mix(0.15, 1.0, params.vignette.y);
    let feather = mix(0.05, 1.2, params.vignette.z);
    let v = smoothstep(mid - feather * 0.5, mid + feather * 0.5, r);
    return rgb * exp2(1.5 * amount * v); // up to ±1.5 EV at the edges
}

fn pcg(v: u32) -> u32 {
    let s = v * 747796405u + 2891336453u;
    let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return (w >> 22u) ^ w;
}

fn lattice(p: vec2<i32>) -> f32 {
    return f32(pcg(u32(p.x) + pcg(u32(p.y)))) / 4294967295.0 * 2.0 - 1.0;
}

fn value_noise(p: vec2<f32>) -> f32 {
    let i = vec2<i32>(floor(p));
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = lattice(i);
    let b = lattice(i + vec2<i32>(1, 0));
    let c = lattice(i + vec2<i32>(0, 1));
    let d = lattice(i + vec2<i32>(1, 1));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// Film-like grain in display space, sized relative to the image (identical at any resolution).
fn grain(c: vec3<f32>, uv: vec2<f32>, aspect: f32) -> vec3<f32> {
    let amount = params.grain.x;
    if (amount == 0.0) {
        return c;
    }
    let cells = mix(1600.0, 250.0, params.grain.y); // grains per image height
    let q = uv * vec2<f32>(aspect, 1.0) * cells;
    let n = 0.7 * value_noise(q) + 0.3 * value_noise(q * 2.13 + 17.0);
    let y = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    let midtones = 0.35 + 2.6 * y * (1.0 - y);
    return c + vec3<f32>(0.16 * amount * n * midtones);
}

fn apply_curve(c: vec3<f32>) -> vec3<f32> {
    let n = 1024.0;
    let t = (clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)) * (n - 1.0) + 0.5) / n;
    return vec3<f32>(
        textureSampleLevel(curve, curve_sampler, vec2<f32>(t.x, 0.5), 0.0).r,
        textureSampleLevel(curve, curve_sampler, vec2<f32>(t.y, 0.5), 0.0).g,
        textureSampleLevel(curve, curve_sampler, vec2<f32>(t.z, 0.5), 0.0).b,
    );
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

// ---- Masks -------------------------------------------------------------------------------------
// How much mask `i` selects at source position `uv`, for a pixel of display luminance `lum`.
fn mask_value(i: u32, uv: vec2<f32>, lum: f32) -> f32 {
    let mk = params.masks[i];
    let q = uv * params.masks_head.yz;
    var m = 0.0;
    if (mk.info.x < 1.5) {
        let a = mk.shape0.xy;
        let d = mk.shape0.zw - a;
        let t = dot(q - a, d) / max(dot(d, d), 1e-12);
        m = 1.0 - smoothstep(0.0, 1.0, t);
    } else if (mk.info.x < 2.5) {
        let d = q - mk.shape0.xy;
        let cs = mk.shape1.xy;
        let r = vec2<f32>(cs.x * d.x + cs.y * d.y, cs.x * d.y - cs.y * d.x) / max(mk.shape0.zw, vec2<f32>(1e-6));
        m = 1.0 - smoothstep(1.0 - max(mk.shape1.z, 1e-3), 1.0, length(r));
    } else {
        m = textureSampleLevel(brush, image_sampler, uv, i32(mk.info.w), 0.0).r;
    }
    if (mk.info.y > 0.5) {
        m = 1.0 - m;
    }
    if (mk.range.w > 0.5) {
        let s = max(mk.range.z, 1e-3);
        m = m * smoothstep(mk.range.x - s, mk.range.x, lum) * (1.0 - smoothstep(mk.range.y, mk.range.y + s, lum));
    }
    return m * mk.info.z;
}

// Shift white balance by `t` (cooler/warmer) and `g` (green/magenta), keeping luminance.
fn white_shift(t: f32, g: f32) -> vec3<f32> {
    let k = exp2(vec3<f32>(0.45 * t + 0.15 * g, -0.3 * g, -0.45 * t + 0.15 * g));
    return k / dot(k, REC2020_LUMA);
}

// `cam`: white-balanced camera RGB (see `detail`); `f`: position in the frame; `uv`: in the source.
fn develop(cam: vec3<f32>, f: vec2<f32>, uv: vec2<f32>, aspect: f32) -> vec3<f32> {
    var rgb = vec3<f32>(dot(params.cam0.xyz, cam), dot(params.cam1.xyz, cam), dot(params.cam2.xyz, cam));
    rgb = max(rgb, vec3<f32>(0.0)) * params.light0.x;

    // Local adjustments: each mask adds its settings, weighted by how much it selects here, to
    // the global ones; then the pipeline runs once.
    var t = vec4<f32>(params.light0.yzw, params.light1.x);
    var blacks = params.light1.y;
    var saturation = params.color0.x;
    let n = u32(params.masks_head.x);
    if (n > 0u) {
        let lum = srgb_encode(agx(vec3<f32>(max(dot(rgb, REC2020_LUMA), 0.0)))).y;
        var ev = 0.0;
        var wb = vec2<f32>(0.0);
        for (var i = 0u; i < n; i = i + 1u) {
            let m = mask_value(i, uv, lum);
            if (m <= 0.0) {
                continue;
            }
            let mk = params.masks[i];
            ev = ev + m * mk.adj0.x;
            t = t + m * vec4<f32>(mk.adj0.yzw, mk.adj1.x);
            blacks = blacks + m * mk.adj1.y;
            wb = wb + m * mk.adj1.zw;
            saturation = saturation + m * mk.adj2.x;
        }
        rgb = rgb * exp2(ev) * white_shift(wb.x, wb.y);
    }

    rgb = vignette(rgb, f);
    rgb = tone(rgb, t, blacks);
    rgb = color(rgb, saturation);
    var c = srgb_encode(agx(rgb));
    c = apply_curve(c);
    c = grain(c, f, aspect);
    return clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
}

// Preview only: tint what the selected mask (view.frame.z - 1) selects.
fn mask_overlay(c: vec3<f32>, f: vec2<f32>, uv: vec2<f32>, cam: vec3<f32>, which: f32) -> vec3<f32> {
    if (which < 0.5) {
        return c;
    }
    var rgb = vec3<f32>(dot(params.cam0.xyz, cam), dot(params.cam1.xyz, cam), dot(params.cam2.xyz, cam));
    rgb = max(rgb, vec3<f32>(0.0)) * params.light0.x;
    let lum = srgb_encode(agx(vec3<f32>(max(dot(rgb, REC2020_LUMA), 0.0)))).y;
    let m = clamp(mask_value(u32(which - 1.0), uv, lum), 0.0, 1.0);
    return mix(c, vec3<f32>(1.0, 0.18, 0.12), 0.55 * m);
}

// ---- Lens correction ---------------------------------------------------------------------------
// Everything downstream (framing, masks) works on the corrected image; only the texture reads go
// through the lens. Radial: a pixel at radius r reads the source at r × ratio(r), per channel.

const LENS_RMAX: f32 = 1.2;

fn lens_at(r: f32) -> vec4<f32> {
    let t = clamp(r / LENS_RMAX, 0.0, 1.0) * 31.0;
    let i = min(u32(t), 30u);
    return mix(params.lens[i], params.lens[i + 1u], t - f32(i));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let uv = vec2<f32>(dot(params.geo0.xyz, vec3<f32>(in.f, 1.0)), dot(params.geo1.xyz, vec3<f32>(in.f, 1.0)));
    // Outside the source (a straightened frame's corners while cropping): leave the background.
    let eps = 0.5 * in.px.zw;
    if (any(uv < -eps) || any(uv > vec2<f32>(1.0) + eps)) {
        discard;
    }
    let lens_on = params.lens_head.x > 0.5;
    var src = uv;
    var k = vec4<f32>(1.0);
    var gain = 1.0;
    if (lens_on) {
        let d = uv - vec2<f32>(0.5);
        let r = length(d * 2.0 * params.lens_head.zw);
        k = lens_at(r);
        // Clamped: auto-zoom keeps the frame inside the source, up to table interpolation error.
        src = clamp(vec2<f32>(0.5) + k.y * d, vec2<f32>(0.0), vec2<f32>(1.0));
        gain = lens_at(r * k.y).w;
    }
    let tc = (src - in.tex.xy) / in.tex.zw;
    // Outside this texture (a zoomed-in full-res tile): whatever is underneath shows.
    if (any(tc < vec2<f32>(0.0)) || any(tc > vec2<f32>(1.0))) {
        discard;
    }
    var cam = detail(tc, in.px.xy);
    if (lens_on) {
        // Lateral chromatic aberration: red and blue come from slightly different radii.
        if (k.x != k.y || k.z != k.y) {
            let d = uv - vec2<f32>(0.5);
            let g = cam_at(tc);
            let tr = (clamp(vec2<f32>(0.5) + k.x * d, vec2<f32>(0.0), vec2<f32>(1.0)) - in.tex.xy) / in.tex.zw;
            let tb = (clamp(vec2<f32>(0.5) + k.z * d, vec2<f32>(0.0), vec2<f32>(1.0)) - in.tex.xy) / in.tex.zw;
            cam.r = max(cam.r + cam_at(tr).r - g.r, 0.0);
            cam.b = max(cam.b + cam_at(tb).b - g.b, 0.0);
        }
        cam = cam * gain;
    }
    let c = develop(cam, in.f, uv, params.geo0.w / params.geo1.w);
    return vec4<f32>(mask_overlay(c, in.f, uv, cam, in.overlay), 1.0);
}
