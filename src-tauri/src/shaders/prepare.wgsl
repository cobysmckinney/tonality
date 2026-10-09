// Helpers that prepare a photo for editing: copying between sizes (for
// mipmaps and downsampled copies) and Gaussian blurring.

struct Blur {
    // One texel along the blur direction, in uv.
    step: vec2f,
    sigma: f32,
    radius: f32,
}

@group(0) @binding(0) var input: texture_2d<f32>;
@group(0) @binding(1) var linear_sampler: sampler;
@group(0) @binding(2) var<uniform> blur: Blur;

struct VertexOutput {
    @builtin(position) position: vec4f,
    @location(0) uv: vec2f,
}

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> VertexOutput {
    let corner = vec2f(f32((index << 1u) & 2u), f32(index & 2u));
    var out: VertexOutput;
    out.position = vec4f(corner * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2f(corner.x, 1.0 - corner.y);
    return out;
}

// Copies the input to the target, averaging when the target is smaller.
@fragment
fn copy(in: VertexOutput) -> @location(0) vec4f {
    return textureSample(input, linear_sampler, in.uv);
}

// One direction of a separable Gaussian blur.
@fragment
fn gaussian(in: VertexOutput) -> @location(0) vec4f {
    var sum = vec4f(0.0);
    var total = 0.0;
    let radius = i32(blur.radius);
    for (var i = -radius; i <= radius; i++) {
        let offset = f32(i);
        let weight = exp(-offset * offset / (2.0 * blur.sigma * blur.sigma));
        sum += textureSampleLevel(input, linear_sampler, in.uv + blur.step * offset, 0.0) * weight;
        total += weight;
    }
    return sum / total;
}

// A film negative's look, worked out by film.rs (`Look::uniform`).
struct Film {
    // The clear film's colour, as the scan holds it; then 1 for colour film, 2 for black and white.
    base: vec4f,
    // Each channel's density at the frame's thinnest point (its deepest shadow).
    low: vec4f,
    // How far each channel's density runs from there to the densest point.
    span: vec4f,
    // Where the densest point lands in scene light, and the scene's range in stops.
    out: vec4f,
}

@group(0) @binding(3) var<uniform> film: Film;

// Turns a scan of a negative into a positive in scene light, like a RAW's.
// `Look::invert` in film.rs is the same arithmetic.
@fragment
fn film_positive(in: VertexOutput) -> @location(0) vec4f {
    let scan = max(textureLoad(input, vec2i(in.position.xy), 0).rgb, vec3f(0.0));
    let darkest = 1e-5;
    // What the film lets through, relative to clear film.
    var through = max(scan / max(film.base.rgb, vec3f(darkest)), vec3f(darkest));
    if (film.base.w > 1.5) {
        // Black and white: one density, so a tint in the base or the light leaves no cast.
        through = vec3f(max((through.r + through.g + through.b) / 3.0, darkest));
    }
    let density = -log2(through) * 0.30102999566;
    let position = (density - film.low.rgb) / film.span.rgb;
    // Capped far past white, where the black of a holder would otherwise overflow.
    return vec4f(min(film.out.x * exp2(film.out.y * (position - 1.0)), vec3f(64.0)), 1.0);
}

// One spot of dust, a hair or a scratch to heal, laid out by heal.rs (`Packed`).
struct Spot {
    // Points in use, the radius, and how far away the patch is (x, y), all in photo pixels.
    info: vec4f,
    // How far in from the edge the spot fades, in pixels; the rest is spare.
    edge: vec4f,
    // The points in photo pixels, two to a row.
    points: array<vec4f, 32>,
}

@group(0) @binding(4) var<uniform> spot: Spot;

fn spot_point(i: i32) -> vec2f {
    let pair = spot.points[i / 2];
    return select(pair.xy, pair.zw, i % 2 == 1);
}

// How far `q` is from the spot's line of points.
fn spot_distance(q: vec2f) -> f32 {
    let count = i32(spot.info.x);
    var a = spot_point(0);
    var nearest = length(q - a);
    for (var i = 1; i < count; i++) {
        let b = spot_point(i);
        let ab = b - a;
        let t = clamp(dot(q - a, ab) / max(dot(ab, ab), 1e-6), 0.0, 1.0);
        nearest = min(nearest, length(q - a - ab * t));
        a = b;
    }
    return nearest;
}

// The working image at a pixel, with the spots before this one healed; the
// nearest pixel on the photo for one off its edge.
fn working(p: vec2f) -> vec3f {
    let last = vec2i(textureDimensions(input)) - 1;
    return textureLoad(input, clamp(vec2i(floor(p)), vec2i(0), last), 0).rgb;
}

// The working image around a point, `reach` pixels each way averaged in,
// so grain doesn't sway the blend.
fn around(p: vec2f, reach: f32) -> vec3f {
    let x = vec2f(reach, 0.0);
    let y = vec2f(0.0, reach);
    return (working(p) + working(p + x) + working(p - x) + working(p + y) + working(p - y)) / 5.0;
}

// The middle of the first `count` of `values`.
fn median(values: array<f32, 16>, count: i32) -> f32 {
    var sorted = values;
    for (var i = 1; i < count; i++) {
        let value = sorted[i];
        var j = i - 1;
        while (j >= 0 && sorted[j] > value) {
            sorted[j + 1] = sorted[j];
            j--;
        }
        sorted[j + 1] = value;
    }
    return select(0.0, sorted[count / 2], count > 0);
}

// Heals one spot, drawn over the working image (blended by the alpha it
// returns): the texture of the patch, with the brightness and colour of
// what surrounds the spot. The difference between the photo around the
// spot and around the patch is measured on the spot's edge, in every
// direction, and blended in from all of them, nearer ones counting more;
// so at the edge the fix meets its surroundings exactly. A direction whose
// difference is far from most of the others' (it found another speck, say)
// counts for less, down to nothing.
@fragment
fn heal(in: VertexOutput) -> @location(0) vec4f {
    let p = in.position.xy;
    let radius = spot.info.y;
    let d = spot_distance(p);
    if (d >= radius) {
        discard;
    }
    let offset = spot.info.zw;
    let reach = max(1.0, radius * 0.15);
    var differences: array<vec3f, 16>;
    var weights: array<f32, 16>;
    var shades: array<f32, 16>;
    var found = 0;
    for (var k = 0; k < 16; k++) {
        let angle = (f32(k) + 0.5) * 0.39269908;
        let way = vec2f(cos(angle), sin(angle));
        // Out along this way to the spot's edge. Along a line a way may
        // not leave it soon; the ways across it do.
        var t = 0.0;
        var out = false;
        for (var step = 0; step < 16; step++) {
            let inside = radius - spot_distance(p + way * t);
            if (inside <= 0.0) {
                out = true;
                break;
            }
            t += max(inside, 0.75);
        }
        if (!out) {
            continue;
        }
        let edge = p + way * (t + reach);
        let difference = around(edge, reach) - around(edge + offset, reach);
        differences[found] = difference;
        weights[found] = 1.0 / (t * t + 1.0);
        shades[found] = dot(difference, vec3f(0.2126, 0.7152, 0.0722));
        found++;
    }
    // The middle difference, and how far the differences typically stray from it.
    let middle = median(shades, found);
    var strays: array<f32, 16>;
    for (var i = 0; i < found; i++) {
        strays[i] = abs(shades[i] - middle);
    }
    let limit = 3.0 * median(strays, found) + 0.002;
    var sum = vec3f(0.0);
    var total = 0.0;
    for (var i = 0; i < found; i++) {
        let weight = weights[i] * (1.0 - smoothstep(limit, 2.0 * limit, abs(shades[i] - middle)));
        sum += differences[i] * weight;
        total += weight;
    }
    let lift = select(vec3f(0.0), sum / max(total, 1e-6), total > 0.0);
    let healed = max(working(p + offset) + lift, vec3f(0.0));
    return vec4f(healed, 1.0 - smoothstep(radius - spot.edge.x, radius, d));
}
