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
