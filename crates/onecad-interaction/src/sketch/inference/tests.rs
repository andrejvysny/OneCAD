use super::*;
use onecad_core::sketch::WorldPlane;
use onecad_scene::{Camera, SceneIdentity, ViewSnapshot, Viewport};

mod typed_destination;

fn fixture() -> (Sketch, SnapSource, EntityId) {
    let mut sketch = Sketch::on_world_plane(SketchId::new(), "Snap", WorldPlane::XY);
    let center = EntityId::new();
    let circle = EntityId::new();
    sketch
        .add_entity(SketchEntity::point(
            center,
            Vec2::new(10.0, 10.0).unwrap(),
            false,
            false,
        ))
        .unwrap();
    sketch
        .add_entity(SketchEntity::circle(circle, center, 5.0, false).unwrap())
        .unwrap();
    let capture = ViewSnapshot::new(
        SceneIdentity {
            session: 1,
            scene: 1,
        },
        1,
        Camera::default(),
        Viewport {
            width: 800.0,
            height: 600.0,
            scale_factor: 2.0,
        },
    )
    .unwrap()
    .identity();
    let source = source_for(&sketch, capture, 1);
    (sketch, source, circle)
}

fn accepted(source: SnapSource, intent: SnapIntent) -> AcceptedSnap {
    AcceptedSnap {
        source,
        kind: SnapKind::Quadrant,
        point: Vec2::new(15.0, 10.0).unwrap(),
        intent,
    }
}

#[test]
fn quadrant_persists_curve_membership_and_center_alignment_together() {
    let (sketch, source, circle) = fixture();
    let center = SketchTarget::Point {
        entity: circle,
        role: CurvePosition::Center,
    };
    for axis in [PointAlignment::Horizontal, PointAlignment::Vertical] {
        let snap = accepted(
            source,
            SnapIntent::Quadrant {
                curve: circle,
                center,
                alignment: axis,
            },
        );
        let point = EntityId::new();
        let constraints =
            persist_snap(&sketch, &snap, source, point, &mut ConstraintId::new).unwrap();
        assert_eq!(constraints.len(), 2);
        assert!(
            matches!(&constraints[0],Constraint::OnCurve {point:p,curve,..} if *p==point&&*curve==circle)
        );
        match &constraints[1] {
            Constraint::HorizontalPoints {
                point2,
                point2_position,
                ..
            }
            | Constraint::VerticalPoints {
                point2,
                point2_position,
                ..
            } => {
                assert_eq!(*point2, circle);
                assert_eq!(*point2_position, CurvePosition::Center);
            }
            _ => panic!("quadrant alignment missing"),
        }
    }
}

#[test]
fn intersections_persist_both_curves_and_geometry_only_persists_nothing() {
    let (mut sketch, source, first) = fixture();
    let second = EntityId::new();
    let center = sketch.entities()[0].id();
    sketch
        .add_entity(SketchEntity::circle(second, center, 7.0, false).unwrap())
        .unwrap();
    let source = source_for(&sketch, source.capture, source.generation);
    let snap = accepted(source, SnapIntent::Intersection { first, second });
    let constraints = persist_snap(
        &sketch,
        &snap,
        source,
        EntityId::new(),
        &mut ConstraintId::new,
    )
    .unwrap();
    assert_eq!(constraints.len(), 2);
    for intent in [SnapIntent::GeometryOnly, SnapIntent::Origin] {
        let count = persist_snap(
            &sketch,
            &accepted(source, intent),
            source,
            EntityId::new(),
            &mut ConstraintId::new,
        )
        .unwrap()
        .len();
        assert_eq!(count, usize::from(intent == SnapIntent::Origin));
    }
}

#[test]
fn stale_generation_capture_geometry_roles_and_duplicate_ids_fail_closed() {
    let (sketch, source, circle) = fixture();
    let snap = accepted(source, SnapIntent::OnCurve(circle));
    for mut changed in [source; 3].into_iter().enumerate() {
        match changed.0 {
            0 => changed.1.generation += 1,
            1 => changed.1.geometry_fingerprint ^= 1,
            _ => changed.1.capture.view += 1,
        }
        assert!(persist_snap(
            &sketch,
            &snap,
            changed.1,
            EntityId::new(),
            &mut ConstraintId::new
        )
        .is_err());
    }
    let snap = accepted(
        source,
        SnapIntent::Coincident(SketchTarget::Point {
            entity: circle,
            role: CurvePosition::Start,
        }),
    );
    assert!(persist_snap(
        &sketch,
        &snap,
        source,
        EntityId::new(),
        &mut ConstraintId::new
    )
    .is_err());
    let snap = accepted(
        source,
        SnapIntent::Quadrant {
            curve: circle,
            center: SketchTarget::Point {
                entity: circle,
                role: CurvePosition::Center,
            },
            alignment: PointAlignment::Horizontal,
        },
    );
    let duplicate = ConstraintId::new();
    assert!(persist_snap(&sketch, &snap, source, EntityId::new(), &mut || duplicate).is_err());
}

#[test]
fn arc_endpoint_identity_and_role_survive_persistence_and_source_changes_are_detected() {
    let (mut sketch, source, _) = fixture();
    assert_eq!(
        source_for(&sketch, source.capture, source.generation),
        source
    );
    let arc = EntityId::new();
    let center = sketch.entities()[0].id();
    sketch
        .add_entity(SketchEntity::arc(arc, center, 5.0, 0.0, 1.0, false).unwrap())
        .unwrap();
    let current = source_for(&sketch, source.capture, source.generation);
    assert_ne!(current.geometry_fingerprint, source.geometry_fingerprint);
    let snap = accepted(
        current,
        SnapIntent::Coincident(SketchTarget::Point {
            entity: arc,
            role: CurvePosition::End,
        }),
    );
    let constraints = persist_snap(
        &sketch,
        &snap,
        current,
        EntityId::new(),
        &mut ConstraintId::new,
    )
    .unwrap();
    assert!(
        matches!(&constraints[0],Constraint::Coincident {point2,point2_position:CurvePosition::End,..} if *point2==arc)
    );
}
