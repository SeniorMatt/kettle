// egui's shader for the Vulkan layer. Positions arrive already in clip space (the CPU converts them), so there
// is no uniform. Colors are premultiplied sRGB-encoded; a linear (sRGB format) target gets them converted.

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_main(@location(0) pos: vec2<f32>, @location(1) uv: vec2<f32>, @location(2) color: vec4<f32>) -> VOut {
    var o: VOut;
    o.pos = vec4<f32>(pos, 0.0, 1.0);
    o.uv = uv;
    o.color = color;
    return o;
}

@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

@fragment
fn fs_gamma(in: VOut) -> @location(0) vec4<f32> {
    return in.color * textureSample(tex, samp, in.uv);
}

@fragment
fn fs_linear(in: VOut) -> @location(0) vec4<f32> {
    let c = in.color * textureSample(tex, samp, in.uv);
    return vec4<f32>(to_linear(c.rgb), c.a);
}
