//! Package an import-free core Wasm guest as a validated component.
use onecad_extension_sdk::{ExtensionRuntime, Manifest};
use serde_json::Value;
use std::{env, error::Error, fs, path::PathBuf};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.len() != 3 {
        return Err(
            "usage: onecad-addon-build <cargo-messages.jsonl> <manifest.json> <output-root>".into(),
        );
    }
    let messages = fs::read_to_string(&args[0])?;
    let manifest_bytes = fs::read(&args[1])?;
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)?;
    manifest.validate()?;
    if manifest.runtime != ExtensionRuntime::PortableComponent {
        return Err("build tools only package portable components".into());
    }
    if manifest.entry != "extension.wasm" {
        return Err("manifest entry must be extension.wasm".into());
    }
    let module = guest_artifact(&messages)?;
    let component = wit_component::ComponentEncoder::default()
        .module(&fs::read(module)?)?
        .validate(true)
        .encode()?;
    let output = PathBuf::from(&args[2]).join(&manifest.id);
    fs::create_dir_all(&output)?;
    fs::write(output.join("extension.wasm"), component)?;
    fs::write(output.join("manifest.json"), manifest_bytes)?;
    println!("Staged addon: {}", output.display());
    Ok(())
}

fn guest_artifact(messages: &str) -> Result<PathBuf, Box<dyn Error>> {
    let mut artifacts = Vec::new();
    for line in messages.lines() {
        let message: Value = serde_json::from_str(line)?;
        if message["reason"] != "compiler-artifact" {
            continue;
        }
        let Some(types) = message["target"]["crate_types"].as_array() else {
            continue;
        };
        if !types.iter().any(|kind| kind == "cdylib") {
            continue;
        }
        if let Some(files) = message["filenames"].as_array() {
            artifacts.extend(
                files
                    .iter()
                    .filter_map(Value::as_str)
                    .filter(|file| file.ends_with(".wasm"))
                    .map(PathBuf::from),
            );
        }
    }
    match artifacts.as_slice() {
        [artifact] => Ok(artifact.clone()),
        _ => Err("expected exactly one compiled addon cdylib Wasm artifact".into()),
    }
}
