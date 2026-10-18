//! Builds `onecad_worker_lib` (worker/CMakeLists.txt) with CMake in this
//! spike's own target dir and links it, using the manifest CMake generates
//! (`onecad_worker_lib.link`). Provenance flags match CLAUDE.md's manual worker
//! configure, so the in-process hello carries the sidecar's fingerprint.
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const DEFAULT_BUILD_ID: &str = "onecad-occt-8.0.1-b8f597c67781-kp1";

fn run(cmd: &mut Command) {
    let status = cmd
        .status()
        .unwrap_or_else(|e| panic!("spawn {cmd:?}: {e}"));
    assert!(status.success(), "{cmd:?} failed: {status}");
}

fn configure(worker: &Path, build_dir: &Path) {
    let home = env::var("HOME").expect("HOME");
    let occt_root =
        env::var("ONECAD_OCCT_ROOT").unwrap_or_else(|_| format!("{home}/.onecad-occt/8.0.1"));
    let build_id =
        env::var("ONECAD_OCCT_BUILD_ID").unwrap_or_else(|_| DEFAULT_BUILD_ID.to_string());
    let mut cmd = Command::new("cmake");
    cmd.arg("-S")
        .arg(worker)
        .arg("-B")
        .arg(build_dir)
        .args(["-G", "Ninja", "-DCMAKE_BUILD_TYPE=Release"])
        .arg(format!("-DONECAD_OCCT_ROOT={occt_root}"))
        .arg("-DONECAD_OCCT_VERSION=8.0.1")
        .arg(format!("-DONECAD_OCCT_BUILD_ID={build_id}"))
        .arg(format!("-DONECAD_WORKER_BUILD_DIR={}", build_dir.display()));
    run(&mut cmd);
}

fn emit_link_manifest(manifest: &Path) {
    let text = fs::read_to_string(manifest).expect("read onecad_worker_lib.link");
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let (kind, value) = line.split_once('=').expect("kind=value");
        let path = Path::new(value);
        match kind {
            "static" => {
                let dir = path.parent().expect("archive dir");
                let stem = path.file_stem().unwrap().to_str().unwrap();
                let name = stem.strip_prefix("lib").unwrap_or(stem);
                println!("cargo:rustc-link-search=native={}", dir.display());
                println!("cargo:rustc-link-lib=static={name}");
            }
            "dylib" => println!("cargo:rustc-link-arg={value}"),
            "rpath" => println!("cargo:rustc-link-arg=-Wl,-rpath,{value}"),
            other => panic!("unknown link manifest kind {other}"),
        }
    }
    println!("cargo:rustc-link-lib=dylib=c++");
}

fn main() {
    let crate_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let worker = crate_dir
        .join("../../worker")
        .canonicalize()
        .expect("worker dir");
    // Outside OUT_DIR on purpose: OUT_DIR moves whenever this script changes,
    // and a fresh worker_core compile is several minutes.
    let build_dir = crate_dir.join("target/worker-lib-build");
    fs::create_dir_all(&build_dir).unwrap();
    let build_dir = build_dir.canonicalize().unwrap();

    if !build_dir.join("CMakeCache.txt").exists() {
        configure(&worker, &build_dir);
    }
    let jobs = env::var("SPIKE_CMAKE_JOBS").unwrap_or_else(|_| "4".into());
    run(Command::new("cmake").arg("--build").arg(&build_dir).args([
        "--target",
        "onecad_worker_lib",
        "--parallel",
        &jobs,
    ]));
    emit_link_manifest(&build_dir.join("onecad_worker_lib.link"));

    cc::Build::new()
        .cpp(true)
        .std("c++17")
        .file("csrc/cout_bridge.cpp")
        .file("csrc/lock_probe.cpp")
        .compile("spike_csrc");

    println!("cargo:rerun-if-changed=csrc/cout_bridge.cpp");
    println!("cargo:rerun-if-changed=csrc/lock_probe.cpp");
    println!("cargo:rerun-if-changed={}", worker.join("src").display());
    println!(
        "cargo:rerun-if-changed={}",
        worker.join("CMakeLists.txt").display()
    );
    println!("cargo:rerun-if-env-changed=ONECAD_OCCT_ROOT");
    println!("cargo:rerun-if-env-changed=ONECAD_OCCT_BUILD_ID");
}
