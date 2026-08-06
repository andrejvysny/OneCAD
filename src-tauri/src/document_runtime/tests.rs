//! Unit tests for [`DocumentRuntime`] driven by a local scripted backend.
//!
//! The `FakeBackend` implements both [`GeometryEngine`] and
//! [`MeshProvider`](crate::worker::MeshProvider) with no OCCT: each op creates a
//! deterministic body (`BodyId(opId.uuid)` — the D1 `body_<opId>` rule in the
//! core's UUID space) unless overridden, echoes the plan's opaque history-prefix
//! token the executor verifies, and serves canned MESH1 bytes per body.

use std::collections::HashMap;
use std::str::FromStr;
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
    PlanPrepared, PlanRequest, PlanStepEvent, PreparedMeshRef, RefResolution, RegenRequest,
    ResolveRequest, RestoreRequest, RestoreResult, Signature, StepResult, StepSignatures,
    StepStatus, StoppedReason, TessellateRequest, TessellateResult, WorkerElementEvidence,
    WorkerHead,
};

use onecad_core::document::refs::{
    AnchorIntent, AxisRef, ElementKind, ElementRef, IntentQuery, PrimaryRef, SketchRegionRef,
};
use onecad_core::ids::{ElementId, EntityId, RegionId, SketchId, TopoKey};
use onecad_core::math::{Vec2, Vec3};
use onecad_core::sketch::{Sketch, SketchEntity, WorldPlane};

use super::*;
use crate::dto::{
    BeginGestureDto, DragSolveDto, SketchRegionDto, SketchSolveStatus, SketchUpsertDto,
};
use crate::worker::wire::GestureTarget;
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
    /// The `drag` target of the most recent `begin_gesture` — the observable that
    /// proves the runtime passes it through untouched (SCHEMA §7.4).
    last_gesture_target: Option<crate::worker::wire::GestureTarget>,
    /// How many times `save_checkpoint` was invoked — the observable a skipped mint
    /// is asserted against (a re-save at the same step would leave the count alone).
    checkpoint_saves: usize,
    /// How many times `fetch_mesh` was invoked — the observable the inline-artifact
    /// path is pinned against (a seeded cache must fetch ZERO times).
    mesh_fetches: usize,
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
    /// When set, `save_checkpoint` returns artifacts instead of `Unsupported`.
    checkpoints_work: bool,
    /// When set, the terminal `PlanPrepared` carries an inline MESH1 artifact per
    /// created body (what the real worker's `artifacts.tessellate` rider produces).
    inline_meshes: bool,
    /// The opaque descriptor `acquire_element_ids` echoes per pick. Mutable so a
    /// test can move the promotion cache's evidence between two promotions of the
    /// SAME element and prove which stamp an edit kept (H5 fill-iff-None).
    descriptor: Mutex<serde_json::Value>,
    /// When set, `acquire_element_ids` echoes this as the pick's ALREADY-MINTED id
    /// (SCHEMA §7.5 `existing`), so a test can warm the promotion cache for an
    /// element id it did not mint in that runtime.
    existing: Mutex<Option<ElementId>>,
    /// CHANGED curve members every `solve_drag`/`end_gesture` echoes (SCHEMA §7.4
    /// `curves`). Set by a test to model a solver that reshaped a curve — the
    /// channel `positions` cannot carry.
    echo_curves: Mutex<std::collections::BTreeMap<String, crate::dto::CurveParamsDto>>,
    state: Mutex<FakeState>,
}

impl Default for FakeBackend {
    fn default() -> Self {
        Self {
            body_overrides: HashMap::new(),
            solver_fails: false,
            plan_fails: false,
            checkpoints_work: false,
            inline_meshes: false,
            descriptor: Mutex::new(serde_json::json!({ "fake": true })),
            existing: Mutex::new(None),
            echo_curves: Mutex::new(std::collections::BTreeMap::new()),
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

    /// A backend whose `SaveCheckpoint` succeeds, counting every call.
    fn with_checkpoints() -> Self {
        Self {
            checkpoints_work: true,
            ..Self::default()
        }
    }

    /// A backend that inlines a MESH1 artifact per prepared body into the terminal
    /// `PlanPrepared` — the real worker's `artifacts.tessellate` behavior.
    fn with_inline_meshes() -> Self {
        Self {
            inline_meshes: true,
            ..Self::default()
        }
    }

    fn checkpoint_saves(&self) -> usize {
        self.state.lock().unwrap().checkpoint_saves
    }

    fn mesh_fetches(&self) -> usize {
        self.state.lock().unwrap().mesh_fetches
    }

    /// The blob the INLINE artifact carries — deliberately distinct from
    /// [`Self::fetch_mesh`]'s bytes so a test can tell which lane served a mesh.
    fn inline_bytes(body: BodyId) -> Vec<u8> {
        format!("MESH1-INLINE:{body}").into_bytes()
    }

    /// Re-points the descriptor every later `acquire_element_ids` echoes.
    fn set_descriptor(&self, d: serde_json::Value) {
        *self.descriptor.lock().unwrap() = d;
    }

    /// Makes every later pick resolve to an already-minted `elementId`.
    fn set_existing(&self, id: Option<ElementId>) {
        *self.existing.lock().unwrap() = id;
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

            let artifact_lod = request.artifacts.tessellate.as_ref().map(|spec| spec.lod);
            let mut events = Vec::new();
            let mut per_step: Vec<StepResult> = Vec::new();
            let mut last_valid: Option<usize> = None;
            let mut artifact_meshes: Vec<PreparedMeshRef> = Vec::new();
            for op in &request.ops {
                let step = op.step_index;
                let body_ids = self.bodies_for(step, op.record_id);
                if let (true, Some(lod)) = (self.inline_meshes, artifact_lod) {
                    artifact_meshes.extend(body_ids.iter().map(|b| PreparedMeshRef {
                        body: *b,
                        lod,
                        bytes: Arc::new(Self::inline_bytes(*b)),
                    }));
                }
                let body_events: Vec<BodyLifecycleEvent> = body_ids
                    .iter()
                    .map(|b| BodyLifecycleEvent::Created { body: *b })
                    .collect();
                events.push(PlanEvent::Step(PlanStepEvent {
                    step_index: step,
                    body_events,
                    body_rank_keys: Default::default(),
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
                artifact_meshes,
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
    async fn save_checkpoint(&self, step: usize) -> Result<CheckpointArtifacts, EngineError> {
        if !self.checkpoints_work {
            return Err(EngineError::OpFailed {
                code: OpFailureCode::Unsupported,
                recoverable: true,
                message: "fake".into(),
            });
        }
        self.state.lock().unwrap().checkpoint_saves += 1;
        Ok(CheckpointArtifacts {
            step,
            artifacts: Vec::new(),
            element_map_partition: Vec::new(),
            signatures: sigs(step),
            history_prefix_hash: HistoryPrefixHash::empty(),
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
        // `kind` follows the TopoKey prefix (`f:`/`e:`/`v:`), as the real worker's
        // `kind_of_prefix` does — a fillet's edge pick must not come back a face.
        Ok(r.picks
            .into_iter()
            .map(|p| WorkerElementEvidence {
                kind: match p.topo_key.as_str().as_bytes().first() {
                    Some(b'e') => onecad_core::document::refs::ElementKind::Edge,
                    Some(b'v') => onecad_core::document::refs::ElementKind::Vertex,
                    _ => onecad_core::document::refs::ElementKind::Face,
                },
                topo_key: p.topo_key,
                body: r.body,
                anchor: p.anchor,
                descriptor: Some(self.descriptor.lock().unwrap().clone()),
                existing: self.existing.lock().unwrap().clone(),
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
        self.state.lock().unwrap().mesh_fetches += 1;
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
            solved_curves: std::collections::BTreeMap::new(),
        })
    }
    async fn begin_gesture(
        &self,
        _sketch_id: &str,
        _sketch_revision: u64,
        gesture_id: u64,
        drag_point: EntityId,
        target: crate::worker::wire::GestureTarget,
        _solver_policy_hash: &str,
    ) -> Result<BeginGestureDto, EngineError> {
        let mut state = self.state.lock().unwrap();
        state
            .gestures
            .insert(gesture_id, (drag_point.to_string(), [0.0, 0.0]));
        state.last_gesture_target = Some(target);
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
            curves: self.echo_curves.lock().unwrap().clone(),
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
            solved_curves: self.echo_curves.lock().unwrap().clone(),
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

/// A Hole on `body`'s face `face`, with a world-point anchor as its evidence.
fn hole_record_at(seed: u128, body: BodyId, face: &str, anchor: Vec3) -> OperationRecord {
    use onecad_core::document::record::{HoleParams, HoleType};
    let op = Operation::Known(KnownOperation::Hole(HoleParams {
        target_body: body,
        face: ElementRef {
            primary: Some(PrimaryRef {
                body,
                element: ElementId::new(face),
                kind: ElementKind::Face,
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
        },
        point: anchor,
        hole_type: HoleType::Simple,
        diameter: Scalar::new(6.0),
        depth: None,
        cb_diameter: None,
        cb_depth: None,
        cs_diameter: None,
        cs_angle_deg: None,
        extra: Default::default(),
    }));
    OperationRecord::new(RecordId(Uuid::from_u128(seed)), 0, "Hole", op)
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

    assert!(rt.undo().is_some(), "undo removes the second op");
    assert_eq!(rt.projection().features.len(), 1);

    assert!(rt.redo().unwrap().is_some(), "redo re-applies it");
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
        .get_mesh(body, DISPLAY_LOD, None)
        .await
        .expect("miss → fetch");
    let expected = b"MESH1:00000000-0000-0000-0000-000000000010:fine".to_vec();
    assert_eq!(*first, expected, "provider bytes served verbatim");

    let second = rt
        .get_mesh(body, DISPLAY_LOD, None)
        .await
        .expect("cache hit");
    assert!(
        Arc::ptr_eq(&first, &second),
        "hit returns the same cached Arc"
    );
}

#[tokio::test]
async fn regen_seeds_the_mesh_cache_from_inline_plan_artifacts() {
    // The whole point: the engine already tessellated every prepared body and shipped
    // the blobs with the terminal prepare, so the FIRST post-regen `get_mesh` must be
    // a cache HIT — zero `fetch_mesh` calls, ever.
    let backend = Arc::new(FakeBackend::with_inline_meshes());
    let mut rt = runtime_with(backend.clone());
    rt.apply(add_extrude(0x10, 10.0)).unwrap();
    let report = rt
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    assert!(matches!(report.outcome, Outcome::Published(_)));

    let body = BodyId(Uuid::from_u128(0x10));
    let bytes = rt
        .get_mesh(body, DISPLAY_LOD, None)
        .await
        .expect("the seeded cache serves the body");
    assert_eq!(
        *bytes,
        FakeBackend::inline_bytes(body),
        "the INLINE artifact bytes were served (not the pull path's)"
    );
    assert_eq!(
        backend.mesh_fetches(),
        0,
        "a seeded cache must never re-tessellate"
    );

    // And it is a real cache entry: the second call hands back the same Arc.
    let again = rt.get_mesh(body, DISPLAY_LOD, None).await.expect("hit");
    assert!(Arc::ptr_eq(&bytes, &again));
    assert_eq!(backend.mesh_fetches(), 0);
}

#[tokio::test]
async fn superseded_regen_never_seeds_the_mesh_cache() {
    // Fencing gates the cache fill exactly as it gates the commit: a prepare that
    // loses the window race published nothing, so its inline meshes must be dropped
    // — caching them would serve geometry the document never adopted.
    let backend = Arc::new(FakeBackend::with_inline_meshes());
    let mut rt = runtime_with(backend.clone());
    rt.apply(add_extrude(0x10, 25.0)).unwrap();

    let prepared = rt
        .begin_regen(RegenRequest::ToEnd { from: 0 })
        .expect("non-empty plan");
    rt.apply(add_extrude(0x11, 10.0)).unwrap(); // bumps the fencing revision
    let driven = prepared.drive(CancelToken::new()).await;
    let report = rt.finish_regen(driven);

    assert!(
        matches!(report.outcome, Outcome::Superseded),
        "{:?}",
        report.outcome
    );
    assert!(
        rt.mesh_cache.is_empty(),
        "a superseded prepare seeded {} cache entries",
        rt.mesh_cache.len()
    );

    // The converging regen at the new revision DOES seed — still zero fetches.
    let converge = rt
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    assert!(matches!(converge.outcome, Outcome::Published(_)));
    for seed in [0x10u128, 0x11] {
        let body = BodyId(Uuid::from_u128(seed));
        let bytes = rt.get_mesh(body, DISPLAY_LOD, None).await.expect("seeded");
        assert_eq!(*bytes, FakeBackend::inline_bytes(body));
    }
    assert_eq!(backend.mesh_fetches(), 0);
}

#[tokio::test]
async fn regen_without_inline_artifacts_still_pulls() {
    // Invariant 7: the artifacts are acceleration. An engine that inlines nothing
    // (the default fake, a retransmitted prepare) must still render via the pull.
    let backend = Arc::new(FakeBackend::new());
    let mut rt = runtime_with(backend.clone());
    rt.apply(add_extrude(0x10, 10.0)).unwrap();
    rt.run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;

    let body = BodyId(Uuid::from_u128(0x10));
    assert!(rt.get_mesh(body, DISPLAY_LOD, None).await.is_some());
    assert_eq!(backend.mesh_fetches(), 1, "fell back to exactly one pull");
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
            .starts_with(&format!("{}:fine:", ref_.body_id)),
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
        .get_mesh(BodyId(Uuid::from_u128(0x10)), DISPLAY_LOD, None)
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

// ─────────────────────────────────────────────────────────────────────────────
// Save payload split (VF-B7 / plan F1a) — the lock-free write half
// ─────────────────────────────────────────────────────────────────────────────

fn test_save_meta() -> onecad_core::io::container::SaveMeta {
    onecad_core::io::container::SaveMeta {
        app_version: "0.1.0-test".into(),
        occt_fingerprint: None,
        created: "2026-08-03T00:00:00Z".into(),
        modified: "2026-08-03T00:00:00Z".into(),
    }
}

/// A carrier blob keyed by its real digest (the writer verifies the equation).
fn seed_import_blob(rt: &mut DocumentRuntime, bytes: Vec<u8>) -> String {
    use onecad_core::document::record::ImportSourceCodec;
    use sha2::{Digest, Sha256};
    let sha = format!("{:x}", Sha256::digest(&bytes));
    rt.imports.insert(
        sha.clone(),
        ImportBlob {
            codec: ImportSourceCodec::Step,
            bytes: Arc::new(bytes),
        },
    );
    sha
}

#[tokio::test]
async fn build_and_write_payload_produce_the_same_container_as_save() {
    // The split must be a pure refactor of the bytes: `build_save_payload` +
    // `write_payload` (the lock-free lane) and `save` (the in-place lane) write
    // containers that reopen to the same document.
    let dir = tempfile::tempdir().unwrap();
    let split_path = dir.path().join("split.onecad");
    let save_path = dir.path().join("save.onecad");

    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    rt.apply(add_extrude(0x10, 25.0)).unwrap();
    rt.run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;

    // Lane A: snapshot, then write with NO `&self` in sight.
    let payload = rt.build_save_payload(test_save_meta(), SaveCaches::explicit());
    DocumentRuntime::write_payload(&split_path, &payload).unwrap();
    // Lane B: the legacy in-place save.
    rt.save(&save_path, test_save_meta()).unwrap();

    let a = onecad_core::io::container::ContainerReader::open(&split_path).unwrap();
    let b = onecad_core::io::container::ContainerReader::open(&save_path).unwrap();
    assert_eq!(
        onecad_core::io::document_io::serialize_document(a.document()).unwrap(),
        onecad_core::io::document_io::serialize_document(b.document()).unwrap(),
        "the split lane must persist exactly the document `save` does"
    );
    assert_eq!(
        std::fs::read(&split_path).unwrap(),
        std::fs::read(&save_path).unwrap(),
        "same payload + same meta ⇒ byte-identical containers"
    );
}

#[test]
fn save_payload_shares_import_blob_bytes_instead_of_copying_them() {
    // VF-B7: the payload is built UNDER the runtime lock, so it must be cheap. An
    // import blob is up to 256 MiB — cloning the carrier has to be a refcount bump.
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    let sha = seed_import_blob(&mut rt, b"ISO-10303-21; a large source file".to_vec());

    let payload = rt.build_save_payload(test_save_meta(), SaveCaches::explicit());
    let live = &rt.imports[&sha].bytes;
    let snapshotted = &payload.imports[&sha].bytes;
    assert!(
        Arc::ptr_eq(live, snapshotted),
        "the payload must share the blob allocation, not deep-copy it"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Container caches: what an explicit save embeds so the next open paints at once
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn explicit_save_embeds_the_head_meshes_and_autosave_embeds_none() {
    let backend = Arc::new(FakeBackend::with_inline_meshes());
    let mut rt = runtime_with(backend);
    rt.apply(add_extrude(0x10, 25.0)).unwrap();
    rt.apply(add_extrude(0x11, 10.0)).unwrap();
    rt.run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;

    let explicit = rt.build_save_payload(test_save_meta(), SaveCaches::explicit());
    let mut persisted: Vec<(BodyId, Vec<u8>)> = explicit
        .caches
        .meshes
        .iter()
        .map(|m| {
            assert_eq!(m.lod, "fine", "V1 persists the display-quality tier only");
            (m.body, m.bytes.as_ref().clone())
        })
        .collect();
    persisted.sort_by_key(|(b, _)| *b);
    assert_eq!(
        persisted,
        vec![
            (
                BodyId(Uuid::from_u128(0x10)),
                FakeBackend::inline_bytes(BodyId(Uuid::from_u128(0x10)))
            ),
            (
                BodyId(Uuid::from_u128(0x11)),
                FakeBackend::inline_bytes(BodyId(Uuid::from_u128(0x11)))
            ),
        ],
        "every head body whose mesh is resident is embedded, verbatim"
    );

    let auto = rt.build_save_payload(test_save_meta(), SaveCaches::none());
    assert!(
        auto.caches.is_empty(),
        "the autosave lane embeds no cache section at all"
    );
}

/// A body whose mesh the LRU already evicted is simply not persisted — the save
/// must never fetch or tessellate to fill the cache, because it runs under the
/// single-writer lock.
#[tokio::test]
async fn a_save_persists_only_already_resident_meshes() {
    let backend = Arc::new(FakeBackend::with_inline_meshes());
    let mut rt = runtime_with(backend);
    rt.apply(add_extrude(0x10, 25.0)).unwrap();
    rt.run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    assert_eq!(rt.mesh_cache.len(), 1, "the regen seeded one blob");

    rt.mesh_cache.clear();
    let payload = rt.build_save_payload(test_save_meta(), SaveCaches::explicit());
    assert!(
        payload.caches.meshes.is_empty(),
        "an evicted mesh is skipped, not re-fetched under the lock"
    );
}

/// The 64 MiB per-save budget truncates by ascending body id, so the same document
/// always drops the same tail — a save must not produce a different container on
/// each run.
#[tokio::test]
async fn saved_mesh_budget_truncates_deterministically() {
    let backend = Arc::new(FakeBackend::with_inline_meshes());
    let mut rt = runtime_with(backend);
    for seed in [0x10u128, 0x11, 0x12] {
        rt.apply(add_extrude(seed, 25.0)).unwrap();
    }
    rt.run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    let generation = rt.latest_snapshot.as_ref().expect("published").generation;

    // Three blobs at 33 MiB: the first fits the 64 MiB budget, the second would
    // exceed it, and the walk stops there.
    let big = MAX_SAVED_MESH_BYTES / 2 + 1;
    for seed in [0x10u128, 0x11, 0x12] {
        rt.mesh_cache.put(
            MeshKey {
                body: BodyId(Uuid::from_u128(seed)),
                lod: DISPLAY_LOD,
                generation,
            },
            Arc::new(vec![0u8; big]),
        );
    }

    let first = rt.build_save_payload(test_save_meta(), SaveCaches::explicit());
    assert_eq!(
        first
            .caches
            .meshes
            .iter()
            .map(|m| m.body)
            .collect::<Vec<_>>(),
        vec![BodyId(Uuid::from_u128(0x10))],
        "only the lowest-id body fits the budget"
    );

    let second = rt.build_save_payload(test_save_meta(), SaveCaches::explicit());
    assert_eq!(
        second
            .caches
            .meshes
            .iter()
            .map(|m| m.body)
            .collect::<Vec<_>>(),
        first
            .caches
            .meshes
            .iter()
            .map(|m| m.body)
            .collect::<Vec<_>>(),
        "truncation is deterministic — the same document drops the same tail"
    );
}

/// A save that carries no viewport capture must CARRY FORWARD the thumbnail the
/// document already has. Erasing it would blank the start-screen card of every
/// project saved from a window that could not produce a capture.
#[tokio::test]
async fn save_without_a_capture_carries_the_preview_forward() {
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    rt.apply(add_extrude(0x10, 25.0)).unwrap();

    // Nothing captured yet ⇒ nothing written.
    let bare = rt.build_save_payload(test_save_meta(), SaveCaches::explicit());
    assert_eq!(bare.caches.preview_png, None);

    let first = b"\x89PNG\r\n\x1a\nfirst".to_vec();
    let captured = rt.build_save_payload(
        test_save_meta(),
        SaveCaches {
            meshes: true,
            preview_png: Some(first.clone()),
        },
    );
    assert_eq!(captured.caches.preview_png, Some(first.clone()));

    // …and every later capture-less save re-writes the SAME picture.
    let carried = rt.build_save_payload(test_save_meta(), SaveCaches::explicit());
    assert_eq!(
        carried.caches.preview_png,
        Some(first.clone()),
        "a capture-less save must not erase the thumbnail"
    );

    // A fresh capture replaces it.
    let second = b"\x89PNG\r\n\x1a\nsecond".to_vec();
    let replaced = rt.build_save_payload(
        test_save_meta(),
        SaveCaches {
            meshes: true,
            preview_png: Some(second.clone()),
        },
    );
    assert_eq!(replaced.caches.preview_png, Some(second.clone()));

    // The autosave lane never writes one, even though the runtime is holding it.
    let auto = rt.build_save_payload(test_save_meta(), SaveCaches::none());
    assert_eq!(auto.caches.preview_png, None);

    // And it survives a save → reopen round-trip.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("preview.onecad");
    rt.save(&path, test_save_meta()).unwrap();

    let backend = Arc::new(FakeBackend::new());
    let engine: Arc<dyn GeometryEngine> = backend.clone();
    let meshes: Arc<dyn MeshProvider> = backend.clone();
    let solver: Arc<dyn SolverEngine> = backend;
    let mut reopened = DocumentRuntime::open(&path, engine, meshes, solver).unwrap();
    let round_tripped = reopened.build_save_payload(test_save_meta(), SaveCaches::explicit());
    assert_eq!(
        round_tripped.caches.preview_png,
        Some(second),
        "the preview is loaded at open and carried by the next save"
    );
    assert_eq!(
        onecad_core::io::container::read_preview(&path),
        round_tripped.caches.preview_png,
        "and the light start-screen lane reads the same bytes"
    );
}

/// The pre-publish window is served from the container, and only from it: a
/// non-MESH1 entry never reaches the viewport, and the first publish retires the
/// whole set.
#[tokio::test]
async fn cached_meshes_serve_before_the_first_publish_and_are_dropped_after() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cached.onecad");
    let body = BodyId(Uuid::from_u128(0x10));

    {
        let mut rt = runtime_with(Arc::new(FakeBackend::with_inline_meshes()));
        rt.apply(add_extrude(0x10, 25.0)).unwrap();
        rt.run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
            .await;
        // The fake's inline bytes are not real MESH1, so hand-write a container
        // whose one mesh entry IS — this test is about the serving rule, not the
        // fake's blob format.
        let mut payload = rt.build_save_payload(test_save_meta(), SaveCaches::explicit());
        payload.caches.meshes = vec![onecad_core::io::container::MeshCache {
            body,
            lod: "fine".into(),
            bytes: Arc::new(minimal_mesh1()),
        }];
        DocumentRuntime::write_payload(&path, &payload).unwrap();
    }

    let backend = Arc::new(FakeBackend::with_inline_meshes());
    let engine: Arc<dyn GeometryEngine> = backend.clone();
    let meshes: Arc<dyn MeshProvider> = backend.clone();
    let solver: Arc<dyn SolverEngine> = backend.clone();
    let mut rt = DocumentRuntime::open(&path, engine, meshes, solver).unwrap();

    assert_eq!(rt.projection().geometry_source, "cached");
    assert_eq!(
        rt.get_mesh(body, DISPLAY_LOD, None).await.as_deref(),
        Some(&minimal_mesh1()),
        "the container blob is served verbatim before any publish"
    );
    assert_eq!(
        backend.mesh_fetches(),
        0,
        "serving the container cache costs no provider round-trip"
    );

    rt.run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    assert!(
        rt.cached_meshes.is_empty(),
        "the first publish retires the container cache"
    );
    assert_eq!(rt.projection().geometry_source, "live");
    assert_eq!(
        rt.get_mesh(body, DISPLAY_LOD, None).await.as_deref(),
        Some(&FakeBackend::inline_bytes(body)),
        "and every later fetch comes from the published snapshot"
    );
}

/// A 64-byte MESH1 blob with an empty section table — the smallest thing
/// `validate_mesh_blob` accepts.
fn minimal_mesh1() -> Vec<u8> {
    let mut buf = vec![0u8; 64];
    buf[0..4].copy_from_slice(&0x4D45_5348u32.to_le_bytes());
    buf[4..6].copy_from_slice(&1u16.to_le_bytes());
    buf
}

#[tokio::test]
async fn mark_saved_keeps_the_document_dirty_when_an_edit_lands_mid_write() {
    // The write now runs OFF the runtime lock, so an edit can land between the
    // payload snapshot and the commit. Those bytes are not in the file that was just
    // written — clearing `dirty` would advertise unsaved work as saved.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("model.onecad");

    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    rt.apply(add_extrude(0x10, 25.0)).unwrap();

    let payload = rt.build_save_payload(test_save_meta(), SaveCaches::explicit());
    let revision = rt.revision();
    DocumentRuntime::write_payload(&path, &payload).unwrap();
    // …an edit lands while the container was being deflated.
    rt.apply(add_extrude(0x11, 40.0)).unwrap();
    rt.mark_saved(&path, revision);

    assert_eq!(
        rt.path(),
        Some(path.as_path()),
        "the path is adopted anyway"
    );
    assert!(
        rt.is_dirty(),
        "an edit that is NOT in the written bytes must leave the document dirty"
    );

    // The quiet case still goes clean.
    let payload = rt.build_save_payload(test_save_meta(), SaveCaches::explicit());
    let revision = rt.revision();
    DocumentRuntime::write_payload(&path, &payload).unwrap();
    rt.mark_saved(&path, revision);
    assert!(!rt.is_dirty(), "no interleaved edit ⇒ the save is clean");
}

#[tokio::test]
async fn checkpoint_ticket_is_discarded_when_an_edit_lands_during_save_checkpoint() {
    // W1's ticket recheck becomes load-bearing in W3: `save_document` now awaits
    // `SaveCheckpoint` with the runtime lock RELEASED, so an edit really can land
    // between prepare and adopt. Artifacts describing a superseded head must be
    // dropped, not stored under the (now wrong) head step.
    let backend = Arc::new(FakeBackend::with_checkpoints());
    let mut rt = runtime_with(backend.clone());
    rt.apply(add_extrude(0x10, 25.0)).unwrap();
    rt.run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;

    let ticket = rt.prepare_checkpoint().expect("head is checkpointable");
    let engine = rt.engine_arc();
    // …the lock is released here in production; an edit lands and bumps the revision.
    let artifacts = engine.save_checkpoint(ticket.step()).await.unwrap();
    rt.apply(add_extrude(0x11, 40.0)).unwrap();
    rt.adopt_checkpoint(ticket, artifacts);
    assert_eq!(
        rt.checkpoint_count(),
        0,
        "artifacts for a head the edit superseded must NOT be adopted"
    );

    // The undisturbed round-trip still adopts.
    rt.run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    let ticket = rt.prepare_checkpoint().expect("head is checkpointable");
    let artifacts = engine.save_checkpoint(ticket.step()).await.unwrap();
    rt.adopt_checkpoint(ticket, artifacts);
    assert_eq!(rt.checkpoint_count(), 1, "a quiet round-trip adopts");
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

#[tokio::test]
async fn regen_driven_on_another_runtime_instance_never_commits() {
    // VF-B2: `new_document` swaps the runtime slot WITHOUT cancelling the in-flight
    // job, and the fresh runtime's FencingCell restarts from the same origin — so the
    // (revision, epoch) pair alone cannot tell doc A's regen from doc B's. One edit in
    // each document puts both at revision 1 with the same epoch: the tuple collides
    // exactly, and phase 3 would commit doc A's scratch (timeline + bodies) INTO doc B.
    // The per-runtime `instance` id is what makes that impossible.
    let mut rt_a = runtime_with(Arc::new(FakeBackend::new()));
    rt_a.apply(add_extrude(0x10, 25.0)).unwrap();
    let prepared = rt_a
        .begin_regen(RegenRequest::ToEnd { from: 0 })
        .expect("non-empty plan");
    let driven = prepared.drive(CancelToken::new()).await;

    // Document B: a different runtime, edited once so its fencing tokens MATCH the
    // in-flight regen's captured pair.
    let mut rt_b = runtime_with(Arc::new(FakeBackend::new()));
    rt_b.apply(add_extrude(0x20, 10.0)).unwrap();
    assert_eq!(
        rt_b.revision(),
        DocumentRevision(1),
        "the two documents must share fencing tokens for this test to prove anything"
    );

    let report = rt_b.finish_regen(driven);
    assert!(
        matches!(report.outcome, Outcome::Superseded),
        "a regen prepared on ANOTHER runtime instance must be discarded, got {:?}",
        report.outcome
    );
    assert!(
        report.document_change().is_none(),
        "nothing may be published into the unrelated document"
    );
    assert!(
        rt_b.projection().bodies.is_empty(),
        "document B must hold NO body from document A's regen"
    );
    let features = rt_b.projection().features;
    assert_eq!(features.len(), 1, "document B keeps its own single feature");
    assert_eq!(
        features[0].id,
        RecordId(Uuid::from_u128(0x20)).to_string(),
        "document B's timeline must not be replaced by document A's"
    );
}

#[tokio::test]
async fn checkpoint_is_not_minted_over_a_restarted_worker() {
    // VF-M2: after a worker restart the fresh process holds NO geometry, but the
    // runtime's `latest_snapshot` still describes the pre-restart head — so the
    // "fully regenerated" guard passes and SaveCheckpoint mints an EMPTY-prefix
    // checkpoint OVER the valid one (the store's per-step insert overwrites), which
    // then gets persisted into the container. The mint must wait for the replay.
    let backend = Arc::new(FakeBackend::with_checkpoints());
    let mut rt = runtime_with(backend.clone());
    rt.apply(add_extrude(0x10, 25.0)).unwrap();
    let r = rt
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    assert!(
        matches!(r.outcome, Outcome::Published(_)),
        "{:?}",
        r.outcome
    );

    rt.take_checkpoint_at_head().await;
    assert_eq!(rt.checkpoint_count(), 1, "the valid checkpoint is minted");
    assert_eq!(backend.checkpoint_saves(), 1, "one SaveCheckpoint so far");

    // The worker crashed and came back under a new epoch. The replay has NOT run yet.
    rt.on_worker_restart(WorkerEpoch(2));
    rt.take_checkpoint_at_head().await;
    assert_eq!(
        backend.checkpoint_saves(),
        1,
        "SaveCheckpoint must NOT be issued against a worker that no longer holds the \
         head geometry — the valid checkpoint would be overwritten by an empty one"
    );
    assert_eq!(rt.checkpoint_count(), 1, "the cache still holds one entry");

    // Once the replay republishes the head under the new epoch, minting resumes.
    let replay = rt
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    assert!(
        matches!(replay.outcome, Outcome::Published(_)),
        "{:?}",
        replay.outcome
    );
    rt.take_checkpoint_at_head().await;
    assert_eq!(
        backend.checkpoint_saves(),
        2,
        "after the replay the head geometry is the running worker's again"
    );
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

    assert!(rt.undo().is_some());
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
    let g = rt
        .begin_gesture(sid, point, GestureTarget::point(point))
        .await
        .unwrap();
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
    assert!(rt.undo().is_some());
    assert_eq!(
        point_at(&rt, sid, point),
        [0.0, 0.0],
        "undo reverts the drag"
    );
}

#[tokio::test]
async fn end_gesture_applies_solved_curves_and_forwards_the_target() {
    // SP-2: the `curves` channel is what a radius/arcEnd drag reports — `positions`
    // carries none of it. A solver that reshaped the arc must land in the DOCUMENT,
    // in the same single undo command as the positions.
    let backend = Arc::new(FakeBackend::new());
    let (sid, arc, circle, center) = (
        SketchId(Uuid::from_u128(0x5d)),
        EntityId(Uuid::from_u128(0x141)),
        EntityId(Uuid::from_u128(0x142)),
        EntityId(Uuid::from_u128(0x140)),
    );
    let mut sk = Sketch::on_world_plane(sid, "Curves", WorldPlane::XY);
    sk.add_entity(SketchEntity::point(
        center,
        Vec2::new_unchecked(0.0, 0.0),
        false,
        false,
    ))
    .unwrap();
    sk.add_entity(
        SketchEntity::arc(arc, center, 4.0, 0.0, std::f64::consts::FRAC_PI_2, false).unwrap(),
    )
    .unwrap();
    sk.add_entity(SketchEntity::circle(circle, center, 5.0, false).unwrap())
        .unwrap();

    *backend.echo_curves.lock().unwrap() = std::collections::BTreeMap::from([
        (
            arc.to_string(),
            crate::dto::CurveParamsDto {
                radius: Some(11.0),
                start_angle: Some(-0.25),
                end_angle: None, // absent ⇒ UNCHANGED, not zero
            },
        ),
        (
            // An unknown id must be ignored, not panic the commit.
            EntityId(Uuid::from_u128(0xDEAD)).to_string(),
            crate::dto::CurveParamsDto {
                radius: Some(1.0),
                ..Default::default()
            },
        ),
    ]);

    let mut rt = runtime_with(backend.clone());
    rt.apply(EditCommand::AddSketch { sketch: sk }).unwrap();
    rt.enter_sketch(sid).await.unwrap();
    let depth_before = rt.session.undo_depth();

    let target = GestureTarget {
        kind: crate::worker::wire::DragKind::ArcEnd,
        entity: arc,
        role: Some("end"),
        grab: Some([9.0, 1.5]),
    };
    rt.begin_gesture(sid, center, target).await.unwrap();
    assert_eq!(
        backend.state.lock().unwrap().last_gesture_target,
        Some(target),
        "the runtime forwards the target verbatim (no rewrite)"
    );
    let solved = rt.end_gesture(Some([9.0, 1.5])).await.unwrap();
    assert_eq!(solved.solved_curves[&arc.to_string()].radius, Some(11.0));

    let doc = rt.session.document().sketch(sid).unwrap();
    match doc.get_entity(arc).unwrap() {
        SketchEntity::Arc {
            radius,
            start_angle,
            end_angle,
            ..
        } => {
            assert_eq!(*radius, 11.0, "the arc's radius came from `curves`");
            assert_eq!(*start_angle, -0.25);
            assert_eq!(
                *end_angle,
                std::f64::consts::FRAC_PI_2,
                "an absent member is UNCHANGED"
            );
        }
        other => panic!("{other:?} is not an arc"),
    }
    match doc.get_entity(circle).unwrap() {
        SketchEntity::Circle { radius, .. } => {
            assert_eq!(*radius, 5.0, "an unreported curve is untouched")
        }
        other => panic!("{other:?} is not a circle"),
    }
    assert_eq!(
        rt.session.undo_depth(),
        depth_before + 1,
        "curves ride the SAME single undo command as positions"
    );

    // One undo reverts the reshape too (it was applied to the `before` memento).
    assert!(rt.undo().is_some());
    match rt
        .session
        .document()
        .sketch(sid)
        .unwrap()
        .get_entity(arc)
        .unwrap()
    {
        SketchEntity::Arc { radius, .. } => assert_eq!(*radius, 4.0, "undo reverts the reshape"),
        other => panic!("{other:?} is not an arc"),
    }
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

    rt.begin_gesture(sid, point, GestureTarget::point(point))
        .await
        .unwrap();
    let (_, gesture_outcome) = rt.end_gesture_with_outcome(Some([4.0, 5.0])).await.unwrap();
    assert_eq!(gesture_outcome.regen, RegenHint::ToEnd);
}

#[tokio::test]
async fn solve_drag_without_gesture_is_recoverable_error() {
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    let err = rt.solve_drag([1.0, 1.0]).await.unwrap_err();
    assert!(matches!(err, EngineError::OpFailed { .. }));
}

// ─────────────────────────────────────────────────────────────────────────────
// Mid-gesture solver-cache clobber guard — prepare_sketch_regions must refuse
// while a drag is live on the SAME sketch (drive() would re-upsert stale
// pre-drag geometry into the worker's solver cache, corrupting the live drag);
// finish_sketch must clear a dangling gesture the same way cancel_sketch does.
// ─────────────────────────────────────────────────────────────────────────────

/// A second, minimal sketch (no entities needed — prepare/drive only touch the
/// FakeBackend, which is content-agnostic).
fn other_sketch(sid_seed: u128) -> Sketch {
    Sketch::on_world_plane(SketchId(Uuid::from_u128(sid_seed)), "Other", WorldPlane::XY)
}

#[tokio::test]
async fn prepare_sketch_regions_refuses_during_a_live_gesture_on_the_same_sketch() {
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    let (sk, point) = sketch_with_point();
    let sid = sk.id;
    rt.apply(EditCommand::AddSketch { sketch: sk }).unwrap();
    rt.enter_sketch(sid).await.unwrap();

    rt.begin_gesture(sid, point, GestureTarget::point(point))
        .await
        .unwrap();

    // `PreparedSketchRegions` has no `Debug` impl, so match manually instead of
    // `expect_err`/`unwrap_err` (both require `T: Debug` for their panic path).
    let err = match rt.prepare_sketch_regions(sid) {
        Err(e) => e,
        Ok(_) => panic!("a live gesture on the SAME sketch must refuse the region snapshot"),
    };
    match err {
        EngineError::OpFailed {
            message,
            recoverable,
            ..
        } => {
            assert!(recoverable, "must be recoverable — the session stays open");
            assert!(message.contains(&sid.to_string()), "{message}");
            assert!(message.contains("active drag gesture"), "{message}");
        }
        other => panic!("expected OpFailed, got {other:?}"),
    }
}

#[tokio::test]
async fn prepare_sketch_regions_allows_a_different_sketch_during_a_live_gesture() {
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    let (sk, point) = sketch_with_point();
    let sid_a = sk.id;
    rt.apply(EditCommand::AddSketch { sketch: sk }).unwrap();
    rt.enter_sketch(sid_a).await.unwrap();

    let sk_b = other_sketch(0x5d);
    let sid_b = sk_b.id;
    rt.apply(EditCommand::AddSketch { sketch: sk_b }).unwrap();

    rt.begin_gesture(sid_a, point, GestureTarget::point(point))
        .await
        .unwrap();

    // Sketch B carries no gesture — A's drag must not block it.
    rt.prepare_sketch_regions(sid_b)
        .expect("a different sketch's gesture must not block")
        .drive()
        .await
        .expect("drive succeeds");
}

#[tokio::test]
async fn prepare_sketch_regions_ok_again_after_gesture_ends() {
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    let (sk, point) = sketch_with_point();
    let sid = sk.id;
    rt.apply(EditCommand::AddSketch { sketch: sk }).unwrap();
    rt.enter_sketch(sid).await.unwrap();

    rt.begin_gesture(sid, point, GestureTarget::point(point))
        .await
        .unwrap();
    assert!(
        rt.prepare_sketch_regions(sid).is_err(),
        "blocked mid-gesture"
    );

    rt.end_gesture(Some([1.0, 1.0])).await.unwrap();
    rt.prepare_sketch_regions(sid)
        .expect("an ended gesture unblocks prepare")
        .drive()
        .await
        .expect("drive succeeds");
}

#[tokio::test]
async fn prepare_sketch_regions_ok_again_after_cancel() {
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    let (sk, point) = sketch_with_point();
    let sid = sk.id;
    rt.apply(EditCommand::AddSketch { sketch: sk }).unwrap();
    rt.enter_sketch(sid).await.unwrap();

    rt.begin_gesture(sid, point, GestureTarget::point(point))
        .await
        .unwrap();
    assert!(
        rt.prepare_sketch_regions(sid).is_err(),
        "blocked mid-gesture"
    );

    rt.cancel_sketch(sid).await.unwrap();
    rt.prepare_sketch_regions(sid)
        .expect("a cancelled gesture unblocks prepare")
        .drive()
        .await
        .expect("drive succeeds");
}

#[tokio::test]
async fn finish_sketch_mid_gesture_clears_active_gesture() {
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    let (sk, point) = sketch_with_point();
    let sid = sk.id;
    rt.apply(EditCommand::AddSketch { sketch: sk }).unwrap();
    rt.enter_sketch(sid).await.unwrap();

    rt.begin_gesture(sid, point, GestureTarget::point(point))
        .await
        .unwrap();
    assert!(rt.active_gesture.is_some(), "gesture is live");

    // The Enter/E finish handoff: the frontend's pointer-up cancel lost the
    // race, so finish_sketch is reached while the gesture is still open.
    rt.finish_sketch(sid).await.unwrap();

    assert!(
        rt.active_gesture.is_none(),
        "finish_sketch must clear the dangling gesture (same as cancel_sketch)"
    );
    rt.prepare_sketch_regions(sid)
        .expect("gesture cleared by finish ⇒ prepare unblocked")
        .drive()
        .await
        .expect("drive succeeds");

    // A stray late SolveDrag/EndGesture for the now-dead gesture is a
    // recoverable error, not a panic.
    let err = rt.solve_drag([9.0, 9.0]).await.unwrap_err();
    assert!(matches!(err, EngineError::OpFailed { .. }));
    let err = rt.end_gesture(None).await.unwrap_err();
    assert!(matches!(err, EngineError::OpFailed { .. }));
}

// ─────────────────────────────────────────────────────────────────────────────
// SP-0 D3 — the RAII solver-lane claim. The refusals above are check-then-act on
// `active_gesture`; these pin the claim that actually closes the window, and that
// EVERY gesture teardown path releases it (the claim is a field of
// `ActiveGesture`, so `Drop` is the only release site — nothing to forget).
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_held_region_query_refuses_a_new_gesture_until_it_is_driven() {
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    let (sk, point) = sketch_with_point();
    let sid = sk.id;
    rt.apply(EditCommand::AddSketch { sketch: sk }).unwrap();
    rt.enter_sketch(sid).await.unwrap();

    // Prepared, NOT driven — the window `get_sketch_regions` leaves open while it
    // awaits the worker with the runtime lock released.
    let prepared = rt.prepare_sketch_regions(sid).expect("prepare");
    assert_eq!(
        rt.solver_lane.owner(sid),
        Some(super::solver_lane::LaneOwner::RegionQuery)
    );

    let err = rt
        .begin_gesture(sid, point, GestureTarget::point(point))
        .await
        .unwrap_err();
    match err {
        EngineError::OpFailed {
            message,
            recoverable,
            ..
        } => {
            assert!(recoverable, "must be recoverable — the user just retries");
            assert!(message.contains(&sid.to_string()), "{message}");
            assert!(message.contains("region query"), "{message}");
        }
        other => panic!("expected OpFailed, got {other:?}"),
    }
    assert!(
        rt.active_gesture.is_none(),
        "a refused begin must leave no half-open gesture"
    );

    prepared.drive().await.expect("drive succeeds");
    assert_eq!(rt.solver_lane.owner(sid), None, "drive released the claim");
    rt.begin_gesture(sid, point, GestureTarget::point(point))
        .await
        .expect("the query finished ⇒ the drag opens");
}

#[tokio::test]
async fn a_held_region_query_on_another_sketch_never_blocks_a_gesture() {
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    let (sk, point) = sketch_with_point();
    let sid_a = sk.id;
    rt.apply(EditCommand::AddSketch { sketch: sk }).unwrap();
    let sk_b = other_sketch(0x5e);
    let sid_b = sk_b.id;
    rt.apply(EditCommand::AddSketch { sketch: sk_b }).unwrap();
    rt.enter_sketch(sid_a).await.unwrap();

    let prepared_b = rt.prepare_sketch_regions(sid_b).expect("prepare B");
    rt.begin_gesture(sid_a, point, GestureTarget::point(point))
        .await
        .expect("the claim is per sketch, not a global solver mutex");
    prepared_b.drive().await.expect("drive B succeeds");
}

#[tokio::test]
async fn every_gesture_teardown_path_releases_the_lane_claim() {
    // end_gesture, cancel_sketch and finish_sketch's dangling-gesture clear all
    // TAKE the `ActiveGesture`; the claim rides along and is released by `Drop`.
    for (label, teardown) in [
        ("end_gesture", 0_u8),
        ("cancel_sketch", 1),
        ("finish_sketch", 2),
    ] {
        let mut rt = runtime_with(Arc::new(FakeBackend::new()));
        let (sk, point) = sketch_with_point();
        let sid = sk.id;
        rt.apply(EditCommand::AddSketch { sketch: sk }).unwrap();
        rt.enter_sketch(sid).await.unwrap();

        rt.begin_gesture(sid, point, GestureTarget::point(point))
            .await
            .unwrap();
        assert_eq!(
            rt.solver_lane.owner(sid),
            Some(super::solver_lane::LaneOwner::Gesture),
            "{label}: the gesture holds the lane"
        );

        match teardown {
            0 => {
                rt.end_gesture(Some([1.0, 1.0])).await.unwrap();
            }
            1 => rt.cancel_sketch(sid).await.unwrap(),
            _ => {
                rt.finish_sketch(sid).await.unwrap();
            }
        }

        assert_eq!(
            rt.solver_lane.owner(sid),
            None,
            "{label}: the claim must be released"
        );
        // The observable consequence: a SECOND region query gets through.
        rt.prepare_sketch_regions(sid)
            .unwrap_or_else(|e| panic!("{label}: prepare must be unblocked, got {e:?}"))
            .drive()
            .await
            .expect("drive succeeds");
    }
}

#[tokio::test]
async fn concurrent_region_queries_share_the_lane() {
    // Two `getSketchRegions` on one sketch are pure reads and already overlap in
    // the frontend (sketchStaticSync vs the extrude/revolve arm). The claim must
    // not turn that into a new refusal.
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    let (sk, point) = sketch_with_point();
    let sid = sk.id;
    rt.apply(EditCommand::AddSketch { sketch: sk }).unwrap();
    rt.enter_sketch(sid).await.unwrap();

    let first = rt.prepare_sketch_regions(sid).expect("first");
    let second = rt.prepare_sketch_regions(sid).expect("second shares");
    // …but a gesture is still fenced while EITHER is outstanding.
    assert!(rt
        .begin_gesture(sid, point, GestureTarget::point(point))
        .await
        .is_err());
    first.drive().await.expect("drive 1");
    assert!(
        rt.begin_gesture(sid, point, GestureTarget::point(point))
            .await
            .is_err(),
        "one holder left ⇒ still fenced"
    );
    second.drive().await.expect("drive 2");
    rt.begin_gesture(sid, point, GestureTarget::point(point))
        .await
        .expect("lane free");
}

// ─────────────────────────────────────────────────────────────────────────────
// SP-0 D3 audit of HISTORY-HARDEN H8 — undo/redo never looked at
// `active_gesture`. Mid-drag, `end_gesture` rebuilds the sketch from the `before`
// memento captured at begin, so a revert that lands in between is silently
// resurrected at pointer-up. See `revert_blocked_by_gesture`.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn undo_stands_down_while_a_drag_gesture_is_open() {
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    let (sk, point) = sketch_with_point();
    let sid = sk.id;
    rt.apply(EditCommand::AddSketch { sketch: sk }).unwrap();
    rt.enter_sketch(sid).await.unwrap();

    // One committed edit to aim the undo at.
    rt.sketch_upsert(
        sid,
        vec![SketchEditOp::SetEntityPositions {
            positions: vec![(point, Vec2::new_unchecked(3.0, 4.0))],
        }],
    )
    .await
    .unwrap();
    let depth = rt.undo_depth();
    assert_eq!(point_at(&rt, sid, point), [3.0, 4.0]);

    rt.begin_gesture(sid, point, GestureTarget::point(point))
        .await
        .unwrap();
    assert!(
        rt.undo().is_none(),
        "⌘Z mid-drag must stand down, not corrupt the drag's `before` memento"
    );
    assert!(rt.redo().unwrap().is_none(), "so must ⇧⌘Z");
    assert_eq!(rt.undo_depth(), depth, "the undo stack is untouched");

    // Pointer-up commits normally, and undo works again the moment it does.
    rt.end_gesture(Some([9.0, 9.0])).await.unwrap();
    assert!(
        rt.undo().is_some(),
        "the gesture ended ⇒ undo is live again"
    );
    assert_eq!(
        point_at(&rt, sid, point),
        [3.0, 4.0],
        "the drag is what got reverted — the pre-drag edit survived"
    );
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

/// VF-M3, the RUST half of the stale-pick gate, isolated from the worker's.
///
/// The real worker refuses a stale `snapshotId` too (SCHEMA §7.5), so a
/// worker-backed test cannot tell the two gates apart. `FakeBackend` has no gate at
/// all and resolves any pick it is handed — exactly the shape of every non-worker
/// backend — so this pins that the refusal is Rust's own and does not depend on a
/// round-trip.
#[tokio::test]
async fn promote_selection_refuses_a_pick_against_a_superseded_snapshot() {
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    rt.apply(add_extrude(0x10, 25.0)).unwrap();
    let report = rt
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    let head = report.snapshot_id;
    assert!(
        head > 0,
        "the fake backend published a positive snapshot id"
    );

    let body = BodyId(Uuid::from_u128(0x10));
    let stale = SnapshotId(head - 1);
    let err = rt
        .promote_selection(stale, body, vec![(TopoKey::new("f:22"), None)])
        .await
        .expect_err("a pick against a superseded snapshot must be refused");
    match err {
        EngineError::OpFailed {
            recoverable,
            ref message,
            ..
        } => {
            assert!(recoverable, "a stale pick leaves the session editable");
            assert!(
                message.contains(&stale.0.to_string()) && message.contains(&head.to_string()),
                "the refusal names both ids: {message}"
            );
        }
        other => panic!("expected a recoverable OpFailed, got {other:?}"),
    }

    // The same pick at the head is accepted — the gate is about staleness only.
    rt.promote_selection(SnapshotId(head), body, vec![(TopoKey::new("f:22"), None)])
        .await
        .expect("promote at the head");
}

/// The gate MUST NOT fire before there is a positive published head: `drive_clear`
/// publishes `SnapshotId(0)` (no `AcceptPrepared` happened, so there is no worker
/// snapshot id to address) and the frontend only ever adopts a POSITIVE id, so
/// gating on `0` would refuse every legitimate first pick.
#[tokio::test]
async fn promote_selection_skips_the_gate_before_the_first_publish() {
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    let body = BodyId(Uuid::from_u128(0xB0));
    rt.promote_selection(SnapshotId(0), body, vec![(TopoKey::new("f:1"), None)])
        .await
        .expect("no published snapshot ⇒ nothing to be stale against");
    rt.promote_selection(SnapshotId(9), body, vec![(TopoKey::new("f:1"), None)])
        .await
        .expect("still nothing to be stale against");
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

/// H9 (closing an H5 follow-up): a Hole's `inputs[]` is `[host body, host face]`
/// (`worker::wire::wire_op_inputs`, pinned by
/// `wire_op_inputs_slot_order_is_the_repair_slot_table`), so a refId-only
/// `resolve_refs` for `<hole>.input1` must hydrate the STORED `params.face` —
/// evidence included. Before this, `element_ref_input` had no Hole arm, so the
/// lean request came back with no descriptor/anchor for the ladder to score
/// against while the planner lowered the full ref: the resolve dry-run and the
/// real regen disagreed about the same slot.
#[test]
fn hole_face_input_hydrates_at_slot_1() {
    let body = BodyId(Uuid::from_u128(0xB9));
    let anchor = Vec3::new_unchecked(7.0, 8.0, 9.0);
    let rec = hole_record_at(0xF19, body, "f:22", anchor);

    // Slot 0 is the host BODY ref — not an element, so nothing hydrates there.
    assert!(
        element_ref_input(&rec.op, 0).is_none(),
        "slot 0 is the host body, not an element ref"
    );
    let r = element_ref_input(&rec.op, 1).expect("host face ref at slot 1");
    assert_eq!(r.primary.as_ref().unwrap().element.as_str(), "f:22");
    assert_eq!(
        r.anchor.as_ref().map(|a| a.world_point),
        Some(anchor),
        "the stored evidence must ride the hydration, not just the id"
    );
    assert!(element_ref_input(&rec.op, 2).is_none());
}

/// The OffsetFace slot table (SCHEMA §7.3, normative): operative faces occupy
/// `0..faces.len()`, the `Total` opposite face is the LAST slot. `element_ref_input`
/// must agree with `wire::wire_op_inputs` slot for slot — a divergence hydrates a
/// refId-only `resolve_refs` from the WRONG face, so the dry-run and the real
/// regen would disagree about the same repair.
#[test]
fn offset_face_inputs_hydrate_faces_then_the_opposite_face() {
    use onecad_core::document::record::{OffsetDistanceType, OffsetFaceParams};

    let body = BodyId(Uuid::from_u128(0xBA));
    let at = |el: &str, p: Vec3| ElementRef {
        primary: Some(PrimaryRef {
            body,
            element: ElementId::new(el),
            kind: ElementKind::Face,
            extra: Default::default(),
        }),
        intent: None,
        anchor: Some(AnchorIntent {
            world_point: p,
            surface_uv: None,
            local_frame: None,
            adjacency_hint: None,
            extra: Default::default(),
        }),
        extra: Default::default(),
    };
    let top = Vec3::new_unchecked(1.0, 0.0, 10.0);
    let under = Vec3::new_unchecked(1.0, 0.0, 0.0);
    let op = Operation::Known(KnownOperation::OffsetFace(OffsetFaceParams {
        face_ids: vec![ElementId::new("f:1"), ElementId::new("f:2")],
        faces: vec![at("f:1", top), at("f:2", top)],
        distance: Scalar::new(12.0),
        distance_type: OffsetDistanceType::Offset,
        chain_tangent_faces: true,
        opposite_face_id: Some(ElementId::new("f:9")),
        opposite_face: Some(at("f:9", under)),
        target_body: body,
        extra: Default::default(),
    }));

    for (slot, el) in [(0usize, "f:1"), (1, "f:2"), (2, "f:9")] {
        let r = element_ref_input(&op, slot).unwrap_or_else(|| panic!("slot {slot}"));
        assert_eq!(r.primary.as_ref().unwrap().element.as_str(), el);
    }
    assert_eq!(
        element_ref_input(&op, 2)
            .and_then(|r| r.anchor.as_ref())
            .map(|a| a.world_point),
        Some(under),
        "the stored evidence rides the hydration, not just the id"
    );
    assert!(element_ref_input(&op, 3).is_none());

    // With no opposite face the table ends at the operative set — nothing
    // collapses into the vacated last slot.
    let Operation::Known(KnownOperation::OffsetFace(mut p)) = op else {
        panic!("expected OffsetFace");
    };
    p.opposite_face = None;
    p.opposite_face_id = None;
    let plain = Operation::Known(KnownOperation::OffsetFace(p));
    assert!(element_ref_input(&plain, 1).is_some());
    assert!(element_ref_input(&plain, 2).is_none());
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

/// H9: every `needs-repair` item names the body its step OPERATES ON.
///
/// Without it the frontend had to guess, and refused outright in a multi-body
/// document (a candidate `TopoKey` can only be promoted against a body) — repair
/// was unavailable in exactly the documents where topological naming is hardest.
/// The derivation is the record's own first input body, falling back to what the
/// step produced; it is never a guess.
#[tokio::test]
async fn needs_repair_items_name_the_operated_body() {
    use onecad_core::document::repair::{LadderLevel, RepairItem, RepairReason};

    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    let fillet_body = BodyId(Uuid::from_u128(0xBA1));
    let hole_body = BodyId(Uuid::from_u128(0xBA2));
    let anchor = Vec3::new_unchecked(1.0, 2.0, 3.0);
    rt.apply(EditCommand::AddOperation {
        record: fillet_record(0xFA1, fillet_body, "e:5", anchor),
        at_cursor: true,
    })
    .unwrap();
    rt.apply(EditCommand::AddOperation {
        record: hole_record_at(0xFA2, hole_body, "f:22", anchor),
        at_cursor: true,
    })
    .unwrap();

    // The repair state is a REGEN product; drive it directly so the derivation is
    // tested without standing up a worker.
    rt.regen.timeline = rt.session.document().timeline.clone();
    let item = |step: usize, ref_id: &str| RepairItem {
        step_index: step,
        ref_id: ref_id.into(),
        element_id: None,
        ladder_failed: LadderLevel::Descriptor,
        reason: RepairReason::Ambiguous,
        candidates: Vec::new(),
        scoring_version: Some(1),
        anchor: None,
        ui_label: String::new(),
        seeded: false,
        ordinal_anchor: None,
    };
    rt.regen.repair.set_step(0, vec![item(0, "op.input0")]);
    rt.regen.repair.set_step(1, vec![item(1, "op.input1")]);

    let items = rt.needs_repair_items();
    assert_eq!(items.len(), 2);
    assert_eq!(
        items[0].body_id.as_deref(),
        Some(fillet_body.to_string().as_str()),
        "a fillet's operated body is recovered from its typed edge refs"
    );
    assert_eq!(
        items[1].body_id.as_deref(),
        Some(hole_body.to_string().as_str()),
        "a hole's operated body is its targetBodyId"
    );
    // Bare uuid form — what the frontend's body registry / promote_selection use.
    assert!(
        !items[0].body_id.as_deref().unwrap().starts_with("body_"),
        "bodyId is the BARE uuid, not the worker wire form"
    );
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
    assert!(rt.undo().is_some(), "the extrude undoes");
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
    assert!(rt.undo().is_some());
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

// ─────────────────────────────────────────────────────────────────────────────
// WP-FIX W4 / VF-B5a — the host-face stamp at record mint/refresh
// ─────────────────────────────────────────────────────────────────────────────

/// A minimal `Sketch` timeline record for `sketch`, in the LEGACY shape (no
/// `host_face` param) — what `backfill_missing_sketch_records` authors.
fn legacy_sketch_record(seed: u128, sketch: SketchId) -> OperationRecord {
    use onecad_core::document::record::{PlaneKind, SketchOpParams, SketchPlaneRef};
    let op = Operation::Known(KnownOperation::Sketch(SketchOpParams {
        sketch,
        plane: SketchPlaneRef {
            kind: PlaneKind::Xy,
            origin: Vec3::new_unchecked(0.0, 0.0, 0.0),
            x_axis: Vec3::new_unchecked(0.0, 1.0, 0.0),
            y_axis: Vec3::new_unchecked(-1.0, 0.0, 0.0),
            normal: Vec3::new_unchecked(0.0, 0.0, 1.0),
            extra: Default::default(),
        },
        entities: vec![],
        constraints: vec![],
        host_face: None,
        extra: Default::default(),
    }));
    OperationRecord::new(RecordId(Uuid::from_u128(seed)), 0, "Sketch", op)
}

fn host_face_attachment(body: BodyId, el: &str) -> onecad_core::sketch::SketchAttachment {
    onecad_core::sketch::SketchAttachment::HostFace {
        face: ElementRef {
            primary: Some(PrimaryRef {
                body,
                element: ElementId::new(el),
                kind: ElementKind::Face,
                extra: Default::default(),
            }),
            intent: None,
            anchor: None,
            extra: Default::default(),
        },
        projected_boundary_version: 1,
    }
}

/// Finishing a face-hosted sketch STAMPS the record's `host_face`, which is what
/// gives the dependency graph (and the §7.3 edit-safety gate) the edge.
#[tokio::test]
async fn finish_sketch_stamps_the_host_face_on_the_minted_record() {
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    rt.apply(add_extrude(0x10, 25.0)).unwrap();
    rt.run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    let host = BodyId(Uuid::from_u128(0x10));

    let sid = SketchId(Uuid::from_u128(0x501));
    let sketch = Sketch::new(sid, "OnFace", host_face_attachment(host, "el_top"));
    rt.apply(EditCommand::AddSketch { sketch }).unwrap();
    rt.finish_sketch(sid).await.expect("finishSketch");

    let params = rt
        .projection()
        .features
        .iter()
        .find(|f| f.op_type == "Sketch")
        .and_then(|f| RecordId::from_str(&f.id).ok())
        .and_then(|r| rt.operation_params(r))
        .expect("the minted Sketch record's params");
    assert_eq!(
        params["hostFace"]["primary"]["elementId"], "el_top",
        "the mint path stamps the host-face dependency, got {params}"
    );
}

/// …but NOT when the record sits AHEAD of the body it would depend on. A legacy
/// container's sketch records are backfilled at the FRONT of the timeline
/// (`backfill_missing_sketch_records`), so a refresh that stamped `host_face`
/// there would author an edge `validate_temporal` must reject — turning an
/// ordinary re-finish into a hard error. The stamp is skipped instead; the gate's
/// attachment bridge still covers the sketch.
#[tokio::test]
async fn finish_sketch_skips_the_stamp_for_a_record_ahead_of_its_host_body() {
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    let sid = SketchId(Uuid::from_u128(0x502));
    // step 0: the front-inserted legacy Sketch record …
    rt.apply(EditCommand::AddOperation {
        record: legacy_sketch_record(0x502, sid),
        at_cursor: true,
    })
    .unwrap();
    // step 1: … and only THEN the op producing its host body.
    rt.apply(add_extrude(0x10, 25.0)).unwrap();
    rt.run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    let host = BodyId(Uuid::from_u128(0x10));

    let sketch = Sketch::new(sid, "OnFace", host_face_attachment(host, "el_top"));
    rt.apply(EditCommand::AddSketch { sketch }).unwrap();
    rt.finish_sketch(sid)
        .await
        .expect("finishSketch must NOT hard-fail on a legacy front-inserted record");

    let params = rt
        .operation_params(RecordId(Uuid::from_u128(0x502)))
        .expect("the legacy Sketch record's params");
    assert!(
        params.get("hostFace").is_none(),
        "no stamp when the host body is produced by a LATER op, got {params}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// HISTORY-HARDEN H5 — `intent.descriptor` hydration at the single writer
// ─────────────────────────────────────────────────────────────────────────────

/// The `intent` stored on the first (and only) edge ref of a Fillet record.
fn stored_edge_intent(rt: &DocumentRuntime, record: RecordId) -> Option<IntentQuery> {
    let rec = rt
        .session
        .document()
        .timeline
        .records()
        .iter()
        .find(|r| r.record_id == record)
        .expect("the record is in the timeline");
    let Operation::Known(KnownOperation::Fillet(p)) = &rec.op else {
        panic!("Fillet record")
    };
    p.edges[0].intent.clone()
}

/// Promotes one edge pick and returns its Rust-minted id.
async fn promote_edge(rt: &mut DocumentRuntime, body: BodyId, key: &str) -> ElementId {
    let promoted = rt
        .promote_selection(SnapshotId(0), body, vec![(TopoKey::new(key), None)])
        .await
        .expect("promote");
    ElementId::new(promoted[0].element_id.clone())
}

/// A ref authored against a promoted element is stamped with that element's
/// worker-authored descriptor — the evidence the SCHEMA §10 ladder scores. Without
/// it every ref ships `intent: null` and L2 degrades to a pure-anchor
/// nearest-centroid match.
#[tokio::test]
async fn add_operation_stamps_the_promoted_descriptor_onto_a_fresh_ref() {
    let backend = Arc::new(FakeBackend::new());
    let mut rt = runtime_with(backend.clone());
    let body = BodyId(Uuid::from_u128(0xB0));
    backend.set_descriptor(serde_json::json!({ "gen": 1 }));
    let edge = promote_edge(&mut rt, body, "e:7").await;

    let record = RecordId(Uuid::from_u128(0xF0));
    let mut rec = fillet_record(
        0xF0,
        body,
        edge.as_str(),
        Vec3::new_unchecked(1.0, 2.0, 3.0),
    );
    rec.record_id = record;
    assert!(
        matches!(&rec.op, Operation::Known(KnownOperation::Fillet(p)) if p.edges[0].intent.is_none()),
        "precondition: the FE authors no intent (it holds no descriptor)"
    );
    rt.apply(EditCommand::AddOperation {
        record: rec,
        at_cursor: true,
    })
    .unwrap();

    let intent = stored_edge_intent(&rt, record).expect("the ref was stamped");
    assert_eq!(intent.descriptor, serde_json::json!({ "gen": 1 }));
    assert_eq!(
        intent.kind,
        ElementKind::Edge,
        "the stamped kind is the worker's own classification of the promoted element"
    );
    assert_eq!(intent.version, 1, "the descriptor policy axis (SCHEMA §6)");
}

/// **Fill iff `intent.is_none()`.** An already-stamped ref is frozen evidence from
/// the moment it was authored (Invariant 2). Re-stamping it from a LATER promotion
/// would teach the ladder that post-move geometry was always the intended target —
/// the silent-rebind class this wave exists to kill. The complement is pinned in
/// the same test: a ref that arrives unstamped still picks up the CURRENT evidence,
/// so the freeze is a real decision and not a dead code path.
#[tokio::test]
async fn update_operation_params_never_overwrites_an_existing_intent() {
    let backend = Arc::new(FakeBackend::new());
    let mut rt = runtime_with(backend.clone());
    let body = BodyId(Uuid::from_u128(0xB0));
    let record = RecordId(Uuid::from_u128(0xF0));
    let anchor = Vec3::new_unchecked(1.0, 2.0, 3.0);

    backend.set_descriptor(serde_json::json!({ "gen": 1 }));
    let edge = promote_edge(&mut rt, body, "e:7").await;
    let mut rec = fillet_record(0xF0, body, edge.as_str(), anchor);
    rec.record_id = record;
    rt.apply(EditCommand::AddOperation {
        record: rec,
        at_cursor: true,
    })
    .unwrap();
    let authored = stored_edge_intent(&rt, record).expect("stamped at mint");
    assert_eq!(authored.descriptor, serde_json::json!({ "gen": 1 }));

    // The element is re-promoted after an edit moved it: the cache now holds gen 2
    // evidence under the SAME persistent id (Invariant 1 reuses the id).
    backend.set_descriptor(serde_json::json!({ "gen": 2 }));
    let same = promote_edge(&mut rt, body, "e:7").await;
    assert_eq!(same, edge, "Invariant 1: the re-pick reuses the id");

    // (a) A scalar re-edit that PRESERVES the stored ref (the `operation_params`
    //     path the app uses) keeps the gen-1 stamp byte-for-byte.
    let mut kept = fillet_record(0xF0, body, edge.as_str(), anchor);
    kept.record_id = record;
    if let Operation::Known(KnownOperation::Fillet(p)) = &mut kept.op {
        p.edges[0].intent = Some(authored.clone());
        p.radius = Scalar::new(3.0);
    }
    rt.apply(EditCommand::UpdateOperationParams {
        record,
        op: kept.op.clone(),
    })
    .unwrap();
    let after = stored_edge_intent(&rt, record).expect("still stamped");
    assert_eq!(
        serde_json::to_string(&after).unwrap(),
        serde_json::to_string(&authored).unwrap(),
        "an existing intent is NEVER refreshed — a post-move descriptor must not be \
         frozen onto a pre-move ref"
    );

    // (b) …and an unstamped ref (a genuinely re-picked edge) does take the current
    //     evidence, so (a) is a decision, not an inert branch.
    let mut fresh = fillet_record(0xF0, body, edge.as_str(), anchor);
    fresh.record_id = record;
    rt.apply(EditCommand::UpdateOperationParams {
        record,
        op: fresh.op.clone(),
    })
    .unwrap();
    assert_eq!(
        stored_edge_intent(&rt, record).unwrap().descriptor,
        serde_json::json!({ "gen": 2 }),
        "a re-authored ref picks up the CURRENT promotion evidence"
    );
}

/// A ref whose element this runtime never promoted (a legacy record, a FE-authored
/// id, a ref with no `primary`) is left EXACTLY as authored — anchor-only, the
/// pre-H5 behaviour. Never a fabricated descriptor.
#[tokio::test]
async fn an_unpromoted_ref_is_left_anchor_only() {
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    let body = BodyId(Uuid::from_u128(0xB0));
    let record = RecordId(Uuid::from_u128(0xF1));
    let mut rec = fillet_record(
        0xF1,
        body,
        "el_never_promoted",
        Vec3::new_unchecked(1.0, 2.0, 3.0),
    );
    rec.record_id = record;
    rt.apply(EditCommand::AddOperation {
        record: rec,
        at_cursor: true,
    })
    .unwrap();
    assert!(
        stored_edge_intent(&rt, record).is_none(),
        "no evidence ⇒ no stamp (the ladder falls back to the anchor, as before)"
    );
}

/// **No load-time backfill.** A document saved before H5 carries `intent: null` on
/// every ref. Opening it must not stamp anything: `inputs` and `params` are inside
/// the golden `history_prefix_hash`, so a backfill would silently move every
/// existing document's hash AND its `document.json` bytes, invalidating every
/// stored checkpoint against a change the user never made. (Mirrors the WP-FIX W4
/// `host_face` discipline, `transform_gate::a_legacy_host_face_document_reopens_and_resaves_byte_identical`.)
#[tokio::test]
async fn opening_a_legacy_document_does_not_backfill_intent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.onecad");
    let body = BodyId(Uuid::from_u128(0xB0));
    let record = RecordId(Uuid::from_u128(0xF0));

    // Author the legacy shape through a runtime whose promotion cache is EMPTY, so
    // the stored ref really is `intent: null` (never stamped).
    let mut rt = runtime_with(Arc::new(FakeBackend::new()));
    let mut rec = fillet_record(
        0xF0,
        body,
        "el_legacy_edge",
        Vec3::new_unchecked(1.0, 2.0, 3.0),
    );
    rec.record_id = record;
    rt.apply(EditCommand::AddOperation {
        record: rec,
        at_cursor: true,
    })
    .unwrap();
    assert!(stored_edge_intent(&rt, record).is_none(), "legacy shape");
    rt.save(&path, test_save_meta()).unwrap();
    let first = std::fs::read(&path).unwrap();
    let hash_before =
        onecad_core::regen::history_prefix_hash(rt.session.document().timeline.records());

    let backend = Arc::new(FakeBackend::new());
    let engine: Arc<dyn GeometryEngine> = backend.clone();
    let meshes: Arc<dyn MeshProvider> = backend.clone();
    let solver: Arc<dyn SolverEngine> = backend.clone();
    let mut reopened = DocumentRuntime::open(&path, engine, meshes, solver).unwrap();
    let op_bytes = |rt: &DocumentRuntime| {
        serde_json::to_string(
            &rt.session
                .document()
                .timeline
                .records()
                .iter()
                .find(|r| r.record_id == record)
                .unwrap()
                .op,
        )
        .unwrap()
    };
    let op_before = op_bytes(&reopened);
    assert!(
        stored_edge_intent(&reopened, record).is_none(),
        "no load-time backfill — the record is exactly as authored"
    );
    assert_eq!(
        hash_before,
        onecad_core::regen::history_prefix_hash(reopened.session.document().timeline.records()),
        "the golden prefix hash of a legacy document does not move"
    );
    let path2 = dir.path().join("resaved.onecad");
    reopened.save(&path2, test_save_meta()).unwrap();
    assert_eq!(
        first,
        std::fs::read(&path2).unwrap(),
        "a legacy document opened and re-saved is BYTE-IDENTICAL"
    );

    // …and warming the promotion cache for THAT VERY element id afterwards still
    // backfills nothing — the only configuration in which a lazy stamp could fire.
    backend.set_existing(Some(ElementId::new("el_legacy_edge")));
    backend.set_descriptor(serde_json::json!({ "gen": 9 }));
    reopened
        .promote_selection(SnapshotId(0), body, vec![(TopoKey::new("e:7"), None)])
        .await
        .expect("promote");
    assert!(
        stored_edge_intent(&reopened, record).is_none(),
        "a warm cache does not retro-stamp a stored ref"
    );
    assert_eq!(
        op_before,
        op_bytes(&reopened),
        "the legacy record's params bytes are untouched by a promotion"
    );
    assert_eq!(
        hash_before,
        onecad_core::regen::history_prefix_hash(reopened.session.document().timeline.records()),
        "…and so is its golden prefix hash"
    );
}
