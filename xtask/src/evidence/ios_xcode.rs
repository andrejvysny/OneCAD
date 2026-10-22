//! Xcode steps of `ios-simulator-gate`: build for testing and test without
//! building (step 5), result bundle and app-container collection (steps 6-7)
//! and the unsigned Release products with their nm / otool evidence (step 9).
use super::{
    ios_evidence::{self, MAC_FIXTURE},
    ios_gate::Gate,
    ios_release::Platform,
    ios_verify::BUNDLE_ID,
    process::{self, Limits, Output},
    Result,
};
use serde_json::Value;
use std::{fs, path::Path, time::Duration};

pub(super) const PROJECT: &str = "platforms/ios/OneCAD.xcodeproj";
pub(super) const BUILD: Limits = Limits {
    timeout: Duration::from_secs(3 * 3600),
    output_bytes: 256 * 1024 * 1024,
};
const LARGE_OUTPUT: Limits = Limits {
    timeout: Duration::from_secs(600),
    output_bytes: 128 * 1024 * 1024,
};

pub(super) fn json_output(output: &Output) -> Result<Value> {
    Ok(serde_json::from_str(&output.stdout)?)
}

const TEST_PLAN: [&str; 6] = [
    "-project",
    PROJECT,
    "-scheme",
    "OneCADiPad",
    "-testPlan",
    "iPadAcceptance",
];

/// Step 5: build for testing, then test without building on the same products.
pub(super) fn xctest(gate: &mut Gate, run_id: &str, udid: &str) -> Result<()> {
    let destination = format!("platform=iOS Simulator,id={udid}");
    let derived = gate.dir.join("DerivedData").to_string_lossy().into_owned();
    let mut build: Vec<&str> = vec!["build-for-testing"];
    build.extend(TEST_PLAN);
    build.extend([
        "-configuration",
        "Acceptance",
        "-destination",
        &destination,
        "-derivedDataPath",
        &derived,
    ]);
    gate.run_with(
        "xcodebuild-build-for-testing",
        "xcodebuild",
        &build,
        true,
        BUILD,
    )?;
    test_without_building(gate, run_id, udid, &destination, &derived)
}

fn test_without_building(
    gate: &mut Gate,
    run_id: &str,
    udid: &str,
    destination: &str,
    derived: &str,
) -> Result<()> {
    let result = gate.dir.join("iPadAcceptance.xcresult");
    if result.exists() {
        fs::remove_dir_all(&result)?;
    }
    // A reused simulator keeps the app's data container (poison strikes,
    // recovery, recents, earlier runs' artifacts): every run starts from a
    // fresh install. Not installed yet is fine.
    gate.run_with(
        "simctl-uninstall",
        "xcrun",
        &["simctl", "uninstall", udid, BUNDLE_ID],
        false,
        Limits::DEFAULT,
    )?;
    gate.env
        .insert("TEST_RUNNER_ONECAD_RUN_ID".into(), run_id.into());
    let fixture = gate.dir.join(MAC_FIXTURE).to_string_lossy().into_owned();
    gate.env
        .insert("TEST_RUNNER_ONECAD_MAC_FIXTURE".into(), fixture);
    let result = result.to_string_lossy().into_owned();
    let mut test: Vec<&str> = vec!["test-without-building"];
    test.extend(TEST_PLAN);
    test.extend([
        "-destination",
        destination,
        "-derivedDataPath",
        derived,
        "-resultBundlePath",
        &result,
    ]);
    // Failing tests are rejected by the verifier from the result bundle.
    gate.run_with(
        "xcodebuild-test-without-building",
        "xcodebuild",
        &test,
        false,
        BUILD,
    )?;
    Ok(())
}

/// Steps 6-7: xcresult JSON, attachments and the app's acceptance directory.
pub(super) fn collect(gate: &Gate, run_id: &str, udid: &str) -> Result<()> {
    let result = gate
        .dir
        .join("iPadAcceptance.xcresult")
        .to_string_lossy()
        .into_owned();
    for part in ["tests", "summary"] {
        let args = [
            "xcresulttool",
            "get",
            "test-results",
            part,
            "--path",
            &result,
        ];
        let output = gate.run_with(
            &format!("xcresult-{part}"),
            "xcrun",
            &args,
            true,
            LARGE_OUTPUT,
        )?;
        gate.write(&format!("xcresult-{part}.json"), &json_output(&output)?)?;
    }
    let attachments = gate.dir.join("attachments").to_string_lossy().into_owned();
    let args = [
        "xcresulttool",
        "export",
        "attachments",
        "--path",
        &result,
        "--output-path",
        &attachments,
    ];
    fs::create_dir_all(&attachments)?;
    gate.run("xcresult-attachments", "xcrun", &args)?;
    let container = gate.run(
        "app-container",
        "xcrun",
        &["simctl", "get_app_container", udid, BUNDLE_ID, "data"],
    )?;
    let source = Path::new(container.stdout.trim())
        .join("Documents/acceptance")
        .join(run_id);
    copy_run_directory(&source, &gate.dir.join("artifacts"))
}

/// Copies the run's acceptance directory. The verifier sees only what is
/// copied: anything but a regular file (a directory, a symlink) is rejected
/// here instead.
pub(super) fn copy_run_directory(source: &Path, target: &Path) -> Result<()> {
    if target.exists() {
        fs::remove_dir_all(target)?;
    }
    fs::create_dir_all(target)?;
    if !source.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            return Err(format!(
                "acceptance artifact is not a regular file: {}",
                entry.file_name().to_string_lossy()
            )
            .into());
        }
        fs::copy(entry.path(), target.join(entry.file_name()))?;
    }
    Ok(())
}

/// Step 9: an unsigned Release product and its nm / otool evidence.
pub(super) fn release(gate: &Gate, platform: Platform) -> Result<()> {
    let derived = gate
        .dir
        .join("DerivedData-release")
        .to_string_lossy()
        .into_owned();
    let args = [
        "build",
        "-project",
        PROJECT,
        "-scheme",
        "OneCADiPad",
        "-configuration",
        "Release",
        "-destination",
        platform.destination(),
        "-derivedDataPath",
        &derived,
        "CODE_SIGNING_ALLOWED=NO",
    ];
    gate.run_with(
        &format!("xcodebuild-release-{}", platform.sdk()),
        "xcodebuild",
        &args,
        true,
        BUILD,
    )?;
    let product = ios_evidence::release_product(&gate.dir, platform)
        .to_string_lossy()
        .into_owned();
    let logs = gate.dir.join("release").join(platform.sdk());
    fs::create_dir_all(&logs)?;
    let tool = |name: &str, args: &[&str]| {
        process::run_limited(&logs, name, "xcrun", args, &gate.env, true, LARGE_OUTPUT).map(drop)
    };
    tool("nm", &["nm", "-j", &product])?;
    // Distinct names: `otool-l.log` and `otool-L.log` are one file on a
    // case-insensitive volume.
    tool("otool-load-commands", &["otool", "-l", &product])?;
    tool("otool-libraries", &["otool", "-L", &product])
}
