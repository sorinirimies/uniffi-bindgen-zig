#!/usr/bin/env nu
# ── uniffi-bindgen-zig · fuzz the generated decoders ───────────────────────
#
# Generates the tour fixture's bindings and runs fuzz/zig/fuzz_codecs.zig:
# 200k pseudo-random buffers through the generated decoders, each failing
# cleanly or round-tripping, with Zig's leak-checking test allocator.
#
# Usage: nu scripts/ci/fuzz_zig.nu

use zig.nu

const FIXTURE = "fixtures/tour"

def main [] {
    let root = ($env.FILE_PWD | path dirname | path dirname)
    cd $root
    ^cargo build --quiet --manifest-path $"($FIXTURE)/Cargo.toml"
    let lib_dir = ($root | path join $FIXTURE target debug)
    let gen = (mktemp --directory)
    ^cargo run --quiet -p uniffi-bindgen-zig -- generate ($lib_dir | path join (zig library_file "tour" $nu.os-info.name)) --out-dir $gen --metadata-no-deps
    cp fuzz/zig/fuzz_codecs.zig $gen
    cd $gen
    ^zig ...(zig test_args "fuzz_codecs.zig" $lib_dir "tour" false) "-OReleaseSafe"
    print -e "✔ fuzzing the generated decoders found nothing"
}
