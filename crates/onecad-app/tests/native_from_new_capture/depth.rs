//! Actual GUI saves for the separate depth/history/reopen journey, including holes.
use super::*;

const ROOT: &str = "ONECAD_FROM_NEW_DEPTH_CAPTURE_ROOT";
const CASE: &str = "ONECAD_FROM_NEW_DEPTH_CAPTURE_CASE";
const TEST: &str = "native_from_new_depth_capture_semantics";
const CHECKPOINTS: [(&str, f64, f64); 7] = [
    ("committed-6.onecad", 80.0, 6.0),
    ("edited-8.onecad", 80.0, 8.0),
    ("undo-6.onecad", 80.0, 6.0),
    ("redo-8.onecad", 80.0, 8.0),
    ("width90-8.onecad", 90.0, 8.0),
    ("reopened90-8.onecad", 90.0, 8.0),
    ("further-edited90-6.onecad", 90.0, 6.0),
];

pub fn run() {
    host::required_worker();
    assert_eq!(std::env::var("ONECAD_REQUIRE_WORKER").as_deref(), Ok("1"));
    let root = PathBuf::from(
        std::env::var_os(ROOT).expect("ONECAD_FROM_NEW_DEPTH_CAPTURE_ROOT is required"),
    )
    .canonicalize()
    .unwrap();
    if let Ok(name) = std::env::var(CASE) {
        let (_, width, depth) = CHECKPOINTS
            .iter()
            .find(|(capture, _, _)| *capture == name)
            .unwrap();
        let service =
            PathBuf::from(std::env::var_os(SERVICE_ROOT).expect("private service root required"));
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(qualify_depth(&root, &service, &name, *width, *depth));
        return;
    }
    let before = CHECKPOINTS.map(|(name, _, _)| digest(&root.join(name)));
    let services = tempfile::Builder::new()
        .prefix("onecad-depth-captures-")
        .tempdir()
        .unwrap();
    for (name, _, _) in CHECKPOINTS {
        run_child(&root, &services.path().join(name), name);
    }
    for ((name, _, _), expected) in CHECKPOINTS.into_iter().zip(before) {
        assert_eq!(
            digest(&root.join(name)),
            expected,
            "immutable GUI capture: {name}"
        );
    }
}

fn run_child(root: &Path, service: &Path, name: &str) {
    for directory in ["temporary", "logs", "data/library"] {
        std::fs::create_dir_all(service.join(directory)).unwrap();
    }
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", TEST, "--ignored", "--nocapture"])
        .env(ROOT, root)
        .env(CASE, name)
        .env(SERVICE_ROOT, service)
        .env("TMPDIR", service.join("temporary"))
        .env("ONECAD_LOG_DIR", service.join("logs"))
        .env("ONECAD_LIBRARY_ROOT", service.join("data/library"))
        .status()
        .unwrap();
    assert!(status.success(), "depth capture failed: {name}: {status}");
}

async fn qualify_depth(root: &Path, service: &Path, name: &str, width: f64, depth: f64) {
    let baseline = read_document(&root.join(CHECKPOINTS[0].0));
    let source = root.join(name);
    let saved = read_document(&source);
    semantics::saved_at_depth(&baseline, &saved, width, depth);
    let (harness, projection) = opened(service, &source).await;
    assert_eq!(projection.document_id, baseline.id.to_string());
    assert_eq!(
        (
            projection.applied_ops,
            projection.total_ops,
            projection.features.len()
        ),
        (2, 2, 2)
    );
    assert!(!projection.dirty);
    let (record, params) = extrude(&saved);
    assert_eq!(
        api::get_operation_record(&harness.state, record.record_id)
            .await
            .unwrap()
            .op,
        record.op
    );
    let sketch = qualified_sketch(&harness, &saved, params.profile.as_ref().unwrap(), width).await;
    assert_eq!(projection.bodies.len(), 1);
    let body = projection.bodies.keys().next().unwrap();
    assert_eq!(body, &baseline.bodies.bodies()[0].id.to_string());
    geometry::solid_at_depth(&harness, &projection, &sketch, body, width, depth).await;
    let dof = qualified_solve(&harness, params.profile.as_ref().unwrap().sketch).await;
    eprintln!(
        "{}",
        serde_json::json!({"capture":name,"widthMm":width,"depthMm":depth,
        "dof":dof,"constraints":sketch.constraints().len(),"solids":1,"faces":8,"result":"pass"})
    );
    harness.close().await;
}
