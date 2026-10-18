use super::*;
use std::future::{poll_fn, Future};
use std::task::Poll;

#[tokio::test]
async fn finish_waits_for_every_prepared_region_query_before_committing() {
    let (mut runtime, solver) = runtime(Mode::Unchanged);
    runtime.enter_sketch(sketch_id()).await.unwrap();
    let first = runtime.prepare_sketch_regions(sketch_id()).unwrap();
    let second = runtime.prepare_sketch_regions(sketch_id()).unwrap();
    let calls = solver.calls.load(Ordering::SeqCst);
    {
        let finish = runtime.finish_sketch_with_outcome(sketch_id());
        tokio::pin!(finish);
        assert!(matches!(
            poll_fn(|cx| Poll::Ready(finish.as_mut().poll(cx))).await,
            Poll::Pending
        ));
        assert_eq!(solver.calls.load(Ordering::SeqCst), calls);
        first.drive().await.unwrap();
        assert!(matches!(
            poll_fn(|cx| Poll::Ready(finish.as_mut().poll(cx))).await,
            Poll::Pending
        ));
        second.drive().await.unwrap();
        let (_, outcome) = finish.await.unwrap();
        assert!(outcome.unwrap().projection_delta.timeline_changed);
    }
    assert_eq!(solver.calls.load(Ordering::SeqCst), calls + 3);
    assert!(runtime.get_sketch(sketch_id()).unwrap().solve_current);
    assert!(runtime.solver_lane.owner(sketch_id()).is_none());
}

#[tokio::test]
async fn waiting_upsert_rejects_a_changed_document_fence_without_mutation() {
    let (mut runtime, _) = runtime(Mode::Unchanged);
    let before = runtime.get_sketch(sketch_id()).unwrap().entities;
    let depth = runtime.undo_depth();
    let prepared = runtime.prepare_sketch_regions(sketch_id()).unwrap();
    let fencing = runtime.fencing();
    {
        let upsert = runtime.sketch_upsert_with_outcome(sketch_id(), moved_ops());
        tokio::pin!(upsert);
        assert!(matches!(
            poll_fn(|cx| Poll::Ready(upsert.as_mut().poll(cx))).await,
            Poll::Pending
        ));
        fencing.bump_revision();
        drop(prepared);
        assert!(upsert.await.is_err());
    }
    assert_eq!(runtime.get_sketch(sketch_id()).unwrap().entities, before);
    assert_eq!(runtime.undo_depth(), depth);
    assert!(runtime.solver_lane.owner(sketch_id()).is_none());
}

#[tokio::test]
async fn entry_waits_for_a_region_drive_then_installs_its_session() {
    let (mut runtime, solver) = runtime(Mode::Unchanged);
    let prepared = runtime.prepare_sketch_regions(sketch_id()).unwrap();
    {
        let enter = runtime.enter_sketch_with_outcome(sketch_id());
        tokio::pin!(enter);
        assert!(matches!(
            poll_fn(|cx| Poll::Ready(enter.as_mut().poll(cx))).await,
            Poll::Pending
        ));
        prepared.drive().await.unwrap();
        assert!(enter.await.unwrap().1.is_none());
    }
    assert_eq!(solver.calls.load(Ordering::SeqCst), 2);
    assert!(runtime.sketch_session.is_some());
    assert!(runtime.solver_lane.owner(sketch_id()).is_none());
}

#[tokio::test]
async fn edit_waits_for_a_region_drive_then_commits_once() {
    let (mut runtime, solver) = runtime(Mode::Unchanged);
    let prepared = runtime.prepare_sketch_regions(sketch_id()).unwrap();
    let depth = runtime.undo_depth();
    {
        let upsert = runtime.sketch_upsert_with_outcome(sketch_id(), moved_ops());
        tokio::pin!(upsert);
        assert!(matches!(
            poll_fn(|cx| Poll::Ready(upsert.as_mut().poll(cx))).await,
            Poll::Pending
        ));
        prepared.drive().await.unwrap();
        assert!(upsert.await.unwrap().1.is_some());
    }
    assert_eq!(solver.calls.load(Ordering::SeqCst), 2);
    assert_eq!(runtime.undo_depth(), depth + 1);
    assert!(runtime.get_sketch(sketch_id()).unwrap().solve_current);
    assert!(runtime.solver_lane.owner(sketch_id()).is_none());
}

#[tokio::test]
async fn waiting_finish_rejects_a_worker_restart_without_mutation() {
    let (mut runtime, solver) = runtime(Mode::Unchanged);
    let before = runtime.get_sketch(sketch_id()).unwrap().entities;
    let depth = runtime.undo_depth();
    let prepared = runtime.prepare_sketch_regions(sketch_id()).unwrap();
    {
        let finish = runtime.finish_sketch_with_outcome(sketch_id());
        tokio::pin!(finish);
        assert!(matches!(
            poll_fn(|cx| Poll::Ready(finish.as_mut().poll(cx))).await,
            Poll::Pending
        ));
        solver.backend.epoch.store(2, Ordering::SeqCst);
        drop(prepared);
        assert!(finish.await.is_err());
    }
    assert_eq!(runtime.get_sketch(sketch_id()).unwrap().entities, before);
    assert_eq!(runtime.undo_depth(), depth);
    assert!(runtime.solver_lane.owner(sketch_id()).is_none());
}
