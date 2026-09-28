// The picture's colours inverted: `remux scene filter invert.wgsl`.
@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var scene_sampler: sampler;

@fragment
fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let pixel = textureSample(scene, scene_sampler, uv);
    return vec4<f32>(1.0 - pixel.rgb, pixel.a);
}
