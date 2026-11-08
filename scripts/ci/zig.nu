# ── uniffi-bindgen-zig · Zig toolchain helpers ─────────────────────────────
# Shared by zig_e2e.nu, fixtures.nu and fuzz_zig.nu.

# File name of the shared library `name` on `os`.
export def library_file [name: string, os: string] {
    match $os {
        "macos" => $"lib($name).dylib"
        "windows" => $"($name).dll"
        _ => $"lib($name).so"
    }
}

# `zig test` arguments that build `file` against the Rust library `name` in
# `lib_dir` (libc linked: the Rust library needs it, the runtime uses pthreads).
export def test_args [file: string, lib_dir: string, name: string, sanitize: bool] {
    let sanitizers = if $sanitize { ["-fsanitize-c"] } else { [] }
    ["test" $file "-lc" $"-L($lib_dir)" $"-l($name)" "-rpath" $lib_dir ...$sanitizers]
}

# `zig test` source that references every declaration of the given modules, so
# Zig analyzes them (it only compiles what is used).
export def check_source [modules: list<string>] {
    let imports = $modules | enumerate | each { |m| $"const m($m.index) = @import\(\"($m.item).zig\"\);" }
    let refs = $modules | enumerate | each { |m| $"    std.testing.refAllDecls\(m($m.index)\);" }
    ["const std = @import(\"std\");" ...$imports "test {" ...$refs "}" ""] | str join "\n"
}
