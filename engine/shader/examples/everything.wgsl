// What the writer covers, in one filter: the example proves libobs builds it.
@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var scene_sampler: sampler;
struct Remux { time: f32, resolution: vec2<f32> }
@group(0) @binding(2) var<uniform> remux: Remux;

const WEIGHTS = vec3<f32>(0.299, 0.587, 0.114);

fn wobble(y: f32) -> f32 {
    return sin(y * 40.0 + remux.time * 3.0) * 0.01;
}

fn darken(g: ptr<function, f32>, by: f32) {
    *g = *g * by;
}

@fragment
fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let p = uv + vec2<f32>(wobble(uv.y), 0.0);
    let c = textureSample(scene, scene_sampler, p);
    let cells = floor(uv * vec2<f32>(textureDimensions(scene)) / 8.0);
    var g = dot(c.rgb, WEIGHTS);
    for (var i = 0; i < 3; i++) {
        if ((cells.x + f32(i)) % 2.0 < 1.0) {
            darken(&g, 0.9);
        }
    }
    let m = mat2x2<f32>(1.0, 0.0, 0.0, 1.0);
    let q = m * (p - vec2<f32>(0.5)) + vec2<f32>(0.5);
    let edge = select(0.0, 1.0, length(q - vec2<f32>(0.5)) > 0.45);
    return vec4<f32>(mix(c.rgb, vec3<f32>(g), 0.5) * (1.0 - edge * 0.5), c.a);
}
