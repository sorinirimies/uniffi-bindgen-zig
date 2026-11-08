//! The C ABI declarations of a component, in a private `ffi` namespace.

use std::fmt::Write;

use uniffi_bindgen::interface::FfiDefinition;

use super::names::ident;
use super::types::Gen;

impl Gen<'_> {
    pub(crate) fn emit_ffi(&self) -> String {
        let mut out = String::from("/// The scaffolding's C ABI.\nconst ffi = struct {\n");
        for def in self.ci.ffi_definitions() {
            match def {
                FfiDefinition::CallbackFunction(cb) => {
                    let mut args: Vec<String> = cb
                        .arguments()
                        .iter()
                        .map(|a| self.ffi_type(&a.type_()))
                        .collect();
                    if cb.has_rust_call_status_arg() {
                        args.push("*uniffi.RustCallStatus".into());
                    }
                    let ret = cb
                        .return_type()
                        .map_or_else(|| "void".to_owned(), |t| self.ffi_type(t));
                    let _ = writeln!(
                        out,
                        "    pub const {} = *const fn ({}) callconv(.c) {ret};",
                        cb.name(),
                        args.join(", ")
                    );
                }
                FfiDefinition::Struct(st) => {
                    let _ = writeln!(out, "    pub const {} = extern struct {{", st.name());
                    for field in st.fields() {
                        let _ = writeln!(
                            out,
                            "        {}: {},",
                            ident(field.name()),
                            self.ffi_type(&field.type_())
                        );
                    }
                    let _ = writeln!(out, "    }};");
                }
                FfiDefinition::Function(f) => {
                    let mut args: Vec<String> = f
                        .arguments()
                        .iter()
                        .map(|a| format!("{}: {}", ident(a.name()), self.ffi_type(&a.type_())))
                        .collect();
                    if f.has_rust_call_status_arg() {
                        args.push("out_status: *uniffi.RustCallStatus".into());
                    }
                    let ret = f
                        .return_type()
                        .map_or_else(|| "void".to_owned(), |t| self.ffi_type(t));
                    let _ = writeln!(
                        out,
                        "    pub extern fn {}({}) callconv(.c) {ret};",
                        f.name(),
                        args.join(", ")
                    );
                }
            }
        }
        out.push_str("};\n\n");
        out
    }
}
