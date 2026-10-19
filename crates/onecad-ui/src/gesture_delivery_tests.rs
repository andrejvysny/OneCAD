//! Synthetic native viewport input; physical qualification remains a separate gate.
use super::*;
use onecad_core::{
    math::Vec2,
    sketch::{Sketch, SketchEntity},
};
use onecad_interaction::sketch::SketchStatus;
use onecad_scene::DVec3;
use std::sync::Arc;

use super::point_center_delivery_tests::support::*;

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn viewport_corner_drag_early_release_commits_one_edit_and_local_history() {
    let root = tempfile::tempdir().unwrap();
    let mut host = Host::new(root.path()).await;
    rectangle(&mut host).await;
    let before = host.sketch().clone();
    let undo = host
        .app
        .sketch
        .session
        .as_ref()
        .unwrap()
        .history
        .as_ref()
        .unwrap()
        .undo_depth;
    drag(&mut host, [0.0, 0.0], [22.0, 21.0]);
    settled(&mut host).await;
    eprintln!(
        "corner gesture status={} error={:?} moved={:?} entities={:?}",
        host.app.status,
        host.app.sketch_gesture.error(),
        host.app.sketch_gesture.moved(),
        host.sketch().entities()
    );
    assert_rectangle(host.sketch(), [22.0, 21.0]);
    let session = host.app.sketch.session.as_ref().unwrap();
    assert_eq!(host.app.sketch_gesture.moved(), Some(true));
    assert_eq!(session.history.as_ref().unwrap().undo_depth, undo + 1);
    match &session.status {
        SketchStatus::Entered(solve) => assert_eq!(solve.dof, 4),
        SketchStatus::Solved(solve) => assert_eq!(solve.dof, 4),
        status => panic!("missing released solver truth: {status:?}"),
    }
    let moved = host.sketch().clone();
    history(&mut host, false, &before).await;
    history(&mut host, true, &moved).await;
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn viewport_radius_drag_and_escape_preserve_curve_identity_and_authority() {
    let root = tempfile::tempdir().unwrap();
    let mut host = Host::new(root.path()).await;
    host.control("Circle");
    host.xy(0.0, 0.0);
    host.xy(10.0, 0.0);
    host.count(2).await;
    host.control("Select");
    host.redraw();
    let before = host.sketch().clone();
    let circle = before
        .entities()
        .iter()
        .find_map(|entity| match entity {
            SketchEntity::Circle { id, .. } => Some(*id),
            _ => None,
        })
        .unwrap();
    drag(&mut host, [10.0, 0.0], [15.0, 0.0]);
    settled(&mut host).await;
    let accepted = host.sketch().clone();
    eprintln!(
        "radius gesture status={} error={:?} moved={:?} entities={:?}",
        host.app.status,
        host.app.sketch_gesture.error(),
        host.app.sketch_gesture.moved(),
        accepted.entities()
    );
    assert!(
        matches!(accepted.get_entity(circle), Some(SketchEntity::Circle { radius, .. }) if (*radius-15.0).abs()<1e-5)
    );
    assert_eq!(accepted.entities().len(), before.entities().len());
    let start = host.project(15.0, 0.0);
    host.ui(vec![egui::Event::PointerMoved(start), button(start, true)]);
    host.ui(vec![egui::Event::PointerMoved(host.project(20.0, 0.0))]);
    assert!(host.app.sketch_gesture.busy(), "{}", host.app.status);
    host.ui(vec![key(egui::Key::Escape, egui::Modifiers::NONE)]);
    host.ui(vec![button(host.project(20.0, 0.0), false)]);
    settled(&mut host).await;
    assert_eq!(host.sketch(), &accepted);
    history(&mut host, false, &before).await;
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn viewport_line_body_drag_and_camera_change_cancel_without_edit() {
    let root = tempfile::tempdir().unwrap();
    let mut host = Host::new(root.path()).await;
    rectangle(&mut host).await;
    drag(&mut host, [40.0, 0.0], [45.0, 5.0]);
    settled(&mut host).await;
    for expected in [[5.0, 5.0], [85.0, 5.0], [85.0, 60.0], [5.0, 60.0]] {
        assert_point(host.sketch(), expected);
    }
    let accepted = host.sketch().clone();
    let history = host.app.sketch.session.as_ref().unwrap().history.clone();
    let start = host.project(5.0, 5.0);
    host.ui(vec![egui::Event::PointerMoved(start), button(start, true)]);
    host.ui(vec![egui::Event::PointerMoved(host.project(10.0, 10.0))]);
    assert!(host.app.sketch_gesture.busy());
    host.app.camera.target.x += 1.0;
    host.redraw();
    host.ui(vec![button(host.project(10.0, 10.0), false)]);
    settled(&mut host).await;
    assert_eq!(host.sketch(), &accepted);
    assert_eq!(host.app.sketch.session.as_ref().unwrap().history, history);
    assert!(!host.app.sketch.session.as_ref().unwrap().selection_dragging);
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn viewport_camera_change_before_drag_threshold_rejects_stale_start() {
    let root = tempfile::tempdir().unwrap();
    let mut host = Host::new(root.path()).await;
    rectangle(&mut host).await;
    let accepted = host.sketch().clone();
    let start = host.project(0.0, 0.0);
    host.ui(vec![egui::Event::PointerMoved(start), button(start, true)]);
    host.app.camera.target.x += 1.0;
    host.redraw();
    let end = host.project(22.0, 21.0);
    host.ui(vec![egui::Event::PointerMoved(end)]);
    host.ui(vec![button(end, false)]);
    assert!(!host.app.sketch_gesture.busy());
    assert!(!host.app.sketch.session.as_ref().unwrap().selection_dragging);
    assert_eq!(host.sketch(), &accepted);
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn viewport_escape_before_drag_threshold_cancels_only_pointer_owner() {
    let root = tempfile::tempdir().unwrap();
    let mut host = Host::new(root.path()).await;
    rectangle(&mut host).await;
    let accepted = host.sketch().clone();
    let history = host.app.sketch.session.as_ref().unwrap().history.clone();
    let start = host.project(0.0, 0.0);
    host.ui(vec![egui::Event::PointerMoved(start), button(start, true)]);
    assert!(host.app.armed_sketch_gesture.is_some());
    host.ui(vec![key(egui::Key::Escape, egui::Modifiers::NONE)]);
    host.ui(vec![button(start, false)]);
    assert!(active(&host.app));
    assert!(!host.app.sketch_gesture.busy());
    assert_eq!(host.sketch(), &accepted);
    assert_eq!(host.app.sketch.session.as_ref().unwrap().history, history);
    assert!(
        host.app
            .sketch
            .session
            .as_ref()
            .unwrap()
            .selection
            .is_empty()
    );
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn stationary_pointer_release_completes_on_service_repaint_wakes() {
    let root = tempfile::tempdir().unwrap();
    let mut host = Host::new(root.path()).await;
    rectangle(&mut host).await;
    let wakes = Arc::new(tokio::sync::Notify::new());
    let native_wakes = wakes.clone();
    host.context.set_repaint_observer(move |_| {
        if std::thread::current().name() == Some("onecad-native-services") {
            native_wakes.notify_one();
        }
    });
    drag(&mut host, [0.0, 0.0], [22.0, 21.0]);
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            wakes.notified().await;
            logic_tick(&mut host.app, &host.context, &mut host.frame, false);
            if !host.app.sketch_gesture.busy() && active(&host.app) {
                break;
            }
        }
    })
    .await
    .expect("stationary gesture must wake reactive native logic without more pointer input");
    assert_rectangle(host.sketch(), [22.0, 21.0]);
    assert_eq!(host.app.sketch_gesture.moved(), Some(true));
    host.stop().await;
}

async fn rectangle(host: &mut Host) {
    let plane = host.app.sketch.session.as_ref().unwrap().plane;
    let center = plane.to_world(Vec2::new(40.0, 30.0).unwrap());
    host.app.camera.target = DVec3::new(center.x, center.y, center.z);
    host.app.camera.offset = DVec3::new(plane.normal.x, plane.normal.y, plane.normal.z) * 180.0;
    host.redraw();
    host.control("Rectangle");
    host.xy(0.0, 0.0);
    host.xy(80.0, 60.0);
    host.count(8).await;
    host.control("Select");
    host.redraw();
}

fn drag(host: &mut Host, from: [f64; 2], to: [f64; 2]) {
    let start = host.project(from[0], from[1]);
    let end = host.project(to[0], to[1]);
    host.ui(vec![egui::Event::PointerMoved(start), button(start, true)]);
    host.ui(vec![egui::Event::PointerMoved(end)]);
    assert!(host.app.sketch_gesture.busy(), "{}", host.app.status);
    host.ui(vec![button(end, false)]);
}

async fn settled(host: &mut Host) {
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        !app.sketch_gesture.busy() && active(app) && app.bridge.valid()
    })
    .await;
    host.redraw();
}

async fn history(host: &mut Host, redo: bool, expected: &Sketch) {
    let command = egui::Modifiers {
        command: true,
        mac_cmd: true,
        shift: redo,
        ..Default::default()
    };
    host.ui(vec![
        egui::Event::ModifiersChanged(command),
        key(egui::Key::Z, command),
    ]);
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        active(app)
            && app
                .sketch
                .session
                .as_ref()
                .is_some_and(|session| session.authoritative.entities() == expected.entities())
    })
    .await;
    host.ui(vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)]);
    assert_eq!(host.sketch().constraints(), expected.constraints());
}

fn assert_rectangle(sketch: &Sketch, first: [f64; 2]) {
    for expected in [
        [first[0], first[1]],
        [80.0, first[1]],
        [80.0, 60.0],
        [first[0], 60.0],
    ] {
        assert_point(sketch, expected);
    }
}

fn assert_point(sketch: &Sketch, expected: [f64; 2]) {
    // Native logical positions round through f32 before the plane projection.
    assert!(sketch.entities().iter().any(|entity| matches!(entity, SketchEntity::Point {at,..} if (at.x-expected[0]).abs()<1e-4 && (at.y-expected[1]).abs()<1e-4)), "missing point {expected:?}");
}
