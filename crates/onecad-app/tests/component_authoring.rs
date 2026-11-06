//! Create Component backend (ONECAD-301), against the REAL C++ OCCT worker.
//!
//! A bracket document with variables `width`, `thickness` and the DERIVED
//! `post_h = width / 2` is saved as a parametric component with three named
//! configurations and two attachments derived from picks, then placed directly
//! as a chosen variant.
//!
//! Bracket geometry (all closed-form): a 20x20 plate with a r=3 bore at the
//! origin extruded `thickness`, plus a r=4 post centred at (10, 0) extruded
//! `post_h` and fused onto it (`post_h >= thickness`), so
//! `V = (400 - 17 pi) * thickness + 16 pi * post_h` and the top of the post is
//! the body's highest point, `z = post_h`.
//!
//! Gated on `ONECAD_WORKER_PATH` (else the dev-tree fallback); a missing binary
//! skips cleanly unless `ONECAD_REQUIRE_WORKER=1`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use onecad_core::document::record::{
    BooleanMode, ComponentParamValue, ExtrudeMode, ExtrudeParams, KnownOperation, Operation,
    OperationRecord, PlaceComponentParams, PlaneKind, SketchOpParams, SketchPlaneRef,
};
use onecad_core::document::refs::SketchRegionRef;
use onecad_core::document::variables::{Scalar, Unit, Variable};
use onecad_core::edit::EditCommand;
use onecad_core::ids::{BodyId, EntityId, RecordId, RegionId, SketchId, VariableId};
use onecad_core::io::container::SaveMeta;
use onecad_core::math::{Vec2, Vec3};
use onecad_core::regen::{CancelToken, GeometryEngine, Lod, ModelSnapshot, Outcome, RegenRequest};
use onecad_core::sketch::{Sketch, SketchEntity, WorldPlane};

use onecad_app::document_runtime::{DocumentRuntime, RegenReport};
use onecad_app::export::GeometryExporter;
use onecad_app::library::{
    attachment_from_pick, list_library_components_at, prepare_place_component_at,
    reindex_library_at, save_as_component_at, stage_prepared_blob, FreeParameterInput,
    NewComponentSpec, PlaceComponentRequest,
};
use onecad_app::state::AppState;
use onecad_app::worker::manager::SupervisorConfig;
use onecad_app::worker::wire::sketch_wire;
use onecad_app::worker::{
    resolve_worker_path, CircuitControl, ElementQuery, FaceBoundaryProjection, MeshProvider,
    PreviewEngine, SolverEngine, StepImport, WorkerManager, WorkerReadiness,
};

const BRACKET_BASE: u128 = 0xC0300;
const REFUSAL_BASE: u128 = 0xC1300;
const VARIANT_BASE: u128 = 0xC2300;

// ─────────────────────────────────────────────────────────────────────────────
// Harness (copied from component_rebake.rs)
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

fn var(name: &str, value: f64) -> Variable {
    Variable {
        id: VariableId::new(),
        name: name.to_string(),
        value: Scalar::new(value),
        unit: Unit::Mm,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The bracket document
// ─────────────────────────────────────────────────────────────────────────────

fn circle_sketch(sid: SketchId, base: u128, circles: &[(f64, f64, f64)], square: bool) -> Sketch {
    let e = |n: u128| EntityId(Uuid::from_u128(base + n));
    let mut sk = Sketch::on_world_plane(sid, "Bracket", WorldPlane::XY);
    let point = |sk: &mut Sketch, id: EntityId, x: f64, y: f64| {
        sk.add_entity(SketchEntity::point(
            id,
            Vec2::new_unchecked(x, y),
            false,
            false,
        ))
        .expect("point");
    };
    if square {
        for (n, (x, y)) in [(-10.0, -10.0), (10.0, -10.0), (10.0, 10.0), (-10.0, 10.0)]
            .into_iter()
            .enumerate()
        {
            point(&mut sk, e(n as u128), x, y);
        }
        for (l, a, b) in [(0x10, 0, 1), (0x11, 1, 2), (0x12, 2, 3), (0x13, 3, 0)] {
            sk.add_entity(SketchEntity::line(e(l), e(a), e(b), false))
                .expect("line");
        }
    }
    for (i, (x, y, r)) in circles.iter().enumerate() {
        let i = i as u128 * 2;
        point(&mut sk, e(0x20 + i), *x, *y);
        sk.add_entity(SketchEntity::circle(e(0x21 + i), e(0x20 + i), *r, false).expect("radius"))
            .expect("circle");
    }
    sk
}

fn extrude_record(
    rec: u128,
    sketch: SketchId,
    expr: &str,
    target: Option<BodyId>,
) -> OperationRecord {
    let op = Operation::Known(KnownOperation::Extrude(ExtrudeParams {
        profile: Some(SketchRegionRef {
            sketch,
            region: RegionId::new(""),
            region_identity_version: None,
            region_anchor: None,
            extra: Default::default(),
        }),
        // The cached value is deliberately absurd: only the variable may build.
        distance: Scalar::with_expr(999.0, expr),
        draft_angle_deg: Scalar::new(0.0),
        mode: ExtrudeMode::Blind,
        boolean_mode: target.map_or(BooleanMode::NewBody, |_| BooleanMode::Add),
        target_body: target,
        target_face: None,
        two_directions: false,
        mode2: ExtrudeMode::Blind,
        distance2: Scalar::new(0.0),
        target_face2: None,
        extra: Default::default(),
    }));
    OperationRecord::new(RecordId(Uuid::from_u128(rec)), 0, "Extrude", op)
}

async fn open_blank(app_state: &AppState) {
    let (engine, meshes, solver) = app_state.make_backend();
    let mut rt = DocumentRuntime::new_blank(engine, meshes, solver);
    // The fenced geometry reads (`attachment_from_pick`) need a bound provider.
    rt.bind_geometry_read_query(app_state.element_query())
        .expect("bind the read provider");
    *app_state.runtime.lock().await = Some(rt);
}

/// width = 40, thickness = 10, post_h = width / 2 (derived) -> the Standard bracket.
/// `base` must differ per test: a NewBody's id is its record's uuid.
async fn author_bracket(app_state: &AppState, base: u128) -> BodyId {
    let mut guard = app_state.runtime.lock().await;
    let rt = guard.as_mut().unwrap();
    for variable in [
        var("width", 40.0),
        var("thickness", 10.0),
        Variable {
            id: VariableId::new(),
            name: "post_h".to_string(),
            value: Scalar::with_expr(20.0, "width / 2"),
            unit: Unit::Mm,
        },
    ] {
        rt.apply(EditCommand::AddVariable { variable })
            .expect("AddVariable");
    }
    let plate = SketchId(Uuid::from_u128(base + 2));
    add_op(
        rt,
        sketch_record(
            base,
            &circle_sketch(plate, base + 0x1000, &[(0.0, 0.0, 3.0)], true),
        ),
    );
    add_op(rt, extrude_record(base + 1, plate, "thickness", None));
    let report = regen_all(rt).await;
    let body = published(&report, "plate").bodies[0].body;

    let post = SketchId(Uuid::from_u128(base + 0x12));
    add_op(
        rt,
        sketch_record(
            base + 0x10,
            &circle_sketch(post, base + 0x2000, &[(10.0, 0.0, 4.0)], false),
        ),
    );
    add_op(rt, extrude_record(base + 0x11, post, "post_h", Some(body)));
    let report = regen_all(rt).await;
    let snap = published(&report, "plate + post");
    assert_eq!(snap.bodies.len(), 1, "the post fuses onto the plate");
    snap.bodies[0].body
}

/// Closed-form volume of the bracket at (`width`, `thickness`).
fn bracket_volume(width: f64, thickness: f64) -> f64 {
    let pi = std::f64::consts::PI;
    (400.0 - 17.0 * pi) * thickness + 16.0 * pi * (width / 2.0)
}

fn free(key: &str, value: f64) -> (String, FreeParameterInput) {
    (
        key.to_string(),
        FreeParameterInput {
            key: key.to_string(),
            value,
        },
    )
}

fn config(width: f64, thickness: f64) -> BTreeMap<String, ComponentParamValue> {
    BTreeMap::from([
        ("width".to_string(), ComponentParamValue::Number(width)),
        (
            "thickness".to_string(),
            ComponentParamValue::Number(thickness),
        ),
    ])
}

fn bracket_spec(
    attachments: BTreeMap<String, onecad_app::library::AttachmentSpecInput>,
) -> NewComponentSpec {
    NewComponentSpec {
        id: "acme.bracket".into(),
        version: "1.0.0".into(),
        name: "Bracket".into(),
        category: vec!["mine".into()],
        attachments,
        parameters: [free("width", 40.0), free("thickness", 10.0)]
            .into_iter()
            .collect(),
        configurations: [
            ("Small".to_string(), config(20.0, 6.0)),
            ("Standard".to_string(), config(40.0, 10.0)),
            ("Heavy".to_string(), config(60.0, 16.0)),
        ]
        .into_iter()
        .collect(),
        default_configuration: Some("Standard".into()),
        ..Default::default()
    }
}

/// Finds the first topo key `prefix:1..=24` whose classification satisfies `want`.
async fn find_pick(
    wm: &WorkerManager,
    body: BodyId,
    prefix: &str,
    want: impl Fn(&onecad_app::dto::ClassifyElementDto) -> bool,
) -> String {
    for i in 1..=24 {
        let key = format!("{prefix}:{i}");
        if let Ok(Some(dto)) = ElementQuery::classify_element_by_topo_key(wm, body, &key).await {
            if want(&dto) {
                return key;
            }
        }
    }
    panic!("no {prefix}:1..=24 pick on {body:?} matched");
}

async fn body_extent(rt: &mut DocumentRuntime, body: BodyId) -> ([f32; 3], [f32; 3]) {
    let blob = rt
        .get_mesh(body, Lod::Coarse, None)
        .await
        .expect("mesh the body");
    let view = onecad_protocol::mesh::validate_mesh_blob(&blob).expect("MESH1 validates");
    (view.bbox_min, view.bbox_max)
}

async fn fence(state: &AppState) -> onecad_app::dto::GeometryReadFenceDto {
    let ctx = onecad_app::api::preview_context(state)
        .await
        .expect("preview context");
    onecad_app::dto::GeometryReadFenceDto {
        document_id: ctx.document_id,
        runtime_session: ctx.runtime_session,
        snapshot_id: ctx.snapshot_id.0,
    }
}

fn near(a: f64, b: f64, tol: f64, what: &str) {
    assert!((a - b).abs() <= tol, "{what}: {a} vs {b}");
}

// ─────────────────────────────────────────────────────────────────────────────
// 1. Author -> configurations -> place a variant directly -> reopen library-free
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_bracket_saves_with_configurations_and_places_as_a_chosen_variant() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: real worker binary not found (set ONECAD_WORKER_PATH)");
        return;
    };
    let library_dir = tempfile::tempdir().expect("tempdir");
    let doc_dir = tempfile::tempdir().expect("tempdir");
    let wm = spawn_worker(bin).await;
    let state = app_state_over(&wm);
    open_blank(&state).await;
    let body = author_bracket(&state, BRACKET_BASE).await;
    near(
        exact_volume(&wm, body).await,
        bracket_volume(40.0, 10.0),
        1e-4,
        "the Standard bracket volume (post_h derived from width)",
    );

    let fence = fence(&state).await;
    // Two attachments from picks: the post's top plane face and the bore's rim.
    let top = find_pick(&wm, body, "f", |d| {
        d.surface_type == "plane"
            && d.frame.as_ref().is_some_and(|f| {
                f.normal.is_some_and(|n| n[2] > 0.99) && (f.origin[2] - 20.0).abs() < 1e-6
            })
    })
    .await;
    let rim = find_pick(&wm, body, "e", |d| {
        d.curve_type == "circle" && d.frame.as_ref().is_some_and(|f| f.radius == Some(3.0))
    })
    .await;
    let top_att = attachment_from_pick(&state, body.to_string(), top, "top".into(), &fence)
        .await
        .expect("plane face attachment");
    assert_eq!(top_att.on, "face:top");
    assert_eq!(top_att.accepts, ["plane"]);
    let f = top_att.frame.expect("frame");
    assert_eq!(
        (f.z, f.x),
        ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
        "outward +Z, x = world X"
    );
    near(f.origin[2], 20.0, 1e-6, "top face origin height");
    let rim_att = attachment_from_pick(&state, body.to_string(), rim, "bore".into(), &fence)
        .await
        .expect("circular edge attachment");
    assert_eq!(rim_att.on, "edge:bore");
    assert_eq!(rim_att.accepts, ["cylinder", "hole", "circularEdge"]);
    let rf = rim_att.frame.expect("frame");
    near(rf.z[2].abs(), 1.0, 1e-9, "the rim axis is Z");
    near(rf.origin[0], 0.0, 1e-6, "rim centre x");
    near(rf.origin[1], 0.0, 1e-6, "rim centre y");

    // The bore wall is a cylinder (a hole); the axis stays as measured.
    let wall = find_pick(&wm, body, "f", |d| {
        d.surface_type == "cylinder" && d.frame.as_ref().is_some_and(|f| f.radius == Some(3.0))
    })
    .await;
    let wall_att = attachment_from_pick(&state, body.to_string(), wall, "bore_wall".into(), &fence)
        .await
        .expect("cylinder attachment");
    assert_eq!(wall_att.on, "cylinder:bore_wall");
    assert_eq!(wall_att.accepts, ["cylinder", "hole", "circularEdge"]);
    near(
        wall_att.frame.expect("frame").z[2].abs(),
        1.0,
        1e-9,
        "bore axis is Z",
    );

    // A straight edge is not something a part can seat on.
    let line = find_pick(&wm, body, "e", |d| d.curve_type == "line").await;
    let err = attachment_from_pick(&state, body.to_string(), line, "edge".into(), &fence)
        .await
        .expect_err("a line edge is refused")
        .to_string();
    assert!(
        err.contains("attachments need a planar face, a cylinder or a circular edge"),
        "{err}"
    );

    let saved = save_as_component_at(
        library_dir.path(),
        &state.runtime,
        state.exporter(),
        body.to_string(),
        bracket_spec(
            [("top".to_string(), top_att), ("bore".to_string(), rim_att)]
                .into_iter()
                .collect(),
        ),
        None,
        false,
    )
    .await
    .expect("save the parametric bracket");
    assert_eq!(saved.default_configuration.as_deref(), Some("Standard"));
    reindex_library_at(library_dir.path()).expect("reindex");
    let listed = list_library_components_at(library_dir.path()).expect("list");
    let names: Vec<_> = listed[0].configurations.keys().cloned().collect();
    assert_eq!(names, ["Heavy", "Small", "Standard"]);
    assert_eq!(listed[0].attachments.len(), 2);
    assert_eq!(listed[0].default_configuration.as_deref(), Some("Standard"));

    // Place Heavy DIRECTLY into a fresh document: one undo step, Heavy geometry.
    open_blank(&state).await;
    let prepared = prepare_place_component_at(
        library_dir.path(),
        &state,
        PlaceComponentRequest {
            component_id: "acme.bracket".into(),
            component_version: "1.0.0".into(),
            translate: [0.0; 3],
            rotate: None,
            params: Default::default(),
            mate: None,
            configuration: Some("Heavy".into()),
        },
    )
    .await
    .expect("prepare the Heavy placement");
    stage_prepared_blob(&state, &prepared).await.expect("stage");
    let (record, placed_volume, (lo, hi), undo_before, undo_after) = {
        let mut guard = state.runtime.lock().await;
        let rt = guard.as_mut().unwrap();
        let undo_before = rt.undo_depth();
        rt.apply(prepared.command).expect("apply the placement");
        let undo_after = rt.undo_depth();
        let record =
            std::str::FromStr::from_str(&rt.projection().features[0].id).expect("record id");
        let report = regen_all(rt).await;
        let snap = published(&report, "Heavy placement");
        assert_eq!(snap.bodies.len(), 1);
        let placed = snap.bodies[0].body;
        (
            record,
            exact_volume(&wm, placed).await,
            body_extent(rt, placed).await,
            undo_before,
            undo_after,
        )
    };
    assert_eq!(undo_after, undo_before + 1, "the variant is ONE undo step");
    near(
        placed_volume,
        bracket_volume(60.0, 16.0),
        1e-4,
        "Heavy volume",
    );
    near(f64::from(hi[2]), 30.0, 1e-3, "Heavy post top = width / 2");
    near(f64::from(lo[2]), 0.0, 1e-3, "Heavy base");

    let guard = state.runtime.lock().await;
    let raw = guard.as_ref().unwrap().operation_params(record).unwrap();
    let params: PlaceComponentParams = serde_json::from_value(raw).expect("PlaceComponent");
    drop(guard);
    assert_eq!(
        params.params,
        config(60.0, 16.0),
        "record params = Heavy values"
    );
    let onecad_core::document::record::ComponentSourceRef::Document {
        sha256,
        params: source_params,
        ..
    } = &params.source
    else {
        panic!("a document source, got {:?}", params.source);
    };
    assert_eq!(source_params, &config(60.0, 16.0));
    let default_sha = {
        let library = onecad_library::Library::open(library_dir.path()).expect("library");
        let resolved = library
            .resolve_source(&onecad_library::resolve::ResolveRequest {
                component_id: "acme.bracket",
                component_version: "1.0.0",
                component_revision: &params.component_revision,
            })
            .expect("resolve");
        match resolved {
            onecad_library::resolve::ResolvedSource::Document { blob, .. } => blob.sha256,
            other => panic!("expected a document, got {other:?}"),
        }
    };
    assert_ne!(
        sha256.as_str(),
        default_sha,
        "the variant is a re-baked blob"
    );

    // Save, delete the library, reopen on a fresh worker: the variant stands.
    let path = doc_dir.path().join("placed.onecad");
    state
        .runtime
        .lock()
        .await
        .as_mut()
        .unwrap()
        .save(&path, save_meta())
        .expect("save");
    std::fs::remove_dir_all(library_dir.path()).expect("delete the library");
    wm.shutdown().await;
    wm.retire();
    let wm2 = spawn_worker(real_worker().expect("worker binary")).await;
    let mut reopened = open_over(&wm2, &path);
    let report = regen_all(&mut reopened).await;
    let snap = published(&report, "reopen with no library");
    near(
        exact_volume(&wm2, snap.bodies[0].body).await,
        bracket_volume(60.0, 16.0),
        1e-4,
        "reopened Heavy volume",
    );
    wm2.shutdown().await;
    wm2.retire();
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. Static saves, refusals
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn static_saves_and_invalid_authoring_are_refused_explicitly() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: real worker binary not found (set ONECAD_WORKER_PATH)");
        return;
    };
    let library_dir = tempfile::tempdir().expect("tempdir");
    let wm = spawn_worker(bin).await;
    let state = app_state_over(&wm);
    open_blank(&state).await;
    let body = author_bracket(&state, REFUSAL_BASE).await;
    let save = |spec: NewComponentSpec| {
        save_as_component_at(
            library_dir.path(),
            &state.runtime,
            state.exporter(),
            body.to_string(),
            spec,
            None,
            false,
        )
    };

    // An invalid configuration value is refused AT SAVE, naming the configuration.
    let mut bad = bracket_spec(Default::default());
    bad.configurations.insert(
        "Wrong".into(),
        BTreeMap::from([(
            "width".to_string(),
            ComponentParamValue::Text("wide".into()),
        )]),
    );
    let err = save(bad).await.expect_err("a text width").to_string();
    assert!(err.contains("configuration `Wrong`"), "{err}");

    // A parameter bound to a variable the document does not declare (a renamed
    // variable) is refused.
    let mut renamed = bracket_spec(Default::default());
    renamed
        .parameters
        .insert("girth".into(), free("girth", 1.0).1);
    let err = save(renamed)
        .await
        .expect_err("unknown variable")
        .to_string();
    assert!(err.contains("girth"), "{err}");

    // static_save + free parameters is a contradiction, not a silent drop.
    let mut contradictory = bracket_spec(Default::default());
    contradictory.static_save = true;
    let err = save(contradictory)
        .await
        .expect_err("static + params")
        .to_string();
    assert!(err.contains("static save"), "{err}");

    // A static save has no free parameters and no configurations...
    let fixed = save(NewComponentSpec {
        id: "acme.fixed".into(),
        version: "1.0.0".into(),
        name: "Fixed".into(),
        static_save: true,
        ..Default::default()
    })
    .await
    .expect("static save");
    assert!(fixed.parameters.is_empty() && fixed.configurations.is_empty());
    reindex_library_at(library_dir.path()).expect("reindex");

    // ...so placing it with params (or a configuration) is refused.
    open_blank(&state).await;
    let place = |params: BTreeMap<String, ComponentParamValue>, configuration: Option<String>| {
        prepare_place_component_at(
            library_dir.path(),
            &state,
            PlaceComponentRequest {
                component_id: "acme.fixed".into(),
                component_version: "1.0.0".into(),
                translate: [0.0; 3],
                rotate: None,
                params,
                mate: None,
                configuration,
            },
        )
    };
    assert!(place(config(10.0, 5.0), None).await.is_err());
    assert!(place(Default::default(), Some("Heavy".into()))
        .await
        .is_err());
    // An embedded solid with a configuration table is refused at placement: a
    // baked solid has no free parameters to set.
    {
        let package = onecad_library::ComponentPackage {
            identity: onecad_library::package::Identity {
                id: "acme.slug".into(),
                version: "1.0.0".into(),
                revision: format!("sha256:{}", "0".repeat(64)),
            },
            metadata: onecad_library::package::Metadata {
                name: "Slug".into(),
                unit: "mm".into(),
                ..Default::default()
            },
            source: onecad_library::package::SourceSpec::Generator {
                generator: "ignored".into(),
                generator_version: 0,
            },
            parameters: BTreeMap::from([(
                "size".to_string(),
                onecad_library::package::ParameterSpec::free_variable("size", 1.0),
            )]),
            attachments: Default::default(),
            configurations: BTreeMap::from([(
                "Big".to_string(),
                BTreeMap::from([("size".to_string(), toml::Value::Float(2.0))]),
            )]),
        };
        let mut library = onecad_library::Library::open(library_dir.path()).expect("library");
        library
            .save_embedded_component(onecad_library::EmbeddedComponentRequest {
                package,
                kind: onecad_library::BlobComponentKind::Embedded,
                geometry: b"not really a solid".to_vec(),
                geometry_codec: "step".into(),
                geometry_format: None,
                preview_png: None,
            })
            .expect("save the embedded package");
        let err = prepare_place_component_at(
            library_dir.path(),
            &state,
            PlaceComponentRequest {
                component_id: "acme.slug".into(),
                component_version: "1.0.0".into(),
                translate: [0.0; 3],
                rotate: None,
                params: Default::default(),
                mate: None,
                configuration: Some("Big".into()),
            },
        )
        .await
        .map(|_| ())
        .expect_err("an embedded solid cannot take a configuration");
        assert!(err.to_string().contains("baked geometry"), "{err}");
    }
    let plain = place(Default::default(), None)
        .await
        .expect("a plain place");
    assert!(plain.blob.is_some(), "the fixed solid stages as before");

    wm.shutdown().await;
    wm.retire();
}

// ─────────────────────────────────────────────────────────────────────────────
// 3. Variants never seat by the default bake's frame; authoring reads the document
// ─────────────────────────────────────────────────────────────────────────────

fn mate_input(attachment: &str, body: BodyId) -> onecad_app::dto::PlaceComponentMateInput {
    onecad_app::dto::PlaceComponentMateInput {
        self_attachment: attachment.into(),
        target_body_id: format!("body_{body}"),
        target_topo_key: "f:1".into(),
        target_element_id: Some("el_target".into()),
        target_kind: "face".into(),
        kind: onecad_core::document::record::MateKind::Coincident,
        flipped: false,
        anchor_world_point: [0.0; 3],
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_non_default_variant_is_never_mated_and_authoring_is_checked_against_the_document() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: real worker binary not found (set ONECAD_WORKER_PATH)");
        return;
    };
    let library_dir = tempfile::tempdir().expect("tempdir");
    let wm = spawn_worker(bin).await;
    let state = app_state_over(&wm);
    open_blank(&state).await;
    let body = author_bracket(&state, VARIANT_BASE).await;
    let fence = fence(&state).await;
    let top = find_pick(&wm, body, "f", |d| {
        d.surface_type == "plane"
            && d.frame
                .as_ref()
                .is_some_and(|f| f.normal.is_some_and(|n| n[2] > 0.99))
    })
    .await;

    // A pick fenced to a head that is not the document's is refused, not read.
    let mut stale = fence.clone();
    stale.snapshot_id += 1;
    let err = attachment_from_pick(&state, body.to_string(), top.clone(), "top".into(), &stale)
        .await
        .expect_err("a stale fence");
    assert!(
        matches!(err, onecad_app::error::ApiError::StalePreview(_)),
        "{err}"
    );
    let top_att = attachment_from_pick(&state, body.to_string(), top, "top".into(), &fence)
        .await
        .expect("fresh fence");

    // A derived variable cannot be a free parameter: its inputs can.
    let mut derived = bracket_spec(Default::default());
    derived
        .parameters
        .insert("post_h".into(), free("post_h", 20.0).1);
    let err = save_as_component_at(
        library_dir.path(),
        &state.runtime,
        state.exporter(),
        body.to_string(),
        derived,
        None,
        false,
    )
    .await
    .expect_err("a derived variable")
    .to_string();
    assert!(
        err.contains("derived from an expression; expose its inputs instead"),
        "{err}"
    );

    // The declared default is the DOCUMENT's value; the client's is ignored.
    let mut spec = bracket_spec([("top".to_string(), top_att)].into_iter().collect());
    spec.parameters
        .insert("width".into(), free("width", 999.0).1);
    let saved = save_as_component_at(
        library_dir.path(),
        &state.runtime,
        state.exporter(),
        body.to_string(),
        spec,
        None,
        false,
    )
    .await
    .expect("save");
    assert_eq!(saved.parameters["width"].number(), Some(40.0));
    reindex_library_at(library_dir.path()).expect("reindex");

    // Placing a non-default variant mated by the default bake's frame is refused...
    open_blank(&state).await;
    let request = |configuration: Option<&str>| PlaceComponentRequest {
        component_id: "acme.bracket".into(),
        component_version: "1.0.0".into(),
        translate: [0.0; 3],
        rotate: None,
        params: Default::default(),
        mate: Some(mate_input("top", body)),
        configuration: configuration.map(String::from),
    };
    let err = prepare_place_component_at(library_dir.path(), &state, request(Some("Heavy")))
        .await
        .map(|_| ())
        .expect_err("a mated variant")
        .to_string();
    assert!(err.contains("Mating a non-default variant"), "{err}");
    // ...while the default variant passes that check (it then names the unmated reason it
    // cannot be promoted, which is a different refusal).
    let outcome =
        prepare_place_component_at(library_dir.path(), &state, request(Some("Standard"))).await;
    assert!(
        outcome
            .map(|_| ())
            .err()
            .is_none_or(|e| !e.to_string().contains("non-default variant")),
        "the default variant is not refused as a variant"
    );

    // ...and so is re-baking an instance that is already mated.
    let prepared = prepare_place_component_at(
        library_dir.path(),
        &state,
        PlaceComponentRequest {
            mate: None,
            ..request(None)
        },
    )
    .await
    .expect("unmated default");
    stage_prepared_blob(&state, &prepared).await.expect("stage");
    let record_id = {
        let mut guard = state.runtime.lock().await;
        let rt = guard.as_mut().unwrap();
        rt.apply(prepared.command).expect("apply");
        let id = rt.projection().features[0].id.clone();
        let record: RecordId = std::str::FromStr::from_str(&id).expect("record id");
        let mut params: PlaceComponentParams =
            serde_json::from_value(rt.operation_params(record).unwrap()).unwrap();
        let frame = onecad_core::document::record::MateFrame {
            origin: Vec3::new_unchecked(0.0, 0.0, 20.0),
            z: Vec3::new_unchecked(0.0, 0.0, 1.0),
            x: Vec3::new_unchecked(1.0, 0.0, 0.0),
        };
        params.mate = Some(onecad_app::library::component_mate_from_input(
            &mate_input("top", body),
            body,
            onecad_core::ids::ElementId::new("el_target"),
            Some(frame),
        ));
        rt.apply(EditCommand::UpdateOperationParams {
            record,
            op: Operation::Known(KnownOperation::PlaceComponent(params)),
        })
        .expect("attach a frozen mate");
        id
    };
    let err = onecad_app::library::set_component_params_at(
        library_dir.path(),
        &state,
        record_id,
        config(60.0, 16.0),
    )
    .await
    .expect_err("a mated instance cannot become a variant")
    .to_string();
    assert!(err.contains("Mating a non-default variant"), "{err}");

    wm.shutdown().await;
    wm.retire();
}
