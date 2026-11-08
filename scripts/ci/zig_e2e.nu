#!/usr/bin/env nu
# ── uniffi-bindgen-zig · Zig end-to-end test ───────────────────────────────
#
# Builds the `tour` fixture crate, generates Zig bindings for it and runs
# fixtures/tour/zig/test_tour.zig against them with `zig test` (its testing
# allocator fails on leaks).
#
# Usage: nu scripts/ci/zig_e2e.nu [--sanitize]

const FIXTURE = "fixtures/tour"

use zig.nu

def main [--sanitize] {
    let root = ($env.FILE_PWD | path dirname | path dirname)
    cd $root
    print -e "▶ building the tour fixture"
    ^cargo build --manifest-path $"($FIXTURE)/Cargo.toml"
    let lib_dir = ($root | path join $FIXTURE target debug)
    let lib = ($lib_dir | path join (zig library_file "tour" $nu.os-info.name))
    let out_dir = (mktemp --directory)

    print -e "▶ generating bindings"
    ^cargo run --quiet -p uniffi-bindgen-zig -- generate $lib --out-dir $out_dir --metadata-no-deps
    cp $"($FIXTURE)/zig/test_tour.zig" $out_dir

    print -e "▶ zig test"
    cd $out_dir
    ^zig ...(zig test_args "test_tour.zig" $lib_dir "tour" $sanitize)
    print -e "✔ Zig end-to-end test passed"
}
