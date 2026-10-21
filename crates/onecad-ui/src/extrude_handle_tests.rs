use super::*;
use crate::{gizmo::project, profiles::decode_profiles};
use onecad_app::dto::{FinishSketchDto, PreviewTrianglesDto, SketchRegionDto};
use onecad_core::{
    document::{
        record::{BooleanMode, ExtrudeMode, KnownOperation, Operation, OperationRecord},
        variables::Scalar,
    },
    ids::{RecordId, SketchId},
    sketch::SketchPlane,
};
use onecad_interaction::{DocumentContext, PreviewStatus, RequestTag, SessionPhase};
use onecad_scene::{Camera, Projection, SceneIdentity, Viewport};

fn view() -> ViewSnapshot {
    ViewSnapshot::new(
        SceneIdentity {
            session: 5,
            scene: 9,
        },
        2,
        Camera {
            offset: DVec3::new(70.0, -110.0, 90.0),
            projection: Projection::Perspective,
            ..Default::default()
        },
        Viewport {
            width: 800.0,
            height: 600.0,
            scale_factor: 1.0,
        },
    )
    .unwrap()
}
fn rect() -> Rect {
    Rect::from_min_size(egui::pos2(30.0, 40.0), egui::vec2(800.0, 600.0))
}
fn region(positions: Vec<f64>, indices: Vec<u32>) -> FinishSketchDto {
    FinishSketchDto {
        region_identity_version: 3,
        diagnostics: vec![],
        regions: vec![SketchRegionDto {
            region_id: "cell-v3:synthetic".into(),
            outer_loop: vec![],
            holes: vec![],
            preview_triangles: Some(PreviewTrianglesDto {
                positions,
                indices,
                holes_subtracted: 0,
            }),
        }],
    }
}
fn centroid(positions: Vec<f64>, indices: Vec<u32>) -> DVec3 {
    decode_profiles(
        SketchId::new(),
        SketchPlane::xy(),
        region(positions, indices),
    )
    .unwrap()
    .remove(0)
    .centroid()
}
fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-9, "{a} != {b}");
}

#[test]
fn rectangle_centroid_is_its_center_in_world_space() {
    // xy plane: sketch (u, v) -> world (-v, u, 0).
    let c = centroid(
        vec![0.0, 0.0, 80.0, 0.0, 80.0, 50.0, 0.0, 50.0],
        vec![0, 1, 2, 0, 2, 3],
    );
    close(c.x, -25.0);
    close(c.y, 40.0);
    close(c.z, 0.0);
}

#[test]
fn l_shape_centroid_is_area_weighted() {
    // 2x1 bar plus 1x1 block above its left cell: area 3, centroid (2/3 ... ) in uv.
    let positions = vec![
        0.0, 0.0, 2.0, 0.0, 2.0, 1.0, 1.0, 1.0, 1.0, 2.0, 0.0, 2.0, 0.0, 1.0,
    ];
    let indices = vec![0, 1, 2, 0, 2, 3, 0, 3, 6, 6, 3, 4, 6, 4, 5];
    let c = centroid(positions, indices);
    // uv centroid = (5/6, 5/6) -> world (-5/6, 5/6, 0)
    close(c.x, -5.0 / 6.0);
    close(c.y, 5.0 / 6.0);
}

#[test]
fn screen_motion_along_normal_maps_to_snapped_distance() {
    let (view, rect) = (view(), rect());
    let (base, n) = (DVec3::ZERO, DVec3::Z);
    let at = |d: f64| project(&view, rect, base + n * d).unwrap();
    let t0 = along(&view, rect, at(5.0), base, n).unwrap();
    let t1 = along(&view, rect, at(8.2), base, n).unwrap();
    assert!((t0 - 5.0).abs() < 1e-3);
    assert!((next_distance(5.0, t1 - t0, 5.0, 1.0, true, false) - 8.2).abs() < 1e-3);
    close(next_distance(5.0, t1 - t0, 5.0, 1.0, false, false), 8.0);
    close(next_distance(5.0, t1 - t0, 5.0, 10.0, false, false), 10.0);
}

#[test]
fn never_emits_zero_and_keeps_side() {
    assert_eq!(next_distance(10.0, -10.0, 1.0, 1.0, false, false), 1.0);
    assert_eq!(next_distance(-10.0, 10.0, -1.0, 1.0, false, false), -1.0);
    assert_eq!(next_distance(10.0, -10.001, 1.0, 5.0, true, false), 0.01);
    assert_eq!(next_distance(1.0, -3.0, 4.0, 1.0, false, true), 1.0);
    assert_eq!(next_distance(1.0, f64::NAN, 4.0, 1.0, false, false), 4.0);
    for text in ["0.01", "-0.01", "1"] {
        assert_ne!(text.parse::<f64>().unwrap(), 0.0);
    }
}

#[test]
fn hit_covers_dot_and_near_tip_segment_only() {
    let (base, tip) = (egui::pos2(100.0, 300.0), egui::pos2(100.0, 100.0));
    assert!(hit(egui::pos2(106.0, 104.0), base, tip));
    assert!(hit(egui::pos2(105.0, 112.0), base, tip));
    assert!(!hit(egui::pos2(130.0, 100.0), base, tip));
    assert!(!hit(egui::pos2(100.0, 200.0), base, tip));
}

fn session(
    parameter: NumericParameter,
    editing_existing: bool,
    phase: SessionPhase,
) -> ModelSession {
    let params = ExtrudeParams {
        profile: None,
        distance: Scalar::new(10.0),
        draft_angle_deg: Scalar::new(0.0),
        mode: ExtrudeMode::Blind,
        boolean_mode: BooleanMode::NewBody,
        target_body: None,
        target_face: None,
        two_directions: false,
        mode2: ExtrudeMode::Blind,
        distance2: Scalar::new(0.0),
        target_face2: None,
        extra: Default::default(),
    };
    let record_id = RecordId::new();
    ModelSession {
        tag: RequestTag {
            context: DocumentContext {
                document_id: "d".into(),
                runtime_session: "r".into(),
                revision: 1,
                snapshot_id: onecad_core::ids::SnapshotId(1),
                scene: SceneIdentity {
                    session: 1,
                    scene: 1,
                },
            },
            record_id,
            gesture: 1,
            sequence: 1,
        },
        record: OperationRecord::new(
            record_id,
            0,
            "Extrude",
            Operation::Known(KnownOperation::Extrude(params)),
        ),
        editing_existing,
        parameter,
        draft: "10".into(),
        draft_status: DraftStatus::Pending,
        preview: PreviewStatus::Empty,
        phase,
    }
}

#[test]
fn gate_hides_handle_unless_editing_a_new_extrude_distance() {
    let editing = SessionPhase::Editing;
    assert!(distance_params(&session(NumericParameter::ExtrudeDistance, false, editing)).is_some());
    assert!(distance_params(&session(NumericParameter::ExtrudeDistance, true, editing)).is_none());
    assert!(
        distance_params(&session(
            NumericParameter::TransformRotationAngle,
            false,
            editing
        ))
        .is_none()
    );
    assert!(
        distance_params(&session(
            NumericParameter::ExtrudeDistance,
            false,
            SessionPhase::Committing
        ))
        .is_none()
    );
}

fn target_z() -> Target {
    Target {
        record: RecordId::new(),
        base: DVec3::ZERO,
        normal: DVec3::Z,
        evaluated: Some(10.0),
        fallback: 10.0,
        two_directions: false,
        draft: "10".into(),
    }
}

#[test]
fn axis_parallel_to_view_never_claims_the_pointer() {
    let rect = rect();
    let top = ViewSnapshot::new(
        SceneIdentity {
            session: 5,
            scene: 9,
        },
        2,
        Camera {
            offset: DVec3::new(0.0, 0.0, 100.0),
            projection: Projection::Orthographic,
            ..Default::default()
        },
        Viewport {
            width: 800.0,
            height: 600.0,
            scale_factor: 1.0,
        },
    )
    .unwrap();
    let target = target_z();
    let tip = project(&top, rect, DVec3::Z * 10.0).unwrap();
    assert!(!claims(&top, rect, &target, 10.0, tip));
    let side = view();
    let tip = project(&side, rect, DVec3::Z * 10.0).unwrap();
    assert!(claims(&side, rect, &target, 10.0, tip));
}

fn drag(d: f64, sent: &str, original: &str) -> Drag {
    Drag {
        t0: 0.0,
        d0: 10.0,
        d,
        original: original.into(),
        sent: sent.into(),
    }
}

#[test]
fn many_moves_yield_one_change_with_the_final_text() {
    let mut drag = drag(10.0, "10", "10");
    assert_eq!(drag.pending(), None);
    for d in [11.0, 12.0, 13.5] {
        drag.d = d;
    }
    assert_eq!(drag.pending().as_deref(), Some("13.5"));
    drag.sent = "13.5".into();
    assert_eq!(drag.pending(), None);
}

#[test]
fn target_loss_restores_original_draft_only_when_changed() {
    assert_eq!(drag(13.5, "13.5", "10").restore().as_deref(), Some("10"));
    assert_eq!(drag(10.0, "10", "10").restore(), None);
}
