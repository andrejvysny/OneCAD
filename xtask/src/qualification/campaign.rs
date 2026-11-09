use super::manifest;
use crate::evidence::process::{self, Limits};
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs, path::Path, time::Duration};

pub(super) const REQUIRED: &[&str] = &[
    "platform",
    "hardware",
    "sdk",
    "xcode",
    "architecture",
    "toolchain",
    "cargo",
    "pinned-occt",
    "worker",
    "addon-target",
    "addon",
    "fmt",
    "clippy",
    "workspace-inventory",
    "workspace",
    "ctest-inventory",
    "ctest",
    "layers",
    "modeling-coverage",
    "modeling-contracts",
    "references",
    "python",
    "fuzz",
    "native",
    "metal-inventory",
    "metal",
    "captures",
    "package",
    "packaged-worker",
    "package-archive",
];

pub(super) fn run(args: &[String]) -> crate::Result {
    if args.len() != 2 || args[0] != "--out" {
        return Err("usage: cargo xtask qualify-macos --out DIRECTORY".into());
    }
    let requested = std::path::absolute(&args[1])?;
    let out = fs::canonicalize(requested.parent().ok_or("output parent required")?)?
        .join(requested.file_name().ok_or("output name required")?);
    if out.exists() || out.starts_with(std::env::current_dir()?) {
        return Err("qualification output must be new and outside the repository".into());
    }
    fs::create_dir_all(&out)?;
    let source = manifest::source()?;
    let mut state = json!({"schemaVersion":1,"status":"incomplete","source":source,
        "classification":"automated available-host qualification; physical input, latest OS and final integration acceptance remain separate",
        "phases":[],"artifacts":{}});
    let result = execute(&out, &mut state);
    if let Err(error) = &result {
        state["failure"] = json!(error.to_string());
    } else if state["source"] != manifest::source()? {
        state["failure"] = json!("source changed during campaign");
    } else {
        state["status"] = json!("passed");
    }
    state["artifacts"] = json!(manifest::artifacts(&out)?);
    fs::write(
        out.join("campaign.json"),
        serde_json::to_vec_pretty(&state)?,
    )?;
    result?;
    manifest::verify(&out)
}

fn execute(out: &Path, state: &mut Value) -> crate::Result {
    if std::env::consts::OS != "macos" {
        return Err("qualify-macos requires macOS; no lane passed".into());
    }
    let mut environment: BTreeMap<String, String> = std::env::vars().collect();
    let root = std::env::current_dir()?;
    let occt = environment
        .get("ONECAD_OCCT_ROOT")
        .cloned()
        .unwrap_or_else(|| {
            root.join("worker/build-qualification-occt-prefix/8.0.1")
                .display()
                .to_string()
        });
    environment.insert("ONECAD_OCCT_ROOT".into(), occt.clone());
    environment.insert("ONECAD_OCCT_VERSION".into(), "8.0.1".into());
    environment.insert(
        "ONECAD_OCCT_BUILD_ID".into(),
        "onecad-occt-8.0.1-b8f597c67781-kp1".into(),
    );
    environment.insert(
        "ONECAD_WORKER_BUILD_DIR".into(),
        root.join("worker/build").display().to_string(),
    );
    environment.insert(
        "ONECAD_WORKER_PATH".into(),
        root.join("worker/build/onecad-worker")
            .display()
            .to_string(),
    );
    environment.insert(
        "ONECAD_TEST_ADDON_COMPONENT".into(),
        root.join("packaging/binaries/addons/dev.onecad.bambu-studio/extension.wasm")
            .display()
            .to_string(),
    );
    environment.insert("ONECAD_REQUIRE_WORKER".into(), "1".into());
    environment.insert("ONECAD_REQUIRE_GPU".into(), "1".into());
    environment.remove("ONECAD_RENDER_CAPTURE_DIR");
    let mut phase = |name: &str, program: &str, args: &[&str]| -> crate::Result {
        let started = std::time::Instant::now();
        let arguments = phase_arguments(name, program, args, out);
        let argument_refs: Vec<_> = arguments.iter().map(String::as_str).collect();
        let result = process::run_limited(
            out,
            name,
            "env",
            &argument_refs,
            &environment,
            true,
            Limits {
                timeout: Duration::from_secs(7200),
                output_bytes: 128 * 1024 * 1024,
            },
        );
        state["phases"].as_array_mut().unwrap().push(
            json!({"name":name,"passed":result.is_ok(),"elapsedMs":started.elapsed().as_millis()}),
        );
        if !out.join(format!("{name}.command.json")).exists() {
            fs::write(
                out.join(format!("{name}.command.json")),
                serde_json::to_vec_pretty(&json!({
                    "program":"env","args":arguments,"status":null,"signal":null,
                    "error":result.as_ref().err().map(ToString::to_string)
                }))?,
            )?;
        }
        fs::write(out.join("progress.json"), serde_json::to_vec_pretty(state)?)?;
        result.map(|_| ())
    };
    metadata(&mut phase)?;
    phase(
        "pinned-occt",
        "scripts/build-pinned-occt.sh",
        &[
            "--version",
            "8.0.1",
            "--prefix",
            &occt,
            "--build-dir",
            &root
                .join("worker/build-qualification-occt")
                .display()
                .to_string(),
            "--build-id",
            "onecad-occt-8.0.1-b8f597c67781-kp1",
        ],
    )?;
    phase("worker", "scripts/build-worker.sh", &["Release"])?;
    phase(
        "addon-target",
        "rustup",
        &["target", "add", "wasm32-unknown-unknown"],
    )?;
    phase("addon", "scripts/build-addons.sh", &["addons/bambu-studio"])?;
    validation(out, &mut phase)
}

type Phase<'a> = dyn FnMut(&str, &str, &[&str]) -> crate::Result + 'a;

pub(super) fn phase_arguments(name: &str, program: &str, args: &[&str], out: &Path) -> Vec<String> {
    // Clear inherited capture settings too; diagnostics are not renderer receipts.
    let mut arguments = vec!["-u".into(), "ONECAD_RENDER_CAPTURE_DIR".into()];
    if name == "metal" {
        arguments.push(format!(
            "ONECAD_RENDER_CAPTURE_DIR={}",
            out.join("metal").display()
        ));
    }
    arguments.push(program.into());
    arguments.extend(args.iter().map(|arg| (*arg).into()));
    arguments
}

fn metadata(phase: &mut Phase<'_>) -> crate::Result {
    phase("platform", "sw_vers", &[])?;
    phase(
        "hardware",
        "system_profiler",
        &["SPHardwareDataType", "SPDisplaysDataType", "-json"],
    )?;
    phase("sdk", "xcrun", &["--show-sdk-version"])?;
    phase("xcode", "xcodebuild", &["-version"])?;
    phase("architecture", "uname", &["-m"])?;
    phase("toolchain", "rustc", &["-Vv"])?;
    phase("cargo", "cargo", &["-V"])
}

fn validation(out: &Path, phase: &mut Phase<'_>) -> crate::Result {
    phase("fmt", "cargo", &["fmt", "--all", "--check"])?;
    phase(
        "clippy",
        "cargo",
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    phase(
        "workspace-inventory",
        "cargo",
        &["test", "--workspace", "--", "--list"],
    )?;
    phase(
        "workspace",
        "cargo",
        &[
            "test",
            "--workspace",
            "--no-fail-fast",
            "--",
            "--test-threads=1",
        ],
    )?;
    phase(
        "ctest-inventory",
        "ctest",
        &["--test-dir", "worker/build", "--show-only=json-v1"],
    )?;
    phase(
        "ctest",
        "ctest",
        &[
            "--test-dir",
            "worker/build",
            "--output-on-failure",
            "--no-tests=error",
        ],
    )?;
    fs::write(
        out.join("ctest-inventory.json"),
        serde_json::to_vec_pretty(&super::inventories::ctest(out)?)?,
    )?;
    fs::write(
        out.join("workspace-inventory.json"),
        serde_json::to_vec_pretty(&super::inventories::workspace(out)?)?,
    )?;
    for (name, command) in [
        ("layers", "check-rust-layers"),
        ("modeling-coverage", "verify-modeling-coverage"),
        ("modeling-contracts", "verify-modeling-contracts"),
        ("references", "verify-reference-inputs"),
        ("python", "python-oracles"),
    ] {
        phase(name, "cargo", &["xtask", command])?;
    }
    phase(
        "fuzz",
        "cargo",
        &[
            "test",
            "-p",
            "onecad-core",
            "-p",
            "onecad-protocol",
            "--test",
            "bounded_admission_fuzz",
            "--test",
            "bounded_mesh_fuzz",
            "--",
            "--nocapture",
        ],
    )?;
    phase(
        "native",
        "cargo",
        &[
            "xtask",
            "native-host-gate",
            &out.join("native").display().to_string(),
        ],
    )?;
    fs::write(
        out.join("nested-children.json"),
        serde_json::to_vec_pretty(&super::nested::verify(&out.join("native"))?)?,
    )?;
    phase(
        "metal-inventory",
        "cargo",
        &[
            "test",
            "-p",
            "onecad-render",
            "--lib",
            "--",
            "--ignored",
            "--list",
        ],
    )?;
    phase(
        "metal",
        "cargo",
        &[
            "test",
            "-p",
            "onecad-render",
            "--lib",
            "--",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ],
    )?;
    fs::write(
        out.join("metal-inventory.json"),
        serde_json::to_vec_pretty(&super::inventories::metal(out)?)?,
    )?;
    phase(
        "captures",
        "cargo",
        &[
            "xtask",
            "verify-render-captures",
            &out.join("metal").display().to_string(),
        ],
    )?;
    phase(
        "package",
        "scripts/package-macos.sh",
        &[&out.join("OneCAD.app").display().to_string()],
    )?;
    phase(
        "packaged-worker",
        &out.join("OneCAD.app/Contents/MacOS/onecad-worker")
            .display()
            .to_string(),
        &["--selftest"],
    )?;
    phase(
        "package-archive",
        "tar",
        &[
            "-czf",
            &out.join("OneCAD.app.tar.gz").display().to_string(),
            "-C",
            &out.display().to_string(),
            "OneCAD.app",
        ],
    )?;
    fs::remove_dir_all(out.join("OneCAD.app"))?;
    Ok(())
}
