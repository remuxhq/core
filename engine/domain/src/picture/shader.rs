//! Picture filters: the port a motor compiles an operator's filter through,
//! and the contract a filter keeps. The compiler is an adapter the daemon
//! injects into a motor (`remux-shader`, over naga); the domain carries only
//! a filter's path, and never compiles one.

/// What a filter is, said to the person whose file is refused.
pub const CONTRACT: &str = "a filter is one @fragment function taking @location(0) uv: vec2<f32> and returning @location(0) vec4<f32>, with a texture_2d<f32> at @group(0) @binding(0), its sampler at @binding(1), and optionally a var<uniform> of { time: f32, resolution: vec2<f32> } at @binding(2)";

/// Compiles an operator's WGSL file into what a motor runs.
pub trait ShaderCompiler: Send + Sync {
    /// The file at `path`, compiled, or the reason there is none.
    fn compile(&self, path: &str) -> Result<String, String>;
}
