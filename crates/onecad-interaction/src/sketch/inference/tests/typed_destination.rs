use super::*;

fn endpoint(arc: EntityId, role: CurvePosition) -> SketchTarget {
    SketchTarget::Point { entity: arc, role }
}

#[test]
fn winning_snap_persists_on_new_analytic_arc_endpoint_without_ghost_point() {
    let (sketch, source, circle) = fixture();
    let arc = EntityId::new();
    let additions = [SketchEntity::arc(
        arc,
        sketch.entities()[0].id(),
        5.0,
        0.0,
        std::f64::consts::FRAC_PI_2,
        false,
    )
    .unwrap()];
    let center = endpoint(circle, CurvePosition::Center);
    let intents = [
        SnapIntent::OnCurve(circle),
        SnapIntent::Guide {
            reference: center,
            alignment: PointAlignment::Horizontal,
        },
        SnapIntent::Quadrant {
            curve: circle,
            center,
            alignment: PointAlignment::Horizontal,
        },
    ];
    for intent in intents {
        let constraints = persist_snap_to(
            &sketch,
            &accepted(source, intent),
            source,
            endpoint(arc, CurvePosition::Start),
            &additions,
            &mut ConstraintId::new,
        )
        .unwrap();
        assert!(!constraints.is_empty());
        for constraint in constraints {
            match constraint {
                Constraint::OnCurve {
                    point,
                    point_position,
                    ..
                }
                | Constraint::HorizontalPoints {
                    point1: point,
                    point1_position: point_position,
                    ..
                } => {
                    assert_eq!(point, arc);
                    assert_eq!(point_position, CurvePosition::Start);
                }
                other => panic!("unexpected persisted relation: {other:?}"),
            }
        }
    }
    assert_eq!(sketch.entities().len(), 2);
    assert_eq!(additions.len(), 1);
}

#[test]
fn endpoint_weld_keeps_both_owning_arcs_and_exact_roles() {
    let (mut sketch, source, _) = fixture();
    let old = EntityId::new();
    let center = sketch.entities()[0].id();
    sketch
        .add_entity(
            SketchEntity::arc(old, center, 5.0, 0.0, std::f64::consts::FRAC_PI_2, false).unwrap(),
        )
        .unwrap();
    let source = source_for(&sketch, source.capture, source.generation);
    let arc = EntityId::new();
    let additions = [SketchEntity::arc(
        arc,
        center,
        5.0,
        std::f64::consts::FRAC_PI_2,
        std::f64::consts::PI,
        false,
    )
    .unwrap()];
    let mut snap = accepted(
        source,
        SnapIntent::Coincident(endpoint(old, CurvePosition::End)),
    );
    snap.point = Vec2::new(10.0, 15.0).unwrap();
    let constraints = persist_snap_to(
        &sketch,
        &snap,
        source,
        endpoint(arc, CurvePosition::Start),
        &additions,
        &mut ConstraintId::new,
    )
    .unwrap();
    assert!(matches!(&constraints[..], [Constraint::Coincident {
        point1, point1_position: CurvePosition::Start,
        point2, point2_position: CurvePosition::End, ..
    }] if *point1 == arc && *point2 == old));
}

#[test]
fn typed_destination_refuses_wrong_roles_missing_duplicate_stale_and_displaced_owners() {
    let (sketch, source, circle) = fixture();
    let arc = EntityId::new();
    let center = sketch.entities()[0].id();
    let owner = SketchEntity::arc(arc, center, 5.0, 0.0, 1.0, false).unwrap();
    let snap = accepted(source, SnapIntent::OnCurve(circle));
    let correct = endpoint(arc, CurvePosition::Start);
    for target in [
        SketchTarget::Entity(arc),
        endpoint(arc, CurvePosition::Arbitrary),
        endpoint(arc, CurvePosition::Center),
        endpoint(circle, CurvePosition::Start),
    ] {
        assert!(persist_snap_to(
            &sketch,
            &snap,
            source,
            target,
            std::slice::from_ref(&owner),
            &mut ConstraintId::new
        )
        .is_err());
    }
    for additions in [
        vec![],
        vec![owner.clone(), owner.clone()],
        vec![SketchEntity::circle(arc, center, 5.0, false).unwrap()],
    ] {
        assert!(persist_snap_to(
            &sketch,
            &snap,
            source,
            correct,
            &additions,
            &mut ConstraintId::new
        )
        .is_err());
    }
    let mut changed = source;
    changed.capture.view += 1;
    assert!(persist_snap_to(
        &sketch,
        &snap,
        changed,
        correct,
        std::slice::from_ref(&owner),
        &mut ConstraintId::new
    )
    .is_err());
    let mut displaced = snap;
    displaced.point.x += 0.001;
    assert!(persist_snap_to(
        &sketch,
        &displaced,
        source,
        correct,
        &[owner],
        &mut ConstraintId::new
    )
    .is_err());
}
