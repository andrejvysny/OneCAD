use super::super::document_tabs_tests::two_tabs;
use super::*;

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn selection_does_not_cross_sketch_visits_or_document_tabs() {
    let area = tempfile::tempdir().unwrap();
    let (mut app, context, frame) = two_tabs(area.path()).await;
    context.enable_accesskit();
    let (other, owner) = (app.tabs.parked[0].tab, app.doc.tab);
    app.new_sketch();
    let mut host = Host {
        app,
        context,
        frame,
        size: egui::vec2(1280.0, 820.0),
        widgets: vec![],
    };
    open(&mut host).await;
    host.control("Rectangle");
    host.xy(0.0, 0.0);
    host.xy(40.0, 30.0);
    host.count(8).await;
    host.control("Select");
    host.redraw();
    host.click(host.project(20.0, 0.0));
    assert!(matches!(selection(&host)[..], [SketchTarget::Entity(_)]));
    let sketch = host.sketch().id;
    let first_visit = host.app.doc.sketch.session.as_ref().unwrap().key.clone();
    host.control("Finish sketch");
    wait_closed(&mut host).await;
    let closed = host.app.doc.sketch.session.as_ref().unwrap();
    assert_eq!(closed.phase, SketchPhase::Closed);
    assert!(closed.selection.is_empty(), "Finish clears the visit");

    host.app.new_sketch();
    open(&mut host).await;
    assert!(selection(&host).is_empty(), "sketch B starts unselected");
    host.control("Line");
    host.xy(-20.0, -10.0);
    host.xy(-5.0, -10.0);
    host.count(3).await;
    host.ui(vec![key(egui::Key::Escape, egui::Modifiers::NONE)]);
    host.control("Select");
    host.redraw();
    host.click(host.project(-12.5, -10.0));
    assert!(matches!(selection(&host)[..], [SketchTarget::Entity(_)]));
    host.control("Discard sketch edits");
    wait_closed(&mut host).await;
    assert_eq!(
        host.app
            .doc
            .display
            .as_ref()
            .unwrap()
            .projection
            .sketches
            .len(),
        1,
        "discarding the created visit removes sketch B"
    );

    host.app.edit_sketch(sketch);
    open(&mut host).await;
    let visit = host.app.doc.sketch.session.as_ref().unwrap().key.clone();
    assert_eq!(visit.sketch_id, sketch);
    assert_ne!(visit, first_visit, "re-entry is a new visit");
    assert!(selection(&host).is_empty(), "A→B→A keeps no stale targets");
    host.redraw();
    host.click(host.project(0.0, 0.0));
    let retained = selection(&host);
    assert!(matches!(
        retained[..],
        [SketchTarget::Point {
            role: CurvePosition::Arbitrary,
            ..
        }]
    ));

    host.app.activate_tab(other);
    assert!(host.app.doc.sketch.session.is_none(), "{:?}", other);
    host.redraw();
    host.app.new_sketch();
    open(&mut host).await;
    let foreign = host.app.doc.sketch.session.as_ref().unwrap();
    assert_ne!(foreign.key.sketch_id, sketch);
    assert!(foreign.selection.is_empty(), "no targets leak across tabs");

    host.app.activate_tab(owner);
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        app.doc
            .display
            .as_ref()
            .is_some_and(|display| display.prepared.lock().unwrap().is_some())
    })
    .await;
    host.redraw();
    let session = host.app.doc.sketch.session.as_ref().unwrap();
    assert_eq!(session.key, visit);
    assert_eq!(session.selection, retained, "parked visit keeps its own");
    assert!(
        retained
            .iter()
            .all(|target| target.is_valid(&session.authoritative))
    );
    host.click(host.project(20.0, 30.0));
    assert_eq!(
        selection(&host),
        vec![SketchTarget::Entity(top_edge(&host))],
        "picks after return use the owner's fresh capture"
    );
    host.stop().await;
}

/// The rectangle's top edge: the one line whose midpoint is (20, 30), within
/// the pixel quantisation of the synthetic clicks that placed its corners.
fn top_edge(host: &Host) -> EntityId {
    let sketch = host.sketch();
    let at = |id| match sketch.get_entity(id) {
        Some(SketchEntity::Point { at, .. }) => *at,
        _ => panic!("line endpoint"),
    };
    let edges: Vec<_> = sketch
        .entities()
        .iter()
        .filter_map(|entity| match entity {
            SketchEntity::Line { start, end, .. } => {
                let (a, b) = (at(*start), at(*end));
                let mid = ((a.x + b.x) * 0.5, (a.y + b.y) * 0.5);
                ((mid.0 - 20.0).abs() < 1e-3 && (mid.1 - 30.0).abs() < 1e-3).then(|| entity.id())
            }
            _ => None,
        })
        .collect();
    assert_eq!(edges.len(), 1, "exactly one top edge");
    edges[0]
}
