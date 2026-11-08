//! A Zig bindings generator for [UniFFI](https://mozilla.github.io/uniffi-rs/).
//!
//! Point it at a library built with UniFFI and it writes a Zig file per
//! component, plus one shared runtime file:
//!
//! ```text
//! uniffi-bindgen-zig generate target/release/libmycrate.so --out-dir zig/
//! ```
//!
//! The generated code calls the scaffolding through its C ABI, so you link the
//! Rust library into your program like any other native library.

mod cli;
mod codegen;
mod config;

pub use cli::run;
pub use codegen::{
    GeneratedFiles, RUNTIME, RUNTIME_NAME, ZigBindingGenerator, generate_component,
    generate_from_udl, write_runtime,
};
pub use config::Config;
