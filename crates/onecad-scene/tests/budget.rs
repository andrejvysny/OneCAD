mod support;
use onecad_scene::*;
use std::sync::Arc;
use support::*;

#[test]
fn bounds_all_resident_bytes_triangles_segments_and_bodies() {
    let make = || {
        body(
            &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            &[[0, 1, 2]],
            &[vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]]],
        )
    };
    for limit in [
        SceneLimits {
            max_resident_bytes: 1,
            ..SceneLimits::default()
        },
        SceneLimits {
            max_triangles: 0,
            ..SceneLimits::default()
        },
        SceneLimits {
            max_edge_segments: 0,
            ..SceneLimits::default()
        },
        SceneLimits {
            max_bodies: 0,
            ..SceneLimits::default()
        },
    ] {
        assert!(matches!(
            SceneSnapshot::new(identity(), vec![make()], limit),
            Err(SceneError::BudgetExceeded(_))
        ));
    }
    let accepted = SceneSnapshot::new(identity(), vec![make()], SceneLimits::default()).unwrap();
    assert!(accepted.resident_bytes() > accepted.bodies()[0].bytes().len());
    let bytes = accepted.resident_bytes();
    assert!(SceneSnapshot::new(
        identity(),
        vec![make()],
        SceneLimits {
            max_resident_bytes: bytes,
            ..SceneLimits::default()
        }
    )
    .is_ok());
    assert!(matches!(
        SceneSnapshot::new(
            identity(),
            vec![make()],
            SceneLimits {
                max_resident_bytes: bytes - 1,
                ..SceneLimits::default()
            }
        ),
        Err(SceneError::BudgetExceeded(_))
    ));
}

#[test]
fn invalid_mesh_and_duplicate_body_are_safe_errors() {
    assert!(matches!(
        SceneBody::new(
            BodyId::new(),
            0,
            Arc::from([0u8; 64]),
            MeshLimits::default()
        ),
        Err(SceneError::InvalidMesh(_))
    ));
    let bytes: Arc<[u8]> = Arc::from(blob(&[], &[], &[]));
    let id = BodyId::new();
    let make = || SceneBody::new(id, 1, bytes.clone(), MeshLimits::default()).unwrap();
    assert!(matches!(
        SceneSnapshot::new(identity(), vec![make(), make()], SceneLimits::default()),
        Err(SceneError::DuplicateBody)
    ));
    assert!(Bounds::new(DVec3::ONE, DVec3::ZERO).is_err());
    assert!(Bounds::new(DVec3::splat(f64::NAN), DVec3::ZERO).is_err());
}
