//! Function bodies that call into Rust (blocking on async ones), and their signatures.

use std::fmt::Write;

use uniffi_bindgen::interface::{AsType, Callable, Type};

use super::names::{discard_unused, uses};
use super::types::Gen;

/// How a method passes its receiver.
pub(crate) enum Receiver {
    None,
    /// An object: its handle.
    Object,
    /// A record or enum: lowered by value.
    Value(Type),
}

impl Gen<'_> {
    /// Every function, method and constructor of the component.
    pub(crate) fn all_callables(&self) -> Vec<&dyn Callable> {
        let mut out: Vec<&dyn Callable> = Vec::new();
        for f in self.ci.function_definitions() {
            out.push(f);
        }
        for o in self.ci.object_definitions() {
            o.constructors().into_iter().for_each(|c| out.push(c));
            o.methods().into_iter().for_each(|m| out.push(m));
        }
        for r in self.ci.record_definitions() {
            r.constructors().iter().for_each(|c| out.push(c));
            r.methods().iter().for_each(|m| out.push(m));
        }
        for e in self.ci.enum_definitions() {
            e.constructors().iter().for_each(|c| out.push(c));
            e.methods().iter().for_each(|m| out.push(m));
        }
        for cb in self.ci.callback_interface_definitions() {
            cb.methods().into_iter().for_each(|m| out.push(m));
        }
        out
    }

    /// Whether a call returns (or throws) something that needs an allocator.
    pub(crate) fn takes_allocator(&self, c: &dyn Callable) -> bool {
        c.return_type().is_some_and(|t| self.needs_alloc(t))
            || c.throws_type().is_some_and(|t| self.needs_alloc(t))
    }

    /// `E.Set` of the error type `t`.
    pub(crate) fn error_set(&self, t: &Type) -> String {
        format!("{}.Set", self.zig_type(t))
    }

    /// The return type: `(E.Set || uniffi.Error)!T` or `uniffi.Error!T`.
    pub(crate) fn return_type(&self, c: &dyn Callable) -> String {
        let ok = c
            .return_type()
            .map_or_else(|| "void".to_owned(), |t| self.zig_type(t));
        match c.throws_type() {
            Some(t) => format!("({} || uniffi.Error)!{ok}", self.error_set(t)),
            None => format!("uniffi.Error!{ok}"),
        }
    }

    /// Parameters: the receiver, `allocator`, the arguments, then `err`.
    pub(crate) fn params(
        &self,
        c: &dyn Callable,
        receiver: Option<(&str, String)>,
    ) -> Vec<(String, String)> {
        // The receiver first, so methods can be called as `value.method(...)`.
        let mut out = Vec::new();
        if let Some((name, ty)) = receiver {
            out.push((name.to_owned(), ty));
        }
        if self.takes_allocator(c) {
            out.push(("allocator".to_owned(), "std.mem.Allocator".to_owned()));
        }
        for a in c.arguments() {
            out.push((self.arg(a.name()), self.zig_type(&a.as_type())));
        }
        if let Some(t) = c.throws_type() {
            out.push(("err".to_owned(), format!("?*{}", self.zig_type(t))));
        }
        out
    }

    /// A public function or method calling `c`.
    pub(crate) fn callable(
        &self,
        c: &dyn Callable,
        name: &str,
        receiver: &Receiver,
        self_type: Option<&str>,
        prelude: &str,
    ) -> String {
        let recv = match receiver {
            Receiver::None => None,
            _ => Some(("self", self_type.unwrap_or_default().to_owned())),
        };
        let params = self.params(c, recv);
        let body = format!("{prelude}{}", self.call_body(c, receiver));
        let body = if uses(&body, "a") {
            let source = if params.iter().any(|(n, _)| n == "allocator") {
                "allocator"
            } else {
                "uniffi.allocator"
            };
            format!("    const a = {source};\n{body}")
        } else {
            body
        };
        let names: Vec<&str> = params.iter().map(|(n, _)| n.as_str()).collect();
        let list: Vec<String> = params.iter().map(|(n, t)| format!("{n}: {t}")).collect();
        format!(
            "pub fn {name}({}) {} {{\n{}}}\n\n",
            list.join(", "),
            self.return_type(c),
            discard_unused(&body, &names)
        )
    }

    fn lowered_args(&self, c: &dyn Callable, receiver: &Receiver) -> Vec<String> {
        let mut args = Vec::new();
        match receiver {
            Receiver::None => {}
            Receiver::Object => args.push("self.uniffiLower()".to_owned()),
            Receiver::Value(t) => args.push(self.lower(t, "self")),
        }
        for a in c.arguments() {
            let name = self.arg(a.name());
            if a.is_borrowed_bytes() {
                args.push(format!(
                    "uniffi.ForeignBytes{{ .len = @intCast({name}.len), .data = {name}.ptr }}"
                ));
            } else {
                args.push(self.lower(&a.as_type(), &name));
            }
        }
        args
    }

    /// Calls the scaffolding, checks the status and lifts the result.
    fn call_body(&self, c: &dyn Callable, receiver: &Receiver) -> String {
        let mut out = String::from("    uniffiEnsureInit();\n");
        let args = self.lowered_args(c, receiver);
        let has_return = c.return_type().is_some();
        if c.is_async() {
            let _ = writeln!(
                out,
                "    const future = ffi.{}({});",
                c.ffi_func().name(),
                args.join(", ")
            );
            let _ = writeln!(
                out,
                "    uniffi.blockOn(future, &ffi.{});",
                c.ffi_rust_future_poll(self.ci)
            );
            let _ = writeln!(out, "    var status: uniffi.RustCallStatus = .{{}};");
            let complete = c.ffi_rust_future_complete(self.ci);
            if has_return {
                let _ = writeln!(out, "    const raw = ffi.{complete}(future, &status);");
            } else {
                let _ = writeln!(out, "    ffi.{complete}(future, &status);");
            }
            let _ = writeln!(out, "    ffi.{}(future);", c.ffi_rust_future_free(self.ci));
        } else {
            let _ = writeln!(out, "    var status: uniffi.RustCallStatus = .{{}};");
            let mut all = args;
            all.push("&status".to_owned());
            if has_return {
                let _ = writeln!(
                    out,
                    "    const raw = ffi.{}({});",
                    c.ffi_func().name(),
                    all.join(", ")
                );
            } else {
                let _ = writeln!(out, "    ffi.{}({});", c.ffi_func().name(), all.join(", "));
            }
        }
        out.push_str(&self.check_status(c.throws_type()));
        if let Some(t) = c.return_type() {
            let _ = writeln!(out, "    return {};", self.lift(t, "raw"));
        }
        out
    }

    /// Turns `status` into an error of the declared type or `uniffi.Error`.
    pub(crate) fn check_status(&self, throws: Option<&Type>) -> String {
        match throws {
            None => "    if (status.code != uniffi.call_success) return uniffi.unexpected(&status, bufferFree);\n".to_owned(),
            Some(t) => {
                let free = self.free_stmt(t, "e").map(|f| format!(" else {}", f.trim_end_matches(';'))).unwrap_or_default();
                let lift = if Self::is_buffer_type(t) {
                    self.lift(t, "status.error_buf")
                } else {
                    // An object error: its handle in a buffer.
                    format!("try liftHandle{}(status.error_buf)", self.mangle(t))
                };
                format!(
                    "    switch (status.code) {{\n        uniffi.call_success => {{}},\n        uniffi.call_error => {{\n            const e = {lift};\n            if (err) |p| p.* = e{free};\n            return e.toError();\n        }},\n        else => return uniffi.unexpected(&status, bufferFree),\n    }}\n"
                )
            }
        }
    }
}
