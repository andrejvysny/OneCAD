//! ONECAD-296 S296-a: lane-safe projection services inside an open sketch visit,
//! against the real C++ OCCT worker.
//!
//! What each test pins:
//!
//! * a project / update / detach committed in a visit is ONE sketch edit that
//!   the visit's own history moves, that Finish squashes, and that resyncs the
//!   worker solver lane (`SketchUpsert`) — the receipt carries that solve;
//! * provenance survives: update keeps entity ids, detach keeps geometry and
//!   drops provenance, history restores either;
//! * an unsupported curve is refused per source, never approximated, and
//!   commits nothing;
//! * an open drag, a held upsert lane, a superseded pick, a closed visit and a
//!   concurrent document edit all refuse loudly and commit nothing.
#[path = "projection_visit/support.rs"]
mod support;

use onecad_app::api;
use onecad_core::edit::{EditCommand, HistoryDirection, SketchEditOp};
use onecad_core::ids::EntityId;
use onecad_core::math::Vec2;
use onecad_core::sketch::{Sketch, SketchEntity};
use support::{fixed_pins, fixture, max_u, point_uvs, Fixture};

fn locked(sketch: &Sketch) -> usize {
    sketch
        .entities()
        .iter()
        .filter(|e| e.is_reference_locked())
        .count()
}

async fn project(
    fx: &Fixture,
) -> api::CommandCompletion<
    onecad_app::dto::SketchProjectionReceipt<onecad_app::dto::ProjectToSketchDto>,
> {
    api::project_to_sketch_with_completion(
        &fx.state,
        fx.host.clone(),
        fx.snapshot.0,
        fx.proj.to_string(),
        "faceOutline".into(),
        fx.source(),
    )
    .await
    .expect("projectToSketch in the visit")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn project_in_visit_resyncs_the_solver_and_follows_visit_and_document_history() {
    let Some(fx) = fixture(false).await else {
        return;
    };
    fx.enter().await;
    let entered = fx.snapshot_sketch().await;
    let completion = project(&fx).await;
    let receipt = completion.result.expect("receipt");
    assert!(receipt.value.refusals.is_empty(), "{:?}", receipt.value);
    assert_eq!(receipt.value.entities.len(), 4, "a rectangle outline");
    assert_eq!(receipt.value.point_count, 4);
    let solve = receipt
        .solve
        .expect("a committed edit resyncs SketchUpsert");
    assert_eq!(solve.sketch_id, fx.proj.to_string());
    assert_eq!(solve.dof, 0, "every projected point is pinned");
    let projected = fx.snapshot_sketch().await;
    assert_eq!(
        projected.projections.len(),
        4,
        "one provenance row per curve"
    );
    assert_eq!(locked(&projected), 8, "four lines and four points locked");
    assert_eq!(fixed_pins(&projected), 4);
    {
        let guard = fx.state.runtime.lock().await;
        let rt = guard.as_ref().unwrap();
        let live = rt.get_sketch(fx.proj).unwrap();
        assert!(live.edit_session.is_some(), "the visit stays open");
        assert!(live.solve_current, "the cached solve matches the document");
        assert_eq!(live.history.unwrap().undo_depth, 1, "one visit step");
    }

    assert!(fx.history(HistoryDirection::Undo).await, "visit undo");
    assert_eq!(fx.snapshot_sketch().await, entered, "undo restores entry");
    assert!(fx.history(HistoryDirection::Redo).await, "visit redo");
    assert_eq!(
        fx.snapshot_sketch().await,
        projected,
        "redo restores the same ids and provenance"
    );

    assert_eq!(fx.finish().await, 1, "the outline closes one region");
    let mut guard = fx.state.runtime.lock().await;
    let rt = guard.as_mut().unwrap();
    // The first Finish mints the sketch's timeline record as its own step; the
    // squashed visit step sits beneath it.
    let depth = rt.undo_depth();
    rt.undo().expect("undo the record mint");
    assert_eq!(fx.sketch(rt), projected, "the record mint owns no geometry");
    rt.undo().expect("document undo of the squashed visit");
    assert_eq!(rt.undo_depth(), depth - 2, "the whole visit is ONE step");
    assert_eq!(fx.sketch(rt), entered, "undo drops the projection");
    rt.redo().unwrap().expect("document redo of the visit");
    rt.redo().unwrap().expect("document redo of the record");
    assert_eq!(fx.sketch(rt), projected, "redo restores ids and provenance");
    drop(guard);
    fx.worker.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn update_in_visit_refreshes_stale_rows_in_place() {
    let Some(fx) = fixture(false).await else {
        return;
    };
    fx.enter().await;
    project(&fx).await.result.expect("projected");
    assert_eq!(fx.finish().await, 1);
    let frozen = fx.snapshot_sketch().await;
    let head = fx.widen().await;
    assert_ne!(head, fx.snapshot, "the upstream edit moved the head");
    let projector = fx.state.face_projection();
    onecad_app::sketch_projection::refresh_projection_staleness(
        &fx.state.runtime,
        &projector,
        &onecad_core::regen::CancelToken::new(),
    )
    .await;
    let stale = {
        let guard = fx.state.runtime.lock().await;
        guard.as_ref().unwrap().projection_stale_entities(fx.proj)
    };
    assert_eq!(stale.len(), 3, "the u = 0 wall did not move");
    assert_eq!(fx.snapshot_sketch().await, frozen, "a projection is frozen");

    fx.enter().await;
    let receipt =
        api::update_projection_with_completion(&fx.state, fx.host.clone(), fx.proj.to_string())
            .await
            .expect("updateProjection in the visit")
            .result
            .expect("receipt");
    assert!(receipt.value.refusals.is_empty(), "{:?}", receipt.value);
    assert_eq!(
        receipt.value.snapshot_id, head.0,
        "answered on the new head"
    );
    let replaced: Vec<EntityId> = receipt
        .value
        .entities
        .iter()
        .map(|e| e.entity_id.parse().unwrap())
        .collect();
    assert_eq!(replaced.len(), 3);
    assert!(replaced.iter().all(|id| stale.contains(id)), "ids kept");
    assert!(receipt.solve.is_some(), "the refresh resyncs SketchUpsert");
    let refreshed = fx.snapshot_sketch().await;
    assert!(
        (max_u(&refreshed) - 44.0).abs() < 1e-6,
        "follows the widened cap"
    );
    assert_eq!(locked(&refreshed), 8, "an update keeps the lock");
    {
        let guard = fx.state.runtime.lock().await;
        let rt = guard.as_ref().unwrap();
        assert!(rt.projection_stale_entities(fx.proj).is_empty(), "cleared");
    }

    assert!(fx.history(HistoryDirection::Undo).await);
    assert_eq!(
        fx.snapshot_sketch().await,
        frozen,
        "undo restores the old rows"
    );
    assert!(fx.history(HistoryDirection::Redo).await);
    assert_eq!(fx.snapshot_sketch().await, refreshed);
    assert_eq!(fx.finish().await, 1);
    fx.worker.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn detach_in_visit_keeps_geometry_and_drops_provenance() {
    let Some(fx) = fixture(false).await else {
        return;
    };
    fx.enter().await;
    project(&fx).await.result.expect("projected");
    let projected = fx.snapshot_sketch().await;
    let receipt = api::detach_projection_with_completion(
        &fx.state,
        fx.host.clone(),
        fx.proj.to_string(),
        None,
    )
    .await
    .expect("detachProjection in the visit")
    .result
    .expect("receipt");
    assert_eq!(receipt.value.entity_ids.len(), 4);
    assert_eq!(receipt.value.released_constraints, 4);
    assert_eq!(receipt.value.remaining, 0);
    assert!(receipt.solve.is_some(), "the detach resyncs SketchUpsert");
    let detached = fx.snapshot_sketch().await;
    assert_eq!(point_uvs(&detached), point_uvs(&projected), "geometry kept");
    assert!(detached.projections.is_empty(), "provenance dropped");
    assert_eq!(locked(&detached), 0);
    assert_eq!(fixed_pins(&detached), 0);

    assert!(fx.history(HistoryDirection::Undo).await);
    assert_eq!(fx.snapshot_sketch().await, projected, "undo re-attaches");
    assert!(fx.history(HistoryDirection::Redo).await);
    assert_eq!(fx.snapshot_sketch().await, detached);
    // Detached geometry is ordinary: the edit layer moves it.
    let (point, _) = point_uvs(&detached)[0];
    api::sketch_upsert_with_completion(
        &fx.state,
        fx.host.clone(),
        fx.proj.to_string(),
        vec![SketchEditOp::SetEntityPositions {
            positions: vec![(point, Vec2::new_unchecked(3.0, 4.0))],
        }],
    )
    .await
    .expect("a detached point moves")
    .result
    .expect("solve");
    fx.worker.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unsupported_curve_is_refused_and_commits_nothing() {
    let Some(fx) = fixture(true).await else {
        return;
    };
    fx.enter().await;
    let entered = fx.snapshot_sketch().await;
    let completion = project(&fx).await;
    assert!(!completion.regen_expected);
    let receipt = completion.result.expect("refusals are answers");
    assert!(receipt.value.entities.is_empty(), "{:?}", receipt.value);
    assert_eq!(receipt.value.point_count, 0);
    assert_eq!(receipt.value.refusals.len(), 1, "{:?}", receipt.value);
    assert_eq!(receipt.value.refusals[0].code, "unsupportedCurve");
    assert!(
        receipt.solve.is_none(),
        "nothing committed, nothing resynced"
    );
    assert_eq!(fx.snapshot_sketch().await, entered, "no approximation");
    {
        let guard = fx.state.runtime.lock().await;
        let live = guard.as_ref().unwrap().get_sketch(fx.proj).unwrap();
        assert_eq!(live.history.unwrap().undo_depth, 0, "no visit step");
    }
    assert!(!fx.history(HistoryDirection::Undo).await);
    fx.worker.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lane_head_and_visit_fences_refuse_loudly() {
    let Some(fx) = fixture(false).await else {
        return;
    };
    // A free point for a drag to grab.
    let grabbed = EntityId::new();
    let free = SketchEntity::point(grabbed, Vec2::new_unchecked(1.0, 1.0), false, false);
    {
        let mut guard = fx.state.runtime.lock().await;
        let rt = guard.as_mut().unwrap();
        rt.apply(EditCommand::SketchEdit {
            sketch: fx.proj,
            ops: vec![SketchEditOp::AddEntity { entity: free }],
        })
        .unwrap();
    }
    fx.enter().await;
    let before = fx.snapshot_sketch().await;

    // ── An open drag owns the lane ───────────────────────────────────────────
    {
        let mut guard = fx.state.runtime.lock().await;
        let target = onecad_app::worker::wire::GestureTarget::point(grabbed);
        let rt = guard.as_mut().unwrap();
        rt.begin_gesture(fx.proj, grabbed, target).await.unwrap();
    }
    let id = fx.proj.to_string();
    let errors = [
        project_err(&fx).await,
        api::update_projection_with_completion(&fx.state, fx.host.clone(), id.clone())
            .await
            .expect_err("update under a drag")
            .to_string(),
        api::detach_projection_with_completion(&fx.state, fx.host.clone(), id, None)
            .await
            .expect_err("detach under a drag")
            .to_string(),
    ];
    for error in errors {
        assert!(error.contains("drag gesture"), "{error}");
    }
    {
        let mut guard = fx.state.runtime.lock().await;
        guard.as_mut().unwrap().cancel_gesture().await.unwrap();
    }

    // ── A superseded pick ────────────────────────────────────────────────────
    fx.widen().await;
    let error = project_err(&fx).await;
    assert!(error.contains("re-pick"), "{error}");

    // ── A held lane, a closed visit, a concurrent document edit ──────────────
    let mut guard = fx.state.runtime.lock().await;
    let rt = guard.as_mut().unwrap();
    let probe = || {
        vec![SketchEditOp::AddEntity {
            entity: SketchEntity::point(
                EntityId::new(),
                Vec2::new_unchecked(5.0, 5.0),
                false,
                false,
            ),
        }]
    };
    let visit = rt.claim_projection_visit(fx.proj, None, "t").await.unwrap();
    let error = rt.sketch_upsert(fx.proj, probe()).await.unwrap_err();
    assert!(error.to_string().contains("in flight"), "{error}");
    rt.cancel_sketch(fx.proj, false).await.unwrap();
    let error = rt
        .commit_projection_visit(visit, probe(), "t")
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("changed while projecting"),
        "{error}"
    );

    let visit = rt.claim_projection_visit(fx.proj, None, "t").await.unwrap();
    rt.apply(EditCommand::SketchEdit {
        sketch: fx.base,
        ops: probe(),
    })
    .expect("a concurrent edit elsewhere in the document");
    let error = rt
        .commit_projection_visit(visit, probe(), "t")
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("changed while projecting"),
        "{error}"
    );
    assert_eq!(fx.sketch(rt), before, "every refusal committed nothing");
    drop(guard);
    fx.worker.shutdown().await;
}

async fn project_err(fx: &Fixture) -> String {
    api::project_to_sketch_with_completion(
        &fx.state,
        fx.host.clone(),
        fx.snapshot.0,
        fx.proj.to_string(),
        "faceOutline".into(),
        fx.source(),
    )
    .await
    .expect_err("the projection must be refused")
    .to_string()
}
