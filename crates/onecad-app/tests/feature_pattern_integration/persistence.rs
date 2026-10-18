use super::super::*;

pub(crate) fn open_runtime(worker: &WorkerManager, path: &Path) -> DocumentRuntime {
    let geometry: Arc<dyn GeometryEngine> = Arc::new(worker.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(worker.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(worker.clone());
    DocumentRuntime::open(path, geometry, meshes, solver).expect("open saved pattern")
}

pub(crate) fn save_meta() -> SaveMeta {
    SaveMeta {
        app_version: "feature-pattern-integration".into(),
        occt_fingerprint: None,
        created: "2026-09-11T00:00:00Z".into(),
        modified: "2026-09-11T00:00:00Z".into(),
    }
}

pub(crate) fn document_json(path: &Path) -> serde_json::Value {
    let file = std::fs::File::open(path).expect("open container");
    let mut archive = zip::ZipArchive::new(file).expect("container zip");
    let mut entry = archive.by_name("document.json").expect("document.json");
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes).expect("read document.json");
    serde_json::from_slice(&bytes).expect("parse document.json")
}
