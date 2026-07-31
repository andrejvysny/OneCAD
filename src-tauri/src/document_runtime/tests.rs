//! Unit tests for [`DocumentRuntime`] driven by a local scripted backend.
//!
//! The `FakeBackend` implements both [`GeometryEngine`] and
//! [`MeshProvider`](crate::worker::MeshProvider) with no OCCT: each op creates a
//! deterministic body (`BodyId(opId.uuid)` — the D1 `body_<opId>` rule in the
//! core's UUID space) unless overridden, echoes the plan's opaque history-prefix
//! token the executor verifies, and serves canned MESH1 bytes per body.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::sync::mpsc;
use uuid::Uuid;

use onecad_core::document::body::{BodyLifecycleEvent, BodyRegistry};
use onecad_core::document::record::{
    BooleanMode, ExtrudeMode, ExtrudeParams, FilletParams, KnownOperation, Operation,
    OperationRecord, RevolveParams,
};
use onecad_core::document::variables::Scalar;
use onecad_core::edit::{EditCommand, RegenHint, SketchEditOp, VisibilityTarget};
use onecad_core::history::{StepState, Timeline};
use onecad_core::ids::{
    BodyId, DocumentId, DocumentRevision, JobId, RecordId, SnapshotId, WorkerEpoch,
};
use onecad_core::regen::{
    AcceptResult, AcquireRequest, CheckpointArtifacts, ElementMapDelta, EngineError, Fencing,
    GeometryEngine, HistoryPrefixHash, Lod, OpFailureCode, OpenSessionRequest, Outcome, PlanEvent,
    PlanPrepared, PlanRequest, PlanStepEvent, RefResolution, RegenRequest, ResolveRequest,
    RestoreRequest, RestoreResult, Signature, StepResult, StepSignatures, StepStatus,
    StoppedReason, TessellateRequest, TessellateResult, WorkerElementEvidence, WorkerHead,
};

use onecad_core::document::refs::{
    AnchorIntent, AxisRef, ElementKind, ElementRef, PrimaryRef, SketchRegionRef,
};
use onecad_core::ids::{ElementId, EntityId, RegionId, SketchId, TopoKey};
use onecad_core::math::{Vec2, Vec3};
use onecad_core::sketch::{Sketch, SketchEntity, WorldPlane};

use super::*;
use crate::dto::{
    BeginGestureDto, DragSolveDto, SketchRegionDto, SketchSolveStatus, SketchUpsertDto,
};
use crate::worker::{MeshProvider, SolverEngine};

// ─────────────────────────────────────────────────────────────────────────────
// Scripted backend
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Default)]
struct FakeState {
    prepared: HashMap<JobId, SnapshotId>,
    snapshot_counter: u64,
    /// Active gestures: `gestureId → (dragPoint id string, last target)`.
    gestures: HashMap<u64, (String, [f64; 2])>,
}

struct FakeBackend {
    /// Per-step created-body overrides; a step without an entry creates one body
    /// `BodyId(opId.uuid)` (the deterministic D1 id).
    body_overrides: HashMap<usize, Vec<BodyId>>,
    /// When set, the solver lane (`sketch_upsert`) hard-fails — models a worker error
    /// on `enter_sketch` (finding 2c: a failed enter must open NO session).
    solver_fails: bool,
    /// When set, `execute_plan` emits a single `Failed` terminal — models a hard regen
    /// failure (finding 5: EngineFailed-while-superseded downgrades to Superseded).
    plan_fails: bool,
    state: Mutex<FakeState>,
}

impl Default for FakeBackend {
    fn default() -> Self {
        Self {
            body_overrides: HashMap::new(),
            solver_fails: false,
            plan_fails: false,
            state: Mutex::new(FakeState::default()),
        }
    }
}

impl FakeBackend {
    fn new() -> Self {
        Self::default()
    }

    fn with_overrides(overrides: HashMap<usize, Vec<BodyId>>) -> Self {
        Self {
            body_overrides: overrides,
            ..Self::default()
        }
    }

    /// A backend whose solver lane fails every `sketch_upsert`.
    fn with_failing_solver() -> Self {
        Self {
            solver_fails: true,
            ..Self::default()
        }
    }

    /// A backend whose `execute_plan` hard-fails (a `Failed` terminal).
    fn with_failing_plan() -> Self {
        Self {
            plan_fails: true,
            ..Self::default()
        }
    }

    fn bodies_for(&self, step: usize, record: RecordId) -> Vec<BodyId> {
        self.body_overrides
            .get(&step)
            .cloned()
            .unwrap_or_else(|| vec![BodyId(record.as_uuid())])
    }
}

fn sigs(step: usize) -> StepSignatures {
    StepSignatures {
        geometry: Signature::new(format!("g{step}")),
        body_lifecycle: Signature::new(format!("b{step}")),
        referenced_binding: Signature::new(format!("r{step}")),
    }
}

/// The opaque history-prefix token a well-behaved worker echoes (mirrors the
/// executor's expectation, so verification passes by construction).
fn echo_hash(request: &PlanRequest, last_valid: Option<usize>) -> HistoryPrefixHash {
    match last_valid {
        Some(step) => request
            .ops
            .iter()
            .position(|o| o.step_index == step)
            .and_then(|j| request.prefix_hashes.get(j).cloned())
            .unwrap_or_else(|| request.expected_base_hash.clone()),
        None => request.expected_base_hash.clone(),
    }
}

#[async_trait]
impl GeometryEngine for FakeBackend {
    async fn execute_plan(&self, request: PlanRequest) -> mpsc::Receiver<PlanEvent> {
        // Finding 5: a hard regen failure — a single `Failed` terminal, no steps.
        let events = if self.plan_fails {
            vec![PlanEvent::Failed(EngineError::OpFailed {
                code: OpFailureCode::OpFailed,
                recoverable: false,
                message: "fake plan failure".into(),
            })]
        } else {
            let mut st = self.state.lock().unwrap();
            st.snapshot_counter += 1;
            let snapshot_id = SnapshotId(5000 + st.snapshot_counter);
            let job = request.job_id;

            let mut events = Vec::new();
            let mut per_step: Vec<StepResult> = Vec::new();
            let mut last_valid: Option<usize> = None;
            for op in &request.ops {
                let step = op.step_index;
                let body_ids = self.bodies_for(step, op.record_id);
                let body_events: Vec<BodyLifecycleEvent> = body_ids
                    .iter()
                    .map(|b| BodyLifecycleEvent::Created { body: *b })
                    .collect();
                events.push(PlanEvent::Step(PlanStepEvent {
                    step_index: step,
                    body_events,
                    element_map_delta: ElementMapDelta::default(),
                    needs_repair: vec![],
                    signatures: sigs(step),
                    diagnostics: vec![],
                }));
                per_step.push(StepResult {
                    step_index: step,
                    status: StepStatus::Ok,
                    body_ids,
                    message: String::new(),
                });
                last_valid = Some(step);
            }
            st.prepared.insert(job, snapshot_id);
            events.push(PlanEvent::Prepared(PlanPrepared {
                job_id: job,
                prepared_snapshot_id: snapshot_id,
                last_valid_step: last_valid,
                stopped_reason: StoppedReason::Completed,
                per_step,
                history_prefix_hash: echo_hash(&request, last_valid),
            }));
            events
        };

        let (tx, rx) = mpsc::channel(64);
        tokio::spawn(async move {
            for ev in events {
                if tx.send(ev).await.is_err() {
                    return;
                }
            }
        });
        rx
    }

    async fn accept_prepared(
        &self,
        job_id: JobId,
        fencing: Fencing,
    ) -> Result<AcceptResult, EngineError> {
        let mut st = self.state.lock().unwrap();
        let snapshot_id = st.prepared.remove(&job_id).unwrap_or(SnapshotId(0));
        Ok(AcceptResult {
            snapshot_id,
            document_revision: DocumentRevision(fencing.document_revision.0 + 1),
        })
    }

    async fn discard_prepared(&self, job_id: JobId) -> Result<(), EngineError> {
        self.state.lock().unwrap().prepared.remove(&job_id);
        Ok(())
    }

    async fn open_session(&self, req: OpenSessionRequest) -> Result<WorkerHead, EngineError> {
        Ok(WorkerHead {
            document_revision: req.document_revision,
            worker_epoch: req.worker_epoch,
            snapshot_id: SnapshotId(0),
            history_prefix_hash: HistoryPrefixHash::empty(),
            has_scratch: false,
        })
    }
    async fn close_session(&self, _d: DocumentId, _e: WorkerEpoch) -> Result<(), EngineError> {
        Ok(())
    }
    async fn reset(&self, _d: DocumentId, e: WorkerEpoch) -> Result<WorkerEpoch, EngineError> {
        Ok(WorkerEpoch(e.0 + 1))
    }
    async fn get_worker_head(&self) -> Result<WorkerHead, EngineError> {
        Ok(WorkerHead {
            document_revision: DocumentRevision(0),
            worker_epoch: WorkerEpoch(1),
            snapshot_id: SnapshotId(0),
            history_prefix_hash: HistoryPrefixHash::empty(),
            has_scratch: false,
        })
    }
    async fn tessellate(&self, _r: TessellateRequest) -> Result<TessellateResult, EngineError> {
        Ok(TessellateResult { meshes: vec![] })
    }
    async fn save_checkpoint(&self, _s: usize) -> Result<CheckpointArtifacts, EngineError> {
        Err(EngineError::OpFailed {
            code: OpFailureCode::Unsupported,
            recoverable: true,
            message: "fake".into(),
        })
    }
    async fn restore_checkpoint(&self, _r: RestoreRequest) -> Result<RestoreResult, EngineError> {
        Err(EngineError::Protocol {
            message: "fake has no checkpoints".into(),
        })
    }
    async fn acquire_element_ids(
        &self,
        r: AcquireRequest,
    ) -> Result<Vec<WorkerElementEvidence>, EngineError> {
        // Echo one evidence entry per pick (empty `existing` — Rust mints the id).
        Ok(r.picks
            .into_iter()
            .map(|p| WorkerElementEvidence {
                topo_key: p.topo_key,
                body: r.body,
                kind: onecad_core::document::refs::ElementKind::Face,
                anchor: p.anchor,
                descriptor: Some(serde_json::json!({ "fake": true })),
                existing: None,
            })
            .collect())
    }
    async fn resolve_refs(&self, _r: ResolveRequest) -> Result<Vec<RefResolution>, EngineError> {
        Ok(vec![])
    }
    async fn cancel(&self, _j: JobId) -> Result<(), EngineError> {
        Ok(())
    }
    async fn ping(&self) -> Result<(), EngineError> {
        Ok(())
    }
}

#[async_trait]
impl MeshProvider for FakeBackend {
    async fn fetch_mesh(
        &self,
        body: BodyId,
        lod: Lod,
        _snapshot: SnapshotId,
    ) -> Result<Vec<u8>, EngineError> {
        Ok(format!("MESH1:{body}:{}", crate::worker::lod_str(lod)).into_bytes())
    }
}

#[async_trait]
impl SolverEngine for FakeBackend {
    async fn sketch_upsert(&self, sketch: &Sketch) -> Result<SketchUpsertDto, EngineError> {
        if self.solver_fails {
            return Err(EngineError::OpFailed {
                code: OpFailureCode::OpFailed,
                recoverable: true,
                message: "fake solver failure".into(),
            });
        }
        Ok(SketchUpsertDto {
            sketch_id: sketch.id.to_string(),
            sketch_revision: 1,
            dof: 0,
            status: SketchSolveStatus::FullyConstrained,
            conflicting: vec![],
            solved_positions: std::collections::BTreeMap::new(),
        })
    }
    async fn begin_gesture(
        &self,
        _sketch_id: &str,
        _sketch_revision: u64,
        gesture_id: u64,
        drag_point: EntityId,
        _solver_policy_hash: &str,
    ) -> Result<BeginGestureDto, EngineError> {
        self.state
            .lock()
            .unwrap()
            .gestures
            .insert(gesture_id, (drag_point.to_string(), [0.0, 0.0]));
        Ok(BeginGestureDto {
            gesture_id,
            ready: true,
        })
    }
    async fn solve_drag(
        &self,
        gesture_id: u64,
        seq: u64,
        drag_point: EntityId,
        target: [f64; 2],
    ) -> Result<DragSolveDto, EngineError> {
        let mut positions = std::collections::BTreeMap::new();
        if let Some(g) = self.state.lock().unwrap().gestures.get_mut(&gesture_id) {
            g.1 = target;
        }
        positions.insert(drag_point.to_string(), target);
        Ok(DragSolveDto {
            gesture_id,
            seq,
            status: "success".into(),
            dof: 0,
            conflicting: vec![],
            positions,
            solve_micros: 0,
            superseded: false,
        })
    }
    async fn end_gesture(
        &self,
        sketch_id: &str,
        gesture_id: u64,
        final_target: Option<[f64; 2]>,
    ) -> Result<SketchUpsertDto, EngineError> {
        let (drag_point, last) = self
            .state
            .lock()
            .unwrap()
            .gestures
            .remove(&gesture_id)
            .unwrap_or_default();
        let pos = final_target.unwrap_or(last);
        let mut solved = std::collections::BTreeMap::new();
        if !drag_point.is_empty() {
            solved.insert(drag_point, pos);
        }
        Ok(SketchUpsertDto {
            sketch_id: sketch_id.to_string(),
            sketch_revision: 2,
            dof: 0,
            status: SketchSolveStatus::FullyConstrained,
            conflicting: vec![],
            solved_positions: solved,
        })
    }
    async fn sketch_regions(&self, _sketch_id: &str) -> Result<Vec<SketchRegionDto>, EngineError> {
        Ok(vec![])
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Fixtures
// ─────────────────────────────────────────────────────────────────────────────

fn extrude_record(seed: u128, distance: f64) -> OperationRecord {
    let op = Operation::Known(KnownOperation::Extrude(ExtrudeParams {
        profile: None,
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
    OperationRecord::new(RecordId(Uuid::from_u128(seed)), 0, "Extrude", op)
}

fn add_extrude(seed: u128, distance: f64) -> EditCommand {
    EditCommand::AddOperation {
        record: extrude_record(seed, distance),
        at_cursor: true,
    }
}

/// A revolve record carrying a NON-DEFAULT sketch-line axis (the re-edit case whose
/// axis a whole-params replace would clobber).
fn revolve_record(seed: u128, angle: f64, line: u128) -> OperationRecord {
    let op = Operation::Known(KnownOperation::Revolve(RevolveParams {
        profile: None,
        angle_deg: Scalar::new(angle),
        axis: Some(AxisRef::SketchLine {
            sketch: SketchId(Uuid::from_u128(0x5c)),
            line: EntityId(Uuid::from_u128(line)),
            extra: Default::default(),
        }),
        boolean_mode: BooleanMode::NewBody,
        target_body: None,
        extra: Default::default(),
    }));
    OperationRecord::new(RecordId(Uuid::from_u128(seed)), 0, "Revolve", op)
}

/// A single-edge fillet record (a typed edge ref: `primary.element` == `edge_ids[0]`
/// + a world-midpoint anchor) — the SCHEMA §7.3 `inputs[]` shape.
fn fillet_record(seed: u128, body: BodyId, edge: &str, anchor: Vec3) -> OperationRecord {
    let el = ElementId::new(edge);
    let edge_ref = ElementRef {
        primary: Some(PrimaryRef {
            body,
            element: el.clone(),
            kind: ElementKind::Edge,
            extra: Default::default(),
        }),
        intent: None,
        anchor: Some(AnchorIntent {
            world_point: anchor,
            surface_uv: None,
            local_frame: None,
            adjacency_hint: None,
            extra: Default::default(),
        }),
        extra: Default::default(),
    };
    let op = Operation::Known(KnownOperation::Fillet(FilletParams {
        radius: Scalar::new(2.0),
        edge_ids: vec![el],
        edges: vec![edge_ref],
        chain_tangent_edges: false,
        extra: Default::default(),
    }));
    OperationRecord::new(RecordId(Uuid::from_u128(seed)), 0, "Fillet", op)
}

fn runtime_with(backend: Arc<FakeBackend>) -> DocumentRuntime {
    let engine: Arc<dyn GeometryEngine> = backend.clone();
    let meshes: Arc<dyn MeshProvider> = backend.clone();
    let solver: Arc<dyn SolverEngine> = backend;
    DocumentRuntime::new_blank(engine, meshes, solver)
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn apply_then_regen_publishes_body_and_marks_feature_ok() {
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    let out = rt.apply(add_extrude(0x10, 25.0)).unwrap();
    assert!(matches!(out.regen, onecad_core::edit::RegenHint::ToEnd));

    let report = rt
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    assert!(
        matches!(report.outcome, Outcome::Published(_)),
        "{:?}",
        report.outcome
    );

    let proj = rt.projection();
    let body = BodyId(Uuid::from_u128(0x10)).to_string();
    assert!(proj.bodies.contains_key(&body), "regen body in projection");
    assert_eq!(proj.features.len(), 1);
    assert_eq!(proj.features[0].value_text, "25.0 mm");
    assert_eq!(proj.features[0].status, crate::dto::FeatureStatus::Ok);
    assert!(proj.dirty);
}

#[tokio::test]
async fn projection_prefers_document_body_metadata() {
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    rt.apply(add_extrude(0x10, 25.0)).unwrap();
    rt.run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    let body = BodyId(Uuid::from_u128(0x10));

    // The regen minted the body; adoption gave it a document row, so a body command
    // can address it at all.
    rt.apply(EditCommand::RenameBody {
        body,
        name: "Bracket".into(),
    })
    .unwrap();
    rt.apply(EditCommand::SetVisibility {
        target: VisibilityTarget::Body(body),
        visible: false,
    })
    .unwrap();

    // The regen row is untouched — the projection is an OVERLAY, not a mutation of
    // the mirror (which the next regen would overwrite anyway).
    assert_ne!(rt.regen.bodies.get(body).unwrap().name, "Bracket");
    assert!(rt.regen.bodies.get(body).unwrap().visible);

    let dto = rt.projection().bodies.remove(&body.to_string()).unwrap();
    assert_eq!(dto.name, "Bracket", "the document's name wins");
    assert!(!dto.visible, "the document's visibility wins");

    // A from-0 regen rebuilds the registry from `BodyRegistry::new()` (default name,
    // visible) — user intent must still win, and it is what a save writes.
    rt.run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    let dto = rt.projection().bodies.remove(&body.to_string()).unwrap();
    assert_eq!(dto.name, "Bracket");
    assert!(!dto.visible);
    let saved = rt.merged_bodies();
    let meta = saved.get(body).unwrap();
    assert_eq!((meta.name.as_str(), meta.visible), ("Bracket", false));
}

#[tokio::test]
async fn undo_redo_round_trips_the_timeline() {
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    rt.apply(add_extrude(0x10, 10.0)).unwrap();
    rt.apply(add_extrude(0x11, 20.0)).unwrap();
    assert_eq!(rt.projection().features.len(), 2);

    assert!(rt.undo(), "undo removes the second op");
    assert_eq!(rt.projection().features.len(), 1);

    assert!(rt.redo().unwrap(), "redo re-applies it");
    assert_eq!(rt.projection().features.len(), 2);

    // The redo re-executed the forward command → revision advanced past the apply.
    assert!(rt.revision().0 >= 4);
}

#[tokio::test]
async fn d1_adoption_rejects_malformed_body_id() {
    // Op 0x10 mints a body whose id is NOT derived from any known opId.
    let mut overrides = HashMap::new();
    overrides.insert(0usize, vec![BodyId(Uuid::from_u128(0xBAD))]);
    let mut rt = runtime_with(Arc::new(FakeBackend::with_overrides(overrides)));
    rt.apply(add_extrude(0x10, 10.0)).unwrap();

    let report = rt
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    match report.outcome {
        Outcome::EngineFailed(EngineError::Protocol { .. }) => {}
        other => panic!("malformed body must reject the plan, got {other:?}"),
    }
    // Nothing published: no body, no document-changed payload.
    assert!(rt.projection().bodies.is_empty());
    assert!(report.document_change().is_none());
}

#[tokio::test]
async fn d1_adoption_rejects_colliding_body_id() {
    // Two ops (0x10, 0x11); the second re-mints op-0's body id → collision.
    let mut overrides = HashMap::new();
    overrides.insert(1usize, vec![BodyId(Uuid::from_u128(0x10))]);
    let mut rt = runtime_with(Arc::new(FakeBackend::with_overrides(overrides)));
    rt.apply(add_extrude(0x10, 10.0)).unwrap();
    rt.apply(add_extrude(0x11, 20.0)).unwrap();

    let report = rt
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    assert!(
        matches!(
            report.outcome,
            Outcome::EngineFailed(EngineError::Protocol { .. })
        ),
        "collision must reject the plan, got {:?}",
        report.outcome
    );
    assert!(rt.projection().bodies.is_empty());
}

#[tokio::test]
async fn mesh_cache_miss_then_hit_returns_identical_bytes() {
    let backend = Arc::new(FakeBackend::new());
    let mut rt = runtime_with(backend);
    rt.apply(add_extrude(0x10, 10.0)).unwrap();
    rt.run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;

    let body = BodyId(Uuid::from_u128(0x10));
    let first = rt
        .get_mesh(body, Lod::Coarse, None)
        .await
        .expect("miss → fetch");
    let expected = b"MESH1:00000000-0000-0000-0000-000000000010:coarse".to_vec();
    assert_eq!(*first, expected, "provider bytes served verbatim");

    let second = rt
        .get_mesh(body, Lod::Coarse, None)
        .await
        .expect("cache hit");
    assert!(
        Arc::ptr_eq(&first, &second),
        "hit returns the same cached Arc"
    );
}

#[tokio::test]
async fn regen_report_builds_document_change_payload() {
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    rt.apply(add_extrude(0x10, 10.0)).unwrap();
    let report = rt
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;

    let change = report
        .document_change()
        .expect("published → change payload");
    assert_eq!(change.changed_bodies.len(), 1);
    let ref_ = &change.changed_bodies[0];
    assert_eq!(ref_.body_id, "00000000-0000-0000-0000-000000000010");
    // meshKey = "<bodyId>:<lod>:<generation>" (matches the mock's mockMeshKey).
    assert!(
        ref_.mesh_key
            .starts_with(&format!("{}:coarse:", ref_.body_id)),
        "{}",
        ref_.mesh_key
    );
    assert!(change.removed_bodies.is_empty());
}

#[tokio::test]
async fn get_mesh_without_geometry_is_a_miss() {
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    // No regen yet → no snapshot → get_mesh returns None (not a panic).
    let got = rt
        .get_mesh(BodyId(Uuid::from_u128(0x10)), Lod::Coarse, None)
        .await;
    assert!(got.is_none());
}

#[tokio::test]
async fn save_then_reopen_round_trips_the_document() {
    use onecad_core::io::container::SaveMeta;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("model.onecad");

    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    rt.apply(add_extrude(0x10, 25.0)).unwrap();
    rt.run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;

    let meta = SaveMeta {
        app_version: "0.1.0-test".into(),
        occt_fingerprint: None,
        created: "2026-07-17T00:00:00Z".into(),
        modified: "2026-07-17T00:00:00Z".into(),
    };
    rt.save(&path, meta).unwrap();
    assert!(!rt.is_dirty(), "save clears the dirty flag");

    // Reopen with a fresh backend: the timeline (feature) + merged geometry body
    // survive the round-trip; a reopened document starts clean.
    let backend = Arc::new(FakeBackend::new());
    let engine: Arc<dyn GeometryEngine> = backend.clone();
    let meshes: Arc<dyn MeshProvider> = backend.clone();
    let solver: Arc<dyn SolverEngine> = backend;
    let reopened = DocumentRuntime::open(&path, engine, meshes, solver).unwrap();
    let proj = reopened.projection();
    assert_eq!(proj.features.len(), 1);
    assert_eq!(proj.features[0].value_text, "25.0 mm");
    assert!(
        proj.bodies
            .contains_key(&BodyId(Uuid::from_u128(0x10)).to_string()),
        "merged regen body persisted"
    );
    assert!(!reopened.is_dirty());
}

#[tokio::test]
async fn sequential_from_zero_regens_wholesale_replace_no_d1_false_positive() {
    // D5 / D1-vs-from-0: two sequential regen cycles where cycle 1 PUBLISHES a body,
    // then cycle 2 (a full replay-from-0 plan) re-creates that SAME `body_<opId>` id
    // plus a new one. The D1 uniqueness check must NOT false-positive against the
    // previous cycle's published body — `begin_regen` seeds the AdoptingEngine's
    // `existing` set EMPTY because a from-0 replay's base is empty, so only in-plan
    // duplicates are rejected. Cycle 2 must publish and REPLACE the head wholesale.
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));

    // Cycle 1 — publish body 0x10 (sets latest_snapshot, the source of `prior`).
    rt.apply(add_extrude(0x10, 25.0)).unwrap();
    let r1 = rt
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    assert!(
        matches!(r1.outcome, Outcome::Published(_)),
        "{:?}",
        r1.outcome
    );
    assert_eq!(
        rt.projection().bodies.len(),
        1,
        "cycle 1 published one body"
    );

    // "Edit": append op 0x11. Cycle 2 is a from-0 replay of BOTH ops → it re-creates
    // body_<0x10> (already in the published head) and mints body_<0x11>.
    rt.apply(add_extrude(0x11, 10.0)).unwrap();
    let r2 = rt
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    assert!(
        matches!(r2.outcome, Outcome::Published(_)),
        "cycle 2 must publish — re-creating the prior cycle's body_<opId> must NOT \
         trip the D1 uniqueness check (from-0 base is empty), got {:?}",
        r2.outcome
    );
    // Wholesale replace: head = {body_0x10, body_0x11}, no duplication of body_0x10.
    let bodies = rt.projection().bodies;
    assert_eq!(bodies.len(), 2, "cycle 2 head has exactly two bodies");
    assert!(bodies.contains_key(&BodyId(Uuid::from_u128(0x10)).to_string()));
    assert!(bodies.contains_key(&BodyId(Uuid::from_u128(0x11)).to_string()));
}

#[tokio::test]
async fn edit_during_regen_supersedes_via_live_fencing() {
    // R-WP11 fencing-live: begin_regen captures the tokens; an edit that lands
    // before the (lock-free) drive commits bumps the shared FencingCell, so the
    // executor's gate supersedes the stale prepare — nothing partial is published.
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    rt.apply(add_extrude(0x10, 25.0)).unwrap();

    let prepared = rt
        .begin_regen(RegenRequest::ToEnd { from: 0 })
        .expect("non-empty plan");
    // The concurrent edit (bumps the fencing revision the gate reads).
    rt.apply(add_extrude(0x11, 10.0)).unwrap();
    let driven = prepared.drive(CancelToken::new()).await;
    let report = rt.finish_regen(driven);

    assert!(
        matches!(report.outcome, Outcome::Superseded),
        "the stale prepare must be superseded, got {:?}",
        report.outcome
    );
    assert!(report.document_change().is_none(), "nothing published");

    // A fresh regen at the new revision converges to both bodies.
    let converge = rt
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    assert!(matches!(converge.outcome, Outcome::Published(_)));
    assert_eq!(rt.projection().bodies.len(), 2, "converged after supersede");
}

// ─────────────────────────────────────────────────────────────────────────────
// Sketch solver lane + promotion (R-WP12)
// ─────────────────────────────────────────────────────────────────────────────

fn sketch_with_point() -> (Sketch, EntityId) {
    let sid = SketchId(Uuid::from_u128(0x5c));
    let p = EntityId(Uuid::from_u128(0x100));
    let mut sk = Sketch::on_world_plane(sid, "Sketch 1", WorldPlane::XY);
    sk.add_entity(SketchEntity::point(
        p,
        Vec2::new_unchecked(0.0, 0.0),
        false,
        false,
    ))
    .unwrap();
    (sk, p)
}

fn point_at(rt: &DocumentRuntime, sid: SketchId, p: EntityId) -> [f64; 2] {
    match rt
        .session
        .document()
        .sketch(sid)
        .unwrap()
        .get_entity(p)
        .unwrap()
    {
        SketchEntity::Point { at, .. } => [at.x, at.y],
        _ => panic!("not a point"),
    }
}

#[test]
fn get_sketch_reads_geometry_without_a_worker_call() {
    // Sync fn, no `.await` — the signature itself proves get_sketch cannot reach
    // the solver (SolverEngine's methods are all async). Never entered/upserted ⇒
    // the "not yet solved" default (dof:0/UnderConstrained), mirroring the
    // SketchSolveStatus::parse fallback.
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    let (sk, _point) = sketch_with_point();
    let sid = sk.id;
    rt.apply(EditCommand::AddSketch { sketch: sk }).unwrap();

    let session = rt.get_sketch(sid).unwrap();
    assert_eq!(session.sketch_id, sid.to_string());
    assert!(
        matches!(session.entities, serde_json::Value::Array(ref a) if !a.is_empty()),
        "entities carry the added point"
    );
    assert_eq!(session.dof, 0);
    assert_eq!(session.status, SketchSolveStatus::UnderConstrained);
}

#[test]
fn projection_sketch_geometry_token_tracks_only_authoritative_geometry() {
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    let (sk, _) = sketch_with_point();
    let sid = sk.id;
    rt.apply(EditCommand::AddSketch { sketch: sk }).unwrap();

    let initial = rt.projection().sketches[&sid.to_string()]
        .geometry_token
        .clone();
    assert_eq!(initial.len(), 64, "SHA-256 token is lowercase hex");

    rt.apply(EditCommand::RenameSketch {
        sketch: sid,
        name: "Renamed".into(),
    })
    .unwrap();
    assert_eq!(
        rt.projection().sketches[&sid.to_string()].geometry_token,
        initial,
        "metadata-only edits do not invalidate static geometry"
    );

    rt.apply(EditCommand::SketchEdit {
        sketch: sid,
        ops: vec![SketchEditOp::AddEntity {
            entity: SketchEntity::point(
                EntityId(Uuid::from_u128(0x101)),
                Vec2::new_unchecked(2.0, 3.0),
                false,
                false,
            ),
        }],
    })
    .unwrap();
    assert_ne!(
        rt.projection().sketches[&sid.to_string()].geometry_token,
        initial,
        "geometry edits invalidate the profile cache"
    );

    assert!(rt.undo());
    assert_eq!(
        rt.projection().sketches[&sid.to_string()].geometry_token,
        initial,
        "undo restores the deterministic geometry token"
    );
}

#[tokio::test]
async fn get_sketch_reflects_the_last_solver_lane_solve() {
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    let (sk, _point) = sketch_with_point();
    let sid = sk.id;
    rt.apply(EditCommand::AddSketch { sketch: sk }).unwrap();

    // Enter runs a real solve and caches dof/status (FakeBackend's SolverEngine
    // reports FullyConstrained ⇒ dof 0 — see the `enter_sketch` test above).
    rt.enter_sketch(sid).await.unwrap();

    let session = rt.get_sketch(sid).unwrap();
    assert_eq!(session.dof, 0);
    assert_eq!(session.status, SketchSolveStatus::FullyConstrained);
}

#[test]
fn get_sketch_unknown_id_is_a_recoverable_error() {
    let rt = runtime_with(Arc::new(FakeBackend::new()));
    let err = rt
        .get_sketch(SketchId(Uuid::from_u128(0xDEAD)))
        .unwrap_err();
    match err {
        EngineError::OpFailed { message, .. } => {
            assert!(message.contains("getSketch"), "{message}");
        }
        other => panic!("expected OpFailed, got {other:?}"),
    }
}

#[tokio::test]
async fn persisted_region_query_does_not_finish_the_edit_session() {
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    let (sk, _) = sketch_with_point();
    let sid = sk.id;
    rt.apply(EditCommand::AddSketch { sketch: sk }).unwrap();
    rt.enter_sketch(sid).await.unwrap();
    let depth = rt.undo_depth();
    let revision = rt.revision();

    let regions = rt
        .prepare_sketch_regions(sid)
        .unwrap()
        .drive()
        .await
        .unwrap();
    assert!(regions.regions.is_empty());
    assert!(
        rt.sketch_session.is_some(),
        "read-only query keeps session open"
    );
    assert_eq!(rt.undo_depth(), depth);
    assert_eq!(rt.revision(), revision);
}

#[tokio::test]
async fn sketch_gesture_commits_exactly_one_undo_command() {
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    let (sk, point) = sketch_with_point();
    let sid = sk.id;
    rt.apply(EditCommand::AddSketch { sketch: sk }).unwrap();

    // Enter → real dof/status flow into the projection (FullyConstrained ⇒ dof 0).
    let session = rt.enter_sketch(sid).await.unwrap();
    assert_eq!(session.sketch_id, sid.to_string());
    let proj = rt.projection();
    assert_eq!(proj.sketches[&sid.to_string()].dof, 0);
    assert_eq!(
        proj.sketches[&sid.to_string()].status,
        crate::dto::SketchStatus::Ok
    );

    // Drag: begin → N drags → pointer-up commits ONE undo command.
    let depth_before = rt.session.undo_depth();
    let g = rt.begin_gesture(sid, point).await.unwrap();
    assert!(g.ready);
    rt.solve_drag([5.0, 0.0]).await.unwrap();
    rt.solve_drag([10.0, 2.0]).await.unwrap();
    let end = rt.end_gesture(Some([12.0, 3.0])).await.unwrap();
    assert!(end.solved_positions.contains_key(&point.to_string()));

    assert_eq!(
        rt.session.undo_depth(),
        depth_before + 1,
        "the whole gesture is exactly ONE undo command"
    );
    assert_eq!(
        point_at(&rt, sid, point),
        [12.0, 3.0],
        "point moved to target"
    );

    // One undo reverts the whole drag.
    assert!(rt.undo());
    assert_eq!(
        point_at(&rt, sid, point),
        [0.0, 0.0],
        "undo reverts the drag"
    );
}

#[tokio::test]
async fn sketch_mutations_expose_regen_outcomes_to_the_scheduler() {
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    let (sk, point) = sketch_with_point();
    let sid = sk.id;
    rt.apply(EditCommand::AddSketch { sketch: sk }).unwrap();

    let mut dependent = extrude_record(0xE11, 5.0);
    let Operation::Known(KnownOperation::Extrude(params)) = &mut dependent.op else {
        unreachable!();
    };
    params.profile = Some(SketchRegionRef {
        sketch: sid,
        region: RegionId::new("r_profile"),
        extra: Default::default(),
    });
    dependent.inputs = dependent.op.derive_inputs();
    rt.apply(EditCommand::AddOperation {
        record: dependent,
        at_cursor: false,
    })
    .unwrap();

    let added = SketchEntity::point(
        EntityId(Uuid::from_u128(0xE12)),
        Vec2::new_unchecked(2.0, 3.0),
        false,
        false,
    );
    let (_, upsert_outcome) = rt
        .sketch_upsert_with_outcome(sid, vec![SketchEditOp::AddEntity { entity: added }])
        .await
        .unwrap();
    assert!(matches!(
        upsert_outcome.map(|outcome| outcome.regen),
        Some(RegenHint::ToEnd)
    ));

    rt.begin_gesture(sid, point).await.unwrap();
    let (_, gesture_outcome) = rt.end_gesture_with_outcome(Some([4.0, 5.0])).await.unwrap();
    assert_eq!(gesture_outcome.regen, RegenHint::ToEnd);
}

#[tokio::test]
async fn solve_drag_without_gesture_is_recoverable_error() {
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    let err = rt.solve_drag([1.0, 1.0]).await.unwrap_err();
    assert!(matches!(err, EngineError::OpFailed { .. }));
}

#[tokio::test]
async fn promote_selection_mints_ids_and_is_stable() {
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    let body = BodyId(Uuid::from_u128(0xB0));
    let picks = vec![(TopoKey::new("f:22"), None), (TopoKey::new("e:3"), None)];
    let ids = rt
        .promote_selection(SnapshotId(5), body, picks)
        .await
        .unwrap();
    assert_eq!(ids.len(), 2);
    assert!(ids[0].element_id.starts_with("el_"));
    assert_eq!(ids[0].topo_key, "f:22");
    assert_eq!(ids[0].kind, "face");

    // Re-promote the SAME (body, topoKey) ⇒ the SAME id (Invariant 1).
    let again = rt
        .promote_selection(SnapshotId(5), body, vec![(TopoKey::new("f:22"), None)])
        .await
        .unwrap();
    assert_eq!(
        again[0].element_id, ids[0].element_id,
        "re-pick reuses the id (Invariant 1)"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// get_operation_params (re-edit deep-merge source; Findings 3+4) + refId-only
// resolve_refs hydration (Finding 2)
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn operation_params_returns_the_stored_params_for_a_reedit() {
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    rt.apply(EditCommand::AddOperation {
        record: revolve_record(0x1234, 90.0, 0x77),
        at_cursor: true,
    })
    .unwrap();

    // The stored params carry the NON-scalar axis a scalar re-edit must preserve.
    let params = rt
        .operation_params(RecordId(Uuid::from_u128(0x1234)))
        .expect("params for a known revolve record");
    assert_eq!(params["angleDeg"]["value"].as_f64(), Some(90.0));
    assert_eq!(params["axis"]["kind"], "sketchLine");
    assert_eq!(
        params["axis"]["lineId"],
        serde_json::json!(EntityId(Uuid::from_u128(0x77)).to_string())
    );

    // An unknown record yields None (→ the command surfaces an InvalidCommand error).
    assert!(rt
        .operation_params(RecordId(Uuid::from_u128(0xDEAD)))
        .is_none());
}

#[test]
fn parse_input_ref_id_and_element_ref_input() {
    // refId grammar `<recordId>.input<k>` (worker PlanExecutor mints `<opId>.input<i>`).
    let rec = RecordId(Uuid::from_u128(0xF11));
    assert_eq!(parse_input_ref_id(&format!("{rec}.input0")), Some((rec, 0)));
    assert_eq!(parse_input_ref_id(&format!("{rec}.input3")), Some((rec, 3)));
    assert!(parse_input_ref_id("not-a-uuid.input0").is_none());
    assert!(parse_input_ref_id(&format!("{rec}.face0")).is_none());

    // A fillet op's input0 is edges[0]; there is no input1 (single edge).
    let body = BodyId(Uuid::from_u128(0xB2));
    let rec2 = fillet_record(0xF11, body, "e:5", Vec3::new_unchecked(1.0, 2.0, 3.0));
    let r = element_ref_input(&rec2.op, 0).expect("edge 0 ref");
    assert_eq!(r.primary.as_ref().unwrap().element.as_str(), "e:5");
    assert!(element_ref_input(&rec2.op, 1).is_none());
}

#[tokio::test]
async fn resolve_refs_hydrates_a_refid_only_request_from_the_stored_ref() {
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    let rec = RecordId(Uuid::from_u128(0xF12));
    let body = BodyId(Uuid::from_u128(0xB3));
    let anchor = Vec3::new_unchecked(4.0, 5.0, 6.0);
    rt.apply(EditCommand::AddOperation {
        record: fillet_record(0xF12, body, "e:5", anchor),
        at_cursor: true,
    })
    .unwrap();

    // A lean refId-only request (an empty ElementRef) hydrates from the stored edge.
    let hydrated = rt
        .stored_input_ref(&format!("{rec}.input0"))
        .expect("hydrated from the stored fillet edge");
    let primary = hydrated.primary.expect("primary");
    assert_eq!(primary.element.as_str(), "e:5");
    assert_eq!(primary.body, body);
    assert_eq!(
        hydrated.anchor.expect("anchor").world_point,
        anchor,
        "the stored anchor rides so the ladder resolves the edge"
    );

    // A refId naming no known record does not hydrate (fails soft to the empty ref).
    let bogus = format!("{}.input0", RecordId(Uuid::from_u128(0xFEE)));
    assert!(rt.stored_input_ref(&bogus).is_none());
    // An out-of-range slot (only one edge) does not hydrate.
    assert!(rt.stored_input_ref(&format!("{rec}.input9")).is_none());
}

#[tokio::test]
async fn anchor_carries_through_promotion() {
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    let body = BodyId(Uuid::from_u128(0xB1));
    let anchor = AnchorIntent {
        world_point: onecad_core::math::Vec3::new_unchecked(1.0, 2.0, 3.0),
        surface_uv: Vec2::new(0.5, 0.5),
        local_frame: None,
        adjacency_hint: None,
        extra: Default::default(),
    };
    let ids = rt
        .promote_selection(
            SnapshotId(9),
            body,
            vec![(TopoKey::new("f:7"), Some(anchor))],
        )
        .await
        .unwrap();
    assert_eq!(ids.len(), 1);
    assert!(ids[0].element_id.starts_with("el_"));
}

// ─────────────────────────────────────────────────────────────────────────────
// MODEL-HARDEN finding 1 — regen-finished failedSteps / affectedBodies derivation
// (pure functions over the committed regen mirror). A published from-0 regen can
// republish sibling bodies while leaving the newly-committed op in Error, so the
// per-record maps must let the frontend correlate its OWN commit precisely.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn failed_steps_maps_errored_records_with_their_reason() {
    let ok = extrude_record(0xA0, 5.0);
    let bad = revolve_record(0xB0, 90.0, 0x11);
    let mut timeline = Timeline::from_records(vec![ok.clone(), bad.clone()]);
    timeline.mark_state(0, StepState::Valid).unwrap();
    timeline
        .mark_state(
            1,
            StepState::Error {
                reason: "revolve axis lineId not found".into(),
            },
        )
        .unwrap();
    let failed = failed_steps_of(&timeline);
    assert_eq!(
        failed.len(),
        1,
        "only the errored step lands in failedSteps"
    );
    assert_eq!(failed[0].record_id, bad.record_id.to_string());
    assert_eq!(failed[0].message, "revolve axis lineId not found");
}

#[test]
fn affected_bodies_maps_a_newbody_extrude_to_its_body() {
    let ext = extrude_record(0xE1, 8.0);
    // D1: a NewBody body is `BodyId(opId.uuid)`.
    let body = BodyId(ext.record_id.as_uuid());
    let timeline = Timeline::from_records(vec![ext.clone()]);
    let mut bodies = BodyRegistry::new();
    bodies.fold(0, ext.record_id, BodyLifecycleEvent::Created { body });
    let map = affected_bodies_of(&timeline, &bodies);
    assert_eq!(map.len(), 1);
    assert_eq!(
        map[&ext.record_id.to_string()],
        vec![body.to_string()],
        "the extrude's recordId maps to the body it created (document-changed wire form)"
    );
}

#[test]
fn affected_bodies_maps_a_boolean_op_to_its_modified_target() {
    let base = extrude_record(0xE2, 8.0); // creates the target
    let boolean = extrude_record(0xE3, 4.0); // stands in for an Add/Cut op modifying it
    let target = BodyId(base.record_id.as_uuid());
    let timeline = Timeline::from_records(vec![base.clone(), boolean.clone()]);
    let mut bodies = BodyRegistry::new();
    bodies.fold(
        0,
        base.record_id,
        BodyLifecycleEvent::Created { body: target },
    );
    bodies.fold(
        1,
        boolean.record_id,
        BodyLifecycleEvent::Modified { body: target },
    );
    let map = affected_bodies_of(&timeline, &bodies);
    // The boolean op MODIFIED the target in place → its recordId maps to the target.
    assert_eq!(
        map[&boolean.record_id.to_string()],
        vec![target.to_string()]
    );
    // The base op created it → still mapped (both ops touched the same body id).
    assert_eq!(map[&base.record_id.to_string()], vec![target.to_string()]);
}

#[test]
fn affected_bodies_skips_a_delete_only_op() {
    let base = extrude_record(0xE4, 8.0);
    let del = extrude_record(0xE5, 4.0);
    let target = BodyId(base.record_id.as_uuid());
    let timeline = Timeline::from_records(vec![base.clone(), del.clone()]);
    let mut bodies = BodyRegistry::new();
    bodies.fold(
        0,
        base.record_id,
        BodyLifecycleEvent::Created { body: target },
    );
    bodies.fold(
        1,
        del.record_id,
        BodyLifecycleEvent::Deleted { body: target },
    );
    let map = affected_bodies_of(&timeline, &bodies);
    assert!(
        !map.contains_key(&del.record_id.to_string()),
        "a delete-only op creates/modifies nothing ⇒ no affectedBodies entry"
    );
    assert!(map.contains_key(&base.record_id.to_string()));
}

// ─────────────────────────────────────────────────────────────────────────────
// MODEL-HARDEN finding 2 — sketch-session hygiene across worker failure + undo
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn enter_sketch_solver_error_leaves_no_session() {
    // (2c) The fallible solve runs BEFORE the session opens, so a worker error leaves
    // no stale open session (which a later stray finish would try to squash).
    let mut rt = runtime_with(Arc::new(FakeBackend::with_failing_solver()));
    let (sk, _p) = sketch_with_point();
    let sid = sk.id;
    rt.apply(EditCommand::AddSketch { sketch: sk }).unwrap();

    let err = rt.enter_sketch(sid).await.unwrap_err();
    assert!(
        matches!(err, EngineError::OpFailed { .. }),
        "the solver error propagates, got {err:?}"
    );
    assert!(
        rt.sketch_session.is_none(),
        "a failed enter opens NO session"
    );
}

#[tokio::test]
async fn undo_below_watermark_drops_the_sketch_session() {
    // (2a) A document undo that pops below the session's enter watermark drops the
    // stale session — a later finish then squashes nothing.
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    let (sk, _p) = sketch_with_point();
    let sid = sk.id;
    rt.apply(EditCommand::AddSketch { sketch: sk }).unwrap(); // depth 1
    rt.apply(add_extrude(0xE0, 5.0)).unwrap(); // depth 2

    rt.enter_sketch(sid).await.unwrap(); // watermark = 2
    assert!(rt.sketch_session.is_some(), "enter opened a session");

    // Undo the extrude → depth 1 < watermark 2 (the stack shrank below enter).
    assert!(rt.undo(), "the extrude undoes");
    assert!(
        rt.sketch_session.is_none(),
        "an undo below the watermark drops the stale session (no squash)"
    );

    // A later finish squashes nothing — only the sketch-record upsert (its own
    // undoable command) moves the depth.
    let depth = rt.session.undo_depth();
    rt.finish_sketch(sid).await.unwrap();
    assert_eq!(
        rt.session.undo_depth(),
        depth + 1,
        "finish over a dropped session squashes nothing (the +1 is the minted Sketch record)"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// MODEL-HARDEN finding 3 — UNDO_CAP eviction invalidates the depth watermark
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn eviction_during_session_refuses_the_squash() {
    // Watermark 150, then 60 session edits ⇒ the stack overflows UNDO_CAP (200) and
    // evicts the bottom 10. The depth-based `count = 200 − 150 = 50` no longer
    // addresses the 60 session steps, so a squash would strand the earliest 10 below
    // the net-inverse. The eviction guard refuses the squash entirely.
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    let (sk, _p) = sketch_with_point();
    let sid = sk.id;
    rt.apply(EditCommand::AddSketch { sketch: sk }).unwrap(); // depth 1
    for i in 0..149u128 {
        rt.apply(add_extrude(0x1000 + i, 5.0)).unwrap(); // → depth 150
    }
    assert_eq!(rt.session.undo_depth(), 150);

    rt.enter_sketch(sid).await.unwrap(); // watermark 150, evicted_at_enter 0

    for i in 0..60u128 {
        let point = SketchEntity::point(
            EntityId(Uuid::from_u128(0x9000 + i)),
            Vec2::new_unchecked(i as f64, 0.0),
            false,
            false,
        );
        rt.sketch_upsert(sid, vec![SketchEditOp::AddEntity { entity: point }])
            .await
            .unwrap();
    }
    // Capped at 200; 10 evictions happened during the session.
    assert_eq!(rt.session.undo_depth(), 200);
    assert_eq!(rt.session.evictions(), 10, "the bottom 10 steps evicted");

    rt.finish_sketch(sid).await.unwrap();
    assert_eq!(
        rt.session.undo_depth(),
        200,
        "the eviction guard REFUSED the squash — the granular steps stay (no collapse)"
    );
    assert!(rt.sketch_session.is_none(), "the session is consumed");
    // Undo stays monotonic (each pop reverts exactly one step).
    assert!(rt.undo());
    assert_eq!(rt.session.undo_depth(), 199);
}

#[tokio::test]
async fn below_cap_session_still_squashes() {
    // Control: a below-cap session (no eviction) squashes as before — the guard does
    // not over-refuse.
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    let (sk, _p) = sketch_with_point();
    let sid = sk.id;
    rt.apply(EditCommand::AddSketch { sketch: sk }).unwrap(); // depth 1
    rt.enter_sketch(sid).await.unwrap(); // watermark 1

    for i in 0..3u128 {
        let point = SketchEntity::point(
            EntityId(Uuid::from_u128(0xA000 + i)),
            Vec2::new_unchecked(i as f64, 0.0),
            false,
            false,
        );
        rt.sketch_upsert(sid, vec![SketchEditOp::AddEntity { entity: point }])
            .await
            .unwrap();
    }
    assert_eq!(rt.session.undo_depth(), 4); // AddSketch + 3 edits
    assert_eq!(rt.session.evictions(), 0);

    rt.finish_sketch(sid).await.unwrap();
    assert_eq!(
        rt.session.undo_depth(),
        3,
        "the 3 contiguous sketch edits collapse into ONE net command \
         (AddSketch + squash + the minted Sketch record)"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// MODEL-HARDEN finding 8 — a rolled-back (draft) append leaves applied < total, so
// the frontend can detect the stalled-awaiter case from the projection alone.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn draft_append_leaves_applied_ops_below_total_ops() {
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    let mk = |seed| EditCommand::AddOperation {
        record: extrude_record(seed, 5.0),
        at_cursor: false,
    };
    rt.apply(mk(0xA0)).unwrap(); // frontier append → applied; cursor 1, len 1
    rt.apply(mk(0xA1)).unwrap(); // frontier append → applied; cursor 2, len 2
                                 // Roll the applied bar back to just after A0.
    rt.apply(EditCommand::SetRollback { cursor: 1 }).unwrap(); // cursor 1, len 2
                                                               // A commit at the END while rolled back is a DRAFT (RegenHint::None) — the reported
                                                               // 8 s stall: no regen scheduled, so no completion ever fires.
    let outcome = rt.apply(mk(0xA2)).unwrap(); // draft; cursor 1, len 3
    assert_eq!(
        outcome.regen,
        RegenHint::None,
        "a rolled-back append stays a draft (no regen)"
    );

    let proj = rt.projection();
    assert!(
        proj.applied_ops < proj.total_ops,
        "a draft append leaves applied_ops({}) < total_ops({}) — the frontend short-circuits on it",
        proj.applied_ops,
        proj.total_ops
    );
    assert_eq!(proj.applied_ops, 1);
    assert_eq!(proj.total_ops, 3);
}

// ─────────────────────────────────────────────────────────────────────────────
// MODEL-HARDEN finding 5 — an EngineFailed regen whose fencing MOVED during the
// unlocked worker IO downgrades to Superseded (a covering regen is coming) instead of
// sending a spurious `failed` to a commit that regen will resolve.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn engine_failure_with_moved_fencing_reports_superseded() {
    let mut rt = runtime_with(Arc::new(FakeBackend::with_failing_plan()));
    rt.apply(add_extrude(0xF0, 5.0)).unwrap();
    let prepared = rt
        .begin_regen(RegenRequest::ToEnd { from: 0 })
        .expect("non-empty plan");
    // An edit lands during the (unlocked) worker IO → bumps the fencing revision.
    rt.apply(add_extrude(0xF1, 6.0)).unwrap();
    let driven = prepared.drive(CancelToken::new()).await;
    let report = rt.finish_regen(driven);
    assert_eq!(
        report.outcome_str(),
        "superseded",
        "a fail-while-superseded downgrades to Superseded (no spurious failure)"
    );
    assert!(
        report.failure_message().is_none(),
        "the downgraded report carries no failure message"
    );
}

#[tokio::test]
async fn engine_failure_without_moved_fencing_reports_failed() {
    // Control: a hard failure with UNCHANGED fencing is still `failed` — `run_regen`
    // holds the lock throughout, so no edit can move fencing.
    let mut rt = runtime_with(Arc::new(FakeBackend::with_failing_plan()));
    rt.apply(add_extrude(0xF2, 5.0)).unwrap();
    let report = rt
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    assert_eq!(report.outcome_str(), "failed");
    assert!(report.failure_message().is_some());
}

// ─────────────────────────────────────────────────────────────────────────────
// T0 suppression: the record flag reaches the projection, and a committed regen
// gives the dependency graph its body edges (so `cascade` can actually reach a
// downstream op).
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn feature_dto_carries_suppressed() {
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    rt.apply(add_extrude(0x10, 10.0)).unwrap();
    rt.apply(add_extrude(0x11, 20.0)).unwrap();
    assert!(
        rt.projection().features.iter().all(|f| !f.suppressed),
        "nothing is suppressed to start with"
    );

    rt.apply(EditCommand::SetOperationSuppression {
        record: RecordId(Uuid::from_u128(0x11)),
        suppressed: true,
        cascade: false,
    })
    .unwrap();

    let features = rt.projection().features;
    assert_eq!(features.len(), 2);
    assert!(!features[0].suppressed, "op 0 untouched");
    assert!(features[1].suppressed, "op 1 carries the record flag");
    // The flag is available BEFORE any regen — it is read off the record, not the
    // regen mirror state.
    assert_eq!(features[1].status, crate::dto::FeatureStatus::Dirty);

    rt.apply(EditCommand::SetOperationSuppression {
        record: RecordId(Uuid::from_u128(0x11)),
        suppressed: false,
        cascade: false,
    })
    .unwrap();
    assert!(rt.projection().features.iter().all(|f| !f.suppressed));
}

#[tokio::test]
async fn committed_regen_backfills_record_outputs_so_cascade_reaches_a_fillet() {
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    // The FakeBackend mints `BodyId(opId.uuid)` per op (the D1 rule), so op 0x10's
    // body is `BodyId(0x10)` — the body the fillet stands on.
    let body = BodyId(Uuid::from_u128(0x10));
    rt.apply(add_extrude(0x10, 10.0)).unwrap();
    rt.apply(EditCommand::AddOperation {
        record: fillet_record(0x20, body, "el_edge0", Vec3::new_unchecked(0.0, 0.0, 0.0)),
        at_cursor: true,
    })
    .unwrap();

    // Records arrive with EMPTY outputs, so before a regen the graph has no body
    // edge and the cascade cannot see the fillet.
    assert!(rt.session.document().timeline.records()[0]
        .outputs
        .is_empty());
    assert!(rt
        .session
        .graph()
        .downstream(RecordId(Uuid::from_u128(0x10)))
        .is_empty());

    let report = rt
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    assert!(matches!(report.outcome, Outcome::Published(_)));

    // The commit wrote the produced body back onto the record → body edge exists.
    assert_eq!(
        rt.session.document().timeline.records()[0].outputs,
        vec![body],
        "the extrude record adopted its produced body"
    );
    assert!(
        rt.session
            .graph()
            .downstream(RecordId(Uuid::from_u128(0x10)))
            .contains(&RecordId(Uuid::from_u128(0x20))),
        "the fillet is now downstream of the extrude"
    );

    // …so a cascading suppression reaches it.
    rt.apply(EditCommand::SetOperationSuppression {
        record: RecordId(Uuid::from_u128(0x10)),
        suppressed: true,
        cascade: true,
    })
    .unwrap();
    let features = rt.projection().features;
    assert!(features[0].suppressed, "the extrude is suppressed");
    assert!(features[1].suppressed, "the fillet cascaded");
}
