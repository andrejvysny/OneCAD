//! Read-only semantic checks for five immutable From-New native GUI captures.
//! Physical input/history evidence remains separate from these service checks.
#[path = "native_from_new_capture/geometry.rs"]
mod geometry;
#[allow(dead_code)]
#[path = "native_plate_support/host.rs"]
mod host;
#[path = "native_from_new_capture/semantics.rs"]
mod semantics;

use host::Harness;
use onecad_app::api;
use onecad_core::{
    document::{
        record::{ExtrudeParams, KnownOperation, Operation, OperationRecord},
        Document,
    },
    io::container::ContainerReader,
};
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

const CAPTURES: [(&str, f64); 5] = [
    ("plate80-4.onecad", 80.0),
    ("plate90-4.onecad", 90.0),
    ("plate-undo80.onecad", 80.0),
    ("plate-redo90.onecad", 90.0),
    ("plate-reopened90.onecad", 90.0),
];
const CHILD: &str = "ONECAD_FROM_NEW_CAPTURE_CASE";
const SERVICE_ROOT: &str = "ONECAD_FROM_NEW_CAPTURE_SERVICE_ROOT";

#[test]
#[ignore = "requires actual GUI captures in ONECAD_FROM_NEW_CAPTURE_ROOT and staged production worker"]
fn native_from_new_capture_semantics() {
    host::required_worker();
    assert_eq!(std::env::var("ONECAD_REQUIRE_WORKER").as_deref(), Ok("1"));
    let root = PathBuf::from(
        std::env::var_os("ONECAD_FROM_NEW_CAPTURE_ROOT")
            .expect("ONECAD_FROM_NEW_CAPTURE_ROOT is required"),
    )
    .canonicalize()
    .unwrap();
    if let Ok(name) = std::env::var(CHILD) {
        let width = CAPTURES
            .iter()
            .find(|(capture, _)| *capture == name)
            .expect("known capture")
            .1;
        let service =
            PathBuf::from(std::env::var_os(SERVICE_ROOT).expect("private service root required"));
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(qualify(&root, &service, &name, width));
        return;
    }
    let before = CAPTURES.map(|(name, _)| digest(&root.join(name)));
    let services = tempfile::Builder::new()
        .prefix("onecad-from-new-capture-")
        .tempdir()
        .unwrap();
    for (name, _) in CAPTURES {
        run_capture(&root, &services.path().join(name), name);
    }
    for ((name, _), expected) in CAPTURES.into_iter().zip(before) {
        assert_eq!(
            digest(&root.join(name)),
            expected,
            "immutable capture: {name}"
        );
    }
}

fn run_capture(captures: &Path, service: &Path, name: &str) {
    for directory in ["temporary", "logs", "data/library"] {
        std::fs::create_dir_all(service.join(directory)).unwrap();
    }
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "native_from_new_capture_semantics",
            "--ignored",
            "--nocapture",
        ])
        .env("ONECAD_FROM_NEW_CAPTURE_ROOT", captures)
        .env(CHILD, name)
        .env(SERVICE_ROOT, service)
        .env("TMPDIR", service.join("temporary"))
        .env("ONECAD_LOG_DIR", service.join("logs"))
        .env("ONECAD_LIBRARY_ROOT", service.join("data/library"))
        .status()
        .unwrap();
    assert!(
        status.success(),
        "From-New capture failed: {name}: {status}"
    );
}

async fn qualify(root: &Path, service: &Path, name: &str, width: f64) {
    let baseline = read_document(&root.join(CAPTURES[0].0));
    let source = root.join(name);
    let saved = read_document(&source);
    semantics::saved(&baseline, &saved, width);
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
    assert!(!projection.dirty, "fresh-open regeneration stays clean");
    let (record, params) = extrude(&saved);
    assert_eq!(
        api::get_operation_record(&harness.state, record.record_id)
            .await
            .unwrap()
            .op,
        record.op
    );
    let profile = params.profile.as_ref().unwrap();
    let sketch = qualified_sketch(&harness, &saved, profile, width).await;
    assert_eq!(projection.bodies.len(), 1);
    let body = projection.bodies.keys().next().unwrap();
    assert_eq!(body, &baseline.bodies.bodies()[0].id.to_string());
    geometry::solid(&harness, &projection, &sketch, body, width).await;
    let dof = qualified_solve(&harness, profile.sketch).await;
    eprintln!(
        "{}",
        serde_json::json!({"capture": name, "documentId": projection.document_id,
        "sketchId": sketch.id, "extrudeId": record.record_id, "bodyId": body, "widthMm": width,
        "depthMm": 4, "dof": dof, "entities": sketch.entities().len(),
        "constraints": sketch.constraints().len(), "solids": 1, "faces": 8,
        "volumeMm3": 4.0 * (width * 50.0 - 18.0 * std::f64::consts::PI), "result": "pass"})
    );
    harness.close().await;
}

fn extrude(document: &Document) -> (&OperationRecord, &ExtrudeParams) {
    let records: Vec<_> = document
        .timeline
        .records()
        .iter()
        .filter_map(|record| match &record.op {
            Operation::Known(KnownOperation::Extrude(params)) => Some((record, params)),
            _ => None,
        })
        .collect();
    assert_eq!(records.len(), 1, "exactly one authored extrusion");
    records[0]
}

fn read_document(path: &Path) -> Document {
    ContainerReader::open(path)
        .unwrap_or_else(|error| panic!("cannot open capture {}: {error}", path.display()))
        .document()
        .clone()
}

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() <= 1e-6, "{actual} != {expected}");
}

fn digest(path: &Path) -> [u8; 32] {
    let mut source = std::fs::File::open(path)
        .unwrap_or_else(|error| panic!("cannot read capture {}: {error}", path.display()));
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 65536];
    loop {
        let length = source.read(&mut buffer).unwrap();
        if length == 0 {
            break;
        }
        hash.update(&buffer[..length]);
    }
    hash.finalize().into()
}

async fn qualified_solve(harness: &Harness, sketch: onecad_core::ids::SketchId) -> u32 {
    // Fresh-open regeneration does not populate the editing solver cache. Run the
    // solver only in this disposable runtime, after qualifying published geometry.
    let solve = api::enter_sketch(
        &harness.state,
        harness.host.clone(),
        sketch.to_string(),
        Some(false),
    )
    .await
    .unwrap();
    assert!(
        solve.solve_current && solve.conflicting.is_empty(),
        "{solve:?}"
    );
    assert_eq!(solve.dof, 0, "fully constrained From-New plate");
    assert_eq!(
        solve.status,
        onecad_app::dto::SketchSolveStatus::FullyConstrained
    );
    api::cancel_sketch(
        &harness.state,
        harness.host.clone(),
        sketch.to_string(),
        Some(false),
    )
    .await
    .unwrap();
    solve.dof
}

async fn opened(service: &Path, source: &Path) -> (Harness, onecad_app::dto::DocumentProjection) {
    let harness = Harness::new(service).await;
    api::open_document(
        &harness.state,
        harness.host.clone(),
        source.to_string_lossy().into_owned(),
        None,
    )
    .await
    .unwrap();
    assert!(
        harness
            .state
            .readiness()
            .wait_ready(Duration::from_secs(15))
            .await
    );
    let projection = harness.settled(&harness.projection().await).await;
    (harness, projection)
}

async fn qualified_sketch(
    harness: &Harness,
    saved: &Document,
    profile: &onecad_core::document::refs::SketchRegionRef,
    width: f64,
) -> onecad_core::sketch::Sketch {
    let sketch = api::get_sketch_snapshot(&harness.state, profile.sketch)
        .await
        .unwrap();
    semantics::sketch(&sketch, width);
    assert_eq!(
        serde_json::to_value(&sketch).unwrap(),
        serde_json::to_value(&saved.sketches[&profile.sketch]).unwrap()
    );
    let regions = api::get_sketch_regions(&harness.state, profile.sketch.to_string())
        .await
        .unwrap();
    semantics::profile(&regions, &sketch, profile);
    sketch
}
