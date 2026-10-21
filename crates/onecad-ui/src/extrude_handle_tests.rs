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
        press: egui::pos2(100.0, 100.0),
        t0: 0.0,
        d0: 10.0,
        d,
        evaluated: Some(10.0),
        original: original.into(),
        sent: sent.into(),
        moved: true,
        free: false,
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

const RULE: Rule = Rule {
    slop: 6.0,
    step: 1.0,
    two_directions: false,
};
const PRESS: Pos2 = egui::pos2(100.0, 100.0);

/// Ten screen pixels upward along the axis read as one unit of distance.
fn axis(pos: Pos2) -> Option<f64> {
    Some(f64::from(PRESS.y - pos.y) / 10.0)
}

fn pressed(draft: &str, value: f64) -> Drag {
    let target = Target {
        evaluated: Some(value),
        draft: draft.into(),
        ..target_z()
    };
    Drag::start(PRESS, axis(PRESS).unwrap(), value, &target, false)
}

fn moved(pos: Pos2) -> egui::Event {
    egui::Event::PointerMoved(pos)
}

fn released(pos: Pos2, alt: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers {
            alt,
            ..Default::default()
        },
    }
}

#[test]
fn click_and_sub_threshold_motion_author_nothing() {
    for draft in ["6 * 2", "12.3456"] {
        let mut drag = pressed(draft, 12.0);
        assert_eq!(drag.pending(), None, "press never flushes");
        assert_eq!(
            drag.feed(&released(PRESS, false), &axis, &RULE),
            Fed::Released
        );
        assert_eq!((drag.moved, drag.pending()), (false, None));

        let mut drag = pressed(draft, 12.0);
        let nudge = PRESS + egui::vec2(3.0, -4.0);
        assert_eq!(drag.feed(&moved(nudge), &axis, &RULE), Fed::Tracking);
        assert_eq!(
            drag.feed(&released(nudge, false), &axis, &RULE),
            Fed::Released
        );
        assert_eq!((drag.moved, drag.d, drag.pending()), (false, 12.0, None));
        assert_eq!(drag.restore(), None);
    }
}

#[test]
fn accepted_motion_authors_and_return_restores_the_expression() {
    let mut drag = pressed("6 * 2", 12.0);
    drag.feed(&moved(egui::pos2(100.0, 50.0)), &axis, &RULE);
    assert!(drag.moved);
    assert_eq!(drag.pending().as_deref(), Some("17"));
    drag.sent = "17".into();
    // Back at the press point the value reads 12 again: author the expression, not "12".
    drag.feed(&moved(PRESS), &axis, &RULE);
    assert_eq!(drag.pending().as_deref(), Some("6 * 2"));
}

#[test]
fn equal_value_release_keeps_authored_text_and_full_precision() {
    // Sideways travel crosses the slop without moving along the axis.
    let mut drag = pressed("6 * 2", 12.0);
    let side = PRESS + egui::vec2(10.0, 0.0);
    assert_eq!(
        drag.feed(&released(side, false), &axis, &RULE),
        Fed::Released
    );
    assert!(drag.moved);
    assert_eq!(drag.pending(), None);

    let mut drag = pressed("12.3456", 12.3456);
    drag.feed(&released(side, true), &axis, &RULE);
    assert_eq!(drag.pending(), None, "12.35 reads as the authored value");
    let mut drag = pressed("12.3456", 12.3456);
    drag.feed(&released(side, false), &axis, &RULE);
    assert_eq!(
        drag.pending().as_deref(),
        Some("12"),
        "snapping changes the value"
    );
}

#[test]
fn modifiers_are_read_per_event() {
    let mut drag = pressed("12.3456", 12.3456);
    let alt = egui::Modifiers {
        alt: true,
        ..Default::default()
    };
    drag.feed(&egui::Event::ModifiersChanged(alt), &axis, &RULE);
    drag.feed(&moved(egui::pos2(100.0, 75.0)), &axis, &RULE);
    close(drag.d, 14.8456);
    drag.feed(
        &egui::Event::ModifiersChanged(egui::Modifiers::NONE),
        &axis,
        &RULE,
    );
    drag.feed(&moved(egui::pos2(100.0, 75.0)), &axis, &RULE);
    close(drag.d, 15.0);
}

#[test]
fn loss_events_cancel_and_restore_the_original_exactly() {
    let escape = egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    for event in [
        egui::Event::PointerGone,
        egui::Event::WindowFocused(false),
        escape,
    ] {
        let mut drag = pressed("6 * 2", 12.0);
        drag.feed(&moved(egui::pos2(100.0, 50.0)), &axis, &RULE);
        drag.sent = drag.pending().unwrap();
        assert_eq!(drag.feed(&event, &axis, &RULE), Fed::Lost);
        assert_eq!(drag.restore().as_deref(), Some("6 * 2"));
    }
}

#[test]
fn release_position_ends_the_drag_and_frames_carry_motion() {
    // Frame N: press and move; frame N+1: final move and release.
    let mut drag = pressed("6 * 2", 12.0);
    drag.feed(&moved(egui::pos2(100.0, 60.0)), &axis, &RULE);
    close(drag.d, 16.0);
    drag.sent = drag.pending().unwrap();
    drag.feed(&moved(egui::pos2(100.0, 30.0)), &axis, &RULE);
    let end = egui::pos2(100.0, 20.0);
    assert_eq!(
        drag.feed(&released(end, false), &axis, &RULE),
        Fed::Released
    );
    close(drag.d, 20.0);
    assert_eq!(drag.pending().as_deref(), Some("20"));
}

#[test]
fn second_press_in_a_batch_starts_from_the_dispatched_draft() {
    let mut target = Target {
        evaluated: Some(12.0),
        draft: "6 * 2".into(),
        ..target_z()
    };
    rebase(&mut target, Some("6 * 2"));
    assert_eq!(
        (target.draft.as_str(), target.evaluated),
        ("6 * 2", Some(12.0))
    );
    rebase(&mut target, None);
    assert_eq!(target.evaluated, Some(12.0));

    rebase(&mut target, Some("20"));
    assert_eq!((target.draft.as_str(), target.evaluated), ("20", None));
    let mut drag = Drag::start(PRESS, axis(PRESS).unwrap(), 20.0, &target, false);
    assert_eq!((drag.original.as_str(), drag.sent.as_str()), ("20", "20"));
    drag.feed(&moved(egui::pos2(100.0, 180.0)), &axis, &RULE);
    assert_eq!(drag.pending().as_deref(), Some("12"));
    drag.sent = "12".into();
    assert_eq!(
        drag.restore().as_deref(),
        Some("20"),
        "cancel keeps drag one"
    );
    drag.feed(&moved(PRESS), &axis, &RULE);
    assert_eq!(drag.pending().as_deref(), Some("20"));
}
