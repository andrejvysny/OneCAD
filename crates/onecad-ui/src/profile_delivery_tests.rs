//! Synthetic host validation of real profile refusal and service-owned repair re-entry.
use super::*;
use onecad_core::{
    ids::{ConstraintId, EntityId},
    math::Vec2,
};
use onecad_interaction::sketch::{
    CapturedSketchPoint, PrimitiveIds, SelectionMode, SketchAction, SketchPhase, SketchTarget,
    SketchTool,
};
use onecad_scene::{ViewSnapshot, Viewport};

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic adapters do not qualify physical input"]
async fn finish_refusal_reenters_fresh_service_visit_for_repair() {
    let area = tempfile::tempdir().unwrap();
    let (mut app, context) = create_app(area.path()).await;
    let mut frame = eframe::Frame::_new_kittest();
    pump_until(&mut app, &context, &mut frame, |app| app.display.is_some()).await;
    app.document_request(Request::New);
    pump_until(&mut app, &context, &mut frame, |app| !app.documents.running).await;
    app.new_sketch();
    wait_active(&mut app, &context, &mut frame).await;
    primitive(
        &mut app,
        &context,
        &mut frame,
        SketchTool::Rectangle,
        [0.0, 0.0],
        [80.0, 50.0],
    )
    .await;
    let line = primitive(
        &mut app,
        &context,
        &mut frame,
        SketchTool::Line,
        [20.0, 0.0],
        [60.0, 0.0],
    )
    .await;
    assert!(
        app.bridge.valid(),
        "recoverable profile refusal must not invalidate sketch rendering"
    );
    assert!(app.display.as_ref().unwrap().profiles.is_empty());
    assert!(
        app.diagnostics
            .profile
            .as_ref()
            .unwrap()
            .has_reason("SKETCH_PROFILE_OVERLAPPING_CURVES")
    );
    app.sketch_dispatch(SketchAction::SetTool(SketchTool::Select));
    let visit = app.sketch.session.as_ref().unwrap().key.clone();
    let edit_session = app.sketch.session.as_ref().unwrap().edit_session.clone();
    let count = app
        .sketch
        .session
        .as_ref()
        .unwrap()
        .authoritative
        .entities()
        .len();
    app.sketch_dispatch(SketchAction::Finish);
    pump_until(&mut app, &context, &mut frame, |app| {
        app.sketch
            .session
            .as_ref()
            .is_some_and(|session| session.phase == SketchPhase::Faulted && session.repair_reentry)
    })
    .await;
    assert!(app.bridge.valid());
    assert!(
        app.diagnostics
            .profile
            .as_ref()
            .unwrap()
            .has_reason("SKETCH_PROFILE_OVERLAPPING_CURVES")
    );
    let session = app.sketch.session.as_ref().unwrap();
    assert_eq!(session.key, visit);
    assert!(session.history.is_none() && session.edit_session.is_none());
    app.sketch_dispatch(SketchAction::ReenterForRepair {
        key: session.key.clone(),
        document: session.document.clone(),
    });
    wait_active(&mut app, &context, &mut frame).await;
    let session = app.sketch.session.as_ref().unwrap();
    assert_ne!(session.key.generation, visit.generation);
    assert_ne!(session.edit_session, edit_session);
    assert!(session.edit_session.is_some() && !session.repair_reentry);
    assert_eq!(session.authoritative.entities().len(), count);
    assert!(session.authoritative.contains_entity(line));
    select(&mut app, line);
    crate::sketch_repair::delete_selected(&mut app);
    pump_until(&mut app, &context, &mut frame, |app| {
        app.sketch.session.as_ref().is_some_and(|session| {
            session.phase == SketchPhase::Active && !session.authoritative.contains_entity(line)
        })
    })
    .await;
    assert!(app.bridge.valid());
    assert!(!app.display.as_ref().unwrap().profiles.is_empty());
    app.sketch_dispatch(SketchAction::Finish);
    pump_until(&mut app, &context, &mut frame, |app| !app.sketch_active()).await;
    assert_eq!(app.display.as_ref().unwrap().profiles.len(), 1);
    assert!(
        app.document_dirty(),
        "accepted pre-refusal geometry remains authored"
    );
    app.stop_discard();
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
}

async fn wait_active(app: &mut App, context: &egui::Context, frame: &mut eframe::Frame) {
    pump_until(app, context, frame, |app| {
        app.sketch
            .session
            .as_ref()
            .is_some_and(|session| session.phase == SketchPhase::Active)
    })
    .await;
}

fn ids() -> PrimitiveIds {
    PrimitiveIds {
        entities: std::array::from_fn(|_| EntityId::new()),
        constraints: std::array::from_fn(|_| ConstraintId::new()),
    }
}

fn capture(app: &mut App) -> ViewSnapshot {
    let session = app.sketch.session.as_ref().unwrap();
    app.view_number += 1;
    let view = ViewSnapshot::new(
        session.document.scene,
        app.view_number,
        Default::default(),
        Viewport {
            width: 900.0,
            height: 560.0,
            scale_factor: 1.0,
        },
    )
    .unwrap();
    app.sketch_dispatch(SketchAction::AcceptView {
        key: session.key.clone(),
        document: session.document.clone(),
        capture: view.identity(),
    });
    view
}

fn click(app: &mut App, view: &ViewSnapshot, xy: [f64; 2], ids: PrimitiveIds) {
    app.sketch_dispatch(SketchAction::Click {
        point: CapturedSketchPoint {
            key: app.sketch.session.as_ref().unwrap().key.clone(),
            capture: view.identity(),
            point: Vec2::new(xy[0], xy[1]).unwrap(),
        },
        ids,
    });
}

async fn primitive(
    app: &mut App,
    context: &egui::Context,
    frame: &mut eframe::Frame,
    tool: SketchTool,
    first: [f64; 2],
    end: [f64; 2],
) -> EntityId {
    app.sketch_dispatch(SketchAction::SetTool(tool));
    let view = capture(app);
    click(app, &view, first, ids());
    let final_ids = ids();
    let entity = if tool == SketchTool::Rectangle {
        final_ids.entities[7]
    } else {
        final_ids.entities[2]
    };
    click(app, &view, end, final_ids);
    pump_until(app, context, frame, |app| {
        app.sketch.session.as_ref().is_some_and(|session| {
            session.phase == SketchPhase::Active && session.authoritative.contains_entity(entity)
        })
    })
    .await;
    entity
}

fn select(app: &mut App, entity: EntityId) {
    app.sketch_dispatch(SketchAction::SetTool(SketchTool::Select));
    let view = capture(app);
    let session = app.sketch.session.as_ref().unwrap();
    app.sketch_dispatch(SketchAction::Select {
        key: session.key.clone(),
        document: session.document.clone(),
        capture: view.identity(),
        targets: vec![SketchTarget::Entity(entity)],
        mode: SelectionMode::Replace,
    });
}
