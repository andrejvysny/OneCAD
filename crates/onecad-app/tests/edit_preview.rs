//! ONECAD-369 edit preview against the REAL C++ worker (SCHEMA §7.6 `PreviewPlan`).
//!
//! The service previews replacing an existing record's operation by a full,
//! non-publishing replay. What this target proves end to end:
//!
//! * `equivalence` — preview == commit: the displayed body set, the last step's
//!   three §12 signatures and the MESH1 bytes of every re-meshed body equal what
//!   the commit publishes, for Extrude and Fillet edits with dependents and for
//!   both WP-D1 isolation cases (a halt after the edit, and an earlier halt the
//!   isolation pass routes around); a preview changes nothing (worker head,
//!   revision, dirty flag, undo depth); an edited-feature failure is classified
//!   at its own step; a stale fence is refused; save/reopen sees the unedited
//!   document;
//! * `races` — abandoning a preview frees the worker's kernel lane well inside
//!   the replay time (the worker-side cancel, not just the Rust fence), Apply
//!   preempts an in-flight preview as `Cancelled` without queueing behind it, a
//!   supersede storm leaves only the last draft `Ready`, and a worker crash
//!   mid-preview is a worker-class error after which the restart replay equals
//!   the pre-preview head and a fresh preview runs;
//! * `capability` — an old worker without `plan.preview` is `Unsupported` and is
//!   never sent the verb (stub worker);
//! * the canonical `preview_plan.ndjson` fixture replays from Rust.
//!
//! Run: `ONECAD_REQUIRE_WORKER=1 ONECAD_WORKER_PATH=… cargo test -p onecad-app --test edit_preview`
//! (`capability` needs `cargo build -p onecad-worker-stub` first and skips without it).

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use sha2::{Digest, Sha256};
use uuid::Uuid;

use onecad_app::api::{self, EditPreview, EditPreviewOutcome, PreviewContext};
use onecad_app::document_runtime::DocumentRuntime;
use onecad_app::error::ApiError;
use onecad_app::export::GeometryExporter;
use onecad_app::state::AppState;
use onecad_app::worker::manager::SupervisorConfig;
use onecad_app::worker::wire::sketch_wire;
use onecad_app::worker::{
    resolve_worker_path, CircuitControl, ElementQuery, FaceBoundaryProjection, MeshProvider,
    PreviewEngine, SolverEngine, StepImport, WorkerManager, WorkerReadiness,
};
use onecad_core::document::record::{
    BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, Operation, OperationRecord, PlaneKind,
    SketchOpParams, SketchPlaneRef,
};
use onecad_core::document::refs::SketchRegionRef;
use onecad_core::document::variables::Scalar;
use onecad_core::edit::EditCommand;
use onecad_core::ids::{BodyId, ConstraintId, EntityId, RecordId, RegionId, SketchId};
use onecad_core::io::container::SaveMeta;
use onecad_core::math::{Vec2, Vec3};
use onecad_core::regen::{
    CancelToken, GeometryEngine, Lod, ModelSnapshot, Outcome, RegenRequest, StepSignatures,
};
use onecad_core::sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane};

#[path = "fixtures/replay.rs"]
mod fixture_replay;

// ── Harness ────────────────────────────────────────────────────────────────

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
    assert!(
        wm.wait_ready(Duration::from_secs(10)).await,
        "worker handshake"
    );
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
        let projection: Arc<dyn FaceBoundaryProjection> = Arc::new(wm.clone());
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
            projection,
            step_import,
            circuit,
            readiness,
        )
    }))
}

/// A state over a fresh worker with `records` applied and regenerated.
async fn document(records: Vec<OperationRecord>) -> Option<(WorkerManager, AppState)> {
    Some(document_on(real_worker()?, records).await)
}

/// [`document`] over the worker binary at `bin`.
async fn document_on(bin: PathBuf, records: Vec<OperationRecord>) -> (WorkerManager, AppState) {
    let wm = spawn_worker(bin).await;
    let state = app_state_over(&wm);
    let (engine, meshes, solver) = state.make_backend();
    let mut rt = DocumentRuntime::new_blank(engine, meshes, solver);
    for record in records {
        rt.apply(EditCommand::AddOperation {
            record,
            at_cursor: true,
        })
        .expect("AddOperation");
    }
    let report = rt
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    assert!(
        matches!(report.outcome, Outcome::Published(_)),
        "{:?}",
        report.outcome
    );
    *state.runtime.lock().await = Some(rt);
    (wm, state)
}

async fn preview(state: &AppState, record: u128, op: Operation) -> Result<EditPreview, ApiError> {
    let fence = api::preview_context(state).await.expect("preview context");
    api::preview_existing_edit(state, fence, rec(record), op, Lod::Fine).await
}

/// Commits the same edit the way Apply does and returns the published snapshot.
async fn commit(state: &AppState, record: u128, op: Operation) -> Arc<ModelSnapshot> {
    let mut guard = state.runtime.lock().await;
    let rt = guard.as_mut().unwrap();
    let outcome = rt
        .apply(EditCommand::UpdateOperationParams {
            record: rec(record),
            op,
        })
        .expect("commit edit");
    let from = outcome.dirty.map_or(0, |d| d.from);
    let report = rt
        .run_regen(RegenRequest::ToEnd { from }, CancelToken::new())
        .await;
    match report.outcome {
        Outcome::Published(snapshot) => snapshot,
        other => panic!("commit must publish, got {other:?}"),
    }
}

/// Everything a preview must leave alone.
#[derive(Debug, PartialEq)]
struct HeadState {
    context: PreviewContext,
    dirty: bool,
    undo_depth: usize,
    bodies: Vec<BodyId>,
}

async fn head_state(state: &AppState) -> HeadState {
    let context = api::preview_context(state).await.unwrap();
    let guard = state.runtime.lock().await;
    let rt = guard.as_ref().unwrap();
    HeadState {
        context,
        dirty: rt.is_dirty(),
        undo_depth: rt.undo_depth(),
        bodies: rt.head_body_ids(),
    }
}

async fn committed_mesh(state: &AppState, body: BodyId) -> Vec<u8> {
    let mut guard = state.runtime.lock().await;
    let rt = guard.as_mut().unwrap();
    rt.get_mesh(body, Lod::Fine, None)
        .await
        .expect("committed mesh")
        .to_vec()
}

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// preview == commit: bodies, the last step's signatures, and MESH1 bytes.
async fn assert_preview_equals_commit(
    state: &AppState,
    preview: &EditPreview,
    snap: &ModelSnapshot,
) {
    let committed: BTreeSet<BodyId> = snap.bodies.iter().map(|b| b.body).collect();
    let previewed: BTreeSet<BodyId> = preview.bodies.iter().copied().collect();
    assert_eq!(
        previewed, committed,
        "the candidate body set is what Apply publishes"
    );
    let last: Option<&StepSignatures> = preview.step_signatures.last().map(|(_, s)| s);
    assert_eq!(last, snap.signatures.as_ref(), "last-step §12 signatures");
    for mesh in &preview.meshes {
        let body: BodyId = mesh.body_id.parse().unwrap();
        assert_eq!(
            sha(&mesh.mesh),
            sha(&committed_mesh(state, body).await),
            "MESH1 of {body} at equal LOD"
        );
    }
}

// ── Records ────────────────────────────────────────────────────────────────

const SKETCH_A: u128 = 0x5A0;
const EXTRUDE_A: u128 = 0x5A1;
const SKETCH_B: u128 = 0x5B0;
const BOSS_B: u128 = 0x5B1;
const SKETCH_C: u128 = 0x5C0;
const EXTRUDE_C: u128 = 0x5C1;
const BROKEN: u128 = 0x5D1;

fn rec(seed: u128) -> RecordId {
    RecordId(Uuid::from_u128(seed))
}

fn body(seed: u128) -> BodyId {
    BodyId(Uuid::from_u128(seed))
}

fn sketch_id(seed: u128) -> SketchId {
    SketchId(Uuid::from_u128(seed + 0x1_0000))
}

fn rect_sketch(sid: SketchId, base: u128, x0: f64, y0: f64, w: f64, h: f64) -> Sketch {
    let e = |n: u128| EntityId(Uuid::from_u128(base + n));
    let c = |n: u128| ConstraintId(Uuid::from_u128(base + 0x40 + n));
    let mut sk = Sketch::on_world_plane(sid, "Rect", WorldPlane::XY);
    let corners = [(x0, y0), (x0 + w, y0), (x0 + w, y0 + h), (x0, y0 + h)];
    for (i, (x, y)) in corners.iter().enumerate() {
        let i = i as u128;
        sk.add_entity(SketchEntity::point(
            e(2 * i),
            Vec2::new_unchecked(*x, *y),
            false,
            false,
        ))
        .unwrap();
        let (nx, ny) = corners[(i as usize + 1) % 4];
        sk.add_entity(SketchEntity::point(
            e(2 * i + 1),
            Vec2::new_unchecked(nx, ny),
            false,
            false,
        ))
        .unwrap();
        sk.add_entity(SketchEntity::line(
            e(0x10 + i),
            e(2 * i),
            e(2 * i + 1),
            false,
        ))
        .unwrap();
    }
    for i in 0..4u128 {
        sk.add_constraint(Constraint::Coincident {
            id: c(i),
            point1: e(2 * i + 1),
            point2: e((2 * i + 2) % 8),
            point1_position: CurvePosition::Arbitrary,
            point2_position: CurvePosition::Arbitrary,
        })
        .unwrap();
    }
    sk
}

fn sketch_record(seed: u128, x0: f64, y0: f64, w: f64, h: f64) -> OperationRecord {
    let sk = rect_sketch(sketch_id(seed), seed * 0x100, x0, y0, w, h);
    let (_plane, entities, constraints) = sketch_wire(&sk);
    let params = SketchOpParams {
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
    };
    OperationRecord::new(
        rec(seed),
        0,
        "Sketch",
        Operation::Known(KnownOperation::Sketch(params)),
    )
}

fn extrude_op(sketch: u128, region: &str, distance: f64, target: Option<u128>) -> Operation {
    Operation::Known(KnownOperation::Extrude(ExtrudeParams {
        profile: Some(SketchRegionRef {
            sketch: sketch_id(sketch),
            region: RegionId::new(region),
            region_identity_version: None,
            region_anchor: None,
            extra: Default::default(),
        }),
        distance: Scalar::new(distance),
        draft_angle_deg: Scalar::new(0.0),
        mode: ExtrudeMode::Blind,
        boolean_mode: if target.is_some() {
            BooleanMode::Add
        } else {
            BooleanMode::NewBody
        },
        target_body: target.map(body),
        target_face: None,
        two_directions: false,
        mode2: ExtrudeMode::Blind,
        distance2: Scalar::new(0.0),
        target_face2: None,
        extra: Default::default(),
    }))
}

fn extrude_record(seed: u128, op: Operation) -> OperationRecord {
    OperationRecord::new(rec(seed), 0, "Extrude", op)
}

/// 0 SkA · 1 ExA(10) · 2 SkB · 3 boss B onto A (dependent) · 4 SkC · 5 ExC
/// (independent). The edit is ExA 10 → 5.
fn part_with_dependent() -> Vec<OperationRecord> {
    vec![
        sketch_record(SKETCH_A, 0.0, 0.0, 20.0, 20.0),
        extrude_record(EXTRUDE_A, extrude_op(SKETCH_A, "", 10.0, None)),
        sketch_record(SKETCH_B, 5.0, 5.0, 10.0, 10.0),
        extrude_record(BOSS_B, extrude_op(SKETCH_B, "", 25.0, Some(EXTRUDE_A))),
        sketch_record(SKETCH_C, 60.0, 0.0, 10.0, 10.0),
        extrude_record(EXTRUDE_C, extrude_op(SKETCH_C, "", 8.0, None)),
    ]
}

mod edit_preview {
    mod capability;
    mod equivalence;
    mod races;
}

#[tokio::test]
async fn canonical_preview_plan_fixture_replays_on_real_worker() {
    let Some(worker) = real_worker() else { return };
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    fixture_replay::replay(&worker, &root.join("protocol/fixtures/preview_plan.ndjson")).await;
}
