//! Real worker/Metal widget delivery; physical device acceptance remains separate.
use super::point_center_delivery_tests::support::Host;
use super::*;
use onecad_interaction::selection_set::{SelectionItem, SelectionMode};
use onecad_scene::Camera;

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic widget input is not physical acceptance"]
async fn sidebar_viewport_selection_and_delete_share_authority_and_history() {
    let root = tempfile::tempdir().unwrap();
    let mut host = prepared(root.path()).await;
    let display = host.app.doc.display.clone().unwrap();
    let body = display.scene.bodies()[0].body_id();
    let sketch = display.projection.sketches.values().next().unwrap().clone();
    let bounds = display.scene.bounds().unwrap();
    let view = host.app.doc.view.unwrap();
    let point = host
        .app
        .doc
        .view
        .unwrap()
        .project((bounds.min + bounds.max) * 0.5)
        .unwrap()
        .logical;
    crate::model_pick::pick(
        &mut host.app,
        &display,
        &view,
        point,
        false,
        SelectionMode::Replace,
    );
    assert_eq!(host.app.doc.selected_body(), Some(body));
    assert!(matches!(
        host.app.doc.selection.items(),
        [SelectionItem::Face(_) | SelectionItem::Edge(_)]
    ));
    assert!(!crate::item_selection::whole_body_selected(&host.app, body));
    crate::model_pick::pick(
        &mut host.app,
        &display,
        &view,
        point,
        true,
        SelectionMode::Replace,
    );
    assert!(crate::item_selection::whole_body_selected(&host.app, body));
    assert!(matches!(
        host.app.doc.selection.items(),
        [SelectionItem::Body(_)]
    ));
    host.app.ui.explorer_open = true;
    host.app.ui.inspector_open = true;
    host.redraw();
    select_row(&mut host, &sketch.name);
    for _ in 0..100 {
        host.ui(vec![]);
        if host.app.doc.selection_preview.ready() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    assert_eq!(
        host.app.doc.selected_sketch().unwrap().to_string(),
        sketch.id
    );
    assert!(host.app.doc.selected_body().is_none());
    assert!(
        host.app.doc.selection_preview.ready(),
        "hidden sketch selection must draw a preview"
    );
    assert!(!sketch.visible, "fixture preserves saved hidden visibility");
    let undo = host.app.doc.display.as_ref().unwrap().projection.undo_depth;
    let position = host.rect("Body 1").center();
    for pressed in [true, false] {
        host.ui(vec![button(position, pressed)]);
    }
    assert!(crate::item_selection::whole_body_selected(&host.app, body));
    host.control("Delete Delete / Backspace");
    confirmation(&mut host).await;
    host.control("Delete");
    settled(&mut host).await;
    assert!(
        host.app
            .doc
            .display
            .as_ref()
            .unwrap()
            .projection
            .bodies
            .is_empty()
    );
    assert_eq!(
        host.app
            .doc
            .display
            .as_ref()
            .unwrap()
            .projection
            .sketches
            .len(),
        1
    );
    assert_eq!(
        host.app.doc.display.as_ref().unwrap().projection.undo_depth,
        undo + 1
    );
    history(&mut host, false).await;
    assert_eq!(
        host.app
            .doc
            .display
            .as_ref()
            .unwrap()
            .projection
            .bodies
            .len(),
        1
    );
    host.redraw();
    keyboard_and_sketch_delete(&mut host, &sketch.name).await;
    host.stop().await;
}

async fn keyboard_and_sketch_delete(host: &mut Host, name: &str) {
    for _ in 0..24 {
        host.ui(vec![]);
    }
    select_row(host, name);
    assert!(!host.app.sketch_active());
    assert!(host.app.doc.selected_sketch().is_some());
    // Text editing owns Backspace even while a document item is selected.
    let filter = crate::layout::chrome(host.screen(), &host.app.ui)
        .explorer
        .min
        + egui::vec2(60.0, 70.0);
    host.click(filter);
    host.ui(vec![egui::Event::Text("Sketc".into())]);
    host.ui(vec![key(egui::Key::Backspace)]);
    assert!(!host.app.doc.items.blocks());
    host.app.ui.explorer_filter.clear();
    host.context.memory_mut(|memory| {
        if let Some(id) = memory.focused() {
            memory.surrender_focus(id);
        }
    });
    assert!(
        host.app.doc.selected_sketch().is_some(),
        "Filter must preserve item selection"
    );
    host.ui(vec![key(egui::Key::Delete)]);
    confirmation(host).await;
    host.control("Cancel");
    assert!(!host.app.doc.items.blocks());
    let count = host
        .app
        .doc
        .display
        .as_ref()
        .unwrap()
        .projection
        .features
        .len();
    host.ui(vec![key(egui::Key::Backspace)]);
    confirmation(host).await;
    host.control("Delete");
    settled(host).await;
    let projection = &host.app.doc.display.as_ref().unwrap().projection;
    assert!(
        projection.bodies.is_empty()
            && projection.sketches.is_empty()
            && projection.features.is_empty()
    );
    assert!(host.app.doc.selected_sketch().is_none() && host.app.doc.selected_body().is_none());
    history(host, false).await;
    assert_eq!(
        host.app
            .doc
            .display
            .as_ref()
            .unwrap()
            .projection
            .features
            .len(),
        count
    );
    history(host, true).await;
    assert!(
        host.app
            .doc
            .display
            .as_ref()
            .unwrap()
            .projection
            .sketches
            .is_empty()
    );
}

async fn prepared(root: &std::path::Path) -> Host {
    let (mut app, context) = create_app(root).await;
    context.enable_accesskit();
    let mut frame = eframe::Frame::_new_kittest();
    pump_until(&mut app, &context, &mut frame, |app| {
        app.doc.display.is_some()
    })
    .await;
    super::sketch_entry_delivery_tests::make_host(&mut app, &context, &mut frame).await;
    app.ui.start_open = false;
    app.doc.camera = Camera::default();
    app.doc.fit_pending = true;
    let mut host = Host {
        app,
        context,
        frame,
        size: egui::vec2(1280.0, 820.0),
        widgets: vec![],
    };
    host.redraw();
    host
}

async fn confirmation(host: &mut Host) {
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        app.doc.items.confirmation.is_some()
    })
    .await;
    host.redraw();
}

async fn settled(host: &mut Host) {
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        !app.doc.items.blocks()
    })
    .await;
    assert_eq!(host.app.doc.status, "Delete complete");
}

async fn history(host: &mut Host, redo: bool) {
    let before = host.app.doc.display.as_ref().unwrap().projection.undo_depth;
    host.app.submit(if redo {
        crate::bridge::Command::Redo
    } else {
        crate::bridge::Command::Undo
    });
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        app.doc.display.as_ref().unwrap().projection.undo_depth != before
    })
    .await;
}

fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Secondary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

fn key(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

fn select_row(host: &mut Host, name: &str) {
    host.redraw();
    let bounds = crate::layout::chrome(host.screen(), &host.app.ui).explorer;
    let position = host
        .widgets
        .iter()
        .find(|(label, rect)| label == name && bounds.contains(rect.center()))
        .unwrap()
        .1
        .center();
    host.click(position);
}
