//! Regressions for the 8103a3af kernel audit and its independent second review.
//! Each test names the finding it closes (audit number / second-review id).

use super::*;
use onecad_core::regen::ResolveOutcome;

/// Drives one request through the production driver behind a real scheduler and
/// returns every completion the driver emitted as `(outcome label, body count)`.
async fn drive_through_scheduler(
    runtime: Arc<tokio::sync::Mutex<Option<DocumentRuntime>>>,
    on_started: crate::RegenStartEmitter,
    request: RegenRequest,
) -> Vec<(&'static str, usize)> {
    let emitted = Arc::new(Mutex::new(Vec::new()));
    let sink = emitted.clone();
    let driver = crate::regen_driver_with_started(
        runtime,
        Arc::new(move |report: &RegenReport, projection| {
            let label = match &report.outcome {
                Outcome::Published(_) => "published",
                Outcome::Superseded => "superseded",
                Outcome::EngineFailed(_) => "failed",
                Outcome::Cancelled => "cancelled",
                Outcome::NoOp => "noop",
            };
            sink.lock().unwrap().push((label, projection.bodies.len()));
        }),
        on_started,
        Arc::new(|_| Box::pin(async {})),
        Arc::new(tokio::sync::watch::channel(0u64).0),
    );
    let (scheduler, handle) = RegenScheduler::new(driver);
    let mut status = handle.subscribe_status();
    handle.request(request);
    let task = tokio::spawn(scheduler.run());
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            status.changed().await.expect("scheduler status stays open");
            if !matches!(
                *status.borrow(),
                SchedulerStatus::Running { .. } | SchedulerStatus::Debouncing { .. }
            ) {
                break;
            }
        }
    })
    .await
    .expect("the regen lane settles");
    task.abort();
    let out = emitted.lock().unwrap().clone();
    out
}

/// Applies `edit` exactly once, from the driver's start sink: the one point where
/// the plan is prepared and the runtime lock is released for the worker phase.
fn edit_once_during_drive(
    runtime: &Arc<tokio::sync::Mutex<Option<DocumentRuntime>>>,
    edit: EditCommand,
) -> crate::RegenStartEmitter {
    let runtime = runtime.clone();
    let edit = Mutex::new(Some(edit));
    Arc::new(move |_| {
        let Some(edit) = edit.lock().unwrap().take() else {
            return;
        };
        let mut guard = runtime
            .try_lock()
            .expect("the driver releases the runtime lock before driving");
        let outcome = guard.as_mut().unwrap().apply(edit).unwrap();
        assert!(
            matches!(outcome.regen, RegenHint::None),
            "the interleaved edit must be one that queues no regen of its own"
        );
    })
}

/// Audit 2 / SR-02: a rename landing while a regen is out supersedes it and
/// queues nothing. The owed geometry must still be published.
#[tokio::test]
async fn metadata_edit_during_regen_still_publishes_the_owed_geometry() {
    let (mut rt, body, _) = promotable_fake_runtime().await;
    rt.apply(add_extrude(0x11, 10.0)).unwrap();
    let runtime = Arc::new(tokio::sync::Mutex::new(Some(rt)));
    let rename = EditCommand::RenameBody {
        body,
        name: "renamed".into(),
    };
    let emitted = drive_through_scheduler(
        runtime.clone(),
        edit_once_during_drive(&runtime, rename),
        RegenRequest::ToEnd { from: 1 },
    )
    .await;
    assert_eq!(emitted, vec![("superseded", 1), ("published", 2)]);
    let guard = runtime.lock().await;
    let projection = guard.as_ref().unwrap().projection();
    assert_eq!(projection.bodies.len(), 2, "both extrudes are published");
    assert_eq!(projection.bodies[&body.to_string()].name, "renamed");
}

/// Audit 2: a hard failure inside that window used to be relabelled `superseded`
/// and dropped. The re-prepared request must report it.
#[tokio::test]
async fn hard_failure_superseded_by_metadata_edit_is_still_reported() {
    let backend = Arc::new(FakeBackend {
        hard_fail_from_plan: Some(2),
        ..FakeBackend::default()
    });
    let mut rt = runtime_with(backend);
    rt.apply(add_extrude(0x10, 25.0)).unwrap();
    let first = rt
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    assert!(matches!(first.outcome, Outcome::Published(_)));
    rt.apply(add_extrude(0x11, 10.0)).unwrap();
    let body = BodyId(Uuid::from_u128(0x10));
    let runtime = Arc::new(tokio::sync::Mutex::new(Some(rt)));
    let rename = EditCommand::RenameBody {
        body,
        name: "renamed".into(),
    };
    let emitted = drive_through_scheduler(
        runtime.clone(),
        edit_once_during_drive(&runtime, rename),
        RegenRequest::ToEnd { from: 1 },
    )
    .await;
    assert_eq!(emitted, vec![("superseded", 1), ("failed", 1)]);
}

fn face_pick() -> Vec<(TopoKey, Option<AnchorIntent>)> {
    vec![(TopoKey::new("f:1"), None)]
}

/// Audit 8 / SR-04: a persistent id promoted while a regen is out lives only in
/// the live index. Adopting the older scratch must not erase it.
#[tokio::test]
async fn promotion_during_inflight_regen_survives_its_commit() {
    let (mut rt, body, snapshot) = promotable_fake_runtime().await;
    rt.apply(add_extrude(0x11, 10.0)).unwrap();
    let prepared = rt
        .begin_regen(RegenRequest::ToEnd { from: 1 })
        .expect("plan");
    let promoted = rt
        .promote_selection(snapshot, body, face_pick())
        .await
        .unwrap();
    assert_eq!(rt.persisted_element_count(), 1);
    let report = rt.finish_regen(prepared.drive(CancelToken::new()).await);
    assert!(matches!(report.outcome, Outcome::Published(_)));
    assert_eq!(
        rt.persisted_element_count(),
        1,
        "the promotion outlives the regen that was prepared before it"
    );
    let id = ElementId::new(promoted[0].element_id.clone());
    let entry = rt.regen.elements.get(&id).expect("same persistent id");
    assert!(entry.has_rebind_evidence(), "durable evidence is kept");
}

/// The carry is for ids the job never saw. An id the job was seeded with and
/// then retired must stay retired.
#[tokio::test]
async fn element_retired_by_the_regen_is_not_resurrected() {
    let (mut rt, body, snapshot) = promotable_fake_runtime().await;
    let promoted = rt
        .promote_selection(snapshot, body, face_pick())
        .await
        .unwrap();
    let id = ElementId::new(promoted[0].element_id.clone());
    let mut scratch = rt.clone_regen_session();
    let base: BTreeSet<ElementId> = scratch.elements.iter().map(|(id, _)| id.clone()).collect();
    scratch.elements.remove(&id);
    rt.carry_inflight_promotions(&mut scratch, &base);
    assert!(!scratch.elements.contains(&id));
}

/// Second review SR-05 (audit's unnumbered clear-to-zero claim): the empty
/// publish drops live partition rows but keeps durable evidence dormant. The
/// fake backend cannot reset its session, so this pins the clear fold itself;
/// `tests/reference_lifecycle.rs` runs the whole path on the real worker.
#[tokio::test]
async fn empty_publish_keeps_durable_element_evidence() {
    let (mut rt, body, snapshot) = promotable_fake_runtime().await;
    let promoted = rt
        .promote_selection(snapshot, body, face_pick())
        .await
        .unwrap();
    let durable = ElementId::new(promoted[0].element_id.clone());
    let partition_row = ElementId::new("el_partition_row");
    rt.regen.elements.insert(
        partition_row.clone(),
        ElementEntry::new(body, ElementKind::Face),
    );
    let mut scratch = rt.clone_regen_session();
    let outcome = drive_clear(&mut scratch, &rt.publisher, &CancelToken::new(), 1);
    let Outcome::Published(snapshot) = outcome else {
        panic!("clear publishes");
    };
    assert!(snapshot.bodies.is_empty(), "nothing is shown");
    assert!(scratch.bodies.bodies().is_empty());
    assert!(
        scratch
            .elements
            .get(&durable)
            .is_some_and(ElementEntry::has_rebind_evidence),
        "evidence stays dormant"
    );
    assert!(
        !scratch.elements.contains(&partition_row),
        "a row with no evidence describes only the geometry that is gone"
    );
}

/// Second review NEW-02: one completed rebind used to stand for the session.
/// Every publish replaces the worker partition, so every publish re-arms it.
#[tokio::test]
async fn completed_rebind_is_rearmed_by_a_later_full_replay() {
    let backend = Arc::new(FakeBackend::new());
    let mut rt = runtime_with(backend.clone());
    rt.apply(add_extrude(0x10, 25.0)).unwrap();
    let report = rt
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    let body = BodyId(Uuid::from_u128(0x10));
    let promoted = rt
        .promote_selection(SnapshotId(report.snapshot_id), body, face_pick())
        .await
        .unwrap();
    *backend.resolve_outcome.lock().unwrap() = Some(ResolveOutcome::Unchanged {
        element_id: Some(ElementId::new(promoted[0].element_id.clone())),
    });
    assert_eq!(rt.rebind_persisted_elements().await, (1, 0));
    assert!(
        !rt.has_unbound_persisted_elements(),
        "answered on this head"
    );
    let replay = rt
        .run_regen(RegenRequest::RevertToEnd { from: 0 }, CancelToken::new())
        .await;
    assert!(matches!(replay.outcome, Outcome::Published(_)));
    assert!(
        rt.has_unbound_persisted_elements(),
        "a same-epoch replay discarded the bindings; the pass is owed again"
    );
}

/// Audit 12: snapshot ids restart after a worker restart, so a promotion cached
/// under the old process must not answer an exact-key hit in the new one.
#[tokio::test]
async fn promotion_cache_does_not_survive_a_worker_epoch_change() {
    let (mut rt, body, snapshot) = promotable_fake_runtime().await;
    rt.promote_selection(snapshot, body, face_pick())
        .await
        .unwrap();
    assert_eq!(rt.promoted.len(), 1);
    let same = rt.head_epoch;
    rt.retire_promotions_of_other_epochs(same);
    assert_eq!(rt.promoted.len(), 1, "same epoch keeps the reuse window");
    rt.retire_promotions_of_other_epochs(onecad_core::ids::WorkerEpoch(same.0 + 1));
    assert!(
        rt.promoted.is_empty(),
        "another worker history owns those ids"
    );
    assert_eq!(rt.persisted_element_count(), 1, "durable identity is kept");
}

/// Audit 7: the edit claim used to live on the queued request, so replacing
/// that request (latest-wins) dropped it. The runtime now owns it until a
/// publish covers the edit.
#[tokio::test]
async fn unpublished_edit_claim_survives_request_replacement() {
    let (mut rt, _, _) = promotable_fake_runtime().await;
    rt.apply(add_extrude(0x11, 10.0)).unwrap();
    rt.apply(add_extrude(0x12, 10.0)).unwrap();
    let published = rt
        .run_regen(RegenRequest::ToEnd { from: 1 }, CancelToken::new())
        .await;
    assert!(matches!(published.outcome, Outcome::Published(_)));
    let nothing_owed = rt
        .begin_regen(RegenRequest::RevertToEnd { from: 0 })
        .expect("plan");
    assert_eq!(nothing_owed.edited_from, None, "a replay claims no edit");

    // Edit step 1; its regen never publishes (cancelled / superseded).
    rt.apply(EditCommand::UpdateOperationParams {
        record: RecordId(Uuid::from_u128(0x11)),
        op: extrude_record(0x11, 12.0).op,
    })
    .unwrap();
    // The job that finally runs was queued by something else: a restart replay…
    let replay = rt
        .begin_regen(RegenRequest::RevertToEnd { from: 0 })
        .expect("plan");
    assert_eq!(replay.edited_from, Some(1), "the edit is still unpublished");
    // …or a later edit further down the timeline.
    let later = rt
        .begin_regen(RegenRequest::ToEnd { from: 2 })
        .expect("plan");
    assert_eq!(later.edited_from, Some(1), "the lower floor wins");

    // Undoing that very edit restores what accepted geometry was built from.
    rt.undo().expect("the edit is undone");
    let reverted = rt
        .begin_regen(RegenRequest::RevertToEnd { from: 1 })
        .expect("plan");
    assert_eq!(reverted.edited_from, None, "a pure revert claims nothing");
}

/// Audit 7: a publish that covers the edit settles the claim.
#[tokio::test]
async fn accepted_publish_settles_the_edit_claim() {
    let (mut rt, _, _) = promotable_fake_runtime().await;
    rt.apply(add_extrude(0x11, 10.0)).unwrap();
    let published = rt
        .run_regen(RegenRequest::ToEnd { from: 1 }, CancelToken::new())
        .await;
    assert!(matches!(published.outcome, Outcome::Published(_)));
    assert!(rt.unaccepted_edit.is_none());
}

/// Review of the stranded-regen retry: only a supersede that queues nothing is
/// stranded. A content edit brings its own request, and re-running the old one
/// beside it could settle that edit's awaiter with the wrong terminal.
#[tokio::test]
async fn only_a_supersede_without_a_successor_is_stranded() {
    let (mut rt, body, _) = promotable_fake_runtime().await;
    rt.apply(add_extrude(0x11, 10.0)).unwrap();

    let prepared = rt.begin_regen(RegenRequest::ToEnd { from: 1 }).unwrap();
    rt.apply(EditCommand::RenameBody {
        body,
        name: "renamed".into(),
    })
    .unwrap();
    let report = rt.finish_regen(prepared.drive(CancelToken::new()).await);
    assert!(matches!(report.outcome, Outcome::Superseded));
    assert!(rt.take_stranded_regen(), "a rename queues no regen");
    assert!(!rt.take_stranded_regen(), "the mark is consumed");

    let prepared = rt.begin_regen(RegenRequest::ToEnd { from: 1 }).unwrap();
    rt.apply(add_extrude(0x12, 10.0)).unwrap();
    let report = rt.finish_regen(prepared.drive(CancelToken::new()).await);
    assert!(matches!(report.outcome, Outcome::Superseded));
    assert!(
        !rt.take_stranded_regen(),
        "the new feature's own request owns the work"
    );
}

/// Review: a publish whose plan carried no claim resolved nothing under the
/// veto, so it must not settle an outstanding edit; and a publish that DID
/// carry one must not re-arm the claim-blind rebind lane.
#[tokio::test]
async fn claim_free_publish_keeps_the_claim_and_claimed_publish_keeps_rebind_closed() {
    let backend = Arc::new(FakeBackend::new());
    let mut rt = runtime_with(backend.clone());
    rt.apply(add_extrude(0x10, 25.0)).unwrap();
    let report = rt
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    let body = BodyId(Uuid::from_u128(0x10));
    let promoted = rt
        .promote_selection(SnapshotId(report.snapshot_id), body, face_pick())
        .await
        .unwrap();
    *backend.resolve_outcome.lock().unwrap() = Some(ResolveOutcome::Unchanged {
        element_id: Some(ElementId::new(promoted[0].element_id.clone())),
    });
    assert_eq!(rt.rebind_persisted_elements().await, (1, 0));

    rt.apply(EditCommand::UpdateOperationParams {
        record: RecordId(Uuid::from_u128(0x10)),
        op: extrude_record(0x10, 30.0).op,
    })
    .unwrap();
    let preview = rt
        .run_regen(RegenRequest::ToStep(0), CancelToken::new())
        .await;
    assert!(matches!(preview.outcome, Outcome::Published(_)));
    assert!(rt.unaccepted_edit.is_some(), "a preview claims nothing");

    let commit = rt
        .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
        .await;
    assert!(matches!(commit.outcome, Outcome::Published(_)));
    assert!(
        rt.unaccepted_edit.is_none(),
        "the claiming publish settles it"
    );
    assert!(
        !rt.has_unbound_persisted_elements(),
        "ResolveRefs has no edit context; an edit publish must not re-arm it"
    );
    let replay = rt
        .run_regen(RegenRequest::RevertToEnd { from: 0 }, CancelToken::new())
        .await;
    assert!(matches!(replay.outcome, Outcome::Published(_)));
    assert!(
        rt.has_unbound_persisted_elements(),
        "a claim-free replay does"
    );
}
