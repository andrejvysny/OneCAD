//! H10 dependency view — `DocumentRuntime::feature_dependencies`, the method the
//! `feature_dependencies` tauri command (`api/mod.rs`) thinly wraps. Pure Rust
//! over the session's derived [`DependencyGraph`]; no worker/geometry needed, so
//! this runs over [`PendingBackend`] like `autosave_recovery.rs`/`persistence_lane.rs`.
//!
//! The graph's body-producer edges come from `OperationRecord::outputs`
//! (`DependencyGraph::rebuild_edges`, called by `DocumentSession::add_operation`
//! on every `AddOperation`) — so a record's `outputs` is set explicitly here
//! rather than relying on a regen that never runs in this harness.
//!
//! [`DependencyGraph`]: onecad_core::history::DependencyGraph

use std::sync::Arc;

use uuid::Uuid;

use onecad_core::document::record::{
    BooleanMode, ExtrudeMode, ExtrudeParams, KnownOperation, Operation, OperationRecord,
};
use onecad_core::document::variables::Scalar;
use onecad_core::edit::EditCommand;
use onecad_core::ids::{BodyId, RecordId};
use onecad_core::regen::GeometryEngine;

use onecad_lib::document_runtime::DocumentRuntime;
use onecad_lib::worker::{MeshProvider, PendingBackend, SolverEngine};

fn pending_runtime() -> DocumentRuntime {
    let backend = Arc::new(PendingBackend);
    let engine: Arc<dyn GeometryEngine> = backend.clone();
    let meshes: Arc<dyn MeshProvider> = backend.clone();
    let solver: Arc<dyn SolverEngine> = backend;
    DocumentRuntime::new_blank(engine, meshes, solver)
}

/// An Extrude record. `target_body: None` + `NewBody` mints a body (no input
/// edge); `Some(body)` + `Add` both depends on and re-produces it — the shape
/// `Operation::derive_inputs` needs to form a body-producer chain
/// (`record.rs` `KnownOperation::Extrude` arm).
fn extrude_record(seed: u128, target_body: Option<BodyId>, output: BodyId) -> OperationRecord {
    let mode = target_body.map_or(BooleanMode::NewBody, |_| BooleanMode::Add);
    let op = Operation::Known(KnownOperation::Extrude(ExtrudeParams {
        profile: None,
        distance: Scalar::new(10.0),
        draft_angle_deg: Scalar::new(0.0),
        mode: ExtrudeMode::Blind,
        boolean_mode: mode,
        target_body,
        target_face: None,
        two_directions: false,
        mode2: ExtrudeMode::Blind,
        distance2: Scalar::new(0.0),
        target_face2: None,
        extra: Default::default(),
    }));
    let mut record = OperationRecord::new(RecordId(Uuid::from_u128(seed)), 0, "Extrude", op);
    record.outputs = vec![output];
    record
}

/// A 3-record chain R1 -> R2 -> R3, all producing/consuming the SAME body B1
/// (each an Extrude that unions into the prior's output) — a real transitive
/// producer chain, not a stub.
fn chained_runtime() -> (DocumentRuntime, RecordId, RecordId, RecordId) {
    let body = BodyId(Uuid::from_u128(0xB0D1));
    let r1 = RecordId(Uuid::from_u128(0x51));
    let r2 = RecordId(Uuid::from_u128(0x52));
    let r3 = RecordId(Uuid::from_u128(0x53));

    let mut rt = pending_runtime();
    rt.apply(EditCommand::AddOperation {
        record: extrude_record(0x51, None, body),
        at_cursor: true,
    })
    .expect("R1 AddOperation");
    rt.apply(EditCommand::AddOperation {
        record: extrude_record(0x52, Some(body), body),
        at_cursor: true,
    })
    .expect("R2 AddOperation");
    rt.apply(EditCommand::AddOperation {
        record: extrude_record(0x53, Some(body), body),
        at_cursor: true,
    })
    .expect("R3 AddOperation");
    (rt, r1, r2, r3)
}

#[test]
fn upstream_and_downstream_are_the_transitive_producer_chain() {
    let (rt, r1, r2, r3) = chained_runtime();

    // R2 depends on R1 (its target body's producer); R2's own downstream is R3.
    let deps2 = rt.feature_dependencies(r2).expect("R2 is a known record");
    assert_eq!(deps2.upstream, vec![r1.to_string()]);
    assert_eq!(deps2.downstream, vec![r3.to_string()]);

    // R1's downstream is the FULL transitive closure {R2, R3}, never itself.
    let deps1 = rt.feature_dependencies(r1).expect("R1 is a known record");
    assert!(deps1.upstream.is_empty());
    let mut downstream1 = deps1.downstream.clone();
    downstream1.sort();
    let mut expected_23 = vec![r2.to_string(), r3.to_string()];
    expected_23.sort();
    assert_eq!(downstream1, expected_23);
    assert!(!deps1.downstream.contains(&r1.to_string()));

    // R3's upstream is the full transitive closure {R1, R2}; nothing downstream.
    let deps3 = rt.feature_dependencies(r3).expect("R3 is a known record");
    assert!(deps3.downstream.is_empty());
    let mut upstream3 = deps3.upstream.clone();
    upstream3.sort();
    let mut expected_12 = vec![r1.to_string(), r2.to_string()];
    expected_12.sort();
    assert_eq!(upstream3, expected_12);
}

#[test]
fn an_unknown_record_returns_none() {
    let (rt, ..) = chained_runtime();
    let unknown = RecordId(Uuid::from_u128(0xDEAD));
    assert!(rt.feature_dependencies(unknown).is_none());
}

#[test]
fn a_suppressed_record_is_still_a_valid_query_target() {
    let (mut rt, r1, r2, r3) = chained_runtime();

    rt.apply(EditCommand::SetOperationSuppression {
        record: r2,
        suppressed: true,
        cascade: false,
    })
    .expect("suppress R2");

    // Suppression is a node FLAG on the graph, not removal — R2 stays listed with
    // its edges intact, and so does everything around it.
    let deps2 = rt
        .feature_dependencies(r2)
        .expect("a suppressed record is still a known node");
    assert_eq!(deps2.upstream, vec![r1.to_string()]);
    assert_eq!(deps2.downstream, vec![r3.to_string()]);
}
