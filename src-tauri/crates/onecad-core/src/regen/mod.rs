//! Regeneration: the geometry-engine abstraction, plan compilation, execution,
//! scheduling, snapshots and checkpoints — the heart of the system.
//!
//! Data flow (SCHEMA §7.2 ExecutePlan model):
//!
//! ```text
//!   Timeline ─┐                         ┌─ ModelSnapshot (published via watch)
//!             ├─ RegenPlanner::plan ──▶ RegenPlan ──▶ PlanRequest
//! checkpoints ┘                                           │
//!                                          RegenExecutor::run
//!                                            │  drives GeometryEngine::execute_plan
//!                                            │  folds PlanStepEvents into scratch
//!                                            │  fences (RevisionGate) → accept/discard
//!                                            ▼
//!                                          Outcome { Published | Superseded |
//!                                                    EngineFailed | Cancelled }
//! ```
//!
//! * [`engine`] — the [`GeometryEngine`] async trait + all core-level plan/event
//!   types (transport-agnostic; the app layer maps them to the OCW1 wire).
//! * [`planner`] — the pure [`RegenPlanner`] + the history-prefix hash.
//! * [`executor`] — the [`RegenExecutor`] drive loop (scratch fold, fencing,
//!   cancellation).
//! * [`snapshot`] — the immutable [`ModelSnapshot`] + [`SnapshotPublisher`].
//! * [`checkpoint`] — the [`CheckpointStore`] + versioned envelope validation.
//! * [`scheduler`] — the single-in-flight, latest-wins debounce/coalesce/cancel
//!   funnel (R-WP8); engine-agnostic, driving an app-supplied [`RegenDriver`].
//!
//! ## Test double
//!
//! The scripted `FakeEngine` that backs the golden fixtures lives in the
//! integration-test support module `tests/support/mod.rs` (not in the library),
//! matching this crate's existing `tests/common` pattern: it keeps the test
//! double out of production builds while remaining shared across the
//! `regen_executor` / `regen_planner` integration tests, and it runs under a
//! plain `cargo test` with no feature flag.

pub mod checkpoint;
pub mod engine;
pub mod executor;
pub mod planner;
pub mod scheduler;
pub mod snapshot;

pub use checkpoint::{
    CheckpointArtifact, CheckpointArtifacts, CheckpointEnvelope, CheckpointId, CheckpointMeta,
    CheckpointRef, CheckpointStore, DriftDetail, DriftSignature, InMemoryCheckpointStore,
    RestoreResult, StoredCheckpoint, ARTIFACT_SCHEMA_VERSION,
};
pub use engine::{
    mint_element_ids, AcceptResult, AcquireRequest, BindElementIdsRequest, BodySelector,
    Diagnostic, ElementBinding, ElementMapDelta, ElementMapEntry, EngineError, Fencing,
    GeometryEngine, MeshHandle, OpFailureCode, OpenSessionRequest, Pick, PlanArtifacts, PlanEvent,
    PlanPrepared, PlanRequest, PlanStepEvent, PlannedOp, PolicyVersions, PreparedMeshRef,
    RefResolution, ResolveOutcome, ResolveRef, ResolveRequest, RestoreRequest, SessionMode,
    Severity, Signature, StepResult, StepSignatures, StepStatus, StoppedReason, TessellateRequest,
    TessellateResult, TessellateSpec, WorkerElementEvidence, WorkerHead,
};
pub use executor::{CancelToken, MeshSink, Outcome, RegenExecutor, RegenSession, RevisionGate};
pub use planner::{
    history_prefix_hash, HistoryPrefixHash, PlanContext, RegenPlan, RegenPlanner, RegenRequest,
};
pub use scheduler::{
    JobKind, RegenDirective, RegenDriver, RegenScheduler, SchedulerConfig, SchedulerHandle,
    SchedulerStatus, CANCEL_GRACE_SECS, PREVIEW_DEBOUNCE_MS,
};
pub use snapshot::{BodySnapshot, Lod, MeshKey, ModelSnapshot, RepairSummary, SnapshotPublisher};
