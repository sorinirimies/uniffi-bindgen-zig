//! The `uniffi-bindgen-zig` binary: parses the arguments and reports errors.

use std::process::ExitCode;

fn main() -> ExitCode {
    match uniffi_bindgen_zig::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}
