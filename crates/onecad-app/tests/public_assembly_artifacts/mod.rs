use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::{Path, PathBuf},
};

pub fn retain(path: &Path, label: &str, test_source: &[u8]) {
    let Some(root) = std::env::var_os("ONECAD_PUBLIC_ASSEMBLY_CAPTURE_DIR") else {
        return;
    };
    let directory = PathBuf::from(root).join(label);
    std::fs::create_dir_all(directory.parent().unwrap()).unwrap();
    std::fs::create_dir(&directory).expect("assembly capture must not overwrite prior evidence");
    let bytes = std::fs::read(path).unwrap();
    std::fs::write(directory.join("assembly.onecad"), &bytes).unwrap();
    let worker =
        onecad_app::worker::resolve_worker_path().expect("capture requires resolved worker");
    let executable = std::env::current_exe().unwrap();
    let manifest = serde_json::json!({
        "schemaVersion": 1,
        "classification": "successful shared-service save and shutdown; cold-reopen validation continues after capture",
        "geometry": "imported source parts; not native feature-history qualification",
        "artifact": "assembly.onecad",
        "artifactSha256": digest(&bytes),
        "topLevelTestSourceSha256": digest(test_source),
        "testExecutableSha256": digest_file(&executable),
        "resolvedWorkerPathSha256AtCapture": digest_file(&worker),
        "sourceCommitFromEnvironment": std::env::var("ONECAD_SOURCE_COMMIT").ok(),
        "limits": "Exact OS, toolchain, independent oracle and executed native UI evidence require a companion record."
    });
    std::fs::write(
        directory.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
}

fn digest_file(path: &Path) -> String {
    let mut file = std::fs::File::open(path).unwrap();
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let count = file.read(&mut buffer).unwrap();
        if count == 0 {
            return format!("{:x}", hash.finalize());
        }
        hash.update(&buffer[..count]);
    }
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
