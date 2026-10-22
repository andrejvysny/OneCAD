//! `cargo xtask ios-simulator-gate`: the reproducible runner of
//! docs/ios/SIMULATOR_GATE.md (steps 1-10). Every command is logged as
//! `<name>.log` + `<name>.command.json`; `acceptance.json` is written last and
//! only when `ios_evidence::verify` finds nothing to reject.
use super::{
    ios_evidence::{self, BUNDLES, MAC_FIXTURE, SIMULATOR_BUNDLE},
    ios_release::Platform,
    ios_sim,
    ios_xcode::{collect, json_output, release, xctest, BUILD},
    process::{self, Limits, Output},
    Result,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

const USAGE: &str =
    "usage: cargo xtask ios-simulator-gate [--provenance-only | --verify-only] OUTPUT_DIRECTORY

  OUTPUT_DIRECTORY   evidence directory, new or empty (outside the source tree or ignored)
  --provenance-only  steps 1-2 and a read-only simulator resolution; builds nothing
  --verify-only      re-verify an existing evidence directory; never writes acceptance.json

Environment: ONECAD_WORKER_PATH (Mac sidecar, required), ONECAD_OCCT_ROOT,
ONECAD_SIM_UDID, ONECAD_SIM_DEVICE_TYPE, ONECAD_SIM_RUNTIME. Contract: docs/ios/SIMULATOR_GATE.md";

pub(super) struct Gate {
    pub dir: PathBuf,
    pub env: BTreeMap<String, String>,
}

impl Gate {
    pub(super) fn run(&self, name: &str, program: &str, args: &[&str]) -> Result<Output> {
        process::run(&self.dir, name, program, args, &self.env, true)
    }

    pub(super) fn run_with(
        &self,
        name: &str,
        program: &str,
        args: &[&str],
        success: bool,
        limits: Limits,
    ) -> Result<Output> {
        process::run_limited(&self.dir, name, program, args, &self.env, success, limits)
    }

    pub(super) fn write(&self, name: &str, value: &Value) -> Result<()> {
        let path = self.dir.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, serde_json::to_string_pretty(value)?)?;
        Ok(())
    }
}

pub(super) fn run(args: &[String]) -> Result<()> {
    let (mode, dir) = match args {
        [flag] if flag == "--help" || flag == "-h" => {
            println!("{USAGE}");
            return Ok(());
        }
        [dir] if !dir.starts_with('-') => ("full", dir),
        [mode, dir] if mode == "--provenance-only" || mode == "--verify-only" => {
            (mode.as_str(), dir)
        }
        _ => return Err(USAGE.into()),
    };
    let dir = std::path::absolute(dir)?;
    let root = std::env::current_dir()?;
    if mode == "--verify-only" {
        return report(&dir, &root, false);
    }
    // A reused directory would mix another run's verdict, artifacts or run
    // id into this one's evidence.
    fs::create_dir_all(&dir)?;
    if fs::read_dir(&dir)?.next().is_some() {
        return Err(format!(
            "{} is not empty; use a new evidence directory",
            dir.display()
        )
        .into());
    }
    let mut gate = Gate {
        dir,
        env: std::env::vars().collect(),
    };
    let run_id = provenance(&gate)?;
    if mode == "--provenance-only" {
        let udid = simulator(&gate, false)?;
        println!("run id {run_id}; simulator {udid}");
        return Ok(());
    }
    full(&mut gate, &run_id)?;
    // The run takes hours: the verified source must still be the tree it
    // built and tested, and the inventory/expectations the verifier reads.
    gate.run("source-final", "git", &["rev-parse", "HEAD"])?;
    gate.run("source-status-final", "git", &["status", "--porcelain=v1"])?;
    report(&gate.dir, &root, true)
}

/// Runs the verifier; writes `verification.json` and, when `accept`,
/// `acceptance.json` only if nothing was rejected.
pub(super) fn report(dir: &Path, root: &Path, accept: bool) -> Result<()> {
    let failures = ios_evidence::verify(dir, root);
    let verdict = json!({ "passed": failures.is_empty(), "failures": failures });
    println!("{}", serde_json::to_string_pretty(&verdict)?);
    if !accept {
        return if failures.is_empty() {
            Ok(())
        } else {
            Err("evidence rejected".into())
        };
    }
    fs::write(
        dir.join("verification.json"),
        serde_json::to_string_pretty(&verdict)?,
    )?;
    if !failures.is_empty() {
        return Err(format!("iPad simulator gate rejected {} check(s)", failures.len()).into());
    }
    let run = ios_evidence::json(&dir.join("run.json"))?;
    let simulator =
        ios_evidence::json(&dir.join("kernel").join(format!("{SIMULATOR_BUNDLE}.json")))?;
    let acceptance = json!({
        // No replay test is inventoried and no trace is required yet, so
        // synthetic replay (class 2) is not claimed.
        "classification": "iPad Simulator: native XCTest and XCUITest with the in-process kernel, Mac geometry oracle, unsigned Release compile/link (evidence classes 3-5); no synthetic replay, physical device, Pencil hardware, performance or release qualification",
        "run": run,
        "simulatorKernel": simulator,
        "inventory": ios_evidence::json(&root.join(super::ios_verify::INVENTORY))?,
        "checks": ["provenance", "kernel bundles", "tests", "artifacts", "oracle", "release iphoneos", "release iphonesimulator"],
    });
    fs::write(
        dir.join("acceptance.json"),
        serde_json::to_string_pretty(&acceptance)?,
    )?;
    Ok(())
}

/// Steps 1-2: tool and source provenance, then a fresh run id.
fn provenance(gate: &Gate) -> Result<String> {
    let source = gate
        .run("source", "git", &["rev-parse", "HEAD"])?
        .text
        .trim()
        .to_string();
    gate.run("source-status", "git", &["status", "--porcelain=v1"])?;
    let tools: [(&str, &str, &[&str]); 11] = [
        ("platform", "sw_vers", &[]),
        ("xcode-select", "xcode-select", &["-p"]),
        ("xcodebuild-version", "xcodebuild", &["-version"]),
        ("xcodebuild-sdks", "xcodebuild", &["-showsdks"]),
        (
            "simctl-runtimes",
            "xcrun",
            &["simctl", "list", "runtimes", "-j"],
        ),
        (
            "simctl-devices",
            "xcrun",
            &["simctl", "list", "devices", "available", "-j"],
        ),
        ("toolchain", "rustc", &["-Vv"]),
        ("cargo", "cargo", &["-V"]),
        ("rust-targets", "rustup", &["target", "list", "--installed"]),
        ("xcodegen", "xcodegen", &["--version"]),
        (
            "simctl-devicetypes",
            "xcrun",
            &["simctl", "list", "devicetypes", "-j"],
        ),
    ];
    for (name, program, args) in tools {
        gate.run(name, program, args)?;
    }
    let unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let run_id = ios_sim::run_id(unix, &source, ios_sim::random_bytes()?);
    gate.write(
        "run.json",
        &json!({ "runId": run_id, "source": source, "startedUnix": unix }),
    )?;
    Ok(run_id)
}

fn update_run(gate: &Gate, fields: Value) -> Result<()> {
    let mut run = ios_evidence::json(&gate.dir.join("run.json"))?;
    for (key, value) in fields.as_object().into_iter().flatten() {
        run[key] = value.clone();
    }
    gate.write("run.json", &run)
}

/// Steps 3-9; the verifier (step 10) runs afterwards over what they left.
fn full(gate: &mut Gate, run_id: &str) -> Result<()> {
    // The independent oracle is the staged sidecar `native-host-gate` pins.
    let worker = super::host::worker(&gate.dir, &gate.env)?;
    kernel_bundles(gate)?;
    let udid = simulator(gate, true)?;
    update_run(
        gate,
        json!({ "udid": udid, "workerPath": worker.0, "workerSha256": worker.1 }),
    )?;
    gate.env.insert("ONECAD_REQUIRE_WORKER".into(), "1".into());
    gate.env
        .insert("ONECAD_WORKER_PATH".into(), worker.0.clone());
    let fixture = gate.dir.join(MAC_FIXTURE);
    let author = [("ONECAD_IOS_MAC_FIXTURE_OUT", gate.dir.join("mac-authored"))];
    oracle(gate, "oracle-author", &author)?;
    xctest(gate, run_id, &udid)?;
    collect(gate, run_id, &udid)?;
    let verify = [
        ("ONECAD_IOS_ACCEPTANCE_DIR", gate.dir.join("artifacts")),
        ("ONECAD_IOS_MAC_FIXTURE", fixture),
    ];
    oracle(gate, "oracle-verify", &verify)?;
    for platform in [Platform::Device, Platform::Simulator] {
        release(gate, platform)?;
    }
    Ok(())
}

/// Step 3: the kernel bundles Xcode links, built where build-rust.sh looks.
fn kernel_bundles(gate: &Gate) -> Result<()> {
    for (name, platform, config, hooks) in BUNDLES {
        let out = std::path::absolute(Path::new("target/ios-worker").join(name))?;
        let out = out.to_string_lossy().into_owned();
        let mut args = vec!["--platform", platform, "--config", config, "--out", &out];
        if hooks {
            args.push("--test-hooks");
        }
        gate.run_with(
            &format!("kernel-{name}"),
            "scripts/build-ios-worker.sh",
            &args,
            true,
            BUILD,
        )?;
        let identity = ios_evidence::json(&Path::new(&out).join("onecad-embedded-kernel.json"))?;
        gate.write(&format!("kernel/{name}.json"), &identity)?;
    }
    Ok(())
}

/// Step 4: ONECAD_SIM_UDID, else reuse or (when `create`) create the gate
/// simulator; boot it and wait for `bootstatus -b`.
fn simulator(gate: &Gate, create: bool) -> Result<String> {
    let devices = json_output(&gate.run(
        "select-devices",
        "xcrun",
        &["simctl", "list", "devices", "available", "-j"],
    )?)?;
    let udid = match gate.env.get("ONECAD_SIM_UDID").filter(|u| !u.is_empty()) {
        Some(udid) => {
            ios_sim::find_udid(&devices, udid)
                .ok_or("ONECAD_SIM_UDID is not an available simulator")?;
            udid.clone()
        }
        None => named_simulator(gate, &devices, create)?,
    };
    if create {
        // Already booted is fine; bootstatus decides.
        gate.run_with(
            "simctl-boot",
            "xcrun",
            &["simctl", "boot", &udid],
            false,
            Limits::DEFAULT,
        )?;
        gate.run(
            "simctl-bootstatus",
            "xcrun",
            &["simctl", "bootstatus", &udid, "-b"],
        )?;
    }
    Ok(udid)
}

fn named_simulator(gate: &Gate, devices: &Value, create: bool) -> Result<String> {
    let runtimes = json_output(&gate.run(
        "select-runtimes",
        "xcrun",
        &["simctl", "list", "runtimes", "-j"],
    )?)?;
    let runtime = ios_sim::runtime(
        &runtimes,
        gate.env.get("ONECAD_SIM_RUNTIME").map(String::as_str),
    )?;
    let device = gate
        .env
        .get("ONECAD_SIM_DEVICE_TYPE")
        .map_or(ios_sim::DEFAULT_DEVICE_TYPE, String::as_str);
    let types = json_output(&gate.run(
        "select-devicetypes",
        "xcrun",
        &["simctl", "list", "devicetypes", "-j"],
    )?)?;
    let type_id = ios_sim::device_type(&types, device)?;
    let name = ios_sim::gate_name(device, &runtime);
    if let Some(udid) = ios_sim::find_named(devices, &runtime, &name) {
        return Ok(udid);
    }
    if !create {
        return Ok(format!("(would create {name})"));
    }
    let created = gate.run(
        "simctl-create",
        "xcrun",
        &["simctl", "create", &name, &type_id, &runtime.identifier],
    )?;
    Ok(created.stdout.trim().to_string())
}

/// Oracle inputs; each half of step 8 gets only its own (a stray one from the
/// caller's shell would run the other half too).
const ORACLE_INPUTS: [&str; 4] = [
    "ONECAD_IOS_MAC_FIXTURE_OUT",
    "ONECAD_IOS_ACCEPTANCE_DIR",
    "ONECAD_IOS_MAC_FIXTURE",
    "ONECAD_IOS_ORACLE_REPORT",
];

/// Step 8 (one half): the ignored Mac oracle test with exactly `inputs`.
fn oracle(gate: &Gate, name: &str, inputs: &[(&str, PathBuf)]) -> Result<()> {
    let mut env = gate.env.clone();
    for variable in ORACLE_INPUTS {
        env.remove(variable);
    }
    for (variable, path) in inputs {
        env.insert((*variable).into(), path.to_string_lossy().into_owned());
    }
    let report = gate.dir.join(format!("{name}.json"));
    env.insert(
        "ONECAD_IOS_ORACLE_REPORT".into(),
        report.to_string_lossy().into_owned(),
    );
    let args = [
        "test",
        "--locked",
        "-p",
        "onecad-app",
        "--test",
        "ios_simulator_artifacts",
        "--",
        "--ignored",
        "--exact",
        "ios_simulator_artifacts",
        "--nocapture",
        "--test-threads=1",
    ];
    process::run_limited(&gate.dir, name, "cargo", &args, &env, false, BUILD)?;
    Ok(())
}
