//! Synthetic native-host pointer delivery with production worker and Metal.
use super::dimension_delivery_tests::{accept_capture, click, ids};
use super::*;
use onecad_core::math::Vec2;
use onecad_interaction::sketch::{SketchAction, SketchPhase, SketchTarget, SketchTool};
use onecad_scene::{Camera, DVec3, Projection};

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic pointer delivery is not physical acceptance"]
async fn marquee_survives_redraw_and_cancels_when_camera_changes() {
    let area = tempfile::tempdir().unwrap();
    let (mut app, context) = create_app(area.path()).await;
    let mut frame = eframe::Frame::_new_kittest();
    pump_until(&mut app, &context, &mut frame, |app| {
        app.doc.display.is_some()
    })
    .await;
    let rectangle = create_rectangle(&mut app, &context, &mut frame).await;
    prepare_view(&mut app);
    for _ in 0..3 {
        editor_frame(&mut app, &context, &mut frame, vec![]);
    }
    let (start, end) = window(&app);
    let capture = app.doc.view.unwrap().identity();
    begin_drag(&mut app, &context, &mut frame, start);
    redraw_at(&mut app, &context, &mut frame, end);
    assert!(app.doc.sketch.session.as_ref().unwrap().selection_dragging);
    assert_eq!(app.doc.view.unwrap().identity(), capture);
    assert_eq!(
        app.doc.sketch.session.as_ref().unwrap().accepted_capture,
        Some(capture)
    );
    editor_frame(&mut app, &context, &mut frame, vec![button(end, false)]);
    let selected = &app.doc.sketch.session.as_ref().unwrap().selection;
    assert_eq!(selected.len(), 4, "window must select four whole lines");
    for id in &rectangle.entities[4..8] {
        assert!(selected.contains(&SketchTarget::Entity(*id)));
    }
    clear_with_pointer(&mut app, &context, &mut frame, start);
    begin_drag(&mut app, &context, &mut frame, start);
    app.doc.camera.target.x += 1.0;
    redraw_at(&mut app, &context, &mut frame, end);
    assert_ne!(app.doc.view.unwrap().identity(), capture);
    assert!(!app.doc.sketch.session.as_ref().unwrap().selection_dragging);
    editor_frame(&mut app, &context, &mut frame, vec![button(end, false)]);
    assert!(
        app.doc
            .sketch
            .session
            .as_ref()
            .unwrap()
            .selection
            .is_empty()
    );
    app.sketch_dispatch(SketchAction::Discard);
    pump_until(&mut app, &context, &mut frame, |app| !app.sketch_active()).await;
    app.stop_discard();
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
}

async fn create_rectangle(
    app: &mut App,
    context: &egui::Context,
    frame: &mut eframe::Frame,
) -> onecad_interaction::sketch::PrimitiveIds {
    app.new_sketch();
    pump_until(app, context, frame, |app| {
        app.doc
            .sketch
            .session
            .as_ref()
            .is_some_and(|session| session.phase == SketchPhase::Active)
    })
    .await;
    app.sketch_dispatch(SketchAction::SetTool(SketchTool::Rectangle));
    let capture = accept_capture(app);
    click(app, capture, -10.0, -5.0, ids());
    let rectangle = ids();
    click(app, capture, 10.0, 5.0, rectangle.clone());
    pump_until(app, context, frame, |app| {
        app.doc.sketch.session.as_ref().is_some_and(|session| {
            session.phase == SketchPhase::Active
                && session.authoritative.contains_entity(rectangle.entities[7])
        })
    })
    .await;
    app.sketch_dispatch(SketchAction::SetTool(SketchTool::Select));
    rectangle
}

fn prepare_view(app: &mut App) {
    app.ui.start_open = false;
    app.ui.explorer_open = false;
    app.ui.inspector_open = false;
    app.doc.fit_pending = false;
    let plane = app.doc.sketch.session.as_ref().unwrap().plane;
    let target = plane.to_world(Vec2::new(0.0, 0.0).unwrap());
    app.doc.camera = Camera {
        target: DVec3::new(target.x, target.y, target.z),
        offset: DVec3::new(plane.normal.x, plane.normal.y, plane.normal.z) * 100.0,
        projection: Projection::Orthographic,
        ..Default::default()
    };
    app.draw_dirty = true;
}

fn window(app: &App) -> (egui::Pos2, egui::Pos2) {
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 820.0));
    let rect = crate::layout::chrome(screen, &app.ui).viewport;
    let view = app.doc.view.unwrap();
    let plane = app.doc.sketch.session.as_ref().unwrap().plane;
    let mut bounds = egui::Rect::NOTHING;
    for (x, y) in [(-10.0, -5.0), (10.0, -5.0), (10.0, 5.0), (-10.0, 5.0)] {
        let world = plane.to_world(Vec2::new(x, y).unwrap());
        let screen = view.project(DVec3::new(world.x, world.y, world.z)).unwrap();
        bounds.extend_with(rect.min + egui::vec2(screen.logical.x as f32, screen.logical.y as f32));
    }
    let bounds = bounds.expand(24.0);
    assert!(rect.contains_rect(bounds));
    (bounds.min, bounds.max)
}

fn begin_drag(
    app: &mut App,
    context: &egui::Context,
    frame: &mut eframe::Frame,
    start: egui::Pos2,
) {
    editor_frame(
        app,
        context,
        frame,
        vec![egui::Event::PointerMoved(start), button(start, true)],
    );
    redraw_at(app, context, frame, start + egui::vec2(12.0, 12.0));
    assert!(
        app.doc.sketch.session.as_ref().unwrap().selection_dragging,
        "native viewport must own marquee drag: {}",
        app.doc.status
    );
}

fn redraw_at(
    app: &mut App,
    context: &egui::Context,
    frame: &mut eframe::Frame,
    position: egui::Pos2,
) {
    app.draw_dirty = true;
    editor_frame(
        app,
        context,
        frame,
        vec![egui::Event::PointerMoved(position)],
    );
    for _ in 0..200 {
        if !app.draw_dirty {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
        editor_frame(app, context, frame, vec![]);
    }
    panic!("forced GPU redraw did not complete: {}", app.doc.status);
}

fn clear_with_pointer(
    app: &mut App,
    context: &egui::Context,
    frame: &mut eframe::Frame,
    position: egui::Pos2,
) {
    editor_frame(
        app,
        context,
        frame,
        vec![egui::Event::PointerMoved(position), button(position, true)],
    );
    editor_frame(app, context, frame, vec![button(position, false)]);
    assert!(
        app.doc
            .sketch
            .session
            .as_ref()
            .unwrap()
            .selection
            .is_empty()
    );
}

fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
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
    crate::input::route(app, context, &mut raw);
    let mut output = context.run_ui(raw, |ui| eframe::App::ui(app, ui, frame));
    output.textures_delta.clear();
}
