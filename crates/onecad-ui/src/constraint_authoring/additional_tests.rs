use super::*;

#[test]
fn highlights_preserve_point_roles_and_do_not_turn_distance_lines_into_points() {
    let f = fixture();
    let endpoint = SketchTarget::Point {
        entity: f.arc.owner_id(),
        role: CurvePosition::End,
    };
    let constraint =
        operands::build(Relation::Distance, &f.sketch, &[endpoint, f.parallel], None).unwrap();
    assert_eq!(
        constraint_targets(&f.sketch, &constraint),
        vec![endpoint, f.parallel]
    );
    let constraint = operands::build(
        Relation::Symmetric,
        &f.sketch,
        &[f.point, f.point2, f.line],
        None,
    )
    .unwrap();
    assert_eq!(
        constraint_targets(&f.sketch, &constraint),
        vec![f.point, f.point2, f.line]
    );
}

#[test]
fn consecutive_authored_relations_allocate_distinct_nonnil_constraint_ids() {
    let f = fixture();
    let first = operands::create(Relation::Horizontal, &f.sketch, &[f.line], None).unwrap();
    let second = operands::create(Relation::Vertical, &f.sketch, &[f.rotated], None).unwrap();
    assert_ne!(first.id(), second.id());
    assert_ne!(first.id().0, uuid::Uuid::nil());
    assert_ne!(second.id().0, uuid::Uuid::nil());
}

#[test]
fn line_distance_uses_perpendicular_measure_for_rotated_parallel_lines() {
    let mut f = fixture();
    let a = point(&mut f.sketch, 0.0, 0.0);
    let b = point(&mut f.sketch, 10.0, 10.0);
    let c = point(&mut f.sketch, 0.0, 5.0);
    let d = point(&mut f.sketch, 10.0, 15.0);
    let first = add_line(&mut f.sketch, a, b);
    let second = add_line(&mut f.sketch, c, d);
    assert!(operands::build(Relation::Distance, &f.sketch, &[first, second], None).is_ok());
    assert!(
        (dimensions::initial(Relation::Distance, &f.sketch, &[first, second])
            - 5.0 / 2.0f64.sqrt())
        .abs()
            < 1e-12
    );
}

#[test]
fn circular_pair_matrix_includes_circle_circle_arc_arc_and_point_centers() {
    let mut f = fixture();
    let center = point(&mut f.sketch, 30.0, 20.0);
    let circle = EntityId::new();
    let arc = EntityId::new();
    f.sketch
        .add_entity(SketchEntity::circle(circle, center, 4.0, false).unwrap())
        .unwrap();
    f.sketch
        .add_entity(SketchEntity::arc(arc, center, 5.0, 0.2, 1.1, false).unwrap())
        .unwrap();
    for pair in [
        [f.circle, SketchTarget::Entity(circle)],
        [f.arc, SketchTarget::Entity(arc)],
        [f.circle, f.arc],
    ] {
        for relation in [Relation::Tangent, Relation::Concentric, Relation::Equal] {
            assert!(operands::build(relation, &f.sketch, &pair, None).is_ok());
            assert!(operands::build(relation, &f.sketch, &[pair[1], pair[0]], None).is_ok());
        }
    }
    let center = SketchTarget::Point {
        entity: circle,
        role: CurvePosition::Center,
    };
    let fixed = operands::build(Relation::Fixed, &f.sketch, &[center], None).unwrap();
    assert!(matches!(
        fixed,
        Constraint::Fixed {
            point_position: CurvePosition::Center,
            ..
        }
    ));
    let endpoint = SketchTarget::Point {
        entity: f.line.owner_id(),
        role: CurvePosition::Start,
    };
    assert_eq!(
        endpoint.constraint_point(&f.sketch),
        f.point.constraint_point(&f.sketch)
    );
}
