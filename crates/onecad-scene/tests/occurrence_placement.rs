mod support;
use onecad_scene::{
    instances::{OccurrenceKey, OccurrencePath},
    *,
};
use support::*;

fn path(n: u128) -> OccurrencePath {
    OccurrencePath(vec![OccurrenceKey(n)])
}
fn triangle() -> SceneBody {
    body(
        &[[-1.0, 0.0, -1.0], [1.0, 0.0, -1.0], [0.0, 0.0, 1.0]],
        &[[0, 1, 2]],
        &[],
    )
}

#[test]
fn twenty_occurrences_share_ten_checked_definitions_and_charge_unique_triangles() {
    let definitions: Vec<_> = (0..10).map(|_| triangle()).collect();
    let mut placed = Vec::new();
    for (i, definition) in definitions.iter().enumerate() {
        for copy in 0..2 {
            let n = (i * 2 + copy + 1) as u128;
            let model = DMat4::from_translation(DVec3::new(n as f64 * 3.0, 0.0, 0.0));
            let body = definition
                .instance(BodyId::new(), path(n), model, true)
                .unwrap();
            assert!(body.shares_geometry(definition));
            assert_eq!(body.source_body_id(), definition.body_id());
            placed.push(body);
        }
    }
    let limits = SceneLimits {
        max_triangles: 20,
        ..SceneLimits::default()
    };
    let scene = SceneSnapshot::new(identity(), placed.clone(), limits).unwrap();
    assert_eq!(scene.bodies().len(), 20);
    let admitted = scene.resident_bytes();
    assert!(SceneSnapshot::new(
        identity(),
        scene.bodies().to_vec(),
        SceneLimits {
            max_resident_bytes: admitted - 1,
            ..limits
        }
    )
    .is_err());
    assert_eq!(scene.bodies().len(), 20);
    assert!(SceneSnapshot::new(
        identity(),
        placed,
        SceneLimits {
            max_triangles: 19,
            ..limits
        }
    )
    .is_err());
}

#[test]
fn picked_path_world_pose_and_visibility_are_per_occurrence() {
    let definition = triangle();
    let a = definition
        .instance(BodyId::new(), path(1), DMat4::IDENTITY, true)
        .unwrap();
    let b = definition
        .instance(
            BodyId::new(),
            path(2),
            DMat4::from_translation(DVec3::new(0.0, -2.0, 0.0)),
            true,
        )
        .unwrap();
    let scene = SceneSnapshot::new(
        identity(),
        vec![a.clone(), b.clone()],
        SceneLimits::default(),
    )
    .unwrap();
    let view = view(Projection::Perspective, 1.0);
    let hit = pick(&scene, &view, view.identity(), DVec2::new(400.0, 300.0))
        .unwrap()
        .unwrap();
    assert_eq!(hit.body_id, b.body_id());
    assert_eq!(hit.source_body_id, definition.body_id());
    assert_eq!(hit.occurrence_path, Some(&path(2)));
    assert_eq!(hit.topology_id, "f:1");
    assert!((hit.world_position.y + 2.0).abs() < 1e-12);
    let hidden = definition
        .instance(b.body_id(), path(2), b.model(), false)
        .unwrap();
    let scene2 =
        SceneSnapshot::new(identity(), vec![a.clone(), hidden], SceneLimits::default()).unwrap();
    assert_eq!(
        pick(&scene2, &view, view.identity(), DVec2::new(400.0, 300.0))
            .unwrap()
            .unwrap()
            .body_id,
        a.body_id()
    );
    assert!(a.shares_geometry(&b));
}

#[test]
fn rotated_occurrence_picking_matches_brute_force() {
    let definition = triangle();
    let model = DMat4::from_translation(DVec3::new(0.2, 0.0, 0.0))
        * DMat4::from_rotation_z(0.3)
        * DMat4::from_rotation_y(0.7);
    let instance = definition
        .instance(BodyId::new(), path(3), model, true)
        .unwrap();
    let scene = SceneSnapshot::new(identity(), vec![instance], SceneLimits::default()).unwrap();
    let view = view(Projection::Perspective, 1.0);
    let point = view.project(DVec3::new(0.2, 0.0, 0.0)).unwrap().logical;
    let a = pick(&scene, &view, view.identity(), point)
        .unwrap()
        .unwrap();
    let b = pick_brute_force(&scene, &view, view.identity(), point)
        .unwrap()
        .unwrap();
    assert_eq!(a.occurrence_path, Some(&path(3)));
    assert!((a.world_position - b.world_position).length() < 1e-12);
    assert!((a.normal.unwrap() - model.transform_vector3(DVec3::NEG_Y)).length() < 1e-12);
}

#[test]
fn transformed_edges_and_hidden_front_body_obey_pick_and_occlusion() {
    let definition = body(&[], &[], &[vec![[-1.0, 0.0, 0.0], [1.0, 0.0, 0.0]]]);
    let transform =
        DMat4::from_translation(DVec3::new(0.0, -1.0, 0.0)) * DMat4::from_rotation_y(0.5);
    let edge = definition
        .instance(BodyId::new(), path(5), transform, true)
        .unwrap();
    let scene = SceneSnapshot::new(identity(), vec![edge], SceneLimits::default()).unwrap();
    let view = view(Projection::Perspective, 1.0);
    let point = view
        .project(transform.transform_point3(DVec3::ZERO))
        .unwrap()
        .logical;
    for pick_fn in [pick, pick_brute_force] {
        let hit = pick_fn(&scene, &view, view.identity(), point)
            .unwrap()
            .unwrap();
        assert_eq!(hit.kind, PickKind::Edge);
        assert_eq!(hit.occurrence_path, Some(&path(5)));
        assert!((hit.world_position - transform.transform_point3(DVec3::ZERO)).length() < 1e-9);
    }
}

#[test]
fn invalid_transforms_and_duplicate_occurrence_bodies_fail_closed() {
    let definition = triangle();
    assert!(definition
        .instance(BodyId::new(), OccurrencePath(vec![]), DMat4::IDENTITY, true)
        .is_err());
    assert!(definition
        .instance(
            BodyId::new(),
            OccurrencePath(vec![OccurrenceKey(1); 9]),
            DMat4::IDENTITY,
            true
        )
        .is_err());
    assert!(definition
        .instance(
            BodyId::new(),
            path(1),
            DMat4::from_scale(DVec3::new(-1.0, 1.0, 1.0)),
            true
        )
        .is_err());
    let a = definition
        .instance(BodyId::new(), path(1), DMat4::IDENTITY, true)
        .unwrap();
    let b = definition
        .instance(BodyId::new(), path(1), DMat4::IDENTITY, true)
        .unwrap();
    assert!(SceneSnapshot::new(identity(), vec![a, b], SceneLimits::default()).is_err());
}

#[test]
fn repeated_heavy_geometry_exceeds_visible_work_without_losing_existing_scene() {
    let triangles = vec![[0, 1, 2]; 250_001];
    let definition = body(
        &[[-1.0, 0.0, -1.0], [1.0, 0.0, -1.0], [0.0, 0.0, 1.0]],
        &triangles,
        &[],
    );
    let placements = (1..=2)
        .map(|n| {
            definition
                .instance(BodyId::new(), path(n), DMat4::IDENTITY, true)
                .unwrap()
        })
        .collect();
    let old = SceneSnapshot::new(identity(), vec![triangle()], SceneLimits::default()).unwrap();
    assert!(matches!(
        SceneSnapshot::new(identity(), placements, SceneLimits::default()),
        Err(SceneError::BudgetExceeded("visible triangles"))
    ));
    assert_eq!(old.bodies().len(), 1);
}

#[test]
fn visible_edge_work_is_counted_per_placement() {
    let definition = body(&[], &[], &[vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]]]);
    let placement = |n, visible| {
        definition
            .instance(BodyId::new(), path(n), DMat4::IDENTITY, visible)
            .unwrap()
    };
    let limits = SceneLimits {
        max_edge_segments: 1,
        ..SceneLimits::default()
    };
    assert!(matches!(
        SceneSnapshot::new(
            identity(),
            vec![placement(1, true), placement(2, true)],
            limits
        ),
        Err(SceneError::BudgetExceeded("visible edge segments"))
    ));
    assert!(SceneSnapshot::new(
        identity(),
        vec![placement(1, true), placement(2, false)],
        limits
    )
    .is_ok());
}
