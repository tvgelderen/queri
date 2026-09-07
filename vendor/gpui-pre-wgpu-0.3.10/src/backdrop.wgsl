struct Params {
    bounds: vec4<f32>,
    sigma: f32,
    opacity: f32,
    padding: vec2<f32>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var source: texture_2d<f32>;
@group(0) @binding(2) var blurred: texture_2d<f32>;
@group(0) @binding(3) var linear_sampler: sampler;

struct Vertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_fullscreen(@builtin(vertex_index) index: u32) -> Vertex {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return Vertex(vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0), uv);
}

@fragment
fn fs_downsample(vertex: Vertex) -> @location(0) vec4<f32> {
    // Four bilinear taps prefilter the 4x4 source footprint, avoiding aliasing
    // when fine text or grid lines are reduced before the Gaussian passes.
    let step = 1.0 / vec2<f32>(textureDimensions(source));
    return (
        textureSampleLevel(source, linear_sampler, vertex.uv + vec2<f32>(-step.x, -step.y), 0.0) +
        textureSampleLevel(source, linear_sampler, vertex.uv + vec2<f32>( step.x, -step.y), 0.0) +
        textureSampleLevel(source, linear_sampler, vertex.uv + vec2<f32>(-step.x,  step.y), 0.0) +
        textureSampleLevel(source, linear_sampler, vertex.uv + vec2<f32>( step.x,  step.y), 0.0)
    ) * 0.25;
}

fn gaussian(uv: vec2<f32>, direction: vec2<f32>) -> vec4<f32> {
    let sigma = max(params.sigma, 0.01);
    let radius = ceil(sigma * 3.0);
    let step = direction / vec2<f32>(textureDimensions(source));
    var color = vec4<f32>(0.0);
    var total = 0.0;
    for (var offset = -radius; offset <= radius; offset += 1.0) {
        let weight = exp(-0.5 * offset * offset / (sigma * sigma));
        color += textureSampleLevel(source, linear_sampler, uv + step * offset, 0.0) * weight;
        total += weight;
    }
    return color / total;
}

@fragment
fn fs_horizontal(vertex: Vertex) -> @location(0) vec4<f32> {
    return gaussian(vertex.uv, vec2<f32>(1.0, 0.0));
}

@fragment
fn fs_vertical(vertex: Vertex) -> @location(0) vec4<f32> {
    return gaussian(vertex.uv, vec2<f32>(0.0, 1.0));
}

@fragment
fn fs_composite(vertex: Vertex) -> @location(0) vec4<f32> {
    if any(vertex.position.xy < params.bounds.xy) || any(vertex.position.xy >= params.bounds.zw) {
        discard;
    }
    let original = textureSampleLevel(source, linear_sampler, vertex.uv, 0.0);
    let frost = textureSampleLevel(blurred, linear_sampler, vertex.uv, 0.0);
    return mix(original, frost, params.opacity);
}

@fragment
fn fs_copy(vertex: Vertex) -> @location(0) vec4<f32> {
    return textureSampleLevel(source, linear_sampler, vertex.uv, 0.0);
}
