//! Raw host routing must preserve the state under dismissible chrome.
use super::*;
use onecad_core::{ids::EntityId, math::Vec2};
use onecad_interaction::sketch::{SketchDraft, SketchPhase, SketchTool};

fn key(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn overlays_preserve_sketch_draft_selection_and_history_through_raw_replay() {
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
    for customize in [true, false] {
        let session = app.sketch.session.as_mut().unwrap();
        session.tool = SketchTool::Line;
        session
            .selection
            .push(onecad_interaction::sketch::SketchTarget::Entity(
                EntityId::new(),
            ));
        session.draft = Some(SketchDraft {
            first: Vec2::new(0.0, 0.0).unwrap(),
            anchor: Vec2::new(0.0, 0.0).unwrap(),
            first_id: None,
            anchor_id: None,
            cursor: Vec2::new(10.0, 0.0).unwrap(),
            segments: 0,
            anchor_snap: None,
            secondary: None,
            preceding: None,
            tangent_arc: false,
        });
        app.ui.shell_facade.customize_open = customize;
        app.ui.shell_facade.layers_open = !customize;
        assert_owned_input(&mut app, &context);
    }
    app.stop();
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
}

fn assert_owned_input(app: &mut App, context: &egui::Context) {
    let before = format!("{:?}", app.sketch.session);
    let document = app.document_state.clone();
    let mut raw = egui::RawInput {
        events: vec![key(egui::Key::Enter), key(egui::Key::Delete)],
        ..Default::default()
    };
    crate::menu::push_command(&mut raw.events, egui::Key::Z, false);
    crate::menu::push_command(&mut raw.events, egui::Key::S, false);
    app.raw_input_hook(context, &mut raw);
    assert_eq!(format!("{:?}", app.sketch.session), before);
    assert!(!app.documents.running);
    raw.events.push(key(egui::Key::Escape));
    app.raw_input_hook(context, &mut raw);
    assert!(!app.ui.shell_facade.customize_open && !app.ui.shell_facade.layers_open);
    app.raw_input_hook(context, &mut raw);
    assert_eq!(format!("{:?}", app.sketch.session), before);
    assert_eq!(format!("{:?}", app.document_state), format!("{document:?}"));
    raw.events.push(key(egui::Key::Escape));
    app.raw_input_hook(context, &mut raw);
    assert!(app.sketch.session.as_ref().unwrap().draft.is_none());
    assert!(!app.sketch.session.as_ref().unwrap().selection.is_empty());
}
