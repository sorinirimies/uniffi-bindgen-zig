//! Zig implementations of callback interfaces and traits: the handle maps and
//! the C-callable wrappers Rust calls through the vtable.

use std::fmt::Write;

use uniffi_bindgen::interface::{AsType, FfiCallbackFunction, FfiDefinition, Method, Type};

use super::decls::handles_var;
use super::names::{camel, discard_unused, ident, snake, uses};
use super::types::Gen;

impl Gen<'_> {
    pub(crate) fn emit_callbacks(&self) -> String {
        let mut out = String::new();
        for cb in self.ci.callback_interface_definitions() {
            out.push_str(&self.emit_vtable_impl(
                cb.name(),
                &cb.vtable_methods(),
                cb.ffi_init_callback().name(),
                &self.ffi_type(&cb.vtable()),
                false,
            ));
        }
        for obj in self.ci.object_definitions() {
            if obj.has_callback_interface()
                && let Some(vtable) = obj.vtable()
            {
                out.push_str(&self.emit_vtable_impl(
                    obj.name(),
                    &obj.vtable_methods(),
                    obj.ffi_init_callback().name(),
                    &self.ffi_type(&vtable),
                    true,
                ));
            }
        }
        // Objects Rust throws arrive as a handle in a buffer.
        for t in self.all_types() {
            if let Type::Object { .. } = &t
                && self
                    .all_callables()
                    .iter()
                    .any(|c| c.throws_type() == Some(&t))
            {
                let _ = write!(
                    out,
                    "fn liftHandle{m}(buffer: uniffi.RustBuffer) uniffi.Error!{zt} {{\n    defer bufferFree(buffer);\n    const data: []const u8 = if (buffer.data) |d| d[0..@intCast(buffer.len)] else &.{{}};\n    var r: uniffi.Reader = .{{ .data = data }};\n    return .{{ .handle = try r.int(u64) }};\n}}\n\n",
                    m = self.mangle(&t),
                    zt = self.zig_type(&t)
                );
            }
        }
        out
    }

    /// The names of the vtable struct's fields, in order.
    fn vtable_fields(&self, vtable_type: &str) -> Vec<String> {
        self.ci
            .ffi_definitions()
            .find_map(|d| match d {
                FfiDefinition::Struct(st)
                    if Some(st.name()) == vtable_type.strip_prefix("ffi.") =>
                {
                    Some(st.fields().iter().map(|f| ident(f.name())).collect())
                }
                _ => None,
            })
            .unwrap_or_default()
    }

    fn emit_vtable_impl(
        &self,
        name: &str,
        methods: &[(FfiCallbackFunction, Method)],
        init_fn: &str,
        vtable_type: &str,
        object: bool,
    ) -> String {
        let zname = ident(self.renamed_type(name));
        let base = snake(name)
            .trim_start_matches("@\"")
            .trim_end_matches('"')
            .to_owned();
        let handles = handles_var(name);
        let raw = super::names::unquote(&zname).to_owned();
        let mut out = format!(
            "const {raw}Impl = struct {{\n    ptr: *anyopaque,\n    vtable: *const {zname}.VTable,\n}};\n\nvar {handles}: uniffi.HandleMap({raw}Impl) = .{{}};\n\n"
        );
        let release = if object {
            format!(
                "    if ({handles}.remove(handle)) |impl| if (impl.vtable.deinit) |release| release(impl.ptr);\n"
            )
        } else {
            format!("    _ = {handles}.remove(handle);\n")
        };
        let _ = write!(
            out,
            "fn {base}Free(handle: u64) callconv(.c) void {{\n{release}}}\n\nfn {base}Clone(handle: u64) callconv(.c) u64 {{\n    return {handles}.clone(handle);\n}}\n\n"
        );
        let mut entries = vec![format!("{base}Free"), format!("{base}Clone")];
        for (i, (callback, m)) in methods.iter().enumerate() {
            let fname = format!("{base}M{i}");
            if m.is_async() {
                out.push_str(&self.async_wrapper(&fname, &handles, callback, m));
            } else {
                out.push_str(&self.sync_wrapper(&fname, &handles, callback, m));
            }
            entries.push(fname);
        }
        let fields = self.vtable_fields(vtable_type);
        let inits: Vec<String> = fields
            .iter()
            .zip(&entries)
            .map(|(f, e)| format!(".{f} = {e}"))
            .collect();
        let _ = write!(
            out,
            "var {base}_ffi_vtable: {vtable_type} = .{{ {} }};\n\nfn {base}Init() void {{\n    ffi.{init_fn}(&{base}_ffi_vtable);\n}}\n\n",
            inits.join(", ")
        );
        out
    }

    fn wrapper_params(
        callback: &FfiCallbackFunction,
        method: &Method,
        g: &Gen,
    ) -> (Vec<String>, Vec<String>) {
        let n_args = method.arguments().len();
        let mut names = Vec::new();
        let mut params = Vec::new();
        for (i, a) in callback.arguments().iter().enumerate() {
            let name = match i {
                0 => "uniffi_handle".to_owned(),
                i if i <= n_args => format!("arg{}", i - 1),
                _ => a.name().to_owned(),
            };
            params.push(format!("{name}: {}", g.ffi_type(&a.type_())));
            names.push(name);
        }
        if callback.has_rust_call_status_arg() {
            params.push("uniffi_call_status: *uniffi.RustCallStatus".to_owned());
            names.push("uniffi_call_status".to_owned());
        }
        (names, params)
    }

    /// Lifts `arg<i>` into `<holder>a<i>`, freeing on return; `fail` reports a bad argument.
    fn lift_args(&self, m: &Method, holder: &str, declare: bool, fail: &str) -> String {
        let mut out = String::new();
        for (i, a) in m.arguments().iter().enumerate() {
            let t = a.as_type();
            let lifted = self.lift(&t, &format!("arg{i}"));
            let expr = match lifted.strip_prefix("try ") {
                Some(e) => format!("{e} catch {fail}"),
                None => lifted,
            };
            let decl = if declare { "const " } else { "" };
            let _ = writeln!(out, "    {decl}{holder}a{i} = {expr};");
            if declare && let Some(free) = self.free_stmt(&t, &format!("a{i}")) {
                let _ = writeln!(out, "    defer {free}");
            }
        }
        out
    }

    /// Reports a failed Zig implementation through `status` (a RustCallStatus lvalue).
    fn report_error(&self, m: &Method, status: &str) -> String {
        let mut out = String::new();
        if let Some(t) = m.throws_type() {
            let lower = if Self::is_buffer_type(t) {
                self.lower(t, "ev")
            } else {
                "blk: {\n                var w: uniffi.Writer = .{};\n                defer w.deinit();\n                w.int(u64, ev.uniffiLower());\n                break :blk bufferFrom(w.list.items);\n            }".to_owned()
            };
            let free = self
                .free_stmt(t, "ev")
                .map(|f| format!("            {f}\n"))
                .unwrap_or_default();
            let _ = write!(
                out,
                "        if (err_value) |ev| {{\n            {status}.code = uniffi.call_error;\n            {status}.error_buf = {lower};\n{free}            return;\n        }}\n"
            );
            if let Type::Enum { name, .. } = t
                && let Some(e) = self.ci.get_enum_definition(name)
                && !e.is_flat()
            {
                for v in e.variants().iter().filter(|v| !v.has_fields()) {
                    let _ = write!(
                        out,
                        "        if (std.mem.eql(u8, @errorName(e), \"{}\")) {{\n            {status}.code = uniffi.call_error;\n            {status}.error_buf = {};\n            return;\n        }}\n",
                        v.name(),
                        self.lower(
                            t,
                            &format!("@as({}, .{})", self.zig_type(t), snake(v.name()))
                        )
                    );
                }
            }
        }
        let status_ptr = if status.contains('.') {
            format!("&{status}")
        } else {
            status.to_owned()
        };
        let _ = write!(
            out,
            "        uniffi.callbackPanic({status_ptr}, @errorName(e), bufferFrom);\n        return;\n"
        );
        out
    }

    fn invoke(&self, m: &Method, holder: &str) -> String {
        let mut args = vec!["impl.ptr".to_owned(), "a".to_owned()];
        args.extend((0..m.arguments().len()).map(|i| format!("{holder}a{i}")));
        if m.throws_type().is_some() {
            args.push("&err_value".to_owned());
        }
        format!("impl.vtable.{}({})", camel(m.name()), args.join(", "))
    }

    fn sync_wrapper(
        &self,
        fname: &str,
        handles: &str,
        callback: &FfiCallbackFunction,
        m: &Method,
    ) -> String {
        let (names, params) = Self::wrapper_params(callback, m, self);
        let fail = "return uniffi.callbackPanic(uniffi_call_status, \"could not read an argument\", bufferFrom)";
        let mut body = String::new();
        body.push_str(&self.lift_args(m, "", true, fail));
        if let Some(t) = m.throws_type() {
            let _ = writeln!(body, "    var err_value: ?{} = null;", self.zig_type(t));
        }
        let call = self.invoke(m, "");
        let report = self.report_error(m, "uniffi_call_status");
        match m.return_type() {
            Some(t) => {
                let _ = writeln!(
                    body,
                    "    const result = {call} catch |e| {{\n{report}    }};"
                );
                let _ = writeln!(
                    body,
                    "    uniffi_out_return.* = {};",
                    self.lower(t, "result")
                );
                if let Some(free) = self.free_stmt(t, "result") {
                    let _ = writeln!(body, "    {free}");
                }
            }
            None => {
                let _ = writeln!(body, "    {call} catch |e| {{\n{report}    }};");
            }
        }
        let alloc = if uses(&body, "a") {
            "    const a = uniffi.allocator;\n"
        } else {
            ""
        };
        let body = format!(
            "    const impl = {handles}.get(uniffi_handle) orelse return uniffi.callbackPanic(uniffi_call_status, \"the object was released already\", bufferFrom);\n{alloc}{body}"
        );
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        format!(
            "fn {fname}({}) callconv(.c) void {{\n{}}}\n\n",
            params.join(", "),
            discard_unused(&body, &refs)
        )
    }

    fn async_wrapper(
        &self,
        fname: &str,
        handles: &str,
        callback: &FfiCallbackFunction,
        m: &Method,
    ) -> String {
        let (names, params) = Self::wrapper_params(callback, m, self);
        let job = format!("{fname}Job");
        let result_struct = m.foreign_future_ffi_result_struct().name().to_owned();
        let complete_type = callback
            .arguments()
            .iter()
            .find(|a| a.name() == "uniffi_future_callback")
            .map(|a| self.ffi_type(&a.type_()))
            .unwrap_or_default();
        let mut fields = String::new();
        for (j, a) in m.arguments().iter().enumerate() {
            let _ = writeln!(fields, "    a{j}: {},", self.zig_type(&a.as_type()));
        }
        let mut run = format!(
            "    defer uniffi.allocator.destroy(job);\n    var result = std.mem.zeroes(ffi.{result_struct});\n"
        );
        let mut frees = String::new();
        for (j, a) in m.arguments().iter().enumerate() {
            if let Some(free) = self.free_stmt(&a.as_type(), &format!("job.a{j}")) {
                let _ = writeln!(frees, "    {free}");
            }
        }
        let _ = writeln!(
            run,
            "    defer {{\n    {}    }}",
            frees.replace('\n', "\n    ").trim_end_matches(' ')
        );
        let free_fn = format!("{}Free", fname.rsplit_once('M').map_or(fname, |(b, _)| b));
        let _ = writeln!(
            run,
            "    defer {{\n        {free_fn}(job.handle);\n        job.complete(job.data, result);\n    }}"
        );
        let _ = writeln!(
            run,
            "    const impl = {handles}.get(job.handle) orelse {{\n        uniffi.callbackPanic(&result.call_status, \"the object was released already\", bufferFrom);\n        return;\n    }};"
        );
        if let Some(t) = m.throws_type() {
            let _ = writeln!(run, "    var err_value: ?{} = null;", self.zig_type(t));
        }
        let call = self.invoke(m, "job.");
        let report = self.report_error(m, "result.call_status");
        match m.return_type() {
            Some(t) => {
                let _ = writeln!(
                    run,
                    "    const value = {call} catch |e| {{\n{report}    }};"
                );
                let _ = writeln!(run, "    result.return_value = {};", self.lower(t, "value"));
                if let Some(free) = self.free_stmt(t, "value") {
                    let _ = writeln!(run, "    {free}");
                }
            }
            None => {
                let _ = writeln!(run, "    {call} catch |e| {{\n{report}    }};");
            }
        }
        let fail = "{\n        uniffi.allocator.destroy(job);\n        return;\n    }";
        let mut start = format!(
            "    uniffi_out_dropped_callback.* = .{{ .handle = 0, .free = droppedFuture }};\n    const job = uniffi.allocator.create({job}) catch uniffi.oom();\n    job.handle = {handles}.clone(uniffi_handle);\n"
        );
        start.push_str(&self.lift_args(m, "job.", false, fail));
        start.push_str(
            "    job.complete = uniffi_future_callback;\n    job.data = uniffi_callback_data;\n",
        );
        let _ = writeln!(start, "    uniffi.spawn({job}Run, .{{job}});");
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let with_alloc = |body: String| {
            if uses(&body, "a") {
                format!("    const a = uniffi.allocator;\n{body}")
            } else {
                body
            }
        };
        let run = with_alloc(run);
        let start = with_alloc(start);
        format!(
            "const {job} = struct {{\n    handle: u64,\n{fields}    complete: {complete_type},\n    data: u64,\n}};\n\nfn {job}Run(job: *{job}) void {{\n{}}}\n\nfn {fname}({}) callconv(.c) void {{\n{}}}\n\n",
            discard_unused(&run, &["job"]),
            params.join(", "),
            discard_unused(&start, &refs)
        )
    }
}
