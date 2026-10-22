//! Typed model selection through the real host route (ONECAD-315 S-315b) with the
//! production worker and Metal. Synthetic input is not physical acceptance.
use super::point_center_delivery_tests::support::{Host, key};
use super::*;
use onecad_interaction::selection_set::SelectionItem;
use onecad_scene::{Camera, DVec2, PickKind, PickPolicy};

pub(super) async fn boxed(root: &std::path::Path) -> Host {
    boxed_with(root, onecad_render::RenderProfile::Desktop).await
}

pub(super) async fn boxed_with(
    root: &std::path::Path,
    profile: onecad_render::RenderProfile,
) -> Host {
    let (mut app, context) = create_app_with(root, profile).await;
    context.enable_accesskit();
    let mut frame = eframe::Frame::_new_kittest();
    pump_until(&mut app, &context, &mut frame, |app| {
        app.doc.display.is_some()
    })
    .await;
    super::sketch_entry_delivery_tests::make_host(&mut app, &context, &mut frame).await;
    app.ui.start_open = false;
    app.ui.explorer_open = false;
    app.ui.inspector_open = false;
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

/// Screen points over distinct sub-shapes of `kind`, from the central viewport.
pub(super) fn targets(host: &Host, kind: PickKind) -> Vec<(egui::Pos2, String)> {
    let display = host.app.doc.display.clone().unwrap();
    let view = host.app.doc.view.unwrap();
    let rect = crate::layout::chrome(host.screen(), &host.app.ui).viewport;
    let size = view.viewport();
    let at = |logical: DVec2| {
        onecad_scene::pick_with_policy(
            &display.scene,
            &view,
            view.identity(),
            logical,
            PickPolicy::FacesAndEdges,
        )
        .ok()
        .flatten()
        .map(|hit| (hit.kind, hit.topology_id.to_owned()))
    };
    let mut out: Vec<(egui::Pos2, String)> = Vec::new();
    for i in 0..120 {
        for j in 0..120 {
            let logical = DVec2::new(
                size.width * (0.2 + 0.6 * f64::from(i) / 120.0),
                size.height * (0.2 + 0.6 * f64::from(j) / 120.0),
            );
            let Some((hit, id)) = at(logical) else {
                continue;
            };
            // Faces must hold under a small offset so the click is not marginal.
            let stable = kind == PickKind::Edge
                || [(6.0, 0.0), (-6.0, 0.0), (0.0, 6.0), (0.0, -6.0)]
                    .iter()
                    .all(|(dx, dy)| at(logical + DVec2::new(*dx, *dy)) == Some((hit, id.clone())));
            if hit == kind && stable && !out.iter().any(|(_, known)| *known == id) {
                let pos = rect.min + egui::vec2(logical.x as f32, logical.y as f32);
                out.push((pos, id));
            }
        }
    }
    out
}

/// Explorer rows appear once the panel has laid out.
fn row(host: &mut Host, name: &str) -> egui::Pos2 {
    for _ in 0..40 {
        host.ui(vec![]);
        if host.widgets.iter().any(|(label, _)| label == name) {
            break;
        }
    }
    host.rect(name).center()
}

pub(super) fn press(pos: egui::Pos2, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers,
    }
}

/// A single click whose press carries `modifiers`, kept apart from double-clicks.
pub(super) fn click(host: &mut Host, pos: egui::Pos2, modifiers: egui::Modifiers) {
    for _ in 0..24 {
        host.ui(vec![]);
    }
    host.ui(vec![
        egui::Event::PointerMoved(pos),
        press(pos, true, modifiers),
    ]);
    host.ui(vec![press(pos, false, modifiers)]);
}

pub(super) fn labels(host: &Host) -> Vec<String> {
    host.app
        .doc
        .selection
        .items()
        .iter()
        .filter_map(|item| match item {
            SelectionItem::Face(t) | SelectionItem::Edge(t) => {
                Some(crate::model_pick::label(&t.id).to_owned())
            }
            _ => None,
        })
        .collect()
}

fn pill_label(host: &Host) -> String {
    let kind = crate::tool_registry::selection_kind(&host.app);
    crate::pill::selection_label(&host.app, kind)
        .map(|(kind, _, _)| kind)
        .unwrap_or_default()
}

/// Waits for the highlight upload that matches the current selection.
fn highlights(host: &mut Host) -> usize {
    host.ui(vec![egui::Event::PointerGone]);
    host.redraw();
    for _ in 0..200 {
        host.ui(vec![]);
        if host.app.doc.topology.settled() && !host.app.draw_dirty {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    host.app.doc.topology.uploaded
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic pointer delivery is not physical acceptance"]
async fn modifier_clicks_build_face_and_edge_sets_and_escape_clears() {
    let root = tempfile::tempdir().unwrap();
    let mut host = boxed(root.path()).await;
    let faces = targets(&host, PickKind::Face);
    let edges = targets(&host, PickKind::Edge);
    assert!(faces.len() >= 2 && edges.len() >= 2, "{faces:?} {edges:?}");
    let (shift, command) = (egui::Modifiers::SHIFT, egui::Modifiers::COMMAND);
    let (a, b) = (faces[0].clone(), faces[1].clone());

    click(&mut host, a.0, egui::Modifiers::NONE);
    assert_eq!(labels(&host), std::slice::from_ref(&a.1));
    assert_eq!(pill_label(&host), "Face");
    click(&mut host, b.0, shift);
    assert_eq!(labels(&host), [a.1.clone(), b.1.clone()]);
    assert_eq!(pill_label(&host), "2 Faces");
    assert!(highlights(&mut host) >= 2, "both faces highlighted");
    click(&mut host, a.0, command);
    assert_eq!(
        labels(&host),
        std::slice::from_ref(&b.1),
        "Cmd toggles A off"
    );
    click(&mut host, b.0, shift | command);
    assert!(host.app.doc.selection.is_empty(), "Shift+Cmd subtracts B");

    // Seed {b, a}: on `a`, Add keeps the set, Toggle drops `a`, Replace drops `b`,
    // so only the press modifiers (Shift) leave it unchanged.
    click(&mut host, b.0, egui::Modifiers::NONE);
    click(&mut host, a.0, shift);
    let seeded = [b.1.clone(), a.1.clone()];
    assert_eq!(labels(&host), seeded);
    // One batch: Shift press, modifier change, plain release. The press decides.
    for _ in 0..24 {
        host.ui(vec![]);
    }
    host.ui(vec![
        egui::Event::PointerMoved(a.0),
        press(a.0, true, shift),
        egui::Event::ModifiersChanged(command),
        press(a.0, false, egui::Modifiers::NONE),
    ]);
    assert_eq!(labels(&host), seeded, "one batch: Shift press is Add");
    // Press and release in separate frames: the stored press mode decides.
    for _ in 0..24 {
        host.ui(vec![]);
    }
    host.ui(vec![
        egui::Event::PointerMoved(a.0),
        press(a.0, true, shift),
    ]);
    host.ui(vec![
        egui::Event::ModifiersChanged(command),
        press(a.0, false, egui::Modifiers::NONE),
    ]);
    assert_eq!(labels(&host), seeded, "split frames: Shift press is Add");
    click(&mut host, edges[0].0, shift);
    click(&mut host, edges[1].0, shift);
    assert_eq!(
        labels(&host),
        [
            b.1.clone(),
            a.1.clone(),
            edges[0].1.clone(),
            edges[1].1.clone()
        ]
    );
    assert_eq!(
        pill_label(&host),
        "4 items",
        "mixed kinds offer no scoped tools"
    );
    assert!(
        highlights(&mut host) >= 4,
        "every face and edge highlighted"
    );

    host.ui(vec![key(egui::Key::Escape, egui::Modifiers::NONE)]);
    assert!(host.app.doc.selection.is_empty(), "Esc clears");
    assert_eq!(highlights(&mut host), 0);

    // A scene change drops topology picked in the old scene.
    click(&mut host, a.0, egui::Modifiers::NONE);
    let undo = host.app.doc.display.as_ref().unwrap().projection.undo_depth;
    host.app.submit(crate::bridge::Command::Undo);
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        app.doc.display.as_ref().unwrap().projection.undo_depth != undo
    })
    .await;
    assert!(host.app.doc.selection.is_empty(), "stale face dropped");
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic pointer delivery is not physical acceptance"]
async fn explorer_rows_and_viewport_share_one_selection() {
    let root = tempfile::tempdir().unwrap();
    let mut host = boxed(root.path()).await;
    let body = host.app.doc.display.as_ref().unwrap().scene.bodies()[0].body_id();
    let face = targets(&host, PickKind::Face)[0].0;
    host.app.ui.explorer_open = true;
    host.redraw();
    let row = row(&mut host, "Body 1");
    click(&mut host, row, egui::Modifiers::NONE);
    assert!(host.app.doc.whole_body_selected(body));
    click(&mut host, face, egui::Modifiers::SHIFT);
    assert_eq!(
        host.app.doc.selection.len(),
        2,
        "viewport adds to the row pick"
    );
    assert!(!host.app.doc.whole_body_selected(body));
    assert_eq!(host.app.doc.selected_body(), Some(body));
    // Cmd on the press only: the row mode comes from the press, not the release.
    for _ in 0..24 {
        host.ui(vec![]);
    }
    host.ui(vec![
        egui::Event::PointerMoved(row),
        press(row, true, egui::Modifiers::COMMAND),
    ]);
    host.ui(vec![press(row, false, egui::Modifiers::NONE)]);
    assert!(
        matches!(host.app.doc.selection.items(), [SelectionItem::Face(_)]),
        "Cmd on the row toggles the body off"
    );
    // Double-click picks the whole body, exactly like the Explorer row.
    for _ in 0..24 {
        host.ui(vec![]);
    }
    for _ in 0..2 {
        host.ui(vec![
            egui::Event::PointerMoved(face),
            press(face, true, egui::Modifiers::NONE),
        ]);
        host.ui(vec![press(face, false, egui::Modifiers::NONE)]);
    }
    assert!(matches!(
        host.app.doc.selection.items(),
        [SelectionItem::Body(id)] if *id == body
    ));
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; run with ONECAD_REQUIRE_WORKER=1 ONECAD_REQUIRE_GPU=1"]
async fn tab_switch_keeps_each_documents_selection() {
    let root = tempfile::tempdir().unwrap();
    let mut host = boxed(root.path()).await;
    let face = targets(&host, PickKind::Face)[0].clone();
    click(&mut host, face.0, egui::Modifiers::NONE);
    let first = host.app.doc.tab;
    let selected = host.app.doc.selection.clone();
    assert_eq!(selected.len(), 1);
    host.app.document_request(Request::New);
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        app.tabs.parked.len() == 1 && app.document_open() && !app.busy()
    })
    .await;
    let second = host.app.doc.tab;
    assert!(
        host.app.doc.selection.is_empty(),
        "a new document starts empty"
    );
    assert_eq!(host.app.tabs.parked[0].selection, selected);
    host.app.activate_tab(first);
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        app.doc
            .display
            .as_ref()
            .is_some_and(|display| display.prepared.lock().unwrap().is_some())
    })
    .await;
    host.redraw();
    assert_eq!(host.app.doc.selection, selected, "kept across the switch");
    host.app.activate_tab(second);
    assert!(host.app.doc.selection.is_empty());
    host.app.activate_tab(first);
    host.stop().await;
}
