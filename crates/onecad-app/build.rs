use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const STAGED_MANIFEST: &str = "../../packaging/binaries/onecad-worker-manifest.json";
const EMBEDDED_MANIFEST: &str = "onecad-worker-manifest.json";
const EMBEDDED_DIR_ENV: &str = "ONECAD_EMBEDDED_WORKER_DIR";
const EMBEDDED_IDENTITY: &str = "onecad-embedded-kernel.json";
const EMBEDDED_LINK: &str = "onecad-embedded-link.txt";

fn main() {
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo must set OUT_DIR"));
    let release = env::var("PROFILE").is_ok_and(|profile| profile == "release");
    if env::var_os("CARGO_FEATURE_SIDECAR").is_some() {
        println!("cargo:rerun-if-changed={STAGED_MANIFEST}");
        embed_worker_manifest(&out_dir, release);
    } else {
        // No sidecar in this build: nothing reads the manifest, and an iOS
        // release must not demand a desktop worker staged beside the sources.
        fs::write(out_dir.join(EMBEDDED_MANIFEST), "null\n").expect("write manifest sentinel");
    }
    if env::var_os("CARGO_FEATURE_EMBEDDED_KERNEL").is_some() {
        println!("cargo:rerun-if-env-changed={EMBEDDED_DIR_ENV}");
        embed_kernel_identity(&out_dir, release);
    }
}

fn embed_worker_manifest(out_dir: &Path, release: bool) {
    let destination = out_dir.join(EMBEDDED_MANIFEST);
    match fs::read_to_string(STAGED_MANIFEST) {
        Ok(manifest) => fs::write(destination, manifest).expect("embed bundled-worker manifest"),
        Err(error) if release => panic!(
            "release build requires {STAGED_MANIFEST}; run scripts/build-worker.sh Release: {error}"
        ),
        Err(_) => fs::write(destination, "null\n").expect("write debug manifest sentinel"),
    }
}

/// The embedded kernel's identity (manifest v2) from the bundle directory
/// onecad-embedded-worker links. A release build requires it, for this exact
/// platform, and an iOS release refuses one built with test hooks.
fn embed_kernel_identity(out_dir: &Path, release: bool) {
    let destination = out_dir.join(EMBEDDED_IDENTITY);
    let Some(dir) = env::var_os(EMBEDDED_DIR_ENV).filter(|dir| !dir.is_empty()) else {
        assert!(
            !release,
            "a release build with embedded-kernel requires {EMBEDDED_DIR_ENV} \
             (scripts/build-ios-worker.sh --out <dir>)"
        );
        fs::write(destination, "").expect("write identity sentinel");
        return;
    };
    let dir = PathBuf::from(dir);
    let source = dir.join(EMBEDDED_IDENTITY);
    println!("cargo:rerun-if-changed={}", source.display());
    let identity = fs::read_to_string(&source)
        .unwrap_or_else(|error| panic!("read {}: {error}", source.display()));
    if release {
        check_release_identity(&identity, &source);
    }
    fs::write(destination, &identity).expect("embed kernel identity");
    add_test_rpaths(&dir);
}

fn check_release_identity(identity: &str, source: &Path) {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let simulator = env::var("TARGET").is_ok_and(|target| target.ends_with("-sim"));
    let platform = match (target_os.as_str(), simulator) {
        ("ios", true) => "iphonesimulator",
        ("ios", false) => "iphoneos",
        _ => "macosx",
    };
    assert!(
        identity.contains(&format!("\"platform\": \"{platform}\"")),
        "{} names another platform; this release needs {platform}",
        source.display()
    );
    assert!(
        target_os != "ios" || identity.contains("\"testHooks\": false"),
        "{} was built with --test-hooks; an iOS release must not answer Debug.Busy",
        source.display()
    );
}

/// The macOS test lane links the desktop OCCT dylibs (`@rpath/...`). This
/// crate's test binaries (unit and integration) need the bundle's rpath, which
/// the build script of a dependency (onecad-embedded-worker) cannot add to
/// them. `rustc-link-arg` only reaches linked targets — this rlib has none
/// besides its tests — and an iOS link file carries no rpath at all.
fn add_test_rpaths(dir: &Path) {
    let link = dir.join(EMBEDDED_LINK);
    println!("cargo:rerun-if-changed={}", link.display());
    let Ok(directives) = fs::read_to_string(&link) else {
        return;
    };
    for rpath in directives
        .lines()
        .filter_map(|line| line.strip_prefix("rpath="))
    {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{rpath}");
    }
}
