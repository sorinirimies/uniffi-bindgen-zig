//! Serialization of every type, releasing values, and lowering/lifting
//! buffer-borne values across the FFI.

use std::fmt::Write;

use uniffi_bindgen::interface::{AsType, Enum, Record, Type};

use super::names::{discard_unused, field, ident, snake};
use super::types::Gen;

impl Gen<'_> {
    /// `fn name(params) ret {body}` with unused parameters discarded.
    pub(crate) fn func(
        &self,
        visibility: &str,
        name: &str,
        params: &[(&str, String)],
        ret: &str,
        body: &str,
    ) -> String {
        let names: Vec<&str> = params.iter().map(|(n, _)| *n).collect();
        let list: Vec<String> = params.iter().map(|(n, t)| format!("{n}: {t}")).collect();
        format!(
            "{visibility}fn {name}({}) {ret} {{\n{}}}\n\n",
            list.join(", "),
            discard_unused(body, &names)
        )
    }

    pub(crate) fn emit_codecs(&self) -> String {
        let mut out = String::from("// ---- Serialization ----\n\n");
        for r in self.ci.record_definitions() {
            out.push_str(&self.record_codec(r));
        }
        for e in self.ci.enum_definitions() {
            out.push_str(&self.enum_codec(e));
        }
        for t in self.all_types() {
            if let Type::Custom { builtin, .. } = &t
                && !self.ci.is_external(&t)
            {
                let zt = self.zig_type(&t);
                let name = self.renamed_type(custom_name(&t));
                out.push_str(&self.func(
                    "pub ",
                    &format!("uniffiWrite{name}"),
                    &[("w", "*uniffi.Writer".into()), ("v", zt.clone())],
                    "void",
                    &format!("    {}\n", self.write_stmt(builtin, "v")),
                ));
                out.push_str(&self.func(
                    "pub ",
                    &format!("uniffiRead{name}"),
                    &[
                        ("r", "*uniffi.Reader".into()),
                        ("a", "std.mem.Allocator".into()),
                    ],
                    &format!("uniffi.Error!{zt}"),
                    &format!("    return {};\n", self.read_expr(builtin)),
                ));
            }
        }
        for obj in self.ci.object_definitions() {
            out.push_str(&self.handle_codec(&obj.as_type()));
        }
        for cb in self.ci.callback_interface_definitions() {
            out.push_str(&self.handle_codec(&cb.as_type()));
        }
        for t in self.containers() {
            out.push_str(&self.container_codec(&t));
        }
        for t in self.buffer_types() {
            out.push_str(&self.lower_lift(&t));
        }
        out
    }

    fn handle_codec(&self, t: &Type) -> String {
        let zt = self.zig_type(t);
        let name = custom_name(t);
        let mut out = self.func(
            "pub ",
            &format!("uniffiWrite{name}"),
            &[("w", "*uniffi.Writer".into()), ("v", zt.clone())],
            "void",
            &format!("    w.int(u64, {});\n", self.lower(t, "v")),
        );
        out.push_str(&self.func(
            "pub ",
            &format!("uniffiRead{name}"),
            &[
                ("r", "*uniffi.Reader".into()),
                ("a", "std.mem.Allocator".into()),
            ],
            &format!("uniffi.Error!{zt}"),
            &format!("    return {};\n", self.lift(t, "try r.int(u64)")),
        ));
        out
    }

    fn record_codec(&self, r: &Record) -> String {
        let t = r.as_type();
        let zt = self.zig_type(&t);
        let name = self.renamed_type(r.name());
        let mut write = String::new();
        let mut read = if r.fields().is_empty() {
            format!("    const v: {zt} = .{{}};\n")
        } else {
            format!("    var v: {zt} = undefined;\n")
        };
        for f in r.fields() {
            let ft = f.as_type();
            let fname = ident(f.name());
            let _ = writeln!(write, "    {}", self.write_stmt(&ft, &format!("v.{fname}")));
            let _ = writeln!(read, "    v.{fname} = {};", self.read_expr(&ft));
            if let Some(free) = self.free_stmt(&ft, &format!("v.{fname}")) {
                let _ = writeln!(read, "    errdefer {free}");
            }
        }
        read.push_str("    return v;\n");
        let mut out = self.func(
            "pub ",
            &format!("uniffiWrite{name}"),
            &[("w", "*uniffi.Writer".into()), ("v", zt.clone())],
            "void",
            &write,
        );
        out.push_str(&self.func(
            "pub ",
            &format!("uniffiRead{name}"),
            &[
                ("r", "*uniffi.Reader".into()),
                ("a", "std.mem.Allocator".into()),
            ],
            &format!("uniffi.Error!{zt}"),
            &read,
        ));
        out
    }

    /// The `deinit` method of a record, inside its declaration.
    pub(crate) fn record_deinit(&self, r: &Record) -> String {
        let zt = self.zig_type(&r.as_type());
        let mut body = String::new();
        for f in r.fields() {
            if let Some(free) = self.free_stmt(&f.as_type(), &format!("self.{}", ident(f.name()))) {
                let _ = writeln!(body, "    {free}");
            }
        }
        let body = body
            .replace("\n    ", "\n        ")
            .replacen("    ", "        ", 1);
        indent(
            &self.func(
                "pub ",
                "deinit",
                &[
                    ("self", format!("*const {zt}")),
                    ("a", "std.mem.Allocator".into()),
                ],
                "void",
                &body,
            ),
            4,
        )
    }

    fn enum_codec(&self, e: &Enum) -> String {
        let t = e.as_type();
        let zt = self.zig_type(&t);
        let name = self.renamed_type(e.name());
        let mut write = String::from("    switch (v) {\n");
        let mut read = String::from("    return switch (try r.int(i32)) {\n");
        let flat = self.is_flat_enum(&t);
        let flat_error = e.is_flat() && !flat;
        for (i, v) in e.variants().iter().enumerate() {
            let vname = snake(v.name());
            if flat {
                let _ = writeln!(write, "        .{vname} => w.int(i32, {}),", i + 1);
                let _ = writeln!(read, "        {} => .{vname},", i + 1);
            } else if flat_error {
                let _ = writeln!(write, "        .{vname} => w.int(i32, {}),", i + 1);
                let _ = writeln!(
                    read,
                    "        {} => .{{ .{vname} = try r.string(a) }},",
                    i + 1
                );
            } else if v.has_fields() {
                let _ = writeln!(
                    write,
                    "        .{vname} => |x| {{\n            w.int(i32, {});",
                    i + 1
                );
                let mut fields = String::new();
                let mut reads = String::new();
                for (j, f) in v.fields().iter().enumerate() {
                    let ft = f.as_type();
                    let fname = field(f.name(), j);
                    let _ = writeln!(
                        write,
                        "            {}",
                        self.write_stmt(&ft, &format!("x.{fname}"))
                    );
                    let _ = writeln!(reads, "            const f{j} = {};", self.read_expr(&ft));
                    if let Some(free) = self.free_stmt(&ft, &format!("f{j}")) {
                        let _ = writeln!(reads, "            errdefer {free}");
                    }
                    let _ = write!(fields, ".{fname} = f{j}, ");
                }
                let _ = writeln!(write, "        }},");
                let _ = writeln!(
                    read,
                    "        {} => blk: {{\n{reads}            break :blk .{{ .{vname} = .{{ {fields}}} }};\n        }},",
                    i + 1
                );
            } else {
                let _ = writeln!(write, "        .{vname} => w.int(i32, {}),", i + 1);
                let _ = writeln!(read, "        {} => .{vname},", i + 1);
            }
        }
        if e.variants().is_empty() {
            write = "    _ = v;\n    w.int(i32, 0);\n".into();
        } else {
            write.push_str("    }\n");
        }
        read.push_str("        else => error.Internal,\n    };\n");
        let mut out = self.func(
            "pub ",
            &format!("uniffiWrite{name}"),
            &[("w", "*uniffi.Writer".into()), ("v", zt.clone())],
            "void",
            &write,
        );
        out.push_str(&self.func(
            "pub ",
            &format!("uniffiRead{name}"),
            &[
                ("r", "*uniffi.Reader".into()),
                ("a", "std.mem.Allocator".into()),
            ],
            &format!("uniffi.Error!{zt}"),
            &read,
        ));
        out
    }

    /// The `deinit` method of an enum, inside its declaration.
    pub(crate) fn enum_deinit(&self, e: &Enum) -> String {
        let t = e.as_type();
        let zt = self.zig_type(&t);
        let mut body = String::new();
        if e.is_flat() && !self.is_flat_enum(&t) {
            body.push_str(
                "        switch (self.*) {\n            inline else => |m| a.free(m),\n        }\n",
            );
        } else if !self.is_flat_enum(&t) {
            let mut arms = String::new();
            let mut covered = 0;
            for v in e.variants() {
                let mut frees = String::new();
                for (j, f) in v.fields().iter().enumerate() {
                    if let Some(free) =
                        self.free_stmt(&f.as_type(), &format!("x.{}", field(f.name(), j)))
                    {
                        let _ = writeln!(frees, "                {free}");
                    }
                }
                if !frees.is_empty() {
                    covered += 1;
                    let _ = writeln!(
                        arms,
                        "            .{} => |x| {{\n{frees}            }},",
                        snake(v.name())
                    );
                }
            }
            if !arms.is_empty() {
                let rest = if covered < e.variants().len() {
                    "            else => {},\n"
                } else {
                    ""
                };
                let _ = write!(body, "        switch (self.*) {{\n{arms}{rest}        }}\n");
            }
        }
        indent(
            &self.func(
                "pub ",
                "deinit",
                &[
                    ("self", format!("*const {zt}")),
                    ("a", "std.mem.Allocator".into()),
                ],
                "void",
                &body
                    .replace("\n        ", "\n    ")
                    .replacen("        ", "    ", 1),
            ),
            4,
        )
    }

    fn container_codec(&self, t: &Type) -> String {
        let zt = self.zig_type(t);
        let m = self.mangle(t);
        let rw = [("w", "*uniffi.Writer".to_owned()), ("v", zt.clone())];
        let rr = [
            ("r", "*uniffi.Reader".to_owned()),
            ("a", "std.mem.Allocator".to_owned()),
        ];
        let rf = [("a", "std.mem.Allocator".to_owned()), ("v", zt.clone())];
        let ret = format!("uniffi.Error!{zt}");
        match t {
            Type::Optional { inner_type } => {
                let inner = inner_type.as_ref();
                let free = self
                    .free_stmt(inner, "x")
                    .map(|f| format!("    if (v) |x| {f}\n"))
                    .unwrap_or_default();
                let mut out = self.func("", &format!("write{m}"), &rw, "void", &format!("    if (v) |x| {{\n        w.int(i8, 1);\n        {}\n    }} else w.int(i8, 0);\n", self.write_stmt(inner, "x")));
                out.push_str(&self.func("", &format!("read{m}"), &rr, &ret, &format!("    return switch (try r.int(i8)) {{\n        0 => null,\n        1 => {},\n        else => error.Internal,\n    }};\n", self.read_expr(inner))));
                out.push_str(&self.func("", &format!("free{m}"), &rf, "void", &free));
                out
            }
            Type::Sequence { inner_type } | Type::Set { inner_type } => {
                let inner = inner_type.as_ref();
                let it = self.zig_type(inner);
                let free_item = self
                    .free_stmt(inner, "x")
                    .map(|f| format!("    for (v) |x| {f}\n"))
                    .unwrap_or_default();
                let free_read = self
                    .free_stmt(inner, "x")
                    .map(|f| {
                        format!("    errdefer for (items[0..done]) |x| {{\n        {f}\n    }};\n")
                    })
                    .unwrap_or_default();
                let mut out = self.func(
                    "",
                    &format!("write{m}"),
                    &rw,
                    "void",
                    &format!(
                        "    w.len(v.len);\n    for (v) |x| {{\n        {}\n    }}\n",
                        self.write_stmt(inner, "x")
                    ),
                );
                out.push_str(&self.func("", &format!("read{m}"), &rr, &ret, &format!("    const n = try r.len({min});\n    const items = try a.alloc({it}, n);\n    errdefer a.free(items);\n    var done: usize = 0;\n{free_read}    while (done < n) : (done += 1) {{\n        items[done] = {};\n    }}\n    return items;\n", self.read_expr(inner), min = self.min_wire_size(inner))));
                out.push_str(&self.func(
                    "",
                    &format!("free{m}"),
                    &rf,
                    "void",
                    &format!("{free_item}    a.free(v);\n"),
                ));
                out
            }
            Type::Map {
                key_type,
                value_type,
            } => {
                let (k, val) = (key_type.as_ref(), value_type.as_ref());
                let entry = format!("uniffi.Entry({}, {})", self.zig_type(k), self.zig_type(val));
                let mut frees = String::new();
                if let Some(f) = self.free_stmt(k, "e.key") {
                    let _ = write!(frees, " {f}");
                }
                if let Some(f) = self.free_stmt(val, "e.value") {
                    let _ = write!(frees, " {f}");
                }
                let (free_items, free_read) = if frees.is_empty() {
                    (String::new(), String::new())
                } else {
                    (
                        format!("    for (v) |e| {{{frees} }}\n"),
                        format!("    errdefer for (items[0..done]) |e| {{{frees} }};\n"),
                    )
                };
                let mut out = self.func(
                    "",
                    &format!("write{m}"),
                    &rw,
                    "void",
                    &format!(
                        "    w.len(v.len);\n    for (v) |e| {{\n        {}\n        {}\n    }}\n",
                        self.write_stmt(k, "e.key"),
                        self.write_stmt(val, "e.value")
                    ),
                );
                out.push_str(&self.func("", &format!("read{m}"), &rr, &ret, &format!("    const n = try r.len({min});\n    const items = try a.alloc({entry}, n);\n    errdefer a.free(items);\n    var done: usize = 0;\n{free_read}    while (done < n) : (done += 1) {{\n        const key = {};\n        items[done] = .{{ .key = key, .value = {} }};\n    }}\n    return items;\n", self.read_expr(k), self.read_expr(val), min = self.min_wire_size(k) + self.min_wire_size(val))));
                out.push_str(&self.func(
                    "",
                    &format!("free{m}"),
                    &rf,
                    "void",
                    &format!("{free_items}    a.free(v);\n"),
                ));
                out
            }
            Type::Box { inner_type } => {
                let inner = inner_type.as_ref();
                let it = self.zig_type(inner);
                let free = self
                    .free_stmt(inner, "v.*")
                    .map(|f| format!("    {f}\n"))
                    .unwrap_or_default();
                let mut out = self.func(
                    "",
                    &format!("write{m}"),
                    &rw,
                    "void",
                    &format!("    {}\n", self.write_stmt(inner, "v.*")),
                );
                out.push_str(&self.func("", &format!("read{m}"), &rr, &ret, &format!("    const p = try a.create({it});\n    errdefer a.destroy(p);\n    p.* = {};\n    return p;\n", self.read_expr(inner))));
                out.push_str(&self.func(
                    "",
                    &format!("free{m}"),
                    &rf,
                    "void",
                    &format!("{free}    a.destroy(v);\n"),
                ));
                out
            }
            _ => String::new(),
        }
    }

    /// Lowering into and lifting out of a RustBuffer. A string travels as its bare
    /// UTF-8 bytes, everything else serialized.
    fn lower_lift(&self, t: &Type) -> String {
        let zt = self.zig_type(t);
        let m = self.mangle(t);
        let is_string = matches!(t, Type::String)
            || matches!(t, Type::Custom { builtin, .. } if matches!(**builtin, Type::String));
        if is_string {
            let mut out = self.func(
                "",
                &format!("lower{m}"),
                &[("v", zt.clone())],
                "uniffi.RustBuffer",
                "    return bufferFrom(v);\n",
            );
            out.push_str(&self.func("", &format!("lift{m}"), &[("a", "std.mem.Allocator".into()), ("buffer", "uniffi.RustBuffer".into())], &format!("uniffi.Error!{zt}"), "    defer bufferFree(buffer);\n    const data = buffer.data orelse return try a.dupe(u8, \"\");\n    return try a.dupe(u8, data[0..@intCast(buffer.len)]);\n"));
            return out;
        }
        let read = self.read_expr(t);
        let mut out = self.func("", &format!("lower{m}"), &[("v", zt.clone())], "uniffi.RustBuffer", &format!("    var writer: uniffi.Writer = .{{}};\n    defer writer.deinit();\n    const w = &writer;\n    {}\n    return bufferFrom(writer.list.items);\n", self.write_stmt(t, "v")));
        out.push_str(&self.func("", &format!("lift{m}"), &[("a", "std.mem.Allocator".into()), ("buffer", "uniffi.RustBuffer".into())], &format!("uniffi.Error!{zt}"), &format!("    defer bufferFree(buffer);\n    const data: []const u8 = if (buffer.data) |d| d[0..@intCast(buffer.len)] else &.{{}};\n    var reader: uniffi.Reader = .{{ .data = data }};\n    const r = &reader;\n    const v = {read};\n    if (r.remaining() != 0) return error.Internal;\n    return v;\n")));
        out
    }
}

fn custom_name(t: &Type) -> &str {
    match t {
        Type::Object { name, .. }
        | Type::CallbackInterface { name, .. }
        | Type::Custom { name, .. } => name,
        _ => "",
    }
}

/// Indents every non-empty line by `n` spaces.
pub(crate) fn indent(text: &str, n: usize) -> String {
    let pad = " ".repeat(n);
    text.lines()
        .map(|l| {
            if l.is_empty() {
                String::new()
            } else {
                format!("{pad}{l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}
