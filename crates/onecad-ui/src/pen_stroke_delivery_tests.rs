//! Pencil Stroke over production Metal and worker with synthetic pointer
//! strokes (mouse-equivalent egui events); not physical Pencil acceptance.
use super::point_center_delivery_tests::support::*;
use onecad_core::sketch::{Sketch, SketchEntity};
use onecad_interaction::sketch::SketchTool;

/// Draws one stroke through sketch points, one egui frame per sample.
fn stroke(host: &mut Host, points: &[(f64, f64)], cancel: bool) {
    host.redraw();
    let screen: Vec<_> = points.iter().map(|(x, y)| host.project(*x, *y)).collect();
    host.ui(vec![
        egui::Event::PointerMoved(screen[0]),
        button(screen[0], true),
    ]);
    for position in &screen[1..] {
        host.ui(vec![egui::Event::PointerMoved(*position)]);
    }
    let last = *screen.last().unwrap();
    host.ui(vec![if cancel {
        egui::Event::PointerGone
    } else {
        button(last, false)
    }]);
}

/// A 60° arc of radius 30 about the origin: bent enough to be an arc, and its
/// chord still covers 95% of its travel, so the Line override is allowed.
fn sixty_degree_arc() -> Vec<(f64, f64)> {
    (0..=24)
        .map(|i| {
            let angle = (60.0 + 60.0 * f64::from(i) / 24.0).to_radians();
            (30.0 * angle.cos(), 30.0 * angle.sin())
        })
        .collect()
}

fn kinds(sketch: &Sketch) -> (usize, usize) {
    let lines = sketch
        .entities()
        .iter()
        .filter(|entity| matches!(entity, SketchEntity::Line { .. }))
        .count();
    let arcs = sketch
        .entities()
        .iter()
        .filter(|entity| matches!(entity, SketchEntity::Arc { .. }))
        .count();
    (lines, arcs)
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn strokes_commit_override_undo_and_refuse_without_history() {
    let area = tempfile::tempdir().unwrap();
    let mut host = Host::new(area.path()).await;
    host.control("Pencil Stroke");
    assert_eq!(
        host.app.doc.sketch.session.as_ref().unwrap().tool,
        SketchTool::PenStroke
    );
    let line: Vec<_> = (0..=20)
        .map(|i| (-20.0 + 2.0 * f64::from(i), 5.0))
        .collect();
    stroke(&mut host, &line, false);
    host.count(3).await;
    assert_eq!(
        kinds(host.sketch()),
        (1, 0),
        "a straight stroke is one line"
    );

    stroke(&mut host, &sixty_degree_arc(), false);
    host.count(5).await;
    assert_eq!(kinds(host.sketch()), (1, 1), "a bent stroke is one arc");
    host.control("As line");
    // The arc's step is undone first; the next frame commits the line.
    host.count(3).await;
    host.count(6).await;
    assert_eq!(
        kinds(host.sketch()),
        (2, 0),
        "the override swaps the last stroke"
    );
    let depth = host
        .app
        .doc
        .sketch
        .session
        .as_ref()
        .unwrap()
        .history
        .as_ref()
        .unwrap()
        .undo_depth;
    assert_eq!(depth, 2, "the override leaves one history step per stroke");
    host.control("Undo stroke");
    host.count(3).await;
    assert_eq!(
        kinds(host.sketch()),
        (1, 0),
        "undo reverts the redrawn stroke"
    );

    let zigzag: Vec<_> = (0..=12)
        .map(|i| {
            (
                -20.0 + 3.0 * f64::from(i),
                if i % 2 == 0 { -10.0 } else { -25.0 },
            )
        })
        .collect();
    stroke(&mut host, &zigzag, false);
    host.ui(vec![]);
    assert_eq!(
        host.sketch().entities().len(),
        3,
        "no history for a refusal"
    );
    assert!(host.app.doc.status.contains("Stroke not recognized"));

    stroke(&mut host, &line, true);
    host.ui(vec![]);
    assert_eq!(
        host.sketch().entities().len(),
        3,
        "a cancelled contact never commits"
    );
    assert_eq!(
        host.app.doc.sketch.session.as_ref().unwrap().tool,
        SketchTool::PenStroke,
        "the tool stays armed"
    );
    host.stop().await;
}

fn tool(host: &Host) -> SketchTool {
    host.app.doc.sketch.session.as_ref().unwrap().tool
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn accessory_previous_tool_and_adaptive_palette() {
    let area = tempfile::tempdir().unwrap();
    let mut host = Host::new(area.path()).await;
    host.control("Line");
    host.control("Pencil Stroke");
    assert!(crate::tool_palette::toggle_previous(&mut host.app));
    assert_eq!(tool(&host), SketchTool::Line);
    assert!(crate::tool_palette::toggle_previous(&mut host.app));
    assert_eq!(
        tool(&host),
        SketchTool::PenStroke,
        "a second toggle returns"
    );

    let at = egui::pos2(400.0, 300.0);
    crate::tool_palette::open(&mut host.app, at);
    host.ui(vec![]);
    host.ui(vec![]);
    // Palette rows repeat pill labels; the palette's own sit right of its anchor.
    let circle = host
        .widgets
        .iter()
        .find(|(name, rect)| name == "Circle" && rect.min.x >= at.x && rect.min.y >= at.y)
        .map(|(_, rect)| rect.center())
        .expect("the sketch palette offers Circle");
    host.click(circle);
    assert_eq!(tool(&host), SketchTool::Circle);
    assert!(host.app.ui.tool_palette.is_none(), "choosing closes it");

    crate::tool_palette::open(&mut host.app, at);
    host.ui(vec![]);
    host.ui(vec![egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    assert!(host.app.ui.tool_palette.is_none(), "Escape closes it");
    assert_eq!(tool(&host), SketchTool::Circle);
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn a_stroke_whose_release_the_canvas_missed_is_dropped() {
    let area = tempfile::tempdir().unwrap();
    let mut host = Host::new(area.path()).await;
    host.control("Pencil Stroke");
    host.redraw();
    let (start, end) = (host.project(-20.0, 5.0), host.project(20.0, 5.0));
    host.ui(vec![egui::Event::PointerMoved(start), button(start, true)]);
    host.ui(vec![egui::Event::PointerMoved(end)]);
    assert!(crate::sketch_pen_stroke::drawing(&host.context));
    // An overlay owns the pointer for the frame that carries the release.
    crate::tool_palette::open(&mut host.app, end);
    host.ui(vec![]);
    host.ui(vec![button(end, false)]);
    host.app.ui.tool_palette = None;
    host.ui(vec![egui::Event::PointerMoved(start)]);
    assert!(
        !crate::sketch_pen_stroke::drawing(&host.context),
        "hover moves must not extend a stroke whose release was missed"
    );
    host.ui(vec![]);
    assert!(host.sketch().entities().is_empty(), "nothing committed");
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn drag_authoring_follows_sketch_tools_and_canvas_handles() {
    let area = tempfile::tempdir().unwrap();
    let mut host = Host::new(area.path()).await;
    assert_eq!(tool(&host), SketchTool::Select);
    assert!(
        !crate::host::drag_authoring(&host.app),
        "a finger drag orbits in Select"
    );
    host.control("Line");
    assert!(crate::host::drag_authoring(&host.app));
    host.stop().await;

    let root = tempfile::tempdir().unwrap();
    let (mut host, _) =
        super::extrude_handle_delivery_tests::extruding(root.path(), "12", 12.0).await;
    assert!(
        crate::host::drag_authoring(&host.app),
        "the extrude distance handle takes a finger drag"
    );
    super::extrude_handle_delivery_tests::confirm(&mut host).await;
    assert!(host.app.doc.interaction.tool.is_none());
    assert!(
        !crate::host::drag_authoring(&host.app),
        "no tool: fingers orbit"
    );
    host.stop().await;
}
