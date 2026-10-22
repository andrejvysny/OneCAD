//! Links the in-process worker bundle staged by `scripts/build-ios-worker.sh`.
//!
//! `ONECAD_EMBEDDED_WORKER_DIR` names its `--out` directory. Set: the bundle,
//! libc++ and the derived system libraries are linked, the identity manifest is
//! embedded, and `cfg(onecad_embedded_linked)` is on. Unset: the crate builds
//! stubs that fail with "embedded kernel not linked", so host `cargo check`,
//! clippy and tests keep working — except an iOS release build, which must
//! never ship without a kernel and fails here.
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const DIR_ENV: &str = "ONECAD_EMBEDDED_WORKER_DIR";
const LINK_FILE: &str = "onecad-embedded-link.txt";
const IDENTITY_FILE: &str = "onecad-embedded-kernel.json";

fn main() {
    println!("cargo:rerun-if-env-changed={DIR_ENV}");
    println!("cargo::rustc-check-cfg=cfg(onecad_embedded_linked)");
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo sets OUT_DIR"));
    let identity_out = out_dir.join(IDENTITY_FILE);
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let release = env::var("PROFILE").is_ok_and(|profile| profile == "release");

    match env::var_os(DIR_ENV).filter(|dir| !dir.is_empty()) {
        Some(dir) => link_bundle(Path::new(&dir), &target_os, &identity_out),
        None if target_os == "ios" && release => panic!(
            "an iOS release build requires {DIR_ENV} (scripts/build-ios-worker.sh --out <dir>)"
        ),
        None => fs::write(&identity_out, "").expect("write empty identity sentinel"),
    }
}

/// The platform name the manifest must carry for this Rust target.
fn expected_platform(target_os: &str) -> &'static str {
    let simulator = env::var("CARGO_CFG_TARGET_ABI").is_ok_and(|abi| abi == "sim")
        || env::var("TARGET").is_ok_and(|target| target.ends_with("-sim"));
    match (target_os, simulator) {
        ("ios", true) => "iphonesimulator",
        ("ios", false) => "iphoneos",
        ("macos", _) => "macosx",
        (other, _) => panic!("the embedded kernel is built for Apple targets only, not {other}"),
    }
}

fn link_bundle(dir: &Path, target_os: &str, identity_out: &Path) {
    let identity_path = dir.join(IDENTITY_FILE);
    let link_path = dir.join(LINK_FILE);
    for path in [
        &identity_path,
        &link_path,
        &dir.join("libonecad_worker_bundle.a"),
    ] {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    let identity = fs::read_to_string(&identity_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", identity_path.display()));
    let platform = expected_platform(target_os);
    let needle = format!("\"platform\": \"{platform}\"");
    assert!(
        identity.contains(&needle),
        "{} was built for another platform; this target needs {platform}",
        identity_path.display()
    );
    let directives = fs::read_to_string(&link_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", link_path.display()));
    println!("cargo:rustc-link-search=native={}", dir.display());
    for line in directives.lines().filter(|line| !line.trim().is_empty()) {
        emit_directive(line, target_os);
    }
    fs::write(identity_out, identity).expect("embed the kernel identity");
    println!("cargo:rustc-cfg=onecad_embedded_linked");
}

/// One `kind=value` line of the link file. iOS links nothing outside the SDK:
/// a dylib or rpath line there means a desktop bundle was pointed at.
fn emit_directive(line: &str, target_os: &str) {
    let (kind, value) = line
        .split_once('=')
        .unwrap_or_else(|| panic!("malformed link directive {line:?}"));
    let desktop_only = matches!(kind, "dylib" | "dylib-dir" | "rpath");
    assert!(
        !(desktop_only && target_os == "ios"),
        "link directive {line:?} is not allowed for iOS"
    );
    match kind {
        // Not bundled into the rlib: the archive is large and the final link
        // reads it from the search path above.
        "static" => println!("cargo:rustc-link-lib=static:-bundle={value}"),
        "lib" | "dylib" => println!("cargo:rustc-link-lib=dylib={value}"),
        "framework" => println!("cargo:rustc-link-lib=framework={value}"),
        "dylib-dir" => println!("cargo:rustc-link-search=native={value}"),
        // Reaches this crate's own test binaries; onecad-app's build script
        // adds the same rpath to its tests (macOS in-process test lane).
        "rpath" => println!("cargo:rustc-link-arg=-Wl,-rpath,{value}"),
        other => panic!("unknown link directive kind {other:?}"),
    }
}
