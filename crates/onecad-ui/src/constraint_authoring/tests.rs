use super::*;
use onecad_core::{
    ids::{EntityId, SketchId},
    math::Vec2,
    sketch::{CurvePosition, Sketch, SketchAttachment, SketchEntity, WorldPlane},
};

struct Fixture {
    sketch: Sketch,
    point: SketchTarget,
    point2: SketchTarget,
    line: SketchTarget,
    parallel: SketchTarget,
    rotated: SketchTarget,
    circle: SketchTarget,
    arc: SketchTarget,
    ellipse: SketchTarget,
}
fn point(sketch: &mut Sketch, x: f64, y: f64) -> EntityId {
    let id = EntityId::new();
    sketch
        .add_entity(SketchEntity::point(
            id,
            Vec2::new_unchecked(x, y),
            false,
            false,
        ))
        .unwrap();
    id
}
fn add_line(sketch: &mut Sketch, a: EntityId, b: EntityId) -> SketchTarget {
    let id = EntityId::new();
    sketch
        .add_entity(SketchEntity::line(id, a, b, false))
        .unwrap();
    SketchTarget::Entity(id)
}
fn fixture() -> Fixture {
    let mut sketch = Sketch::new(
        SketchId::new(),
        "Test",
        SketchAttachment::World {
            plane: WorldPlane::XY,
        },
    );
    let a = point(&mut sketch, 0.0, 0.0);
    let b = point(&mut sketch, 10.0, 0.0);
    let c = point(&mut sketch, 0.0, 5.0);
    let d = point(&mut sketch, 10.0, 5.0);
    let line = add_line(&mut sketch, a, b);
    let parallel = add_line(&mut sketch, c, d);
    let rotated = add_line(&mut sketch, a, c);
    let circle = EntityId::new();
    let arc = EntityId::new();
    let ellipse = EntityId::new();
    sketch
        .add_entity(SketchEntity::circle(circle, a, 3.0, false).unwrap())
        .unwrap();
    sketch
        .add_entity(SketchEntity::arc(arc, c, 2.0, 0.0, std::f64::consts::PI, false).unwrap())
        .unwrap();
    sketch
        .add_entity(SketchEntity::ellipse(ellipse, d, 4.0, 2.0, 0.0, false).unwrap())
        .unwrap();
    Fixture {
        sketch,
        point: SketchTarget::Point {
            entity: a,
            role: CurvePosition::Arbitrary,
        },
        point2: SketchTarget::Point {
            entity: b,
            role: CurvePosition::Arbitrary,
        },
        line,
        parallel,
        rotated,
        circle: SketchTarget::Entity(circle),
        arc: SketchTarget::Entity(arc),
        ellipse: SketchTarget::Entity(ellipse),
    }
}

#[test]
fn geometric_operand_matrix_accepts_supported_pairs_and_reversed_order() {
    let f = fixture();
    for (relation, targets) in [
        (Relation::Coincident, vec![f.point, f.point2]),
        (Relation::HorizontalPoints, vec![f.point, f.point2]),
        (Relation::VerticalPoints, vec![f.point, f.point2]),
        (Relation::Horizontal, vec![f.line]),
        (Relation::Vertical, vec![f.line]),
        (Relation::Fixed, vec![f.point]),
        (Relation::Parallel, vec![f.line, f.parallel]),
        (Relation::Perpendicular, vec![f.line, f.rotated]),
        (Relation::Equal, vec![f.line, f.rotated]),
        (Relation::Equal, vec![f.circle, f.arc]),
        (Relation::Tangent, vec![f.line, f.circle]),
        (Relation::Tangent, vec![f.line, f.arc]),
        (Relation::Tangent, vec![f.circle, f.arc]),
        (Relation::Concentric, vec![f.circle, f.arc]),
        (Relation::Midpoint, vec![f.point, f.line]),
        (Relation::OnCurve, vec![f.point, f.circle]),
        (Relation::OnCurve, vec![f.point, f.arc]),
    ] {
        assert!(
            operands::build(relation, &f.sketch, &targets, None).is_ok(),
            "{}",
            relation.label()
        );
        let reversed: Vec<_> = targets.iter().rev().copied().collect();
        assert!(
            operands::build(relation, &f.sketch, &reversed, None).is_ok(),
            "reversed {}",
            relation.label()
        );
    }
    for pair in [[f.point, f.point2], [f.point2, f.point]] {
        assert!(
            operands::build(
                Relation::Symmetric,
                &f.sketch,
                &[pair[0], pair[1], f.line],
                None
            )
            .is_ok()
        );
    }
}

#[test]
fn incompatible_operands_are_explicit_and_point_roles_remain_distinct() {
    let f = fixture();
    for (relation, targets) in [
        (Relation::Tangent, vec![f.line, f.rotated]),
        (Relation::Equal, vec![f.line, f.circle]),
        (Relation::Concentric, vec![f.line, f.circle]),
        (Relation::Radius, vec![f.ellipse]),
        (Relation::OnCurve, vec![f.point, f.ellipse]),
        (Relation::Horizontal, vec![f.point, f.point2]),
        (Relation::HorizontalPoints, vec![f.line]),
        (Relation::Symmetric, vec![f.line, f.point, f.point2]),
    ] {
        assert!(
            operands::build(relation, &f.sketch, &targets, None).is_err(),
            "{}",
            relation.label()
        );
    }
    let start = SketchTarget::Point {
        entity: f.arc.owner_id(),
        role: CurvePosition::Start,
    };
    let center = SketchTarget::Point {
        entity: f.circle.owner_id(),
        role: CurvePosition::Center,
    };
    let c = operands::build(Relation::Coincident, &f.sketch, &[start, center], None).unwrap();
    assert!(matches!(
        c,
        Constraint::Coincident {
            point1_position: CurvePosition::Start,
            point2_position: CurvePosition::Center,
            ..
        }
    ));
    let c = operands::build(Relation::Fixed, &f.sketch, &[start], None).unwrap();
    assert!(matches!(
        c,
        Constraint::Fixed {
            point_position: CurvePosition::Start,
            ..
        }
    ));
}

#[test]
fn dimensional_operand_matrix_preserves_order_and_supports_rotated_parallel_lines() {
    let f = fixture();
    for (relation, targets) in [
        (Relation::Distance, vec![f.line]),
        (Relation::Distance, vec![f.point, f.point2]),
        (Relation::Distance, vec![f.point, f.parallel]),
        (Relation::Distance, vec![f.line, f.parallel]),
        (Relation::HorizontalDistance, vec![f.point, f.point2]),
        (Relation::VerticalDistance, vec![f.point, f.point2]),
        (Relation::Angle, vec![f.line, f.rotated]),
        (Relation::Radius, vec![f.arc]),
        (Relation::Diameter, vec![f.arc]),
    ] {
        assert!(operands::build(relation, &f.sketch, &targets, Some(Scalar::new(3.0))).is_ok());
    }
    assert!(operands::build(Relation::Distance, &f.sketch, &[f.line, f.rotated], None).is_err());
    let value = dimensions::initial(
        Relation::HorizontalDistance,
        &f.sketch,
        &[f.point2, f.point],
    );
    assert_eq!(value, -10.0);
    assert_eq!(
        dimensions::initial(Relation::VerticalDistance, &f.sketch, &[f.point, f.point2]),
        0.0
    );
    assert_eq!(
        dimensions::initial(Relation::Distance, &f.sketch, &[f.line, f.parallel]),
        5.0
    );
}

#[test]
fn site_specific_scalar_rules_bind_unbind_and_convert_angles_once() {
    for relation in [
        Relation::HorizontalDistance,
        Relation::VerticalDistance,
        Relation::Angle,
    ] {
        for text in ["0", "-2.5"] {
            assert!(dimensions::scalar(text, relation, &[]).is_ok());
        }
    }
    for relation in [Relation::Distance, Relation::Radius, Relation::Diameter] {
        for text in ["0", "-2.5"] {
            assert!(dimensions::scalar(text, relation, &[]).is_err());
        }
    }
    assert_eq!(
        dimensions::scalar("180", Relation::Angle, &[]).unwrap(),
        Scalar::new(std::f64::consts::PI)
    );
    let angle = dimensions::scalar("90deg * 2", Relation::Angle, &[]).unwrap();
    assert_eq!(angle.value, std::f64::consts::PI);
    assert_eq!(angle.expr.as_deref(), Some("90deg * 2"));
    assert!(dimensions::scalar("1mm", Relation::Angle, &[]).is_err());
    assert!(dimensions::scalar("1deg", Relation::Distance, &[]).is_err());
    assert_eq!(
        dimensions::scalar("=width * 2", Relation::Distance, &[])
            .unwrap()
            .expr
            .as_deref(),
        Some("width * 2")
    );
    assert!(dimensions::scalar("NaN", Relation::Distance, &[]).is_err());
    for invalid in ["", "1/0", "width +", "1mm + 2deg"] {
        assert!(
            dimensions::scalar(invalid, Relation::Distance, &[]).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn editing_existing_dimension_keeps_identity_and_roles_and_shows_expression() {
    let f = fixture();
    let start = SketchTarget::Point {
        entity: f.arc.owner_id(),
        role: CurvePosition::Start,
    };
    let mut constraint = operands::build(
        Relation::HorizontalDistance,
        &f.sketch,
        &[start, f.point2],
        Some(Scalar::with_expr(5.0, "width / 2")),
    )
    .unwrap();
    let id = constraint.id();
    let refs = constraint.entities();
    assert_eq!(dimensions::text(&constraint).as_deref(), Some("width / 2"));
    dimensions::set_value(
        &mut constraint,
        dimensions::scalar("-3", Relation::HorizontalDistance, &[]).unwrap(),
    );
    assert_eq!(constraint.id(), id);
    assert_eq!(constraint.entities(), refs);
    assert!(constraint.value().unwrap().expr.is_none());
    assert!(matches!(
        constraint,
        Constraint::HorizontalDistance {
            point1_position: CurvePosition::Start,
            ..
        }
    ));
    dimensions::set_value(
        &mut constraint,
        dimensions::scalar("new_width", Relation::HorizontalDistance, &[]).unwrap(),
    );
    assert_eq!(
        constraint.value().unwrap().expr.as_deref(),
        Some("new_width")
    );
    let angle = operands::build(
        Relation::Angle,
        &f.sketch,
        &[f.line, f.rotated],
        Some(Scalar::new(std::f64::consts::FRAC_PI_2)),
    )
    .unwrap();
    assert_eq!(dimensions::text(&angle).as_deref(), Some("90"));
}

#[test]
fn duplicate_relations_are_named_without_new_geometry_or_identity() {
    let mut f = fixture();
    let existing = operands::build(Relation::Equal, &f.sketch, &[f.circle, f.arc], None).unwrap();
    let id = existing.id();
    f.sketch.add_constraint(existing).unwrap();
    let reversed = operands::build(Relation::Equal, &f.sketch, &[f.arc, f.circle], None).unwrap();
    assert!(operands::already_applied(&f.sketch, &reversed));
    assert_eq!(f.sketch.constraints().len(), 1);
    assert_eq!(f.sketch.constraints()[0].id(), id);
}

fn draft() -> Draft {
    let f = fixture();
    let constraint = operands::build(
        Relation::Diameter,
        &f.sketch,
        &[f.circle],
        Some(Scalar::new(6.0)),
    )
    .unwrap();
    let document = DocumentContext {
        document_id: "document".into(),
        runtime_session: "runtime".into(),
        revision: 1,
        snapshot_id: onecad_core::ids::SnapshotId(1),
        scene: onecad_scene::SceneIdentity {
            session: 1,
            scene: 1,
        },
    };
    Draft {
        key: SketchSessionKey {
            document_id: "document".into(),
            runtime_session: "runtime".into(),
            sketch_id: f.sketch.id,
            generation: 1,
        },
        document,
        targets: vec![f.circle],
        constraint,
        editing: true,
        text: "6".into(),
        focus: true,
        position: None,
        viewport: None,
        error: None,
        pending: None,
    }
}

#[test]
fn dimension_editor_keeps_composition_enter_escape_and_tab_noncommitting() {
    for key in [egui::Key::Enter, egui::Key::Escape, egui::Key::Tab] {
        let ctx = egui::Context::default();
        let mut draft = draft();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            editor(ui.ctx(), &mut draft, &[]);
        });
        output.textures_delta.clear();
        for ime in [
            egui::ImeEvent::Preedit {
                text: "候補".into(),
                active_range_chars: None,
            },
            egui::ImeEvent::Commit("2".into()),
        ] {
            let mut raw = egui::RawInput {
                events: vec![
                    egui::Event::Ime(ime),
                    egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                ..Default::default()
            };
            let mut composing = true;
            crate::input::protect_composition(&mut composing, &mut raw, true);
            let mut result = (true, true);
            let mut output = ctx.run_ui(raw, |ui| {
                result = editor(ui.ctx(), &mut draft, &[]);
            });
            output.textures_delta.clear();
            assert_eq!(result, (false, false));
        }
    }
}

#[test]
fn expression_preview_uses_resolved_units_and_preserves_source_for_authority() {
    let variable = VariableDto {
        id: "angle".into(),
        name: "theta".into(),
        value: 90.0,
        expr: Some("90deg".into()),
        resolved_value: 90.0,
        dimension: "angle".into(),
        error: None,
    };
    let value = dimensions::scalar("theta * 2", Relation::Angle, &[variable]).unwrap();
    assert_eq!(value.value, std::f64::consts::PI);
    assert_eq!(value.expr.as_deref(), Some("theta * 2"));
    let length = VariableDto {
        id: "width".into(),
        name: "width".into(),
        value: 80.0,
        expr: None,
        resolved_value: 80.0,
        dimension: "length".into(),
        error: None,
    };
    assert_eq!(
        dimensions::scalar("width/2", Relation::Distance, &[length])
            .unwrap()
            .value,
        40.0
    );
    assert_eq!(
        dimensions::scalar("=2", Relation::Distance, &[])
            .unwrap()
            .expr
            .as_deref(),
        Some("2")
    );
}

#[path = "additional_tests.rs"]
mod additional_tests;
