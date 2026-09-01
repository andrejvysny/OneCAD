//! Vendor-STEP → component-library INGEST (Component Library WP-C2), against the
//! REAL C++ OCCT worker.
//!
//! Every fixture is built in-process, exported to STEP, and then ingested — so the
//! numbers below are analytic, not recorded, and the whole round trip (build →
//! `ExportStep` → `InspectStep` → `ImportStep` → keep-list → fuse →
//! `ExtractPrismProfile` / `ExportGeometry` → package → place) is exercised end to
//! end.
//!
//! * `a_stick_ingests_as_a_profile_and_places_at_a_length_the_vendor_never_shipped`
//!   — the `profile` leg: a 500 mm 20×20 + Ø5 stick becomes a length-parametric
//!   package, placed at 120 mm for the exact analytic volume.
//! * `two_disjoint_boxes_refuse_with_the_disjoint_marker` — the `embedded` leg's
//!   refusal: a keep-list whose solids do not touch is a REFUSAL naming the solids
//!   it kept, never a component made of two pieces.
//! * `two_touching_boxes_fuse_into_one_solid` — the same leg's success: the fused
//!   volume is the exact sum.
//! * `the_tracked_vendor_recipe_ingests` — replays `STEP/ingest.toml` when the
//!   (gitignored) vendor files are present. This is the ONE sanctioned skip in
//!   this file, and it prints why.
//!
//! Gated on `ONECAD_WORKER_PATH` (else the dev-tree fallback);
//! `ONECAD_REQUIRE_WORKER=1` turns a missing binary into a hard failure.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use onecad_core::document::record::{
    BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, Operation, OperationRecord, PlaneKind,
    SketchPlaneRef,
};
use onecad_core::document::variables::Scalar;
use onecad_core::edit::EditCommand;
use onecad_core::ids::{BodyId, RecordId, SketchId};
use onecad_core::math::Vec3;
use onecad_core::regen::{CancelToken, GeometryEngine, ModelSnapshot, Outcome, RegenRequest};

use onecad_lib::document_runtime::{DocumentRuntime, RegenReport};
use onecad_lib::dto::IngestStatusDto;
use onecad_lib::export::GeometryExporter;
use onecad_lib::library_ingest::{
    ingest_components_at, ingest_supervisor_config, parse_recipe, IngestDefaults, IngestKind,
    IngestPart, IngestPlan, KeepMode, KeepSpec, ProfileSpec, DISJOINT_REFUSAL,
};
use onecad_lib::worker::{
    resolve_worker_path, ElementQuery, MeshProvider, SolverEngine, WorkerManager,
};

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors component_ops.rs)
// ─────────────────────────────────────────────────────────────────────────────

fn real_worker() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("ONECAD_WORKER_PATH") {
        let path = PathBuf::from(&p);
        assert!(
            path.is_file(),
            "ONECAD_WORKER_PATH={p:?} is set but no worker binary exists there \
             (misconfiguration — refusing to skip as green)"
        );
        return Some(path);
    }
    if let Some(path) = resolve_worker_path() {
        return Some(path);
    }
    assert!(
        std::env::var("ONECAD_REQUIRE_WORKER").as_deref() != Ok("1"),
        "ONECAD_REQUIRE_WORKER=1 but no worker binary resolved (CI must hard-fail here)"
    );
    None
}

/// The ingest's OWN supervision policy, not production's — a 67-solid vendor
/// assembly keeps the worker busy well past SCHEMA §8's interactive ping budget,
/// and spawning it under `SupervisorConfig::production` SIGKILLs it mid-import.
async fn spawn_worker(bin: PathBuf) -> WorkerManager {
    let wm = WorkerManager::spawn(ingest_supervisor_config(bin));
    assert!(
        wm.wait_ready(Duration::from_secs(10)).await,
        "real worker must connect + handshake + OpenSession"
    );
    wm
}

fn runtime_over(wm: &WorkerManager) -> DocumentRuntime {
    let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
    DocumentRuntime::new_blank(engine, meshes, solver)
}

fn add_op(rt: &mut DocumentRuntime, record: OperationRecord) {
    rt.apply(EditCommand::AddOperation {
        record,
        at_cursor: true,
    })
    .expect("AddOperation");
}

async fn regen_all(rt: &mut DocumentRuntime) -> RegenReport {
    rt.run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await
}

fn published<'a>(report: &'a RegenReport, what: &str) -> &'a Arc<ModelSnapshot> {
    match &report.outcome {
        Outcome::Published(s) => s,
        other => panic!("{what}: expected Published, got {other:?}"),
    }
}

/// Exact kernel volume of a body (`QueryMassProperties`, SCHEMA §7.5) — read from
/// the BRep, never re-derived from the tessellation.
async fn exact_volume(wm: &WorkerManager, body: BodyId) -> f64 {
    ElementQuery::query_mass_properties(wm, body, "b".into())
        .await
        .expect("QueryMassProperties")
        .volume
}

// ─────────────────────────────────────────────────────────────────────────────
// Fixture geometry (the stock profile is verbatim from component_ops.rs, so the
// two files agree on `400 − π·2.5²` by construction)
// ─────────────────────────────────────────────────────────────────────────────

/// `400 − π·2.5²` — the 20×20 + Ø5 profile's exact area.
const PROFILE_AREA_MM2: f64 = 400.0 - std::f64::consts::PI * 2.5 * 2.5;
const STOCK_LENGTH_MM: f64 = 500.0;
const PLACED_LENGTH_MM: f64 = 120.0;
/// `PROFILE_AREA_MM2 × 120`, spelled out so a drift in either factor is loud.
const PLACED_VOLUME_MM3: f64 = 45_643.805_509_807_65;

fn xy_plane_ref() -> SketchPlaneRef {
    SketchPlaneRef {
        kind: PlaneKind::Xy,
        origin: Vec3::new_unchecked(0.0, 0.0, 0.0),
        x_axis: Vec3::new_unchecked(0.0, 1.0, 0.0),
        y_axis: Vec3::new_unchecked(-1.0, 0.0, 0.0),
        normal: Vec3::new_unchecked(0.0, 0.0, 1.0),
        extra: Default::default(),
    }
}

/// A closed 20×20 square with a concentric Ø5 hole.
fn stock_profile_sketch(sid: SketchId, base: u128) -> onecad_core::sketch::Sketch {
    use onecad_core::ids::EntityId;
    use onecad_core::math::Vec2;
    use onecad_core::sketch::{Sketch, SketchEntity, WorldPlane};

    let e = |n: u128| EntityId(Uuid::from_u128(base + n));
    let mut sk = Sketch::on_world_plane(sid, "Stock profile", WorldPlane::XY);
    let point = |sk: &mut Sketch, id: EntityId, x: f64, y: f64| {
        sk.add_entity(SketchEntity::point(
            id,
            Vec2::new_unchecked(x, y),
            false,
            false,
        ))
        .expect("point");
    };
    point(&mut sk, e(0), 0.0, 0.0);
    point(&mut sk, e(1), 20.0, 0.0);
    point(&mut sk, e(2), 20.0, 20.0);
    point(&mut sk, e(3), 0.0, 20.0);
    for (l, a, b) in [(0x10, 0, 1), (0x11, 1, 2), (0x12, 2, 3), (0x13, 3, 0)] {
        sk.add_entity(SketchEntity::line(e(l), e(a), e(b), false))
            .expect("line");
    }
    point(&mut sk, e(0x20), 10.0, 10.0);
    sk.add_entity(SketchEntity::circle(e(0x21), e(0x20), 2.5, false).expect("finite radius"))
        .expect("circle");
    sk
}

/// An axis-aligned rectangle `[x0, x0+w] × [y0, y0+h]`.
fn rect_sketch(
    sid: SketchId,
    base: u128,
    x0: f64,
    y0: f64,
    w: f64,
    h: f64,
) -> onecad_core::sketch::Sketch {
    use onecad_core::ids::EntityId;
    use onecad_core::math::Vec2;
    use onecad_core::sketch::{Sketch, SketchEntity, WorldPlane};

    let e = |n: u128| EntityId(Uuid::from_u128(base + n));
    let mut sk = Sketch::on_world_plane(sid, "Rect", WorldPlane::XY);
    for (i, (x, y)) in [(x0, y0), (x0 + w, y0), (x0 + w, y0 + h), (x0, y0 + h)]
        .into_iter()
        .enumerate()
    {
        sk.add_entity(SketchEntity::point(
            e(i as u128),
            Vec2::new_unchecked(x, y),
            false,
            false,
        ))
        .expect("point");
    }
    for (l, a, b) in [(0x10, 0, 1), (0x11, 1, 2), (0x12, 2, 3), (0x13, 3, 0)] {
        sk.add_entity(SketchEntity::line(e(l), e(a), e(b), false))
            .expect("line");
    }
    sk
}

fn sketch_record(rec: u128, sk: &onecad_core::sketch::Sketch) -> OperationRecord {
    let (_plane, entities, constraints) = onecad_lib::worker::wire::sketch_wire(sk);
    let op = Operation::Known(KnownOperation::Sketch(
        onecad_core::document::record::SketchOpParams {
            sketch: sk.id,
            plane: xy_plane_ref(),
            entities: entities.as_array().cloned().unwrap_or_default(),
            constraints: constraints.as_array().cloned().unwrap_or_default(),
            host_face: None,
            extra: Default::default(),
        },
    ));
    OperationRecord::new(RecordId(Uuid::from_u128(rec)), 0, "Sketch", op)
}

fn extrude_record(rec: u128, sketch: SketchId, distance: f64) -> OperationRecord {
    let op = Operation::Known(KnownOperation::Extrude(ExtrudeParams {
        profile: Some(onecad_core::document::refs::SketchRegionRef {
            sketch,
            region: onecad_core::ids::RegionId::new(""), // first-region fallback
            region_identity_version: None,
            extra: Default::default(),
        }),
        distance: Scalar::new(distance),
        draft_angle_deg: Scalar::new(0.0),
        mode: ExtrudeMode::Blind,
        boolean_mode: BooleanMode::NewBody,
        target_body: None,
        target_face: None,
        two_directions: false,
        mode2: ExtrudeMode::Blind,
        distance2: Scalar::new(0.0),
        target_face2: None,
        extra: Default::default(),
    }));
    OperationRecord::new(RecordId(Uuid::from_u128(rec)), 0, "Extrude", op)
}

/// Builds a document from `records` on `wm`, regens it, and writes every head body
/// to `path` as STEP — the vendor file the ingest then reads.
async fn export_fixture(
    wm: &WorkerManager,
    path: &Path,
    records: Vec<OperationRecord>,
    want_bodies: usize,
) -> Vec<f64> {
    let mut rt = runtime_over(wm);
    for r in records {
        add_op(&mut rt, r);
    }
    let report = regen_all(&mut rt).await;
    let snap = published(&report, "fixture build");
    assert_eq!(
        snap.bodies.len(),
        want_bodies,
        "the fixture must build {want_bodies} bodies"
    );
    let bodies: Vec<BodyId> = snap.bodies.iter().map(|b| b.body).collect();
    let mut volumes = Vec::new();
    for b in &bodies {
        volumes.push(exact_volume(wm, *b).await);
    }
    let written = GeometryExporter::export_step(
        wm,
        &path.to_string_lossy(),
        &bodies,
        "AP214IS",
        &Default::default(),
    )
    .await;
    written.expect("ExportStep the fixture");
    volumes
}

/// A one-part plan over `file`.
fn plan_for(file: &Path, id: &str, kind: IngestKind, keep: KeepSpec) -> IngestPlan {
    let d = IngestDefaults::default();
    IngestPlan {
        parts: vec![IngestPart {
            file: file.to_path_buf(),
            kind,
            id: id.to_string(),
            version: d.version.clone(),
            name: id.to_string(),
            standard: None,
            designation: None,
            category: d.category.clone(),
            tags: d.tags.clone(),
            keep,
            profile: ProfileSpec {
                length_default: STOCK_LENGTH_MM,
                length_min: Some(1.0),
                axis_hint: None,
            },
            attachment: None,
        }],
    }
}

fn keep_all() -> KeepSpec {
    KeepSpec {
        mode: KeepMode::All,
        drop_names: Vec::new(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// (a) the `profile` leg
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stick_ingests_as_a_profile_and_places_at_a_length_the_vendor_never_shipped() {
    use onecad_core::document::record::ComponentParamValue;
    use tauri::Manager;

    let Some(bin) = real_worker() else {
        eprintln!("skip: real worker binary not found (set ONECAD_WORKER_PATH)");
        return;
    };
    let tmp = tempfile::tempdir().expect("tempdir");
    let library = tempfile::tempdir().expect("tempdir");
    let step = tmp.path().join("stock_2020.step");

    let wm = spawn_worker(bin).await;
    const BASE: u128 = 0xC0DE00;
    let sid = SketchId(Uuid::from_u128(BASE + 2));
    let volumes = export_fixture(
        &wm,
        &step,
        vec![
            sketch_record(BASE, &stock_profile_sketch(sid, BASE + 0x1000)),
            extrude_record(BASE + 1, sid, STOCK_LENGTH_MM),
        ],
        1,
    )
    .await;
    let want_stock = PROFILE_AREA_MM2 * STOCK_LENGTH_MM;
    assert!(
        (volumes[0] - want_stock).abs() <= 1e-6 * want_stock,
        "the exported stick must be the square MINUS the hole: got {}, want {want_stock}",
        volumes[0]
    );

    let plan = plan_for(&step, "acme.stock.2020", IngestKind::Profile, keep_all());
    let report = ingest_components_at(&wm, library.path(), &plan).await;
    let part = &report.parts[0];
    assert_eq!(
        part.status,
        IngestStatusDto::Ok,
        "the stick must ingest as a profile. An `unknown verb` / protocol failure here \
         means the C++ half of WP-C is NOT in the staged worker — rebuild the sidecar; \
         this test must never be read as green without it. message={:?}",
        part.message
    );
    assert_eq!(part.solids_found, Some(1));
    assert_eq!(part.solids_kept, Some(1));

    // The package the ingest wrote is a PROFILE — the whole point of the kind.
    let lib = onecad_library::Library::open(library.path()).expect("open library");
    let (version, entry) = lib.get("acme.stock.2020", None).expect("indexed");
    assert_eq!(entry.source_kind, "profile");
    assert_eq!(version, "1.0.0");

    // Place it at 120 mm — a length the "vendor" never shipped.
    let app_state = app_state_over(&wm);
    {
        let (engine, meshes, solver) = app_state.make_backend();
        *app_state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));
    }
    let app = tauri::test::mock_app();
    app.manage(app_state);
    let state: tauri::State<'_, onecad_lib::state::AppState> = app.state();

    let mut at_120 = std::collections::BTreeMap::new();
    at_120.insert(
        "length".to_string(),
        ComponentParamValue::Number(PLACED_LENGTH_MM),
    );
    onecad_lib::library::place_component_at(
        library.path(),
        &state,
        "acme.stock.2020".to_string(),
        "1.0.0".to_string(),
        [0.0, 0.0, 0.0],
        None,
        at_120,
        None,
    )
    .await
    .expect("place the ingested profile at 120 mm");

    let placed = {
        let mut guard = state.runtime.lock().await;
        let rt = guard.as_mut().unwrap();
        let report = regen_all(rt).await;
        let snap = published(&report, "place the ingested profile");
        assert_eq!(snap.bodies.len(), 1, "a prism over one face is one solid");
        exact_volume(&wm, snap.bodies[0].body).await
    };
    assert!(
        (placed - PLACED_VOLUME_MM3).abs() <= 1e-6 * PLACED_VOLUME_MM3,
        "placed at 120 mm: got {placed}, want {PLACED_VOLUME_MM3}"
    );

    wm.shutdown().await;
}

/// An `AppState` wired to the SAME `WorkerManager` for every facet (verbatim from
/// `component_ops.rs` — production's `real_worker_factory` minus the restart-hook
/// wiring these tests never exercise).
fn app_state_over(wm: &WorkerManager) -> onecad_lib::state::AppState {
    use onecad_lib::worker::{
        CircuitControl, FaceBoundaryProjection, PreviewEngine, StepImport, WorkerReadiness,
    };
    let wm = wm.clone();
    onecad_lib::state::AppState::new(Arc::new(move || {
        let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
        let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
        let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
        let exporter: Arc<dyn GeometryExporter> = Arc::new(wm.clone());
        let elements: Arc<dyn ElementQuery> = Arc::new(wm.clone());
        let preview: Arc<dyn PreviewEngine> = Arc::new(wm.clone());
        let face_projection: Arc<dyn FaceBoundaryProjection> = Arc::new(wm.clone());
        let step_import: Arc<dyn StepImport> = Arc::new(wm.clone());
        let circuit: Arc<dyn CircuitControl> = Arc::new(wm.clone());
        let readiness: Arc<dyn WorkerReadiness> = Arc::new(wm.clone());
        (
            engine,
            meshes,
            solver,
            exporter,
            elements,
            preview,
            face_projection,
            step_import,
            circuit,
            readiness,
        )
    }))
}

// ─────────────────────────────────────────────────────────────────────────────
// (b) the `embedded` leg — refusal and success
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_disjoint_boxes_refuse_with_the_disjoint_marker() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: real worker binary not found (set ONECAD_WORKER_PATH)");
        return;
    };
    let tmp = tempfile::tempdir().expect("tempdir");
    let library = tempfile::tempdir().expect("tempdir");
    let step = tmp.path().join("two_disjoint.step");
    let wm = spawn_worker(bin).await;

    const BASE: u128 = 0xD15000;
    let a = SketchId(Uuid::from_u128(BASE + 1));
    let b = SketchId(Uuid::from_u128(BASE + 2));
    export_fixture(
        &wm,
        &step,
        vec![
            sketch_record(
                BASE + 0x10,
                &rect_sketch(a, BASE + 0x1000, 0.0, 0.0, 10.0, 10.0),
            ),
            extrude_record(BASE + 0x11, a, 10.0),
            // 100 mm away in X: nothing to fuse to.
            sketch_record(
                BASE + 0x20,
                &rect_sketch(b, BASE + 0x2000, 100.0, 0.0, 10.0, 10.0),
            ),
            extrude_record(BASE + 0x21, b, 10.0),
        ],
        2,
    )
    .await;

    let plan = plan_for(
        &step,
        "acme.disjoint.pair",
        IngestKind::Embedded,
        keep_all(),
    );
    let report = ingest_components_at(&wm, library.path(), &plan).await;
    let part = &report.parts[0];
    assert_eq!(
        part.status,
        IngestStatusDto::Refused,
        "two solids that do not touch cannot be one component; message={:?}",
        part.message
    );
    let message = part.message.clone().expect("a refusal carries its reason");
    assert!(
        message.contains(DISJOINT_REFUSAL),
        "the refusal must carry the machine-readable marker, got {message:?}"
    );
    assert_eq!(part.solids_found, Some(2));
    assert_eq!(part.solids_kept, Some(2));
    assert!(
        onecad_library::Library::open(library.path())
            .expect("open library")
            .get("acme.disjoint.pair", None)
            .is_none(),
        "a refusal must write NO package"
    );

    wm.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_touching_boxes_fuse_into_one_solid() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: real worker binary not found (set ONECAD_WORKER_PATH)");
        return;
    };
    let tmp = tempfile::tempdir().expect("tempdir");
    let library = tempfile::tempdir().expect("tempdir");
    let step = tmp.path().join("two_touching.step");
    let wm = spawn_worker(bin).await;

    const BASE: u128 = 0x70CE00;
    let a = SketchId(Uuid::from_u128(BASE + 1));
    let b = SketchId(Uuid::from_u128(BASE + 2));
    let volumes = export_fixture(
        &wm,
        &step,
        vec![
            sketch_record(
                BASE + 0x10,
                &rect_sketch(a, BASE + 0x1000, 0.0, 0.0, 20.0, 10.0),
            ),
            extrude_record(BASE + 0x11, a, 10.0),
            // Shares the x = 20 face exactly.
            sketch_record(
                BASE + 0x20,
                &rect_sketch(b, BASE + 0x2000, 20.0, 0.0, 10.0, 10.0),
            ),
            extrude_record(BASE + 0x21, b, 10.0),
        ],
        2,
    )
    .await;
    let want: f64 = volumes.iter().sum();

    let plan = plan_for(
        &step,
        "acme.touching.pair",
        IngestKind::Embedded,
        keep_all(),
    );
    let report = ingest_components_at(&wm, library.path(), &plan).await;
    let part = &report.parts[0];
    assert_eq!(
        part.status,
        IngestStatusDto::Ok,
        "two solids sharing a face fuse; message={:?}",
        part.message
    );
    assert_eq!(part.solids_found, Some(2));
    assert_eq!(part.solids_kept, Some(2));

    let lib = onecad_library::Library::open(library.path()).expect("open library");
    let (_v, entry) = lib.get("acme.touching.pair", None).expect("indexed");
    assert_eq!(entry.source_kind, "embedded");

    // The volume of what the LIBRARY holds, not of the document the fuse ran in:
    // placing the package replays the baked blob, so this is the only measurement
    // that proves the right bytes landed. A union of two solids sharing a face
    // adds and removes nothing, so it is the exact sum.
    let app_state = app_state_over(&wm);
    {
        let (engine, meshes, solver) = app_state.make_backend();
        *app_state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));
    }
    let app = tauri::test::mock_app();
    tauri::Manager::manage(&app, app_state);
    let state: tauri::State<'_, onecad_lib::state::AppState> = tauri::Manager::state(&app);
    onecad_lib::library::place_component_at(
        library.path(),
        &state,
        "acme.touching.pair".to_string(),
        "1.0.0".to_string(),
        [0.0, 0.0, 0.0],
        None,
        Default::default(),
        None,
    )
    .await
    .expect("place the ingested embedded component");
    let placed = {
        let mut guard = state.runtime.lock().await;
        let rt = guard.as_mut().unwrap();
        let report = regen_all(rt).await;
        let snap = published(&report, "place the fused component");
        assert_eq!(snap.bodies.len(), 1, "the fuse produced ONE solid");
        exact_volume(&wm, snap.bodies[0].body).await
    };
    assert!(
        (placed - want).abs() < 1e-9,
        "fused volume {placed} must be the exact sum of the two boxes {want}"
    );

    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// (c) the tracked vendor recipe
// ─────────────────────────────────────────────────────────────────────────────

/// Replays the repo's own `STEP/ingest.toml` against the real vendor files.
///
/// `STEP/` is gitignored (vendor downloads, 0.2–7 MB), so CI has neither the
/// files nor the recipe. Absence is the ONE sanctioned skip in this file and it
/// says so out loud; a recipe that is present but unparseable, or a part that
/// regresses from `ok`, still fails.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_tracked_vendor_recipe_ingests() {
    let recipe = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .join("STEP/ingest.toml");
    if !recipe.is_file() {
        eprintln!("SKIP: STEP/ vendor files not present (gitignored)");
        return;
    }
    let Some(bin) = real_worker() else {
        eprintln!("skip: real worker binary not found (set ONECAD_WORKER_PATH)");
        return;
    };
    let text = std::fs::read_to_string(&recipe).expect("read STEP/ingest.toml");
    let plan = parse_recipe(&text, recipe.parent().expect("STEP/")).expect("parse the recipe");
    if plan.parts.iter().any(|p| !p.file.is_file()) {
        eprintln!("SKIP: STEP/ vendor files not present (gitignored)");
        return;
    }

    let library = tempfile::tempdir().expect("tempdir");
    let wm = spawn_worker(bin).await;
    let report = ingest_components_at(&wm, library.path(), &plan).await;
    for part in &report.parts {
        eprintln!(
            "{:?} {:<40} found={:?} kept={:?} faces={:?} {}",
            part.status,
            part.id.as_deref().unwrap_or("-"),
            part.solids_found,
            part.solids_kept,
            part.face_count,
            part.message.as_deref().unwrap_or("")
        );
    }
    // The five Rollco sticks and the SG90 are `ok`; the NEMA 17 is a RECORDED
    // refusal (see the recipe's own comment) and would be a green-washing lie to
    // assert as ok. Anything that FAILED is an environment problem.
    assert!(
        report
            .parts
            .iter()
            .all(|p| p.status != IngestStatusDto::Failed),
        "no part of the tracked recipe may FAIL (a refusal is a recorded answer)"
    );
    let ok = report
        .parts
        .iter()
        .filter(|p| p.status == IngestStatusDto::Ok)
        .count();
    assert!(
        ok >= plan.parts.len() - 1,
        "at most the recorded NEMA 17 refusal may be non-ok; {ok} of {} were ok",
        plan.parts.len()
    );

    wm.shutdown().await;
}
