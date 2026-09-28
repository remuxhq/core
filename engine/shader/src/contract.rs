//! The contract a filter has to meet, the same in both motors, checked on
//! what naga read.

use naga::{
    AddressSpace, Binding, ImageClass, ImageDimension, Module, ResourceBinding, ScalarKind,
    ShaderStage, TypeInner, VectorSize,
};

/// What a filter has to be, said when it is not: the domain's words.
pub use remuxd_domain::picture::shader::CONTRACT;

fn float(inner: &TypeInner, size: Option<VectorSize>) -> bool {
    match (inner, size) {
        (TypeInner::Vector { size, scalar }, Some(want)) => {
            *size == want && scalar.kind == ScalarKind::Float && scalar.width == 4
        }
        (TypeInner::Scalar(scalar), None) => scalar.kind == ScalarKind::Float && scalar.width == 4,
        _ => false,
    }
}

/// The shared contract, checked on what naga read.
pub(crate) fn validate(module: &Module) -> Result<(), String> {
    let invalid = || CONTRACT.to_string();
    let [entry] = module.entry_points.as_slice() else {
        return Err(invalid());
    };
    if entry.stage != ShaderStage::Fragment {
        return Err(invalid());
    }
    let [input] = entry.function.arguments.as_slice() else {
        return Err(invalid());
    };
    if !matches!(input.binding, Some(Binding::Location { location: 0, .. }))
        || !float(&module.types[input.ty].inner, Some(VectorSize::Bi))
    {
        return Err(invalid());
    }
    let Some(output) = &entry.function.result else {
        return Err(invalid());
    };
    if !matches!(output.binding, Some(Binding::Location { location: 0, .. }))
        || !float(&module.types[output.ty].inner, Some(VectorSize::Quad))
    {
        return Err(invalid());
    }
    let mut seen = [false; 3]; // texture, sampler, optional uniforms
    for (_, global) in module.global_variables.iter() {
        let index = match (
            global.space,
            global.binding.as_ref(),
            &module.types[global.ty].inner,
        ) {
            (
                AddressSpace::Handle,
                Some(ResourceBinding {
                    group: 0,
                    binding: 0,
                }),
                TypeInner::Image {
                    dim: ImageDimension::D2,
                    arrayed: false,
                    class:
                        ImageClass::Sampled {
                            kind: ScalarKind::Float,
                            multi: false,
                        },
                },
            ) => 0,
            (
                AddressSpace::Handle,
                Some(ResourceBinding {
                    group: 0,
                    binding: 1,
                }),
                TypeInner::Sampler { comparison: false },
            ) => 1,
            (
                AddressSpace::Uniform,
                Some(ResourceBinding {
                    group: 0,
                    binding: 2,
                }),
                TypeInner::Struct { members, .. },
            ) if valid_uniforms(module, members) => 2,
            _ => return Err(invalid()),
        };
        if seen[index] {
            return Err(invalid());
        }
        seen[index] = true;
    }
    if !seen[0] || !seen[1] {
        return Err(invalid());
    }
    Ok(())
}

fn valid_uniforms(module: &Module, members: &[naga::StructMember]) -> bool {
    let [time, rest @ ..] = members else {
        return false;
    };
    time.name.as_deref() == Some("time")
        && time.offset == 0
        && float(&module.types[time.ty].inner, None)
        && (rest.is_empty()
            || matches!(rest, [resolution] if resolution.name.as_deref() == Some("resolution")
                && resolution.offset == 8
                && float(&module.types[resolution.ty].inner, Some(VectorSize::Bi))))
}
