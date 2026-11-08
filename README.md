# uniffi-bindgen-zig

[![CI](https://github.com/sorinirimies/uniffi-bindgen-zig/actions/workflows/ci.yml/badge.svg)](https://github.com/sorinirimies/uniffi-bindgen-zig/actions/workflows/ci.yml)
[![Security](https://github.com/sorinirimies/uniffi-bindgen-zig/actions/workflows/security.yml/badge.svg)](https://github.com/sorinirimies/uniffi-bindgen-zig/actions/workflows/security.yml)
[![Fuzz](https://github.com/sorinirimies/uniffi-bindgen-zig/actions/workflows/fuzz.yml/badge.svg)](https://github.com/sorinirimies/uniffi-bindgen-zig/actions/workflows/fuzz.yml)
[![crates.io](https://img.shields.io/crates/v/uniffi-bindgen-zig.svg)](https://crates.io/crates/uniffi-bindgen-zig)

Zig bindings generator for [UniFFI](https://mozilla.github.io/uniffi-rs/) 0.32.

Point it at a library built with UniFFI and it writes one Zig file per component, plus a
shared runtime file. The generated code calls the scaffolding through its C ABI with
`extern fn`, so there is no C in between: link the Rust library and `@import` the file.

* **Idiomatic Zig:** error unions, optionals, slices, tagged unions you can `switch` on,
  records with default field values, explicit allocators, `deinit`.
* **Complete:** every fixture of uniffi's own test suite generates and compiles, each
  declaration analyzed and linked against its library; the tour test covers objects,
  traits, callback interfaces in both directions, async, errors, recursive types, sets
  and lent byte slices, under Zig's leak-checking test allocator.
* **Interfaces like `std.mem.Allocator`:** implement a callback interface or a Rust
  trait with any type that has the methods: `Listener.from(&my_listener)`.
* **Hardened decoding:** what comes from Rust is bounds-checked; the generated decoders
  are fuzzed to fail cleanly (freeing what they had read) or round-trip byte for byte.

## Install

```sh
cargo install uniffi-bindgen-zig
```

| uniffi-bindgen-zig | UniFFI | Zig |
|---|---|---|
| 0.1 | 0.32 | 0.17 |

A UniFFI minor version can change the FFI, so use the release that matches the `uniffi`
version of the Rust library (a library built with another version fails at generation).
Zig's standard library still changes between releases; the bindings are tested with the
Zig version in the table.

## Use

```sh
cargo build --release
uniffi-bindgen-zig generate target/release/libmylib.so --out-dir src/bindings/
```

```zig
// build.zig
exe.root_module.addImport("mylib", b.createModule(.{ .root_source_file = b.path("src/bindings/mylib.zig") }));
exe.root_module.addLibraryPath(b.path("target/release"));
exe.root_module.linkSystemLibrary("mylib", .{});
exe.root_module.link_libc = true;
```

`generate` also takes a `.udl` file (`--library` names the compiled library, `--crate` the
crate). `uniffi-bindgen-zig runtime <dir>` writes just `uniffi.zig`.

Config, in the crate's `uniffi.toml`:

```toml
[bindings.zig]
omit_checksums = false    # skip the API checksum check at start-up

[bindings.zig.rename]     # Zig names only, in UniFFI's dot notation
OldRecord = "NewRecord"
```

## The generated API

Functions and methods are `camelCase`, types `PascalCase`, fields and enum/union members
`snake_case`.

| Rust | Zig |
|---|---|
| `u8`…`i64`, `f32`, `f64`, `bool` | `u8`…`i64`, `f32`, `f64`, `bool` |
| `String`, `Vec<u8>` | `[]const u8` (`&[u8]` arguments are lent to Rust without a copy) |
| `SystemTime`, `Duration` | `uniffi.Timestamp`, `uniffi.Duration` |
| `Option<T>` | `?T` |
| `Vec<T>`, `HashSet<T>` | `[]const T` |
| `HashMap<K, V>` | `[]const uniffi.Entry(K, V)` (`.key`, `.value`) |
| `Box<T>` | `*const T` (recursive records and enums) |
| record | `struct` with default field values, `deinit(allocator)` |
| enum without data | `enum(repr)` with Rust's discriminants |
| enum with data | `union(enum)` |
| error | `union(enum)` with `Set` (its error set) and `toError()` |
| object | `struct { handle }`: `init` (Rust's `new`), methods, `clone`, `deinit` |
| callback interface | `struct { ptr, vtable }`, made with `from(&impl)` |
| custom type | its builtin type |
| `Display`, `Debug`, `Eq`, `Hash`, `Ord` | `toString`, `debugString`, `eql`, `hash`, `order` (`std.math.Order`) |

```zig
const tour = @import("tour");

const counter = try tour.Counter.init(10);
defer counter.deinit();
_ = try counter.add(5);
const text = try counter.toString(allocator);   // "Counter(15)"
defer allocator.free(text);
```

### Memory

* **Arguments are borrowed**: string literals, stack arrays and slices all work.
* **A function whose result (or error payload) holds memory takes an `allocator`** after
  the receiver, and allocates the result with it; free it with `allocator.free`, or
  `deinit(allocator)` for records and enums. Objects are handles: `deinit()` releases one,
  `clone()` makes a second.

### Errors

Every call can fail with `uniffi.Error`: `error.RustPanic` (the message is in
`uniffi.lastErrorMessage()`), `error.Cancelled`, `error.Internal` or `error.OutOfMemory`.
A function returning `Result<_, E>` returns `(E.Set || uniffi.Error)!T`, where `E.Set`
has one error per variant, and takes an `err: ?*E` for the payload:

```zig
var err: tour.DataError = undefined;
tour.failData(1, &err) catch |e| switch (e) {
    error.BadCode => std.debug.print("{d}\n", .{err.bad_code.code}),
    else => return e,
};
```

### Callback interfaces and traits

Any type with the interface's methods implements it; each method gets an allocator for
what it returns (the bindings free it) and, when the method can fail, an `err: *?E` to
fill before returning an error. Returning an error named like a variant without fields
also works.

```zig
const Recorder = struct {
    count: u32 = 0,
    pub fn onEvent(self: *Recorder, allocator: std.mem.Allocator, name: []const u8, value: i32) !i32 {
        _ = allocator;
        _ = name;
        self.count += 1;
        return value * 10;
    }
    // ... the other methods
};

var recorder: Recorder = .{};
_ = try tour.notify(tour.Listener.from(&recorder), 4);
```

A Rust trait that Zig may implement (`#[uniffi::export(with_foreign)]`) is an object made
with `fromImpl(&impl)`; `impl.deinit()`, if it exists, runs once no handle is left.

### Async

Async functions block the calling thread until Rust's future completes. Async trait
methods implemented in Zig are plain functions; Rust's call runs them on a thread of their
own.

## Limits

* Function arguments have no defaults in Zig: pass every argument.
* POSIX only for now (the runtime uses pthreads); Windows is not tested.
* Custom types are their builtin type; converting functions are not generated.

## Development

Everything is scripted in Nushell and wired through `just`:

```sh
just check-all    # fmt, clippy, tests, docs, Nushell script tests
just e2e          # generate the tour fixture's bindings and run its Zig test
just fixtures     # uniffi's fixture suite: generate, compile, link (--only <lib>)
just fuzz-zig     # fuzz the generated decoders
just vendor-fixtures ~/uniffi-rs v0.32.2   # re-vendor uniffi's fixtures
just release-all 0.1.0                    # bump, changelog, tag, push to GitHub and Gitea
```

## License

MIT
