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
    // Masks (local adjustments): x: count, yz: source size / its long edge, w: 1 if any mask
    // sharpens or denoises.
    masks_head: vec4<f32>,
    masks: array<MaskParams, 16>,
    // Lens correction (see `tonality_core::lens`): x: active, y: tile margin, zw: source size / diagonal.
    lens_head: vec4<f32>,
    // Per radius knot (0..LENS_RMAX half-diagonals): rgb: source ÷ output radius, w: vignetting gain.
    lens: array<vec4<f32>, 32>,
    // AgX (see `tonality_core::agx` and docs/agx.md), matrices as rows: 0–2 Rec.2020 → E-Gamut,
    // 3–5 back, 6–8 inset, 9–11 outset then → display primaries; 12: Rec.2020 luminance weights,
    // w: input floor; 13: display luminance weights, w: input ceiling; 14: sigmoid pivot x, y,
    // toe scale, shoulder scale.
    agx: array<vec4<f32>, 15>,
    // Texture, Clarity, Dehaze (see `tonality_render::presence`): 0: as-shot luminance weights
    // for camera RGB, w: clip level; 1: as-shot white balance, w: the airlight's as-shot
    // luminance; 2: the airlight (camera RGB), w: 1 if the maps are bound; 3: global texture,
    // clarity, dehaze (-1..1).
    presence: array<vec4<f32>, 4>,
};

// One mask; see `tonality_core::mask`. Positions are in "long-edge" units: source uv × masks_head.yz.
struct MaskParams {
    shape0: vec4<f32>, // linear: from.xy, to.xy · radial: centre.xy, radius.xy
    shape1: vec4<f32>, // radial: cos(angle), sin(angle), feather
    info: vec4<f32>,   // x: kind (1 linear, 2 radial, 3 brush), y: invert, z: amount (0..1), w: brush layer
    range: vec4<f32>,  // luminance range: low, high, smoothness (0..1), w: enabled
    adj0: vec4<f32>,   // exposure (stops), contrast, highlights, shadows (-1..1)
    adj1: vec4<f32>,   // whites, blacks, temperature, tint
    adj2: vec4<f32>,   // saturation, vibrance, hue shift, sharpness (-1..1)
    adj3: vec4<f32>,   // x: noise reduction (-1..1), y: tint hue (OkLCh, radians), z: tint amount (0..1), w: texture
    adj4: vec4<f32>,   // x: clarity, y: dehaze (-1..1)
};

// Where to draw. Rects are (x, y, w, h). The *frame* is the framed (cropped, rotated) output,
// 0..1 across; the *source* is the full-resolution image, 0..1 across. So the preview, zoomed
// tiles and export tiles all address the same image the same way.
struct View {
    xform: vec4<f32>,   // xy: scale, zw: offset — places the quad in clip space
    quad: vec4<f32>,    // the part of the frame the quad shows
    tex: vec4<f32>,     // the part of the source the bound texture holds
    frame: vec4<f32>,   // xy: full-resolution source size in pixels; z: mask overlay, w: overlay mode (preview)
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
// Clarity and dehaze maps (`tonality_render::presence`), covering the whole source: rg: clarity's
// guided-filter coefficients, ba: the haze transmission's.
@group(0) @binding(7) var presence_map: texture_2d<f32>;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) f: vec2<f32>,                       // frame coordinates
    @location(1) @interpolate(flat) tex: vec4<f32>,  // view.tex
    @location(2) @interpolate(flat) px: vec4<f32>,   // xy: texture units per source pixel; zw: source uv per pixel
    @location(3) @interpolate(flat) overlay: vec2<f32>, // preview: x: 1 + index of the mask to show, or 0; y: 1 = the mask alone, in black and white
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
    out.overlay = view.frame.zw;
    return out;
}

// ---- Detail: noise reduction and sharpening ----------------------------------------------------
// Runs on white-balanced camera RGB, negatives included: sensor noise around black is
// symmetric, and clipping it before averaging would lift and tint the shadows. Colour splits into
// luminance L and an additive chroma difference d = c − L (as YCbCr does), so nothing divides by
// a near-zero L. Luminance is handled as a signed √L, where photon (shot) noise is roughly
// uniform, so one strength works across shadows and highlights. Kernel sizes are in
// full-resolution pixels: on the downscaled preview the taps fall within a texel and the effect
// shrinks accordingly, just as it would in a downscaled export.

const CAM_LUMA = vec3<f32>(0.25, 0.5, 0.25);

fn cam_at(tc: vec2<f32>) -> vec3<f32> {
    let c = textureSampleLevel(image, image_sampler, tc, 0.0).rgb;
    // White balance, then clip so blown highlights stay neutral (the low end is clipped after
    // noise reduction, in fs_main).
    return min(c * params.wb.xyz, vec3<f32>(params.wb.w));
}

// Signed square root of camera luminance, and back.
fn root_luma(c: vec3<f32>) -> f32 {
    let l = dot(c, CAM_LUMA);
    return sign(l) * sqrt(abs(l));
}

fn unroot(s: f32) -> f32 {
    return s * abs(s);
}

// `local`: what masks add here: x: sharpness, y: luminance noise reduction (-1..1).
fn detail(tc: vec2<f32>, px: vec2<f32>, local: vec2<f32>) -> vec3<f32> {
    let c0 = cam_at(tc);
    // Negative sharpening softens (down to the blur itself at -1).
    let amount = max(params.detail0.x + 1.6 * local.x, -1.0);
    let lnr = clamp(params.detail0.w + local.y, 0.0, 1.0);
    let cnr = params.detail1.x;
    if (amount == 0.0 && lnr == 0.0 && cnr == 0.0) {
        return c0;
    }
    let s0 = root_luma(c0);
    var s = s0;
    var chroma = c0 - vec3<f32>(unroot(s0));

    // Luminance: 5×5 bilateral filter. Neighbours are weighted by their difference in √L (where
    // noise is uniform), but averaged linearly: averaging √L and squaring back would darken
    // noisy shadows (the mean of a square root is below the square root of the mean).
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
                acc = acc + unroot(v) * w;
                wsum = wsum + w;
            }
        }
        let l = mix(unroot(s0), acc / wsum, min(1.0, lnr * 2.0));
        s = sign(l) * sqrt(abs(l));
    }

    // Colour: average the chroma difference over a wide window, stopping at luminance edges.
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
                acc = acc + w * (c - vec3<f32>(unroot(v)));
                wsum = wsum + w;
            }
        }
        chroma = mix(chroma, acc / wsum, min(1.0, cnr * 1.5));
    }

    // Sharpening: unsharp mask on sqrt luminance, with coring (ignores noise-sized detail when
    // noise reduction is on), edge masking and overshoot limiting (no halos).
    if (amount != 0.0) {
        let soften = amount < 0.0;
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
        if (!soften) {
            let t = 0.5 * sigma_r * step(1e-3, lnr);
            d = sign(d) * max(abs(d) - t, 0.0);
        }
        let gx = (n[2] + 2.0 * n[5] + n[8]) - (n[0] + 2.0 * n[3] + n[6]);
        let gy = (n[6] + 2.0 * n[7] + n[8]) - (n[0] + 2.0 * n[1] + n[2]);
        let edge = abs(gx) + abs(gy);
        let m = params.detail0.z * 0.12;
        let mask = select(1.0, smoothstep(m * 0.5, m, edge), m > 0.0 && !soften);
        let margin = 0.02 + 0.1 * (mx - mn);
        s = clamp(s + amount * d * mask, mn - margin, mx + margin);
    }

    return chroma + vec3<f32>(unroot(s));
}

const REC2020_LUMA = vec3<f32>(0.2627, 0.6780, 0.0593);
const MIDDLE_GREY: f32 = 0.18;

// Luminance for scaling a colour by a luminance ratio. Colours outside Rec.2020 (a negative
// component) can have a luminance near zero, or below it, while another component is large, and
// the ratio would blow them up; so it's floored at the smallest weight × the largest component,
// which a colour inside the gamut never goes below (it changes nothing there).
fn scaling_luma(rgb: vec3<f32>) -> f32 {
    return max(max(dot(rgb, REC2020_LUMA), REC2020_LUMA.z * max(rgb.x, max(rgb.y, rgb.z))), 1e-7);
}

// ---- Tone ------------------------------------------------------------------------------------
// Operates on log2 luminance relative to middle grey and rescales RGB by the luminance ratio,
// which preserves hue and saturation.
// `t`: contrast, highlights, shadows, whites; `blacks` (-1..1, larger where masks add up).
fn tone(rgb: vec3<f32>, t: vec4<f32>, blacks: f32) -> vec3<f32> {
    let y = scaling_luma(rgb);
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
// Rec.2020 linear ↔ OkLab, with the XYZ step folded into the LMS matrices. Colours outside
// Rec.2020 go through as they are (the cube root is signed); AgX brings them in.
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

// `saturation`, `vibrance`, `hue` (-1..1: ±30°): global plus what masks add here.
fn color(rgb: vec3<f32>, saturation: f32, vibrance: f32, hue: f32) -> vec3<f32> {
    let lab = to_oklab(rgb);
    var l = lab.x;
    var c = length(lab.yz);
    var h = degrees(atan2(lab.z, lab.y));
    h = (h + 360.0) % 360.0;

    // Colour mixer: fades out towards neutral, where hue is meaningless.
    let colourful = smoothstep(0.0, 0.05, c);
    let dh = band_value(params.mix_hue0, params.mix_hue1, h);
    let ds = band_value(params.mix_sat0, params.mix_sat1, h);
    let dl = band_value(params.mix_lum0, params.mix_lum1, h);
    h = h + 30.0 * (dh + hue) * colourful;
    c = c * max(0.0, 1.0 + ds * colourful);
    l = l * exp2(0.5 * dl * colourful);

    // Vibrance favours muted colours; saturation is uniform.
    c = c * max(0.0, 1.0 + vibrance * (1.0 - smoothstep(0.0, 0.2, c)));
    c = c * max(0.0, 1.0 + saturation);

    let hr = radians(h);
    return from_oklab(vec3<f32>(l, c * cos(hr), c * sin(hr)));
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
    // The LUT's own width (`LUT_SIZE`), so the two can't drift apart.
    let n = f32(textureDimensions(curve, 0).x);
    let t = (clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)) * (n - 1.0) + 0.5) / n;
    return vec3<f32>(
        textureSampleLevel(curve, curve_sampler, vec2<f32>(t.x, 0.5), 0.0).r,
        textureSampleLevel(curve, curve_sampler, vec2<f32>(t.y, 0.5), 0.0).g,
        textureSampleLevel(curve, curve_sampler, vec2<f32>(t.z, 0.5), 0.0).b,
    );
}

// ---- Display transform: AgX Base (docs/agx.md; `tonality_core::agx::Agx::display`) -----------
fn agx_mat(i: i32, c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(dot(params.agx[i].xyz, c), dot(params.agx[i + 1].xyz, c), dot(params.agx[i + 2].xyz, c));
}

fn max3(c: vec3<f32>) -> f32 {
    return max(c.x, max(c.y, c.z));
}

fn min3(c: vec3<f32>) -> f32 {
    return min(c.x, min(c.y, c.z));
}

// `max(x) − Y(max(x) − x) + Y(x)`: luminance that counts what a negative component takes away.
fn agx_compensated(x: vec3<f32>, w: vec3<f32>) -> f32 {
    let inv = vec3<f32>(max3(x)) - x;
    return max3(inv) - dot(inv, w) + dot(x, w);
}

fn agx_lift(c: vec3<f32>) -> vec3<f32> {
    return c + vec3<f32>(max(-min3(c), 0.0));
}

// Lower guard rail in Rec.2020: lift negatives, keep the compensated luminance.
fn agx_low_side(c: vec3<f32>) -> vec3<f32> {
    let w = params.agx[12].xyz;
    let y = agx_compensated(c, w);
    let lifted = agx_lift(c);
    let y_new = agx_compensated(lifted, w);
    return lifted * select(1.0, y / y_new, y_new > y);
}

// The display gamut's guard rail: compensation blended in by min(Y^0.08, 1). A negative
// luminance leaves the colour unscaled. Returns -1 for that.
fn agx_display_y(x: vec3<f32>) -> f32 {
    let w = params.agx[13].xyz;
    let y = dot(x, w);
    if (y < 0.0) {
        return -1.0;
    }
    let t = min(pow(y, 0.08), 1.0);
    return (1.0 - t) * agx_compensated(x, w) + t * y;
}

fn agx_low_side_display(c: vec3<f32>) -> vec3<f32> {
    let y = agx_display_y(c);
    let lifted = agx_lift(c);
    let y_new = agx_display_y(lifted);
    let scale = y >= 0.0 && y_new >= 0.0 && y_new > y;
    return lifted * select(1.0, y / max(y_new, 1e-30), scale);
}

// HSV hue (hexcone), 0..1; 0 for greys.
fn hsv_hue(c: vec3<f32>) -> f32 {
    let mx = max3(c);
    let d = mx - min3(c);
    if (d <= 0.0) {
        return 0.0;
    }
    var h: f32;
    if (mx == c.x) {
        h = (c.y - c.z) / d;
        h = h - 6.0 * floor(h / 6.0);
    } else if (mx == c.y) {
        h = (c.z - c.x) / d + 2.0;
    } else {
        h = (c.x - c.y) / d + 4.0;
    }
    return h / 6.0;
}

// `c` with its HSV hue set to `h`, saturation and value kept.
fn with_hsv_hue(c: vec3<f32>, h: f32) -> vec3<f32> {
    let v = max3(c);
    if (v <= 0.0) {
        return c;
    }
    let s = (v - min3(c)) / v;
    let k = clamp(abs(fract(vec3<f32>(h) + vec3<f32>(0.0, 2.0 / 3.0, 1.0 / 3.0)) * 6.0 - 3.0) - 1.0, vec3<f32>(0.0), vec3<f32>(1.0));
    return v * (1.0 - s + s * k);
}

// Jed Smith's tunable sigmoid through the pivot (slope 2.4, toe and shoulder power 1.5).
fn agx_sigmoid(x: vec3<f32>) -> vec3<f32> {
    let k = params.agx[14];
    let s = select(vec3<f32>(k.w), vec3<f32>(k.z), x < vec3<f32>(k.x));
    let u = 2.4 * (x - vec3<f32>(k.x)) / s; // ≥ 0 on both sides (the toe's scale is negative)
    return s * u / pow(1.0 + pow(u, vec3<f32>(1.5)), vec3<f32>(1.0 / 1.5)) + vec3<f32>(k.y);
}

// Scene-linear Rec.2020 → linear display Rec.709, clipped to 0..1 (encode with srgb_encode).
fn agx(rgb: vec3<f32>) -> vec3<f32> {
    // 1. Input domain: E-Gamut, −10…+15 stops around middle grey.
    var c = agx_mat(3, clamp(agx_mat(0, rgb), vec3<f32>(params.agx[12].w), vec3<f32>(params.agx[13].w)));
    // 2. Guard rail, 3. inset, 4. its hue.
    c = agx_mat(6, agx_low_side(c));
    let h0 = hsv_hue(c);
    // 5.–7. Log2 over −10…+6.5 stops, sigmoid, linearise.
    let x = (log2(max(c, vec3<f32>(1e-30)) / 0.18) + 10.0) / 16.5;
    c = pow(max(agx_sigmoid(x), vec3<f32>(0.0)), vec3<f32>(2.4));
    // 8. Keep 40% of the sigmoid's hue shift.
    var h = hsv_hue(c);
    let d = h - h0;
    h = h - select(0.0, 1.0, d > 0.5) + select(0.0, 1.0, d < -0.5);
    h = fract(h0 + 0.4 * (h - h0) + 1.0);
    c = with_hsv_hue(c, h);
    // 9.–11. Outset, to the display's primaries, guard rail. 12. Clip.
    c = agx_low_side_display(agx_mat(9, c));
    return clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
}

fn srgb_encode(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

// ---- Masks -------------------------------------------------------------------------------------
// Display luminance of camera RGB `cam`: what masks' luminance ranges compare against.
fn display_lum(cam: vec3<f32>) -> f32 {
    let rgb = vec3<f32>(dot(params.cam0.xyz, cam), dot(params.cam1.xyz, cam), dot(params.cam2.xyz, cam)) * params.light0.x;
    return srgb_encode(agx(vec3<f32>(max(dot(rgb, REC2020_LUMA), 0.0)))).y;
}

// A colour of hue `h` (radians, OkLCh) and unit luminance: a mask's tint multiplies by it.
fn tint_color(h: f32) -> vec3<f32> {
    let c = max(from_oklab(vec3<f32>(0.75, 0.08 * cos(h), 0.08 * sin(h))), vec3<f32>(1e-4));
    return c / dot(c, REC2020_LUMA);
}

// What masks add to sharpening (x) and noise reduction (y) at source position `uv`.
fn local_detail(tc: vec2<f32>, uv: vec2<f32>) -> vec2<f32> {
    let lum = display_lum(cam_at(tc));
    var out = vec2<f32>(0.0);
    let n = u32(params.masks_head.x);
    for (var i = 0u; i < n; i = i + 1u) {
        let m = mask_value(i, uv, lum);
        if (m > 0.0) {
            out = out + m * vec2<f32>(params.masks[i].adj2.w, params.masks[i].adj3.x);
        }
    }
    return out;
}

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
// ---- Presence: Texture, Clarity, Dehaze ---------------------------------------------------------
// They compare a pixel with its surroundings in the photo as shot (camera RGB, as-shot white
// balance, no edits), so what counts as detail or haze doesn't move as other sliders do; the
// result is applied to the developed colour as a gain (texture, clarity) or a haze model (dehaze).

fn shot_luminance(tc: vec2<f32>) -> f32 {
    let raw = textureSampleLevel(image, image_sampler, tc, 0.0).rgb;
    return dot(params.presence[0].xyz, clamp(raw * params.presence[1].xyz, vec3<f32>(0.0), vec3<f32>(params.presence[0].w)));
}

fn shot_log(tc: vec2<f32>) -> f32 {
    return log2(max(shot_luminance(tc), 1e-5));
}

// The haze colour, developed with the current white balance and exposure.
fn airlight() -> vec3<f32> {
    let a = clamp(params.presence[2].xyz * params.wb.xyz, vec3<f32>(0.0), vec3<f32>(params.wb.w));
    return max(vec3<f32>(dot(params.cam0.xyz, a), dot(params.cam1.xyz, a), dot(params.cam2.xyz, a)), vec3<f32>(0.0)) * params.light0.x;
}

// `amount`: x texture, y clarity, z dehaze (-1..1). `tc`, `px`: the image texture coordinate and
// texture units per full-resolution pixel; `src`: source uv (where the maps are sampled).
fn presence(rgb: vec3<f32>, tc: vec2<f32>, px: vec2<f32>, src: vec2<f32>, amount: vec3<f32>) -> vec3<f32> {
    var c = rgb;
    let maps = params.presence[2].w > 0.5;
    let l0 = shot_luminance(tc);
    if (maps && (amount.y != 0.0 || amount.z != 0.0)) {
        let m = textureSampleLevel(presence_map, image_sampler, src, 0.0);
        if (amount.z != 0.0) {
            // Transmission: how much of the scene (rather than haze) reaches the camera here.
            let t = clamp(m.z * l0 / max(params.presence[1].w, 1e-6) + m.w, 0.0, 1.0);
            let a = airlight();
            if (amount.z > 0.0) {
                let te = max(1.0 - amount.z * (1.0 - t), 0.1);
                // Floored so an over-corrected component stops at black; one that was already
                // below it (a colour outside Rec.2020) scales with the rest of the signal.
                c = max((c - a) / te + a, min(c, vec3<f32>(0.0)) / te);
            } else {
                c = mix(c, a, -amount.z * (0.08 + 0.4 * (1.0 - t)));
            }
        }
        if (amount.y != 0.0) {
            // Local contrast (stops) against the edge-aware base, limited so strong edges
            // don't overshoot, weighted toward the mid-tones.
            let l = log2(max(l0, 1e-5));
            let d = l - (m.x * l + m.y);
            let dl = d / (1.0 + 0.5 * abs(d));
            let e = log2(max(dot(c, REC2020_LUMA), 1e-6) / 0.18);
            let mid = exp(-e * e / 12.5);
            let k = select(0.9 * amount.y, 0.8 * amount.y, amount.y > 0.0);
            c = c * exp2(k * dl * mid);
        }
    }
    if (amount.x != 0.0) {
        // Band-pass detail: the pixel and a 2 px ring against a 6 px ring (full-resolution
        // pixels), in stops, limited so edges stay put.
        let l = log2(max(l0, 1e-5));
        var inner = l;
        var outer = 0.0;
        for (var k = 0; k < 8; k = k + 1) {
            let a = f32(k) * 0.7853982;
            let dir = vec2<f32>(cos(a), sin(a));
            if (k % 2 == 0) {
                inner = inner + shot_log(tc + dir * 2.0 * px);
            }
            outer = outer + shot_log(tc + dir * 6.0 * px);
        }
        var band = inner / 5.0 - outer / 8.0;
        if (amount.x > 0.0) {
            // Coring: detail smaller than the noise (in stops, larger in the shadows, as shot
            // noise goes) isn't boosted.
            let core = 0.03 * sqrt(0.18 / max(l0, 2e-3));
            band = sign(band) * max(abs(band) - core, 0.0);
        }
        let bl = band / (1.0 + 2.0 * abs(band));
        let k = select(amount.x, 1.6 * amount.x, amount.x > 0.0);
        c = c * exp2(k * bl);
    }
    return c;
}

fn develop(cam: vec3<f32>, f: vec2<f32>, uv: vec2<f32>, aspect: f32, tc: vec2<f32>, px: vec2<f32>, src: vec2<f32>) -> vec3<f32> {
    // Rec.2020, unclipped: saturated colours the camera saw outside it go negative here and stay
    // so until AgX, whose guard rail brings them in smoothly.
    var rgb = vec3<f32>(dot(params.cam0.xyz, cam), dot(params.cam1.xyz, cam), dot(params.cam2.xyz, cam)) * params.light0.x;

    // Local adjustments: each mask adds its settings, weighted by how much it selects here, to
    // the global ones; then the pipeline runs once.
    var t = vec4<f32>(params.light0.yzw, params.light1.x);
    var blacks = params.light1.y;
    var saturation = params.color0.x;
    var vibrance = params.color0.y;
    var hue = 0.0;
    var pres = params.presence[3].xyz;
    let n = u32(params.masks_head.x);
    if (n > 0u) {
        let lum = display_lum(cam);
        var ev = 0.0;
        var wb = vec2<f32>(0.0);
        var tint = vec3<f32>(1.0);
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
            vibrance = vibrance + m * mk.adj2.y;
            hue = hue + m * mk.adj2.z;
            pres = pres + m * vec3<f32>(mk.adj3.w, mk.adj4.xy);
            if (mk.adj3.z > 0.0) {
                tint = tint * mix(vec3<f32>(1.0), tint_color(mk.adj3.y), m * mk.adj3.z);
            }
        }
        rgb = rgb * exp2(ev) * white_shift(wb.x, wb.y) * tint;
    }

    pres = clamp(pres, vec3<f32>(-1.0), vec3<f32>(1.0));
    if (any(pres != vec3<f32>(0.0))) {
        rgb = presence(rgb, tc, px, src, pres);
    }
    rgb = vignette(rgb, f);
    rgb = tone(rgb, t, blacks);
    rgb = color(rgb, saturation, vibrance, hue);
    var c = srgb_encode(agx(rgb));
    c = apply_curve(c);
    c = grain(c, f, aspect);
    return clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
}

// Preview only: how much mask `which - 1` (its shader index) selects here.
fn overlay_mask(uv: vec2<f32>, cam: vec3<f32>, which: f32) -> f32 {
    return clamp(mask_value(u32(which - 1.0), uv, display_lum(cam)), 0.0, 1.0);
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
    var local = vec2<f32>(0.0);
    if (params.masks_head.w > 0.5) {
        local = local_detail(tc, uv);
    }
    // Black clips here, after noise reduction has averaged the noise around it.
    var cam = max(detail(tc, in.px.xy, local), vec3<f32>(0.0));
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
    if (in.overlay.x > 0.5 && in.overlay.y > 0.5) {
        // The mask alone (mask thumbnails): white where it applies, black where it doesn't.
        return vec4<f32>(vec3<f32>(overlay_mask(uv, cam, in.overlay.x)), 1.0);
    }
    var c = develop(cam, in.f, uv, params.geo0.w / params.geo1.w, tc, in.px.xy, src);
    if (in.overlay.x > 0.5) {
        c = mix(c, vec3<f32>(0.95, 0.08, 0.08), 0.5 * overlay_mask(uv, cam, in.overlay.x));
    }
    return vec4<f32>(c, 1.0);
}
