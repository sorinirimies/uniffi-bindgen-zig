//! Identifiers: Zig keywords and primitive type names, Zig naming style
//! (camelCase functions, snake_case fields and variants), and templating helpers.

/// Keywords and primitive names Zig does not accept as plain identifiers.
const KEYWORDS: &[&str] = &[
    "addrspace",
    "align",
    "allowzero",
    "and",
    "anyframe",
    "anytype",
    "asm",
    "break",
    "callconv",
    "catch",
    "comptime",
    "const",
    "continue",
    "defer",
    "else",
    "enum",
    "errdefer",
    "error",
    "export",
    "extern",
    "fn",
    "for",
    "if",
    "inline",
    "linksection",
    "noalias",
    "noinline",
    "nosuspend",
    "opaque",
    "or",
    "orelse",
    "packed",
    "pub",
    "resume",
    "return",
    "struct",
    "suspend",
    "switch",
    "test",
    "threadlocal",
    "try",
    "union",
    "unreachable",
    "var",
    "volatile",
    "while",
    // Primitive values and types cannot be shadowed.
    "true",
    "false",
    "null",
    "undefined",
    "type",
    "void",
    "bool",
    "anyerror",
    "anyopaque",
    "noreturn",
    "isize",
    "usize",
    "comptime_int",
    "comptime_float",
    "f16",
    "f32",
    "f64",
    "f80",
    "f128",
    "c_char",
    "c_short",
    "c_ushort",
    "c_int",
    "c_uint",
    "c_long",
    "c_ulong",
    "c_longlong",
    "c_ulonglong",
    "c_longdouble",
];

/// Names the generated code uses for its own parameters and locals.
/// Only the locals of public functions and callback wrappers, which also hold
/// user arguments, matter here.
const RESERVED: &[&str] = &[
    "allocator",
    "err",
    "self",
    "status",
    "raw",
    "future",
    "std",
    "uniffi",
    "ffi",
    "ptr",
    "a",
    "e",
    "p",
    "le",
    "local_err",
    "impl",
    "result",
    "err_value",
    "ev",
    "job",
    "w",
];

fn is_keyword(name: &str) -> bool {
    KEYWORDS.contains(&name)
        || (name.len() > 1
            && (name.starts_with('u') || name.starts_with('i'))
            && name[1..].chars().all(|c| c.is_ascii_digit()))
}

/// A name usable as a Zig identifier: `@"name"` when it is a keyword.
pub(crate) fn ident(name: &str) -> String {
    if is_keyword(name) {
        format!("@\"{name}\"")
    } else {
        name.to_owned()
    }
}

/// An argument or local name that cannot clash with the generated code's own names.
pub(crate) fn var(name: &str) -> String {
    if RESERVED.contains(&name) {
        format!("{name}_")
    } else {
        ident(name)
    }
}

/// `name` without the `@"..."` quoting of a keyword.
pub(crate) fn unquote(name: &str) -> &str {
    name.strip_prefix("@\"")
        .and_then(|n| n.strip_suffix('"'))
        .unwrap_or(name)
}

/// A field name inside a variant: nameless fields become `v0`, `v1`, ...
pub(crate) fn field(name: &str, index: usize) -> String {
    if name.is_empty() {
        format!("v{index}")
    } else {
        ident(name)
    }
}

/// `snake_case` / `SCREAMING` / `PascalCase` to `camelCase` (functions, methods).
pub(crate) fn camel(name: &str) -> String {
    let words = words(name);
    let mut out = String::new();
    for (i, w) in words.iter().enumerate() {
        if i == 0 {
            out.push_str(&w.to_ascii_lowercase());
        } else {
            let mut chars = w.chars();
            if let Some(first) = chars.next() {
                out.push(first.to_ascii_uppercase());
                out.push_str(&chars.as_str().to_ascii_lowercase());
            }
        }
    }
    let prefix: String = name.chars().take_while(|c| *c == '_').collect();
    ident(&format!("{prefix}{out}"))
}

/// `PascalCase` / `SCREAMING_CASE` to `snake_case` (enum and union fields).
pub(crate) fn snake(name: &str) -> String {
    ident(
        &words(name)
            .iter()
            .map(|w| w.to_ascii_lowercase())
            .collect::<Vec<_>>()
            .join("_"),
    )
}

/// Splits an identifier into words at underscores and lower→upper case changes.
fn words(name: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in name.split('_').filter(|p| !p.is_empty()) {
        let chars: Vec<char> = part.chars().collect();
        let mut word = String::new();
        for (i, c) in chars.iter().enumerate() {
            let boundary = i > 0
                && c.is_ascii_uppercase()
                && (chars[i - 1].is_ascii_lowercase()
                    || chars[i - 1].is_ascii_digit()
                    || chars.get(i + 1).is_some_and(|n| n.is_ascii_lowercase())
                        && chars[i - 1].is_ascii_uppercase());
            if boundary && !word.is_empty() {
                out.push(std::mem::take(&mut word));
            }
            word.push(*c);
        }
        if !word.is_empty() {
            out.push(word);
        }
    }
    out
}

/// Quote `s` as a Zig string literal.
pub(crate) fn string_literal(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A `///` doc comment, indented by `indent` spaces.
pub(crate) fn doc(docstring: Option<&str>, indent: usize) -> String {
    let Some(text) = docstring else {
        return String::new();
    };
    let pad = " ".repeat(indent);
    text.trim_end()
        .lines()
        .map(|l| {
            let l = l.strip_prefix(' ').unwrap_or(l).trim_end();
            if l.is_empty() {
                format!("{pad}///\n")
            } else {
                format!("{pad}/// {l}\n")
            }
        })
        .collect()
}

/// Whether `name` occurs in `body` as an identifier of its own (not a field
/// access `.name`, not part of a longer name).
pub(crate) fn uses(body: &str, name: &str) -> bool {
    let bytes = body.as_bytes();
    let is_ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut start = 0;
    while let Some(i) = body[start..].find(name) {
        let at = start + i;
        let end = at + name.len();
        let before_ok = at == 0 || (!is_ident(bytes[at - 1]) && bytes[at - 1] != b'.');
        let after_ok = end >= bytes.len() || !is_ident(bytes[end]);
        if before_ok && after_ok {
            return true;
        }
        start = end;
    }
    false
}

/// `_ = name;` for each parameter `body` does not use: Zig rejects unused
/// parameters, and also discards of used ones.
pub(crate) fn discard_unused(body: &str, params: &[&str]) -> String {
    let mut out = String::new();
    for p in params {
        if !uses(body, p) {
            out.push_str(&format!("    _ = {p};\n"));
        }
    }
    out.push_str(body);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camel_converts_snake_and_pascal_names() {
        assert_eq!(camel("fail_data"), "failData");
        assert_eq!(camel("add_u8"), "addU8");
        assert_eq!(camel("new"), "new");
        assert_eq!(camel("HTTPServer"), "httpServer");
        assert_eq!(camel("_private"), "_private");
    }

    #[test]
    fn snake_converts_variant_names() {
        assert_eq!(snake("BadCode"), "bad_code");
        assert_eq!(snake("RED"), "red");
        assert_eq!(snake("Red"), "red");
        assert_eq!(snake("IOError"), "io_error");
        assert_eq!(snake("Error"), "@\"error\"");
    }

    #[test]
    fn keywords_and_primitive_names_are_quoted() {
        assert_eq!(ident("error"), "@\"error\"");
        assert_eq!(ident("u8"), "@\"u8\"");
        assert_eq!(ident("u"), "u");
        assert_eq!(ident("name"), "name");
    }

    #[test]
    fn var_avoids_the_generated_code_names() {
        assert_eq!(var("allocator"), "allocator_");
        assert_eq!(var("type"), "@\"type\"");
    }

    #[test]
    fn uses_ignores_field_access_and_longer_names() {
        assert!(uses("foo(a, b)", "a"));
        assert!(!uses("v.a + ab", "a"));
        assert!(!uses("self.allocator", "allocator"));
    }

    #[test]
    fn discard_unused_discards_only_unused_params() {
        assert_eq!(
            discard_unused("    return a;\n", &["a", "b"]),
            "    _ = b;\n    return a;\n"
        );
    }
}
