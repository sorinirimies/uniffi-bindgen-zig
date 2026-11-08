//! Mapping UniFFI types to Zig: type names, which types need an allocator, and
//! the expressions that read, write, free, lower and lift them.

use std::collections::BTreeSet;

use uniffi_bindgen::ComponentInterface;
use uniffi_bindgen::interface::{AsType, DefaultValue, FfiType, Literal, Radix, Type};

use super::names::{camel, ident, snake, string_literal, unquote, var};
use crate::config::Config;

/// Everything the emitters need to know about the component being generated.
#[derive(Debug)]
pub(crate) struct Gen<'a> {
    pub(crate) ci: &'a ComponentInterface,
    pub(crate) config: &'a Config,
    /// Names of the error types of the other components generated alongside this one.
    external_errors: &'a BTreeSet<String>,
    /// The names of the file's types: a method or function may not reuse one.
    type_names: BTreeSet<String>,
    /// Every name declared in the file or in one of its types: a parameter may not
    /// shadow one.
    taken: BTreeSet<String>,
}

/// Names the bindings declare on types or in the file themselves.
const OWN_DECLS: &[&str] = &[
    "init",
    "deinit",
    "clone",
    "toString",
    "debugString",
    "eql",
    "hash",
    "order",
    "toError",
    "from",
    "fromImpl",
    "uniffiLower",
    "uniffiLift",
    "uniffiCmp",
    "bufferFrom",
    "bufferFree",
    "uniffiEnsureInit",
    "droppedFuture",
    "Set",
    "VTable",
    "handle",
    "std",
    "uniffi",
    "ffi",
];

impl<'a> Gen<'a> {
    pub(crate) fn new(
        ci: &'a ComponentInterface,
        config: &'a Config,
        external_errors: &'a BTreeSet<String>,
    ) -> Self {
        let mut type_names = BTreeSet::new();
        let mut taken: BTreeSet<String> = OWN_DECLS.iter().map(|s| (*s).to_owned()).collect();
        let fn_name = |n: &str| unquote(&camel(n)).to_owned();
        for f in ci.function_definitions() {
            taken.insert(fn_name(f.name()));
        }
        for o in ci.object_definitions() {
            type_names.insert(o.name().to_owned());
            o.methods().iter().for_each(|m| {
                taken.insert(fn_name(m.name()));
            });
            o.constructors().iter().for_each(|c| {
                taken.insert(fn_name(c.name()));
            });
        }
        for r in ci.record_definitions() {
            type_names.insert(r.name().to_owned());
            r.methods().iter().for_each(|m| {
                taken.insert(fn_name(m.name()));
            });
            r.constructors().iter().for_each(|c| {
                taken.insert(fn_name(c.name()));
            });
        }
        for e in ci.enum_definitions() {
            type_names.insert(e.name().to_owned());
            e.methods().iter().for_each(|m| {
                taken.insert(fn_name(m.name()));
            });
            e.constructors().iter().for_each(|c| {
                taken.insert(fn_name(c.name()));
            });
        }
        for cb in ci.callback_interface_definitions() {
            type_names.insert(cb.name().to_owned());
            cb.methods().iter().for_each(|m| {
                taken.insert(fn_name(m.name()));
            });
        }
        for t in ci.iter_local_types() {
            if let Type::Custom { name, .. } = t {
                type_names.insert(name.clone());
            }
        }
        // Methods and functions named like a type or like one of the bindings' own
        // declarations get a trailing `_`: that name is taken too.
        let renamed: Vec<String> = taken
            .iter()
            .filter(|n| type_names.contains(*n) || OWN_DECLS.contains(&n.as_str()))
            .map(|n| format!("{n}_"))
            .collect();
        taken.extend(renamed);
        taken.extend(type_names.iter().cloned());
        Self {
            ci,
            config,
            external_errors,
            type_names,
            taken,
        }
    }

    /// An argument name that shadows no declaration of the file (Zig forbids it).
    pub(crate) fn arg(&self, name: &str) -> String {
        let v = var(name);
        let mut raw = unquote(&v).to_owned();
        if !self.taken.contains(&raw) {
            return v;
        }
        while self.taken.contains(&raw) {
            raw.push('_');
        }
        raw
    }

    /// A function or method name that is not also a type's name (Zig finds that ambiguous).
    pub(crate) fn callable_name(&self, name: &str, reserved: &[&str]) -> String {
        let c = camel(name);
        let raw = unquote(&c);
        if self.type_names.contains(raw) || reserved.contains(&raw) {
            format!("{raw}_")
        } else {
            c
        }
    }

    /// `name` after the `rename` table.
    pub(crate) fn renamed_type<'n>(&'n self, name: &'n str) -> &'n str {
        self.config
            .rename
            .get(name)
            .and_then(|v| v.as_str())
            .unwrap_or(name)
    }

    /// The module (`@import`) name of the component that defines `t`, when external.
    pub(crate) fn external_module(&self, t: &Type) -> Option<String> {
        if !self.ci.is_external(t) {
            return None;
        }
        self.ci.namespace_for_type(t).ok().map(str::to_owned)
    }

    /// `Name`, or `ns.Name` for a type another component defines.
    fn named(&self, t: &Type, name: &str) -> String {
        let name = ident(self.renamed_type(name));
        match self.external_module(t) {
            Some(ns) => format!("{}.{name}", ident(&ns)),
            None => name,
        }
    }

    /// The Zig type of a value of `t`.
    pub(crate) fn zig_type(&self, t: &Type) -> String {
        match t {
            Type::UInt8 => "u8".into(),
            Type::Int8 => "i8".into(),
            Type::UInt16 => "u16".into(),
            Type::Int16 => "i16".into(),
            Type::UInt32 => "u32".into(),
            Type::Int32 => "i32".into(),
            Type::UInt64 => "u64".into(),
            Type::Int64 => "i64".into(),
            Type::Float32 => "f32".into(),
            Type::Float64 => "f64".into(),
            Type::Boolean => "bool".into(),
            Type::String | Type::Bytes => "[]const u8".into(),
            Type::Timestamp => "uniffi.Timestamp".into(),
            Type::Duration => "uniffi.Duration".into(),
            Type::Object { name, .. }
            | Type::CallbackInterface { name, .. }
            | Type::Record { name, .. }
            | Type::Enum { name, .. }
            | Type::Custom { name, .. } => self.named(t, name),
            Type::Optional { inner_type } => format!("?{}", self.zig_type(inner_type)),
            Type::Sequence { inner_type } | Type::Set { inner_type } => {
                format!("[]const {}", self.zig_type(inner_type))
            }
            Type::Map {
                key_type,
                value_type,
            } => {
                format!(
                    "[]const uniffi.Entry({}, {})",
                    self.zig_type(key_type),
                    self.zig_type(value_type)
                )
            }
            Type::Box { inner_type } => format!("*const {}", self.zig_type(inner_type)),
        }
    }

    /// A name for `t` that is a valid identifier, unique among the component's types.
    pub(crate) fn mangle(&self, t: &Type) -> String {
        match t {
            Type::UInt8 => "U8".into(),
            Type::Int8 => "I8".into(),
            Type::UInt16 => "U16".into(),
            Type::Int16 => "I16".into(),
            Type::UInt32 => "U32".into(),
            Type::Int32 => "I32".into(),
            Type::UInt64 => "U64".into(),
            Type::Int64 => "I64".into(),
            Type::Float32 => "F32".into(),
            Type::Float64 => "F64".into(),
            Type::Boolean => "Bool".into(),
            Type::String => "String".into(),
            Type::Bytes => "Bytes".into(),
            Type::Timestamp => "Timestamp".into(),
            Type::Duration => "Duration".into(),
            Type::Object { name, .. }
            | Type::CallbackInterface { name, .. }
            | Type::Record { name, .. }
            | Type::Enum { name, .. }
            | Type::Custom { name, .. } => {
                let name = self.renamed_type(name).to_owned();
                match self.external_module(t) {
                    Some(ns) => format!("{}{name}", camel_upper(&ns)),
                    None => name,
                }
            }
            Type::Box { inner_type } => format!("Box{}", self.mangle(inner_type)),
            Type::Optional { inner_type } => format!("Option{}", self.mangle(inner_type)),
            Type::Sequence { inner_type } => format!("Sequence{}", self.mangle(inner_type)),
            Type::Set { inner_type } => format!("Set{}", self.mangle(inner_type)),
            Type::Map {
                key_type,
                value_type,
            } => format!("Map{}{}", self.mangle(key_type), self.mangle(value_type)),
        }
    }

    pub(crate) fn is_container(t: &Type) -> bool {
        matches!(
            t,
            Type::Optional { .. }
                | Type::Sequence { .. }
                | Type::Set { .. }
                | Type::Map { .. }
                | Type::Box { .. }
        )
    }

    /// Whether a value of `t` owns heap memory (so reading one needs an allocator).
    pub(crate) fn needs_alloc(&self, t: &Type) -> bool {
        self.needs_alloc_in(t, &mut BTreeSet::new())
    }

    fn needs_alloc_in(&self, t: &Type, seen: &mut BTreeSet<String>) -> bool {
        match t {
            Type::String
            | Type::Bytes
            | Type::Sequence { .. }
            | Type::Set { .. }
            | Type::Map { .. }
            | Type::Box { .. } => true,
            Type::Optional { inner_type } => self.needs_alloc_in(inner_type, seen),
            Type::Custom { builtin, .. } => self.needs_alloc_in(builtin, seen),
            Type::Record { name, .. } | Type::Enum { name, .. } if self.ci.is_external(t) => {
                let _ = name;
                true
            }
            Type::Record { name, .. } => {
                if !seen.insert(name.clone()) {
                    return false;
                }
                self.ci.get_record_definition(name).is_some_and(|r| {
                    r.fields()
                        .iter()
                        .any(|f| self.needs_alloc_in(&f.as_type(), seen))
                })
            }
            Type::Enum { name, .. } => {
                if !seen.insert(name.clone()) {
                    return false;
                }
                self.ci.get_enum_definition(name).is_some_and(|e| {
                    (e.is_flat() && self.is_error_enum(name))
                        || e.variants().iter().any(|v| {
                            v.fields()
                                .iter()
                                .any(|f| self.needs_alloc_in(&f.as_type(), seen))
                        })
                })
            }
            _ => false,
        }
    }

    /// Whether a value of `t` must be released: heap memory or object handles.
    pub(crate) fn needs_release(&self, t: &Type) -> bool {
        self.needs_release_in(t, &mut BTreeSet::new())
    }

    fn needs_release_in(&self, t: &Type, seen: &mut BTreeSet<String>) -> bool {
        match t {
            Type::Object { .. } => true,
            Type::Optional { inner_type } => self.needs_release_in(inner_type, seen),
            Type::Custom { builtin, .. } => self.needs_release_in(builtin, seen),
            Type::Record { name, .. } if !self.ci.is_external(t) => {
                if !seen.insert(name.clone()) {
                    return false;
                }
                self.needs_alloc(t)
                    || self.ci.get_record_definition(name).is_some_and(|r| {
                        r.fields()
                            .iter()
                            .any(|f| self.needs_release_in(&f.as_type(), seen))
                    })
            }
            Type::Enum { name, .. } if !self.ci.is_external(t) => {
                if !seen.insert(name.clone()) {
                    return false;
                }
                self.needs_alloc(t)
                    || self.ci.get_enum_definition(name).is_some_and(|e| {
                        e.variants().iter().any(|v| {
                            v.fields()
                                .iter()
                                .any(|f| self.needs_release_in(&f.as_type(), seen))
                        })
                    })
            }
            Type::Sequence { inner_type } | Type::Set { inner_type } | Type::Box { inner_type } => {
                let _ = inner_type;
                true
            }
            _ => self.needs_alloc(t),
        }
    }

    pub(crate) fn is_flat_enum(&self, t: &Type) -> bool {
        match t {
            Type::Enum { name, .. } => self
                .ci
                .get_enum_definition(name)
                .is_some_and(|e| e.is_flat() && !self.is_error_enum(name)),
            _ => false,
        }
    }

    pub(crate) fn is_error_enum(&self, name: &str) -> bool {
        self.ci.get_object_definition(name).is_none()
            && self.ci.get_record_definition(name).is_none()
            && (self.ci.is_name_used_as_error(name) || self.external_errors.contains(name))
    }

    /// The function that writes a `t` (named types and containers).
    fn write_fn(&self, t: &Type) -> String {
        match self.external_module(t) {
            Some(ns) if !Self::is_container(t) => format!(
                "{}.uniffiWrite{}",
                ident(&ns),
                self.renamed_type(type_name(t))
            ),
            _ if Self::is_container(t) => format!("write{}", self.mangle(t)),
            _ => format!("uniffiWrite{}", self.renamed_type(type_name(t))),
        }
    }

    fn read_fn(&self, t: &Type) -> String {
        match self.external_module(t) {
            Some(ns) if !Self::is_container(t) => format!(
                "{}.uniffiRead{}",
                ident(&ns),
                self.renamed_type(type_name(t))
            ),
            _ if Self::is_container(t) => format!("read{}", self.mangle(t)),
            _ => format!("uniffiRead{}", self.renamed_type(type_name(t))),
        }
    }

    /// A statement writing the value `expr` of `t` to the writer `w`.
    pub(crate) fn write_stmt(&self, t: &Type, expr: &str) -> String {
        match t {
            Type::Boolean => format!("w.boolean({expr});"),
            Type::Float32 => format!("w.float32({expr});"),
            Type::Float64 => format!("w.float64({expr});"),
            Type::UInt8
            | Type::Int8
            | Type::UInt16
            | Type::Int16
            | Type::UInt32
            | Type::Int32
            | Type::UInt64
            | Type::Int64 => format!("w.int({}, {expr});", self.zig_type(t)),
            Type::String | Type::Bytes => format!("w.string({expr});"),
            Type::Timestamp => format!("w.timestamp({expr});"),
            Type::Duration => format!("w.duration({expr});"),
            Type::Custom { builtin, .. } if !self.ci.is_external(t) => {
                self.write_stmt(builtin, expr)
            }
            _ => format!("{}(w, {expr});", self.write_fn(t)),
        }
    }

    /// An expression (with `try`) reading a `t` from the reader `r`, allocating with `a`.
    pub(crate) fn read_expr(&self, t: &Type) -> String {
        match t {
            Type::Boolean => "try r.boolean()".into(),
            Type::Float32 => "try r.float32()".into(),
            Type::Float64 => "try r.float64()".into(),
            Type::UInt8
            | Type::Int8
            | Type::UInt16
            | Type::Int16
            | Type::UInt32
            | Type::Int32
            | Type::UInt64
            | Type::Int64 => format!("try r.int({})", self.zig_type(t)),
            Type::String | Type::Bytes => "try r.string(a)".into(),
            Type::Timestamp => "try r.timestamp()".into(),
            Type::Duration => "try r.duration()".into(),
            Type::Custom { builtin, .. } if !self.ci.is_external(t) => self.read_expr(builtin),
            _ => format!("try {}(r, a)", self.read_fn(t)),
        }
    }

    /// A statement releasing the value `expr` of `t` with the allocator `a`, or `None`.
    pub(crate) fn free_stmt(&self, t: &Type, expr: &str) -> Option<String> {
        if !self.needs_release(t)
            && !(self.ci.is_external(t) && matches!(t, Type::Record { .. } | Type::Enum { .. }))
        {
            return None;
        }
        Some(match t {
            Type::String | Type::Bytes => format!("a.free({expr});"),
            Type::Object { .. } => format!("{expr}.deinit();"),
            Type::Record { .. } | Type::Enum { .. } => format!("{expr}.deinit(a);"),
            Type::Custom { builtin, .. } => return self.free_stmt(builtin, expr),
            _ => format!("free{}(a, {expr});", self.mangle(t)),
        })
    }

    /// The Zig spelling of an FFI-level type.
    pub(crate) fn ffi_type(&self, t: &FfiType) -> String {
        match t {
            FfiType::UInt8 => "u8".into(),
            FfiType::Int8 => "i8".into(),
            FfiType::UInt16 => "u16".into(),
            FfiType::Int16 => "i16".into(),
            FfiType::UInt32 => "u32".into(),
            FfiType::Int32 => "i32".into(),
            FfiType::UInt64 | FfiType::Handle => "u64".into(),
            FfiType::Int64 => "i64".into(),
            FfiType::Float32 => "f32".into(),
            FfiType::Float64 => "f64".into(),
            FfiType::RustBuffer(_) => "uniffi.RustBuffer".into(),
            FfiType::ForeignBytes => "uniffi.ForeignBytes".into(),
            FfiType::Callback(name) | FfiType::Struct(name) => format!("ffi.{name}"),
            FfiType::RustCallStatus => "uniffi.RustCallStatus".into(),
            FfiType::Reference(inner) => format!("*const {}", self.ffi_type(inner)),
            FfiType::MutReference(inner) => format!("*{}", self.ffi_type(inner)),
            FfiType::VoidPointer => "?*anyopaque".into(),
        }
    }

    /// Whether a value of `t` crosses the FFI in a RustBuffer.
    pub(crate) fn is_buffer_type(t: &Type) -> bool {
        match t {
            Type::Boolean
            | Type::UInt8
            | Type::Int8
            | Type::UInt16
            | Type::Int16
            | Type::UInt32
            | Type::Int32
            | Type::UInt64
            | Type::Int64
            | Type::Float32
            | Type::Float64
            | Type::Object { .. }
            | Type::CallbackInterface { .. } => false,
            Type::Custom { builtin, .. } => Self::is_buffer_type(builtin),
            _ => true,
        }
    }

    /// Expression lowering the Zig value `expr` of `t` to its FFI value.
    pub(crate) fn lower(&self, t: &Type, expr: &str) -> String {
        match t {
            Type::Boolean => format!("@as(i8, if ({expr}) 1 else 0)"),
            Type::Object { .. } => format!("{expr}.uniffiLower()"),
            Type::CallbackInterface { .. } => format!("{}.uniffiLower({expr})", self.zig_type(t)),
            Type::Custom { builtin, .. } => self.lower(builtin, expr),
            t if Self::is_buffer_type(t) => format!("lower{}({expr})", self.mangle(t)),
            _ => expr.to_owned(),
        }
    }

    /// Expression (with `try` where needed) lifting the FFI value `expr` to `t`,
    /// allocating with `a`.
    pub(crate) fn lift(&self, t: &Type, expr: &str) -> String {
        match t {
            Type::Boolean => format!("({expr} != 0)"),
            Type::Object { .. } => format!("{}{{ .handle = {expr} }}", self.zig_type(t)),
            Type::CallbackInterface { .. } => format!("{}.uniffiLift({expr})", self.zig_type(t)),
            Type::Custom { builtin, .. } => self.lift(builtin, expr),
            t if Self::is_buffer_type(t) => format!("try lift{}(a, {expr})", self.mangle(t)),
            _ => expr.to_owned(),
        }
    }

    /// The smallest number of bytes a `t` takes in a buffer; 0 when it can be empty.
    pub(crate) fn min_wire_size(&self, t: &Type) -> usize {
        match t {
            Type::Boolean | Type::UInt8 | Type::Int8 | Type::Optional { .. } => 1,
            Type::UInt16 | Type::Int16 => 2,
            Type::UInt32 | Type::Int32 | Type::Float32 => 4,
            Type::UInt64 | Type::Int64 | Type::Float64 => 8,
            Type::String
            | Type::Bytes
            | Type::Sequence { .. }
            | Type::Set { .. }
            | Type::Map { .. }
            | Type::Enum { .. } => 4,
            Type::Timestamp | Type::Duration => 12,
            Type::Object { .. } | Type::CallbackInterface { .. } => 8,
            Type::Box { inner_type } => self.min_wire_size(inner_type),
            Type::Custom { builtin, .. } => self.min_wire_size(builtin),
            Type::Record { name, .. } => self.ci.get_record_definition(name).map_or(0, |r| {
                r.fields()
                    .iter()
                    .map(|f| self.min_wire_size(&f.as_type()))
                    .sum()
            }),
        }
    }

    /// A Zig default value for a field, or `None` when it has no constant form.
    pub(crate) fn default_value(&self, value: &DefaultValue, t: &Type) -> Option<String> {
        match value {
            DefaultValue::Default => self.zero(t),
            DefaultValue::Literal(lit) => self.literal(lit, t),
        }
    }

    fn zero(&self, t: &Type) -> Option<String> {
        Some(match t {
            Type::Boolean => "false".into(),
            Type::UInt8
            | Type::Int8
            | Type::UInt16
            | Type::Int16
            | Type::UInt32
            | Type::Int32
            | Type::UInt64
            | Type::Int64 => "0".into(),
            Type::Float32 | Type::Float64 => "0.0".into(),
            Type::String | Type::Bytes => "\"\"".into(),
            Type::Optional { .. } => "null".into(),
            Type::Sequence { .. } | Type::Set { .. } | Type::Map { .. } => "&.{}".into(),
            Type::Custom { builtin, .. } => return self.zero(builtin),
            _ => return None,
        })
    }

    fn literal(&self, lit: &Literal, t: &Type) -> Option<String> {
        Some(match lit {
            Literal::Boolean(b) => b.to_string(),
            Literal::String(s) => string_literal(s),
            Literal::UInt(v, radix, _) => int_literal(i128::from(*v), *radix),
            Literal::Int(v, radix, _) => int_literal(i128::from(*v), *radix),
            Literal::Float(s, _) => s.clone(),
            Literal::Enum(variant, ty) => match ty {
                Type::Enum { name, .. } => {
                    format!(".{}", snake(&self.variant_named(name, variant)))
                }
                _ => return None,
            },
            Literal::EmptySequence | Literal::EmptyMap | Literal::EmptySet => "&.{}".into(),
            Literal::None => "null".into(),
            Literal::Some { inner } => {
                let inner_type = match t {
                    Type::Optional { inner_type } => inner_type.as_ref(),
                    other => other,
                };
                return self.default_value(inner, inner_type);
            }
        })
    }

    /// The variant of enum `enum_name` a default refers to; UDL spells it in any case.
    pub(crate) fn variant_named(&self, enum_name: &str, literal: &str) -> String {
        let normal = |s: &str| s.replace('_', "").to_ascii_lowercase();
        self.ci
            .get_enum_definition(enum_name)
            .and_then(|e| {
                e.variants()
                    .iter()
                    .find(|v| normal(v.name()) == normal(literal))
            })
            .map_or_else(|| literal.to_owned(), |v| v.name().to_owned())
    }

    /// Every type the component uses, nested ones included, each once (by mangled name).
    pub(crate) fn all_types(&self) -> Vec<Type> {
        let mut roots: Vec<Type> = self
            .ci
            .iter_local_types()
            .chain(self.ci.iter_external_types())
            .cloned()
            .collect();
        for r in self.ci.record_definitions() {
            roots.extend(r.fields().iter().map(|f| f.as_type()));
        }
        for e in self.ci.enum_definitions() {
            for v in e.variants() {
                roots.extend(v.fields().iter().map(|f| f.as_type()));
            }
        }
        for c in self.ci.iter_callables() {
            roots.extend(c.arguments().iter().map(|a| a.as_type()));
            roots.extend(c.return_type().cloned());
            roots.extend(c.throws_type().cloned());
        }
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        for t in &roots {
            for inner in t.iter_types() {
                if seen.insert(self.mangle(inner)) {
                    out.push(inner.clone());
                }
            }
        }
        out
    }

    /// Every container type the component uses.
    pub(crate) fn containers(&self) -> Vec<Type> {
        self.all_types()
            .into_iter()
            .filter(Self::is_container)
            .collect()
    }

    /// Every type that crosses the FFI in a buffer somewhere in this component.
    pub(crate) fn buffer_types(&self) -> Vec<Type> {
        self.all_types()
            .into_iter()
            .filter(Self::is_buffer_type)
            .collect()
    }

    /// A function name in Zig style.
    pub(crate) fn fn_name(&self, name: &str) -> String {
        self.callable_name(name, &[])
    }
}

/// The name of a user-defined type.
fn type_name(t: &Type) -> &str {
    match t {
        Type::Object { name, .. }
        | Type::CallbackInterface { name, .. }
        | Type::Record { name, .. }
        | Type::Enum { name, .. }
        | Type::Custom { name, .. } => name,
        _ => "",
    }
}

fn camel_upper(name: &str) -> String {
    let c = camel(name);
    let mut chars = c.chars();
    chars
        .next()
        .map(|f| f.to_ascii_uppercase().to_string() + chars.as_str())
        .unwrap_or_default()
}

fn int_literal(v: i128, radix: Radix) -> String {
    if v < 0 {
        return v.to_string();
    }
    match radix {
        Radix::Decimal => v.to_string(),
        Radix::Octal => format!("0o{v:o}"),
        Radix::Hexadecimal => format!("0x{v:x}"),
    }
}
