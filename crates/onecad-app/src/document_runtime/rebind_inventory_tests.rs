//! Rebinding must validate the whole response before installing any identity.
use super::*;
use onecad_core::{document::element_index::ElementEntry, regen::ResolveOutcome};

fn pending() -> Vec<(ElementId, ElementEntry)> {
    ["el_first", "el_second"]
        .into_iter()
        .map(|id| {
            (
                ElementId::new(id),
                ElementEntry::new(BodyId(Uuid::from_u128(1)), ElementKind::Face),
            )
        })
        .collect()
}

fn resolved(id: &str, snapshot: u64) -> RefResolution {
    RefResolution {
        ref_id: id.into(),
        outcome: ResolveOutcome::AutoBind {
            element_id: ElementId::new(""),
            score: 1.0,
            margin: 1.0,
            topo_key: Some(TopoKey::new(if id == "el_second" { "f:2" } else { "f:1" })),
        },
        snapshot_id: SnapshotId(snapshot),
        revision: 0,
        body_id: Some(BodyId(Uuid::from_u128(1))),
    }
}

#[tokio::test]
async fn malformed_rebind_inventory_never_installs_a_partial_binding() {
    let first = resolved("el_first", 5);
    let second = resolved("el_second", 5);
    let mut wrong_body = second.clone();
    wrong_body.body_id = Some(BodyId(Uuid::from_u128(2)));
    let mut missing_body = second.clone();
    missing_body.body_id = None;
    let mut wrong_revision = second.clone();
    wrong_revision.revision = 1;
    let cases = [
        vec![],
        vec![first.clone()],
        vec![first.clone(), first.clone()],
        vec![first.clone(), resolved("el_foreign", 5)],
        vec![first.clone(), second.clone(), resolved("el_foreign", 5)],
        vec![first.clone(), resolved("el_second", 4)],
        vec![first.clone(), wrong_body],
        vec![first.clone(), missing_body],
        vec![first.clone(), wrong_revision],
    ];
    for responses in cases {
        let backend = Arc::new(FakeBackend::new());
        *backend.resolve_responses.lock().unwrap() = Some(responses.clone());
        let engine: Arc<dyn GeometryEngine> = backend.clone();
        let result = DocumentRuntime::rebind_entries_with(&engine, SnapshotId(5), &pending()).await;
        assert!(
            result.is_err(),
            "malformed response accepted: {responses:?}"
        );
        assert_eq!(backend.bind_calls.load(Ordering::Relaxed), 0);
    }
}

#[tokio::test]
async fn complete_reordered_rebind_inventory_installs_one_batch() {
    let backend = Arc::new(FakeBackend::new());
    *backend.resolve_responses.lock().unwrap() =
        Some(vec![resolved("el_second", 5), resolved("el_first", 5)]);
    let engine: Arc<dyn GeometryEngine> = backend.clone();
    let result = DocumentRuntime::rebind_entries_with(&engine, SnapshotId(5), &pending()).await;
    assert_eq!(result, Ok((2, 0)));
    assert_eq!(backend.bind_calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn missing_body_no_candidates_is_unresolved_without_blocking_valid_binding() {
    let backend = Arc::new(FakeBackend::new());
    let mut refusal = resolved("el_second", 5);
    refusal.body_id = None;
    refusal.outcome = ResolveOutcome::NeedsRepair(
        serde_json::from_value(serde_json::json!({
            "stepIndex": 0, "refId": "el_second", "ladderFailed": "descriptor",
            "reason": "no-candidates", "candidates": []
        }))
        .unwrap(),
    );
    *backend.resolve_responses.lock().unwrap() = Some(vec![resolved("el_first", 5), refusal]);
    let engine: Arc<dyn GeometryEngine> = backend.clone();
    let result = DocumentRuntime::rebind_entries_with(&engine, SnapshotId(5), &pending()).await;
    assert_eq!(result, Ok((1, 1)));
    assert_eq!(backend.bind_calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn duplicate_pending_identity_is_refused_without_binding() {
    let backend = Arc::new(FakeBackend::new());
    *backend.resolve_responses.lock().unwrap() =
        Some(vec![resolved("el_first", 5), resolved("el_first", 5)]);
    let engine: Arc<dyn GeometryEngine> = backend.clone();
    let first = pending().remove(0);
    let result =
        DocumentRuntime::rebind_entries_with(&engine, SnapshotId(5), &[first.clone(), first]).await;
    assert_eq!(result, Err(()));
    assert_eq!(backend.bind_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn repair_item_cannot_name_another_identity_in_a_valid_batch() {
    let backend = Arc::new(FakeBackend::new());
    let mut refusal = resolved("el_second", 5);
    refusal.outcome = ResolveOutcome::NeedsRepair(
        serde_json::from_value(serde_json::json!({
            "stepIndex": 0, "refId": "el_second", "elementId": "el_foreign",
            "ladderFailed": "descriptor", "reason": "no-candidates", "candidates": []
        }))
        .unwrap(),
    );
    *backend.resolve_responses.lock().unwrap() = Some(vec![resolved("el_first", 5), refusal]);
    let engine: Arc<dyn GeometryEngine> = backend.clone();
    assert!(
        DocumentRuntime::rebind_entries_with(&engine, SnapshotId(5), &pending())
            .await
            .is_err()
    );
    assert_eq!(backend.bind_calls.load(Ordering::Relaxed), 0);
}
