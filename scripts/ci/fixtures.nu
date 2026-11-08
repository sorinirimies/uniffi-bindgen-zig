#!/usr/bin/env nu
# ── uniffi-bindgen-zig · uniffi fixture suite ────────────────────────────────
#
# Runs uniffi's own test fixtures (fixtures/uniffi, vendored from uniffi-rs)
# against the Zig bindings:
#   1. builds every fixture crate,
#   2. generates Zig for every cdylib and compiles every declaration of it,
#      linked against the library (so every extern function must exist),
#   3. runs each fixture's zig/test_*.zig.
#
# Usage: nu scripts/ci/fixtures.nu [--sanitize] [--only <cdylib name>]

use zig.nu

const FIXTURES = "fixtures/uniffi"

# Libraries we only build: upstream does not generate bindings for them either.
export const SKIP = [
    # Its UDL lives in another crate, so library mode cannot find the UDL types.
    "uniffi_proc_macro_nip"
]

# The cdylib targets in `cargo metadata` output: name and package directory.
export def cdylibs [metadata: record] {
    $metadata.packages | each { |p|
        $p.targets | where { |t| "cdylib" in $t.crate_types } | each { |t|
            {name: $t.name, dir: ($p.manifest_path | path dirname)}
        }
    } | flatten | sort-by name
}

# The libraries to run: all but SKIP, or just `only`.
export def selected [libs: list, only: string] {
    $libs | where { |l| $l.name not-in $SKIP and ($only == "" or $l.name == $only) }
}

def run_library [lib: record, bin: string, lib_dir: string, out_root: string, sanitize: bool] {
    let out = ($out_root | path join $lib.name)
    mkdir $out
    let library = ($lib_dir | path join (zig library_file $lib.name $nu.os-info.name))
    # Library mode finds UDL files through `cargo metadata`, so run inside the fixture workspace.
    let gen = (do { cd $lib.dir; ^$bin generate $library --out-dir $out --no-format } | complete)
    if $gen.exit_code != 0 { return [{library: $lib.name, step: "generate", detail: $gen.stderr}] }

    let modules = (glob $"($out)/*.zig" | path parse | get stem | where { |m| $m != "uniffi" })
    zig check_source $modules | save --force ($out | path join "uniffi_check.zig")
    let check = (do { cd $out; ^zig ...(zig test_args "uniffi_check.zig" $lib_dir $lib.name $sanitize) } | complete)
    if $check.exit_code != 0 { return [{library: $lib.name, step: "compile", detail: $check.stderr}] }

    mut failures = []
    for test in (glob $"($lib.dir)/zig/test_*.zig") {
        cp $test $out
        let name = ($test | path basename)
        let r = (do { cd $out; ^zig ...(zig test_args $name $lib_dir $lib.name $sanitize) } | complete)
        if $r.exit_code != 0 {
            $failures = ($failures | append {library: $lib.name, step: $"run ($name)", detail: $"($r.stdout)($r.stderr)"})
        } else {
            print -e $"  ($lib.name)/($name): passed"
        }
    }
    $failures
}

def main [--sanitize, --only: string = ""] {
    let root = ($env.FILE_PWD | path dirname | path dirname)
    cd $root
    let manifest = ($FIXTURES | path join Cargo.toml)
    print -e "▶ building the fixtures"
    ^cargo build --quiet --manifest-path $manifest
    ^cargo build --quiet -p uniffi-bindgen-zig
    let bin = ($root | path join target debug uniffi-bindgen-zig)
    let lib_dir = ($root | path join $FIXTURES target debug)
    let libs = (selected (cdylibs (^cargo metadata --format-version 1 --no-deps --manifest-path $manifest | from json)) $only)
    let out_root = (mktemp --directory)

    print -e $"▶ generating, compiling and testing ($libs | length) libraries with zig (^zig version)"
    let failures = ($libs | each { |lib| run_library $lib $bin $lib_dir $out_root $sanitize } | flatten)
    if ($failures | is-not-empty) {
        for f in $failures {
            print -e $"✗ ($f.library): ($f.step)\n($f.detail | lines | first 30 | str join "\n")"
        }
        error make {msg: $"($failures | length) fixture steps failed"}
    }
    print -e "✔ all fixtures passed"
}
