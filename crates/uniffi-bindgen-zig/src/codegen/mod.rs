//! The Zig code generator.

mod callbacks;
mod calls;
mod codec;
mod decls;
mod ffi;
mod names;
mod source;
mod types;

use std::collections::{BTreeSet, HashMap};

use anyhow::{Context, Result};
use camino::Utf8Path;
use uniffi_bindgen::{BindingGenerator, Component, ComponentInterface, GenerationSettings};

use crate::config::Config;
use types::Gen;

/// The shared runtime every generated binding imports.
pub const RUNTIME: &str = include_str!("../runtime/uniffi.zig");

/// Name of the runtime file.
pub const RUNTIME_NAME: &str = "uniffi.zig";

/// The generated Zig file of one component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedFiles {
    /// File name, `<namespace>.zig`.
    pub name: String,
    /// Contents.
    pub source: String,
}

/// Generates the Zig file for one component.
pub fn generate_component(ci: &ComponentInterface, config: &Config) -> GeneratedFiles {
    generate_with_errors(ci, config, &BTreeSet::new())
}

/// [`generate_component`], knowing the error types of the other components.
fn generate_with_errors(
    ci: &ComponentInterface,
    config: &Config,
    external_errors: &BTreeSet<String>,
) -> GeneratedFiles {
    let generator = Gen::new(ci, config, external_errors);
    GeneratedFiles {
        name: format!("{}.zig", ci.namespace()),
        source: generator.emit_source(),
    }
}

/// Generates the Zig file from UDL source text, without a compiled library.
///
/// Meant for tests and tooling: the result has the same shape as when generating from a
/// library, but definitions that only exist as proc-macros are missing.
pub fn generate_from_udl(udl: &str, crate_name: &str, config: &Config) -> Result<GeneratedFiles> {
    let mut ci = ComponentInterface::from_webidl(udl, crate_name)?;
    ci.derive_ffi_funcs()?;
    Ok(generate_component(&ci, config))
}

/// UniFFI plugs into binding generators through this trait.
#[derive(Debug, Clone, Copy, Default)]
pub struct ZigBindingGenerator;

impl BindingGenerator for ZigBindingGenerator {
    type Config = Config;

    fn new_config(&self, root_toml: &toml::Value) -> Result<Config> {
        Config::from_root_toml(root_toml)
    }

    fn update_component_configs(
        &self,
        settings: &GenerationSettings,
        components: &mut Vec<Component<Config>>,
    ) -> Result<()> {
        for component in components.iter_mut() {
            if component.config.cdylib_name.is_none() {
                component.config.cdylib_name = settings.cdylib.clone();
            }
        }
        apply_renames(components);
        Ok(())
    }

    fn write_bindings(
        &self,
        settings: &GenerationSettings,
        components: &[Component<Config>],
    ) -> Result<()> {
        write_runtime(&settings.out_dir)?;
        let errors: BTreeSet<String> = components
            .iter()
            .flat_map(|c| {
                c.ci.enum_definitions()
                    .iter()
                    .map(|e| e.name())
                    .filter(|n| c.ci.is_name_used_as_error(n))
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .collect();
        for component in components {
            let files = generate_with_errors(&component.ci, &component.config, &errors);
            write_file(&settings.out_dir, &files.name, &files.source)?;
            if settings.try_format_code {
                format_file(&settings.out_dir.join(&files.name));
            }
        }
        Ok(())
    }
}

/// Applies every component's `rename` table to all components, so that a renamed
/// type is renamed wherever it is used. Runs before the FFI functions are derived.
fn apply_renames(components: &mut [Component<Config>]) {
    let renames: HashMap<String, toml::Table> = components
        .iter()
        .filter(|c| !c.config.rename.is_empty())
        .map(|c| (c.ci.crate_name().to_owned(), c.config.rename.clone()))
        .collect();
    if renames.is_empty() {
        return;
    }
    for component in components {
        uniffi_bindgen::interface::rename(&mut component.ci, &renames);
    }
}

/// Writes the runtime into `out_dir`.
pub fn write_runtime(out_dir: &Utf8Path) -> Result<()> {
    write_file(out_dir, RUNTIME_NAME, RUNTIME)
}

fn write_file(dir: &Utf8Path, name: &str, contents: &str) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {dir}"))?;
    let path = dir.join(name);
    std::fs::write(&path, contents).with_context(|| format!("writing {path}"))
}

/// Best effort: format with `zig fmt` when Zig is installed.
fn format_file(path: &Utf8Path) {
    if let Err(e) = std::process::Command::new("zig")
        .arg("fmt")
        .arg(path.as_str())
        .output()
    {
        eprintln!("warning: could not format {path} with zig fmt: {e}");
    }
}
