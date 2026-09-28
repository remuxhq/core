//! What naga read, written out in the language of OBS's `.effect` files.
//!
//! Scalars, vectors and square matrices, arithmetic, the math library,
//! sampling, branches, loops and functions: what a picture filter is made of.
//! Anything past that is refused by name, never guessed at.

use std::collections::HashSet;
use std::fmt::Write as _;

use naga::{
    AddressSpace, BinaryOperator, Expression, Handle, ImageQuery, MathFunction, Module,
    RelationalFunction, SampleLevel, ScalarKind, Statement, TypeInner, UnaryOperator,
};

/// The uniform block's members as the effect's own uniforms: the filter
/// sets `time` and `resolution` by name.
const UNIFORMS: [&str; 2] = ["time", "resolution"];

fn unsupported(what: &str) -> String {
    format!("this filter uses {what}, which the libobs motor cannot run")
}

pub(crate) struct Writer<'a> {
    module: &'a Module,
    info: &'a naga::valid::ModuleInfo,
    out: String,
    loops: usize,
}

/// The function being written: its IR, what naga resolved its expressions
/// to, whether it is the entry point, and what its body evaluates in order
/// (naga's `Emit`), each held in a temporary.
struct Scope<'a> {
    function: &'a naga::Function,
    info: &'a naga::valid::FunctionInfo,
    entry: bool,
    emitted: HashSet<Handle<Expression>>,
}

fn emitted(block: &naga::Block, into: &mut HashSet<Handle<Expression>>) {
    for statement in block.iter() {
        match statement {
            Statement::Emit(range) => into.extend(range.clone()),
            Statement::Block(inner) => emitted(inner, into),
            Statement::If { accept, reject, .. } => {
                emitted(accept, into);
                emitted(reject, into);
            }
            Statement::Loop {
                body, continuing, ..
            } => {
                emitted(body, into);
                emitted(continuing, into);
            }
            _ => {}
        }
    }
}

impl<'a> Scope<'a> {
    fn of(function: &'a naga::Function, info: &'a naga::valid::FunctionInfo, entry: bool) -> Self {
        let mut held = HashSet::new();
        emitted(&function.body, &mut held);
        Self {
            function,
            info,
            entry,
            emitted: held,
        }
    }
}

impl<'a> Writer<'a> {
    pub(crate) fn new(module: &'a Module, info: &'a naga::valid::ModuleInfo) -> Self {
        Self {
            module,
            info,
            out: String::new(),
            loops: 0,
        }
    }

    pub(crate) fn write(mut self) -> Result<String, String> {
        self.out.push_str(
            "uniform float4x4 ViewProj;\n\
             uniform texture2d image;\n\
             uniform float time;\n\
             uniform float2 resolution;\n\n\
             sampler_state remux_sampler {\n\
             \tFilter   = Linear;\n\
             \tAddressU = Clamp;\n\
             \tAddressV = Clamp;\n\
             };\n\n\
             struct VertData {\n\
             \tfloat4 pos : POSITION;\n\
             \tfloat2 uv  : TEXCOORD0;\n\
             };\n\n\
             VertData VSDefault(VertData v_in)\n{\n\
             \tVertData vert_out;\n\
             \tvert_out.pos = mul(float4(v_in.pos.xyz, 1.0), ViewProj);\n\
             \tvert_out.uv  = v_in.uv;\n\
             \treturn vert_out;\n}\n\n",
        );
        for (handle, function) in self.module.functions.iter() {
            self.function(handle, function)?;
        }
        let entry = &self.module.entry_points[0];
        self.out
            .push_str("float4 PSRemux(VertData v_in) : TARGET\n{\n");
        let scope = Scope::of(&entry.function, self.info.get_entry_point(0), true);
        self.locals(&scope)?;
        self.block(&scope, &entry.function.body, 1)?;
        self.out.push_str("}\n\n");
        self.out.push_str(
            "technique Draw\n{\n\tpass\n\t{\n\
             \t\tvertex_shader = VSDefault(v_in);\n\
             \t\tpixel_shader  = PSRemux(v_in);\n\
             \t}\n}\n",
        );
        Ok(self.out)
    }

    fn type_name(&self, inner: &TypeInner) -> Result<String, String> {
        let scalar = |scalar: &naga::Scalar| -> Result<&'static str, String> {
            Ok(match (scalar.kind, scalar.width) {
                (ScalarKind::Float, 4) => "float",
                (ScalarKind::Sint, 4) => "int",
                (ScalarKind::Uint, 4) => "uint",
                (ScalarKind::Bool, _) => "bool",
                _ => return Err(unsupported("a number of a width other than 32 bits")),
            })
        };
        Ok(match inner {
            TypeInner::Scalar(s) => scalar(s)?.to_string(),
            TypeInner::Vector { size, scalar: s } => format!("{}{}", scalar(s)?, *size as u8),
            // GLSL's own name: the motor's graphics module is OpenGL, and
            // libobs hands a name it does not translate through as it is.
            TypeInner::Matrix {
                columns,
                rows,
                scalar: s,
            } if columns == rows && s.kind == ScalarKind::Float => {
                format!("mat{}", *columns as u8)
            }
            TypeInner::Matrix { .. } => return Err(unsupported("a matrix that is not square")),
            TypeInner::Array { .. } => return Err(unsupported("an array")),
            TypeInner::Struct { .. } => return Err(unsupported("a struct of its own")),
            _ => {
                return Err(unsupported(
                    "a type other than numbers, vectors and matrices",
                ))
            }
        })
    }

    fn ty(&self, handle: Handle<naga::Type>) -> Result<String, String> {
        self.type_name(&self.module.types[handle].inner)
    }

    fn zero(&self, inner: &TypeInner) -> Result<String, String> {
        Ok(match inner {
            TypeInner::Scalar(s) => match s.kind {
                ScalarKind::Float => "0.0".into(),
                ScalarKind::Bool => "false".into(),
                ScalarKind::Uint => "0u".into(),
                _ => "0".into(),
            },
            TypeInner::Vector { scalar, .. } => {
                let one = self.zero(&TypeInner::Scalar(*scalar))?;
                format!("{}({one})", self.type_name(inner)?)
            }
            TypeInner::Matrix { .. } => format!("{}(0.0)", self.type_name(inner)?),
            _ => return Err(unsupported("a zero of a composite type")),
        })
    }

    fn literal(literal: &naga::Literal) -> Result<String, String> {
        use naga::Literal as L;
        let float = |f: f64| -> Result<String, String> {
            if !f.is_finite() {
                return Err(unsupported("an infinite or NaN constant"));
            }
            let text = format!("{f:?}");
            Ok(if text.contains(['.', 'e', 'E']) {
                text
            } else {
                format!("{text}.0")
            })
        };
        Ok(match literal {
            L::F32(f) => float(f64::from(*f))?,
            L::F64(f) | L::AbstractFloat(f) => float(*f)?,
            L::I32(i) => i.to_string(),
            L::I64(i) | L::AbstractInt(i) => i.to_string(),
            L::U32(u) => format!("{u}u"),
            L::Bool(b) => b.to_string(),
            _ => return Err(unsupported("a 16- or 64-bit constant")),
        })
    }

    /// A module-level constant's value, from its initializer.
    fn global_expression(&self, handle: Handle<Expression>) -> Result<String, String> {
        match &self.module.global_expressions[handle] {
            Expression::Literal(literal) => Self::literal(literal),
            Expression::Constant(c) => self.global_expression(self.module.constants[*c].init),
            Expression::ZeroValue(ty) => self.zero(&self.module.types[*ty].inner),
            Expression::Compose { ty, components } => {
                let parts = components
                    .iter()
                    .map(|c| self.global_expression(*c))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(format!("{}({})", self.ty(*ty)?, parts.join(", ")))
            }
            Expression::Splat { value, size } => {
                let one = self.global_expression(*value)?;
                Ok(format!(
                    "float{}({})",
                    *size as u8,
                    vec![one; *size as usize].join(", ")
                ))
            }
            _ => Err(unsupported("a constant this writer does not read")),
        }
    }

    fn function(
        &mut self,
        handle: Handle<naga::Function>,
        function: &'a naga::Function,
    ) -> Result<(), String> {
        let returns = match &function.result {
            Some(result) => self.ty(result.ty)?,
            None => "void".into(),
        };
        let mut params = Vec::new();
        for (i, argument) in function.arguments.iter().enumerate() {
            params.push(match &self.module.types[argument.ty].inner {
                TypeInner::Pointer { base, .. } => format!("inout {} a{i}", self.ty(*base)?),
                inner => format!("{} a{i}", self.type_name(inner)?),
            });
        }
        let _ = writeln!(
            self.out,
            "{returns} f{}({})\n{{",
            handle.index(),
            params.join(", ")
        );
        let scope = Scope::of(function, &self.info[handle], false);
        self.locals(&scope)?;
        self.block(&scope, &function.body, 1)?;
        self.out.push_str("}\n\n");
        Ok(())
    }

    fn locals(&mut self, scope: &Scope) -> Result<(), String> {
        for (handle, local) in scope.function.local_variables.iter() {
            let ty = &self.module.types[local.ty].inner;
            let init = match local.init {
                Some(init) => self.expression(scope, init)?,
                None => self.zero(ty)?,
            };
            let _ = writeln!(
                self.out,
                "\t{} l{} = {init};",
                self.type_name(ty)?,
                handle.index()
            );
        }
        Ok(())
    }

    fn inner<'s>(&'s self, scope: &'s Scope, handle: Handle<Expression>) -> &'s TypeInner {
        scope.info[handle].ty.inner_with(&self.module.types)
    }

    /// Whether an expression is written where it is used rather than held in
    /// a temporary: a place (a pointer), a texture or sampler, and whatever
    /// the body never evaluates in order (a constant). A call's result is
    /// held at the call.
    fn inline(&self, scope: &Scope, handle: Handle<Expression>) -> bool {
        if matches!(
            scope.function.expressions[handle],
            Expression::CallResult(_)
        ) {
            return false;
        }
        matches!(
            self.inner(scope, handle),
            TypeInner::Pointer { .. }
                | TypeInner::ValuePointer { .. }
                | TypeInner::Struct { .. }
                | TypeInner::Image { .. }
                | TypeInner::Sampler { .. }
        ) || !scope.emitted.contains(&handle)
    }

    fn block(&mut self, scope: &Scope, block: &naga::Block, depth: usize) -> Result<(), String> {
        let pad = "\t".repeat(depth);
        for statement in block.iter() {
            match statement {
                Statement::Emit(range) => {
                    for handle in range.clone() {
                        if self.inline(scope, handle) {
                            continue;
                        }
                        let ty = self.type_name(self.inner(scope, handle))?;
                        let value = self.expression(scope, handle)?;
                        let _ = writeln!(self.out, "{pad}{ty} e{} = {value};", handle.index());
                    }
                }
                Statement::Block(inner) => {
                    let _ = writeln!(self.out, "{pad}{{");
                    self.block(scope, inner, depth + 1)?;
                    let _ = writeln!(self.out, "{pad}}}");
                }
                Statement::If {
                    condition,
                    accept,
                    reject,
                } => {
                    let condition = self.value(scope, *condition)?;
                    let _ = writeln!(self.out, "{pad}if ({condition}) {{");
                    self.block(scope, accept, depth + 1)?;
                    if !reject.is_empty() {
                        let _ = writeln!(self.out, "{pad}}} else {{");
                        self.block(scope, reject, depth + 1)?;
                    }
                    let _ = writeln!(self.out, "{pad}}}");
                }
                Statement::Loop {
                    body,
                    continuing,
                    break_if,
                } => {
                    // naga's loop: the body, then `continuing` before every
                    // turn but the first, then `break_if`. A `continue` lands
                    // at the top, which runs `continuing` first, as naga means.
                    let first = format!("first{}", self.loops);
                    self.loops += 1;
                    let _ = writeln!(self.out, "{pad}bool {first} = true;");
                    let _ = writeln!(self.out, "{pad}while (true) {{");
                    if !continuing.is_empty() || break_if.is_some() {
                        let _ = writeln!(self.out, "{pad}\tif (!{first}) {{");
                        self.block(scope, continuing, depth + 2)?;
                        if let Some(stop) = break_if {
                            let stop = self.value(scope, *stop)?;
                            let _ = writeln!(self.out, "{pad}\t\tif ({stop}) {{ break; }}");
                        }
                        let _ = writeln!(self.out, "{pad}\t}}");
                    }
                    let _ = writeln!(self.out, "{pad}\t{first} = false;");
                    self.block(scope, body, depth + 1)?;
                    let _ = writeln!(self.out, "{pad}}}");
                }
                Statement::Break => {
                    let _ = writeln!(self.out, "{pad}break;");
                }
                Statement::Continue => {
                    let _ = writeln!(self.out, "{pad}continue;");
                }
                Statement::Kill => {
                    let _ = writeln!(self.out, "{pad}discard;");
                }
                Statement::Return { value } => {
                    let returned = match value {
                        Some(value) => format!(" {}", self.value(scope, *value)?),
                        None => String::new(),
                    };
                    let _ = writeln!(self.out, "{pad}return{returned};");
                }
                Statement::Store { pointer, value } => {
                    let place = self.expression(scope, *pointer)?;
                    let value = self.value(scope, *value)?;
                    let _ = writeln!(self.out, "{pad}{place} = {value};");
                }
                Statement::Call {
                    function,
                    arguments,
                    result,
                } => {
                    let args = arguments
                        .iter()
                        .map(|a| self.value(scope, *a))
                        .collect::<Result<Vec<_>, _>>()?;
                    let call = format!("f{}({})", function.index(), args.join(", "));
                    match result {
                        Some(result) => {
                            let ty = self.type_name(self.inner(scope, *result))?;
                            let _ = writeln!(self.out, "{pad}{ty} e{} = {call};", result.index());
                        }
                        None => {
                            let _ = writeln!(self.out, "{pad}{call};");
                        }
                    }
                }
                Statement::Switch { .. } => return Err(unsupported("a switch")),
                _ => return Err(unsupported("a statement a picture filter has no use for")),
            }
        }
        Ok(())
    }

    /// An expression where it is used: its temporary when it was held in
    /// one, itself otherwise.
    fn value(&self, scope: &Scope, handle: Handle<Expression>) -> Result<String, String> {
        if self.inline(scope, handle) {
            self.expression(scope, handle)
        } else {
            Ok(format!("e{}", handle.index()))
        }
    }

    fn expression(&self, scope: &Scope, handle: Handle<Expression>) -> Result<String, String> {
        let v = |h: Handle<Expression>| self.value(scope, h);
        Ok(match &scope.function.expressions[handle] {
            Expression::Literal(literal) => Self::literal(literal)?,
            Expression::Constant(c) => self.global_expression(self.module.constants[*c].init)?,
            Expression::ZeroValue(ty) => self.zero(&self.module.types[*ty].inner)?,
            Expression::FunctionArgument(i) if scope.entry => {
                debug_assert_eq!(*i, 0, "the contract has one input");
                "v_in.uv".into()
            }
            Expression::FunctionArgument(i) => format!("a{i}"),
            Expression::LocalVariable(l) => format!("l{}", l.index()),
            Expression::GlobalVariable(g) => {
                let global = &self.module.global_variables[*g];
                match (global.space, &self.module.types[global.ty].inner) {
                    (AddressSpace::Handle, TypeInner::Image { .. }) => "image".into(),
                    (AddressSpace::Handle, TypeInner::Sampler { .. }) => "remux_sampler".into(),
                    _ => return Err(unsupported("the uniform block whole, not its members")),
                }
            }
            Expression::Load { pointer } => self.expression(scope, *pointer)?,
            Expression::Access { base, index } => format!("{}[{}]", v(*base)?, v(*index)?),
            Expression::AccessIndex { base, index } => {
                // The uniform block's members are the effect's own uniforms.
                if let Expression::GlobalVariable(g) = scope.function.expressions[*base] {
                    if self.module.global_variables[g].space == AddressSpace::Uniform {
                        return UNIFORMS
                            .get(*index as usize)
                            .map(|name| name.to_string())
                            .ok_or_else(|| {
                                unsupported("a uniform other than time and resolution")
                            });
                    }
                }
                let base_inner = match self.inner(scope, *base) {
                    TypeInner::Pointer { base, .. } => &self.module.types[*base].inner,
                    inner => inner,
                };
                let base = v(*base)?;
                match base_inner {
                    TypeInner::Vector { .. } | TypeInner::ValuePointer { size: Some(_), .. } => {
                        format!("{base}.{}", ["x", "y", "z", "w"][*index as usize])
                    }
                    TypeInner::Matrix { .. } => format!("{base}[{index}]"),
                    _ => return Err(unsupported("a member of a struct")),
                }
            }
            Expression::Splat { size, value } => {
                let ty = self.type_name(self.inner(scope, handle))?;
                let one = v(*value)?;
                format!("{ty}({})", vec![one; *size as usize].join(", "))
            }
            Expression::Swizzle {
                size,
                vector,
                pattern,
            } => {
                let letters: String = pattern[..*size as usize]
                    .iter()
                    .map(|c| ['x', 'y', 'z', 'w'][*c as usize])
                    .collect();
                format!("{}.{letters}", v(*vector)?)
            }
            Expression::Compose { ty, components } => {
                let parts = components
                    .iter()
                    .map(|c| v(*c))
                    .collect::<Result<Vec<_>, _>>()?;
                format!("{}({})", self.ty(*ty)?, parts.join(", "))
            }
            Expression::ImageSample {
                coordinate,
                level,
                gather,
                array_index,
                offset,
                depth_ref,
                ..
            } => {
                if gather.is_some() || array_index.is_some() || depth_ref.is_some() {
                    return Err(unsupported("a gather, an array or a depth comparison"));
                }
                if offset.is_some() {
                    return Err(unsupported("a sample with an offset"));
                }
                let at = v(*coordinate)?;
                match level {
                    SampleLevel::Auto => format!("image.Sample(remux_sampler, {at})"),
                    SampleLevel::Zero => format!("image.SampleLevel(remux_sampler, {at}, 0.0)"),
                    SampleLevel::Exact(l) => {
                        format!("image.SampleLevel(remux_sampler, {at}, {})", v(*l)?)
                    }
                    SampleLevel::Bias(b) => {
                        format!("image.SampleBias(remux_sampler, {at}, {})", v(*b)?)
                    }
                    SampleLevel::Gradient { x, y } => format!(
                        "image.SampleGrad(remux_sampler, {at}, {}, {})",
                        v(*x)?,
                        v(*y)?
                    ),
                }
            }
            Expression::ImageLoad {
                coordinate,
                level,
                array_index,
                sample,
                ..
            } => {
                if array_index.is_some() || sample.is_some() {
                    return Err(unsupported("an array or multisampled load"));
                }
                let level = match level {
                    Some(l) => format!("int({})", v(*l)?),
                    None => "0".into(),
                };
                format!("image.Load(int3(int2({}), {level}))", v(*coordinate)?)
            }
            // The picture's size is the `resolution` the filter sets.
            Expression::ImageQuery {
                query: ImageQuery::Size { level: None },
                ..
            } => "uint2(resolution)".into(),
            Expression::ImageQuery { .. } => {
                return Err(unsupported("a texture query other than its size"))
            }
            Expression::Unary { op, expr } => {
                let op = match op {
                    UnaryOperator::Negate => "-",
                    UnaryOperator::LogicalNot => "!",
                    UnaryOperator::BitwiseNot => "~",
                };
                format!("({op}{})", v(*expr)?)
            }
            Expression::Binary { op, left, right } => {
                let (l, r) = (v(*left)?, v(*right)?);
                let vector = matches!(self.inner(scope, *left), TypeInner::Vector { .. })
                    || matches!(self.inner(scope, *right), TypeInner::Vector { .. });
                let float = matches!(
                    self.inner(scope, *left).scalar_kind(),
                    Some(ScalarKind::Float)
                );
                use BinaryOperator as B;
                match op {
                    B::Add => format!("({l} + {r})"),
                    B::Subtract => format!("({l} - {r})"),
                    B::Multiply => format!("({l} * {r})"),
                    B::Divide => format!("({l} / {r})"),
                    // WGSL's `%` truncates, for floats as for integers.
                    B::Modulo if float => format!("({l} - {r} * trunc({l} / {r}))"),
                    B::Modulo => format!("({l} % {r})"),
                    B::Equal
                    | B::NotEqual
                    | B::Less
                    | B::LessEqual
                    | B::Greater
                    | B::GreaterEqual
                        if vector =>
                    {
                        let name = match op {
                            B::Equal => "equal",
                            B::NotEqual => "notEqual",
                            B::Less => "lessThan",
                            B::LessEqual => "lessThanEqual",
                            B::Greater => "greaterThan",
                            _ => "greaterThanEqual",
                        };
                        format!("{name}({l}, {r})")
                    }
                    B::Equal => format!("({l} == {r})"),
                    B::NotEqual => format!("({l} != {r})"),
                    B::Less => format!("({l} < {r})"),
                    B::LessEqual => format!("({l} <= {r})"),
                    B::Greater => format!("({l} > {r})"),
                    B::GreaterEqual => format!("({l} >= {r})"),
                    B::And => format!("({l} & {r})"),
                    B::ExclusiveOr => format!("({l} ^ {r})"),
                    B::InclusiveOr => format!("({l} | {r})"),
                    B::LogicalAnd => format!("({l} && {r})"),
                    B::LogicalOr => format!("({l} || {r})"),
                    B::ShiftLeft => format!("({l} << {r})"),
                    B::ShiftRight => format!("({l} >> {r})"),
                }
            }
            Expression::Select {
                condition,
                accept,
                reject,
            } => {
                let (c, a, r) = (v(*condition)?, v(*accept)?, v(*reject)?);
                if matches!(self.inner(scope, *condition), TypeInner::Vector { .. }) {
                    format!("mix({r}, {a}, {c})")
                } else {
                    format!("({c} ? {a} : {r})")
                }
            }
            Expression::Derivative { axis, expr, .. } => {
                let name = match axis {
                    naga::DerivativeAxis::X => "ddx",
                    naga::DerivativeAxis::Y => "ddy",
                    naga::DerivativeAxis::Width => "fwidth",
                };
                format!("{name}({})", v(*expr)?)
            }
            Expression::Relational { fun, argument } => {
                let name = match fun {
                    RelationalFunction::All => "all",
                    RelationalFunction::Any => "any",
                    RelationalFunction::IsNan => "isnan",
                    RelationalFunction::IsInf => "isinf",
                };
                format!("{name}({})", v(*argument)?)
            }
            Expression::Math {
                fun,
                arg,
                arg1,
                arg2,
                arg3,
            } => {
                let mut args = vec![v(*arg)?];
                for extra in [arg1, arg2, arg3].into_iter().flatten() {
                    args.push(v(*extra)?);
                }
                if *fun == MathFunction::Fma {
                    return Ok(format!("(({} * {}) + {})", args[0], args[1], args[2]));
                }
                format!("{}({})", math(*fun)?, args.join(", "))
            }
            Expression::As {
                expr,
                convert: Some(_),
                ..
            } => format!(
                "{}({})",
                self.type_name(self.inner(scope, handle))?,
                v(*expr)?
            ),
            Expression::As { convert: None, .. } => return Err(unsupported("a bit cast")),
            Expression::CallResult(_) => format!("e{}", handle.index()),
            _ => return Err(unsupported("an expression a picture filter has no use for")),
        })
    }
}

/// A math function by the name OBS's effect language gives it, which libobs
/// turns back into its graphics module's own.
fn math(fun: MathFunction) -> Result<&'static str, String> {
    use MathFunction as M;
    Ok(match fun {
        M::Abs => "abs",
        M::Min => "min",
        M::Max => "max",
        M::Clamp => "clamp",
        M::Saturate => "saturate",
        M::Cos => "cos",
        M::Cosh => "cosh",
        M::Sin => "sin",
        M::Sinh => "sinh",
        M::Tan => "tan",
        M::Tanh => "tanh",
        M::Acos => "acos",
        M::Asin => "asin",
        M::Atan => "atan",
        M::Atan2 => "atan2",
        M::Radians => "radians",
        M::Degrees => "degrees",
        M::Ceil => "ceil",
        M::Floor => "floor",
        M::Round => "round",
        M::Fract => "frac",
        M::Trunc => "trunc",
        M::Exp => "exp",
        M::Exp2 => "exp2",
        M::Log => "log",
        M::Log2 => "log2",
        M::Pow => "pow",
        M::Dot => "dot",
        M::Cross => "cross",
        M::Distance => "distance",
        M::Length => "length",
        M::Normalize => "normalize",
        M::FaceForward => "faceforward",
        M::Reflect => "reflect",
        M::Refract => "refract",
        M::Sign => "sign",
        M::Mix => "lerp",
        M::Step => "step",
        M::SmoothStep => "smoothstep",
        M::Sqrt => "sqrt",
        M::InverseSqrt => "rsqrt",
        M::Transpose => "transpose",
        M::Determinant => "determinant",
        M::Inverse => "inverse",
        _ => return Err(unsupported("a bit or packing function")),
    })
}
