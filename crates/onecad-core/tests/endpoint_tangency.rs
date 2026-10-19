use onecad_core::ids::{ConstraintId, EntityId, SketchId};
use onecad_core::math::Vec2;
use onecad_core::sketch::{Constraint, CurvePosition as Role, Sketch, SketchEntity, WorldPlane};
use uuid::Uuid;

fn eid(n: u128) -> EntityId {
    EntityId(Uuid::from_u128(n))
}
fn cid(n: u128) -> ConstraintId {
    ConstraintId(Uuid::from_u128(n))
}
fn fixture() -> Sketch {
    let mut sketch =
        Sketch::on_world_plane(SketchId(Uuid::from_u128(99)), "Endpoint", WorldPlane::XY);
    for (n, x, y) in [
        (1, 0., 0.),
        (2, 10., 0.),
        (3, 0., 5.),
        (4, 0., 0.),
        (5, -10., 0.),
    ] {
        sketch
            .add_entity(SketchEntity::point(
                eid(n),
                Vec2::new_unchecked(x, y),
                false,
                false,
            ))
            .unwrap();
    }
    sketch
        .add_entity(SketchEntity::line(eid(10), eid(1), eid(2), false))
        .unwrap();
    sketch
        .add_entity(SketchEntity::line(eid(11), eid(1), eid(5), false))
        .unwrap();
    sketch
        .add_entity(
            SketchEntity::arc(eid(12), eid(3), 5., -std::f64::consts::FRAC_PI_2, 0., false)
                .unwrap(),
        )
        .unwrap();
    sketch
        .add_entity(SketchEntity::circle(eid(13), eid(3), 5., false).unwrap())
        .unwrap();
    sketch
}
fn tangent(a: u128, b: u128, ra: Role, rb: Role) -> Constraint {
    Constraint::TangentAtPoint {
        id: cid(100),
        entity1: eid(a),
        entity2: eid(b),
        entity1_position: ra,
        entity2_position: rb,
    }
}
fn weld(a: u128, b: u128, ra: Role, rb: Role) -> Constraint {
    Constraint::Coincident {
        id: cid(101),
        point1: eid(a),
        point2: eid(b),
        point1_position: ra,
        point2_position: rb,
    }
}

#[test]
fn endpoint_roles_are_required_and_exact_in_persistence() {
    let constraint = tangent(10, 12, Role::Start, Role::Start);
    let value = serde_json::to_value(&constraint).unwrap();
    assert_eq!(value["kind"], "tangentAtPoint");
    assert_eq!(value["entity1Position"], "start");
    assert_eq!(
        serde_json::from_value::<Constraint>(value.clone()).unwrap(),
        constraint
    );
    for key in ["entity1Position", "entity2Position"] {
        let mut malformed = value.clone();
        malformed.as_object_mut().unwrap().remove(key);
        assert!(serde_json::from_value::<Constraint>(malformed).is_err());
    }
    assert_eq!(constraint.entities(), vec![eid(10), eid(12)]);
    assert_eq!(constraint.id(), cid(100));
    assert!(constraint.value().is_none());
}

#[test]
fn canonical_line_point_weld_is_direct_and_order_independent() {
    let mut sketch = fixture();
    let constraints = vec![
        tangent(10, 12, Role::Start, Role::Start),
        weld(12, 1, Role::Start, Role::Arbitrary),
    ];
    sketch.add_constraints(constraints.clone()).unwrap();
    assert_eq!(sketch.constraints(), constraints);
    let loaded: Sketch = serde_json::from_value(serde_json::to_value(&sketch).unwrap()).unwrap();
    loaded.validate_endpoint_tangencies().unwrap();
    assert_eq!(loaded, sketch);
}

#[test]
fn shared_line_endpoint_is_welded_without_coincident() {
    fixture()
        .add_constraint(tangent(10, 11, Role::Start, Role::Start))
        .unwrap();
}

#[test]
fn proximity_transitive_weld_and_wrong_role_never_substitute() {
    let mut sketch = fixture();
    assert!(sketch
        .add_constraint(tangent(10, 12, Role::Start, Role::Start))
        .is_err());
    sketch
        .add_constraint(weld(12, 4, Role::Start, Role::Arbitrary))
        .unwrap();
    assert!(sketch
        .add_constraint(tangent(10, 12, Role::Start, Role::Start))
        .is_err());
    for (a, b, ra, rb) in [
        (10, 12, Role::Center, Role::Start),
        (10, 13, Role::Start, Role::Start),
        (10, 10, Role::Start, Role::End),
    ] {
        assert!(sketch.add_constraint(tangent(a, b, ra, rb)).is_err());
    }
    assert_eq!(sketch.constraints().len(), 1);
}

#[test]
fn failed_batch_leaves_accepted_geometry_and_constraint_order_unchanged() {
    let mut sketch = fixture();
    let before = sketch.clone();
    assert!(sketch
        .add_constraints(vec![
            tangent(10, 12, Role::Start, Role::End),
            weld(10, 12, Role::Start, Role::Start)
        ])
        .is_err());
    assert_eq!(sketch, before);
}

#[test]
fn degenerate_line_refuses_before_constraint_is_added() {
    let mut sketch = fixture();
    sketch
        .add_entity(SketchEntity::line(eid(14), eid(1), eid(1), false))
        .unwrap();
    let before = sketch.clone();
    let error = sketch
        .add_constraint(tangent(10, 14, Role::Start, Role::Start))
        .unwrap_err();
    assert!(error.to_string().contains("TANGENT_DEGENERATE_OPERAND"));
    assert_eq!(sketch, before);
}

#[test]
fn authoritative_document_load_refuses_broken_endpoint_relationship() {
    use onecad_core::{document::Document, ids::DocumentId, io::document_io};
    let mut sketch = fixture();
    sketch
        .add_constraints(vec![
            tangent(10, 12, Role::Start, Role::Start),
            weld(12, 1, Role::Start, Role::Arbitrary),
        ])
        .unwrap();
    let mut doc = Document::new(DocumentId::new());
    let id = sketch.id;
    doc.sketches.insert(id, sketch);
    let value = serde_json::to_value(&doc).unwrap();
    assert!(document_io::document_from_value(value.clone()).is_ok());
    let mut broken = value;
    broken["sketches"][id.to_string()]["constraints"]
        .as_array_mut()
        .unwrap()
        .pop();
    assert!(document_io::document_from_value(broken)
        .unwrap_err()
        .to_string()
        .contains("TANGENT_ENDPOINT_NOT_WELDED"));
}
