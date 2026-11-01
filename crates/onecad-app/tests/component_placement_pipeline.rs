//! Component placement pipeline against the REAL C++ OCCT worker: the app/core
//! services the native placement flow arms, previews and commits through.
//!
//! * `a_mated_preview_equals_its_commit` (P1.0 spike) — a draft with a mate and
//!   an intentionally WRONG approximate translate previews, then commits, to the
//!   same bbox, with the screw axis on the hole axis.
//! * `a_generator_draft_with_free_params_previews_and_commits_the_sized_part` —
//!   `with_free_params(M8, 25)` previews and commits an M8x25 whose bbox matches
//!   the ISO 4762 table.
//! * `a_copy_placement_is_one_undo_step_and_survives_reopen` — Copy = detached
//!   twin in a single `AddOperation`.
//! * `restage_*` — blob re-staging contract.
//! * `a_cancelled_draft_leaves_history_and_the_saved_file_untouched`.
//!
//! Gated on `ONECAD_WORKER_PATH`; `ONECAD_REQUIRE_WORKER=1` makes a missing
//! binary a hard failure.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use uuid::Uuid;

use onecad_core::document::record::{
    ComponentParamValue, KnownOperation, MateKind, Operation, OperationRecord, PlaceComponentParams,
};
use onecad_core::document::variables::Scalar;
use onecad_core::edit::EditCommand;
use onecad_core::ids::{BodyId, RecordId, SketchId};
use onecad_core::io::container::SaveMeta;
use onecad_core::math::Vec3;
use onecad_core::regen::{CancelToken, GeometryEngine, ModelSnapshot, Outcome, RegenRequest};
use onecad_protocol::mesh::{MeshLimits, MeshView};

use onecad_app::document_runtime::DocumentRuntime;
use onecad_app::dto::PlaceComponentMateInput;
use onecad_app::error::ApiError;
use onecad_app::host::{AppEvent, DialogPort, EventSink, FileDialog, HostContext, HostPaths};
use onecad_app::library::{
    copy_placement_commands, prepare_component_draft_at, reindex_library_at,
    restage_component_blob_at, save_as_component_at, stage_mate_target, NewComponentSpec,
    PlaceComponentRequest,
};
use onecad_app::state::AppState;
use onecad_app::worker::manager::SupervisorConfig;
use onecad_app::worker::wire::body_id_wire;
use onecad_app::worker::{
    resolve_worker_path, CircuitControl, ElementQuery, FaceBoundaryProjection, MeshProvider,
    PreviewEngine, SolverEngine, StepImport, WorkerManager, WorkerReadiness,
};
use onecad_app::{api, dto::PreviewResultDto};
use onecad_library::tables::Iso4762Table;

const SCREW: &str = "onecad.std.iso4762";

// ─────────────────────────────────────────────────────────────────────────────
// Harness
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

fn app_state_over(wm: &WorkerManager) -> AppState {
    let wm = wm.clone();
    AppState::new(Arc::new(move || {
        let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
        let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
        let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
        let exporter: Arc<dyn onecad_app::export::GeometryExporter> = Arc::new(wm.clone());
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

#[derive(Default)]
struct Events(Mutex<Vec<AppEvent>>);
impl EventSink for Events {
    fn emit(&self, event: AppEvent) -> Result<(), String> {
        self.0.lock().unwrap().push(event);
        Ok(())
    }
}

struct NoDialogs;
#[async_trait]
impl DialogPort for NoDialogs {
    async fn pick(&self, _: FileDialog) -> Vec<String> {
        panic!("no dialog expected")
    }
}

fn host() -> HostContext {
    HostContext {
        paths: HostPaths {
            app_data: None,
            app_config: None,
        },
        events: Arc::new(Events::default()),
        dialogs: Arc::new(NoDialogs),
    }
}

async fn open_blank(state: &AppState) {
    let (engine, meshes, solver) = state.make_backend();
    *state.runtime.lock().await = Some(DocumentRuntime::new_blank(engine, meshes, solver));
}

fn open_over(wm: &WorkerManager, path: &Path) -> DocumentRuntime {
    let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
    DocumentRuntime::open(path, engine, meshes, solver).expect("reopen the saved container")
}

fn save_meta() -> SaveMeta {
    SaveMeta {
        app_version: "0.1.0-test".into(),
        occt_fingerprint: Some("occt-8.0.1".into()),
        created: "2026-11-01T00:00:00Z".into(),
        modified: "2026-11-01T00:00:00Z".into(),
    }
}

/// Regens the open document to its end and returns the published snapshot.
async fn regen(state: &AppState) -> Arc<ModelSnapshot> {
    let mut guard = state.runtime.lock().await;
    let rt = guard.as_mut().expect("open document");
    let report = rt
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    match report.outcome {
        Outcome::Published(s) => s,
        other => panic!("expected Published, got {other:?}"),
    }
}

async fn exact_volume(wm: &WorkerManager, body: BodyId) -> f64 {
    ElementQuery::query_mass_properties(wm, body, "b".into())
        .await
        .expect("QueryMassProperties")
        .volume
}

fn mesh_bbox(blob: &[u8]) -> ([f64; 3], [f64; 3]) {
    let mesh = MeshView::decode(blob, MeshLimits::default()).expect("MESH1");
    let (mut min, mut max) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for i in 0..mesh.header().vertex_count as usize {
        let p = mesh.position(i).expect("vertex");
        for a in 0..3 {
            min[a] = min[a].min(f64::from(p[a]));
            max[a] = max[a].max(f64::from(p[a]));
        }
    }
    (min, max)
}

async fn committed_bbox(state: &AppState, body: BodyId) -> ([f64; 3], [f64; 3]) {
    let blob = api::get_mesh(state, body.to_string(), "fine".into(), None, None)
        .await
        .expect("committed mesh");
    mesh_bbox(&blob)
}

fn assert_bbox_eq(a: ([f64; 3], [f64; 3]), b: ([f64; 3], [f64; 3]), what: &str) {
    for axis in 0..3 {
        assert!(
            (a.0[axis] - b.0[axis]).abs() < 1e-6 && (a.1[axis] - b.1[axis]).abs() < 1e-6,
            "{what}: axis {axis} {a:?} vs {b:?}"
        );
    }
}

fn dims(b: ([f64; 3], [f64; 3])) -> [f64; 3] {
    [b.1[0] - b.0[0], b.1[1] - b.0[1], b.1[2] - b.0[2]]
}

async fn fence(state: &AppState) -> onecad_app::dto::GeometryReadFenceDto {
    let scope = api::preview_context(state).await.expect("preview context");
    onecad_app::dto::GeometryReadFenceDto {
        document_id: scope.document_id,
        runtime_session: scope.runtime_session,
        snapshot_id: scope.snapshot_id.0,
    }
}

fn mated_request(
    plate: BodyId,
    key: &str,
    element: String,
    anchor: [f64; 3],
    kind: MateKind,
) -> PlaceComponentRequest {
    let mut req = request([7.0, -4.0, 11.0]);
    req.mate = Some(PlaceComponentMateInput {
        self_attachment: "shank_axis".into(),
        target_body_id: body_id_wire(plate),
        target_topo_key: key.into(),
        target_element_id: Some(element),
        target_kind: "face".into(),
        kind,
        flipped: false,
        anchor_world_point: anchor,
    });
    req
}

async fn persisted_elements(state: &AppState) -> usize {
    let guard = state.runtime.lock().await;
    guard.as_ref().unwrap().persisted_element_count()
}

fn seeded_library() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    onecad_app::library_seed::seed_library(dir.path()).expect("seed");
    reindex_library_at(dir.path()).expect("reindex");
    dir
}

// ─────────────────────────────────────────────────────────────────────────────
// Document builders (copied minimal from component_ops.rs)
// ─────────────────────────────────────────────────────────────────────────────

fn bored_plate_sketch(sid: SketchId, base: u128) -> onecad_core::sketch::Sketch {
    use onecad_core::ids::EntityId;
    use onecad_core::math::Vec2;
    use onecad_core::sketch::{Sketch, SketchEntity, WorldPlane};

    let e = |n: u128| EntityId(Uuid::from_u128(base + n));
    let mut sk = Sketch::on_world_plane(sid, "Bored plate", WorldPlane::XY);
    let point = |sk: &mut Sketch, id: EntityId, x: f64, y: f64| {
        sk.add_entity(SketchEntity::point(
            id,
            Vec2::new_unchecked(x, y),
            false,
            false,
        ))
        .expect("point");
    };
    point(&mut sk, e(0), -10.0, -10.0);
    point(&mut sk, e(1), 10.0, -10.0);
    point(&mut sk, e(2), 10.0, 10.0);
    point(&mut sk, e(3), -10.0, 10.0);
    for (l, a, b) in [(0x10, 0, 1), (0x11, 1, 2), (0x12, 2, 3), (0x13, 3, 0)] {
        sk.add_entity(SketchEntity::line(e(l), e(a), e(b), false))
            .expect("line");
    }
    point(&mut sk, e(0x20), 0.0, 0.0);
    sk.add_entity(SketchEntity::circle(e(0x21), e(0x20), 3.0, false).expect("finite radius"))
        .expect("bore circle");
    sk
}

fn sketch_record(rec: u128, sk: &onecad_core::sketch::Sketch) -> OperationRecord {
    use onecad_core::document::record::{PlaneKind, SketchOpParams, SketchPlaneRef};
    let (_plane, entities, constraints) = onecad_app::worker::wire::sketch_wire(sk);
    let op = Operation::Known(KnownOperation::Sketch(SketchOpParams {
        sketch: sk.id,
        plane: SketchPlaneRef {
            kind: PlaneKind::Xy,
            origin: Vec3::new_unchecked(0.0, 0.0, 0.0),
            x_axis: Vec3::new_unchecked(0.0, 1.0, 0.0),
            y_axis: Vec3::new_unchecked(-1.0, 0.0, 0.0),
            normal: Vec3::new_unchecked(0.0, 0.0, 1.0),
            extra: Default::default(),
        },
        entities: entities.as_array().cloned().unwrap_or_default(),
        constraints: constraints.as_array().cloned().unwrap_or_default(),
        host_face: None,
        extra: Default::default(),
    }));
    OperationRecord::new(RecordId(Uuid::from_u128(rec)), 0, "Sketch", op)
}

fn extrude_record(rec: u128, sketch: SketchId, distance: f64) -> OperationRecord {
    use onecad_core::document::record::{BooleanMode, ExtrudeMode, ExtrudeParams};
    use onecad_core::document::variables::Scalar;
    let op = Operation::Known(KnownOperation::Extrude(ExtrudeParams {
        profile: Some(onecad_core::document::refs::SketchRegionRef {
            sketch,
            region: onecad_core::ids::RegionId::new(""),
            region_identity_version: None,
            region_anchor: None,
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

/// A 20x20x5 plate with a through bore of radius 3 at the origin. `base` must
/// differ per test (a NewBody's id is its record's uuid).
async fn author_bored_plate(state: &AppState, base: u128) -> (BodyId, Arc<ModelSnapshot>) {
    {
        let mut guard = state.runtime.lock().await;
        let rt = guard.as_mut().expect("open document");
        let sid = SketchId(Uuid::from_u128(base + 2));
        for record in [
            sketch_record(base, &bored_plate_sketch(sid, base + 0x1000)),
            extrude_record(base + 1, sid, 5.0),
        ] {
            rt.apply(EditCommand::AddOperation {
                record,
                at_cursor: true,
            })
            .expect("AddOperation");
        }
    }
    let snap = regen(state).await;
    assert_eq!(snap.bodies.len(), 1, "one plate");
    (snap.bodies[0].body, snap)
}

/// `(topoKey, axis origin, axis direction)` of the cylindrical face of `radius`.
async fn find_cylinder(
    wm: &WorkerManager,
    body: BodyId,
    radius: f64,
) -> (String, [f64; 3], [f64; 3]) {
    for i in 1..=8 {
        let key = format!("f:{i}");
        let Some(dto) = ElementQuery::classify_element_by_topo_key(wm, body, &key)
            .await
            .expect("ClassifyElement")
        else {
            continue;
        };
        if dto.surface_type != "cylinder" {
            continue;
        }
        let Some(frame) = &dto.frame else { continue };
        if (frame.radius.unwrap_or(0.0) - radius).abs() < 1e-6 {
            return (
                key,
                frame.origin,
                frame.axis.expect("a cylinder face has an axis"),
            );
        }
    }
    panic!("no cylindrical face of radius {radius} on {body:?} within f:1..=8");
}

fn place_record(draft: PlaceComponentParams) -> OperationRecord {
    OperationRecord::new(
        RecordId::new(),
        0,
        "Place Component",
        Operation::Known(KnownOperation::PlaceComponent(draft)),
    )
}

fn add(record: OperationRecord) -> EditCommand {
    EditCommand::AddOperation {
        record,
        at_cursor: true,
    }
}

fn request(translate: [f64; 3]) -> PlaceComponentRequest {
    PlaceComponentRequest {
        component_id: SCREW.into(),
        component_version: "1.0.0".into(),
        translate,
        rotate: None,
        params: BTreeMap::new(),
        mate: None,
        configuration: None,
    }
}

async fn preview(state: &AppState, draft: &PlaceComponentParams, id: RecordId) -> PreviewResultDto {
    let before = api::get_projection(state).await.unwrap();
    let scope = api::preview_context(state).await.expect("preview context");
    let result = api::preview_operation(
        state,
        Operation::Known(KnownOperation::PlaceComponent(draft.clone())),
        id,
        None,
        Some(scope.snapshot_id),
        onecad_core::regen::Lod::Fine,
    )
    .await
    .expect("preview");
    let after = api::get_projection(state).await.unwrap();
    assert_eq!(before.revision, after.revision, "preview mutates nothing");
    assert_eq!(before.features, after.features);
    assert!(result.needs_repair.is_empty(), "{:?}", result.needs_repair);
    result
}

// ─────────────────────────────────────────────────────────────────────────────
// a. P1.0 spike: mated preview == commit
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_mated_preview_equals_its_commit() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: real worker binary not found (set ONECAD_WORKER_PATH)");
        return;
    };
    let library = seeded_library();
    let wm = spawn_worker(bin).await;
    let state = app_state_over(&wm);
    open_blank(&state).await;
    let (plate, _) = author_bored_plate(&state, 0xD1A100).await;
    let (bore_key, bore_origin, bore_axis) = find_cylinder(&wm, plate, 3.0).await;

    let anchor = [bore_origin[0] + 3.0, bore_origin[1], 2.5];
    let dirty_before = api::get_projection(&state).await.unwrap().dirty;
    let element = stage_mate_target(
        &state,
        &body_id_wire(plate),
        &bore_key,
        anchor,
        &fence(&state).await,
    )
    .await
    .expect("stage the bore face");
    assert_eq!(
        api::get_projection(&state).await.unwrap().dirty,
        dirty_before,
        "staging a hover target never dirties"
    );

    let mut req = request([7.0, -4.0, 11.0]); // intentionally wrong
    req.mate = Some(PlaceComponentMateInput {
        self_attachment: "shank_axis".into(),
        target_body_id: body_id_wire(plate),
        target_topo_key: bore_key,
        target_element_id: Some(element),
        target_kind: "face".into(),
        kind: MateKind::ConcentricAndCoincident,
        flipped: false,
        anchor_world_point: anchor,
    });
    let draft = prepare_component_draft_at(library.path(), &state, req)
        .await
        .expect("arm the draft");
    let record = place_record(draft.clone());

    let previewed = preview(&state, &draft, record.record_id).await;
    assert_eq!(previewed.bodies.len(), 1, "preview mints the screw body");
    assert!(
        previewed.mate_placement.is_some(),
        "a mated preview echoes the resolved placement"
    );
    let preview_bbox = mesh_bbox(&previewed.bodies[0].mesh);

    api::apply_operation(&state, host(), vec![add(record)], None)
        .await
        .expect("commit");
    let snap = regen(&state).await;
    assert_eq!(snap.bodies.len(), 2, "plate + screw");
    let screw = snap.bodies.iter().find(|b| b.body != plate).unwrap().body;
    let commit_bbox = committed_bbox(&state, screw).await;

    eprintln!("[P1.0 spike] preview bbox = {preview_bbox:?}");
    eprintln!("[P1.0 spike] commit  bbox = {commit_bbox:?}");
    assert_bbox_eq(preview_bbox, commit_bbox, "preview vs commit");

    // The screw's shank axis sits on the hole axis (parallel, zero offset).
    let (_, shank_origin, shank_axis) = find_cylinder(&wm, screw, 3.0).await;
    let dot: f64 = (0..3).map(|i| shank_axis[i] * bore_axis[i]).sum();
    assert!(dot.abs() > 1.0 - 1e-9, "axes parallel: {shank_axis:?}");
    let d: [f64; 3] = std::array::from_fn(|i| shank_origin[i] - bore_origin[i]);
    let along: f64 = (0..3).map(|i| d[i] * bore_axis[i]).sum();
    let off: f64 = (0..3)
        .map(|i| (d[i] - along * bore_axis[i]).powi(2))
        .sum::<f64>()
        .sqrt();
    assert!(off < 1e-6, "shank axis is {off} mm off the hole axis");

    // PreviewOp echoes the snapped placement (SCHEMA §7.6 `matePlacement`); a
    // Copy committed at it must land exactly where the linked commit did.
    let resolved = previewed
        .mate_placement
        .clone()
        .expect("mated preview carries matePlacement");
    assert_ne!(
        resolved, draft.placement,
        "the wrong draft placement was re-seated"
    );
    api::undo(&state, host()).await.expect("undo linked commit");
    assert_eq!(regen(&state).await.bodies.len(), 1, "plate only");
    let cmds = copy_placement_commands(place_record(draft.clone()), Some(resolved))
        .expect("copy commands at the resolved placement");
    api::apply_operation(&state, host(), cmds, None)
        .await
        .expect("commit copy");
    let snap = regen(&state).await;
    let copy = snap.bodies.iter().find(|b| b.body != plate).unwrap().body;
    assert_bbox_eq(
        committed_bbox(&state, copy).await,
        commit_bbox,
        "snapped copy vs linked commit",
    );
    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// b. generator draft with free params
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_generator_draft_with_free_params_previews_and_commits_the_sized_part() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: real worker binary not found (set ONECAD_WORKER_PATH)");
        return;
    };
    let library = seeded_library();
    let wm = spawn_worker(bin).await;
    let state = app_state_over(&wm);
    open_blank(&state).await;
    // The preview lane needs a published head to scope against.
    let (plate, _) = author_bored_plate(&state, 0xD1B100).await;

    let armed = prepare_component_draft_at(library.path(), &state, request([30.0, 0.0, 0.0]))
        .await
        .expect("arm");
    let sized: BTreeMap<_, _> = [
        ("thread".to_string(), ComponentParamValue::Text("M8".into())),
        ("length".to_string(), ComponentParamValue::Number(25.0)),
    ]
    .into();
    let draft = armed.with_free_params(sized).expect("free params");
    let record = place_record(draft.clone());

    let previewed = preview(&state, &draft, record.record_id).await;
    assert!(
        previewed.mate_placement.is_none(),
        "an unmated preview echoes no placement"
    );
    let preview_bbox = mesh_bbox(&previewed.bodies[0].mesh);
    api::apply_operation(&state, host(), vec![add(record)], None)
        .await
        .expect("commit");
    let snap = regen(&state).await;
    let screw = snap.bodies.iter().find(|b| b.body != plate).unwrap().body;
    let commit_bbox = committed_bbox(&state, screw).await;
    assert_bbox_eq(preview_bbox, commit_bbox, "M8x25 preview vs commit");

    let row = Iso4762Table::row("M8").expect("M8 row");
    let d = dims(commit_bbox);
    for (got, want, what) in [
        (d[0], row.d2, "head diameter x"),
        (d[1], row.d2, "head diameter y"),
        (d[2], row.k + 25.0, "head height + length"),
    ] {
        assert!((got - want).abs() < 1e-4, "{what}: {got} vs table {want}");
    }
    wm.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// c. Copy placement
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_copy_placement_is_one_undo_step_and_survives_reopen() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: real worker binary not found (set ONECAD_WORKER_PATH)");
        return;
    };
    let library = seeded_library();
    let doc_dir = tempfile::tempdir().expect("tempdir");
    let wm = spawn_worker(bin).await;
    let state = app_state_over(&wm);
    open_blank(&state).await;
    let (plate, _) = author_bored_plate(&state, 0xD1C100).await;
    let before = api::get_projection(&state).await.unwrap();

    let draft = prepare_component_draft_at(library.path(), &state, request([30.0, 0.0, 0.0]))
        .await
        .expect("arm");
    let record = place_record(draft);
    // Unmated copy (no resolved placement) keeps the draft placement.
    let EditCommand::AddOperation { record: twin, .. } =
        &copy_placement_commands(record.clone(), None).expect("copy commands")[0]
    else {
        panic!("one AddOperation");
    };
    let Operation::Known(KnownOperation::DetachComponent(detached)) = &twin.op else {
        panic!("detach twin");
    };
    assert_eq!(
        detached.placement.translate,
        [30.0, 0.0, 0.0].map(Scalar::new)
    );
    let cmds = copy_placement_commands(record.clone(), None).expect("copy commands");
    api::apply_operation(&state, host(), cmds, None)
        .await
        .expect("commit copy");
    let snap = regen(&state).await;
    assert_eq!(snap.bodies.len(), 2, "plate + copied screw");
    let after = api::get_projection(&state).await.unwrap();
    assert_eq!(after.undo_depth, before.undo_depth + 1, "one history step");
    let feature = after
        .features
        .iter()
        .find(|f| f.id == record.record_id.to_string())
        .expect("copy feature");
    assert_eq!(feature.op_type, "DetachComponent");
    let screw = snap.bodies.iter().find(|b| b.body != plate).unwrap().body;
    let volume = exact_volume(&wm, screw).await;
    assert!(volume > 100.0, "a real solid: {volume}");

    // Save, then reopen on a fresh worker: the detached geometry stays.
    let path = doc_dir.path().join("copy.onecad");
    state
        .runtime
        .lock()
        .await
        .as_mut()
        .unwrap()
        .save(&path, save_meta())
        .expect("save");
    // One undo restores the pre-place state.
    let undone = api::undo(&state, host()).await.expect("undo");
    assert_eq!(undone.undo_depth, before.undo_depth);
    assert_eq!(undone.features, before.features);
    assert_eq!(regen(&state).await.bodies.len(), 1, "plate only");
    wm.shutdown().await;
    wm.retire();

    let wm2 = spawn_worker(real_worker().expect("worker")).await;
    let mut reopened = open_over(&wm2, &path);
    let report = reopened
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    let Outcome::Published(snap2) = report.outcome else {
        panic!("reopen must publish");
    };
    assert_eq!(snap2.bodies.len(), 2);
    let copied = snap2.bodies.iter().find(|b| b.body != plate).unwrap().body;
    let reopened_volume = exact_volume(&wm2, copied).await;
    assert!(
        (reopened_volume - volume).abs() < 1e-6,
        "{reopened_volume} vs {volume}"
    );
    wm2.shutdown().await;
    wm2.retire();
}

// ─────────────────────────────────────────────────────────────────────────────
// d / e. Blob components: restage + cancel
// ─────────────────────────────────────────────────────────────────────────────

/// Saves the bored plate as `acme.bored` (a `document` component) into a fresh
/// library and returns it with the authoring body's exact volume.
async fn bored_component(
    wm: &WorkerManager,
    state: &AppState,
    base: u128,
) -> (tempfile::TempDir, f64) {
    let library = tempfile::tempdir().expect("tempdir");
    let (plate, _) = author_bored_plate(state, base).await;
    save_as_component_at(
        library.path(),
        &state.runtime,
        state.exporter(),
        plate.to_string(),
        NewComponentSpec {
            id: "acme.bored".into(),
            version: "1.0.0".into(),
            name: "Bored".into(),
            category: vec!["mine".into()],
            tags: vec![],
            designation: None,
            attachments: Default::default(),
            parameters: Default::default(),
            ..Default::default()
        },
        None,
        false,
    )
    .await
    .expect("save as component");
    reindex_library_at(library.path()).expect("reindex");
    (library, exact_volume(wm, plate).await)
}

fn bored_request() -> PlaceComponentRequest {
    PlaceComponentRequest {
        component_id: "acme.bored".into(),
        ..request([0.0, 0.0, 0.0])
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn restage_is_a_no_op_for_generators_and_restores_blobs_for_baked_components() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: real worker binary not found (set ONECAD_WORKER_PATH)");
        return;
    };
    let wm = spawn_worker(bin).await;
    let state = app_state_over(&wm);

    // Generator: nothing to stage, even with a library that cannot be opened.
    open_blank(&state).await;
    let seeded = seeded_library();
    let gen = prepare_component_draft_at(seeded.path(), &state, request([0.0; 3]))
        .await
        .expect("arm generator");
    let nowhere = seeded.path().join("absent");
    restage_component_blob_at(&nowhere, &state, &gen)
        .await
        .expect("generator restage is a no-op");

    // Baked: arm, lose the document's staging, restage, commit.
    open_blank(&state).await;
    let (library, authored_volume) = bored_component(&wm, &state, 0xD1D100).await;
    open_blank(&state).await;
    let draft = prepare_component_draft_at(library.path(), &state, bored_request())
        .await
        .expect("arm baked");
    open_blank(&state).await; // arm-time staging is gone
    restage_component_blob_at(library.path(), &state, &draft)
        .await
        .expect("restage from the library");
    let record = place_record(draft.clone());
    api::apply_operation(&state, host(), vec![add(record)], None)
        .await
        .expect("commit");
    let snap = regen(&state).await;
    assert_eq!(snap.bodies.len(), 1);
    let volume = exact_volume(&wm, snap.bodies[0].body).await;
    assert!((volume - authored_volume).abs() < 1e-6, "{volume}");

    // Already staged + library gone: still Ok.
    std::fs::remove_dir_all(library.path()).expect("delete the library");
    // The workspace file is lost (sweep/reopen) while the bytes are still carried:
    // restage re-materializes it from the document, no library needed.
    let sha = draft.source.blob_ref().expect("baked").sha256.to_string();
    let workspace = state
        .runtime
        .lock()
        .await
        .as_ref()
        .unwrap()
        .import_workspace_dir()
        .to_path_buf();
    let lost = |dir: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.to_string_lossy().contains(&sha))
                    .collect()
            })
            .unwrap_or_default()
    };
    for file in lost(&workspace) {
        std::fs::remove_file(file).expect("remove the materialized blob");
    }
    assert!(lost(&workspace).is_empty());
    restage_component_blob_at(library.path(), &state, &draft)
        .await
        .expect("staged blob needs no library");
    assert!(
        !lost(&workspace).is_empty(),
        "restage re-materialized the blob"
    );

    // Neither: typed refusal naming the component.
    open_blank(&state).await;
    let err = restage_component_blob_at(library.path(), &state, &draft)
        .await
        .expect_err("nothing to restage from");
    assert!(
        matches!(&err, ApiError::InvalidCommand(m) if m.contains("acme.bored")),
        "{err:?}"
    );
    wm.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_cancelled_draft_leaves_history_and_the_saved_file_untouched() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: real worker binary not found (set ONECAD_WORKER_PATH)");
        return;
    };
    let doc_dir = tempfile::tempdir().expect("tempdir");
    let wm = spawn_worker(bin).await;
    let state = app_state_over(&wm);
    open_blank(&state).await;
    let (library, _) = bored_component(&wm, &state, 0xD1E100).await;
    let before = api::get_projection(&state).await.unwrap();

    let draft = prepare_component_draft_at(library.path(), &state, bored_request())
        .await
        .expect("arm");
    let sha = draft.source.blob_ref().expect("baked").sha256.to_string();
    {
        let guard = state.runtime.lock().await;
        assert!(guard.as_ref().unwrap().import_blob_shas().contains(&sha));
    }
    let after = api::get_projection(&state).await.unwrap();
    assert_eq!(after.revision, before.revision);
    assert_eq!(after.undo_depth, before.undo_depth);
    assert_eq!(after.features, before.features);
    assert_eq!(after.dirty, before.dirty);

    let path = doc_dir.path().join("cancelled.onecad");
    state
        .runtime
        .lock()
        .await
        .as_mut()
        .unwrap()
        .save(&path, save_meta())
        .expect("save");
    wm.shutdown().await;
    wm.retire();

    let wm2 = spawn_worker(real_worker().expect("worker")).await;
    let mut reopened = open_over(&wm2, &path);
    assert!(
        !reopened.import_blob_shas().contains(&sha),
        "an unreferenced staged blob is not persisted"
    );
    let report = reopened
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    let Outcome::Published(snap) = report.outcome else {
        panic!("reopen must publish");
    };
    assert_eq!(snap.bodies.len(), 1, "plate only");
    wm2.shutdown().await;
    wm2.retire();
}

// ─────────────────────────────────────────────────────────────────────────────
// F1 / P1. Staged hover targets: never persisted, never dirty, adopted on commit
// ─────────────────────────────────────────────────────────────────────────────

/// Saves the bored plate to a clean baseline and stages its bore face.
async fn staged_bore(
    wm: &WorkerManager,
    state: &AppState,
    base: u128,
    path: &Path,
) -> (BodyId, String, String, [f64; 3]) {
    let (plate, _) = author_bored_plate(state, base).await;
    state
        .runtime
        .lock()
        .await
        .as_mut()
        .unwrap()
        .save(path, save_meta())
        .expect("save baseline");
    let (key, origin, _) = find_cylinder(wm, plate, 3.0).await;
    let anchor = [origin[0] + 3.0, origin[1], 2.5];
    let element = stage_mate_target(
        state,
        &body_id_wire(plate),
        &key,
        anchor,
        &fence(state).await,
    )
    .await
    .expect("stage");
    (plate, key, element, anchor)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_staged_hover_target_never_dirties_and_is_not_saved() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: real worker binary not found (set ONECAD_WORKER_PATH)");
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let wm = spawn_worker(bin).await;
    let state = app_state_over(&wm);
    open_blank(&state).await;
    let path = dir.path().join("hover.onecad");
    let (plate, key, first, anchor) = staged_bore(&wm, &state, 0xD1F100, &path).await;
    assert!(
        !api::get_projection(&state).await.unwrap().dirty,
        "still clean"
    );
    assert_eq!(persisted_elements(&state).await, 0, "nothing persisted");
    // Re-hovering the same face reuses the id.
    let again = stage_mate_target(
        &state,
        &body_id_wire(plate),
        &key,
        anchor,
        &fence(&state).await,
    )
    .await
    .expect("restage");
    assert_eq!(again, first);

    // Esc: no commit. Save and reopen carry no stray element.
    state
        .runtime
        .lock()
        .await
        .as_mut()
        .unwrap()
        .save(&path, save_meta())
        .expect("save");
    wm.shutdown().await;
    wm.retire();
    let wm2 = spawn_worker(real_worker().expect("worker")).await;
    let reopened = open_over(&wm2, &path);
    assert_eq!(reopened.persisted_element_count(), 0);
    wm2.shutdown().await;
    wm2.retire();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_staged_target_is_adopted_by_the_committed_place_and_mates_after_reopen() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: real worker binary not found (set ONECAD_WORKER_PATH)");
        return;
    };
    let library = seeded_library();
    let dir = tempfile::tempdir().expect("tempdir");
    let wm = spawn_worker(bin).await;
    let state = app_state_over(&wm);
    open_blank(&state).await;
    let path = dir.path().join("adopt.onecad");
    let (plate, key, element, anchor) = staged_bore(&wm, &state, 0xD1F200, &path).await;
    let req = mated_request(
        plate,
        &key,
        element,
        anchor,
        MateKind::ConcentricAndCoincident,
    );
    let draft = prepare_component_draft_at(library.path(), &state, req)
        .await
        .expect("arm");
    let record = place_record(draft);
    api::apply_operation(&state, host(), vec![add(record)], None)
        .await
        .expect("commit");
    assert_eq!(persisted_elements(&state).await, 1, "the commit adopted it");
    let snap = regen(&state).await;
    let screw = snap.bodies.iter().find(|b| b.body != plate).unwrap().body;
    let _ = screw;
    state
        .runtime
        .lock()
        .await
        .as_mut()
        .unwrap()
        .save(&path, save_meta())
        .expect("save");
    wm.shutdown().await;
    wm.retire();

    let wm2 = spawn_worker(real_worker().expect("worker")).await;
    let mut reopened = open_over(&wm2, &path);
    assert_eq!(reopened.persisted_element_count(), 1);
    let report = reopened
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    let Outcome::Published(snap2) = report.outcome else {
        panic!("reopen must publish");
    };
    let projection = reopened.projection();
    assert!(
        projection
            .features
            .iter()
            .all(|f| f.status == onecad_app::dto::FeatureStatus::Ok),
        "the persisted mate target resolves: {:?}",
        projection.features
    );
    let copied = snap2.bodies.iter().find(|b| b.body != plate).unwrap().body;
    let (_, bore_origin, _) = find_cylinder(&wm2, plate, 3.0).await;
    let (_, shank_origin, _) = find_cylinder(&wm2, copied, 3.0).await;
    let off = ((shank_origin[0] - bore_origin[0]).powi(2)
        + (shank_origin[1] - bore_origin[1]).powi(2))
    .sqrt();
    assert!(off < 1e-6, "shank {off} mm off the bore after reopen");
    wm2.shutdown().await;
    wm2.retire();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stale_fence_is_refused_not_substituted() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: real worker binary not found (set ONECAD_WORKER_PATH)");
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let wm = spawn_worker(bin).await;
    let state = app_state_over(&wm);
    open_blank(&state).await;
    let path = dir.path().join("stale.onecad");
    let (plate, key, _, anchor) = staged_bore(&wm, &state, 0xD1F300, &path).await;
    let mut stale = fence(&state).await;
    stale.snapshot_id += 1;
    let err = stage_mate_target(&state, &body_id_wire(plate), &key, anchor, &stale)
        .await
        .expect_err("a fence naming another snapshot is stale");
    assert!(matches!(err, ApiError::StalePreview(_)), "{err:?}");
    wm.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_exact_mated_draft_still_echoes_and_a_mated_copy_discards_the_staged_id() {
    let Some(bin) = real_worker() else {
        eprintln!("skip: real worker binary not found (set ONECAD_WORKER_PATH)");
        return;
    };
    let library = seeded_library();
    let dir = tempfile::tempdir().expect("tempdir");
    let wm = spawn_worker(bin).await;
    let state = app_state_over(&wm);
    open_blank(&state).await;
    let path = dir.path().join("exact.onecad");
    let (plate, key, element, anchor) = staged_bore(&wm, &state, 0xD1F400, &path).await;
    let req = mated_request(
        plate,
        &key,
        element,
        anchor,
        MateKind::ConcentricAndCoincident,
    );
    let rough = prepare_component_draft_at(library.path(), &state, req)
        .await
        .expect("arm");
    let echoed = preview(&state, &rough, RecordId::new())
        .await
        .mate_placement
        .expect("the rough draft's mate resolved");

    // Re-draft at the resolved placement: nothing left to move, echo stays.
    let mut exact = rough.clone();
    exact.placement = echoed.clone();
    let previewed = preview(&state, &exact, RecordId::new()).await;
    assert!(
        previewed.mate_placement.is_some(),
        "unmoved mate still echoes"
    );

    // Copy commits the echoed placement; the staged id is discarded, not adopted.
    let cmds = copy_placement_commands(place_record(rough), Some(echoed)).expect("copy");
    api::apply_operation(&state, host(), cmds, None)
        .await
        .expect("commit copy");
    assert_eq!(
        persisted_elements(&state).await,
        0,
        "a copy carries no mate"
    );
    assert_eq!(regen(&state).await.bodies.len(), 2);
    wm.shutdown().await;
}
