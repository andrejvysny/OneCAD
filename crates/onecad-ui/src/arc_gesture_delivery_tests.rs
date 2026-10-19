//! Prepared Arc fixture, not native Arc creation; subsequent input uses the real native viewport.
use super::*;
use onecad_core::{
    ids::{EntityId, SketchId},
    math::Vec2,
    sketch::{CurvePosition, Sketch, SketchAttachment, SketchEntity, WorldPlane},
};
use onecad_interaction::sketch::{
    SketchAction, SketchTarget,
    gesture::{GestureKind, GesturePhase},
};

use super::point_center_delivery_tests::support::*;

#[derive(Clone, Copy)]
struct ArcIds {
    arc: EntityId,
    center: EntityId,
}

async fn prepared_arc(root: &std::path::Path) -> (Host, ArcIds) {
    let mut host = Host::new(root).await;
    host.app.sketch_dispatch(SketchAction::Discard);
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        !app.sketch_active()
    })
    .await;
    let ids = ArcIds {
        arc: EntityId::new(),
        center: EntityId::new(),
    };
    let mut sketch = Sketch::new(
        SketchId::new(),
        "Prepared arc gesture fixture",
        SketchAttachment::World {
            plane: WorldPlane::XY,
        },
    );
    sketch
        .add_entity(SketchEntity::point(
            ids.center,
            Vec2::new(0.0, 0.0).unwrap(),
            false,
            false,
        ))
        .unwrap();
    sketch
        .add_entity(
            SketchEntity::arc(
                ids.arc,
                ids.center,
                10.0,
                0.0,
                std::f64::consts::FRAC_PI_2,
                false,
            )
            .unwrap(),
        )
        .unwrap();
    let context = host.app.interaction.context.clone().unwrap();
    host.app.sketch_dispatch(SketchAction::Begin {
        sketch: Box::new(sketch),
        create: true,
        context,
    });
    host.count(2).await;
    host.control("Select");
    host.redraw();
    assert_arc(
        host.sketch(),
        ids,
        [0.0, 0.0],
        10.0,
        Some([0.0, std::f64::consts::FRAC_PI_2]),
    );
    (host, ids)
}

#[tokio::test]
#[ignore = "requires production worker and Metal; prepared Arc fixture does not qualify native Arc creation or physical input"]
async fn prepared_arc_center_drag_translates_circle_and_preserves_local_history() {
    let root = tempfile::tempdir().unwrap();
    let (mut host, ids) = prepared_arc(root.path()).await;
    let before = host.sketch().clone();
    let depth = undo_depth(&host);
    drag(
        &mut host,
        [0.0, 0.0],
        [5.0, 6.0],
        GestureKind::EntityBody,
        Some(CurvePosition::Center),
    );
    settled(&mut host).await;
    assert_arc(
        host.sketch(),
        ids,
        [5.0, 6.0],
        10.0,
        Some([0.0, std::f64::consts::FRAC_PI_2]),
    );
    assert_eq!(undo_depth(&host), depth + 1);
    assert_eq!(host.app.sketch_gesture.moved(), Some(true));
    let translated = host.sketch().clone();
    history(&mut host, false, &before).await;
    history(&mut host, true, &translated).await;
    let depth = undo_depth(&host);
    begin_drag(
        &mut host,
        [5.0, 6.0],
        [8.0, 9.0],
        GestureKind::EntityBody,
        Some(CurvePosition::Center),
    );
    host.ui(vec![key(egui::Key::Escape, egui::Modifiers::NONE)]);
    host.ui(vec![button(host.project(8.0, 9.0), false)]);
    settled(&mut host).await;
    assert_eq!(
        host.sketch(),
        &translated,
        "cancelled Arc center drag must preserve accepted geometry and IDs"
    );
    assert_eq!(undo_depth(&host), depth);
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; prepared Arc fixture does not qualify native Arc creation or physical input"]
async fn prepared_arc_endpoint_drag_preserves_roles_and_cancel_keeps_authority() {
    let root = tempfile::tempdir().unwrap();
    let (mut host, ids) = prepared_arc(root.path()).await;
    let before = host.sketch().clone();
    let depth = undo_depth(&host);
    drag(
        &mut host,
        [10.0, 0.0],
        [12.0, 4.0],
        GestureKind::ArcEnd,
        Some(CurvePosition::Start),
    );
    settled(&mut host).await;
    let (radius, start, end) = parameters(host.sketch(), ids);
    assert_arc(host.sketch(), ids, [0.0, 0.0], 12.0_f64.hypot(4.0), None);
    near_angle(start, 4.0_f64.atan2(12.0));
    assert!(
        (end - std::f64::consts::FRAC_PI_2).abs() < 0.05,
        "free sibling endpoint may slide on its owning circle"
    );
    assert_endpoint(host.sketch(), ids, CurvePosition::Start, [12.0, 4.0]);
    let start_state = host.sketch().clone();
    history(&mut host, false, &before).await;
    history(&mut host, true, &start_state).await;
    let endpoint = [radius * end.cos(), radius * end.sin()];
    drag(
        &mut host,
        endpoint,
        [-4.0, 14.0],
        GestureKind::ArcEnd,
        Some(CurvePosition::End),
    );
    settled(&mut host).await;
    let (radius, next_start, next_end) = parameters(host.sketch(), ids);
    assert_arc(host.sketch(), ids, [0.0, 0.0], 4.0_f64.hypot(14.0), None);
    near_angle(next_end, 14.0_f64.atan2(-4.0));
    assert!((next_start - start).abs() < 0.05);
    assert_endpoint(host.sketch(), ids, CurvePosition::End, [-4.0, 14.0]);
    let angle = (next_start + next_end) / 2.0;
    drag(
        &mut host,
        [radius * angle.cos(), radius * angle.sin()],
        [18.0 * angle.cos(), 18.0 * angle.sin()],
        GestureKind::Radius,
        None,
    );
    settled(&mut host).await;
    assert_arc(host.sketch(), ids, [0.0, 0.0], 18.0, None);
    assert_eq!(undo_depth(&host), depth + 3);
    let accepted = host.sketch().clone();
    let (radius, start, end) = parameters(&accepted, ids);
    let endpoint = [radius * end.cos(), radius * end.sin()];
    begin_drag(
        &mut host,
        endpoint,
        [-10.0, 20.0],
        GestureKind::ArcEnd,
        Some(CurvePosition::End),
    );
    host.ui(vec![key(egui::Key::Escape, egui::Modifiers::NONE)]);
    host.ui(vec![button(host.project(-10.0, 20.0), false)]);
    settled(&mut host).await;
    assert_eq!(host.sketch(), &accepted);
    assert_eq!(undo_depth(&host), depth + 3);
    near_angle(parameters(host.sketch(), ids).1, start);
    host.stop().await;
}

fn begin_drag(
    host: &mut Host,
    from: [f64; 2],
    to: [f64; 2],
    kind: GestureKind,
    role: Option<CurvePosition>,
) {
    host.redraw();
    let start = host.project(from[0], from[1]);
    let end = host.project(to[0], to[1]);
    host.ui(vec![egui::Event::PointerMoved(start), button(start, true)]);
    host.ui(vec![egui::Event::PointerMoved(end)]);
    assert!(
        host.app.sketch_gesture.busy(),
        "gesture did not start: {} / {:?}",
        host.app.status,
        host.app.sketch_gesture.error()
    );
    let intent = host.app.sketch_gesture.intent().unwrap();
    assert_eq!(intent.kind, kind);
    assert_eq!(intent.role, role);
    if let Some(role) = role {
        assert!(
            matches!(intent.target, SketchTarget::Point { role: actual, .. } if actual == role)
        );
    }
}

fn drag(
    host: &mut Host,
    from: [f64; 2],
    to: [f64; 2],
    kind: GestureKind,
    role: Option<CurvePosition>,
) {
    begin_drag(host, from, to, kind, role);
    assert_eq!(
        host.app.sketch_gesture.phase(),
        Some(GesturePhase::PendingBegin),
        "this early release intentionally precedes worker readiness"
    );
    host.ui(vec![button(host.project(to[0], to[1]), false)]);
}

async fn settled(host: &mut Host) {
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        !app.sketch_gesture.busy() && active(app) && app.bridge.valid()
    })
    .await;
    assert!(
        host.app.sketch_gesture.error().is_none(),
        "{} / {:?}",
        host.app.status,
        host.app.sketch_gesture.error()
    );
    host.redraw();
}

fn undo_depth(host: &Host) -> usize {
    host.app
        .sketch
        .session
        .as_ref()
        .unwrap()
        .history
        .as_ref()
        .unwrap()
        .undo_depth
}

async fn history(host: &mut Host, redo: bool, expected: &Sketch) {
    let modifiers = egui::Modifiers {
        command: true,
        mac_cmd: true,
        shift: redo,
        ..Default::default()
    };
    host.ui(vec![
        egui::Event::ModifiersChanged(modifiers),
        key(egui::Key::Z, modifiers),
    ]);
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        active(app)
            && app
                .sketch
                .session
                .as_ref()
                .unwrap()
                .authoritative
                .entities()
                == expected.entities()
    })
    .await;
    host.ui(vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)]);
    assert_eq!(host.sketch().constraints(), expected.constraints());
}

fn parameters(sketch: &Sketch, ids: ArcIds) -> (f64, f64, f64) {
    let SketchEntity::Arc {
        center,
        radius,
        start_angle,
        end_angle,
        ..
    } = sketch.get_entity(ids.arc).unwrap()
    else {
        panic!("Arc expected")
    };
    assert_eq!(*center, ids.center);
    (*radius, *start_angle, *end_angle)
}

fn assert_arc(
    sketch: &Sketch,
    ids: ArcIds,
    center: [f64; 2],
    expected_radius: f64,
    angles: Option<[f64; 2]>,
) {
    assert_eq!(
        sketch.entities().len(),
        2,
        "analytic Arc must not gain material endpoint entities"
    );
    assert!(sketch.constraints().is_empty());
    let SketchEntity::Point { at, .. } = sketch.get_entity(ids.center).unwrap() else {
        panic!("center Point expected")
    };
    near_position(at.x, center[0]);
    near_position(at.y, center[1]);
    let (radius, start, end) = parameters(sketch, ids);
    near_position(radius, expected_radius);
    assert!(radius > 0.0 && start.is_finite() && end.is_finite() && end > start);
    if let Some([expected_start, expected_end]) = angles {
        near_angle(start, expected_start);
        near_angle(end, expected_end);
    }
    for role in [CurvePosition::Start, CurvePosition::End] {
        let endpoint = SketchTarget::Point {
            entity: ids.arc,
            role,
        }
        .point_coordinates(sketch)
        .unwrap();
        near_position((endpoint.x - at.x).hypot(endpoint.y - at.y), radius);
    }
}

fn assert_endpoint(sketch: &Sketch, ids: ArcIds, role: CurvePosition, expected: [f64; 2]) {
    let at = SketchTarget::Point {
        entity: ids.arc,
        role,
    }
    .point_coordinates(sketch)
    .unwrap();
    near_position(at.x, expected[0]);
    near_position(at.y, expected[1]);
}

fn near_position(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-4, "{actual} != {expected}");
}
fn near_angle(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1e-4,
        "angle {actual} != {expected}"
    );
}

#[path = "arc_gesture_delivery_tests/failure.rs"]
mod failure;

#[tokio::test]
#[ignore = "requires production worker and Metal; prepared Arc fixture does not qualify native Arc creation or physical input"]
async fn prepared_arc_accepted_end_capture_failure_preserves_history_and_blocks_stale_authoring() {
    let root = tempfile::tempdir().unwrap();
    let (host, ids) = prepared_arc(root.path()).await;
    failure::accepted_capture_failure(host, ids).await;
}
