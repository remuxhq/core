//! The domain's `ShaderCompiler` over naga: a WGSL file, checked against the
//! contract the domain states, as an OBS effect. The daemon injects [`Naga`]
//! into a motor; another compiler is another crate implementing the same port.
//!
//! A filter is one `@fragment` function that takes `@location(0) uv:
//! vec2<f32>`, (0, 0) at the top left, and returns `@location(0) vec4<f32>`;
//! the picture is a `texture_2d<f32>` at `@group(0) @binding(0)` and its
//! sampler at `@binding(1)`; `@binding(2)` may hold a `var<uniform>` of
//! `{ time: f32, resolution: vec2<f32> }`. naga reads and validates it
//! (`contract`), and what it read is written out as an effect libobs builds
//! (`writer`), whose `time` and `resolution` uniforms the motor sets.
//!
//! Everything about compiling a filter is in this crate: the motor hands the
//! port a path and gets back effect text or the reason there is none.

mod contract;
mod writer;

use remuxd_domain::picture::shader::ShaderCompiler;

/// The compiler over naga, as the domain's port.
pub struct Naga;

impl ShaderCompiler for Naga {
    fn compile(&self, path: &str) -> Result<String, String> {
        load(path)
    }
}

pub use contract::CONTRACT;

const MAX_SOURCE: u64 = 64 * 1024;

/// The file at `path`, read under the motors' limits, as an effect.
pub fn load(path: &str) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("cannot open filter: {e}"))?;
    let size = file
        .metadata()
        .map_err(|e| format!("cannot stat filter: {e}"))?
        .len();
    if size > MAX_SOURCE {
        return Err("filter exceeds 64 KiB".into());
    }
    use std::io::Read;
    let mut bytes = Vec::new();
    file.take(MAX_SOURCE + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("cannot read filter: {e}"))?;
    if bytes.len() as u64 > MAX_SOURCE {
        return Err("filter exceeds 64 KiB".into());
    }
    let source = String::from_utf8(bytes).map_err(|_| "filter must be UTF-8")?;
    translate(&source)
}

/// WGSL source to effect source.
pub fn translate(source: &str) -> Result<String, String> {
    let module = naga::front::wgsl::parse_str(source)
        .map_err(|error| format!("WGSL: {}", error.emit_to_string(source).trim()))?;
    contract::validate(&module)?;
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .map_err(|error| format!("WGSL validation: {error}"))?;
    writer::Writer::new(&module, &info).write()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::CONTRACT;

    const HEAD: &str = "@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var scene_sampler: sampler;
";

    #[test]
    fn the_invert_filter_becomes_an_effect_with_a_draw_technique() {
        let effect = translate(&format!(
            "{HEAD}@fragment
fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {{
    let pixel = textureSample(scene, scene_sampler, uv);
    return vec4<f32>(1.0 - pixel.rgb, pixel.a);
}}"
        ))
        .unwrap();
        assert!(effect.contains("technique Draw"), "{effect}");
        assert!(
            effect.contains("image.Sample(remux_sampler, v_in.uv)"),
            "{effect}"
        );
    }

    #[test]
    fn uniforms_helpers_loops_branches_and_the_size_are_written() {
        let effect = translate(&format!(
            "{HEAD}struct Remux {{ time: f32, resolution: vec2<f32> }}
@group(0) @binding(2) var<uniform> remux: Remux;
fn wobble(y: f32) -> f32 {{ return sin(y * 40.0 + remux.time * 3.0) * 0.01; }}
@fragment
fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {{
    let p = uv + vec2<f32>(wobble(uv.y), 0.0);
    let c = textureSample(scene, scene_sampler, p);
    let px = floor(uv * vec2<f32>(textureDimensions(scene)) / 8.0);
    var g = dot(c.rgb, vec3<f32>(0.299, 0.587, 0.114));
    for (var i = 0; i < 3; i++) {{ if ((px.x + f32(i)) % 2.0 < 1.0) {{ g *= 0.9; }} }}
    return vec4<f32>(mix(c.rgb, vec3<f32>(g), 0.5), c.a);
}}"
        ))
        .unwrap();
        for needle in [
            "time",
            "uint2(resolution)",
            "while (true)",
            "lerp(",
            "trunc(",
        ] {
            assert!(effect.contains(needle), "missing {needle}:\n{effect}");
        }
    }

    #[test]
    fn a_file_outside_the_contract_is_refused_with_the_contract() {
        let no_texture = translate(
            "@fragment fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> { return vec4<f32>(uv, 0.0, 1.0); }",
        );
        assert_eq!(no_texture.unwrap_err(), CONTRACT);
        let wrong_input = translate(&format!(
            "{HEAD}@fragment fn main(@location(1) uv: vec2<f32>) -> @location(0) vec4<f32> {{ return textureSample(scene, scene_sampler, uv); }}"
        ));
        assert_eq!(wrong_input.unwrap_err(), CONTRACT);
        assert!(translate("not wgsl").unwrap_err().starts_with("WGSL"));
        let array = translate(&format!(
            "{HEAD}@fragment fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {{ var w = array<f32, 2>(0.5, 0.5); return vec4<f32>(w[0]); }}"
        ));
        assert!(array.unwrap_err().contains("an array"));
    }
}
