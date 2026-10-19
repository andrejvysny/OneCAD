//! Semantic qualification of immutable documents saved by actual native GUI input.
//! This test never generates captures and does not establish physical input acceptance.
#[allow(dead_code)]
mod native_plate_support;

use native_plate_support::{geometry::assert_mesh, Harness};
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
    ("committed-6.onecad", 6.0),
    ("edited-8.onecad", 8.0),
    ("undo-6.onecad", 6.0),
    ("redo-8.onecad", 8.0),
    ("reopened-8.onecad", 8.0),
];
const CHILD: &str = "ONECAD_NATIVE_CAPTURE_CASE";
const SERVICE_ROOT: &str = "ONECAD_NATIVE_CAPTURE_SERVICE_ROOT";

#[test]
#[ignore = "requires actual GUI captures in ONECAD_NATIVE_CAPTURE_ROOT and freshly staged production worker"]
fn native_gui_capture_semantics() {
    native_plate_support::required_worker();
    assert_eq!(std::env::var("ONECAD_REQUIRE_WORKER").as_deref(), Ok("1"));
    let root = PathBuf::from(
        std::env::var_os("ONECAD_NATIVE_CAPTURE_ROOT")
            .expect("ONECAD_NATIVE_CAPTURE_ROOT is required"),
    )
    .canonicalize()
    .unwrap();
    if let Ok(name) = std::env::var(CHILD) {
        let depth = CAPTURES
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
            .block_on(qualify(&root, &service, &name, depth));
        return;
    }
    let before = CAPTURES.map(|(name, _)| digest(&root.join(name)));
    let services = tempfile::Builder::new()
        .prefix("onecad-native-capture-acceptance-")
        .tempdir()
        .unwrap();
    for (name, _) in CAPTURES {
        run_capture(&root, &services.path().join(name), name);
    }
    for ((name, _), expected) in CAPTURES.into_iter().zip(before) {
        assert_eq!(
            digest(&root.join(name)),
            expected,
            "capture must remain immutable: {name}"
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
            "native_gui_capture_semantics",
            "--ignored",
            "--nocapture",
        ])
        .env("ONECAD_NATIVE_CAPTURE_ROOT", captures)
        .env(CHILD, name)
        .env(SERVICE_ROOT, service)
        .env("TMPDIR", service.join("temporary"))
        .env("ONECAD_LOG_DIR", service.join("logs"))
        .env("ONECAD_LIBRARY_ROOT", service.join("data/library"))
        .status()
        .unwrap();
    assert!(status.success(), "native capture failed: {name}: {status}");
}

async fn qualify(root: &Path, service: &Path, name: &str, depth: f64) {
    let baseline = read_document(&root.join(CAPTURES[0].0));
    let source = root.join(name);
    let saved = read_document(&source);
    assert_saved_semantics(&baseline, &saved, depth);
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
    assert_eq!(projection.document_id, baseline.id.to_string());
    assert_eq!(
        (
            projection.applied_ops,
            projection.total_ops,
            projection.features.len()
        ),
        (2, 2, 2)
    );
    assert_eq!(projection.bodies.len(), 1);
    assert!(
        !projection.dirty,
        "opening/regenerating a capture must remain clean"
    );
    let (record, params) = extrude(&saved);
    let live = api::get_operation_record(&harness.state, record.record_id)
        .await
        .unwrap();
    assert_eq!(live.op, record.op);
    let profile = params.profile.as_ref().unwrap();
    let sketch = api::get_sketch_snapshot(&harness.state, profile.sketch)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&sketch).unwrap(),
        serde_json::to_value(&baseline.sketches[&profile.sketch]).unwrap()
    );
    let regions = api::get_sketch_regions(&harness.state, profile.sketch.to_string())
        .await
        .unwrap();
    assert_eq!(
        Some(regions.region_identity_version),
        profile.region_identity_version
    );
    assert_eq!(regions.regions.len(), 1);
    assert_eq!(regions.regions[0].region_id, profile.region.to_string());
    let body = projection.bodies.keys().next().unwrap();
    assert_eq!(body, &baseline.bodies.bodies()[0].id.to_string());
    let volume = assert_geometry(&harness, &projection, body, depth).await;
    eprintln!(
        "{}",
        serde_json::json!({"capture": name, "documentId": projection.document_id,
        "extrudeId": record.record_id.to_string(), "bodyId": body, "profile": profile, "regionAnchor": profile.region_anchor,
        "depthMm": depth, "appliedOps": projection.applied_ops, "dirty": projection.dirty,
        "sketchEntities": sketch.entities().len(), "sketchConstraints": sketch.constraints().len(),
        "solids": 1, "faces": 6, "volumeMm3": volume, "mesh": "checked-closed-connected", "result": "pass"})
    );
    harness.close().await;
}

fn assert_saved_semantics(baseline: &Document, saved: &Document, depth: f64) {
    assert_eq!(saved.id, baseline.id);
    assert_eq!(
        (saved.timeline.cursor(), saved.timeline.records().len()),
        (2, 2)
    );
    assert_eq!(saved.sketches.len(), 1);
    assert_eq!(saved.bodies.len(), 1);
    assert_eq!(saved.bodies.bodies()[0].id, baseline.bodies.bodies()[0].id);
    assert_eq!(
        saved
            .timeline
            .records()
            .iter()
            .map(|record| record.record_id)
            .collect::<Vec<_>>(),
        baseline
            .timeline
            .records()
            .iter()
            .map(|record| record.record_id)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        serde_json::to_value(&saved.sketches).unwrap(),
        serde_json::to_value(&baseline.sketches).unwrap()
    );
    let (expected_record, expected) = extrude(baseline);
    let (record, actual) = extrude(saved);
    assert_eq!(record.record_id, expected_record.record_id);
    assert_eq!(record.step_index, 1);
    assert!(!record.suppressed);
    assert_eq!(actual.distance.value, depth);
    assert_eq!(
        actual.profile, expected.profile,
        "sketch/region/version/anchor must persist exactly"
    );
    assert!(actual
        .profile
        .as_ref()
        .unwrap()
        .region_identity_version
        .is_some());
    let mut normalized = actual.clone();
    normalized.distance = expected.distance.clone();
    assert_eq!(&normalized, expected, "only extrusion depth may change");
    assert_eq!(record.inputs, record.op.derive_inputs());
    assert_eq!(record.outputs, expected_record.outputs);
}

async fn assert_geometry(
    harness: &Harness,
    projection: &onecad_app::dto::DocumentProjection,
    body: &str,
    depth: f64,
) -> f64 {
    let wire = onecad_app::worker::wire::body_id_wire(body.parse().unwrap());
    let topology = api::query_body_topology(&harness.state, wire.clone())
        .await
        .unwrap();
    assert_eq!((topology.solid_count, topology.face_count), (1, 6));
    let mass = api::query_mass_properties(&harness.state, wire, None)
        .await
        .unwrap();
    near(mass.volume, 80.0 * 50.0 * depth);
    near(
        mass.surface_area,
        2.0 * (80.0 * 50.0 + (80.0 + 50.0) * depth),
    );
    for (actual, expected) in mass.centroid.into_iter().zip([-25.0, 40.0, depth / 2.0]) {
        near(actual, expected);
    }
    let mesh = api::get_mesh(
        &harness.state,
        body.into(),
        "fine".into(),
        None,
        Some(projection.runtime_session.clone()),
    )
    .await
    .unwrap();
    assert_mesh(&mesh, depth);
    mass.volume
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
        .unwrap_or_else(|error| panic!("cannot open native capture {}: {error}", path.display()))
        .document()
        .clone()
}
fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() <= 1e-6, "{actual} != {expected}");
}
fn digest(path: &Path) -> [u8; 32] {
    let mut source = std::fs::File::open(path)
        .unwrap_or_else(|error| panic!("cannot read native capture {}: {error}", path.display()));
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
