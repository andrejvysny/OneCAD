//! Synthetic native chooser input with a real application, production worker and Metal.
use super::*;
use onecad_core::{math::Vec2, sketch::WorldPlane};
use onecad_interaction::sketch::{SketchAction, SketchPhase};
use onecad_scene::{DVec3, ViewSnapshot, Viewport};

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic pointer input is not physical acceptance"]
async fn viewport_world_patches_hover_cancel_and_click_without_stray_sketches() {
    let area = tempfile::tempdir().unwrap();
    let (mut app, context) = create_app(area.path()).await;
    let mut frame = eframe::Frame::_new_kittest();
    pump_until(&mut app, &context, &mut frame, |app| {
        app.doc.display.is_some()
    })
    .await;
    for plane in [WorldPlane::XY, WorldPlane::XZ, WorldPlane::YZ] {
        let (view, point) = patch_view(&mut app, plane);
        let before = serde_json::to_value(&app.doc.display.as_ref().unwrap().projection).unwrap();
        let dirty = app.doc.authoring_dirty;
        app.choose_sketch_support();
        chooser_frame(
            &mut app,
            &context,
            view,
            vec![egui::Event::PointerMoved(point)],
        );
        assert_eq!(app.doc.sketch_entry.plane, Some(plane.plane()));
        assert_eq!(
            serde_json::to_value(&app.doc.display.as_ref().unwrap().projection).unwrap(),
            before
        );
        assert_eq!(app.doc.authoring_dirty, dirty);
        chooser_frame(&mut app, &context, view, vec![escape()]);
        assert!(!app.doc.sketch_entry.open);
        assert!(app.doc.sketch_entry.plane.is_none() && !app.sketch_active());
        assert_eq!(
            serde_json::to_value(&app.doc.display.as_ref().unwrap().projection).unwrap(),
            before
        );
        app.choose_sketch_support();
        chooser_frame(
            &mut app,
            &context,
            view,
            vec![egui::Event::PointerMoved(point), button(point, true)],
        );
        chooser_frame(&mut app, &context, view, vec![button(point, false)]);
        pump_until(&mut app, &context, &mut frame, |app| {
            app.doc
                .sketch
                .session
                .as_ref()
                .is_some_and(|session| session.phase == SketchPhase::Active)
        })
        .await;
        assert_eq!(
            app.doc.sketch.session.as_ref().unwrap().plane,
            plane.plane()
        );
        assert_eq!(
            app.doc.display.as_ref().unwrap().projection.sketches.len(),
            1
        );
        app.sketch_dispatch(SketchAction::Discard);
        pump_until(&mut app, &context, &mut frame, |app| !app.sketch_active()).await;
        assert!(
            app.doc
                .display
                .as_ref()
                .unwrap()
                .projection
                .sketches
                .is_empty()
        );
    }
    app.stop_discard();
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
}

fn patch_view(app: &mut App, plane: WorldPlane) -> (ViewSnapshot, egui::Pos2) {
    crate::sketch_entry::align_camera(&mut app.doc.camera, plane.plane());
    let view = ViewSnapshot::new(
        app.doc.display.as_ref().unwrap().scene.identity(),
        500,
        app.doc.camera,
        Viewport {
            width: 900.0,
            height: 560.0,
            scale_factor: 1.0,
        },
    )
    .unwrap();
    let extent = (app.doc.camera.offset.length() * 0.22).clamp(1.0, 1.0e6);
    let point = plane
        .plane()
        .to_world(Vec2::new_unchecked(extent * 0.6, extent * 0.6));
    let point = view
        .project(DVec3::new(point.x, point.y, point.z))
        .unwrap()
        .logical;
    (view, egui::pos2(point.x as f32, point.y as f32))
}

fn chooser_frame(
    app: &mut App,
    context: &egui::Context,
    view: ViewSnapshot,
    events: Vec<egui::Event>,
) {
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 560.0));
    let mut input = egui::RawInput {
        screen_rect: Some(rect),
        events,
        focused: true,
        ..Default::default()
    };
    crate::input::route(app, context, &mut input);
    let display = app.doc.display.clone().unwrap();
    let mut output = context.run_ui(input, |ui| {
        let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
        crate::sketch_entry::viewport(app, ui, rect, &response, &view, &display);
    });
    output.textures_delta.clear();
}

fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

fn escape() -> egui::Event {
    egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}
