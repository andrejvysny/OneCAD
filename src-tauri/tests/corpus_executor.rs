//! P4 real-worker corpus executor (WP4.4).
//!
//! `corpus/cases/*.json` is the frozen characterization of OneCAD-CPP at commit
//! `b4ddccc`: every numeric expectation cites the C++ line that produced it. This
//! test is what turns that from a document into evidence: it enumerates, types,
//! and executes every case. No corpus file is silently skipped.
//!
//! Three things the earlier version got wrong, all of the same kind — a claim with
//! nothing behind it:
//!
//! * **The classification was a hardcoded table in this file.** The coverage
//!   manifest (`docs/qa/modeling-operation-coverage.json`) also classifies every
//!   case, and the two could disagree without either failing. The manifest is now
//!   the single source: this test READS it, and a case missing from it is an error.
//! * **The one executed case asserted its last volume and nothing else.** Region
//!   counts, body events, solid counts and face counts sat in `expected` unread.
//! * **`expected` was never checked for provenance.** WP4.4 requires the citations
//!   to remain present; a number that loses its citation stops being a
//!   characterization and becomes a guess with a decimal point.
//!
//! Executed today: all nine frozen cases. The specialized runners cover hosted
//! profile booleans, MESH1 edge promotion, topology rebind, symmetric repair,
//! gesture/DOF solving, rollback-cursor replay, and profile-region detection.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use onecad_core::document::body::BodyLifecycleEvent;
use onecad_core::document::record::{
    BooleanMode, BooleanOp, BooleanParams, ExtrudeMode, ExtrudeParams, FilletParams,
    KnownOperation, Operation, OperationRecord, PlaneKind, SketchOpParams, SketchPlaneRef,
};
use onecad_core::document::refs::{
    AnchorIntent, ElementKind, ElementRef, PrimaryRef, SketchRegionRef,
};
use onecad_core::document::variables::Scalar;
use onecad_core::history::{DependencyGraph, Timeline};
use onecad_core::ids::{
    BodyId, ConstraintId, DocumentId, DocumentRevision, ElementId, EntityId, JobId, RecordId,
    RegionId, SketchId, SnapshotId, WorkerEpoch,
};
use onecad_core::math::{Vec2, Vec3};
use onecad_core::regen::{
    Fencing, GeometryEngine, Lod, OpenSessionRequest, PlanArtifacts, PlanContext, PlanEvent,
    PlanPrepared, PlanRequest, PolicyVersions, RegenPlanner, RegenRequest, ResolveOutcome,
    ResolveRef, ResolveRequest, SessionMode, StoppedReason,
};
use onecad_core::sketch::{
    Constraint, CurvePosition, Sketch, SketchAttachment, SketchEntity, WorldPlane,
};

use onecad_lib::worker::manager::SupervisorConfig;
use onecad_lib::worker::wire::{sketch_wire, GestureTarget};
use onecad_lib::worker::{
    resolve_worker_path, ElementQuery, MeshProvider, SolverEngine, WorkerManager,
};

use onecad_protocol::mesh::{f32_le, u32_le, validate_mesh_blob, MeshHeaderView};

include!("corpus_executor/model.rs");
include!("corpus_executor/plan.rs");
include!("corpus_executor/geometry.rs");
include!("corpus_executor/legacy.rs");
include!("corpus_executor/boolean_fillet.rs");
include!("corpus_executor/solver.rs");
include!("corpus_executor/entry.rs");
