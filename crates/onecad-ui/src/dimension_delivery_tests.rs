//! Native-host dimension intent and egui field coverage, separate from physical input evidence.
use super::*;
use crate::{constraint_authoring, tool_registry::ToolId};
use onecad_core::{
    ids::{ConstraintId, EntityId},
    math::Vec2,
    sketch::{Constraint, SketchEntity},
};
use onecad_interaction::sketch::{
    CapturedSketchPoint, PrimitiveIds, SelectionMode, SketchAction, SketchPhase, SketchTarget,
    SketchTool,
};
use onecad_scene::{Camera, DVec3, Projection, ViewSnapshot, Viewport};

pub(super) fn accept_capture(app: &mut App) -> onecad_scene::CaptureIdentity {
    app.view_number += 1;
    let session = app.sketch.session.as_ref().unwrap();
    let view = ViewSnapshot::new(
        session.document.scene,
        app.view_number,
        Camera {
            offset: DVec3::new(0.0, 0.0, 100.0),
            projection: Projection::Orthographic,
            ..Default::default()
        },
        Viewport {
            width: 900.0,
            height: 560.0,
            scale_factor: 1.0,
        },
    )
    .unwrap();
    let capture = view.identity();
    app.view = Some(view);
    app.sketch_dispatch(SketchAction::AcceptView {
        key: session.key.clone(),
        document: session.document.clone(),
        capture,
    });
    capture
}

pub(super) fn click(
    app: &mut App,
    capture: onecad_scene::CaptureIdentity,
    x: f64,
    y: f64,
    ids: PrimitiveIds,
) {
    let session = app.sketch.session.as_ref().unwrap();
    app.sketch_dispatch(SketchAction::Click {
        point: CapturedSketchPoint {
            key: session.key.clone(),
            capture,
            point: Vec2::new(x, y).unwrap(),
        },
        ids,
    });
}

pub(super) fn ids() -> PrimitiveIds {
    PrimitiveIds {
        entities: std::array::from_fn(|_| EntityId::new()),
        constraints: std::array::from_fn(|_| ConstraintId::new()),
    }
}

fn modal_frame(app: &mut App, context: &egui::Context, events: Vec<egui::Event>) {
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 560.0),
            )),
            events,
            ..Default::default()
        },
        |_| constraint_authoring::show(app, context),
    );
    output.textures_delta.clear();
}

fn submit_modal(app: &mut App, context: &egui::Context, value: &str) {
    modal_frame(app, context, vec![]);
    let command = egui::Modifiers {
        command: true,
        mac_cmd: true,
        ..Default::default()
    };
    modal_frame(
        app,
        context,
        vec![
            egui::Event::ModifiersChanged(command),
            egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: command,
            },
            egui::Event::Text(value.into()),
        ],
    );
    modal_frame(
        app,
        context,
        vec![
            egui::Event::ModifiersChanged(egui::Modifiers::NONE),
            egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
}

fn radius(app: &App, id: EntityId) -> f64 {
    match app
        .sketch
        .session
        .as_ref()
        .unwrap()
        .authoritative
        .get_entity(id)
        .unwrap()
    {
        SketchEntity::Circle { radius, .. } => *radius,
        _ => panic!("expected circle"),
    }
}

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn circle_dimension_modal_edits_existing_id_and_real_solver_radius() {
    let area = tempfile::tempdir().unwrap();
    let (mut app, context) = create_app(area.path()).await;
    let mut frame = eframe::Frame::_new_kittest();
    pump_until(&mut app, &context, &mut frame, |app| app.display.is_some()).await;
    app.new_sketch();
    pump_until(&mut app, &context, &mut frame, |app| {
        app.sketch
            .session
            .as_ref()
            .is_some_and(|session| session.phase == SketchPhase::Active)
    })
    .await;
    let capture = accept_capture(&mut app);
    app.sketch_dispatch(SketchAction::SetTool(SketchTool::Circle));
    click(&mut app, capture, 0.0, 0.0, ids());
    let second = ids();
    let circle = second.entities[1];
    click(&mut app, capture, 12.5, 0.0, second);
    pump_until(&mut app, &context, &mut frame, |app| {
        app.sketch.session.as_ref().is_some_and(|session| {
            session.phase == SketchPhase::Active && session.authoritative.contains_entity(circle)
        })
    })
    .await;
    app.sketch_dispatch(SketchAction::SetTool(SketchTool::Select));
    let capture = accept_capture(&mut app);
    let session = app.sketch.session.as_ref().unwrap();
    app.sketch_dispatch(SketchAction::Select {
        key: session.key.clone(),
        document: session.document.clone(),
        capture,
        targets: vec![SketchTarget::Entity(circle)],
        mode: SelectionMode::Replace,
    });
    constraint_authoring::activate(&mut app, &context, ToolId::Dimension);
    assert!(
        constraint_authoring::editor_open(&context),
        "dimension activation: status={} busy={} selection={:?} phase={:?} tool={:?} draft={}",
        app.status,
        app.busy(),
        app.sketch.session.as_ref().unwrap().selection,
        app.sketch.session.as_ref().unwrap().phase,
        app.sketch.session.as_ref().unwrap().tool,
        app.sketch.session.as_ref().unwrap().draft.is_some()
    );
    submit_modal(&mut app, &context, "25");
    pump_until(&mut app, &context, &mut frame, |app| {
        app.sketch.session.as_ref().is_some_and(|session| {
            session.phase == SketchPhase::Active
                && session
                    .authoritative
                    .constraints()
                    .iter()
                    .any(|constraint| matches!(constraint, Constraint::Diameter { .. }))
        })
    })
    .await;
    let constraint = app
        .sketch
        .session
        .as_ref()
        .unwrap()
        .authoritative
        .constraints()
        .iter()
        .find(|constraint| matches!(constraint, Constraint::Diameter { .. }))
        .unwrap()
        .id();
    assert!((radius(&app, circle) - 12.5).abs() < 1e-8);
    open_badge_editor(&mut app, &context, &mut frame, constraint).await;
    assert!(constraint_authoring::editor_open(&context));
    submit_modal(&mut app, &context, "30");
    pump_until(&mut app, &context, &mut frame, |app| {
        app.sketch.session.as_ref().is_some_and(|session| {
            session.phase == SketchPhase::Active
                && session
                    .authoritative
                    .get_constraint(constraint)
                    .and_then(Constraint::value)
                    .is_some_and(|value| (value.value - 30.0).abs() < 1e-8)
        })
    })
    .await;
    assert!((radius(&app, circle) - 15.0).abs() < 1e-8);
    assert_eq!(
        app.sketch
            .session
            .as_ref()
            .unwrap()
            .authoritative
            .constraints()
            .len(),
        1
    );
    let before = app.sketch.session.as_ref().unwrap();
    let request = before.request.clone();
    let document = before.document.clone();
    open_badge_editor(&mut app, &context, &mut frame, constraint).await;
    submit_modal(&mut app, &context, "30");
    let after = app.sketch.session.as_ref().unwrap();
    assert_eq!(
        after.request, request,
        "unchanged confirmation must not submit a mutation"
    );
    assert_eq!(
        after.document, document,
        "unchanged confirmation must preserve revision and snapshot"
    );
    assert!(!constraint_authoring::editor_open(&context));
    app.sketch_dispatch(SketchAction::Discard);
    pump_until(&mut app, &context, &mut frame, |app| !app.sketch_active()).await;
    app.stop_discard();
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
}

fn editor_frame(
    app: &mut App,
    context: &egui::Context,
    frame: &mut eframe::Frame,
    events: Vec<egui::Event>,
) {
    let mut raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1280.0, 820.0),
        )),
        events,
        focused: true,
        ..Default::default()
    };
    eframe::App::raw_input_hook(app, context, &mut raw);
    let mut output = context.run_ui(raw, |ui| eframe::App::ui(app, ui, frame));
    output.textures_delta.clear();
}

async fn open_badge_editor(
    app: &mut App,
    context: &egui::Context,
    frame: &mut eframe::Frame,
    constraint: ConstraintId,
) {
    app.ui.start_open = false;
    for _ in 0..100 {
        logic_tick(app, context, frame, false);
        editor_frame(app, context, frame, vec![]);
        let ready = app
            .view
            .as_ref()
            .zip(app.sketch.session.as_ref())
            .is_some_and(|(view, session)| {
                view.identity().scene == session.document.scene
                    && session.accepted_capture == Some(view.identity())
            });
        if ready {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    for _ in 0..3 {
        editor_frame(app, context, frame, vec![]);
    }
    let bounds = crate::sketch_dimensions::badge_rect(context, constraint)
        .expect("accepted native dimension badge");
    let position = bounds.center();
    let event = |pressed| egui::Event::PointerButton {
        pos: position,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    editor_frame(
        app,
        context,
        frame,
        vec![egui::Event::PointerMoved(position), event(true)],
    );
    editor_frame(app, context, frame, vec![event(false)]);
    assert!(
        constraint_authoring::editor_open(context),
        "badge click must open existing dimension editor; status={} phase={:?} tool={:?} current_rect={:?} clicked={:?}",
        app.status,
        app.sketch.session.as_ref().unwrap().phase,
        app.sketch.session.as_ref().unwrap().tool,
        crate::sketch_dimensions::badge_rect(context, constraint),
        position
    );
}
