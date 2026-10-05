// The develop shader: turns the working image into what you see.
//
// It runs in two halves. The first works in linear light, where exposure,
// white balance and local contrast behave like they do in a camera. The
// second works on display-encoded values, where curves and colour tweaks
// behave like they look.

struct Params {
    // The part of the frame being drawn: frame = view.xy + uv * view.zw.
    // The frame is the cropped picture.
    view: vec4f,
    // Two rows of the matrix taking a frame position to a position in the
    // photo, undoing crop, straightening, flips and quarter-turns.
    to_source: array<vec4f, 2>,
    // width, height, source pixels per output pixel, 1 if scene-referred (RAW).
    image: vec4f,
    // exposure (EV), contrast, highlights, shadows
    light: vec4f,
    // whites, blacks, temperature, tint
    tone: vec4f,
    // vibrance, saturation, clarity, dehaze
    color: vec4f,
    // sharpening, noise reduction, vignette, grain
    detail: vec4f,
    // show clipping, leave outside the photo transparent, unused…
    flags: vec4f,
    // Per colour band: hue shift, saturation, luminance, unused.
    mixer: array<vec4f, 8>,
}

@group(0) @binding(0) var source: texture_2d<f32>;
// The source blurred at two radii, for local contrast and tonal masks.
@group(0) @binding(1) var blur_medium: texture_2d<f32>;
@group(0) @binding(2) var blur_large: texture_2d<f32>;
// 256 wide: rgb = per-channel curves, a = the master curve.
@group(0) @binding(3) var curves: texture_2d<f32>;
@group(0) @binding(4) var linear_sampler: sampler;
@group(0) @binding(5) var<uniform> p: Params;

struct VertexOutput {
    @builtin(position) position: vec4f,
    @location(0) uv: vec2f,
}

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> VertexOutput {
    // One triangle that covers the whole target.
    let corner = vec2f(f32((index << 1u) & 2u), f32(index & 2u));
    var out: VertexOutput;
    out.position = vec4f(corner * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2f(corner.x, 1.0 - corner.y);
    return out;
}

const LUMA = vec3f(0.2126, 0.7152, 0.0722);
const MID_GRAY = 0.18;

// The built-in look for RAW files. Exposure and saturation were fitted so the
// brightness distribution and colourfulness of unedited photos match the
// cameras' own JPEGs (Canon, Nikon and Sony samples).
const BASE_EXPOSURE = 1.4;
const BASE_SATURATION = 1.05;
const BASE_SHARPENING = 0.25;

fn luma(c: vec3f) -> f32 {
    return dot(c, LUMA);
}

// Squeezes scene luminance into 0..1 with mid gray at 0.5, for building masks.
fn tonal_position(l: f32) -> f32 {
    return l / (l + MID_GRAY);
}

fn srgb_encode(c: vec3f) -> vec3f {
    let low = c * 12.92;
    let high = 1.055 * pow(max(c, vec3f(0.0)), vec3f(1.0 / 2.4)) - 0.055;
    return select(high, low, c <= vec3f(0.0031308));
}

// The camera-like tone curve: a filmic shoulder that rolls highlights off
// instead of clipping them, applied per channel so bright colours desaturate
// the way they do in camera JPEGs.
fn base_curve(x: vec3f) -> vec3f {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return (x * (a * x + b)) / (x * (c * x + d) + e);
}

// Scene values to display-linear for RAW files: the sensor's clipping point
// lands exactly on white.
fn base_look(c: vec3f) -> vec3f {
    let toned = base_curve(c * BASE_EXPOSURE) / base_curve(vec3f(BASE_EXPOSURE));
    let limited = clamp(toned, vec3f(0.0), vec3f(1.0));
    return mix(vec3f(luma(limited)), limited, BASE_SATURATION);
}

fn rgb_to_hsv(c: vec3f) -> vec3f {
    let k = vec4f(0.0, -1.0 / 3.0, 2.0 / 3.0, -1.0);
    let q = mix(vec4f(c.bg, k.wz), vec4f(c.gb, k.xy), step(c.b, c.g));
    let r = mix(vec4f(q.xyw, c.r), vec4f(c.r, q.yzx), step(q.x, c.r));
    let d = r.x - min(r.w, r.y);
    let e = 1e-10;
    return vec3f(abs(r.z + (r.w - r.y) / (6.0 * d + e)), d / (r.x + e), r.x);
}

fn hsv_to_rgb(c: vec3f) -> vec3f {
    let k = vec4f(1.0, 2.0 / 3.0, 1.0 / 3.0, 3.0);
    let q = abs(fract(c.xxx + k.xyz) * 6.0 - k.www);
    return c.z * mix(k.xxx, clamp(q - k.xxx, vec3f(0.0), vec3f(1.0)), c.y);
}

fn hash(cell: vec2f) -> f32 {
    var q = fract(vec3f(cell.xyx) * vec3f(0.1031, 0.1030, 0.0973));
    q += dot(q, q.yzx + 33.33);
    return fract((q.x + q.y) * q.z);
}

fn read_source(uv: vec2f, lod: f32) -> vec3f {
    return max(textureSampleLevel(source, linear_sampler, uv, lod).rgb, vec3f(0.0));
}

// Edge-preserving smoothing: neighbours count for less the more their
// brightness differs from the centre's.
fn denoise(uv: vec2f, lod: f32, texel: vec2f, center: vec3f, amount: f32) -> vec3f {
    let center_log = log2(luma(center) + 0.004);
    let tolerance = 0.08 + amount * 0.9;
    var sum = center;
    var weight = 1.0;
    for (var y = -2; y <= 2; y++) {
        for (var x = -2; x <= 2; x++) {
            if (x == 0 && y == 0) {
                continue;
            }
            let offset = vec2f(f32(x), f32(y));
            let tap = read_source(uv + offset * texel, lod);
            let difference = (log2(luma(tap) + 0.004) - center_log) / tolerance;
            let w = exp(-dot(offset, offset) / 4.5 - difference * difference);
            sum += tap * w;
            weight += w;
        }
    }
    return mix(center, sum / weight, min(1.0, amount * 1.5));
}

// How much each of the eight colour bands applies to a hue (0..1).
// Neighbouring bands cross-fade, so the weights always add up to one.
fn mix_bands(hue: f32) -> vec3f {
    // red, orange, yellow, green, aqua, blue, purple, magenta, then red again.
    var centers = array<f32, 9>(0.0, 30.0, 60.0, 120.0, 180.0, 240.0, 280.0, 320.0, 360.0);
    let degrees = hue * 360.0;
    var result = vec3f(0.0);
    for (var i = 0; i < 8; i++) {
        let lower = centers[i];
        let upper = centers[i + 1];
        if (degrees >= lower && degrees <= upper) {
            let t = (degrees - lower) / (upper - lower);
            result = mix(p.mixer[i].xyz, p.mixer[(i + 1) % 8].xyz, t);
        }
    }
    return result;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4f {
    let frame_uv = p.view.xy + in.uv * p.view.zw;
    let uv = vec2f(dot(p.to_source[0].xyz, vec3f(frame_uv, 1.0)), dot(p.to_source[1].xyz, vec3f(frame_uv, 1.0)));
    if (p.flags.y > 0.5 && (min(uv.x, uv.y) < 0.0 || max(uv.x, uv.y) > 1.0)) {
        // The crop tool shows the whole tilted photo; its corners are empty.
        return vec4f(0.0);
    }
    let size = p.image.xy;
    let scale = max(p.image.z, 1.0);
    let scene_referred = p.image.w > 0.5;
    // Read from the mip level that matches the zoom, a touch on the sharp side.
    let lod = max(log2(scale) - 0.25, 0.0);
    let texel = exp2(lod) / size;
    // Fine detail tools fade as you zoom out, as their effect would in a
    // full-size render scaled down.
    let fine = inverseSqrt(scale);

    // ---- linear light ----

    var c = read_source(uv, lod);
    if (scene_referred) {
        // Where one sensor channel has clipped the colour can't be trusted;
        // fade it to neutral so blown highlights come out white, not pink.
        let peak = max(c.r, max(c.g, c.b));
        c = mix(c, vec3f(peak), smoothstep(0.82, 1.0, peak));
    }
    let noise_reduction = p.detail.y;
    if (noise_reduction > 0.0) {
        c = denoise(uv, lod, texel, c, noise_reduction * fine);
    }

    var sharpening = p.detail.x * 1.6;
    if (scene_referred) {
        sharpening += BASE_SHARPENING;
    }
    if (sharpening > 0.0) {
        let around = (read_source(uv + vec2f(texel.x, 0.0), lod) + read_source(uv - vec2f(texel.x, 0.0), lod)
            + read_source(uv + vec2f(0.0, texel.y), lod) + read_source(uv - vec2f(0.0, texel.y), lod)) * 0.25;
        let ratio = clamp((luma(c) + 0.002) / (luma(around) + 0.002), 0.5, 2.0);
        c *= pow(ratio, sharpening * fine);
    }

    // White balance: temperature trades red against blue, tint green against
    // magenta, keeping overall brightness where it was.
    var gains = vec3f(exp2(p.tone.z * 0.6), exp2(-p.tone.w * 0.35), exp2(-p.tone.z * 0.6));
    gains /= luma(gains);
    let gain = gains * exp2(p.light.x);
    c *= gain;
    let medium = max(textureSampleLevel(blur_medium, linear_sampler, uv, 0.0).rgb, vec3f(0.0)) * gain;
    let large = max(textureSampleLevel(blur_large, linear_sampler, uv, 0.0).rgb, vec3f(0.0)) * gain;

    // Dehaze: haze is a veil of light, strongest where even the darkest
    // channel of the neighbourhood is bright. Lift it off (or lay it on).
    let dehaze = p.color.w;
    if (dehaze != 0.0) {
        var haze = min(min(large.r, min(large.g, large.b)), 0.9);
        if (dehaze > 0.0) {
            // A pixel can't carry more veil than its own darkest channel;
            // without this limit, dark detail in a bright area turns black.
            haze = min(haze, min(c.r, min(c.g, c.b)));
        }
        let veil = dehaze * 0.6 * haze;
        c = max((c - veil) / (1.0 - veil), vec3f(0.0));
    }

    // Clarity: exaggerate (or soften) how each pixel differs from its
    // surroundings, mostly in the midtones.
    let clarity = p.color.z;
    if (clarity != 0.0) {
        let difference = clamp(log2((luma(c) + 0.003) / (luma(medium) + 0.003)), -2.0, 2.0);
        let position = tonal_position(luma(c));
        let midtones = 1.0 - pow(abs(2.0 * position - 1.0), 2.0);
        c *= exp2(clarity * 0.9 * difference * midtones);
    }

    // Shadows and highlights follow the blurred image, so they brighten or
    // darken whole regions and leave the detail inside them alone.
    let region = tonal_position(mix(luma(large), luma(c), 0.25));
    let shadow_weight = 1.0 - smoothstep(0.0, 0.55, region);
    let highlight_weight = smoothstep(0.45, 1.0, region);
    c *= exp2(p.light.w * 1.8 * shadow_weight + p.light.z * 1.8 * highlight_weight);

    // Contrast pivots on mid gray, so it changes spread without changing exposure.
    c = MID_GRAY * pow(c / MID_GRAY, vec3f(1.0 + p.light.y * 0.5));

    // ---- to the display ----

    var v: vec3f;
    if (scene_referred) {
        v = srgb_encode(base_look(c));
    } else {
        v = srgb_encode(clamp(c, vec3f(0.0), vec3f(1.0)));
    }
    v = clamp(v, vec3f(0.0), vec3f(1.0));

    // Whites and blacks move the two ends of the range.
    v += p.tone.x * 0.25 * v * v;
    v += p.tone.y * 0.25 * (1.0 - v) * (1.0 - v);
    v = clamp(v, vec3f(0.0), vec3f(1.0));

    // Tone curves: the master curve first, then each channel's own.
    let lut = vec2f(255.0 / 256.0, 0.5 / 256.0);
    let m = vec3f(
        textureSampleLevel(curves, linear_sampler, vec2f(v.r * lut.x + lut.y, 0.5), 0.0).a,
        textureSampleLevel(curves, linear_sampler, vec2f(v.g * lut.x + lut.y, 0.5), 0.0).a,
        textureSampleLevel(curves, linear_sampler, vec2f(v.b * lut.x + lut.y, 0.5), 0.0).a,
    );
    v = vec3f(
        textureSampleLevel(curves, linear_sampler, vec2f(m.r * lut.x + lut.y, 0.5), 0.0).r,
        textureSampleLevel(curves, linear_sampler, vec2f(m.g * lut.x + lut.y, 0.5), 0.0).g,
        textureSampleLevel(curves, linear_sampler, vec2f(m.b * lut.x + lut.y, 0.5), 0.0).b,
    );
    v = clamp(v, vec3f(0.0), vec3f(1.0));

    // Colour mixer: shift, saturate or brighten one range of hues. Greys have
    // no hue to speak of, so the effect fades out as colour does.
    var hsv = rgb_to_hsv(v);
    let band = mix_bands(hsv.x);
    let colourful = smoothstep(0.02, 0.25, hsv.y);
    hsv.x = fract(hsv.x + band.x * (30.0 / 360.0) * colourful + 1.0);
    v = hsv_to_rgb(hsv);
    // Saturation moves towards or away from the grey of equal brightness, so
    // muting a colour doesn't also lighten it.
    v = mix(vec3f(luma(v)), v, 1.0 + band.y * colourful);
    v *= exp2(band.z * 0.8 * colourful);

    // Vibrance favours muted colours; saturation treats all alike.
    let gray = luma(v);
    let saturation_now = rgb_to_hsv(clamp(v, vec3f(0.0), vec3f(1.0))).y;
    let boost = (1.0 + p.color.y) * (1.0 + p.color.x * (1.0 - saturation_now));
    v = mix(vec3f(gray), v, boost);

    // Vignette: darken or lighten towards the corners.
    // It follows the crop, so it frames the picture you end up with.
    let from_center = length((frame_uv - 0.5) * 1.41421356);
    let edge = smoothstep(0.25, 1.0, from_center);
    v *= 1.0 + p.detail.z * 0.85 * edge * edge;

    // Grain is tied to image pixels, so it stays put as you pan and zoom.
    let grain = p.detail.w;
    if (grain > 0.0) {
        let cell = floor(uv * size / max(1.6, scale));
        let noise = hash(cell) + hash(cell + 17.0) - 1.0;
        let level = clamp(luma(v), 0.0, 1.0);
        v += noise * grain * 0.16 * (0.35 + 2.6 * level * (1.0 - level)) / sqrt(max(1.0, scale / 1.6));
    }

    v = clamp(v, vec3f(0.0), vec3f(1.0));

    // Clipping warnings: blown highlights in red, crushed shadows in blue.
    if (p.flags.x > 0.5) {
        if (max(v.r, max(v.g, v.b)) >= 0.998) {
            v = vec3f(1.0, 0.1, 0.1);
        } else if (max(v.r, max(v.g, v.b)) <= 0.004) {
            v = vec3f(0.15, 0.35, 1.0);
        }
    }
    return vec4f(v, 1.0);
}
