//! ONECAD-369 edit preview against the REAL C++ worker (SCHEMA §7.6 `PreviewPlan`).
//!
//! The service previews replacing an existing record's operation by a full,
//! non-publishing replay. What this file proves end to end:
//!
//! * preview == commit: the displayed body set, the last step's three §12
//!   signatures and the MESH1 bytes of every re-meshed body equal what the commit
//!   publishes — including the WP-D1 isolation case, where a downstream feature
//!   halts and an independent part above it still regenerates;
//! * a preview changes nothing: worker head, revision, dirty flag, undo depth;
//! * a stale fence is refused, a cancel mid-flight leaves no scratch and the next
//!   regen publishes, an edited-feature failure is classified at its own step, a
//!   worker death is a worker-class error, and save/reopen sees the unedited
//!   document;
//! * the canonical `preview_plan.ndjson` fixture replays from Rust.
//!
//! Run: `ONECAD_REQUIRE_WORKER=1 ONECAD_WORKER_PATH=… cargo test -p onecad-app --test edit_preview`.

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
    let wm = spawn_worker(real_worker()?).await;
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
    Some((wm, state))
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

// ── Tests ──────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preview_equals_commit_and_changes_nothing() {
    let Some((_wm, state)) = document(part_with_dependent()).await else {
        return;
    };
    let before = head_state(&state).await;
    let edited = extrude_op(SKETCH_A, "", 5.0, None);
    let preview = preview(&state, EXTRUDE_A, edited.clone())
        .await
        .expect("preview");

    assert_eq!(preview.outcome, EditPreviewOutcome::Ready);
    assert!(!preview.isolated);
    let meshed: Vec<&str> = preview.meshes.iter().map(|m| m.body_id.as_str()).collect();
    assert_eq!(
        meshed,
        [body(EXTRUDE_A).to_string()],
        "only the edited part is re-meshed"
    );
    assert_eq!(
        preview.suppressed,
        vec![body(EXTRUDE_A)],
        "its old version is hidden"
    );
    assert_eq!(
        head_state(&state).await,
        before,
        "a preview publishes nothing"
    );

    let snap = commit(&state, EXTRUDE_A, edited).await;
    assert_preview_equals_commit(&state, &preview, &snap).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_downstream_halt_previews_the_isolated_commit() {
    let mut records = part_with_dependent();
    // A broken feature on sketch A between the parts: the halt is downstream of
    // the edit, and part C above it is provably independent (WP-D1).
    records.insert(
        4,
        extrude_record(BROKEN, extrude_op(SKETCH_A, "no-such-region", 3.0, None)),
    );
    let Some((_wm, state)) = document(records).await else {
        return;
    };
    let edited = extrude_op(SKETCH_A, "", 5.0, None);
    let preview = preview(&state, EXTRUDE_A, edited.clone())
        .await
        .expect("preview");

    assert!(
        matches!(preview.outcome, EditPreviewOutcome::DownstreamFailed { step: 4, record, .. } if record == rec(BROKEN)),
        "{:?}",
        preview.outcome
    );
    assert!(
        preview.isolated,
        "the isolation pass supplies the displayed bodies"
    );
    assert!(
        preview.bodies.contains(&body(EXTRUDE_C)),
        "{:?}",
        preview.bodies
    );

    let snap = commit(&state, EXTRUDE_A, edited).await;
    assert_preview_equals_commit(&state, &preview, &snap).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_edited_feature_failure_is_reported_at_its_own_step() {
    let Some((_wm, state)) = document(part_with_dependent()).await else {
        return;
    };
    let before = head_state(&state).await;
    let broken = extrude_op(SKETCH_A, "no-such-region", 5.0, None);
    let preview = preview(&state, EXTRUDE_A, broken).await.expect("preview");
    assert!(
        matches!(
            preview.outcome,
            EditPreviewOutcome::EditedFailed {
                needs_repair: false,
                ..
            }
        ),
        "{:?}",
        preview.outcome
    );
    assert_eq!(head_state(&state).await, before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stale_fence_is_refused() {
    let Some((_wm, state)) = document(part_with_dependent()).await else {
        return;
    };
    let fence = api::preview_context(&state).await.unwrap();
    commit(&state, EXTRUDE_C, extrude_op(SKETCH_C, "", 9.0, None)).await;
    let stale = api::preview_existing_edit(
        &state,
        fence,
        rec(EXTRUDE_A),
        extrude_op(SKETCH_A, "", 5.0, None),
        Lod::Coarse,
    )
    .await;
    assert!(matches!(stale, Err(ApiError::StalePreview(_))), "{stale:?}");
}

/// Twenty fused bosses: long enough that a cancel lands mid-replay.
fn heavy_part() -> Vec<OperationRecord> {
    let mut records = vec![
        sketch_record(SKETCH_A, 0.0, 0.0, 40.0, 40.0),
        extrude_record(EXTRUDE_A, extrude_op(SKETCH_A, "", 10.0, None)),
    ];
    for i in 0..20u128 {
        let sketch = 0x700 + 2 * i;
        let x = 1.0 + (i % 5) as f64 * 7.5;
        let y = 1.0 + (i / 5) as f64 * 9.0;
        records.push(sketch_record(sketch, x, y, 5.0, 5.0));
        records.push(extrude_record(
            sketch + 1,
            extrude_op(sketch, "", 12.0 + i as f64, Some(EXTRUDE_A)),
        ));
    }
    records
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn apply_cancels_an_inflight_preview_and_publishes() {
    let Some((wm, state)) = document(heavy_part()).await else {
        return;
    };
    let state = Arc::new(state);
    // The uncancelled replay must take several times the cancel delay, or the
    // "mid-flight" below would really be "after the fact".
    let started = std::time::Instant::now();
    preview(&state, EXTRUDE_A, extrude_op(SKETCH_A, "", 5.0, None))
        .await
        .expect("calibration preview");
    let full = started.elapsed();
    eprintln!("heavy preview replay took {full:?}");
    assert!(
        full >= Duration::from_millis(160),
        "replay too short to cancel mid-flight: {full:?}"
    );
    let task = {
        let state = state.clone();
        tokio::spawn(async move {
            preview(&state, EXTRUDE_A, extrude_op(SKETCH_A, "", 6.0, None)).await
        })
    };
    tokio::time::sleep(Duration::from_millis(40)).await;
    // Apply: `begin_regen` preempts the preview before the commit plan is sent.
    let snap = commit(&state, EXTRUDE_A, extrude_op(SKETCH_A, "", 7.0, None)).await;
    let result = task.await.unwrap();
    assert!(
        matches!(
            result,
            Err(ApiError::Cancelled(_) | ApiError::StalePreview(_))
        ),
        "a preempted preview never reports Ready: {result:?}"
    );
    let head = wm.get_worker_head().await.unwrap();
    assert!(!head.has_scratch, "a cancelled preview leaves no scratch");
    assert_eq!(head.snapshot_id, snap.id, "the commit published");
    // And the lane is free: a later preview runs to completion.
    let again = preview(&state, EXTRUDE_A, extrude_op(SKETCH_A, "", 8.0, None)).await;
    assert!(
        matches!(again, Ok(ref p) if p.outcome == EditPreviewOutcome::Ready),
        "{again:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_worker_death_is_a_worker_class_error() {
    let Some((wm, state)) = document(heavy_part()).await else {
        return;
    };
    let state = Arc::new(state);
    let task = {
        let state = state.clone();
        tokio::spawn(async move {
            preview(&state, EXTRUDE_A, extrude_op(SKETCH_A, "", 6.0, None)).await
        })
    };
    tokio::time::sleep(Duration::from_millis(40)).await;
    wm.retire();
    let result = task.await.unwrap();
    assert!(matches!(result, Err(ApiError::Worker(_))), "{result:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn save_and_reopen_see_the_unedited_document() {
    let Some((wm, state)) = document(part_with_dependent()).await else {
        return;
    };
    let signatures_before = {
        let guard = state.runtime.lock().await;
        guard
            .as_ref()
            .unwrap()
            .subscribe_snapshots()
            .borrow()
            .clone()
            .unwrap()
            .signatures
            .clone()
    };
    preview(&state, EXTRUDE_A, extrude_op(SKETCH_A, "", 5.0, None))
        .await
        .expect("preview");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("previewed.onecad");
    {
        let mut guard = state.runtime.lock().await;
        let meta = SaveMeta {
            app_version: "0.1.0-test".into(),
            occt_fingerprint: None,
            created: "2026-10-21T00:00:00Z".into(),
            modified: "2026-10-21T00:00:00Z".into(),
        };
        guard.as_mut().unwrap().save(&path, meta).expect("save");
    }
    let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
    let mut reopened = DocumentRuntime::open(&path, engine, meshes, solver).expect("reopen");
    let report = reopened
        .run_regen(RegenRequest::RevertToEnd { from: 0 }, CancelToken::new())
        .await;
    let Outcome::Published(snap) = report.outcome else {
        panic!("reopen regen must publish: {:?}", report.outcome);
    };
    assert_eq!(
        snap.signatures, signatures_before,
        "the saved document is the unedited one"
    );
}

#[tokio::test]
async fn canonical_preview_plan_fixture_replays_on_real_worker() {
    let Some(worker) = real_worker() else { return };
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    fixture_replay::replay(&worker, &root.join("protocol/fixtures/preview_plan.ndjson")).await;
}
