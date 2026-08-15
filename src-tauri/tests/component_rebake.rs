//! Component Library WP-F1.3 — the RE-BAKE lane, against the REAL C++ OCCT
//! worker, driven through the app's actual command cores via
//! `tauri::test::mock_app` (the same harness `sketch_on_face.rs` uses).
//!
//! The claim under test is the one the whole program has been building toward:
//! a component authored from a document can expose its VARIABLES as free
//! parameters, and editing one on a placed instance actually moves geometry —
//! not a designation string over an unchanged solid.
//!
//! Volumes come from `QueryMassProperties` (`BRepGProp` over the real BRep), so
//! the analytic expectations hold to kernel precision.
//!
//! * `a_declared_parameter_re_bakes_the_placed_solid_and_survives_a_reopen` —
//!   author `depth = 10` (a 20×20×10 = 4000 mm³ solid) as a component with
//!   `depth` declared free, place it, `setComponentParams depth = 20`, and the
//!   PLACED body is 8000 mm³ — then save, delete the library, reopen on a fresh
//!   worker, and it is still 8000 (the re-baked blob is cached in the document).
//! * `a_missing_package_refuses_the_edit_and_leaves_the_geometry_standing` —
//!   with the library gone the edit is a typed refusal and the instance keeps
//!   rendering exactly what it had.
//! * `a_source_document_with_several_bodies_refuses_the_re_bake` — a re-bake
//!   never silently fuses: the author's bake choice was made at save time.
//!
//! Gated on `ONECAD_WORKER_PATH` (else the dev-tree fallback); a missing binary
//! skips cleanly unless `ONECAD_REQUIRE_WORKER=1`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tauri::Manager;
use uuid::Uuid;

use onecad_core::document::record::{
    BooleanMode, ComponentParamValue, ExtrudeMode, ExtrudeParams, KnownOperation, Operation,
    OperationRecord, PlaneKind, SketchOpParams, SketchPlaneRef,
};
use onecad_core::document::refs::SketchRegionRef;
use onecad_core::document::variables::{Scalar, Unit, Variable};
use onecad_core::edit::EditCommand;
use onecad_core::ids::{BodyId, ConstraintId, EntityId, RecordId, RegionId, SketchId, VariableId};
use onecad_core::io::container::SaveMeta;
use onecad_core::math::{Vec2, Vec3};
use onecad_core::regen::{CancelToken, GeometryEngine, ModelSnapshot, Outcome, RegenRequest};
use onecad_core::sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane};

use onecad_lib::document_runtime::{DocumentRuntime, RegenReport};
use onecad_lib::export::GeometryExporter;
use onecad_lib::library::{
    place_component_at, reindex_library_at, save_as_component_at, set_component_params_at,
    FreeParameterInput, NewComponentSpec,
};
use onecad_lib::state::AppState;
use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::wire::sketch_wire;
use onecad_lib::worker::{
    resolve_worker_path, CircuitControl, ElementQuery, FaceBoundaryProjection, MeshProvider,
    PreviewEngine, SolverEngine, StepImport, WorkerManager, WorkerReadiness,
};

// Per-test record-id bases (see `author_plate` for why they must differ).
const PLATE_BASE: u128 = 0xF1300;
const MISSING_BASE: u128 = 0xF2300;
const TWO_BODY_BASE: u128 = 0xF3300;

// ─────────────────────────────────────────────────────────────────────────────
// Harness (mirrors variable_driven_ops.rs + sketch_on_face.rs)
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

async fn spawn_worker(bin: PathBuf) -> WorkerManager {
    let wm = WorkerManager::spawn(SupervisorConfig::production(bin));
    assert!(
        wm.wait_ready(Duration::from_secs(10)).await,
        "real worker must connect + handshake + OpenSession"
    );
    wm
}

/// An `AppState` wired to the SAME `WorkerManager` for every facet — what
/// `state::real_worker_factory` does in production, minus the restart-hook /
/// status-forwarder wiring these tests never exercise.
fn app_state_over(wm: &WorkerManager) -> AppState {
    let wm = wm.clone();
    AppState::new(Arc::new(move || {
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

fn open_over(wm: &WorkerManager, path: &std::path::Path) -> DocumentRuntime {
    let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
    DocumentRuntime::open(path, engine, meshes, solver).expect("reopen the saved container")
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

/// The EXACT kernel volume (`BRepGProp`), not a tessellation estimate.
async fn exact_volume(wm: &WorkerManager, body: BodyId) -> f64 {
    ElementQuery::query_mass_properties(wm, body, "b".into())
        .await
        .expect("QueryMassProperties")
        .volume
}

fn save_meta() -> SaveMeta {
    SaveMeta {
        app_version: "0.1.0-test".into(),
        occt_fingerprint: Some("occt-8.0.1".into()),
        created: "2026-08-13T00:00:00Z".into(),
        modified: "2026-08-13T00:00:00Z".into(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Sketch + op builders (rect_sketch verbatim from variable_driven_ops.rs)
// ─────────────────────────────────────────────────────────────────────────────

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

fn rect_sketch(sid: SketchId, base: u128, x0: f64, y0: f64, w: f64, h: f64) -> Sketch {
    let e = |n: u128| EntityId(Uuid::from_u128(base + n));
    let c = |n: u128| ConstraintId(Uuid::from_u128(base + 0x40 + n));
    let (p0s, p0e) = (e(0), e(1));
    let (p1s, p1e) = (e(2), e(3));
    let (p2s, p2e) = (e(4), e(5));
    let (p3s, p3e) = (e(6), e(7));
    let (l0, l1, l2, l3) = (e(0x10), e(0x11), e(0x12), e(0x13));

    let mut sk = Sketch::on_world_plane(sid, "Rect", WorldPlane::XY);
    let pt = |sk: &mut Sketch, id: EntityId, x: f64, y: f64| {
        sk.add_entity(SketchEntity::point(
            id,
            Vec2::new_unchecked(x, y),
            false,
            false,
        ))
        .unwrap();
    };
    pt(&mut sk, p0s, x0, y0);
    pt(&mut sk, p0e, x0 + w, y0);
    pt(&mut sk, p1s, x0 + w, y0);
    pt(&mut sk, p1e, x0 + w, y0 + h);
    pt(&mut sk, p2s, x0 + w, y0 + h);
    pt(&mut sk, p2e, x0, y0 + h);
    pt(&mut sk, p3s, x0, y0 + h);
    pt(&mut sk, p3e, x0, y0);
    sk.add_entity(SketchEntity::line(l0, p0s, p0e, false))
        .unwrap();
    sk.add_entity(SketchEntity::line(l1, p1s, p1e, false))
        .unwrap();
    sk.add_entity(SketchEntity::line(l2, p2s, p2e, false))
        .unwrap();
    sk.add_entity(SketchEntity::line(l3, p3s, p3e, false))
        .unwrap();

    let coincident = |sk: &mut Sketch, id, a, b| {
        sk.add_constraint(Constraint::Coincident {
            id,
            point1: a,
            point2: b,
            point1_position: CurvePosition::Arbitrary,
            point2_position: CurvePosition::Arbitrary,
        })
        .unwrap();
    };
    coincident(&mut sk, c(1), p0e, p1s);
    coincident(&mut sk, c(2), p1e, p2s);
    coincident(&mut sk, c(3), p2e, p3s);
    coincident(&mut sk, c(4), p3e, p0s);
    sk.add_constraint(Constraint::Horizontal { id: c(5), line: l0 })
        .unwrap();
    sk.add_constraint(Constraint::Horizontal { id: c(6), line: l2 })
        .unwrap();
    sk.add_constraint(Constraint::Vertical { id: c(7), line: l1 })
        .unwrap();
    sk.add_constraint(Constraint::Vertical { id: c(8), line: l3 })
        .unwrap();
    sk.add_constraint(Constraint::Fixed {
        id: c(9),
        point: p0s,
        point_position: CurvePosition::Arbitrary,
        at: Vec2::new_unchecked(x0, y0),
    })
    .unwrap();
    sk.add_constraint(Constraint::HorizontalDistance {
        id: c(10),
        point1: p0s,
        point1_position: CurvePosition::Arbitrary,
        point2: p0e,
        point2_position: CurvePosition::Arbitrary,
        value: Scalar::new(w),
    })
    .unwrap();
    sk.add_constraint(Constraint::VerticalDistance {
        id: c(11),
        point1: p1s,
        point1_position: CurvePosition::Arbitrary,
        point2: p1e,
        point2_position: CurvePosition::Arbitrary,
        value: Scalar::new(h),
    })
    .unwrap();
    sk
}

fn sketch_record(rec: u128, sk: &Sketch) -> OperationRecord {
    let (_plane, entities, constraints) = sketch_wire(sk);
    let op = Operation::Known(KnownOperation::Sketch(SketchOpParams {
        sketch: sk.id,
        plane: xy_plane_ref(),
        entities: entities.as_array().cloned().unwrap_or_default(),
        constraints: constraints.as_array().cloned().unwrap_or_default(),
        host_face: None,
        extra: Default::default(),
    }));
    OperationRecord::new(RecordId(Uuid::from_u128(rec)), 0, "Sketch", op)
}

/// An extrude whose `distance` is EXPRESSION-DRIVEN (`expr` names a variable),
/// or a plain literal when `expr` is `None`.
fn extrude_record(
    rec: u128,
    sketch: SketchId,
    distance: f64,
    expr: Option<&str>,
) -> OperationRecord {
    let op = Operation::Known(KnownOperation::Extrude(ExtrudeParams {
        profile: Some(SketchRegionRef {
            sketch,
            region: RegionId::new(""), // first-region fallback
            region_identity_version: None,
            extra: Default::default(),
        }),
        distance: match expr {
            Some(e) => Scalar::with_expr(distance, e),
            None => Scalar::new(distance),
        },
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

fn var(name: &str, value: f64) -> Variable {
    Variable {
        id: VariableId::new(),
        name: name.to_string(),
        value: Scalar::new(value),
        unit: Unit::Mm,
    }
}

/// The identity + metadata half of the save, with `parameters` supplied.
fn spec_with(parameters: BTreeMap<String, FreeParameterInput>) -> NewComponentSpec {
    NewComponentSpec {
        id: "acme.plate".to_string(),
        version: "1.0.0".to_string(),
        name: "Plate".to_string(),
        category: vec!["mine".to_string()],
        tags: vec![],
        designation: None,
        attachments: Default::default(),
        parameters,
    }
}

/// `{ depth: { key: "depth", value: 10 } }` — the table the authoring dialog
/// sends when the author checks the `depth` variable.
fn depth_parameter() -> BTreeMap<String, FreeParameterInput> {
    [(
        "depth".to_string(),
        FreeParameterInput {
            key: "depth".to_string(),
            value: 10.0,
        },
    )]
    .into_iter()
    .collect()
}

/// Installs a fresh blank document as `state`'s open runtime, over `wm`.
async fn open_blank(app_state: &AppState) {
    let (engine, meshes, solver) = app_state.make_backend();
    *app_state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));
}

/// The variable-driven authoring document: `depth` (10 mm) driving a 20×20
/// extrude, i.e. 4000 mm³ at rest.
///
/// `base` must DIFFER PER TEST. A NewBody's id is its record's uuid, and
/// `save_as_component_at` names its authoring scratch directory after the body
/// it bakes — two tests baking the same body id in one process would share (and
/// sweep) one directory, which is exactly the flake these ids exist to avoid.
async fn author_plate(app_state: &AppState, base: u128) -> BodyId {
    let mut guard = app_state.runtime.lock().await;
    let rt = guard.as_mut().unwrap();
    let sid = SketchId(Uuid::from_u128(base + 2));
    rt.apply(EditCommand::AddVariable {
        variable: var("depth", 10.0),
    })
    .expect("AddVariable depth");
    add_op(
        rt,
        sketch_record(base, &rect_sketch(sid, base + 0x1000, 0.0, 0.0, 20.0, 20.0)),
    );
    // The cached `value` is deliberately absurd: only the TABLE's number may build.
    add_op(rt, extrude_record(base + 1, sid, 999.0, Some("depth")));
    let report = regen_all(rt).await;
    let snap = published(&report, "author the plate");
    assert_eq!(snap.bodies.len(), 1, "the authoring document has one body");
    snap.bodies[0].body
}

// ─────────────────────────────────────────────────────────────────────────────
// 1. The parameter IS the placed geometry, and it survives the library's death
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_declared_parameter_re_bakes_the_placed_solid_and_survives_a_reopen() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: real worker binary not found (set ONECAD_WORKER_PATH)");
        return;
    };
    let library_dir = tempfile::tempdir().expect("tempdir");
    let doc_dir = tempfile::tempdir().expect("tempdir");
    let wm = spawn_worker(bin).await;

    let app_state = app_state_over(&wm);
    open_blank(&app_state).await;
    let body = author_plate(&app_state, PLATE_BASE).await;
    assert!(
        (exact_volume(&wm, body).await - 4_000.0).abs() < 1e-6,
        "20×20×depth(10) = 4000 before anything is saved"
    );

    let saved = save_as_component_at(
        library_dir.path(),
        &app_state.runtime,
        app_state.exporter(),
        body.to_string(),
        spec_with(depth_parameter()),
        None,
        false,
    )
    .await
    .expect("save as component");
    assert_eq!(saved.source_kind, "document");
    assert_eq!(
        saved.parameters.get("depth").and_then(|p| p.key.clone()),
        Some("depth".to_string()),
        "the manifest binds the parameter to the SOURCE VARIABLE by name"
    );
    reindex_library_at(library_dir.path()).expect("reindex");

    // A FRESH document to place into — the placing document knows nothing about
    // the authoring one beyond the package.
    open_blank(&app_state).await;
    let app = tauri::test::mock_app();
    app.manage(app_state);
    let state: tauri::State<'_, AppState> = app.state();

    let (_outcome, projection) = place_component_at(
        library_dir.path(),
        &state,
        "acme.plate".to_string(),
        "1.0.0".to_string(),
        [0.0, 0.0, 0.0],
        None,
        Default::default(),
        None,
    )
    .await
    .expect("place the authored component");
    let record_id = projection.features[0].id.clone();

    let placed_volume = {
        let mut guard = state.runtime.lock().await;
        let rt = guard.as_mut().unwrap();
        let report = regen_all(rt).await;
        let snap = published(&report, "place");
        assert_eq!(snap.bodies.len(), 1);
        exact_volume(&wm, snap.bodies[0].body).await
    };
    assert!(
        (placed_volume - 4_000.0).abs() < 1e-6,
        "the placement lands the baked 4000 mm³ solid; got {placed_volume}"
    );

    // A key the package never declared is refused BEFORE any re-bake.
    let mut bogus = BTreeMap::new();
    bogus.insert("width".to_string(), ComponentParamValue::Number(5.0));
    let err = set_component_params_at(library_dir.path(), &state, record_id.clone(), bogus)
        .await
        .expect_err("an undeclared key has nothing to drive");
    assert!(
        format!("{err:?}").contains("width"),
        "the refusal names the key: {err:?}"
    );

    // THE PROOF: the override replays the frozen source with `depth = 20` and
    // swaps in the solid it bakes.
    let mut params = BTreeMap::new();
    params.insert("depth".to_string(), ComponentParamValue::Number(20.0));
    set_component_params_at(library_dir.path(), &state, record_id.clone(), params)
        .await
        .expect("re-bake at depth = 20");

    let rebaked_volume = {
        let mut guard = state.runtime.lock().await;
        let rt = guard.as_mut().unwrap();
        let report = regen_all(rt).await;
        let snap = published(&report, "re-baked placement");
        assert_eq!(
            snap.bodies.len(),
            1,
            "the instance is still exactly one body at the SAME record"
        );
        exact_volume(&wm, snap.bodies[0].body).await
    };
    assert!(
        (rebaked_volume - 8_000.0).abs() < 1e-6,
        "20×20×depth(20) = 8000 after the re-bake; got {rebaked_volume} (was {placed_volume})"
    );

    // …and the re-baked bytes are the DOCUMENT's now: save, delete the library
    // entirely, reopen on a fresh worker (spec §4/§12).
    let path = doc_dir.path().join("placed.onecad");
    {
        let mut guard = state.runtime.lock().await;
        guard
            .as_mut()
            .unwrap()
            .save(&path, save_meta())
            .expect("save");
    }
    std::fs::remove_dir_all(library_dir.path()).expect("delete the library");
    wm.shutdown().await;
    wm.retire();

    let bin = real_worker().expect("worker binary");
    let wm2 = spawn_worker(bin).await;
    let mut reopened = open_over(&wm2, &path);
    let report = regen_all(&mut reopened).await;
    let snap = published(&report, "reopen with no library");
    assert_eq!(snap.bodies.len(), 1);
    let reopened_volume = exact_volume(&wm2, snap.bodies[0].body).await;
    assert!(
        (reopened_volume - 8_000.0).abs() < 1e-6,
        "the re-baked solid is cached in the document, not the library; got {reopened_volume}"
    );

    wm2.shutdown().await;
    wm2.retire();
    eprintln!("WP-F1.3 PASS: depth 10 → 20 moved the PLACED volume 4000 → 8000, library-free");
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. A package that is gone refuses the EDIT, never the document
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_missing_package_refuses_the_edit_and_leaves_the_geometry_standing() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: real worker binary not found (set ONECAD_WORKER_PATH)");
        return;
    };
    let library_dir = tempfile::tempdir().expect("tempdir");
    let wm = spawn_worker(bin).await;

    let app_state = app_state_over(&wm);
    open_blank(&app_state).await;
    let body = author_plate(&app_state, MISSING_BASE).await;
    save_as_component_at(
        library_dir.path(),
        &app_state.runtime,
        app_state.exporter(),
        body.to_string(),
        spec_with(depth_parameter()),
        None,
        false,
    )
    .await
    .expect("save as component");
    reindex_library_at(library_dir.path()).expect("reindex");

    open_blank(&app_state).await;
    let app = tauri::test::mock_app();
    app.manage(app_state);
    let state: tauri::State<'_, AppState> = app.state();

    let (_outcome, projection) = place_component_at(
        library_dir.path(),
        &state,
        "acme.plate".to_string(),
        "1.0.0".to_string(),
        [0.0, 0.0, 0.0],
        None,
        Default::default(),
        None,
    )
    .await
    .expect("place");
    let record_id = projection.features[0].id.clone();

    // The package disappears out from under the instance (its directory is the
    // library's own layout decision, so it is read back from the index).
    let package_dir = {
        let library = onecad_library::Library::open(library_dir.path()).expect("open library");
        let (_v, entry) = library.get("acme.plate", None).expect("indexed");
        library_dir.path().join(&entry.path)
    };
    std::fs::remove_dir_all(&package_dir).expect("delete the package");
    reindex_library_at(library_dir.path()).expect("reindex without it");

    let mut params = BTreeMap::new();
    params.insert("depth".to_string(), ComponentParamValue::Number(20.0));
    let err = set_component_params_at(library_dir.path(), &state, record_id, params)
        .await
        .expect_err("a re-bake needs the package it would replay");
    assert!(
        format!("{err:?}").contains("acme.plate"),
        "the refusal names the component: {err:?}"
    );

    // …and the instance still renders its CACHED solid: the edit failed, the
    // document did not.
    let volume = {
        let mut guard = state.runtime.lock().await;
        let rt = guard.as_mut().unwrap();
        let report = regen_all(rt).await;
        let snap = published(&report, "regen after the refused edit");
        assert_eq!(snap.bodies.len(), 1);
        exact_volume(&wm, snap.bodies[0].body).await
    };
    assert!(
        (volume - 4_000.0).abs() < 1e-6,
        "the cached blob is authoritative — geometry must be untouched; got {volume}"
    );

    wm.shutdown().await;
    wm.retire();
}

// ─────────────────────────────────────────────────────────────────────────────
// 3. A re-bake never silently fuses
// ─────────────────────────────────────────────────────────────────────────────

/// The author picked ONE body to bake (and answered the single-solid question
/// at save time). Replaying a source document that builds SEVERAL bodies has no
/// honest answer here — fusing them would change what the component IS, and
/// picking one would be a guess — so the edit is refused, loudly, and the
/// instance keeps the solid it had.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_source_document_with_several_bodies_refuses_the_re_bake() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: real worker binary not found (set ONECAD_WORKER_PATH)");
        return;
    };
    let library_dir = tempfile::tempdir().expect("tempdir");
    let wm = spawn_worker(bin).await;

    let app_state = app_state_over(&wm);
    open_blank(&app_state).await;
    let body = author_plate(&app_state, TWO_BODY_BASE).await;
    // A SECOND, disjoint body in the same document — baked from neither.
    {
        let mut guard = app_state.runtime.lock().await;
        let rt = guard.as_mut().unwrap();
        let sid2 = SketchId(Uuid::from_u128(TWO_BODY_BASE + 0x12));
        add_op(
            rt,
            sketch_record(
                TWO_BODY_BASE + 0x10,
                &rect_sketch(sid2, TWO_BODY_BASE + 0x2000, 100.0, 100.0, 5.0, 5.0),
            ),
        );
        add_op(rt, extrude_record(TWO_BODY_BASE + 0x11, sid2, 5.0, None));
        let report = regen_all(rt).await;
        assert_eq!(
            published(&report, "two bodies").bodies.len(),
            2,
            "the authoring document now holds two bodies"
        );
    }

    save_as_component_at(
        library_dir.path(),
        &app_state.runtime,
        app_state.exporter(),
        body.to_string(),
        spec_with(depth_parameter()),
        None,
        false,
    )
    .await
    .expect("saving ONE body of a two-body document is fine");
    reindex_library_at(library_dir.path()).expect("reindex");

    open_blank(&app_state).await;
    let app = tauri::test::mock_app();
    app.manage(app_state);
    let state: tauri::State<'_, AppState> = app.state();

    let (_outcome, projection) = place_component_at(
        library_dir.path(),
        &state,
        "acme.plate".to_string(),
        "1.0.0".to_string(),
        [0.0, 0.0, 0.0],
        None,
        Default::default(),
        None,
    )
    .await
    .expect("place");
    let record_id = projection.features[0].id.clone();

    let mut params = BTreeMap::new();
    params.insert("depth".to_string(), ComponentParamValue::Number(20.0));
    let err = set_component_params_at(library_dir.path(), &state, record_id, params)
        .await
        .expect_err("a two-body replay has no single solid to swap in");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("2 bodies") || msg.contains("2 solids"),
        "the refusal must say what it found: {msg}"
    );

    wm.shutdown().await;
    wm.retire();
}
