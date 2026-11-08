//! Evidence belongs to persisted identity, not to the worker's partition delta.
mod support;

use onecad_core::document::{
    element_index::ElementEntry,
    refs::{AnchorIntent, ElementKind},
};
use onecad_core::ids::{DocumentRevision, JobId, TopoKey, WorkerEpoch};
use onecad_core::math::Vec3;
use onecad_core::regen::{
    CancelToken, ElementMapDelta, ElementMapEntry, Outcome, RegenExecutor, RegenRequest,
    RegenSession, SnapshotPublisher,
};
use std::collections::BTreeMap;
use support::*;
use uuid::Uuid;

fn evidenced_entry(body: onecad_core::ids::BodyId) -> ElementEntry {
    let cert = serde_json::from_value(serde_json::json!({
        "version":1,"sourceSha256":"a".repeat(64),"codec":"step","codecVersion":"occt-step-stream-v1",
        "readerPolicy":"step-v1","kernelFingerprint":"0a6a1dce34181289",
        "producerRecordId":"00000000-0000-0000-0000-000000000001","bodyId":body,
        "unitScaleBits":"3ff0000000000000","sourceAddress":{"kind":"step","rootEntityLabel":5850,"occurrencePath":[],"faceEntityLabel":3042}
    })).unwrap();
    ElementEntry::new(body, ElementKind::Face)
        .with_evidence(
            Some(AnchorIntent {
                world_point: Vec3::new_unchecked(1.0, 2.0, 3.0),
                surface_uv: None,
                local_frame: None,
                adjacency_hint: None,
                extra: Default::default(),
            }),
            Some(serde_json::json!({"opaque": [1,2,3]})),
        )
        .with_import_source_identity(Some(cert))
}

#[tokio::test]
async fn accepted_relabel_preserves_existing_identity_evidence() {
    let body = default_body_of(rid(0x10));
    let id = elem("el_evidence");
    let previous = evidenced_entry(body);
    let (session, result) = run_delta(false, body, ElementKind::Face, true, false).await;
    assert!(matches!(result, Outcome::Published(_)), "{result:?}");
    assert_eq!(session.elements.get(&id), Some(&previous));
}

async fn run_delta(
    added: bool,
    delta_body: onecad_core::ids::BodyId,
    kind: ElementKind,
    gate_matches: bool,
    crash: bool,
) -> (RegenSession, Outcome) {
    let timeline = timeline_of(if crash { 2 } else { 1 });
    let body = default_body_of(rid(0x10));
    let id = elem("el_evidence");
    let delta = ElementMapEntry {
        element_id: id.clone(),
        topo_key: TopoKey::new("f:0"),
        kind,
        body: delta_body,
    };
    let deltas = if added {
        ElementMapDelta {
            added: vec![delta],
            ..Default::default()
        }
    } else {
        ElementMapDelta {
            relabeled: vec![delta],
            ..Default::default()
        }
    };
    let mut scripts = BTreeMap::new();
    scripts.insert(
        0,
        StepScript::Ok {
            body_events: vec![created(body)],
            deltas,
            signatures: sigs(0),
        },
    );
    if crash {
        scripts.insert(1, StepScript::Crash);
    }
    let executor = RegenExecutor::new(FakeEngine::new(scripts));
    let mut session = RegenSession::with_timeline(timeline.clone());
    session.elements.insert(id, evidenced_entry(body));
    let revision = DocumentRevision(7);
    let epoch = WorkerEpoch(3);
    let plan = plan_request(
        &timeline,
        RegenRequest::ToEnd { from: 0 },
        JobId(Uuid::from_u128(0x105)),
        revision,
        epoch,
    );
    let gate = move || {
        (
            if gate_matches {
                revision
            } else {
                DocumentRevision(8)
            },
            epoch,
        )
    };
    let outcome = executor
        .run(
            plan,
            &mut session,
            &gate,
            &CancelToken::new(),
            &SnapshotPublisher::new(),
        )
        .await;
    (session, outcome)
}

#[tokio::test]
async fn accepted_added_collision_preserves_existing_identity_evidence() {
    let body = default_body_of(rid(0x10));
    let (session, outcome) = run_delta(true, body, ElementKind::Face, true, false).await;
    assert!(matches!(outcome, Outcome::Published(_)), "{outcome:?}");
    assert_eq!(
        session.elements.get(&elem("el_evidence")),
        Some(&evidenced_entry(body))
    );
}

#[tokio::test]
async fn changed_partition_or_kind_does_not_transfer_evidence() {
    let body = default_body_of(rid(0x10));
    let other = onecad_core::ids::BodyId(Uuid::from_u128(0x99));
    for (changed_body, kind) in [(other, ElementKind::Face), (body, ElementKind::Edge)] {
        let (session, outcome) = run_delta(false, changed_body, kind, true, false).await;
        assert!(matches!(outcome, Outcome::Published(_)), "{outcome:?}");
        assert_eq!(
            session.elements.get(&elem("el_evidence")),
            Some(&ElementEntry::new(changed_body, kind))
        );
    }
}

#[tokio::test]
async fn failed_or_superseded_buffered_delta_keeps_prior_evidence() {
    let body = default_body_of(rid(0x10));
    for (gate_matches, crash) in [(true, true), (false, false)] {
        let (session, outcome) =
            run_delta(false, body, ElementKind::Face, gate_matches, crash).await;
        if crash {
            assert!(matches!(outcome, Outcome::EngineFailed(_)), "{outcome:?}");
        } else {
            assert!(matches!(outcome, Outcome::Superseded), "{outcome:?}");
        }
        assert_eq!(
            session.elements.get(&elem("el_evidence")),
            Some(&evidenced_entry(body))
        );
    }
}
