//! preview == commit, non-mutation, classification and fencing.

use onecad_app::worker::ElementQuery;
use onecad_core::document::record::FilletParams;
use onecad_core::document::refs::{AnchorIntent, ElementKind, ElementRef, PrimaryRef};
use onecad_core::ids::{ElementId, TopoKey};

use super::super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preview_equals_commit_and_changes_nothing() {
    let Some((_wm, state)) = document(part_with_dependent()).await else {
        return;
    };
    let before = head_state(&state).await;
    let edited = extrude_op(SKETCH_A, "", 5.0, None);
    let preview = preview(&state, EXTRUDE_A, edited.clone())
        .await
        .expect("preview");

    assert_eq!(preview.outcome, EditPreviewOutcome::Ready);
    assert!(!preview.isolated);
    let meshed: Vec<&str> = preview.meshes.iter().map(|m| m.body_id.as_str()).collect();
    assert_eq!(
        meshed,
        [body(EXTRUDE_A).to_string()],
        "only the edited part is re-meshed"
    );
    assert_eq!(
        preview.suppressed,
        vec![body(EXTRUDE_A)],
        "its old version is hidden"
    );
    assert_eq!(
        head_state(&state).await,
        before,
        "a preview publishes nothing"
    );

    let snap = commit(&state, EXTRUDE_A, edited).await;
    assert_preview_equals_commit(&state, &preview, &snap).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_downstream_halt_previews_the_isolated_commit() {
    let mut records = part_with_dependent();
    // A broken feature on sketch A between the parts: the halt is downstream of
    // the edit, and part C above it is provably independent (WP-D1).
    records.insert(
        4,
        extrude_record(BROKEN, extrude_op(SKETCH_A, "no-such-region", 3.0, None)),
    );
    let Some((_wm, state)) = document(records).await else {
        return;
    };
    let edited = extrude_op(SKETCH_A, "", 5.0, None);
    let preview = preview(&state, EXTRUDE_A, edited.clone())
        .await
        .expect("preview");

    assert!(
        matches!(preview.outcome, EditPreviewOutcome::DownstreamFailed { step: 4, record, .. } if record == rec(BROKEN)),
        "{:?}",
        preview.outcome
    );
    assert!(
        preview.isolated,
        "the isolation pass supplies the displayed bodies"
    );
    assert!(
        preview.bodies.contains(&body(EXTRUDE_C)),
        "{:?}",
        preview.bodies
    );

    let snap = commit(&state, EXTRUDE_A, edited).await;
    assert_preview_equals_commit(&state, &preview, &snap).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_edited_feature_failure_is_reported_at_its_own_step() {
    let Some((_wm, state)) = document(part_with_dependent()).await else {
        return;
    };
    let before = head_state(&state).await;
    let broken = extrude_op(SKETCH_A, "no-such-region", 5.0, None);
    let preview = preview(&state, EXTRUDE_A, broken).await.expect("preview");
    assert!(
        matches!(
            preview.outcome,
            EditPreviewOutcome::EditedFailed {
                needs_repair: false,
                ..
            }
        ),
        "{:?}",
        preview.outcome
    );
    assert_eq!(head_state(&state).await, before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stale_fence_is_refused() {
    let Some((_wm, state)) = document(part_with_dependent()).await else {
        return;
    };
    let fence = api::preview_context(&state).await.unwrap();
    commit(&state, EXTRUDE_C, extrude_op(SKETCH_C, "", 9.0, None)).await;
    let stale = api::preview_existing_edit(
        &state,
        fence,
        rec(EXTRUDE_A),
        extrude_op(SKETCH_A, "", 5.0, None),
        Lod::Coarse,
    )
    .await;
    assert!(matches!(stale, Err(ApiError::StalePreview(_))), "{stale:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn save_and_reopen_see_the_unedited_document() {
    let Some((wm, state)) = document(part_with_dependent()).await else {
        return;
    };
    let signatures_before = {
        let guard = state.runtime.lock().await;
        guard
            .as_ref()
            .unwrap()
            .subscribe_snapshots()
            .borrow()
            .clone()
            .unwrap()
            .signatures
            .clone()
    };
    preview(&state, EXTRUDE_A, extrude_op(SKETCH_A, "", 5.0, None))
        .await
        .expect("preview");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("previewed.onecad");
    {
        let mut guard = state.runtime.lock().await;
        let meta = SaveMeta {
            app_version: "0.1.0-test".into(),
            occt_fingerprint: None,
            created: "2026-10-21T00:00:00Z".into(),
            modified: "2026-10-21T00:00:00Z".into(),
        };
        guard.as_mut().unwrap().save(&path, meta).expect("save");
    }
    let engine: Arc<dyn GeometryEngine> = Arc::new(wm.clone());
    let meshes: Arc<dyn MeshProvider> = Arc::new(wm.clone());
    let solver: Arc<dyn SolverEngine> = Arc::new(wm.clone());
    let mut reopened = DocumentRuntime::open(&path, engine, meshes, solver).expect("reopen");
    let report = reopened
        .run_regen(RegenRequest::RevertToEnd { from: 0 }, CancelToken::new())
        .await;
    let Outcome::Published(snap) = report.outcome else {
        panic!("reopen regen must publish: {:?}", report.outcome);
    };
    assert_eq!(
        snap.signatures, signatures_before,
        "the saved document is the unedited one"
    );
}

/// The other WP-D1 case: a pre-existing broken feature BEFORE the edited one,
/// which is independent of it. The halted pass stops before the edit, the
/// isolation pass routes around the break and runs it — and Apply publishes the
/// edit there, so the preview must show it instead of reporting `NotReached`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_earlier_independent_halt_previews_the_isolated_edit() {
    let records = vec![
        sketch_record(SKETCH_A, 0.0, 0.0, 20.0, 20.0),
        extrude_record(EXTRUDE_A, extrude_op(SKETCH_A, "", 10.0, None)),
        sketch_record(SKETCH_C, 60.0, 0.0, 10.0, 10.0),
        extrude_record(BROKEN, extrude_op(SKETCH_A, "no-such-region", 3.0, None)),
        extrude_record(EXTRUDE_C, extrude_op(SKETCH_C, "", 8.0, None)),
    ];
    let Some((_wm, state)) = document(records).await else {
        return;
    };
    let edited = extrude_op(SKETCH_C, "", 9.0, None);
    let preview = preview(&state, EXTRUDE_C, edited.clone())
        .await
        .expect("preview");
    assert!(
        matches!(preview.outcome, EditPreviewOutcome::DownstreamFailed { step: 3, record, .. } if record == rec(BROKEN)),
        "{:?}",
        preview.outcome
    );
    assert!(preview.isolated, "the isolation pass ran the edited step");
    assert!(
        preview
            .meshes
            .iter()
            .any(|m| m.body_id == body(EXTRUDE_C).to_string()),
        "the edited part is shown"
    );

    let snap = commit(&state, EXTRUDE_C, edited).await;
    assert_preview_equals_commit(&state, &preview, &snap).await;
}

const FILLET: u128 = 0x5E1;

fn fillet_op(edge: &ElementId, anchor: Vec3, radius: f64) -> Operation {
    let edge_ref = ElementRef {
        primary: Some(PrimaryRef {
            body: body(EXTRUDE_A),
            element: edge.clone(),
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
    Operation::Known(KnownOperation::Fillet(FilletParams {
        radius: Scalar::new(radius),
        edge_ids: vec![edge.clone()],
        edges: vec![edge_ref],
        chain_tangent_edges: false,
        tangent_closure_version: None,
        extra: Default::default(),
    }))
}

/// 0 SkA · 1 ExA · 2 Fillet(A, r=2) · 3 SkB · 4 boss B onto A (dependent). The
/// fillet consumes the edge it names, so a replay must re-resolve the ref exactly
/// as the commit does (the H5-B class).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_fillet_edit_with_a_dependent_previews_the_commit() {
    let Some((wm, state)) = document(part_with_dependent()[..2].to_vec()).await else {
        return;
    };
    let (edge, anchor) = {
        let mut guard = state.runtime.lock().await;
        let rt = guard.as_mut().unwrap();
        let snap = rt.head_snapshot_id().expect("head");
        let info = ElementQuery::query_element_by_topo_key(&wm, snap, body(EXTRUDE_A), "e:1")
            .await
            .expect("QueryElement")
            .expect("edge e:1 resolves");
        let anchor = Vec3::new_unchecked(info.center[0], info.center[1], info.center[2]);
        let intent = AnchorIntent {
            world_point: anchor,
            surface_uv: None,
            local_frame: None,
            adjacency_hint: None,
            extra: Default::default(),
        };
        let promoted = rt
            .promote_selection(
                snap,
                body(EXTRUDE_A),
                vec![(TopoKey::new("e:1"), Some(intent))],
            )
            .await
            .expect("promote edge");
        let edge = ElementId::new(promoted[0].element_id.clone());
        let tail = [
            OperationRecord::new(rec(FILLET), 0, "Fillet", fillet_op(&edge, anchor, 2.0)),
            sketch_record(SKETCH_B, 5.0, 5.0, 10.0, 10.0),
            extrude_record(BOSS_B, extrude_op(SKETCH_B, "", 25.0, Some(EXTRUDE_A))),
        ];
        for record in tail {
            rt.apply(EditCommand::AddOperation {
                record,
                at_cursor: true,
            })
            .expect("AddOperation");
        }
        let report = rt
            .run_regen(RegenRequest::ToEnd { from: 2 }, CancelToken::new())
            .await;
        assert!(
            matches!(report.outcome, Outcome::Published(_)),
            "{:?}",
            report.outcome
        );
        (edge, anchor)
    };
    let before = head_state(&state).await;
    let edited = fillet_op(&edge, anchor, 3.0);
    let preview = preview(&state, FILLET, edited.clone())
        .await
        .expect("preview");
    assert_eq!(preview.outcome, EditPreviewOutcome::Ready);
    assert_eq!(
        head_state(&state).await,
        before,
        "a preview publishes nothing"
    );

    let snap = commit(&state, FILLET, edited).await;
    assert_preview_equals_commit(&state, &preview, &snap).await;
}
