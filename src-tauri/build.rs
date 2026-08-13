use std::env;
use std::fs;
use std::path::PathBuf;

const STAGED_MANIFEST: &str = "binaries/onecad-worker-manifest.json";
const EMBEDDED_MANIFEST: &str = "onecad-worker-manifest.json";

fn main() {
    println!("cargo:rerun-if-changed={STAGED_MANIFEST}");
    embed_worker_manifest();
    tauri_build::build()
}

fn embed_worker_manifest() {
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo must set OUT_DIR"));
    let destination = out_dir.join(EMBEDDED_MANIFEST);
    let release = env::var("PROFILE").is_ok_and(|profile| profile == "release");

    match fs::read_to_string(STAGED_MANIFEST) {
        Ok(manifest) => fs::write(destination, manifest).expect("embed bundled-worker manifest"),
        Err(error) if release => panic!(
            "release build requires {STAGED_MANIFEST}; run scripts/build-worker.sh Release: {error}"
        ),
        Err(_) => fs::write(destination, "null\n").expect("write debug manifest sentinel"),
    }
}
