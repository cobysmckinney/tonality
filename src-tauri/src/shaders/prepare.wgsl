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
