use super::*;
use crate::{gizmo::project, profiles::decode_profiles};
use onecad_app::dto::{FinishSketchDto, PreviewTrianglesDto, SketchRegionDto};
use onecad_core::{
    document::{
        record::{BooleanMode, ExtrudeMode, KnownOperation, Operation, OperationRecord},
        refs::SketchRegionRef,
        variables::Scalar,
    },
    ids::{RecordId, RegionId, SketchId},
    sketch::SketchPlane,
};
use onecad_interaction::{DocumentContext, Interaction, SessionPhase};
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
    assert!((next_distance(5.0, t1 - t0, 5.0, 1.0, true, false).0 - 8.2).abs() < 1e-3);
    close(next_distance(5.0, t1 - t0, 5.0, 1.0, false, false).0, 8.0);
    close(next_distance(5.0, t1 - t0, 5.0, 10.0, false, false).0, 10.0);
}

#[test]
fn hit_covers_dot_and_near_tip_segment_only() {
    let (base, tip) = (egui::pos2(100.0, 300.0), egui::pos2(100.0, 100.0));
    assert!(hit(egui::pos2(106.0, 104.0), base, tip));
    assert!(hit(egui::pos2(105.0, 112.0), base, tip));
    assert!(!hit(egui::pos2(130.0, 100.0), base, tip));
    assert!(!hit(egui::pos2(100.0, 200.0), base, tip));
}

fn context() -> DocumentContext {
    DocumentContext {
        document_id: "d".into(),
        runtime_session: "r".into(),
        revision: 1,
        snapshot_id: onecad_core::ids::SnapshotId(1),
        scene: SceneIdentity {
            session: 1,
            scene: 1,
        },
    }
}

fn session(mode: ExtrudeMode, editing_existing: bool) -> ModelSession {
    let params = ExtrudeParams {
        profile: Some(SketchRegionRef {
            sketch: SketchId::new(),
            region: RegionId("region-1".into()),
            region_identity_version: Some(1),
            region_anchor: None,
            extra: Default::default(),
        }),
        distance: Scalar::new(10.0),
        draft_angle_deg: Scalar::new(0.0),
        mode,
        boolean_mode: BooleanMode::NewBody,
        target_body: None,
        target_face: None,
        two_directions: false,
        mode2: ExtrudeMode::Blind,
        distance2: Scalar::new(0.0),
        target_face2: None,
        extra: Default::default(),
    };
    let record = OperationRecord::new(
        RecordId::new(),
        0,
        "Extrude",
        Operation::Known(KnownOperation::Extrude(params)),
    );
    let mut state = Interaction::default();
    state.dispatch(Action::ContextChanged(Some(context())));
    let fields = onecad_interaction::declared_keys(&record.op)
        .into_iter()
        .filter(|key| *key == DISTANCE)
        .collect::<Vec<_>>();
    state.dispatch(if editing_existing {
        Action::BeginParameterEdit {
            record: Box::new(record),
            parameter: NumericParameter::ExtrudeDistance,
            gesture: 1,
            context: context(),
        }
    } else {
        Action::BeginOperation {
            record: Box::new(record),
            focus: fields.first().copied(),
            fields,
            gesture: 1,
            context: context(),
        }
    });
    state.tool.expect("session")
}

#[test]
fn gate_hides_handle_unless_editing_a_new_extrude_distance() {
    assert!(distance_params(&session(ExtrudeMode::Blind, false)).is_some());
    assert!(distance_params(&session(ExtrudeMode::Blind, true)).is_none());
    assert!(distance_params(&session(ExtrudeMode::ThroughAll, false)).is_none());
    let mut committing = session(ExtrudeMode::Blind, false);
    committing.phase = SessionPhase::Committing;
    assert!(distance_params(&committing).is_none());
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

const RULE: Rule = Rule {
    step: 1.0,
    two_directions: false,
};
const PRESS: Pos2 = egui::pos2(100.0, 100.0);

/// Ten screen pixels upward along the axis read as one unit of distance.
fn axis(pos: Pos2) -> Option<f64> {
    Some(f64::from(PRESS.y - pos.y) / 10.0)
}

fn inside(_: Pos2) -> bool {
    true
}

fn frame() -> (Camera, Viewport, Rect) {
    (view().camera(), view().viewport(), rect())
}

fn pressed(draft: &str, value: f64) -> Drag {
    let target = Target {
        evaluated: Some(value),
        draft: draft.into(),
        ..target_z()
    };
    Drag::start(PRESS, axis(PRESS).unwrap(), value, &target, false, frame())
}

fn feed(drag: &mut Drag, event: &egui::Event) -> Fed {
    drag.feed(event, &axis, &inside, &RULE)
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
fn many_moves_yield_one_change_with_the_final_text() {
    let mut drag = pressed("10", 10.0);
    assert_eq!(drag.pending(), None);
    for y in [90.0, 80.0, 70.0] {
        feed(&mut drag, &moved(egui::pos2(100.0, y)));
    }
    assert_eq!(drag.pending().as_deref(), Some("13"));
    drag.sent = Some("13".into());
    assert_eq!(drag.pending(), None);
}

#[test]
fn click_and_sub_threshold_motion_author_nothing() {
    for draft in ["6 * 2", "12.3456"] {
        let mut drag = pressed(draft, 12.0);
        assert_eq!(drag.pending(), None, "press never flushes");
        assert_eq!(feed(&mut drag, &released(PRESS, false)), Fed::Released);
        assert_eq!(
            (drag.accepted, drag.pending(), drag.changed()),
            (false, None, false)
        );

        let mut drag = pressed(draft, 12.0);
        let nudge = PRESS + egui::vec2(3.0, -4.0);
        assert_eq!(feed(&mut drag, &moved(nudge)), Fed::Tracking);
        assert_eq!(feed(&mut drag, &released(nudge, false)), Fed::Released);
        assert_eq!((drag.accepted, drag.d, drag.pending()), (false, 12.0, None));
        assert!(!drag.changed());
    }
}

#[test]
fn accepted_motion_authors_and_return_restores_the_expression() {
    let mut drag = pressed("6 * 2", 12.0);
    feed(&mut drag, &moved(egui::pos2(100.0, 50.0)));
    assert!(drag.accepted);
    assert_eq!(drag.pending().as_deref(), Some("17"));
    drag.sent = Some("17".into());
    // Back at the press point the value reads 12 again: author the expression, not "12".
    feed(&mut drag, &moved(PRESS));
    assert_eq!(drag.pending().as_deref(), Some("6 * 2"));
    assert!(!drag.changed(), "a release here restores the original");
}

#[test]
fn equal_value_release_keeps_authored_text_and_full_precision() {
    // Sideways travel crosses the threshold without moving along the axis.
    let mut drag = pressed("6 * 2", 12.0);
    let side = PRESS + egui::vec2(10.0, 0.0);
    assert_eq!(feed(&mut drag, &released(side, false)), Fed::Released);
    assert!(drag.accepted);
    assert_eq!((drag.pending(), drag.changed()), (None, false));

    let mut drag = pressed("12.3456", 12.3456);
    feed(&mut drag, &released(side, true));
    assert_eq!(drag.pending(), None, "12.35 reads as the authored value");
    let mut drag = pressed("12.3456", 12.3456);
    feed(&mut drag, &released(side, false));
    assert_eq!(
        drag.pending().as_deref(),
        Some("12"),
        "snapping changes the value"
    );
    assert!(drag.changed());
}

#[test]
fn modifiers_are_read_per_event() {
    let mut drag = pressed("12.3456", 12.3456);
    let alt = egui::Modifiers {
        alt: true,
        ..Default::default()
    };
    feed(&mut drag, &egui::Event::ModifiersChanged(alt));
    feed(&mut drag, &moved(egui::pos2(100.0, 75.0)));
    close(drag.d, 14.8456);
    feed(
        &mut drag,
        &egui::Event::ModifiersChanged(egui::Modifiers::NONE),
    );
    feed(&mut drag, &moved(egui::pos2(100.0, 75.0)));
    close(drag.d, 15.0);
}

#[test]
fn pointer_and_focus_loss_end_the_capture() {
    for event in [egui::Event::PointerGone, egui::Event::WindowFocused(false)] {
        let mut drag = pressed("6 * 2", 12.0);
        feed(&mut drag, &moved(egui::pos2(100.0, 50.0)));
        assert_eq!(feed(&mut drag, &event), Fed::Lost);
    }
}

#[test]
fn release_position_ends_the_drag_and_frames_carry_motion() {
    // Frame N: press and move; frame N+1: final move and release.
    let mut drag = pressed("6 * 2", 12.0);
    feed(&mut drag, &moved(egui::pos2(100.0, 60.0)));
    close(drag.d, 16.0);
    drag.sent = drag.pending();
    feed(&mut drag, &moved(egui::pos2(100.0, 30.0)));
    let end = egui::pos2(100.0, 20.0);
    assert_eq!(feed(&mut drag, &released(end, false)), Fed::Released);
    close(drag.d, 20.0);
    assert_eq!(drag.pending().as_deref(), Some("20"));
}

#[test]
fn release_over_chrome_adds_no_motion_of_its_own() {
    let mut drag = pressed("6 * 2", 12.0);
    feed(&mut drag, &moved(egui::pos2(100.0, 20.0)));
    close(drag.d, 20.0);
    let chrome = egui::pos2(140.0, 0.0);
    assert_eq!(
        drag.feed(&released(chrome, false), &axis, &|_| false, &RULE),
        Fed::Released
    );
    close(drag.d, 20.0);
}

#[test]
fn limits_re_anchor_so_reversal_moves_at_once() {
    assert_eq!(
        next_distance(3.0, -10.0, 3.0, 1.0, false, true),
        (1.0, true)
    );
    assert_eq!(next_distance(3.0, 1.0, 3.0, 1.0, false, true), (4.0, false));
    let rule = Rule {
        step: 1.0,
        two_directions: true,
    };
    let mut drag = pressed("3", 3.0);
    // Far past the floor, then back by one unit: the value rises immediately.
    drag.feed(&moved(egui::pos2(100.0, 200.0)), &axis, &inside, &rule);
    close(drag.d, 1.0);
    drag.feed(&moved(egui::pos2(100.0, 190.0)), &axis, &inside, &rule);
    close(drag.d, 2.0);
}

#[test]
fn never_emits_zero_and_keeps_side_with_clamp_flags() {
    assert_eq!(
        next_distance(10.0, -10.0, 1.0, 1.0, false, false),
        (1.0, true)
    );
    assert_eq!(
        next_distance(-10.0, 10.0, -1.0, 1.0, false, false),
        (-1.0, true)
    );
    assert_eq!(
        next_distance(10.0, -10.001, 1.0, 5.0, true, false),
        (0.01, true)
    );
    assert_eq!(
        next_distance(1.0, f64::NAN, 4.0, 1.0, false, false),
        (4.0, false)
    );
    assert_eq!(next_distance(1.0, -3.0, 4.0, 1.0, false, true), (1.0, true));
}
