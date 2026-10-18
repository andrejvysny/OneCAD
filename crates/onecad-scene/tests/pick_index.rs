mod support;
use onecad_scene::*;
use support::*;

#[test]
fn indexed_and_brute_force_match_across_bvh_splits_clips_roll_and_dpi() {
    let (mut vertices, mut triangles, mut edges) = (Vec::new(), Vec::new(), Vec::new());
    for row in -6..=6 {
        for column in -6..=6 {
            let (x, z) = (column as f32 * 0.7, row as f32 * 0.7);
            let y = ((row * 17 + column * 13) % 5) as f32 * 0.2;
            let first = vertices.len() as u32;
            vertices.extend([[x, y, z], [x + 0.6, y, z], [x, y, z + 0.6]]);
            triangles.push([first, first + 1, first + 2]);
            edges.push(vec![[x, y, z], [x + 0.6, y, z + 0.6]]);
        }
    }
    edges.extend([
        vec![[-1.0, -11.0, 0.0], [1.0, 0.0, 0.0]],
        vec![[0.0, -9.95, -1.0], [0.0, 100_001.0, 1.0]],
        vec![[0.0, 10.0, 0.0], [0.0, -9.0, 0.0]],
    ]);
    let scene = scene(&vertices, &triangles, &edges);
    for projection in [Projection::Perspective, Projection::Orthographic] {
        for scale in [1.0, 1.25, 2.0] {
            for roll in [0.0, 0.71] {
                let base = view(projection, scale);
                let view = ViewSnapshot::new(
                    identity(),
                    11,
                    Camera {
                        roll_radians: roll,
                        ..base.camera()
                    },
                    base.viewport(),
                )
                .unwrap();
                for i in 0..600 {
                    let cursor = DVec2::new(
                        ((i * 137) % 800) as f64 + 0.3,
                        ((i * 179) % 600) as f64 + 0.7,
                    );
                    let indexed = pick(&scene, &view, view.identity(), cursor).unwrap();
                    let brute = pick_brute_force(&scene, &view, view.identity(), cursor).unwrap();
                    assert_eq!(
                        indexed.map(signature),
                        brute.map(signature),
                        "{projection:?} {scale} {roll} {cursor:?}"
                    );
                }
            }
        }
    }
}

fn signature(hit: PickHit<'_>) -> (BodyId, PickKind, &str, DVec3, Option<DVec3>, u64) {
    (
        hit.body_id,
        hit.kind,
        hit.topology_id,
        hit.world_position,
        hit.normal,
        hit.mesh_revision,
    )
}

#[test]
fn bvh_storage_is_admitted_before_publication_at_exact_budget() {
    let make = || {
        body(
            &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            &[[0, 1, 2]; 129],
            &[],
        )
    };
    let accepted = SceneSnapshot::new(identity(), vec![make()], SceneLimits::default()).unwrap();
    let cap = accepted.resident_bytes();
    assert!(cap > accepted.bodies()[0].bytes().len() + 129 * 48);
    assert!(SceneSnapshot::new(
        identity(),
        vec![make()],
        SceneLimits {
            max_resident_bytes: cap,
            ..SceneLimits::default()
        }
    )
    .is_ok());
    assert!(matches!(
        SceneSnapshot::new(
            identity(),
            vec![make()],
            SceneLimits {
                max_resident_bytes: cap - 1,
                ..SceneLimits::default()
            }
        ),
        Err(SceneError::BudgetExceeded(_))
    ));
}
