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
    assert_display_popover(&mut app, &context, &mut frame);
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
            first_target: None,
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

fn assert_display_popover(app: &mut App, context: &egui::Context, host: &mut eframe::Frame) {
    app.ui.start_open = false;
    for dark in [false, true] {
        app.ui.appearance = if dark {
            crate::ui_state::Appearance::Dark
        } else {
            crate::ui_state::Appearance::Light
        };
        crate::appearance::sync(app, context);
        for scale in [1.0, 2.0] {
            context.set_pixels_per_point(scale);
            for size in [egui::vec2(900.0, 560.0), egui::vec2(1280.0, 820.0)] {
                let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
                app.ui.display_menu = Some(egui::Rect::from_min_size(
                    egui::pos2(size.x - 60.0, 160.0),
                    egui::vec2(36.0, 36.0),
                ));
                for frame in 0..3 {
                    let mut output = context.run_ui(
                        egui::RawInput {
                            screen_rect: Some(screen),
                            ..Default::default()
                        },
                        |ui| eframe::App::ui(app, ui, host),
                    );
                    output.textures_delta.clear();
                    if frame == 2 {
                        let popup = context
                            .memory(|memory| memory.area_rect(egui::Id::unique("display-menu")))
                            .unwrap();
                        assert!(popup.width() <= 300.0, "oversized display menu: {popup:?}");
                        assert!(screen.contains_rect(popup));
                        let labels: Vec<_> = output
                            .shapes
                            .iter()
                            .filter_map(|shape| {
                                if let egui::Shape::Text(text) = &shape.shape {
                                    Some(text.galley.text())
                                } else {
                                    None
                                }
                            })
                            .collect();
                        assert!(labels.contains(&"Shaded with edges"));
                        assert!(labels.contains(&"Hidden line"));
                        assert!(labels.contains(&"Perspective"));
                        assert!(labels.contains(&"Orthographic"));
                        assert!(!labels.contains(&"SSAA 1×"));
                    }
                }
            }
        }
    }
    app.ui.display_menu = None;
    context.set_pixels_per_point(1.0);
}
