//! Generate from UDL and check the shape of the output. The compile check runs
//! `zig test -fno-emit-bin` (full analysis, no linking) and is skipped without Zig.

use std::process::Command;

use uniffi_bindgen_zig::{Config, GeneratedFiles, RUNTIME, RUNTIME_NAME, generate_from_udl};

const UDL: &str = r#"
namespace sample {
    u32 add(u32 a, u32 b);
    [Throws=Oops] string risky(string s);
    Shape shape_of(Point p);
    [Async] string fetch(string key);
    sequence<string> names();
    record<string, u32> counts();
    bytes reverse(bytes data);
    u32 lend([ByRef] bytes data);
    void use_sink(Sink sink);
};

[Error]
enum Oops { "Bad", "Worse" };

[Error]
interface Detailed {
    Code(i32 code);
    Empty();
};

enum Color { "Red", "Green" };

dictionary Point {
    double x;
    double y = 1.5;
    string? label = null;
    Color color = "Green";
};

[Enum]
interface Shape {
    Circle(Point center, double radius);
    Empty();
};

interface Thing {
    constructor(string name);
    [Name=other] constructor(u32 n);
    string name();
    [Throws=Oops] void check();
    [Async] u32 later();
};

callback interface Sink {
    void push(string s);
    [Throws=Oops] u32 count();
};
"#;

fn generate() -> GeneratedFiles {
    generate_from_udl(UDL, "sample", &Config::default()).expect("generates")
}

#[test]
fn file_declares_every_kind_of_definition() {
    let z = generate().source;
    for expected in [
        "const uniffi = @import(\"uniffi.zig\");",
        "pub fn add(a_: u32, b: u32) uniffi.Error!u32 {",
        "pub fn risky(allocator: std.mem.Allocator, s: []const u8, err: ?*Oops) (Oops.Set || uniffi.Error)![]const u8 {",
        "pub fn fetch(allocator: std.mem.Allocator, key: []const u8) uniffi.Error![]const u8 {",
        "pub fn names(allocator: std.mem.Allocator) uniffi.Error![]const []const u8 {",
        "pub fn counts(allocator: std.mem.Allocator) uniffi.Error![]const uniffi.Entry([]const u8, u32) {",
        "pub fn lend(data: []const u8) uniffi.Error!u32 {",
        "pub const Color = enum(i64) {",
        "    green = 1,",
        "pub const Point = struct {",
        "    y: f64 = 1.5,",
        "    label: ?[]const u8 = null,",
        "    color: Color = .green,",
        "pub const Shape = union(enum) {",
        "    circle: struct { center: Point, radius: f64 },",
        "    empty,",
        "pub const Oops = union(enum) {",
        "    pub const Set = error{ Bad, Worse };",
        "pub const Thing = struct {",
        "    pub fn init(name_: []const u8) uniffi.Error!Thing {",
        "    pub fn other(n: u32) uniffi.Error!Thing {",
        "    pub fn check(self: Thing, allocator: std.mem.Allocator, err: ?*Oops) (Oops.Set || uniffi.Error)!void {",
        "    pub fn deinit(self: Thing) void {",
        "pub const Sink = struct {",
        "        push: *const fn (ptr: *anyopaque, allocator: std.mem.Allocator, s: []const u8) anyerror!void,",
        "    pub fn from(impl: anytype) Sink {",
    ] {
        assert!(z.contains(expected), "lacks `{expected}`:\n{z}");
    }
}

#[test]
fn borrowed_bytes_are_lent_without_a_copy() {
    let z = generate().source;
    assert!(z.contains("uniffi.ForeignBytes{ .len = @intCast(data.len), .data = data.ptr }"));
}

#[test]
fn ffi_declarations_are_private() {
    let z = generate().source;
    assert!(z.contains("const ffi = struct {"));
    assert!(z.contains("pub extern fn uniffi_sample_fn_func_add(a: u32, b: u32, out_status: *uniffi.RustCallStatus) callconv(.c) u32;"));
}

#[test]
fn config_omit_checksums_drops_the_check() {
    let with = generate().source;
    let config = Config {
        omit_checksums: true,
        ..Config::default()
    };
    let without = generate_from_udl(UDL, "sample", &config)
        .expect("generates")
        .source;
    assert!(with.contains("API checksum mismatch"));
    assert!(!without.contains("API checksum mismatch"));
    assert!(without.contains("contract version mismatch"));
}

/// `zig test -fno-emit-bin`: every declaration analyzed, nothing linked. Passes
/// vacuously without Zig.
fn compiles(files: &GeneratedFiles) -> bool {
    if Command::new("zig").arg("version").output().is_err() {
        eprintln!("no zig; skipping the compile check");
        return true;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join(RUNTIME_NAME), RUNTIME).expect("write");
    std::fs::write(dir.path().join(&files.name), &files.source).expect("write");
    let module = files.name.trim_end_matches(".zig");
    std::fs::write(
        dir.path().join("check.zig"),
        format!("const std = @import(\"std\");\nconst m = @import(\"{module}.zig\");\ntest {{\n    std.testing.refAllDecls(m);\n}}\n"),
    )
    .expect("write");
    let out = Command::new("zig")
        .args(["test", "check.zig", "-lc", "-fno-emit-bin"])
        .current_dir(dir.path())
        .output()
        .expect("run zig");
    if !out.status.success() {
        eprintln!("{}", String::from_utf8_lossy(&out.stderr));
    }
    out.status.success()
}

#[test]
fn generated_code_compiles() {
    assert!(compiles(&generate()), "generated code does not compile");
}

#[test]
fn empty_namespace_generates_compilable_code() {
    let files =
        generate_from_udl("namespace empty {};", "empty", &Config::default()).expect("generates");
    assert!(compiles(&files));
}

#[test]
fn invalid_udl_is_an_error() {
    assert!(generate_from_udl("namespace broken { void f(", "broken", &Config::default()).is_err());
}

// Regression: Zig rejects a parameter that shadows a declaration (a method
// `name` and an argument `name`).
#[test]
fn names_never_shadow_or_collide() {
    let udl = "namespace clash { }; interface Node { constructor(string name); string name(); }; interface Pair { Node Node(); };";
    let files = generate_from_udl(udl, "clash", &Config::default()).expect("generate");
    assert!(
        files
            .source
            .contains("pub fn init(name_: []const u8) uniffi.Error!Node {")
    );
    // `Node()` becomes `node`: no clash with the type `Node`, Zig names are case-sensitive.
    assert!(
        files
            .source
            .contains("pub fn node(self: Pair) uniffi.Error!Node {"),
        "{}",
        files.source
    );
    assert!(compiles(&files));
}

// Regression: an empty record was read into a `var` that is never mutated,
// which Zig rejects.
#[test]
fn empty_record_compiles() {
    let udl = "namespace empty_rec { Nothing id(Nothing n); }; dictionary Nothing {};";
    assert!(compiles(
        &generate_from_udl(udl, "empty_rec", &Config::default()).expect("generate")
    ));
}

#[test]
fn generated_snapshot() {
    insta::assert_snapshot!("sample", generate().source);
}
