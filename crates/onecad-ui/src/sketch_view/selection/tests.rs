use super::*;
use onecad_core::{
    ids::{SketchId, SnapshotId},
    sketch::WorldPlane,
};
use onecad_interaction::{
    DocumentContext,
    sketch::{SketchAction, SketchInteraction, SketchTool},
};
use onecad_scene::{Camera, Projection, SceneIdentity, Viewport};

pub(in crate::sketch_view) fn session() -> (SketchSession, Rect, ViewSnapshot) {
    let scene = SceneIdentity {
        session: 1,
        scene: 1,
    };
    let document = DocumentContext {
        document_id: "document".into(),
        runtime_session: "runtime".into(),
        revision: 1,
        snapshot_id: SnapshotId(1),
        scene,
    };
    let sketch = Sketch::on_world_plane(SketchId::new(), "Sketch", WorldPlane::XY);
    let mut interaction = SketchInteraction::default();
    interaction.dispatch(SketchAction::ContextChanged(Some(document.clone())));
    interaction.dispatch(SketchAction::Begin {
        sketch: Box::new(sketch),
        create: false,
        context: document,
    });
    let mut session = interaction.session.unwrap();
    session.phase = SketchPhase::Active;
    session.tool = SketchTool::Select;
    let rect = Rect::from_min_size(egui::pos2(50.0, 80.0), egui::vec2(800.0, 600.0));
    let view = ViewSnapshot::new(
        scene,
        1,
        Camera {
            offset: DVec3::new(0.0, 0.0, 100.0),
            projection: Projection::Orthographic,
            ..Default::default()
        },
        Viewport {
            width: 800.0,
            height: 600.0,
            scale_factor: 2.0,
        },
    )
    .unwrap();
    session.accepted_capture = Some(view.identity());
    (session, rect, view)
}

pub(in crate::sketch_view) fn add_point(session: &mut SketchSession, x: f64, y: f64) -> EntityId {
    let id = EntityId::new();
    session
        .authoritative
        .add_entity(SketchEntity::point(
            id,
            Vec2::new(x, y).unwrap(),
            false,
            false,
        ))
        .unwrap();
    id
}

pub(in crate::sketch_view) fn at(
    session: &SketchSession,
    rect: Rect,
    view: &ViewSnapshot,
    x: f64,
    y: f64,
) -> Pos2 {
    project(rect, view, session.plane, Vec2::new(x, y).unwrap()).unwrap()
}

#[test]
fn analytic_arc_handles_and_curve_body_keep_distinct_roles() {
    let (mut session, rect, view) = session();
    let center = add_point(&mut session, 0.0, 0.0);
    let arc = EntityId::new();
    session
        .authoritative
        .add_entity(SketchEntity::arc(arc, center, 20.0, 0.0, PI / 2.0, false).unwrap())
        .unwrap();
    for (x, y, role) in [
        (0.0, 0.0, CurvePosition::Center),
        (20.0, 0.0, CurvePosition::Start),
        (0.0, 20.0, CurvePosition::End),
    ] {
        assert_eq!(
            pick_entity(rect, &view, &session, at(&session, rect, &view, x, y), 8.0),
            PickResult::Unique(SketchTarget::Point { entity: arc, role })
        );
    }
    let diagonal = 20.0 / 2.0_f64.sqrt();
    assert_eq!(
        pick_entity(
            rect,
            &view,
            &session,
            at(&session, rect, &view, diagonal, diagonal),
            8.0
        ),
        PickResult::Unique(SketchTarget::Entity(arc))
    );
}

#[test]
fn shared_endpoint_aliases_canonicalize_to_same_persistent_point() {
    let (mut session, rect, view) = session();
    let center = add_point(&mut session, 0.0, 0.0);
    let left = add_point(&mut session, -20.0, 0.0);
    let upper = add_point(&mut session, 0.0, 20.0);
    for (start, end) in [(left, center), (center, upper)] {
        session
            .authoritative
            .add_entity(SketchEntity::line(EntityId::new(), start, end, false))
            .unwrap();
    }
    assert_eq!(
        pick_entity(
            rect,
            &view,
            &session,
            at(&session, rect, &view, 0.0, 0.0),
            8.0
        ),
        PickResult::Unique(SketchTarget::Point {
            entity: center,
            role: CurvePosition::Arbitrary
        })
    );
}

#[test]
fn distinct_coincident_arc_and_line_roles_remain_ambiguous() {
    let (mut session, rect, view) = session();
    let start = add_point(&mut session, 0.0, 0.0);
    let end = add_point(&mut session, 20.0, 0.0);
    let center = add_point(&mut session, -20.0, 0.0);
    let arc = EntityId::new();
    let line = EntityId::new();
    session
        .authoritative
        .add_entity(SketchEntity::line(line, start, end, false))
        .unwrap();
    session
        .authoritative
        .add_entity(SketchEntity::arc(arc, center, 20.0, 0.0, PI / 2.0, false).unwrap())
        .unwrap();
    let PickResult::Ambiguous(targets) = pick_entity(
        rect,
        &view,
        &session,
        at(&session, rect, &view, 0.0, 0.0),
        8.0,
    ) else {
        panic!("distinct handles require choice")
    };
    assert!(targets.contains(&SketchTarget::Point {
        entity: arc,
        role: CurvePosition::Start
    }));
    assert!(targets.contains(&SketchTarget::Point {
        entity: line,
        role: CurvePosition::Start
    }));
}

#[test]
fn window_requires_enclosure_crossing_accepts_touched_curve() {
    let (mut session, rect, view) = session();
    let left = add_point(&mut session, -20.0, 0.0);
    let right = add_point(&mut session, 20.0, 0.0);
    let line = EntityId::new();
    session
        .authoritative
        .add_entity(SketchEntity::line(line, left, right, false))
        .unwrap();
    let start = at(&session, rect, &view, -5.0, 5.0);
    let end = at(&session, rect, &view, 5.0, -5.0);
    assert_eq!(
        marquee_targets(rect, &view, &session, start, end),
        Some(vec![])
    );
    assert_eq!(
        marquee_targets(rect, &view, &session, end, start),
        Some(vec![SketchTarget::Entity(line)])
    );
    let start = at(&session, rect, &view, -25.0, 5.0);
    let end = at(&session, rect, &view, 25.0, -5.0);
    assert_eq!(
        marquee_targets(rect, &view, &session, start, end),
        Some(vec![SketchTarget::Entity(line)])
    );
}

#[test]
fn circle_center_window_selects_center_role_and_stale_view_is_refused() {
    let (mut session, rect, view) = session();
    let center = add_point(&mut session, 0.0, 0.0);
    let circle = EntityId::new();
    session
        .authoritative
        .add_entity(SketchEntity::circle(circle, center, 20.0, false).unwrap())
        .unwrap();
    let start = at(&session, rect, &view, -2.0, 2.0);
    let end = at(&session, rect, &view, 2.0, -2.0);
    assert_eq!(
        marquee_targets(rect, &view, &session, start, end),
        Some(vec![SketchTarget::Point {
            entity: circle,
            role: CurvePosition::Center
        }])
    );
    session.accepted_capture = Some(
        ViewSnapshot::new(view.identity().scene, 2, view.camera(), view.viewport())
            .unwrap()
            .identity(),
    );
    assert_eq!(marquee_targets(rect, &view, &session, start, end), None);
    assert_eq!(
        pick_entity(rect, &view, &session, start, 8.0),
        PickResult::None
    );
}

#[test]
fn selected_endpoint_has_handle_highlight_without_curve_highlight_in_both_themes() {
    let (mut session, rect, view) = session();
    let left = add_point(&mut session, -20.0, 0.0);
    let right = add_point(&mut session, 20.0, 0.0);
    let line = EntityId::new();
    session
        .authoritative
        .add_entity(SketchEntity::line(line, left, right, false))
        .unwrap();
    session.selection = vec![SketchTarget::Point {
        entity: line,
        role: CurvePosition::Start,
    }];
    for dark in [true, false] {
        let theme = crate::theme::load(dark).unwrap();
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(rect.expand(100.0)),
                ..Default::default()
            },
            |ui| {
                paint(ui.painter(), rect, &view, &session, None, &theme);
            },
        );
        output.textures_delta.clear();
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Circle(circle) if circle.fill == theme.accent)));
        let lines: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::LineSegment { stroke, .. } => Some(stroke.color),
                _ => None,
            })
            .collect();
        assert_eq!(lines, vec![theme.dof_neutral]);
    }
}
