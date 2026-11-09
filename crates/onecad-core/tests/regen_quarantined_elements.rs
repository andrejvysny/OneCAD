//! Worker echoes and checkpoint restores cannot revive a reserved identity.
mod support;
use onecad_core::{
    document::{
        element_index::{ElementBindingState, ElementEntry},
        refs::ElementKind,
    },
    ids::{DocumentRevision, JobId, TopoKey, WorkerEpoch},
    regen::{
        CancelToken, CheckpointId, CheckpointRef, ElementMapDelta, ElementMapEntry, EngineError,
        Outcome, RegenExecutor, RegenRequest, RegenSession, SnapshotPublisher,
    },
};
use std::collections::BTreeMap;
use support::*;
use uuid::Uuid;

async fn run(delta: ElementMapDelta, checkpoint: bool) -> (RegenSession, Outcome, CallLog) {
    let timeline = timeline_of(1);
    let body = default_body_of(rid(0x10));
    let entry = ElementEntry {
        binding_state: Some(ElementBindingState::Quarantined),
        ..ElementEntry::new(body, ElementKind::Face)
            .with_evidence(None, Some(serde_json::json!({"original": [1, 2, 3]})))
    };
    let mut session = RegenSession::with_timeline(timeline.clone());
    session.elements.insert(elem("el_reserved"), entry);
    let scripts = BTreeMap::from([(
        0,
        StepScript::Ok {
            body_events: vec![created(body)],
            deltas: delta,
            signatures: sigs(0),
        },
    )]);
    let executor = RegenExecutor::new(FakeEngine::new(scripts));
    let revision = DocumentRevision(7);
    let epoch = WorkerEpoch(3);
    let mut request = plan_request(
        &timeline,
        RegenRequest::ToEnd { from: 0 },
        JobId(Uuid::from_u128(0x105)),
        revision,
        epoch,
    );
    if checkpoint {
        request.base_checkpoint = Some(CheckpointRef {
            step_index: 0,
            checkpoint_id: CheckpointId("stale".into()),
        });
    }
    let publisher = SnapshotPublisher::new();
    let outcome = executor
        .run(
            request,
            &mut session,
            &move || (revision, epoch),
            &CancelToken::new(),
            &publisher,
        )
        .await;
    if matches!(outcome, Outcome::EngineFailed(_)) {
        assert!(publisher.latest().is_none());
        assert!(session.bodies.is_empty());
    }
    let log = executor.engine().log();
    (session, outcome, log)
}

fn assert_retained(session: &RegenSession) {
    let entry = session.elements.get(&elem("el_reserved")).unwrap();
    assert!(entry.is_quarantined());
    assert!(!entry.has_rebind_evidence());
    assert_eq!(
        entry.descriptor,
        Some(serde_json::json!({"original": [1, 2, 3]}))
    );
}

#[tokio::test]
async fn removed_worker_row_keeps_orphan_identity_and_frozen_evidence() {
    let (session, outcome, log) = run(
        ElementMapDelta {
            removed: vec![elem("el_reserved")],
            ..Default::default()
        },
        false,
    )
    .await;
    assert!(matches!(outcome, Outcome::Published(_)), "{outcome:?}");
    assert_eq!(log.accepts.len(), 1);
    assert_retained(&session);
}

#[tokio::test]
async fn added_or_relabeled_worker_identity_refuses_before_acceptance() {
    for added in [false, true] {
        let row = ElementMapEntry {
            element_id: elem("el_reserved"),
            topo_key: TopoKey::new("f:0"),
            body: default_body_of(rid(0x10)),
            kind: ElementKind::Face,
        };
        let delta = if added {
            ElementMapDelta {
                added: vec![row],
                ..Default::default()
            }
        } else {
            ElementMapDelta {
                relabeled: vec![row],
                ..Default::default()
            }
        };
        let (session, outcome, log) = run(delta, false).await;
        assert!(
            matches!(outcome, Outcome::EngineFailed(EngineError::Protocol { .. })),
            "{outcome:?}"
        );
        assert!(log.accepts.is_empty());
        assert_eq!(log.discards.len(), 1);
        assert_retained(&session);
    }
}

#[tokio::test]
async fn quarantine_refuses_checkpoint_before_worker_execution() {
    let (session, outcome, log) = run(ElementMapDelta::default(), true).await;
    assert!(
        matches!(outcome, Outcome::EngineFailed(EngineError::Protocol { .. })),
        "{outcome:?}"
    );
    assert!(log.plans.is_empty());
    assert!(log.accepts.is_empty());
    assert_retained(&session);
}
