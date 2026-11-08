//! The public declarations: enums, unions, records, objects, callback interfaces.

use std::fmt::Write;

use uniffi_bindgen::interface::{
    AsType, Callable, Constructor, Enum, Method, Object, Record, Type, UniffiTraitMethods,
};

use super::calls::Receiver;
use super::codec::indent;
use super::names::{camel, doc, field, ident, snake};
use super::types::Gen;

/// Method names the bindings define on every object, record or enum.
const RESERVED_METHODS: &[&str] = &[
    "init",
    "deinit",
    "clone",
    "fromImpl",
    "from",
    "toString",
    "debugString",
    "eql",
    "hash",
    "order",
    "toError",
    "uniffiLower",
    "uniffiLift",
    "Set",
    "VTable",
    "handle",
];

impl Gen<'_> {
    /// The Zig name of a method or non-primary constructor.
    pub(crate) fn method_name(&self, name: &str) -> String {
        self.callable_name(name, RESERVED_METHODS)
    }
}

impl Gen<'_> {
    pub(crate) fn emit_declarations(&self) -> String {
        let mut out = String::new();
        for e in self.ci.enum_definitions() {
            out.push_str(&self.emit_enum(e));
        }
        for r in self.ci.record_definitions() {
            out.push_str(&self.emit_record(r));
        }
        for t in self.all_types() {
            if let Type::Custom { name, builtin, .. } = &t
                && !self.ci.is_external(&t)
            {
                let _ = writeln!(
                    out,
                    "pub const {} = {};\n",
                    ident(self.renamed_type(name)),
                    self.zig_type(builtin)
                );
            }
        }
        for obj in self.ci.object_definitions() {
            out.push_str(&self.emit_object(obj));
        }
        for cb in self.ci.callback_interface_definitions() {
            out.push_str(&self.emit_callback_interface(cb.name(), cb.docstring(), &cb.methods()));
        }
        out
    }

    fn emit_enum(&self, e: &Enum) -> String {
        let t = e.as_type();
        let name = ident(self.renamed_type(e.name()));
        let mut out = doc(e.docstring(), 0);
        let members =
            self.value_members(&t, e.constructors(), e.methods(), &e.uniffi_trait_methods());
        if self.is_flat_enum(&t) {
            let values: Vec<i128> = (0..e.variants().len())
                .map(|i| match e.variant_discr(i) {
                    Ok(uniffi_bindgen::interface::Literal::UInt(v, ..)) => i128::from(v),
                    Ok(uniffi_bindgen::interface::Literal::Int(v, ..)) => i128::from(v),
                    _ => i as i128,
                })
                .collect();
            let repr = match e.variant_discr_type() {
                Some(t) => self.zig_type(t),
                None if values.iter().any(|v| *v > i128::from(i64::MAX)) => "u64".into(),
                None => "i64".into(),
            };
            let _ = writeln!(out, "pub const {name} = enum({repr}) {{");
            for (v, value) in e.variants().iter().zip(&values) {
                out.push_str(&doc(v.docstring(), 4));
                let _ = writeln!(out, "    {} = {value},", snake(v.name()));
            }
            out.push_str(&self.enum_deinit(e));
            out.push_str(&members);
            let _ = writeln!(out, "}};\n");
            return out;
        }
        let error = self.is_error_enum(e.name());
        let flat_error = e.is_flat();
        let _ = writeln!(out, "pub const {name} = union(enum) {{");
        for v in e.variants() {
            out.push_str(&doc(v.docstring(), 4));
            let vname = snake(v.name());
            if flat_error {
                let _ = writeln!(
                    out,
                    "    /// The error's message (its Rust `Display`).\n    {vname}: []const u8,"
                );
            } else if v.has_fields() {
                let fields: Vec<String> = v
                    .fields()
                    .iter()
                    .enumerate()
                    .map(|(j, f)| {
                        format!("{}: {}", field(f.name(), j), self.zig_type(&f.as_type()))
                    })
                    .collect();
                let _ = writeln!(out, "    {vname}: struct {{ {} }},", fields.join(", "));
            } else {
                let _ = writeln!(out, "    {vname},");
            }
        }
        if error {
            let names: Vec<String> = e.variants().iter().map(|v| ident(v.name())).collect();
            let _ = writeln!(
                out,
                "\n    pub const Set = error{{ {} }};\n",
                names.join(", ")
            );
            let mut arms = String::new();
            for v in e.variants() {
                let _ = writeln!(
                    arms,
                    "            .{} => error.{},",
                    snake(v.name()),
                    ident(v.name())
                );
            }
            let _ = writeln!(
                out,
                "    /// The error to return for this value.\n    pub fn toError(self: {name}) Set {{\n        return switch (self) {{\n{arms}        }};\n    }}\n"
            );
        }
        out.push_str(&self.enum_deinit(e));
        out.push_str(&members);
        let _ = writeln!(out, "}};\n");
        out
    }

    fn emit_record(&self, r: &Record) -> String {
        let t = r.as_type();
        let name = ident(self.renamed_type(r.name()));
        let mut out = doc(r.docstring(), 0);
        let _ = writeln!(out, "pub const {name} = struct {{");
        for f in r.fields() {
            out.push_str(&doc(f.docstring(), 4));
            let ft = f.as_type();
            let default = f
                .default_value()
                .and_then(|d| self.default_value(d, &ft))
                .map(|v| format!(" = {v}"))
                .unwrap_or_default();
            let _ = writeln!(
                out,
                "    {}: {}{default},",
                ident(f.name()),
                self.zig_type(&ft)
            );
        }
        out.push('\n');
        out.push_str(&self.record_deinit(r));
        out.push_str(&self.value_members(
            &t,
            r.constructors(),
            r.methods(),
            &r.uniffi_trait_methods(),
        ));
        let _ = writeln!(out, "}};\n");
        out
    }

    /// Constructors, methods and trait methods of a record or enum, indented.
    fn value_members(
        &self,
        t: &Type,
        constructors: &[Constructor],
        methods: &[Method],
        traits: &UniffiTraitMethods,
    ) -> String {
        let zt = self.zig_type(t);
        let receiver = Receiver::Value(t.clone());
        let mut out = String::new();
        for c in constructors {
            out.push_str(&doc(c.docstring(), 0));
            out.push_str(&self.callable(c, &self.constructor_name(c), &Receiver::None, None, ""));
        }
        for m in methods {
            out.push_str(&doc(m.docstring(), 0));
            out.push_str(&self.callable(m, &self.method_name(m.name()), &receiver, Some(&zt), ""));
        }
        out.push_str(&self.trait_members(&zt, traits, &receiver));
        if out.is_empty() {
            out
        } else {
            format!("\n{}", indent(&out, 4))
        }
    }

    fn trait_members(&self, zt: &str, traits: &UniffiTraitMethods, receiver: &Receiver) -> String {
        let mut out = String::new();
        if let Some(m) = &traits.display_fmt {
            out.push_str(&format!(
                "/// Rust's `Display`.\n{}",
                self.callable(m, "toString", receiver, Some(zt), "")
            ));
        }
        if let Some(m) = &traits.debug_fmt {
            out.push_str(&format!(
                "/// Rust's `Debug`.\n{}",
                self.callable(m, "debugString", receiver, Some(zt), "")
            ));
        }
        if let Some(m) = &traits.eq_eq {
            out.push_str(&format!(
                "/// Rust's `PartialEq`.\n{}",
                self.callable(m, "eql", receiver, Some(zt), "")
            ));
        }
        if let Some(m) = &traits.hash_hash {
            out.push_str(&format!(
                "/// Rust's `Hash`.\n{}",
                self.callable(m, "hash", receiver, Some(zt), "")
            ));
        }
        if let Some(m) = &traits.ord_cmp {
            let raw = self.callable(m, "uniffiCmp", receiver, Some(zt), "");
            out.push_str(&raw.replacen("pub fn uniffiCmp", "fn uniffiCmp", 1));
            out.push_str(&format!(
                "/// Rust's `Ord`.\npub fn order(self: {zt}, other: {zt}) uniffi.Error!std.math.Order {{\n    const c = try self.uniffiCmp(other);\n    return if (c < 0) .lt else if (c > 0) .gt else .eq;\n}}\n\n"
            ));
        }
        out
    }

    fn emit_object(&self, obj: &Object) -> String {
        let t = obj.as_type();
        let name = ident(self.renamed_type(obj.name()));
        let handles = handles_var(obj.name());
        let foreign = obj.has_callback_interface();
        let mut members = String::new();
        for c in obj.constructors() {
            members.push_str(&doc(c.docstring(), 0));
            members.push_str(&self.callable(
                c,
                &self.constructor_name(c),
                &Receiver::None,
                None,
                "",
            ));
        }
        for m in obj.methods() {
            members.push_str(&doc(m.docstring(), 0));
            let prelude = if foreign {
                self.foreign_dispatch(obj.name(), m)
            } else {
                String::new()
            };
            members.push_str(&self.callable(
                m,
                &self.method_name(m.name()),
                &Receiver::Object,
                Some(&name),
                &prelude,
            ));
        }
        members.push_str(&self.trait_members(
            &name,
            &obj.uniffi_trait_methods(),
            &Receiver::Object,
        ));
        let foreign_free = if foreign {
            format!(
                "    if (self.handle & 1 == 1) {{\n        if ({handles}.remove(self.handle)) |impl| if (impl.vtable.deinit) |release| release(impl.ptr);\n        return;\n    }}\n"
            )
        } else {
            String::new()
        };
        let foreign_clone = if foreign {
            format!(
                "    if (self.handle & 1 == 1) return .{{ .handle = {handles}.clone(self.handle) }};\n"
            )
        } else {
            String::new()
        };
        let _ = write!(
            members,
            "/// Releases this handle to the object.\npub fn deinit(self: {name}) void {{\n    if (self.handle == 0) return;\n{foreign_free}    var status: uniffi.RustCallStatus = .{{}};\n    ffi.{free}(self.handle, &status);\n}}\n\n/// A second handle to the same object, released separately.\npub fn clone(self: {name}) {name} {{\n    if (self.handle == 0) return self;\n{foreign_clone}    uniffiEnsureInit();\n    var status: uniffi.RustCallStatus = .{{}};\n    const handle = ffi.{clone}(self.handle, &status);\n    if (status.code != uniffi.call_success) @panic(\"uniffi: could not clone a handle\");\n    return .{{ .handle = handle }};\n}}\n\npub fn uniffiLower(self: {name}) u64 {{\n    return self.clone().handle;\n}}\n\n",
            free = obj.ffi_object_free().name(),
            clone = obj.ffi_object_clone().name(),
        );
        if self.ci.is_name_used_as_error(obj.name()) || self.is_thrown(&t) {
            let _ = write!(
                members,
                "pub const Set = error{{{name}}};\n\npub fn toError(self: {name}) Set {{\n    _ = self;\n    return error.{name};\n}}\n\n"
            );
        }
        if foreign {
            members.push_str(&self.vtable_decl(obj.name(), &obj.methods(), true));
            members.push_str(&self.impl_constructor(obj.name(), &obj.methods(), true));
        }
        let mut out = doc(obj.docstring(), 0);
        let _ = write!(
            out,
            "pub const {name} = struct {{\n    /// Owned: release with `deinit`.\n    handle: u64,\n\n{}}};\n\n",
            indent(&members, 4)
        );
        out
    }

    fn is_thrown(&self, t: &Type) -> bool {
        self.all_callables()
            .iter()
            .any(|c| c.throws_type() == Some(t))
    }

    /// Calls the Zig implementation directly when `self` is one (an odd handle).
    fn foreign_dispatch(&self, type_name: &str, m: &Method) -> String {
        let handles = handles_var(type_name);
        let alloc = if self.takes_allocator(m) {
            "allocator"
        } else {
            "uniffi.allocator"
        };
        let mut args = vec!["impl.ptr".to_owned(), alloc.to_owned()];
        args.extend(m.arguments().iter().map(|a| self.arg(a.name())));
        let mut out = format!(
            "    if (self.handle & 1 == 1) {{\n        const impl = {handles}.get(self.handle) orelse return error.Internal;\n"
        );
        let catch = match m.throws_type() {
            Some(t) => {
                let _ = writeln!(out, "        var local_err: ?{} = null;", self.zig_type(t));
                args.push("&local_err".to_owned());
                let free = self.free_stmt(t, "le").map(|f| format!(" else {}", f.trim_end_matches(';'))).unwrap_or_default();
                let free = free.replace("(a)", &format!("({alloc})")).replace("a.", &format!("{alloc}."));
                format!(" catch |e| {{\n            if (local_err) |le| {{\n                if (err) |p| p.* = le{free};\n                return le.toError();\n            }}\n            uniffi.setLastErrorMessage(@errorName(e));\n            return error.RustPanic;\n        }}")
            }
            None => " catch |e| {\n            uniffi.setLastErrorMessage(@errorName(e));\n            return error.RustPanic;\n        }".to_owned(),
        };
        let _ = writeln!(
            out,
            "        return impl.vtable.{}({}){catch};\n    }}",
            camel(m.name()),
            args.join(", ")
        );
        out
    }

    /// `pub const VTable`: what a Zig implementation provides.
    pub(crate) fn vtable_decl(
        &self,
        _type_name: &str,
        methods: &[&Method],
        releasable: bool,
    ) -> String {
        let mut out = String::from(
            "/// The functions a Zig implementation provides; see `from`.\npub const VTable = struct {\n",
        );
        for m in methods {
            out.push_str(&doc(m.docstring(), 4));
            let _ = writeln!(
                out,
                "    {}: *const fn ({}) anyerror!{},",
                camel(m.name()),
                self.impl_params(m, "ptr: *anyopaque").join(", "),
                self.return_zig(m)
            );
        }
        if releasable {
            let _ = writeln!(
                out,
                "    /// Called once Rust and Zig hold no handle to the object any more.\n    deinit: ?*const fn (ptr: *anyopaque) void = null,"
            );
        }
        out.push_str("};\n\n");
        out
    }

    /// `(receiver, allocator, args..., err)` of an implementation's method.
    pub(crate) fn impl_params(&self, m: &Method, receiver: &str) -> Vec<String> {
        let mut params = vec![
            receiver.to_owned(),
            "allocator: std.mem.Allocator".to_owned(),
        ];
        for a in m.arguments() {
            params.push(format!(
                "{}: {}",
                self.arg(a.name()),
                self.zig_type(&a.as_type())
            ));
        }
        if let Some(t) = m.throws_type() {
            params.push(format!("err: *?{}", self.zig_type(t)));
        }
        params
    }

    pub(crate) fn return_zig(&self, m: &Method) -> String {
        m.return_type()
            .map_or_else(|| "void".to_owned(), |t| self.zig_type(t))
    }

    /// `from(impl)` (callback interfaces) or `fromImpl(impl)` (traits): a vtable
    /// built at compile time from the methods of `impl`'s type.
    pub(crate) fn impl_constructor(
        &self,
        type_name: &str,
        methods: &[&Method],
        object: bool,
    ) -> String {
        let name = ident(self.renamed_type(type_name));
        let mut impls = String::new();
        let mut entries = Vec::new();
        for m in methods {
            let mname = camel(m.name());
            let raw = super::names::unquote(&mname).to_owned();
            let params = self.impl_params(m, "p: *anyopaque");
            let mut call_args = vec!["allocator".to_owned()];
            call_args.extend(m.arguments().iter().map(|a| self.arg(a.name())));
            if m.throws_type().is_some() {
                call_args.push("err".to_owned());
            }
            let _ = writeln!(
                impls,
                "        fn impl_{raw}({}) anyerror!{} {{\n            const self: *Impl = @ptrCast(@alignCast(p));\n            return self.{mname}({});\n        }}",
                params.join(", "),
                self.return_zig(m),
                call_args.join(", ")
            );
            entries.push(format!(".{mname} = impl_{raw}"));
        }
        if object {
            let _ = writeln!(
                impls,
                "        fn impl_release(p: *anyopaque) void {{\n            const self: *Impl = @ptrCast(@alignCast(p));\n            if (@hasDecl(Impl, \"deinit\")) self.deinit();\n        }}"
            );
            entries.push(".deinit = impl_release".to_owned());
        }
        let vtable = format!(
            "        const vtable: VTable = .{{ {} }};\n",
            entries.join(", ")
        );
        if object {
            format!(
                "/// An object implemented in Zig by `impl`, a pointer to any type with these\n/// methods (and optionally `deinit`, called once no handle is left).\npub fn fromImpl(impl: anytype) {name} {{\n    const Impl = @TypeOf(impl.*);\n    const gen = struct {{\n{impls}{vtable}    }};\n    return .{{ .handle = {}.insert(.{{ .ptr = impl, .vtable = &gen.vtable }}) }};\n}}\n\n",
                handles_var(type_name)
            )
        } else {
            format!(
                "/// A `{name}` calling `impl`, a pointer to any type with these methods.\npub fn from(impl: anytype) {name} {{\n    const Impl = @TypeOf(impl.*);\n    const gen = struct {{\n{impls}{vtable}    }};\n    return .{{ .ptr = impl, .vtable = &gen.vtable }};\n}}\n\n"
            )
        }
    }

    fn emit_callback_interface(
        &self,
        name: &str,
        docstring: Option<&str>,
        methods: &[&Method],
    ) -> String {
        let zname = ident(self.renamed_type(name));
        let handles = handles_var(name);
        let mut members = self.vtable_decl(name, methods, false);
        members.push_str(&self.impl_constructor(name, methods, false));
        let _ = write!(
            members,
            "pub fn uniffiLower(v: {zname}) u64 {{\n    return {handles}.insert(.{{ .ptr = v.ptr, .vtable = v.vtable }});\n}}\n\npub fn uniffiLift(handle: u64) {zname} {{\n    const impl = {handles}.remove(handle) orelse {handles}.get(handle) orelse @panic(\"uniffi: unknown callback handle\");\n    return .{{ .ptr = impl.ptr, .vtable = impl.vtable }};\n}}\n\n"
        );
        let mut out = doc(docstring, 0);
        let _ = write!(
            out,
            "pub const {zname} = struct {{\n    ptr: *anyopaque,\n    vtable: *const VTable,\n\n{}}};\n\n",
            indent(&members, 4)
        );
        out
    }
}

impl Gen<'_> {
    /// The Zig name of a constructor: `init` for the primary one.
    fn constructor_name(&self, c: &Constructor) -> String {
        if c.is_primary_constructor() {
            "init".to_owned()
        } else {
            self.method_name(c.name())
        }
    }
}

/// The private handle map of a Zig-implementable interface or trait.
pub(crate) fn handles_var(type_name: &str) -> String {
    format!(
        "{}_handles",
        snake(type_name)
            .trim_start_matches("@\"")
            .trim_end_matches('"')
    )
}
