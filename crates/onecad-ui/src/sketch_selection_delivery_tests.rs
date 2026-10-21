//! Typed sketch selection through the native viewport over production worker and Metal.
//! Synthetic host events; physical mouse/keyboard acceptance remains ONECAD-275.
use super::point_center_delivery_tests::support::*;
use super::*;
use onecad_core::{
    ids::EntityId,
    math::Vec2,
    sketch::{Constraint, CurvePosition, SketchEntity},
};
use onecad_interaction::sketch::{SketchPhase, SketchTarget};
use onecad_scene::{Camera, DVec3, Projection};

#[path = "sketch_selection_delivery_tests/tabs.rs"]
mod tabs;

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn arc_roles_line_ends_symmetric_axis_marquee_and_undo_prune() {
    let area = tempfile::tempdir().unwrap();
    let mut host = Host::new(area.path()).await;
    host.control("Arc");
    host.xy(0.0, 0.0);
    host.xy(10.0, 0.0);
    host.xy(0.0, 10.0);
    host.count(2).await;
    let arc = only(&host, |entity| matches!(entity, SketchEntity::Arc { .. }));
    let first = line(&mut host, [-30.0, 30.0], [-15.0, 30.0]).await;
    let second = line(&mut host, [15.0, 30.0], [30.0, 30.0]).await;
    let axis = line(&mut host, [0.0, 20.0], [0.0, 45.0]).await;
    host.control("Select");

    let roles = [
        point(arc, CurvePosition::Start),
        point(arc, CurvePosition::End),
        point(arc, CurvePosition::Center),
    ];
    for target in roles {
        let at = role(&host, target);
        host.redraw();
        host.click(at);
        assert_eq!(selection(&host), vec![target], "{target:?}");
    }
    let body = arc_body(&host, arc);
    host.redraw();
    host.click(body);
    assert_eq!(selection(&host), vec![SketchTarget::Entity(arc)]);
    host.ui(vec![key(egui::Key::Escape, egui::Modifiers::NONE)]);
    assert!(selection(&host).is_empty(), "Escape clears selection");

    let end = point(first, CurvePosition::End);
    let start = point(second, CurvePosition::Start);
    host.redraw();
    host.click(role(&host, end));
    let at = role(&host, start);
    shift_click(&mut host, at);
    assert_eq!(selection(&host), vec![end, start], "Shift adds");
    shift_click(&mut host, at);
    assert_eq!(selection(&host), vec![end], "Shift toggles off");
    shift_click(&mut host, at);
    assert_eq!(selection(&host), vec![end, start], "Shift toggles on");
    let at = host.project(0.0, 32.0);
    shift_click(&mut host, at);
    assert_eq!(
        selection(&host),
        vec![end, start, SketchTarget::Entity(axis)]
    );
    symmetric(&mut host, first, second, axis).await;

    host.redraw();
    let (a, b) = (host.project(-6.0, 28.0), host.project(6.0, 37.0));
    let (left, right) = if a.x < b.x { (a, b) } else { (b, a) };
    drag(&mut host, left, right);
    assert!(selection(&host).is_empty(), "window must enclose the axis");
    drag(&mut host, right, left);
    assert_eq!(selection(&host), vec![SketchTarget::Entity(axis)]);

    let count = host.sketch().entities().len();
    let extra = line(&mut host, [-30.0, -20.0], [-15.0, -20.0]).await;
    host.control("Select");
    host.redraw();
    host.click(host.project(-22.5, -20.0));
    let at = role(&host, roles[0]);
    shift_click(&mut host, at);
    assert_eq!(
        selection(&host),
        vec![SketchTarget::Entity(extra), roles[0]]
    );
    host.history(false, count).await;
    assert_eq!(selection(&host), vec![roles[0]], "Undo prunes removed line");
    host.history(true, count + 3).await;
    assert_eq!(selection(&host), vec![roles[0]], "Redo does not resurrect");
    host.stop().await;
}

pub(super) async fn line(host: &mut Host, from: [f64; 2], to: [f64; 2]) -> EntityId {
    host.control("Line");
    let count = host.sketch().entities().len() + 3;
    host.xy(from[0], from[1]);
    host.xy(to[0], to[1]);
    host.count(count).await;
    host.ui(vec![key(egui::Key::Escape, egui::Modifiers::NONE)]);
    host.sketch()
        .entities()
        .iter()
        .rev()
        .find(|entity| matches!(entity, SketchEntity::Line { .. }))
        .unwrap()
        .id()
}

async fn symmetric(host: &mut Host, first: EntityId, second: EntityId, axis: EntityId) {
    let before = host.sketch().constraints().len();
    host.control("Constraints");
    host.control("Symmetric");
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        active(app)
            && app
                .doc
                .sketch
                .session
                .as_ref()
                .unwrap()
                .authoritative
                .constraints()
                .len()
                == before + 1
    })
    .await;
    let endpoint = |line, end: bool| match host.sketch().get_entity(line) {
        Some(SketchEntity::Line {
            start, end: stop, ..
        }) => {
            if end {
                *stop
            } else {
                *start
            }
        }
        _ => panic!("line"),
    };
    let expected = (endpoint(first, true), endpoint(second, false));
    let symmetric = host
        .sketch()
        .constraints()
        .iter()
        .find(|constraint| matches!(constraint, Constraint::Symmetric { .. }))
        .unwrap();
    let Constraint::Symmetric {
        point1,
        point1_position,
        point2,
        point2_position,
        axis: actual,
        ..
    } = symmetric
    else {
        unreachable!()
    };
    assert_eq!((*point1, *point2), expected);
    assert_eq!(*point1_position, CurvePosition::Arbitrary);
    assert_eq!(*point2_position, CurvePosition::Arbitrary);
    assert_eq!(*actual, axis, "the whole axis line is the third operand");
}

pub(super) fn only(host: &Host, kind: impl Fn(&SketchEntity) -> bool) -> EntityId {
    let found: Vec<_> = host
        .sketch()
        .entities()
        .iter()
        .filter(|e| kind(e))
        .collect();
    assert_eq!(found.len(), 1);
    found[0].id()
}

pub(super) fn point(entity: EntityId, role: CurvePosition) -> SketchTarget {
    SketchTarget::Point { entity, role }
}

pub(super) fn selection(host: &Host) -> Vec<SketchTarget> {
    host.app
        .doc
        .sketch
        .session
        .as_ref()
        .unwrap()
        .selection
        .clone()
}

pub(super) fn role(host: &Host, target: SketchTarget) -> egui::Pos2 {
    let at = target.point_coordinates(host.sketch()).unwrap();
    visible(host, host.project(at.x, at.y))
}

fn arc_body(host: &Host, arc: EntityId) -> egui::Pos2 {
    let Some(SketchEntity::Arc {
        center,
        radius,
        start_angle,
        end_angle,
        ..
    }) = host.sketch().get_entity(arc)
    else {
        panic!("arc")
    };
    let Some(SketchEntity::Point { at, .. }) = host.sketch().get_entity(*center) else {
        panic!("arc center")
    };
    let mut sweep = end_angle - start_angle;
    if sweep <= 0.0 {
        sweep += std::f64::consts::TAU;
    }
    let angle = start_angle + sweep * 0.5;
    let body = Vec2::new(at.x + radius * angle.cos(), at.y + radius * angle.sin()).unwrap();
    visible(host, host.project(body.x, body.y))
}

fn visible(host: &Host, position: egui::Pos2) -> egui::Pos2 {
    let viewport = crate::layout::chrome(host.screen(), &host.app.ui).viewport;
    assert!(viewport.contains(position), "{position:?} outside viewport");
    position
}

pub(super) fn shift_click(host: &mut Host, position: egui::Pos2) {
    let shift = egui::Modifiers::SHIFT;
    host.ui(vec![
        egui::Event::ModifiersChanged(shift),
        egui::Event::PointerMoved(position),
        modified(position, true, shift),
    ]);
    host.ui(vec![modified(position, false, shift)]);
    host.ui(vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)]);
}

fn modified(pos: egui::Pos2, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        pressed,
        button: egui::PointerButton::Primary,
        modifiers,
    }
}

fn drag(host: &mut Host, start: egui::Pos2, end: egui::Pos2) {
    host.ui(vec![egui::Event::PointerMoved(start), button(start, true)]);
    host.ui(vec![egui::Event::PointerMoved(
        start + (end - start) * 0.25,
    )]);
    assert!(
        host.app
            .doc
            .sketch
            .session
            .as_ref()
            .unwrap()
            .selection_dragging,
        "native viewport must own the marquee: {}",
        host.app.doc.status
    );
    host.ui(vec![egui::Event::PointerMoved(end)]);
    host.ui(vec![button(end, false)]);
}

/// The Host fixture's sketch camera, applied to an editor that was already open.
pub(super) fn sketch_camera(app: &mut App) {
    app.ui.start_open = false;
    app.ui.explorer_open = false;
    app.ui.inspector_open = false;
    app.ui.sketch_snap = false;
    app.doc.fit_pending = false;
    let session = app.doc.sketch.session.as_ref().unwrap();
    assert_eq!(session.phase, SketchPhase::Active);
    let plane = session.plane;
    app.doc.camera = Camera {
        target: DVec3::new(plane.origin.x, plane.origin.y, plane.origin.z),
        offset: DVec3::new(plane.normal.x, plane.normal.y, plane.normal.z) * 120.0,
        projection: Projection::Orthographic,
        ..Default::default()
    };
}

pub(super) async fn open(host: &mut Host) {
    pump_until(&mut host.app, &host.context, &mut host.frame, active).await;
    sketch_camera(&mut host.app);
    host.redraw();
}

pub(super) async fn wait_closed(host: &mut Host) {
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        !app.sketch_active()
    })
    .await;
    host.redraw();
}
