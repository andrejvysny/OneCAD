//! ONECAD-315 S-315c (app half): staged operand promotion against the REAL C++
//! worker. Staging mints and binds ids without dirtying, persisting, ticking
//! autosave or bumping the revision; a commit adopts exactly the staged ids it
//! references; the real worker never folds a cancelled stage into `doc.elements`
//! (the leak guard itself is driven through `commit_snapshot` by the FakeBackend
//! unit test `published_regen_strips_a_folded_staged_id_and_keeps_a_promoted_one`);
//! a stale scope, a restarted worker or a swapped runtime refuses loudly with no
//! mutation.
//!
//! Gated on `ONECAD_WORKER_PATH`; `ONECAD_REQUIRE_WORKER=1` makes a missing
//! binary a hard failure.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use onecad_core::document::record::{
    BooleanMode, ExtrudeMode, ExtrudeParams, FilletParams, KnownOperation, Operation,
    OperationRecord, PlaneKind, SketchOpParams, SketchPlaneRef,
};
use onecad_core::document::refs::{ElementKind, ElementRef, SketchRegionRef};
use onecad_core::document::variables::Scalar;
use onecad_core::edit::EditCommand;
use onecad_core::ids::{
    BodyId, ConstraintId, DocumentRevision, ElementId, EntityId, RecordId, RegionId, SketchId,
    SnapshotId, TopoKey, WorkerEpoch,
};
use onecad_core::io::container::{ContainerReader, SaveMeta};
use onecad_core::math::{Vec2, Vec3};
use onecad_core::regen::{CancelToken, GeometryEngine, Lod, Outcome, RegenRequest};
use onecad_core::sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane};

use onecad_app::api::{self, PreviewContext, StageAddress, StagePick};
use onecad_app::document_runtime::{DocumentRuntime, RegenReport};
use onecad_app::error::ApiError;
use onecad_app::export::GeometryExporter;
use onecad_app::state::AppState;
use onecad_app::worker::manager::SupervisorConfig;
use onecad_app::worker::wire::sketch_wire;
use onecad_app::worker::{
    resolve_worker_path, CircuitControl, ElementQuery, FaceBoundaryProjection, MeshProvider,
    PreviewEngine, SolverEngine, StepImport, WorkerManager, WorkerReadiness,
};
use onecad_protocol::mesh::{f32_le, u32_le, validate_mesh_blob, MeshHeaderView};

// ── harness ──────────────────────────────────────────────────────────────────

fn real_worker() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("ONECAD_WORKER_PATH") {
        let path = PathBuf::from(&p);
        assert!(
            path.is_file(),
            "ONECAD_WORKER_PATH={p:?} has no worker binary"
        );
        return Some(path);
    }
    if let Some(path) = resolve_worker_path() {
        return Some(path);
    }
    assert!(
        std::env::var("ONECAD_REQUIRE_WORKER").as_deref() != Ok("1"),
        "ONECAD_REQUIRE_WORKER=1 but no worker binary resolved"
    );
    None
}

async fn spawn_worker(bin: PathBuf) -> WorkerManager {
    let wm = WorkerManager::spawn(SupervisorConfig::production(bin));
    assert!(wm.wait_ready(Duration::from_secs(10)).await, "worker ready");
    wm
}

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

fn save_meta() -> SaveMeta {
    SaveMeta {
        app_version: "staged-operands".into(),
        occt_fingerprint: None,
        created: "2026-10-21T00:00:00Z".into(),
        modified: "2026-10-21T00:00:00Z".into(),
    }
}

async fn regen(rt: &mut DocumentRuntime) -> RegenReport {
    rt.run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await
}

fn published(report: &RegenReport, what: &str) -> usize {
    match &report.outcome {
        Outcome::Published(snapshot) => snapshot.repair_summary.needs_repair_count,
        other => panic!("{what}: expected Published, got {other:?}"),
    }
}

const SKETCH_A: u128 = 0x315A;
const EXTRUDE_A: u128 = 0x315B;
const FILLET: u128 = 0x315C;
const FILLET_2: u128 = 0x315D;

fn body_a() -> BodyId {
    BodyId(Uuid::from_u128(EXTRUDE_A))
}

fn rect_sketch(sid: SketchId, base: u128, w: f64, h: f64) -> Sketch {
    let e = |n: u128| EntityId(Uuid::from_u128(base + n));
    let c = |n: u128| ConstraintId(Uuid::from_u128(base + 0x100 + n));
    let corners = [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)];
    let mut sk = Sketch::on_world_plane(sid, "Rect", WorldPlane::XY);
    for side in 0..4u128 {
        let (x0, y0) = corners[side as usize];
        let (x1, y1) = corners[(side as usize + 1) % 4];
        sk.add_entity(SketchEntity::point(
            e(side * 2),
            Vec2::new_unchecked(x0, y0),
            false,
            false,
        ))
        .unwrap();
        sk.add_entity(SketchEntity::point(
            e(side * 2 + 1),
            Vec2::new_unchecked(x1, y1),
            false,
            false,
        ))
        .unwrap();
        sk.add_entity(SketchEntity::line(
            e(0x10 + side),
            e(side * 2),
            e(side * 2 + 1),
            false,
        ))
        .unwrap();
    }
    for side in 0..4u128 {
        sk.add_constraint(Constraint::Coincident {
            id: c(side),
            point1: e(side * 2 + 1),
            point2: e(((side + 1) % 4) * 2),
            point1_position: CurvePosition::Arbitrary,
            point2_position: CurvePosition::Arbitrary,
        })
        .unwrap();
    }
    sk
}

fn sketch_record(rec: u128, sk: &Sketch) -> OperationRecord {
    let (_plane, entities, constraints) = sketch_wire(sk);
    let params = SketchOpParams {
        sketch: sk.id,
        plane: SketchPlaneRef {
            kind: PlaneKind::Xy,
            origin: Vec3::new_unchecked(0.0, 0.0, 0.0),
            x_axis: Vec3::new_unchecked(1.0, 0.0, 0.0),
            y_axis: Vec3::new_unchecked(0.0, 1.0, 0.0),
            normal: Vec3::new_unchecked(0.0, 0.0, 1.0),
            extra: Default::default(),
        },
        entities: entities.as_array().cloned().unwrap_or_default(),
        constraints: constraints.as_array().cloned().unwrap_or_default(),
        host_face: None,
        extra: Default::default(),
    };
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        0,
        "Sketch",
        Operation::Known(KnownOperation::Sketch(params)),
    )
}

fn extrude_record(sketch: SketchId, dist: f64) -> OperationRecord {
    OperationRecord::new(
        RecordId(Uuid::from_u128(EXTRUDE_A)),
        0,
        "Extrude",
        Operation::Known(KnownOperation::Extrude(ExtrudeParams {
            profile: Some(SketchRegionRef {
                sketch,
                region: RegionId::new(""),
                region_identity_version: None,
                region_anchor: None,
                extra: Default::default(),
            }),
            distance: Scalar::new(dist),
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
        })),
    )
}

fn fillet_record(rec: u128, edges: Vec<ElementRef>, radius: f64) -> OperationRecord {
    let edge_ids = edges
        .iter()
        .map(|edge| edge.primary.as_ref().expect("primary").element.clone())
        .collect();
    OperationRecord::new(
        RecordId(Uuid::from_u128(rec)),
        0,
        "Fillet",
        Operation::Known(KnownOperation::Fillet(FilletParams {
            radius: Scalar::new(radius),
            edge_ids,
            edges,
            chain_tangent_edges: false,
            tangent_closure_version: None,
            extra: Default::default(),
        })),
    )
}

/// A 40×20×25 box as body A, published; the state owns the runtime.
async fn box_state(wm: &WorkerManager) -> AppState {
    let state = app_state_over(wm);
    let rt = box_runtime(&state, wm).await;
    *state.runtime.lock().await = Some(rt);
    state
}

/// The published box runtime, not yet installed in `state`.
async fn box_runtime(state: &AppState, wm: &WorkerManager) -> DocumentRuntime {
    let (engine, meshes, solver) = state.make_backend();
    let mut rt = DocumentRuntime::new_blank(engine, meshes, solver);
    rt.bind_geometry_read_query(Arc::new(wm.clone()))
        .expect("bind read query");
    let sid = SketchId(Uuid::from_u128(0x315));
    for record in [
        sketch_record(SKETCH_A, &rect_sketch(sid, 0x3150_0000, 40.0, 20.0)),
        extrude_record(sid, 25.0),
    ] {
        rt.apply(EditCommand::AddOperation {
            record,
            at_cursor: true,
        })
        .expect("AddOperation");
    }
    assert_eq!(published(&regen(&mut rt).await, "box"), 0);
    rt
}

// ── MESH1 picks ──────────────────────────────────────────────────────────────

const SEC_FACE_ID_OFFS: u32 = 5;
const SEC_FACE_ID_CHARS: u32 = 6;
const SEC_EDGE_RANGES: u32 = 7;
const SEC_EDGE_POSITIONS: u32 = 8;
const SEC_EDGE_ID_OFFS: u32 = 9;
const SEC_EDGE_ID_CHARS: u32 = 10;
const SEC_POSITIONS: u32 = 1;
const SEC_INDICES: u32 = 3;
const SEC_FACE_RANGES: u32 = 4;

fn id_table(view: &MeshHeaderView, blob: &[u8], offs: u32, chars: u32, n: usize) -> Vec<String> {
    let (o, c) = (
        view.section(offs).expect("offs").offset as usize,
        view.section(chars).expect("chars").offset as usize,
    );
    (0..n)
        .map(|i| {
            let lo = u32_le(blob, o + i * 4) as usize;
            let hi = u32_le(blob, o + (i + 1) * 4) as usize;
            String::from_utf8_lossy(&blob[c + lo..c + hi]).into_owned()
        })
        .collect()
}

fn point(blob: &[u8], base: usize, i: usize) -> [f64; 3] {
    let o = base + i * 12;
    [0, 4, 8].map(|k| f64::from(f32_le(blob, o + k)))
}

/// Top-cap edges running along X: (topoKey, mean y).
fn top_x_edges(blob: &[u8]) -> Vec<(String, f64)> {
    let view = validate_mesh_blob(blob).expect("MESH1");
    let keys = id_table(
        &view,
        blob,
        SEC_EDGE_ID_OFFS,
        SEC_EDGE_ID_CHARS,
        view.edge_count as usize,
    );
    let ranges = view.section(SEC_EDGE_RANGES).expect("ranges").offset as usize;
    let positions = view.section(SEC_EDGE_POSITIONS).expect("positions").offset as usize;
    let top = f64::from(view.bbox_max[2]);
    let mut out = Vec::new();
    for (edge, key) in keys.iter().enumerate() {
        let first = u32_le(blob, ranges + edge * 8) as usize;
        let count = u32_le(blob, ranges + edge * 8 + 4) as usize;
        let pts: Vec<_> = (first..first + count)
            .map(|i| point(blob, positions, i))
            .collect();
        let on_top = pts.iter().all(|p| (p[2] - top).abs() < 1e-3);
        let along_x = pts.iter().all(|p| (p[1] - pts[0][1]).abs() < 1e-3);
        if count > 1 && on_top && along_x {
            out.push((key.clone(), pts[0][1]));
        }
    }
    out.sort_by(|a, b| a.1.total_cmp(&b.1));
    out
}

fn top_face_key(blob: &[u8]) -> String {
    let view = validate_mesh_blob(blob).expect("MESH1");
    let keys = id_table(
        &view,
        blob,
        SEC_FACE_ID_OFFS,
        SEC_FACE_ID_CHARS,
        view.face_count as usize,
    );
    let ranges = view.section(SEC_FACE_RANGES).expect("ranges").offset as usize;
    let indices = view.section(SEC_INDICES).expect("indices").offset as usize;
    let positions = view.section(SEC_POSITIONS).expect("positions").offset as usize;
    let top = f64::from(view.bbox_max[2]);
    (0..view.face_count as usize)
        .find(|face| {
            let first = u32_le(blob, ranges + face * 8) as usize;
            let count = u32_le(blob, ranges + face * 8 + 4) as usize;
            (first..first + count).all(|t| {
                (0..3).all(|k| {
                    let v = u32_le(blob, indices + t * 12 + k * 4) as usize;
                    (point(blob, positions, v)[2] - top).abs() < 1e-3
                })
            })
        })
        .map(|face| keys[face].clone())
        .expect("top face")
}

fn mesh_volume(blob: &[u8]) -> f64 {
    let view = validate_mesh_blob(blob).expect("MESH1");
    let indices = view.section(SEC_INDICES).expect("indices").offset as usize;
    let positions = view.section(SEC_POSITIONS).expect("positions").offset as usize;
    let mut volume = 0.0;
    for t in 0..view.triangle_count as usize {
        let [a, b, c] = [0, 1, 2].map(|k| {
            point(
                blob,
                positions,
                u32_le(blob, indices + t * 12 + k * 4) as usize,
            )
        });
        volume += a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
            + a[2] * (b[0] * c[1] - b[1] * c[0]);
    }
    volume / 6.0
}

async fn body_mesh(state: &AppState) -> Arc<Vec<u8>> {
    let mut guard = state.runtime.lock().await;
    guard
        .as_mut()
        .expect("runtime")
        .get_mesh(body_a(), Lod::Coarse, None)
        .await
        .expect("mesh")
}

fn edge_pick(key: &str) -> StagePick {
    StagePick {
        body: body_a(),
        kind: ElementKind::Edge,
        address: StageAddress::TopoKey(key.into()),
    }
}

fn ref_id(reference: &ElementRef) -> ElementId {
    reference.primary.as_ref().expect("primary").element.clone()
}

/// `(dirty, revision, staged count, autosave tick)` — what staging must not move
/// except the staged count.
async fn observe(state: &AppState) -> (bool, u64, usize, u64) {
    let guard = state.runtime.lock().await;
    let rt = guard.as_ref().expect("runtime");
    (
        rt.is_dirty(),
        rt.revision().0,
        rt.staged_element_count(),
        *state.autosave_tick.borrow(),
    )
}

async fn saved_elements(state: &AppState, path: &Path) -> onecad_core::document::Document {
    {
        let mut guard = state.runtime.lock().await;
        guard
            .as_mut()
            .expect("runtime")
            .save(path, save_meta())
            .expect("save");
    }
    ContainerReader::open(path)
        .expect("reopen container")
        .document()
        .clone()
}

fn document_json(document: &onecad_core::document::Document) -> String {
    serde_json::to_string(document).expect("document json")
}

// ── tests ────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn staging_does_not_dirty_and_commit_adopts_the_staged_refs() {
    let Some(bin) = real_worker() else {
        return;
    };
    let wm = spawn_worker(bin).await;
    let state = box_state(&wm).await;
    let dir = tempfile::tempdir().expect("tempdir");
    let before = saved_elements(&state, &dir.path().join("before.onecad")).await;
    let blob = body_mesh(&state).await;
    let plain_volume = mesh_volume(&blob);
    let edges = top_x_edges(&blob);
    assert_eq!(edges.len(), 2, "box top cap has two X edges");
    let scope = api::preview_context(&state).await.expect("scope");
    let observed = observe(&state).await;

    let refs = api::stage_elements(
        &state,
        &scope,
        edges.iter().map(|(key, _)| edge_pick(key)).collect(),
    )
    .await
    .expect("stage two edges");
    assert_eq!(refs.len(), 2);
    for reference in &refs {
        let primary = reference.primary.as_ref().expect("ElementId primary");
        assert!(primary.element.as_str().starts_with("el_"));
        assert_eq!((primary.body, primary.kind), (body_a(), ElementKind::Edge));
        assert!(reference.intent.is_some(), "intent stamped at staging");
    }
    let (dirty, revision, staged, tick) = observe(&state).await;
    assert_eq!(
        (dirty, revision, tick),
        (observed.0, observed.1, observed.3)
    );
    assert_eq!(staged, 2);
    let after = saved_elements(&state, &dir.path().join("after.onecad")).await;
    assert_eq!(
        document_json(&before),
        document_json(&after),
        "document.json unchanged"
    );

    let record = fillet_record(FILLET, refs.clone(), 2.0);
    let preview = wm
        .preview_op(
            record.op.clone(),
            record.record_id.to_string(),
            None,
            None,
            Lod::Coarse,
        )
        .await
        .expect("preview staged fillet");
    assert!(
        preview.needs_repair.is_empty(),
        "{:?}",
        preview.needs_repair
    );
    let preview_volume = mesh_volume(&preview.bodies[0].mesh);
    assert!(preview_volume < plain_volume);
    {
        let mut guard = state.runtime.lock().await;
        let rt = guard.as_mut().expect("runtime");
        rt.apply(EditCommand::AddOperation {
            record,
            at_cursor: true,
        })
        .expect("commit staged fillet");
        assert!(rt.is_dirty());
        assert_eq!(
            rt.staged_element_count(),
            0,
            "commit adopted every staged id"
        );
    }
    // Before regen: the consumed edges are still in the partition with evidence.
    let adopted = saved_elements(&state, &dir.path().join("adopted.onecad")).await;
    for reference in &refs {
        let entry = adopted
            .elements
            .get(&ref_id(reference))
            .expect("adopted entry");
        assert!(entry.descriptor.is_some(), "descriptor evidence kept");
    }
    {
        let mut guard = state.runtime.lock().await;
        assert_eq!(
            published(&regen(guard.as_mut().expect("runtime")).await, "fillet"),
            0
        );
    }
    let committed = mesh_volume(&body_mesh(&state).await);
    assert!(
        (committed - preview_volume).abs() < 2.0,
        "{committed} vs {preview_volume}"
    );
    let path = dir.path().join("fillet.onecad");
    saved_elements(&state, &path).await;

    {
        let mut guard = state.runtime.lock().await;
        let rt = guard.as_mut().expect("runtime");
        rt.undo().expect("undo fillet");
        assert_eq!(published(&regen(rt).await, "undo"), 0);
        rt.redo().expect("redo").expect("redo fillet");
        assert_eq!(published(&regen(rt).await, "redo"), 0);
    }
    wm.shutdown().await;

    let fresh = spawn_worker(real_worker().expect("worker")).await;
    let engine: Arc<dyn GeometryEngine> = Arc::new(fresh.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(fresh.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(fresh.clone());
    let mut reopened = DocumentRuntime::open(&path, engine, meshes, solver).expect("reopen");
    assert_eq!(
        published(&regen(&mut reopened).await, "reopen"),
        0,
        "no repair after reopen"
    );
    fresh.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_worker_does_not_fold_a_cancelled_staged_id() {
    let Some(bin) = real_worker() else {
        return;
    };
    let wm = spawn_worker(bin).await;
    let state = box_state(&wm).await;
    let dir = tempfile::tempdir().expect("tempdir");
    saved_elements(&state, &dir.path().join("clean.onecad")).await;
    let blob = body_mesh(&state).await;
    let edges = top_x_edges(&blob);
    let scope = api::preview_context(&state).await.expect("scope");
    let staged = api::stage_elements(&state, &scope, vec![edge_pick(&edges[0].0)])
        .await
        .expect("stage first edge");
    let staged_id = ref_id(&staged[0]);
    // Cancel: drop the refs. An unrelated edit on the SAME body follows.
    {
        let mut guard = state.runtime.lock().await;
        let rt = guard.as_mut().expect("runtime");
        assert!(!rt.is_dirty(), "staging left the saved document clean");
        let snapshot = rt.head_snapshot_id().expect("head");
        let other = rt
            .promote_selection(snapshot, body_a(), vec![(TopoKey::new(&edges[1].0), None)])
            .await
            .expect("legacy promote second edge");
        assert!(rt.is_dirty(), "legacy promotion still dirties");
        let other = ElementId::new(&other[0].element_id);
        let reference = ElementRef {
            primary: Some(onecad_core::document::refs::PrimaryRef {
                body: body_a(),
                element: other,
                kind: ElementKind::Edge,
                extra: Default::default(),
            }),
            intent: None,
            anchor: None,
            extra: Default::default(),
        };
        rt.apply(EditCommand::AddOperation {
            record: fillet_record(FILLET_2, vec![reference], 2.0),
            at_cursor: true,
        })
        .expect("fillet the other edge");
        assert_eq!(published(&regen(rt).await, "unrelated fillet"), 0);
        assert_eq!(
            rt.staged_element_count(),
            1,
            "the cancelled stage stays staged"
        );
    }
    // Observes the worker's delta, not the guard: the real worker never echoes an
    // unadopted staged binding into `elementMapDelta`.
    let saved = saved_elements(&state, &dir.path().join("leak.onecad")).await;
    assert!(
        !saved.elements.contains(&staged_id),
        "a cancelled stage must not persist"
    );
    wm.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stale_scope_refuses_without_mutation() {
    let Some(bin) = real_worker() else {
        return;
    };
    let wm = spawn_worker(bin).await;
    let state = box_state(&wm).await;
    let blob = body_mesh(&state).await;
    let key = top_x_edges(&blob)[0].0.clone();
    let scope = api::preview_context(&state).await.expect("scope");
    let observed = observe(&state).await;
    let stale: [(&str, PreviewContext); 4] = [
        (
            "snapshot",
            PreviewContext {
                snapshot_id: SnapshotId(scope.snapshot_id.0 + 1),
                ..scope.clone()
            },
        ),
        (
            "revision",
            PreviewContext {
                revision: DocumentRevision(scope.revision.0 + 1),
                ..scope.clone()
            },
        ),
        (
            "runtime",
            PreviewContext {
                runtime_session: Uuid::new_v4().to_string(),
                ..scope.clone()
            },
        ),
        (
            "epoch",
            PreviewContext {
                worker_epoch: WorkerEpoch(scope.worker_epoch.0 + 1),
                ..scope.clone()
            },
        ),
    ];
    for (axis, scope) in stale {
        let error = api::stage_elements(&state, &scope, vec![edge_pick(&key)])
            .await
            .expect_err(axis);
        assert!(
            matches!(error, ApiError::StalePreview(_)),
            "{axis}: {error:?}"
        );
        let error = api::stage_edge_op(&state, &scope, "Fillet", vec![edge_input(&key)], false)
            .await
            .expect_err(axis);
        assert!(
            matches!(error, ApiError::StalePreview(_)),
            "{axis}: {error:?}"
        );
        assert_eq!(observe(&state).await, observed, "{axis}: no mutation");
    }
    // A real edit moves the revision, so the captured scope is stale too.
    {
        let mut guard = state.runtime.lock().await;
        let sketch = rect_sketch(SketchId(Uuid::from_u128(0x3158)), 0x3152_0000, 5.0, 5.0);
        guard
            .as_mut()
            .expect("runtime")
            .apply(EditCommand::AddOperation {
                record: sketch_record(0x3157, &sketch),
                at_cursor: true,
            })
            .expect("unrelated sketch");
    }
    let moved = observe(&state).await;
    assert_ne!(moved.1, observed.1, "revision moved");
    let error = api::stage_elements(&state, &scope, vec![edge_pick(&key)])
        .await
        .expect_err("revision moved");
    assert!(matches!(error, ApiError::StalePreview(_)), "{error:?}");
    assert_eq!(observe(&state).await.2, 0, "nothing staged");
    wm.shutdown().await;
}

fn edge_input(key: &str) -> api::EdgeOpPickInput {
    serde_json::from_value(serde_json::json!({
        "bodyId": onecad_app::worker::wire::body_id_wire(body_a()),
        "topoKey": key,
    }))
    .expect("edge pick input")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn element_labeled_pick_reuses_its_id_and_edge_op_stages_without_dirtying() {
    let Some(bin) = real_worker() else {
        return;
    };
    let wm = spawn_worker(bin).await;
    let state = box_state(&wm).await;
    let dir = tempfile::tempdir().expect("tempdir");
    saved_elements(&state, &dir.path().join("clean.onecad")).await;
    let blob = body_mesh(&state).await;
    let key = top_x_edges(&blob)[0].0.clone();
    let scope = api::preview_context(&state).await.expect("scope");
    let observed = observe(&state).await;
    assert!(!observed.0, "saved before observing");
    let first = api::stage_elements(&state, &scope, vec![edge_pick(&key)])
        .await
        .expect("stage by TopoKey");
    let id = ref_id(&first[0]);
    let again = api::stage_elements(
        &state,
        &scope,
        vec![StagePick {
            body: body_a(),
            kind: ElementKind::Edge,
            address: StageAddress::Element(id.as_str().into()),
        }],
    )
    .await
    .expect("stage by ElementId");
    assert_eq!(ref_id(&again[0]), id, "ElementId pick reuses its id");
    assert_eq!(observe(&state).await.2, 1, "nothing new minted");

    let staged = api::stage_edge_op(&state, &scope, "Fillet", vec![edge_input(&key)], true)
        .await
        .expect("stage edge op");
    assert!(staged.prepared.refusal.is_none());
    assert_eq!(staged.refs.len(), staged.prepared.edges.len());
    for (edge, reference) in staged.prepared.edges.iter().zip(&staged.refs) {
        assert_eq!(edge.element_id.as_deref(), Some(ref_id(reference).as_str()));
        assert!(reference.intent.is_some());
    }
    let (dirty, revision, _, tick) = observe(&state).await;
    assert!(!dirty, "stage_edge_op does not dirty");
    assert_eq!((revision, tick), (observed.1, observed.3));
    {
        let mut guard = state.runtime.lock().await;
        let rt = guard.as_mut().expect("runtime");
        rt.apply(EditCommand::AddOperation {
            record: fillet_record(FILLET, staged.refs.clone(), 2.0),
            at_cursor: true,
        })
        .expect("commit staged closure");
        assert_eq!(rt.staged_element_count(), 0);
    }
    let saved = saved_elements(&state, &dir.path().join("closure.onecad")).await;
    for reference in &staged.refs {
        assert!(
            saved.elements.contains(&ref_id(reference)),
            "closure edge adopted"
        );
    }
    {
        let mut guard = state.runtime.lock().await;
        let rt = guard.as_mut().expect("runtime");
        assert_eq!(published(&regen(rt).await, "closure fillet"), 0);
    }
    wm.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn face_entry_after_cancelled_stage_persists_host_face_evidence() {
    let Some(bin) = real_worker() else {
        return;
    };
    let wm = spawn_worker(bin).await;
    let state = box_state(&wm).await;
    let blob = body_mesh(&state).await;
    let top = top_face_key(&blob);
    let scope = api::preview_context(&state).await.expect("scope");
    let staged = api::stage_elements(
        &state,
        &scope,
        vec![StagePick {
            body: body_a(),
            kind: ElementKind::Face,
            address: StageAddress::TopoKey(top.clone()),
        }],
    )
    .await
    .expect("stage top face");
    let face = ref_id(&staged[0]);
    // Cancel the stage, then enter a sketch on the same face.
    api::add_sketch_on_face(
        &state,
        scope.snapshot_id.0,
        onecad_app::worker::wire::body_id_wire(body_a()),
        String::new(),
        Some(top),
        SketchId(Uuid::from_u128(0x3159)).to_string(),
        "On top".into(),
    )
    .await
    .expect("sketch on face");
    {
        let mut guard = state.runtime.lock().await;
        let rt = guard.as_mut().expect("runtime");
        assert_eq!(
            rt.staged_element_count(),
            0,
            "face entry recorded the staged id"
        );
        assert_eq!(published(&regen(rt).await, "after face entry"), 0);
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("face.onecad");
    let saved = saved_elements(&state, &path).await;
    let entry = saved.elements.get(&face).expect("host face persisted");
    assert!(entry.descriptor.is_some(), "host face keeps its evidence");
    wm.shutdown().await;

    let fresh = spawn_worker(real_worker().expect("worker")).await;
    let engine: Arc<dyn GeometryEngine> = Arc::new(fresh.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(fresh.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(fresh.clone());
    let mut reopened = DocumentRuntime::open(&path, engine, meshes, solver).expect("reopen");
    assert_eq!(published(&regen(&mut reopened).await, "reopen"), 0);
    fresh.shutdown().await;
}

// ── legacy promotion, restart/swap fencing, byte identity ────────────────────

#[derive(Default)]
struct Events {
    projections: std::sync::atomic::AtomicUsize,
}

impl onecad_app::host::EventSink for Events {
    fn emit(&self, event: onecad_app::host::AppEvent) -> Result<(), String> {
        if matches!(event, onecad_app::host::AppEvent::ProjectionUpdated(_)) {
            self.projections
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        Ok(())
    }
}

struct NoDialogs;

#[async_trait::async_trait]
impl onecad_app::host::DialogPort for NoDialogs {
    async fn pick(&self, _request: onecad_app::host::FileDialog) -> Vec<String> {
        Vec::new()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn legacy_prepare_edge_op_still_persists_dirties_emits_and_ticks() {
    let Some(bin) = real_worker() else {
        return;
    };
    let wm = spawn_worker(bin).await;
    let state = box_state(&wm).await;
    let dir = tempfile::tempdir().expect("tempdir");
    saved_elements(&state, &dir.path().join("clean.onecad")).await;
    let key = top_x_edges(&body_mesh(&state).await)[0].0.clone();
    let scope = api::preview_context(&state).await.expect("scope");
    let observed = observe(&state).await;
    assert!(!observed.0, "saved before observing");
    let events = Arc::new(Events::default());
    let host = onecad_app::host::HostContext {
        paths: Default::default(),
        events: events.clone(),
        dialogs: Arc::new(NoDialogs),
    };
    let prepared = api::prepare_edge_op(
        &state,
        host,
        scope.snapshot_id.0,
        "Fillet".into(),
        vec![edge_input(&key)],
        false,
    )
    .await
    .expect("legacy prepare");
    assert!(prepared.refusal.is_none());
    let (dirty, _, staged, tick) = observe(&state).await;
    assert!(dirty, "legacy prepare_edge_op dirties");
    assert_eq!(staged, 0, "legacy prepare stages nothing");
    assert_eq!(tick, observed.3 + 1, "legacy prepare ticks autosave");
    assert_eq!(
        events.projections.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "legacy prepare emits ProjectionUpdated"
    );
    let saved = saved_elements(&state, &dir.path().join("legacy.onecad")).await;
    for edge in &prepared.edges {
        let id = edge.element_id.as_deref().expect("promoted id");
        assert!(saved.elements.contains(&ElementId::new(id)), "persisted");
    }
    wm.shutdown().await;
}

fn assert_stale(error: onecad_core::regen::EngineError, what: &str) {
    assert!(
        matches!(
            error,
            onecad_core::regen::EngineError::OpFailed {
                code: onecad_core::regen::OpFailureCode::StalePreview,
                ..
            }
        ),
        "{what}: {error:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn swapped_runtime_or_restarted_worker_refuses_an_admitted_stage() {
    let Some(bin) = real_worker() else {
        return;
    };
    let wm = spawn_worker(bin).await;
    let state = box_state(&wm).await;
    let key = TopoKey::new(top_x_edges(&body_mesh(&state).await)[0].0.clone());
    let scope = api::preview_context(&state).await.expect("scope");
    let ticket = {
        let guard = state.runtime.lock().await;
        guard
            .as_ref()
            .expect("runtime")
            .stage_ticket(&scope)
            .expect("admitted")
    };
    // The document slot is swapped between admission and staging.
    let swapped = box_runtime(&state, &wm).await;
    drop(state.runtime.lock().await.replace(swapped));
    {
        let mut guard = state.runtime.lock().await;
        let rt = guard.as_mut().expect("runtime");
        let error = rt
            .stage_selection(&ticket, body_a(), vec![key.clone()])
            .await
            .expect_err("a ticket from the replaced runtime");
        assert_stale(error, "swapped runtime");
        assert_eq!(rt.staged_element_count(), 0);
    }
    // The worker restarts between admission and staging.
    let scope = api::preview_context(&state).await.expect("scope");
    let ticket = {
        let guard = state.runtime.lock().await;
        guard
            .as_ref()
            .expect("runtime")
            .stage_ticket(&scope)
            .expect("admitted")
    };
    let epoch = wm.epoch().0;
    wm.shutdown().await;
    let mut restarted = false;
    for _ in 0..200 {
        if wm.epoch().0 > epoch && wm.state() == onecad_app::worker::WorkerState::Ready {
            restarted = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(restarted, "worker restarted (epoch bumped + Ready)");
    {
        let mut guard = state.runtime.lock().await;
        let rt = guard.as_mut().expect("runtime");
        let error = rt
            .stage_selection(&ticket, body_a(), vec![key.clone()])
            .await
            .expect_err("a ticket from the dead worker epoch");
        assert_stale(error, "restarted worker");
        rt.on_worker_restart(wm.epoch());
        let error = rt
            .stage_selection(&ticket, body_a(), vec![key.clone()])
            .await
            .expect_err("still stale after the runtime adopts the epoch");
        assert_stale(error, "adopted epoch");
        assert_eq!(rt.staged_element_count(), 0);
    }
    let error = api::stage_elements(&state, &scope, vec![edge_pick(key.as_str())])
        .await
        .expect_err("captured scope predates the restart");
    assert!(matches!(error, ApiError::StalePreview(_)), "{error:?}");
    wm.shutdown().await;
}

/// Opens `path` into a fresh state over `wm` and publishes it.
async fn opened_state(wm: &WorkerManager, path: &Path) -> AppState {
    let state = app_state_over(wm);
    let (engine, meshes, solver) = state.make_backend();
    let mut rt = DocumentRuntime::open(path, engine, meshes, solver).expect("open");
    assert_eq!(published(&regen(&mut rt).await, "open"), 0);
    *state.runtime.lock().await = Some(rt);
    state
}

fn document_bytes(document: &onecad_core::document::Document) -> Vec<u8> {
    onecad_core::io::document_io::serialize_document(document).expect("serialize")
}

/// Spec test 9, qualified per critique to `document.json`: mesh caches embed
/// MESH1 blobs whose labels follow the live worker partition.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn open_save_equals_open_stage_drop_save() {
    let Some(bin) = real_worker() else {
        return;
    };
    let wm = spawn_worker(bin).await;
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path().join("base.onecad");
    saved_elements(&box_state(&wm).await, &base).await;

    let plain = opened_state(&wm, &base).await;
    let plain = saved_elements(&plain, &dir.path().join("plain.onecad")).await;

    let staged = opened_state(&wm, &base).await;
    let edges = top_x_edges(&body_mesh(&staged).await);
    let scope = api::preview_context(&staged).await.expect("scope");
    api::stage_elements(
        &staged,
        &scope,
        edges.iter().map(|(key, _)| edge_pick(key)).collect(),
    )
    .await
    .expect("stage both edges");
    assert_eq!(observe(&staged).await.2, 2);
    let dropped = saved_elements(&staged, &dir.path().join("dropped.onecad")).await;
    assert_eq!(document_bytes(&plain), document_bytes(&dropped));
    wm.shutdown().await;
}

#[path = "staged_operands/hole.rs"]
mod hole;
