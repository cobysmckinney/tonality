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
    // Where the densest point lands in scene light, the scene's range in
    // stops, then where the highlights start to roll off and how much.
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
    let light = film.out.x * exp2(film.out.y * (position - 1.0));
    // Highlights rolled off above the knee: `over` stops over it come out
    // over / (1 + shoulder * over) stops over, as a print's paper does.
    let over = log2(max(light, vec3f(darkest)) / film.out.z);
    let rolled = film.out.z * exp2(over / (1.0 + film.out.w * max(over, vec3f(0.0))));
    // Capped far past white, where the black of a holder would otherwise overflow.
    return vec4f(min(select(light, rolled, over > vec3f(0.0)), vec3f(64.0)), 1.0);
}
