//! R-WP7 golden fixtures for the pure [`RegenPlanner`] + checkpoint selection:
//!
//! * (i) determinism — same inputs ⇒ identical plan + history-prefix hash;
//! * (g) checkpoint restore — a compatible, hash-matching checkpoint accelerates
//!   the base, and the executor folds the remaining steps onto it (lifecycle
//!   correctness across restart);
//! * (h) corrupt/incompatible checkpoint — fallback to replay-from-0
//!   (correctness never depends on the cache — Invariant 7).

mod support;

use std::collections::BTreeMap;

use uuid::Uuid;

use onecad_core::document::body::{BodyLifecycleEvent, BodyRegistry};
use onecad_core::document::element_index::ElementIndex;
use onecad_core::document::record::{FilletParams, KnownOperation, Operation, OperationRecord};
use onecad_core::document::refs::{AnchorIntent, ElementKind, ElementRef, IntentQuery, PrimaryRef};
use onecad_core::document::variables::Scalar;
use onecad_core::history::{DependencyGraph, StepState, Timeline};
use onecad_core::ids::{BodyId, DocumentRevision, ElementId, JobId, WorkerEpoch};
use onecad_core::math::Vec3;
use onecad_core::regen::{
    history_prefix_hash, CancelToken, CheckpointArtifact, CheckpointArtifacts, CheckpointEnvelope,
    CheckpointStore, HistoryPrefixHash, InMemoryCheckpointStore, OpFailureCode, Outcome,
    PlanArtifacts, PolicyVersions, RegenExecutor, RegenPlanner, RegenRequest, RegenSession,
    SnapshotPublisher, StoppedReason,
};

use support::*;

const JOB: JobId = JobId(Uuid::from_u128(0x205));
const REV: DocumentRevision = DocumentRevision(7);
const EPOCH: WorkerEpoch = WorkerEpoch(3);

/// Builds a compatible checkpoint envelope for `step` whose stored history-prefix
/// hash matches `records[0..=step]` (so the planner accepts it).
fn compatible_envelope(step: usize, tl: &Timeline) -> CheckpointEnvelope {
    CheckpointEnvelope {
        artifact_schema_version: 1,
        body: BodyId(Uuid::from_u128(0xB0)),
        step,
        history_prefix_hash: history_prefix_hash(&tl.records()[0..=step]),
        brep_content_hash: "aa".into(),
        occt_fingerprint: "fp".into(), // matches default_ctx()
        descriptor_version: 1,
        resolver_version: 1,
        quantization_version: 1,
        signature_version: 1,
        codec: "brep-bintools".into(),
        size: 10,
        content_hash: "bb".into(),
    }
}

fn artifacts_for(step: usize, envelope: CheckpointEnvelope) -> CheckpointArtifacts {
    CheckpointArtifacts {
        step,
        artifacts: vec![CheckpointArtifact {
            envelope: envelope.clone(),
            bytes: vec![1, 2, 3],
        }],
        element_map_partition: vec![],
        signatures: sigs(step),
        history_prefix_hash: envelope.history_prefix_hash,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// (i) determinism
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn planner_is_deterministic() {
    let tl = timeline_of(4);
    let g = DependencyGraph::new();
    let a = RegenPlanner::plan(
        &tl,
        &g,
        &[],
        RegenRequest::ToEnd { from: 0 },
        &default_ctx(),
    );
    let b = RegenPlanner::plan(
        &tl,
        &g,
        &[],
        RegenRequest::ToEnd { from: 0 },
        &default_ctx(),
    );
    assert_eq!(a, b, "same inputs → identical plan");
    assert_eq!(a.expected_base_hash, b.expected_base_hash);

    // The hash is prefix-sensitive and stable.
    let r = tl.records();
    assert_eq!(history_prefix_hash(&r[0..2]), history_prefix_hash(&r[0..2]));
    assert_ne!(history_prefix_hash(&r[0..2]), history_prefix_hash(&r[0..3]));
}

// ─────────────────────────────────────────────────────────────────────────────
// (g) checkpoint restore path
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn planner_selects_compatible_checkpoint_as_base() {
    let tl = timeline_of(4);
    let mut store = InMemoryCheckpointStore::new();
    // A checkpoint after step 1, compatible + hash-matching.
    store.save(1, artifacts_for(1, compatible_envelope(1, &tl)));

    let plan = RegenPlanner::plan(
        &tl,
        &DependencyGraph::new(),
        &store.list(),
        RegenRequest::ToEnd { from: 2 },
        &default_ctx(),
    );

    // Restored from the step-1 checkpoint ⇒ execute [2, 3] only.
    let restore = plan.restore.as_ref().expect("checkpoint chosen");
    assert_eq!(restore.step_index, 1);
    assert_eq!(plan.start_step, 2);
    assert_eq!(plan.target_step, 3);
    assert_eq!(plan.planned_ops.len(), 2);
    assert_eq!(plan.planned_ops[0].step_index, 2);
    // expected_base_hash is over records[0..2] (Invariant: what the checkpoint stored).
    assert_eq!(
        plan.expected_base_hash,
        history_prefix_hash(&tl.records()[0..2])
    );
}

/// Builds the two-body restored base a step-1 checkpoint represents (its lifecycle
/// log ends at the checkpoint step). The executor seeds scratch from THIS (via
/// `restore_checkpoint`), not from live session state (review F3).
fn restored_base(a: BodyId, b: BodyId) -> BodyRegistry {
    let mut reg = BodyRegistry::new();
    reg.fold(0, rid(0x10), BodyLifecycleEvent::Created { body: a });
    reg.fold(1, rid(0x11), BodyLifecycleEvent::Created { body: b });
    reg
}

#[tokio::test]
async fn executor_folds_remaining_steps_onto_restored_base() {
    // The checkpoint plan starts at step 2; the executor reconstructs the base from
    // the checkpoint artifacts (restore_checkpoint) and folds only steps 2, 3.
    // F3: re-running the same plan does NOT duplicate lifecycle-log entries,
    // because each run reseeds from the immutable restore result — not from the
    // (now-ahead) live registry.
    let tl = timeline_of(4);
    let mut store = InMemoryCheckpointStore::new();
    store.save(1, artifacts_for(1, compatible_envelope(1, &tl)));

    let plan = RegenPlanner::plan(
        &tl,
        &DependencyGraph::new(),
        &store.list(),
        RegenRequest::ToEnd { from: 2 },
        &default_ctx(),
    );
    assert!(plan.restore.is_some(), "precondition: checkpoint chosen");

    let restored_a = BodyId(Uuid::from_u128(0xA0));
    let restored_b = BodyId(Uuid::from_u128(0xB0));
    let restore = RestoreConfig {
        restored: true,
        drift: false,
        base_registry: restored_base(restored_a, restored_b),
        base_elements: ElementIndex::new(),
    };
    let exec = RegenExecutor::new(FakeEngine::all_ok().with_restore(restore));

    // The live session is EMPTY — the executor must reconstruct the base from the
    // checkpoint, not from the session.
    let mut session = RegenSession::with_timeline(tl.clone());
    let publisher = SnapshotPublisher::new();
    let gate = move || (REV, EPOCH);
    let cancel = CancelToken::new();

    let req = plan.clone().into_request(
        JOB,
        REV,
        EPOCH,
        PolicyVersions::default(),
        PlanArtifacts::default(),
    );
    let out = exec
        .run(req, &mut session, &gate, &cancel, &publisher)
        .await;
    assert!(matches!(out, Outcome::Published(_)), "got {out:?}");
    // Restored 2 + folded 2 = 4 active bodies; lifecycle log appended (not
    // duplicated — clear-before-replay respected via the restored base).
    assert_eq!(session.bodies.len(), 4);
    assert_eq!(session.bodies.log().len(), 4, "2 restored + 2 replayed");
    assert!(session.bodies.contains(restored_a));
    assert_eq!(session.timeline.state(2), Some(&StepState::Valid));
    assert_eq!(session.timeline.state(3), Some(&StepState::Valid));

    // F3: re-run the SAME checkpoint plan. The live session is now ahead (4 bodies,
    // log 4), but the executor reseeds from the restore result ⇒ still 4 / log 4,
    // NOT 6 / log 6.
    let req2 = plan.into_request(
        JOB,
        REV,
        EPOCH,
        PolicyVersions::default(),
        PlanArtifacts::default(),
    );
    let out2 = exec
        .run(req2, &mut session, &gate, &cancel, &publisher)
        .await;
    assert!(matches!(out2, Outcome::Published(_)), "got {out2:?}");
    assert_eq!(session.bodies.len(), 4, "no body accumulation on re-run");
    assert_eq!(
        session.bodies.log().len(),
        4,
        "no lifecycle-log duplication on re-run (F3)"
    );
    assert_eq!(exec.engine().restore_calls(), 2, "restore per run");
}

// ─────────────────────────────────────────────────────────────────────────────
// (h) corrupt / incompatible checkpoint → replay-from-0
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn incompatible_fingerprint_checkpoint_falls_back_to_replay_from_zero() {
    let tl = timeline_of(4);
    let mut store = InMemoryCheckpointStore::new();
    // Fingerprint mismatch (worker rebuilt under a different OCCT) ⇒ discard.
    let mut env = compatible_envelope(1, &tl);
    env.occt_fingerprint = "STALE_FP".into();
    store.save(1, artifacts_for(1, env));

    let plan = RegenPlanner::plan(
        &tl,
        &DependencyGraph::new(),
        &store.list(),
        RegenRequest::ToEnd { from: 2 },
        &default_ctx(),
    );

    assert!(plan.restore.is_none(), "incompatible checkpoint discarded");
    assert_eq!(plan.start_step, 0, "replay from empty base");
    assert_eq!(plan.expected_base_hash, HistoryPrefixHash::empty());
    assert_eq!(plan.planned_ops.len(), 4);
}

/// Signature/policy drift on any version axis is the same gate as the fingerprint:
/// `is_compatible` rejects the checkpoint and the planner replays from 0 — graceful
/// degradation, never an error (Invariant 7). `descriptorVersion` stands in for the
/// four version axes; the container no longer persists checkpoints, so this is where
/// the drift path lives (it used to be a doctored-container integration test).
#[test]
fn descriptor_version_drift_checkpoint_falls_back_to_replay_from_zero() {
    let tl = timeline_of(4);
    let mut store = InMemoryCheckpointStore::new();
    let mut env = compatible_envelope(1, &tl);
    env.descriptor_version = 999_999;
    store.save(1, artifacts_for(1, env));

    let plan = RegenPlanner::plan(
        &tl,
        &DependencyGraph::new(),
        &store.list(),
        RegenRequest::ToEnd { from: 2 },
        &default_ctx(),
    );

    assert!(
        plan.restore.is_none(),
        "a version-drifted checkpoint is discarded by the compatibility gate"
    );
    assert_eq!(plan.start_step, 0, "replay from empty base");
    assert_eq!(plan.expected_base_hash, HistoryPrefixHash::empty());
    assert_eq!(plan.planned_ops.len(), 4);
}

#[test]
fn stale_prefix_hash_checkpoint_falls_back_to_replay_from_zero() {
    let tl = timeline_of(4);
    let mut store = InMemoryCheckpointStore::new();
    // Envelope compatible, but the stored history-prefix hash no longer matches
    // records[0..=1] (an upstream record was edited) ⇒ stale ⇒ discard.
    let mut env = compatible_envelope(1, &tl);
    env.history_prefix_hash = HistoryPrefixHash::new("0000stalehash0000");
    let mut artifacts = artifacts_for(1, env.clone());
    artifacts.history_prefix_hash = env.history_prefix_hash.clone();
    store.save(1, artifacts);

    let plan = RegenPlanner::plan(
        &tl,
        &DependencyGraph::new(),
        &store.list(),
        RegenRequest::ToEnd { from: 2 },
        &default_ctx(),
    );

    assert!(plan.restore.is_none(), "stale checkpoint discarded");
    assert_eq!(plan.start_step, 0);
    assert_eq!(plan.planned_ops.len(), 4);
}

#[tokio::test]
async fn replay_from_zero_produces_same_result_as_the_naive_baseline() {
    // Invariant 7: an unusable cache degrades performance, never correctness. The
    // fallback plan replays everything and still succeeds identically.
    let tl = timeline_of(3);
    let plan = RegenPlanner::plan(
        &tl,
        &DependencyGraph::new(),
        &[], // no checkpoints
        RegenRequest::ToEnd { from: 0 },
        &default_ctx(),
    );
    let req = plan.into_request(
        JOB,
        REV,
        EPOCH,
        PolicyVersions::default(),
        PlanArtifacts::default(),
    );

    let exec = RegenExecutor::new(FakeEngine::all_ok());
    let mut session = RegenSession::with_timeline(tl);
    let publisher = SnapshotPublisher::new();
    let gate = move || (REV, EPOCH);
    let cancel = CancelToken::new();
    let out = exec
        .run(req, &mut session, &gate, &cancel, &publisher)
        .await;

    match out {
        Outcome::Published(s) => {
            assert_eq!(s.stopped_reason, StoppedReason::Completed);
            assert_eq!(session.bodies.len(), 3);
        }
        other => panic!("expected Published, got {other:?}"),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// (F12) Invariant 7: a broken checkpoint transparently replays from 0 (once)
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn checkpoint_restore_drift_retries_from_zero() {
    // A compatible, hash-matching checkpoint is chosen, but the restore reports
    // DRIFT (worker's restored base disagrees) ⇒ the checkpoint is unusable ⇒ the
    // executor strips it and replays from 0, transparently succeeding (F12).
    let tl = timeline_of(4);
    let mut store = InMemoryCheckpointStore::new();
    store.save(1, artifacts_for(1, compatible_envelope(1, &tl)));
    let plan = RegenPlanner::plan(
        &tl,
        &DependencyGraph::new(),
        &store.list(),
        RegenRequest::ToEnd { from: 2 },
        &default_ctx(),
    );
    assert!(plan.restore.is_some(), "precondition: checkpoint chosen");
    let req = plan.into_request(
        JOB,
        REV,
        EPOCH,
        PolicyVersions::default(),
        PlanArtifacts::default(),
    );

    let restore = RestoreConfig {
        restored: true,
        drift: true,
        base_registry: BodyRegistry::new(),
        base_elements: ElementIndex::new(),
    };
    let exec = RegenExecutor::new(FakeEngine::all_ok().with_restore(restore));
    let mut session = RegenSession::with_timeline(tl);
    let publisher = SnapshotPublisher::new();
    let gate = move || (REV, EPOCH);
    let cancel = CancelToken::new();
    let out = exec
        .run(req, &mut session, &gate, &cancel, &publisher)
        .await;

    assert!(
        matches!(out, Outcome::Published(_)),
        "from-0 retry succeeds transparently, got {out:?}"
    );
    // Replay-from-0 executed all 4 steps ⇒ 4 bodies.
    assert_eq!(session.bodies.len(), 4);
    let log = exec.engine().log();
    // The checkpoint attempt aborted at restore (never reached execute_plan); only
    // the from-0 retry did — and it carried NO checkpoint.
    assert_eq!(
        log.plans.len(),
        1,
        "only the from-0 retry reached execute_plan"
    );
    assert!(
        log.plans[0].base_checkpoint.is_none(),
        "retry stripped the checkpoint"
    );
    assert_eq!(log.plans[0].ops.len(), 4, "retry replays all 4 steps");
    assert_eq!(exec.engine().restore_calls(), 1, "restore attempted once");
    // SCHEMA §7.2: the retry — and ONLY the retry — claims the fallback provenance.
    // Without this the worker cannot tell it from an ordinary replay-from-0, which
    // is the whole of VF-M5 (see topology_rebind.rs lane D).
    assert!(
        log.plans[0].checkpoint_fallback_replay,
        "the F12 retry MUST mark itself as a checkpoint-fallback replay"
    );
}

#[tokio::test]
async fn an_ordinary_plan_never_claims_the_checkpoint_fallback() {
    // The negative control for the assertion above: a plain from-0 regen with no
    // checkpoint in play is byte-identical in shape to the F12 retry, so the flag
    // is the ONLY thing separating them and it must stay off here.
    let tl = timeline_of(3);
    let plan = RegenPlanner::plan(
        &tl,
        &DependencyGraph::new(),
        &[],
        RegenRequest::ToEnd { from: 0 },
        &default_ctx(),
    );
    assert!(plan.restore.is_none(), "precondition: no checkpoint chosen");
    let req = plan.into_request(
        JOB,
        REV,
        EPOCH,
        PolicyVersions::default(),
        PlanArtifacts::default(),
    );
    assert!(
        !req.checkpoint_fallback_replay,
        "into_request defaults the claim to absent"
    );

    let exec = RegenExecutor::new(FakeEngine::all_ok());
    let mut session = RegenSession::with_timeline(tl);
    let publisher = SnapshotPublisher::new();
    let gate = move || (REV, EPOCH);
    let cancel = CancelToken::new();
    let out = exec
        .run(req, &mut session, &gate, &cancel, &publisher)
        .await;
    assert!(matches!(out, Outcome::Published(_)), "got {out:?}");
    let log = exec.engine().log();
    assert_eq!(log.plans.len(), 1);
    assert!(
        !log.plans[0].checkpoint_fallback_replay,
        "an ordinary regen must never claim it — that claim disables the §10 \
         anchor-exact carve-out and would NeedsRepair the shipped edit lane"
    );
}

#[tokio::test]
async fn checkpoint_op_failure_does_not_retry_from_zero() {
    // A real op failure (a step event WAS received) must NOT trigger the from-0
    // retry — it surfaces as an accepted m-1 snapshot (F12: retries are only for
    // failures BEFORE any step event).
    let tl = timeline_of(4);
    let mut store = InMemoryCheckpointStore::new();
    store.save(1, artifacts_for(1, compatible_envelope(1, &tl)));
    let plan = RegenPlanner::plan(
        &tl,
        &DependencyGraph::new(),
        &store.list(),
        RegenRequest::ToEnd { from: 2 },
        &default_ctx(),
    );
    let req = plan.into_request(
        JOB,
        REV,
        EPOCH,
        PolicyVersions::default(),
        PlanArtifacts::default(),
    );

    let restore = RestoreConfig {
        restored: true,
        drift: false,
        base_registry: restored_base(BodyId(Uuid::from_u128(0xA0)), BodyId(Uuid::from_u128(0xB0))),
        base_elements: ElementIndex::new(),
    };
    let mut scripts = BTreeMap::new();
    scripts.insert(
        3,
        StepScript::Fail {
            code: OpFailureCode::GeometryInvalid,
        },
    );
    let exec = RegenExecutor::new(FakeEngine::new(scripts).with_restore(restore));
    let mut session = RegenSession::with_timeline(tl);
    let publisher = SnapshotPublisher::new();
    let gate = move || (REV, EPOCH);
    let cancel = CancelToken::new();
    let out = exec
        .run(req, &mut session, &gate, &cancel, &publisher)
        .await;

    let snap = match out {
        Outcome::Published(s) => s,
        other => panic!("op failure → accepted m-1, got {other:?}"),
    };
    assert_eq!(snap.stopped_reason, StoppedReason::OpFailed);
    let log = exec.engine().log();
    assert_eq!(log.plans.len(), 1, "no from-0 retry on a real op failure");
    assert!(
        log.plans[0].base_checkpoint.is_some(),
        "still the checkpoint plan (not stripped)"
    );
    assert_eq!(exec.engine().restore_calls(), 1, "restored once, no retry");
}

// ─────────────────────────────────────────────────────────────────────────────
// M16 (HISTORY-HARDEN H5) — the stamped `intent.descriptor` enters the
// golden-pinned history-prefix hash, so its JSON KEY ORDER is a wire contract.
// ─────────────────────────────────────────────────────────────────────────────

/// A fillet whose edge ref carries the exact `intent` shape the single writer
/// stamps: the descriptor is the verbatim object the worker's
/// `ElementMapPartition::descriptor_to_json` emits (OCCT enums as ints,
/// `adjacencyHash` as a 16-char hex string — SCHEMA §7.3 / §10).
fn fillet_with_stamped_intent() -> OperationRecord {
    let descriptor = serde_json::json!({
        "shapeType": 6,      // TopAbs_EDGE
        "center": [10.0, 40.0, 0.0],
        "size": 10.0,
        "magnitude": 10.0,
        "surfaceType": 0,    // GeomAbs_Plane
        "curveType": 0,      // GeomAbs_Line
        "normal": [0.0, 0.0, 1.0],
        "tangent": [1.0, 0.0, 0.0],
        "hasNormal": false,
        "hasTangent": true,
        "adjacencyHash": "d41d8cd98f00b204",
    });
    let edge = ElementId::new("el_000000000000000000000000000004a1");
    let op = Operation::Known(KnownOperation::Fillet(FilletParams {
        radius: Scalar::new(2.0),
        edge_ids: vec![edge.clone()],
        edges: vec![ElementRef {
            primary: Some(PrimaryRef {
                body: BodyId(Uuid::from_u128(0xB0)),
                element: edge,
                kind: ElementKind::Edge,
                extra: Default::default(),
            }),
            intent: Some(IntentQuery {
                version: 1,
                kind: ElementKind::Edge,
                descriptor,
                extra: Default::default(),
            }),
            anchor: Some(AnchorIntent {
                world_point: Vec3::new_unchecked(10.0, 40.0, 0.0),
                surface_uv: None,
                local_frame: None,
                adjacency_hint: None,
                extra: Default::default(),
            }),
            extra: Default::default(),
        }],
        chain_tangent_edges: false,
        tangent_closure_version: None,
        extra: Default::default(),
    }));
    OperationRecord::new(rid(0x5), 0, "Fillet", op)
}

/// The descriptor is an opaque `serde_json::Value`, so its key order is whatever
/// `serde_json::Map` happens to be — sorted today (a `BTreeMap`; the crate is built
/// WITHOUT the `preserve_order` feature). Enabling that feature anywhere in the
/// dependency graph would silently switch it to insertion order, move every
/// stamped record's bytes, and invalidate every stored checkpoint against a hash
/// nothing else changed. Pinned as exact bytes so the swap cannot land quietly.
#[test]
fn h5_stamped_intent_descriptor_serializes_in_a_pinned_key_order() {
    let rec = fillet_with_stamped_intent();
    let op_json = serde_json::to_string(&rec.op).expect("op serializes");
    assert_eq!(
        op_json,
        concat!(
            r#"{"opType":"Fillet","params":{"radius":{"value":2.0},"#,
            r#""edgeIds":["el_000000000000000000000000000004a1"],"#,
            r#""edges":[{"primary":{"bodyId":"00000000-0000-0000-0000-0000000000b0","#,
            r#""elementId":"el_000000000000000000000000000004a1","kind":"edge"},"#,
            r#""intent":{"version":1,"kind":"edge","descriptor":{"#,
            // ── alphabetical, NOT the insertion order of `descriptor_to_json` ──
            r#""adjacencyHash":"d41d8cd98f00b204","center":[10.0,40.0,0.0],"curveType":0,"#,
            r#""hasNormal":false,"hasTangent":true,"magnitude":10.0,"normal":[0.0,0.0,1.0],"#,
            r#""shapeType":6,"size":10.0,"surfaceType":0,"tangent":[1.0,0.0,0.0]}},"#,
            r#""anchor":{"worldPoint":[10.0,40.0,0.0]}}],"chainTangentEdges":false}}"#,
        ),
        "the stamped descriptor's key order is a wire contract — see the test doc"
    );
}

/// The same record's golden [`history_prefix_hash`] — the value the planner
/// compares against every stored checkpoint and the worker echoes back. It moves
/// only when the canonical wire form of a stamped ref genuinely changes.
#[test]
fn h5_stamped_intent_history_prefix_hash_is_golden() {
    let hash = history_prefix_hash(&[fillet_with_stamped_intent()]);
    assert_eq!(
        hash.to_string(),
        "53b231a27191a637f6bb95d927d8f8765b3c78d0904a6ffbda34fc3dc619ac48"
    );
}
