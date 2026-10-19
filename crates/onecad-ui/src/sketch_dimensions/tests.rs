use super::*;
use onecad_core::{
    document::variables::Scalar,
    ids::{ConstraintId, EntityId, SketchId, SnapshotId},
    math::Vec2,
    sketch::{Constraint, CurvePosition, Sketch, SketchEntity, WorldPlane},
};
use onecad_interaction::{
    DocumentContext,
    sketch::{SketchInteraction, SketchTarget},
};
use onecad_scene::{Camera, DVec3, Projection, SceneIdentity, Viewport};

fn point(sketch: &mut Sketch, x: f64, y: f64) -> EntityId {
    let id = EntityId::new();
    sketch
        .add_entity(SketchEntity::point(
            id,
            Vec2::new(x, y).unwrap(),
            false,
            false,
        ))
        .unwrap();
    id
}
fn line(sketch: &mut Sketch, a: EntityId, b: EntityId) -> EntityId {
    let id = EntityId::new();
    sketch
        .add_entity(SketchEntity::line(id, a, b, false))
        .unwrap();
    id
}

fn fixture() -> (SketchSession, Rect, ViewSnapshot) {
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
    let mut sketch = Sketch::on_world_plane(SketchId::new(), "Dimensions", WorldPlane::XY);
    let a = point(&mut sketch, 0.0, 0.0);
    let b = point(&mut sketch, 40.0, 20.0);
    let c = point(&mut sketch, 50.0, 0.0);
    let d = point(&mut sketch, 50.0, 40.0);
    let first = line(&mut sketch, a, c);
    let second = line(&mut sketch, a, d);
    let circle = EntityId::new();
    sketch
        .add_entity(SketchEntity::circle(circle, c, 12.5, false).unwrap())
        .unwrap();
    let arc = EntityId::new();
    sketch
        .add_entity(SketchEntity::arc(arc, a, 15.0, 0.0, 1.5, false).unwrap())
        .unwrap();
    for constraint in [
        Constraint::Distance {
            id: ConstraintId::new(),
            entity1: a,
            entity1_position: CurvePosition::Arbitrary,
            entity2: b,
            entity2_position: CurvePosition::Arbitrary,
            value: Scalar::new(44.721359549995796),
        },
        Constraint::HorizontalDistance {
            id: ConstraintId::new(),
            point1: b,
            point1_position: CurvePosition::Arbitrary,
            point2: a,
            point2_position: CurvePosition::Arbitrary,
            value: Scalar::new(-40.0),
        },
        Constraint::VerticalDistance {
            id: ConstraintId::new(),
            point1: a,
            point1_position: CurvePosition::Arbitrary,
            point2: b,
            point2_position: CurvePosition::Arbitrary,
            value: Scalar::new(20.0),
        },
        Constraint::Angle {
            id: ConstraintId::new(),
            line1: first,
            line2: second,
            value: Scalar::new(40.0f64.atan2(50.0)),
        },
        Constraint::Radius {
            id: ConstraintId::new(),
            entity: arc,
            value: Scalar::new(15.0),
        },
        Constraint::Diameter {
            id: ConstraintId::new(),
            entity: circle,
            value: Scalar::with_expr(25.0, "hole_diameter"),
        },
    ] {
        sketch.add_constraint(constraint).unwrap();
    }
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
    let rect = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 560.0));
    let view = ViewSnapshot::new(
        scene,
        1,
        Camera {
            offset: DVec3::new(0.0, 0.0, 100.0),
            projection: Projection::Orthographic,
            ..Default::default()
        },
        Viewport {
            width: 900.0,
            height: 560.0,
            scale_factor: 1.0,
        },
    )
    .unwrap();
    session.accepted_capture = Some(view.identity());
    (session, rect, view)
}

fn badges(session: &SketchSession) -> Vec<paint::Badge> {
    session
        .authoritative
        .constraints()
        .iter()
        .filter_map(|constraint| {
            let (text, accessible) = paint::label(constraint)?;
            Some(paint::Badge {
                id: constraint.id(),
                text,
                accessible,
                witness: geometry::witness(&session.authoritative, constraint)?,
            })
        })
        .collect()
}

fn frame(
    ctx: &egui::Context,
    session: &SketchSession,
    rect: Rect,
    view: &ViewSnapshot,
    cache: &mut Cache,
    events: Vec<egui::Event>,
    dark: bool,
) -> (paint::Output, egui::FullOutput) {
    let mut result = paint::Output::default();
    let theme = onecad_widgets::theme::load(dark).unwrap();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(rect),
            events,
            focused: true,
            ..Default::default()
        },
        |ui| {
            ui.allocate_rect(rect, egui::Sense::click_and_drag());
            let canvas = paint::Canvas {
                rect,
                view,
                plane: session.plane,
                theme: &theme,
                enabled: true,
            };
            result = paint::show(ui, &canvas, &badges(session), cache, vec![]);
        },
    );
    output.textures_delta.clear();
    (result, output)
}

#[test]
fn six_badges_show_values_units_expressions_and_accessible_ids_in_both_themes() {
    let (session, rect, view) = fixture();
    assert!(current(&session, rect, &view));
    for dark in [false, true] {
        let ctx = egui::Context::default();
        onecad_widgets::theme::install(&ctx, onecad_widgets::theme::load(dark).unwrap(), dark);
        let mut cache = Cache::sync(None, &session, view.identity());
        for _ in 0..2 {
            let (_, output) = frame(&ctx, &session, rect, &view, &mut cache, vec![], dark);
            let texts: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::epaint::Shape::Text(text) => Some(text.galley.job.text.as_str()),
                    _ => None,
                })
                .collect();
            for badge in badges(&session) {
                assert!(
                    texts.contains(&badge.text.as_str()),
                    "missing {}",
                    badge.text
                );
                assert!(badge.accessible.contains(&badge.id.to_string()));
            }
        }
        let placed: Vec<_> = cache
            .entries
            .iter()
            .filter_map(|entry| entry.last_rect)
            .collect();
        assert_eq!(placed.len(), 6);
        for (index, bounds) in placed.iter().enumerate() {
            assert!(rect.contains_rect(*bounds));
            for other in &placed[index + 1..] {
                assert!(!bounds.intersects(*other));
            }
        }
    }
}

#[test]
fn real_pointer_click_emits_the_existing_constraint_id_without_changing_geometry() {
    let (session, rect, view) = fixture();
    let before = session.authoritative.clone();
    let ctx = egui::Context::default();
    let mut cache = Cache::sync(None, &session, view.identity());
    for _ in 0..2 {
        frame(&ctx, &session, rect, &view, &mut cache, vec![], false);
    }
    let entry = cache
        .entries
        .iter()
        .find(|entry| entry.last_rect.is_some())
        .unwrap();
    let id = entry.id;
    let position = entry.last_rect.unwrap().center();
    let event = |pressed| egui::Event::PointerButton {
        pos: position,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    let (pressed, _) = frame(
        &ctx,
        &session,
        rect,
        &view,
        &mut cache,
        vec![egui::Event::PointerMoved(position), event(true)],
        false,
    );
    assert!(pressed.owns_pointer);
    assert!(pressed.edit.is_none());
    let (released, _) = frame(
        &ctx,
        &session,
        rect,
        &view,
        &mut cache,
        vec![event(false)],
        false,
    );
    assert_eq!(released.edit, Some(id));
    assert!(released.owns_pointer);
    assert_eq!(session.authoritative, before);
}

#[test]
fn stale_capture_refuses_edit_and_revision_change_cancels_drag_but_keeps_placement() {
    let (mut session, rect, view) = fixture();
    let mut cache = Cache::sync(None, &session, view.identity());
    let id = cache.entries[0].id;
    cache.entries[0].offset = egui::vec2(30.0, -40.0);
    cache.drag = Some(cache::Drag {
        id,
        document: session.document.clone(),
        capture: view.identity(),
        origin: cache.entries[0].offset,
        pointer: egui::pos2(200.0, 200.0),
    });
    let stale =
        ViewSnapshot::new(view.identity().scene, 2, view.camera(), view.viewport()).unwrap();
    assert!(!current(&session, rect, &stale));
    session.document.revision += 1;
    cache = Cache::sync(Some(cache), &session, stale.identity());
    assert!(cache.drag.is_none());
    assert_eq!(cache.entries[0].offset, egui::vec2(30.0, -40.0));
    session.key.generation += 1;
    cache = Cache::sync(Some(cache), &session, stale.identity());
    assert_eq!(cache.entries[0].offset, egui::vec2(0.0, -26.0));
}

#[test]
fn arc_endpoint_and_center_roles_and_perpendicular_line_distance_drive_witnesses() {
    let (session, _, _) = fixture();
    let sketch = &session.authoritative;
    let arc = sketch
        .entities()
        .iter()
        .find(|entity| matches!(entity, SketchEntity::Arc { .. }))
        .unwrap()
        .id();
    let circle = sketch
        .entities()
        .iter()
        .find(|entity| matches!(entity, SketchEntity::Circle { .. }))
        .unwrap()
        .id();
    let constraint = Constraint::HorizontalDistance {
        id: ConstraintId::new(),
        point1: arc,
        point1_position: CurvePosition::End,
        point2: circle,
        point2_position: CurvePosition::Center,
        value: Scalar::new(0.0),
    };
    let witness = geometry::witness(sketch, &constraint).unwrap();
    let a = SketchTarget::Point {
        entity: arc,
        role: CurvePosition::End,
    }
    .point_coordinates(sketch)
    .unwrap();
    let b = SketchTarget::Point {
        entity: circle,
        role: CurvePosition::Center,
    }
    .point_coordinates(sketch)
    .unwrap();
    assert_eq!(witness.measured, [a, Vec2::new(b.x, a.y).unwrap()]);
    assert_eq!(witness.segments[1][0], b);
    let line = sketch
        .entities()
        .iter()
        .find(|entity| matches!(entity, SketchEntity::Line { .. }))
        .unwrap()
        .id();
    let constraint = Constraint::Distance {
        id: ConstraintId::new(),
        entity1: arc,
        entity1_position: CurvePosition::End,
        entity2: line,
        entity2_position: CurvePosition::Arbitrary,
        value: Scalar::new(1.0),
    };
    let witness = geometry::witness(sketch, &constraint).unwrap();
    assert!((witness.measured[1].y).abs() < 1e-12);
    assert_eq!(witness.measured[0], a);
}

#[test]
fn placement_avoids_actual_exclusions_and_cache_is_bounded() {
    let (session, rect, view) = fixture();
    let occupied = Rect::from_center_size(rect.center(), egui::vec2(250.0, 120.0));
    let placed = cache::place(
        rect.center(),
        egui::vec2(100.0, 28.0),
        egui::Vec2::ZERO,
        rect,
        &[occupied],
    )
    .unwrap();
    assert!(!placed.intersects(occupied));
    let mut session = session;
    let circle = session
        .authoritative
        .entities()
        .iter()
        .find(|entity| matches!(entity, SketchEntity::Circle { .. }))
        .unwrap()
        .id();
    for _ in 0..200 {
        session
            .authoritative
            .add_constraint(Constraint::Radius {
                id: ConstraintId::new(),
                entity: circle,
                value: Scalar::new(1.0),
            })
            .unwrap();
    }
    let cache = Cache::sync(None, &session, view.identity());
    assert_eq!(cache.entries.len(), cache::MAX_BADGES);
}

#[test]
fn press_on_old_capture_cannot_edit_after_view_or_visit_changes() {
    for replace_visit in [false, true] {
        let (mut session, rect, view) = fixture();
        let ctx = egui::Context::default();
        let mut cache = Cache::sync(None, &session, view.identity());
        for _ in 0..2 {
            frame(&ctx, &session, rect, &view, &mut cache, vec![], false);
        }
        let position = cache.entries[0].last_rect.unwrap().center();
        let event = |pressed| egui::Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(
            &ctx,
            &session,
            rect,
            &view,
            &mut cache,
            vec![egui::Event::PointerMoved(position), event(true)],
            false,
        );
        let next =
            ViewSnapshot::new(view.identity().scene, 2, view.camera(), view.viewport()).unwrap();
        if replace_visit {
            session.key.generation += 1;
        }
        session.accepted_capture = Some(next.identity());
        cache = Cache::sync(Some(cache), &session, next.identity());
        let (output, _) = frame(
            &ctx,
            &session,
            rect,
            &next,
            &mut cache,
            vec![event(false)],
            false,
        );
        assert!(output.edit.is_none());
    }
}

#[test]
fn dragging_label_keeps_grab_offset_and_never_mutates_the_sketch() {
    let (session, rect, view) = fixture();
    let before = session.authoritative.clone();
    let ctx = egui::Context::default();
    let mut cache = Cache::sync(None, &session, view.identity());
    for _ in 0..2 {
        frame(&ctx, &session, rect, &view, &mut cache, vec![], false);
    }
    let original = cache.entries[0].last_rect.unwrap();
    let position = original.center() + egui::vec2(5.0, 2.0);
    let id = cache.entries[0].id;
    let event = |position, pressed| egui::Event::PointerButton {
        pos: position,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    frame(
        &ctx,
        &session,
        rect,
        &view,
        &mut cache,
        vec![egui::Event::PointerMoved(position), event(position, true)],
        false,
    );
    let moved = position + egui::vec2(0.0, -60.0);
    frame(
        &ctx,
        &session,
        rect,
        &view,
        &mut cache,
        vec![egui::Event::PointerMoved(moved)],
        false,
    );
    let (dragged, _) = frame(&ctx, &session, rect, &view, &mut cache, vec![], false);
    assert!(dragged.owns_pointer);
    assert!(dragged.edit.is_none());
    let bounds = cache
        .entries
        .iter()
        .find(|entry| entry.id == id)
        .unwrap()
        .last_rect
        .unwrap();
    assert!((bounds.center().x - original.center().x).abs() < 0.1);
    assert!((bounds.center().y - original.center().y + 60.0).abs() < 0.1);
    let (released, _) = frame(
        &ctx,
        &session,
        rect,
        &view,
        &mut cache,
        vec![event(moved, false)],
        false,
    );
    assert!(released.owns_pointer);
    assert!(released.edit.is_none());
    assert!(cache.drag.is_none());
    assert_eq!(session.authoritative, before);
}
