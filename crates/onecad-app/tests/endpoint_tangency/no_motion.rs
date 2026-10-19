use super::*;
use async_trait::async_trait;
use onecad_app::{
    api,
    dto::NativeGestureSource,
    host::{AppEvent, DialogPort, EventSink, FileDialog, HostContext, HostPaths},
    state::AppState,
    worker::wire::{DragKind, GestureTarget},
};

struct Ports;
impl EventSink for Ports {
    fn emit(&self, _event: AppEvent) -> Result<(), String> {
        Ok(())
    }
}
#[async_trait]
impl DialogPort for Ports {
    async fn pick(&self, _request: FileDialog) -> Vec<String> {
        panic!("gesture must not open dialogs")
    }
}
fn host() -> HostContext {
    HostContext {
        paths: HostPaths::default(),
        events: Arc::new(Ports),
        dialogs: Arc::new(Ports),
    }
}

/// Accepted authority captured before a gesture sequence.
struct Accepted {
    sketch: Sketch,
    revision: onecad_core::ids::DocumentRevision,
    depth: usize,
    history: Option<onecad_app::dto::SketchHistoryAvailability>,
    autosave: u64,
}

async fn accepted(state: &AppState, sid: SketchId) -> Accepted {
    let guard = state.runtime.lock().await;
    let rt = guard.as_ref().unwrap();
    Accepted {
        sketch: rt.sketch_snapshot(sid, "accepted").unwrap(),
        revision: rt.revision(),
        depth: rt.undo_depth(),
        history: rt.get_sketch(sid).unwrap().history,
        autosave: *state.autosave_tick.borrow(),
    }
}

async fn assert_unchanged(state: &AppState, sid: SketchId, accepted: &Accepted) {
    let guard = state.runtime.lock().await;
    let rt = guard.as_ref().unwrap();
    assert_eq!(
        rt.sketch_snapshot(sid, "after").unwrap(),
        accepted.sketch,
        "accepted geometry must remain exact"
    );
    assert_eq!(rt.revision(), accepted.revision);
    assert_eq!(rt.undo_depth(), accepted.depth);
    assert!(!rt.is_dirty());
    assert_eq!(rt.get_sketch(sid).unwrap().history, accepted.history);
    assert_eq!(*state.autosave_tick.borrow(), accepted.autosave);
}

fn start_endpoint(sketch: &Sketch) -> [f64; 2] {
    endpoint(sketch, eid(10), Role::Start).0
}

/// Runs one ArcEnd gesture on the arc start and returns whether End committed.
async fn drive(
    state: &AppState,
    sid: SketchId,
    capture: u64,
    grab: [f64; 2],
    updates: &[[f64; 2]],
    end: Option<[f64; 2]>,
) -> bool {
    let scope = {
        let guard = state.runtime.lock().await;
        let source = NativeGestureSource {
            capture,
            generation: 1,
        };
        guard
            .as_ref()
            .unwrap()
            .native_gesture_scope(sid, source)
            .unwrap()
    };
    let target = GestureTarget {
        kind: DragKind::ArcEnd,
        entity: eid(10),
        role: Some("start"),
        grab: Some(grab),
    };
    let handle = api::begin_native_gesture(state, scope, target)
        .await
        .unwrap();
    api::ready_native_gesture(&handle).await.unwrap();
    for at in updates {
        api::update_native_gesture(&handle, *at).await.unwrap();
    }
    let completion = api::end_native_gesture_with_completion(state, host(), &handle, end)
        .await
        .unwrap();
    let receipt = completion.result.unwrap();
    assert!(receipt.value.cleanup_error.is_none());
    receipt.value.moved
}

async fn cancelled(state: &AppState, sid: SketchId, capture: u64, grab: [f64; 2]) {
    let scope = {
        let guard = state.runtime.lock().await;
        let source = NativeGestureSource {
            capture,
            generation: 1,
        };
        guard
            .as_ref()
            .unwrap()
            .native_gesture_scope(sid, source)
            .unwrap()
    };
    let target = GestureTarget {
        kind: DragKind::ArcEnd,
        entity: eid(10),
        role: Some("start"),
        grab: Some(grab),
    };
    let handle = api::begin_native_gesture(state, scope, target)
        .await
        .unwrap();
    api::ready_native_gesture(&handle).await.unwrap();
    api::update_native_gesture(&handle, [grab[0] + 1., grab[1] + 1.])
        .await
        .unwrap();
    api::cancel_native_gesture(&handle).await.unwrap();
}

async fn prepared(arc_arc: bool) -> Option<(WorkerManager, AppState, SketchId)> {
    let wm = worker().await?;
    let mut sketch = fixture(arc_arc);
    let sid = sketch.id;
    sketch
        .add_constraint(tangent(21, 10, 11, Role::Start, Role::Start))
        .unwrap();
    sketch
        .add_constraint(Constraint::Radius {
            id: cid(22),
            entity: eid(10),
            value: Scalar::new(5.),
        })
        .unwrap();
    let mut rt = runtime(&wm);
    rt.apply(EditCommand::AddSketch { sketch }).unwrap();
    rt.enter_sketch(sid).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    rt.save(&dir.path().join("no-motion.onecad"), meta())
        .unwrap();
    assert!(!rt.is_dirty());
    let state = AppState::default();
    *state.runtime.lock().await = Some(rt);
    Some((wm, state, sid))
}

async fn exercise_no_motion(arc_arc: bool) {
    let Some((wm, state, sid)) = prepared(arc_arc).await else {
        return;
    };
    let accepted = accepted(&state, sid).await;
    let grab = start_endpoint(&accepted.sketch);
    let away = [grab[0] + 1., grab[1] + 1.];
    for capture in 1..=6 {
        eprintln!("endpoint no-motion fixture arc_arc={arc_arc} capture={capture}");
        let moved = match capture {
            2 => {
                cancelled(&state, sid, capture, grab).await;
                false
            }
            // Repeated unchanged targets, then a return after moving away.
            4 => drive(&state, sid, capture, grab, &[grab, grab], Some(grab)).await,
            5 => drive(&state, sid, capture, grab, &[away, grab], Some(grab)).await,
            6 => drive(&state, sid, capture, grab, &[], None).await,
            _ => drive(&state, sid, capture, grab, &[], Some(grab)).await,
        };
        assert!(
            !moved,
            "zero pointer motion must not commit solver roundoff"
        );
        assert_unchanged(&state, sid, &accepted).await;
    }
    drop(state);
    wm.shutdown().await;
}

/// A committed move creates one history entry and Undo restores the accepted pose.
async fn assert_one_commit_then_undo(state: &AppState, sid: SketchId, accepted: &Accepted) {
    let mut guard = state.runtime.lock().await;
    let rt = guard.as_mut().unwrap();
    let after = rt.sketch_snapshot(sid, "moved").unwrap();
    assert_ne!(after, accepted.sketch);
    assert_truth(&after);
    assert_eq!(rt.undo_depth(), accepted.depth + 1);
    assert!(rt.is_dirty());
    assert_eq!(*state.autosave_tick.borrow(), accepted.autosave + 1);
    history(rt, sid, HistoryDirection::Undo).await;
    assert_sketch_close(
        &rt.sketch_snapshot(sid, "undone").unwrap(),
        &accepted.sketch,
    );
}

async fn exercise_real_motion(arc_arc: bool) {
    let Some((wm, state, sid)) = prepared(arc_arc).await else {
        return;
    };
    let accepted = accepted(&state, sid).await;
    let grab = start_endpoint(&accepted.sketch);
    // A sub-micron release is a real target, not solver roundoff.
    let small = [grab[0], grab[1] + 1e-7];
    assert!(drive(&state, sid, 1, grab, &[], Some(small)).await);
    assert_one_commit_then_undo(&state, sid, &accepted).await;
    // ArcEnd ignores grab: an off-handle release moves the endpoint to the target.
    let accepted = self::accepted(&state, sid).await;
    let off = [grab[0] + 0.25, grab[1] + 0.25];
    assert!(drive(&state, sid, 2, off, &[], Some(off)).await);
    {
        let guard = state.runtime.lock().await;
        let moved = guard.as_ref().unwrap().sketch_snapshot(sid, "off").unwrap();
        let at = start_endpoint(&moved);
        assert!(
            (at[0] - off[0]).hypot(at[1] - off[1]) < (grab[0] - off[0]).hypot(grab[1] - off[1])
        );
    }
    assert_one_commit_then_undo(&state, sid, &accepted).await;
    drop(state);
    wm.shutdown().await;
}

#[tokio::test]
async fn endpoint_tangent_no_motion_and_cancel_leave_exact_authority_and_history_unchanged() {
    exercise_no_motion(false).await;
    exercise_no_motion(true).await;
}

#[tokio::test]
async fn endpoint_tangent_small_and_off_handle_releases_commit_one_undoable_edit() {
    exercise_real_motion(false).await;
    exercise_real_motion(true).await;
}
