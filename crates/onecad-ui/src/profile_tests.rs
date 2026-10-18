use super::*;
use onecad_app::dto::PreviewTrianglesDto;
use onecad_scene::{Camera, SceneIdentity, Viewport};

fn region() -> FinishSketchDto {
    FinishSketchDto {
        region_identity_version: 3,
        diagnostics: vec![],
        regions: vec![SketchRegionDto {
            region_id: "cell-v3:synthetic".into(),
            outer_loop: vec![],
            holes: vec![],
            preview_triangles: Some(PreviewTrianglesDto {
                positions: vec![0.0, 0.0, 80.0, 0.0, 80.0, 50.0, 0.0, 50.0],
                indices: vec![0, 1, 2, 0, 2, 3],
                holes_subtracted: 0,
            }),
        }],
    }
}
fn view() -> ViewSnapshot {
    ViewSnapshot::new(
        SceneIdentity {
            session: 1,
            scene: 1,
        },
        1,
        Camera {
            target: DVec3::new(-25.0, 40.0, 0.0),
            offset: DVec3::new(0.0, 0.0, 100.0),
            ..Default::default()
        },
        Viewport {
            width: 800.0,
            height: 600.0,
            scale_factor: 2.0,
        },
    )
    .unwrap()
}
#[test]
fn canonical_nonstandard_plane_is_preserved_for_painting_and_pick_anchor() {
    let profile = decode_profiles(SketchId::new(), SketchPlane::xy(), region())
        .unwrap()
        .remove(0);
    assert_eq!(profile.bounds.min, DVec3::new(-50.0, 0.0, 0.0));
    assert_eq!(profile.bounds.max, DVec3::new(0.0, 80.0, 0.0));
    let hit = profile.pick(&view(), DVec2::new(400.0, 300.0)).unwrap();
    assert!((hit.0[0] - 40.0).abs() < 1e-9 && (hit.0[1] - 25.0).abs() < 1e-9);
    assert_eq!(profile.boundary.len(), 4);
    assert!(profile.pick(&view(), DVec2::new(0.0, 0.0)).is_none());
}
#[test]
fn region_boundary_is_not_a_strict_interior_fallback_anchor() {
    let profile = decode_profiles(SketchId::new(), SketchPlane::xy(), region())
        .unwrap()
        .remove(0);
    let boundary = view().project(DVec3::new(-25.0, 0.0, 0.0)).unwrap().logical;
    assert!(profile.pick(&view(), boundary).is_none());
}
#[test]
fn missing_holes_nonfinite_coordinates_and_bad_indices_fail_closed() {
    let mut missing = region();
    missing.regions[0].holes.push(vec!["hole".into()]);
    assert!(decode_profiles(SketchId::new(), SketchPlane::xy(), missing).is_err());
    let mut nan = region();
    nan.regions[0].preview_triangles.as_mut().unwrap().positions[0] = f64::NAN;
    assert!(decode_profiles(SketchId::new(), SketchPlane::xy(), nan).is_err());
    let mut bad = region();
    bad.regions[0].preview_triangles.as_mut().unwrap().indices[0] = 999;
    assert!(decode_profiles(SketchId::new(), SketchPlane::xy(), bad).is_err());
}
#[test]
fn unsupported_or_missing_worker_fill_is_not_replaced_with_mock_geometry() {
    let mut missing = region();
    missing.regions[0].preview_triangles = None;
    assert!(decode_profiles(SketchId::new(), SketchPlane::xy(), missing).is_err());
    let mut degenerate = region();
    degenerate.regions[0]
        .preview_triangles
        .as_mut()
        .unwrap()
        .indices = vec![0, 0, 1];
    assert!(decode_profiles(SketchId::new(), SketchPlane::xy(), degenerate).is_err());
}

#[test]
fn scaled_skew_plane_preserves_uv_anchor_without_rewriting_basis() {
    use onecad_core::math::Vec3;
    let mut plane = SketchPlane::xy();
    plane.origin = Vec3::new(10.0, 20.0, 30.0).unwrap();
    plane.x_axis = Vec3::new(2.0, 0.0, 0.0).unwrap();
    plane.y_axis = Vec3::new(1.0, 3.0, 0.0).unwrap();
    plane.normal = Vec3::new(0.0, 0.0, 7.0).unwrap();
    let profile = decode_profiles(SketchId::new(), plane, region())
        .unwrap()
        .remove(0);
    let world = DVec3::new(115.0, 95.0, 30.0);
    let view = ViewSnapshot::new(
        view().identity().scene,
        2,
        Camera {
            target: world,
            offset: DVec3::new(0.0, 0.0, 200.0),
            ..Default::default()
        },
        view().viewport(),
    )
    .unwrap();
    let hit = profile.pick(&view, DVec2::new(400.0, 300.0)).unwrap();
    assert!((hit.0[0] - 40.0).abs() < 1e-9);
    assert!((hit.0[1] - 25.0).abs() < 1e-9);
    plane.y_axis = plane.x_axis;
    assert!(decode_profiles(SketchId::new(), plane, region()).is_err());
}
