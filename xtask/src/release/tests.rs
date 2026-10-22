//! Synthetic local inventories do not establish release or license compliance.
use super::{materials::*, prepare, verify, Result};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub(super) struct Fixture {
    _temporary: tempfile::TempDir,
    options: BTreeMap<String, PathBuf>,
}
impl Fixture {
    pub(super) fn path(&self, key: &str) -> &Path {
        &self.options[key]
    }
    pub(super) fn prepare(&self) -> Result<PathBuf> {
        prepare::prepare(&self.options)
    }
    pub(super) fn payload(&self) -> PathBuf {
        self.path("--output").join("materials")
    }
}
fn put(root: &Path, name: &str, value: &str) {
    save(&root.join(name), value.as_bytes()).unwrap();
}
pub(super) fn fixture() -> Fixture {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    let options: BTreeMap<_, _> = [
        ("--source", "source"),
        ("--worker", "worker"),
        ("--worker-manifest", "worker-manifest.json"),
        ("--occt-prefix", "occt-prefix"),
        ("--occt-archive", "occt-archive"),
        ("--eigen-prefix", "eigen"),
        ("--boost-prefix", "boost"),
        ("--boost-license", "boost-license"),
        ("--output", "output"),
    ]
    .into_iter()
    .map(|(key, name)| (key.to_owned(), root.join(name)))
    .collect();
    let source = &options["--source"];
    for name in REQUIRED_SOURCE {
        put(source, name, &format!("synthetic {name}\n"));
    }
    for name in [
        "crates/onecad-app/src/lib.rs",
        "crates/onecad-ui/src/lib.rs",
        "crates/onecad-render/src/lib.rs",
        "crates/onecad-extension-host/src/lib.rs",
        "xtask/src/main.rs",
        "worker/src/main.cpp",
    ] {
        put(source, name, &format!("synthetic {name}\n"));
    }
    put(
        source,
        ".gitignore",
        "ignored-file\n/target/\n/packaging/binaries/\n",
    );
    put(source, "ignored-file", "excluded\n");
    put(
        source,
        "target/wasm32-unknown-unknown/release/addon.wasm",
        "generated core module\n",
    );
    put(
        source,
        "packaging/binaries/addons/example/extension.wasm",
        "generated component\n",
    );
    put(
        root,
        "occt-archive",
        "synthetic OCCT cached archive bytes\n",
    );
    let archive_hash = hash(&fs::read(&options["--occt-archive"]).unwrap());
    put(source, "scripts/build-pinned-occt.sh", &format!("8.0.1 | V8_0_1)\nCOMMIT=\"b8f597c677811d1f9f4d8a97f5ae2825c0353a42\"\nARCHIVE_SHA256=\"{archive_hash}\"\n;;\nreadonly NORMALIZED_OPTIONS=\"BUILD_LIBRARY_TYPE=Shared\"\n"));
    let occt = Occt {
        schema_version: 1,
        version: "8.0.1".into(),
        tag: "V8_0_1".into(),
        source_commit: "b8f597c677811d1f9f4d8a97f5ae2825c0353a42".into(),
        archive_sha256: archive_hash,
        build_id: "synthetic-test-build".into(),
        kernel_policy_version: 1,
        normalized_build_options: "BUILD_LIBRARY_TYPE=Shared".into(),
    };
    save_json(
        &options["--occt-prefix"].join("share/onecad/occt-build.json"),
        &occt,
    )
    .unwrap();
    put(&options["--occt-prefix"], "share/onecad/occt-build.cmake", "set(ONECAD_OCCT_SOURCE_COMMIT \"b8f597c677811d1f9f4d8a97f5ae2825c0353a42\")\nset(ONECAD_OCCT_NORMALIZED_BUILD_OPTIONS \"BUILD_LIBRARY_TYPE=Shared\")\nset(ONECAD_OCCT_BUILT_ID \"synthetic-test-build\")\n");
    for name in ["LICENSE_LGPL_21.txt", "OCCT_LGPL_EXCEPTION.txt"] {
        put(
            &options["--occt-prefix"],
            &format!("share/doc/opencascade/{name}"),
            "synthetic license\n",
        );
    }
    put(
        &options["--eigen-prefix"],
        "share/eigen3/cmake/Eigen3ConfigVersion.cmake",
        "set(PACKAGE_VERSION \"5.0.1\")\n",
    );
    for name in ["README", "MPL2", "BSD", "APACHE", "MINPACK"] {
        put(
            &options["--eigen-prefix"],
            &format!("COPYING.{name}"),
            "synthetic license\n",
        );
    }
    put(
        &options["--boost-prefix"],
        "include/boost/version.hpp",
        "#define BOOST_VERSION 109000\n",
    );
    put(root, "boost-license", "synthetic Boost license\n");
    put(root, "worker", "synthetic bundled worker bytes\n");
    save_json(&options["--worker-manifest"], &json!({"manifestVersion":1,"binarySha256":hash(&fs::read(&options["--worker"]).unwrap()),"occt":{"version":"8.0.1","fingerprint":"fixture-fingerprint"}})).unwrap();
    execute(Command::new("git").args(["init", "--quiet"]).arg(source)).unwrap();
    git(source, &["add", "."]).unwrap();
    git(
        source,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--quiet",
            "-m",
            "Synthetic source fixture",
        ],
    )
    .unwrap();
    Fixture {
        _temporary: temporary,
        options,
    }
}

#[test]
fn clean_snapshot_final_worker_and_dependency_evidence() {
    let fixture = fixture();
    let archive = fixture.prepare().unwrap();
    verify::archive(&archive, fixture.path("--worker")).unwrap();
    let metadata: Value = serde_json::from_slice(
        &fs::read(fixture.payload().join("release-materials.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(metadata["source"]["dirty"], false);
    assert_eq!(metadata["source"]["exactTags"], json!([]));
    assert_eq!(
        metadata["dependencies"],
        json!({"eigen":"5.0.1", "boost":"1.90.0"})
    );
    assert_eq!(
        metadata["validation"],
        json!({"buildProvenance":"unverified","relink":"unverified","publication":"unverified","transitiveLicenseInventory":"incomplete"})
    );
    assert!(!fixture.payload().join("source/ignored-file").exists());
    assert!(!fixture.payload().join("source/target").exists());
    assert!(!fixture.payload().join("source/packaging/binaries").exists());
    for path in [
        "crates/onecad-extension-api/wit/extension.wit",
        "crates/onecad-extension-sdk/src/lib.rs",
        "crates/onecad-extension-sdk/src/bin/onecad-addon-build.rs",
        "addons/bambu-studio/manifest.json",
        "addons/bambu-studio/src/lib.rs",
        "scripts/build-addons.sh",
    ] {
        assert_eq!(
            fs::read(fixture.payload().join("source").join(path)).unwrap(),
            fs::read(fixture.path("--source").join(path)).unwrap(),
        );
    }
    assert_eq!(
        fs::read(fixture.payload().join("occt/OCCT-V8_0_1.tar.gz")).unwrap(),
        fs::read(fixture.path("--occt-archive")).unwrap()
    );
    assert!(
        fs::read_to_string(fixture.path("--output").join("SHA256SUMS"))
            .unwrap()
            .contains(&hash(&fs::read(archive).unwrap()))
    );
}

#[test]
fn dirty_snapshot_tracks_current_bytes_deletions_tags_and_untracked_files() {
    let fixture = fixture();
    let root = fixture.path("--source");
    git(root, &["tag", "fixture-head"]).unwrap();
    put(root, "worker/src/main.cpp", "staged\n");
    git(root, &["add", "worker/src/main.cpp"]).unwrap();
    put(root, "worker/src/main.cpp", "unstaged\n");
    put(root, "untracked source.txt", "untracked\n");
    fs::remove_file(root.join("crates/onecad-render/src/lib.rs")).unwrap();
    put(
        root,
        "crates/onecad-render/src/replacement.rs",
        "replacement\n",
    );
    fixture.prepare().unwrap();
    let metadata: Value = serde_json::from_slice(
        &fs::read(fixture.payload().join("release-materials.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(metadata["source"]["dirty"], true);
    assert_eq!(metadata["source"]["exactTags"], json!(["fixture-head"]));
    assert_eq!(
        fs::read_to_string(fixture.payload().join("source/worker/src/main.cpp")).unwrap(),
        "unstaged\n"
    );
    assert_eq!(
        fs::read_to_string(fixture.payload().join("source/untracked source.txt")).unwrap(),
        "untracked\n"
    );
    assert!(!fixture
        .payload()
        .join("source/crates/onecad-render/src/lib.rs")
        .exists());
    let patch = fs::read_to_string(fixture.payload().join("source-changes.patch")).unwrap();
    assert!(patch.contains("deleted file mode 100644"));
    assert!(patch.contains("unstaged"));
}

#[test]
fn missing_required_source_and_external_licenses_fail_closed() {
    for missing in [
        "LICENSE-MIT",
        "worker/third_party/planegcs/LICENSE",
        "Cargo.lock",
        "scripts/package-macos.sh",
        "scripts/build-addons.sh",
        "addons/bambu-studio/manifest.json",
        "crates/onecad-extension-api/wit/extension.wit",
        "crates/onecad-extension-sdk/src/bin/onecad-addon-build.rs",
    ] {
        let fixture = fixture();
        fs::remove_file(fixture.path("--source").join(missing)).unwrap();
        assert!(fixture
            .prepare()
            .unwrap_err()
            .to_string()
            .contains("missing regular material"));
        assert!(!fixture.path("--output").exists());
    }
    for missing in ["--occt-archive", "--boost-license"] {
        let fixture = fixture();
        fs::remove_file(fixture.path(missing)).unwrap();
        assert!(fixture
            .prepare()
            .unwrap_err()
            .to_string()
            .contains("missing regular material"));
        assert!(!fixture.path("--output").exists());
    }
    let fixture = fixture();
    fs::remove_file(
        fixture
            .path("--occt-prefix")
            .join("share/doc/opencascade/OCCT_LGPL_EXCEPTION.txt"),
    )
    .unwrap();
    assert!(fixture
        .prepare()
        .unwrap_err()
        .to_string()
        .contains("missing regular material"));
    assert!(!fixture.path("--output").exists());
}

#[test]
fn archive_hash_must_match_policy_even_if_metadata_is_changed() {
    let fixture = fixture();
    fs::write(fixture.path("--occt-archive"), "wrong bytes").unwrap();
    assert!(fixture
        .prepare()
        .unwrap_err()
        .to_string()
        .contains("OCCT archive SHA-256 differs"));
    let path = fixture
        .path("--occt-prefix")
        .join("share/onecad/occt-build.json");
    let mut occt: Occt = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    occt.archive_sha256 = hash(b"wrong bytes");
    save_json(&path, &occt).unwrap();
    assert!(fixture
        .prepare()
        .unwrap_err()
        .to_string()
        .contains("pinned build policy"));
}

#[test]
fn worker_source_extra_and_missing_payload_tampering_rejected() {
    let fixture = fixture();
    let archive = fixture.prepare().unwrap();
    fs::write(fixture.path("--worker"), "another binary").unwrap();
    assert!(verify::archive(&archive, fixture.path("--worker"))
        .unwrap_err()
        .to_string()
        .contains("worker or worker manifest SHA-256"));
    put(
        &fixture.payload(),
        "source/worker/src/main.cpp",
        "tampered source",
    );
    assert!(
        verify::directory(&fixture.payload(), fixture.path("--worker"))
            .unwrap_err()
            .to_string()
            .contains("inventory hash, size or mode mismatch")
    );
    put(&fixture.payload(), "unlisted", "extra");
    assert!(
        verify::directory(&fixture.payload(), fixture.path("--worker"))
            .unwrap_err()
            .to_string()
            .contains("inventory file set differs")
    );
    fs::remove_file(fixture.payload().join("unlisted")).unwrap();
    fs::remove_file(fixture.payload().join("source/worker/src/main.cpp")).unwrap();
    assert!(
        verify::directory(&fixture.payload(), fixture.path("--worker"))
            .unwrap_err()
            .to_string()
            .contains("inventory file set differs")
    );
}

#[test]
fn output_must_be_external_and_new() {
    let mut fixture = fixture();
    let output = fixture.options["--output"].clone();
    fixture
        .options
        .insert("--output".into(), fixture.path("--source").join("output"));
    assert!(fixture
        .prepare()
        .unwrap_err()
        .to_string()
        .contains("outside source repository"));
    fixture.options.insert("--output".into(), output.clone());
    fs::create_dir(&output).unwrap();
    assert!(fixture
        .prepare()
        .unwrap_err()
        .to_string()
        .contains("refusing to overwrite"));
}

#[cfg(unix)]
#[test]
fn source_links_and_archive_links_rejected() {
    use std::os::unix::fs::symlink;
    let fixture = fixture();
    symlink(
        fixture.path("--worker"),
        fixture.path("--source").join("linked-source"),
    )
    .unwrap();
    assert!(fixture
        .prepare()
        .unwrap_err()
        .to_string()
        .contains("unsupported source type"));
    fs::remove_file(fixture.path("--source").join("linked-source")).unwrap();
    symlink(
        "/no/such/onecad-material",
        fixture.path("--source").join("dangling-source"),
    )
    .unwrap();
    assert!(fixture
        .prepare()
        .unwrap_err()
        .to_string()
        .contains("unsupported source type"));
    fs::remove_file(fixture.path("--source").join("dangling-source")).unwrap();
    fixture.prepare().unwrap();
    symlink("/etc/passwd", fixture.payload().join("link")).unwrap();
    let archive = fixture.path("--output").join("links.tar.gz");
    execute(
        Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(fixture.path("--output"))
            .arg("materials"),
    )
    .unwrap();
    assert!(verify::archive(&archive, fixture.path("--worker"))
        .unwrap_err()
        .to_string()
        .contains("unsupported links or special files"));
}

#[cfg(unix)]
#[test]
fn executable_source_modes_preserved_and_checked() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = fixture();
    fs::set_permissions(
        fixture.path("--source").join("scripts/package-macos.sh"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    fixture.prepare().unwrap();
    let script = fixture.payload().join("source/scripts/package-macos.sh");
    assert!(executable(&script).unwrap());
    fs::set_permissions(script, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(
        verify::directory(&fixture.payload(), fixture.path("--worker"))
            .unwrap_err()
            .to_string()
            .contains("inventory hash, size or mode mismatch")
    );
}

#[test]
fn metadata_versions_and_worker_manifest_fail_closed() {
    let fixture = fixture();
    put(
        fixture.path("--occt-prefix"),
        "share/onecad/occt-build.cmake",
        "set(ONECAD_OCCT_BUILT_ID \"wrong\")\n",
    );
    assert!(fixture
        .prepare()
        .unwrap_err()
        .to_string()
        .contains("OCCT CMake metadata differs"));
    let fixture = self::fixture();
    put(
        fixture.path("--boost-prefix"),
        "include/boost/version.hpp",
        "// no macro\n",
    );
    assert!(fixture
        .prepare()
        .unwrap_err()
        .to_string()
        .contains("missing Eigen or Boost version evidence"));
    fs::write(fixture.path("--worker"), "different bytes").unwrap();
    assert!(fixture
        .prepare()
        .unwrap_err()
        .to_string()
        .contains("worker SHA-256 differs from worker manifest"));
}

#[test]
fn live_policy_pin_and_unsafe_paths_checked() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    assert_eq!(
        occt_pin(root).unwrap().0,
        "0d6913eae4bcc09a3653ceced6dda1aec11c35a1513d4c06762c9b002092c68a"
    );
    for path in [
        "",
        "/absolute",
        "../escape",
        "a/../b",
        "a//b",
        "./a",
        "a\\b",
        "a\n",
    ] {
        assert!(safe_path(path).is_err(), "{path:?}");
    }
}

#[test]
fn malformed_hello_frames_fail_closed_and_hash_tracks_final_bytes() {
    let temporary = tempfile::tempdir().unwrap();
    let frame = temporary.path().join("frame");
    let binary = temporary.path().join("binary");
    let manifest = temporary.path().join("manifest");
    fs::write(&binary, b"worker").unwrap();
    let args = [
        frame.to_str().unwrap().to_owned(),
        binary.to_str().unwrap().to_owned(),
        manifest.to_str().unwrap().to_owned(),
    ];
    for bytes in [
        b"not a frame".to_vec(),
        [b"OCW1".as_slice(), &[0; 8]].concat(),
    ] {
        fs::write(&frame, bytes).unwrap();
        assert!(super::run("worker-manifest", &args).is_err());
    }
    let hello = serde_json::to_vec(&json!({"t":"hello","seq":0,"result":{"protocolVersion":1,"workerVersion":"test","quantizationVersion":1,"solverPolicyVersion":4,"occt":{"version":"8.0.1","fingerprint":"test"}}})).unwrap();
    let bytes = [
        b"OCW1".as_slice(),
        &(hello.len() as u32).to_le_bytes(),
        &0u32.to_le_bytes(),
        &hello,
    ]
    .concat();
    fs::write(&frame, bytes).unwrap();
    super::run("worker-manifest", &args).unwrap();
    let value: Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    assert_eq!(value["binarySha256"], hash(b"worker"));
    assert_eq!(value["solverPolicyVersion"], 4);
    fs::write(&binary, b"signed worker").unwrap();
    super::run("rehash-worker-manifest", &args[1..]).unwrap();
    let value: Value = serde_json::from_slice(&fs::read(manifest).unwrap()).unwrap();
    assert_eq!(value["binarySha256"], hash(b"signed worker"));
    assert_eq!(value["solverPolicyVersion"], 4);
}
