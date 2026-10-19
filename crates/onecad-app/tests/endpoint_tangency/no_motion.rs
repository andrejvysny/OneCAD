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

async fn gesture(state: &AppState, sid: SketchId, capture: u64, cancel: bool) {
    let (scope, grab, before) = {
        let guard = state.runtime.lock().await;
        let rt = guard.as_ref().unwrap();
        let sketch = rt.sketch_snapshot(sid, "no-motion").unwrap();
        (
            rt.native_gesture_scope(
                sid,
                NativeGestureSource {
                    capture,
                    generation: 1,
                },
            )
            .unwrap(),
            endpoint(&sketch, eid(10), Role::Start).0,
            sketch,
        )
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
    if cancel {
        api::update_native_gesture(&handle, [grab[0] + 1., grab[1] + 1.])
            .await
            .unwrap();
        api::cancel_native_gesture(&handle).await.unwrap();
    } else {
        let completion =
            api::end_native_gesture_with_completion(state, host(), &handle, Some(grab))
                .await
                .unwrap();
        let receipt = completion.result.unwrap();
        if receipt.value.moved {
            let after = api::get_sketch_snapshot(state, sid).await.unwrap();
            eprintln!(
                "unexpected no-motion commit capture={capture} before={} after={}",
                serde_json::to_string(&before).unwrap(),
                serde_json::to_string(&after).unwrap()
            );
        }
        assert!(
            !receipt.value.moved,
            "zero pointer motion must not commit solver roundoff"
        );
        assert!(receipt.value.cleanup_error.is_none());
    }
}

async fn exercise_no_motion(arc_arc: bool) {
    let Some(wm) = worker().await else {
        return;
    };
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
    let before = rt.sketch_snapshot(sid, "before").unwrap();
    let revision = rt.revision();
    let depth = rt.undo_depth();
    let history = rt.get_sketch(sid).unwrap().history;
    assert!(!rt.is_dirty());
    let state = AppState::default();
    *state.runtime.lock().await = Some(rt);
    let autosave = *state.autosave_tick.borrow();
    for (capture, cancel) in [(1, false), (2, true), (3, false), (4, false)] {
        eprintln!("endpoint no-motion fixture arc_arc={arc_arc} capture={capture} cancel={cancel}");
        gesture(&state, sid, capture, cancel).await;
        let guard = state.runtime.lock().await;
        let rt = guard.as_ref().unwrap();
        assert_eq!(
            rt.sketch_snapshot(sid, "after").unwrap(),
            before,
            "accepted geometry must remain exact"
        );
        assert_eq!(rt.revision(), revision);
        assert_eq!(rt.undo_depth(), depth);
        assert!(!rt.is_dirty());
        assert_eq!(rt.get_sketch(sid).unwrap().history, history);
        assert_eq!(*state.autosave_tick.borrow(), autosave);
    }
    drop(state);
    wm.shutdown().await;
}

#[tokio::test]
async fn endpoint_tangent_no_motion_and_cancel_leave_exact_authority_and_history_unchanged() {
    exercise_no_motion(false).await;
    exercise_no_motion(true).await;
}
