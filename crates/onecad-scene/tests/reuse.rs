mod support;
use onecad_scene::*;
use std::sync::Arc;
use support::*;

fn fixture_body() -> SceneBody {
    body(
        &[[-1.0, 0.0, -1.0], [1.0, 0.0, -1.0], [0.0, 0.0, 1.0]],
        &[[0, 1, 2]],
        &[],
    )
}

#[test]
fn fresh_publication_reuses_checked_content_and_indices_with_new_revision() {
    let old = SceneSnapshot::new(identity(), vec![fixture_body()], SceneLimits::default()).unwrap();
    let previous = &old.bodies()[0];
    let incoming: Arc<[u8]> = Arc::from(previous.bytes().to_vec());
    assert!(!Arc::ptr_eq(&incoming, previous.bytes()));
    assert!(previous.same_content(&incoming));
    let reused = SceneBody::new_reusing(
        previous.body_id(),
        99,
        incoming,
        MeshLimits::default(),
        Some(previous),
    )
    .unwrap();
    assert!(previous.shares_geometry(&reused));
    assert!(Arc::ptr_eq(previous.bytes(), reused.bytes()));
    assert!(std::ptr::eq(
        previous.mesh().unwrap().header(),
        reused.mesh().unwrap().header()
    ));
    let next_identity = SceneIdentity {
        scene: 100,
        ..identity()
    };
    let next = SceneSnapshot::new(next_identity, vec![reused], SceneLimits::default()).unwrap();
    assert!(previous.shares_geometry(&next.bodies()[0]));
    assert_eq!(old.resident_bytes(), next.resident_bytes());
    let old_view = view(Projection::Perspective, 1.0);
    let view =
        ViewSnapshot::new(next_identity, 101, old_view.camera(), old_view.viewport()).unwrap();
    let hit = pick(&next, &view, view.identity(), DVec2::new(400.0, 300.0))
        .unwrap()
        .unwrap();
    assert_eq!(hit.mesh_revision, 99);
    assert_eq!(hit.capture.scene, next_identity);
    assert!(matches!(
        pick(&old, &view, view.identity(), DVec2::ZERO),
        Err(SceneError::CaptureMismatch)
    ));
}

#[test]
fn reuse_obeys_current_mesh_and_scene_limits() {
    let previous = fixture_body();
    let result = SceneBody::new_reusing(
        previous.body_id(),
        4,
        previous.bytes().clone(),
        MeshLimits {
            max_triangles: 0,
            ..MeshLimits::default()
        },
        Some(&previous),
    );
    assert!(matches!(result, Err(SceneError::InvalidMesh(_))));
    let shared = previous.with_revision(4);
    assert!(previous.shares_geometry(&shared));
    assert_eq!(shared.mesh_revision(), 4);
    assert!(matches!(
        SceneSnapshot::new(
            identity(),
            vec![shared],
            SceneLimits {
                max_triangles: 0,
                ..SceneLimits::default()
            }
        ),
        Err(SceneError::BudgetExceeded(_))
    ));
}

#[test]
fn different_content_and_invalid_bytes_never_reuse_previous_geometry() {
    let previous = fixture_body();
    let changed = Arc::from(blob(&[], &[], &[]));
    assert!(!previous.same_content(&changed));
    let next = SceneBody::new_reusing(
        previous.body_id(),
        4,
        changed,
        MeshLimits::default(),
        Some(&previous),
    )
    .unwrap();
    assert!(!previous.shares_geometry(&next));
    assert!(SceneBody::new_reusing(
        previous.body_id(),
        5,
        Arc::from([0u8; 64]),
        MeshLimits::default(),
        Some(&previous)
    )
    .is_err());
}
