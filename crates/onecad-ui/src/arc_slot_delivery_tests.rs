//! Native authoring input over production Metal and worker; not physical acceptance.
use super::point_center_delivery_tests::support::*;
use onecad_core::{
    ids::EntityId,
    sketch::{Constraint, CurvePosition, Sketch, SketchEntity},
};
use onecad_interaction::sketch::{SketchTarget, SketchTool};

#[path = "arc_slot_delivery_tests/support.rs"]
mod support;
use support::*;

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn numeric_center_and_three_point_arcs_preview_history_and_refusal() {
    let area = tempfile::tempdir().unwrap();
    let mut host = Host::new(area.path()).await;
    host.control("Arc");
    host.xy(-25.0, 0.0);
    host.xy(-15.0, 0.0);
    pointer(&mut host, -25.0, 10.0);
    let depth = undo_depth(&host);
    host.numeric("4 + 4", "-90");
    host.count(2).await;
    assert_eq!(undo_depth(&host), depth + 1);
    assert_radius(host.sketch(), 8.0, true);
    assert_native_arc_endpoint_seeding(&mut host);
    let accepted = host.sketch().clone();
    host.history(false, 0).await;
    host.history(true, 2).await;
    assert_eq!(host.sketch(), &accepted);
    host.ui(vec![key(egui::Key::A, egui::Modifiers::SHIFT)]);
    assert_eq!(
        host.app.doc.sketch.session.as_ref().unwrap().tool,
        SketchTool::ThreePointArc
    );
    host.xy(5.0, 0.0);
    host.xy(25.0, 0.0);
    pointer(&mut host, 15.0, 8.0);
    refusal(&mut host, "9");
    host.xy(5.0, 0.0);
    host.xy(25.0, 0.0);
    pointer(&mut host, 15.0, 8.0);
    let draft = host
        .app
        .doc
        .sketch
        .session
        .as_ref()
        .unwrap()
        .draft
        .as_ref()
        .unwrap();
    let end = draft.secondary.as_ref().unwrap().point;
    let radius = (end.x - draft.anchor.x).hypot(end.y - draft.anchor.y) * 0.5;
    numeric_one(&mut host, &radius.to_string());
    host.count(4).await;
    assert_eq!(undo_depth(&host), depth + 2);
    assert_radius(host.sketch(), 10.0, false);
    let arc = latest_arc(host.sketch());
    let a = endpoint(host.sketch(), arc, CurvePosition::Start);
    let b = endpoint(host.sketch(), arc, CurvePosition::End);
    near(a.y, 0.0);
    near(b.y, 0.0);
    near((a.x - b.x).abs(), 20.0);
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn numeric_slot_full_width_expression_construction_and_local_history() {
    let area = tempfile::tempdir().unwrap();
    let mut host = Host::new(area.path()).await;
    host.control("Slot");
    host.xy(-20.0, 0.0);
    host.xy(20.0, 0.0);
    pointer(&mut host, 0.0, 4.0);
    let depth = undo_depth(&host);
    numeric_one(&mut host, "3 * 2");
    host.count(10).await;
    assert_eq!(undo_depth(&host), depth + 1);
    assert!(
        host.sketch()
            .constraints()
            .iter()
            .any(|constraint| matches!(
                constraint, Constraint::Diameter { value, .. }
                if (value.value - 6.0).abs() < 1e-5 && value.expr.is_some()
            ))
    );
    assert_eq!(host.app.doc.display.as_ref().unwrap().profiles.len(), 1);
    let accepted = host.sketch().clone();
    host.history(false, 0).await;
    host.history(true, 10).await;
    assert_eq!(host.sketch(), &accepted);
    host.app.ui.inspector_open = true;
    host.control("Construction geometry");
    host.app.ui.inspector_open = false;
    host.xy(-15.0, 18.0);
    host.xy(15.0, 18.0);
    pointer(&mut host, 0.0, 21.0);
    numeric_one(&mut host, "4");
    host.count(20).await;
    assert!(
        host.sketch()
            .entities()
            .iter()
            .filter(|entity| !accepted.contains_entity(entity.id()))
            .all(SketchEntity::is_construction)
    );
    assert_eq!(host.app.doc.display.as_ref().unwrap().profiles.len(), 1);
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn tangent_arc_clockwise_counterclockwise_native_hv_snap_roles() {
    let area = tempfile::tempdir().unwrap();
    let mut host = Host::new(area.path()).await;
    for (sign, expected_role, vertical) in [
        (1.0, CurvePosition::End, true),
        (-1.0, CurvePosition::Start, false),
    ] {
        host.control("Line");
        let count = host.sketch().entities().len() + 3;
        host.xy(-25.0, sign * 15.0);
        host.xy(-5.0, sign * 15.0);
        host.count(count).await;
        host.ui(vec![key(egui::Key::A, egui::Modifiers::NONE)]);
        assert!(tangent_mode(&host));
        host.xy(5.0, sign * 25.0);
        host.count(count + 2).await;
        let curve = host
            .app
            .doc
            .sketch
            .session
            .as_ref()
            .unwrap()
            .draft
            .as_ref()
            .unwrap()
            .preceding
            .unwrap();
        assert_eq!(curve.role, expected_role);
        assert_eq!(
            host.sketch()
                .entities()
                .iter()
                .filter(|entity| matches!(entity, SketchEntity::Point { .. }))
                .count(),
            if sign > 0.0 { 3 } else { 8 }
        );
        let at = endpoint(host.sketch(), curve.entity, curve.role);
        assert!(!tangent_mode(&host));
        host.app.ui.sketch_snap = true;
        let before = host.sketch().entities().len();
        host.xy(
            at.x + if vertical { 0.0 } else { 12.0 },
            at.y + if vertical { 12.0 } else { 0.0 },
        );
        host.count(before + 3).await;
        assert_alignment(host.sketch(), curve.entity, curve.role, vertical);
        host.ui(vec![key(egui::Key::Escape, egui::Modifiers::NONE)]);
        host.app.ui.sketch_snap = false;
    }
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn tangent_chain_keyboard_drag_seed_cancel_and_numeric_radius() {
    let area = tempfile::tempdir().unwrap();
    let mut host = Host::new(area.path()).await;
    host.control("Line");
    host.xy(-30.0, -15.0);
    host.xy(-10.0, -15.0);
    host.count(3).await;
    let before = host.sketch().clone();
    assert_ignored_keys(&mut host);
    assert_numeric_keyboard_ownership(&mut host);
    host.app.ui.sketch_snap = true;
    let line = before
        .entities()
        .iter()
        .find(|entity| matches!(entity, SketchEntity::Line { .. }))
        .unwrap()
        .id();
    let end = endpoint(host.sketch(), line, CurvePosition::End);
    host.xy(end.x, end.y);
    assert!(
        host.app
            .doc
            .sketch
            .session
            .as_ref()
            .unwrap()
            .can_tangent_arc()
    );
    let to = host.project(end.x + 10.0, end.y + 10.0);
    let from = host.project(end.x, end.y);
    host.ui(vec![egui::Event::PointerMoved(from), button(from, true)]);
    host.ui(vec![egui::Event::PointerMoved(to)]);
    assert!(tangent_mode(&host));
    host.ui(vec![key(egui::Key::Escape, egui::Modifiers::NONE)]);
    host.ui(vec![button(to, false)]);
    assert_eq!(host.sketch(), &before);
    assert_eq!(undo_depth(&host), 1);
    host.xy(end.x, end.y);
    host.ui(vec![key(egui::Key::A, egui::Modifiers::NONE)]);
    pointer(&mut host, end.x + 10.0, end.y + 10.0);
    numeric_one(&mut host, "4 + 4");
    host.count(5).await;
    assert_radius(host.sketch(), 8.0, true);
    assert_eq!(undo_depth(&host), 2);
    arc_to_arc_and_stale_capture(&mut host).await;
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn tangent_chain_drag_release_once_and_accepted_closure_profile() {
    let area = tempfile::tempdir().unwrap();
    let mut host = Host::new(area.path()).await;
    host.control("Line");
    host.xy(-10.0, -10.0);
    host.xy(10.0, -10.0);
    host.count(3).await;
    let anchor = host
        .app
        .doc
        .sketch
        .session
        .as_ref()
        .unwrap()
        .draft
        .as_ref()
        .unwrap()
        .anchor;
    let from = host.project(anchor.x, anchor.y);
    let to = host.project(anchor.x, anchor.y + 20.0);
    let depth = undo_depth(&host);
    host.ui(vec![egui::Event::PointerMoved(from), button(from, true)]);
    host.ui(vec![egui::Event::PointerMoved(to)]);
    assert!(tangent_mode(&host));
    host.ui(vec![button(to, false)]);
    host.count(5).await;
    host.ui(vec![button(to, false)]);
    assert_eq!(undo_depth(&host), depth + 1);
    assert!(!tangent_mode(&host));
    let end = host
        .app
        .doc
        .sketch
        .session
        .as_ref()
        .unwrap()
        .draft
        .as_ref()
        .unwrap()
        .anchor;
    host.xy(end.x - 20.0, end.y);
    host.count(8).await;
    host.app.ui.sketch_snap = true;
    let draft = host
        .app
        .doc
        .sketch
        .session
        .as_ref()
        .unwrap()
        .draft
        .as_ref()
        .unwrap();
    let first = draft.first;
    let first_id = draft.first_id.unwrap();
    host.ui(vec![key(egui::Key::A, egui::Modifiers::NONE)]);
    host.xy(first.x, first.y);
    host.count(10).await;
    assert!(
        host.app
            .doc
            .sketch
            .session
            .as_ref()
            .unwrap()
            .draft
            .is_none()
    );
    assert_eq!(undo_depth(&host), depth + 3);
    let arc = latest_arc(host.sketch());
    assert_eq!(host.sketch().constraints().iter().filter(|constraint| matches!(constraint,
        Constraint::Coincident { point1, point1_position: CurvePosition::End, point2, point2_position: CurvePosition::Arbitrary, .. }
        if *point1 == arc && *point2 == first_id)).count(), 1);
    assert_eq!(host.app.doc.display.as_ref().unwrap().profiles.len(), 1);
    host.stop().await;
    // Nested fixture futures otherwise exhaust the debug test stack during shader validation.
    Box::pin(batched_drag_release(
        &area.path().join("batched-release"),
        false,
    ))
    .await;
    Box::pin(batched_drag_release(
        &area.path().join("single-frame"),
        true,
    ))
    .await;
    Box::pin(cancelled_drag_ownership(
        &area.path().join("cancel-ownership"),
    ))
    .await;
}
