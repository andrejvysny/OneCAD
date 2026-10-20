//! Local release source inventories and worker integrity metadata.
mod materials;
mod prepare;
#[cfg(test)]
mod safety_tests;
#[cfg(test)]
mod tests;
mod verify;

use serde_json::{json, Value};
use std::{error::Error, fs, path::Path, process::Command};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

pub fn run(command: &str, args: &[String]) -> Result<()> {
    match command {
        "prepare-release-materials" => prepare::run(args),
        "verify-release-materials" => verify::run(args),
        "worker-manifest" if args.len() == 3 => worker_manifest(args),
        "rehash-worker-manifest" if args.len() == 2 => {
            let mut manifest: Value =
                serde_json::from_slice(&materials::required(Path::new(&args[1]))?)?;
            manifest["binarySha256"] =
                materials::hash(&materials::required(Path::new(&args[0]))?).into();
            materials::save_json(Path::new(&args[1]), &manifest)
        }
        "manifest-hash" if args.len() == 1 => {
            let value: Value = serde_json::from_slice(&materials::required(Path::new(&args[0]))?)?;
            let hash = value["binarySha256"]
                .as_str()
                .ok_or("missing binarySha256")?;
            materials::digest(hash)?;
            println!("{hash}");
            Ok(())
        }
        "target-directory" if args.is_empty() => {
            let output = materials::execute(Command::new("cargo").args([
                "metadata",
                "--format-version",
                "1",
                "--no-deps",
            ]))?;
            let value: Value = serde_json::from_slice(&output)?;
            println!(
                "{}",
                value["target_directory"]
                    .as_str()
                    .ok_or("missing target_directory")?
            );
            Ok(())
        }
        _ => Err(format!("unknown release command or invalid arguments: {command}").into()),
    }
}

fn worker_manifest(args: &[String]) -> Result<()> {
    let frame = materials::required(Path::new(&args[0]))?;
    if frame.len() < 12 || &frame[..4] != b"OCW1" {
        return Err("worker did not emit an OCW1 hello frame".into());
    }
    let json_length = u32::from_le_bytes(frame[4..8].try_into()?) as usize;
    let binary_length = u32::from_le_bytes(frame[8..12].try_into()?);
    if binary_length != 0 || frame.len() != 12 + json_length {
        return Err("worker hello frame has an invalid length or binary tail".into());
    }
    let envelope: Value = serde_json::from_slice(&frame[12..])?;
    if envelope["t"] != "hello" || envelope["seq"] != 0 || !envelope["result"].is_object() {
        return Err("worker first frame is not the required hello".into());
    }
    let hello = &envelope["result"];
    let mut manifest = json!({"manifestVersion": 1,
        "binarySha256": materials::hash(&fs::read(&args[1])?)});
    for key in [
        "protocolVersion",
        "workerVersion",
        "quantizationVersion",
        "solverPolicyVersion",
        "occt",
    ] {
        manifest[key] = hello
            .get(key)
            .ok_or_else(|| format!("worker hello missing {key}"))?
            .clone();
    }
    materials::save_json(Path::new(&args[2]), &manifest)
}
