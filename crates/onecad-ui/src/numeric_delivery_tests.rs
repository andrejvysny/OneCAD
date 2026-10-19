//! Routed keyboard and real egui numeric fields over the production sketch actor.
use super::dimension_delivery_tests::{accept_capture, click, ids};
use super::*;
use onecad_core::{
    math::Vec2,
    sketch::{Constraint, SketchEntity},
};
use onecad_interaction::sketch::{CapturedSketchPoint, SketchAction, SketchPhase, SketchTool};

fn key(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

fn numeric_frame(app: &mut App, context: &egui::Context, events: Vec<egui::Event>) {
    let mut raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(900.0, 560.0),
        )),
        events,
        focused: true,
        ..Default::default()
    };
    crate::input::route(app, context, &mut raw);
    let mut output = context.run_ui(raw, |_| crate::sketch_numeric::show(app, context));
    output.textures_delta.clear();
}

fn replace_field(app: &mut App, context: &egui::Context, text: &str) {
    let modifiers = egui::Modifiers {
        command: true,
        mac_cmd: true,
        ..Default::default()
    };
    numeric_frame(
        app,
        context,
        vec![
            egui::Event::ModifiersChanged(modifiers),
            egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            },
            egui::Event::Text(text.into()),
        ],
    );
    numeric_frame(
        app,
        context,
        vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)],
    );
}

fn draft_rectangle(app: &mut App, anchor: [f64; 2]) {
    app.sketch_dispatch(SketchAction::SetTool(SketchTool::Rectangle));
    let capture = accept_capture(app);
    click(app, capture, anchor[0], anchor[1], ids());
    let session = app.sketch.session.as_ref().unwrap();
    app.sketch_dispatch(SketchAction::Move(CapturedSketchPoint {
        key: session.key.clone(),
        capture,
        point: Vec2::new(anchor[0] + 10.0, anchor[1] + 10.0).unwrap(),
    }));
    assert!(app.sketch.session.as_ref().unwrap().draft.is_some());
}

fn assert_rectangle(app: &App, width: f64, height: f64) {
    let sketch = &app.sketch.session.as_ref().unwrap().authoritative;
    assert_eq!(sketch.entities().len(), 8);
    assert_eq!(sketch.constraints().len(), 6);
    let points: Vec<_> = sketch
        .entities()
        .iter()
        .filter_map(|entity| match entity {
            SketchEntity::Point { at, .. } => Some(*at),
            _ => None,
        })
        .collect();
    let min_x = points
        .iter()
        .map(|point| point.x)
        .fold(f64::INFINITY, f64::min);
    let max_x = points
        .iter()
        .map(|point| point.x)
        .fold(f64::NEG_INFINITY, f64::max);
    let min_y = points
        .iter()
        .map(|point| point.y)
        .fold(f64::INFINITY, f64::min);
    let max_y = points
        .iter()
        .map(|point| point.y)
        .fold(f64::NEG_INFINITY, f64::max);
    assert!((max_x - min_x - width).abs() < 1e-8);
    assert!((max_y - min_y - height).abs() < 1e-8);
    let dimensions: Vec<_> = sketch
        .constraints()
        .iter()
        .filter_map(|constraint| match constraint {
            Constraint::Distance { value, .. } => Some(value.value),
            _ => None,
        })
        .collect();
    assert_eq!(dimensions.len(), 2);
    assert!(dimensions.contains(&width));
    assert!(dimensions.contains(&height));
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native host input is not physical acceptance"]
async fn routed_tab_numeric_fields_enter_once_and_ime_cancel_preserve_authority() {
    let area = tempfile::tempdir().unwrap();
    let (mut app, context) = create_app(area.path()).await;
    let mut frame = eframe::Frame::_new_kittest();
    pump_until(&mut app, &context, &mut frame, |app| app.display.is_some()).await;
    app.document_request(Request::New);
    pump_until(&mut app, &context, &mut frame, |app| !app.documents.running).await;
    app.new_sketch();
    pump_until(&mut app, &context, &mut frame, |app| {
        app.sketch
            .session
            .as_ref()
            .is_some_and(|session| session.phase == SketchPhase::Active)
    })
    .await;
    let before = app.sketch.session.as_ref().unwrap().authoritative.clone();
    let history = app
        .sketch
        .session
        .as_ref()
        .unwrap()
        .history
        .clone()
        .unwrap();
    let revision = app.sketch.session.as_ref().unwrap().document.revision;
    draft_rectangle(&mut app, [0.0, 0.0]);
    numeric_frame(&mut app, &context, vec![key(egui::Key::Tab)]);
    assert!(
        crate::sketch_numeric::editor_open(&context),
        "routed Tab must open numeric creation"
    );
    replace_field(&mut app, &context, "80");
    numeric_frame(&mut app, &context, vec![key(egui::Key::Tab)]);
    replace_field(&mut app, &context, "50");
    assert_eq!(app.sketch.session.as_ref().unwrap().authoritative, before);
    assert_eq!(
        app.sketch.session.as_ref().unwrap().document.revision,
        revision
    );
    numeric_frame(&mut app, &context, vec![key(egui::Key::Enter)]);
    assert!(!crate::sketch_numeric::editor_open(&context));
    pump_until(&mut app, &context, &mut frame, |app| {
        app.sketch.session.as_ref().is_some_and(|session| {
            session.phase == SketchPhase::Active && session.authoritative.entities().len() == 8
        })
    })
    .await;
    assert_rectangle(&app, 80.0, 50.0);
    assert_eq!(
        app.sketch
            .session
            .as_ref()
            .unwrap()
            .history
            .as_ref()
            .unwrap()
            .undo_depth,
        history.undo_depth + 1
    );
    numeric_frame(&mut app, &context, vec![]);
    cancel_during_ime(&mut app, &context);
    app.sketch_dispatch(SketchAction::Discard);
    pump_until(&mut app, &context, &mut frame, |app| !app.sketch_active()).await;
    app.stop_discard();
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
}

fn cancel_during_ime(app: &mut App, context: &egui::Context) {
    let accepted = app.sketch.session.as_ref().unwrap().authoritative.clone();
    let history = app.sketch.session.as_ref().unwrap().history.clone();
    let revision = app.sketch.session.as_ref().unwrap().document.revision;
    draft_rectangle(app, [100.0, 0.0]);
    numeric_frame(app, context, vec![key(egui::Key::Tab)]);
    assert!(crate::sketch_numeric::editor_open(context));
    numeric_frame(
        app,
        context,
        vec![
            egui::Event::Ime(egui::ImeEvent::Preedit {
                text: "候補".into(),
                active_range_chars: None,
            }),
            key(egui::Key::Enter),
        ],
    );
    assert!(crate::sketch_numeric::editor_open(context));
    assert!(app.sketch.session.as_ref().unwrap().draft.is_some());
    numeric_frame(
        app,
        context,
        vec![
            egui::Event::Ime(egui::ImeEvent::Commit("8".into())),
            key(egui::Key::Escape),
        ],
    );
    assert!(crate::sketch_numeric::editor_open(context));
    assert!(app.sketch.session.as_ref().unwrap().draft.is_some());
    numeric_frame(app, context, vec![key(egui::Key::Escape)]);
    assert!(!crate::sketch_numeric::editor_open(context));
    let session = app.sketch.session.as_ref().unwrap();
    assert_eq!(session.phase, SketchPhase::Active);
    assert_eq!(session.authoritative, accepted);
    assert_eq!(session.document.revision, revision);
    assert_eq!(session.history, history);
}
