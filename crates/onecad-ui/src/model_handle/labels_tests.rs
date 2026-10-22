use super::*;

fn captions(viewport: Rect, covers: &[Rect], entries: &[(Pos2, &str)]) -> Vec<Caption> {
    let context = egui::Context::default();
    let mut captions = Vec::new();
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_max(Pos2::ZERO, viewport.max)),
            ..Default::default()
        },
        |ui| {
            let tips: Vec<_> = entries.iter().map(|(tip, _)| *tip).collect();
            let mut layout = Layout::new(viewport, covers.to_vec(), &tips);
            for (tip, text) in entries {
                let caption = layout
                    .caption(ui.painter(), text, *tip, Color32::BLACK)
                    .expect("full readable caption must fit available viewport");
                assert_eq!(
                    caption.galley.text(),
                    *text,
                    "no truncation or authored-value mutation"
                );
                let measured =
                    Rect::from_min_size(caption.bounds.min + PADDING, caption.galley.size());
                assert!(
                    caption.bounds.contains_rect(measured),
                    "whole measured glyph area retained"
                );
                assert!(
                    viewport.contains_rect(caption.bounds),
                    "background and text stay inside viewport"
                );
                assert!(
                    covers.iter().all(|cover| !overlaps(caption.bounds, *cover)),
                    "caption overlaps chrome: {:?}",
                    caption.bounds
                );
                captions.push(caption);
            }
        },
    );
    output.textures_delta.clear();
    captions
}
fn viewport(width: f32, height: f32) -> Rect {
    Rect::from_min_max(pos2(0.0, 44.0), pos2(width, height))
}
fn screenshot_covers() -> Vec<Rect> {
    vec![
        Rect::from_min_max(pos2(10.0, 54.0), pos2(258.0, 822.0)),
        Rect::from_min_max(pos2(1052.0, 54.0), pos2(1340.0, 822.0)),
        Rect::from_min_max(pos2(765.0, 247.0), pos2(1070.0, 555.0)),
        Rect::from_min_max(pos2(894.0, 54.0), pos2(1042.0, 239.0)),
        Rect::from_min_max(pos2(593.0, 54.0), pos2(756.0, 98.0)),
        Rect::from_min_max(pos2(268.0, 784.0), pos2(443.0, 822.0)),
    ]
}

#[test]
fn fixed_two_decimals_are_display_only_and_normalize_negative_zero() {
    for (value, expected) in [
        (50.0, "50.00"),
        (-4.0, "-4.00"),
        (1.8, "1.80"),
        (0.0002, "0.00"),
        (-0.001, "0.00"),
        (8.126, "8.13"),
    ] {
        let original = value;
        assert_eq!(number(value), expected);
        assert_eq!(value, original);
    }
}

#[test]
fn linear_spacing_and_mirror_labels_clear_the_reported_card_and_inspector() {
    for entry in [
        (pos2(1040.0, 607.0), "Pattern spacing: 50.00 mm"),
        (pos2(752.0, 476.0), "Mirror plane offset: -4.00 mm"),
        (pos2(752.0, 300.0), "Shell thickness: 2.00 mm"),
    ] {
        let result = captions(viewport(1350.0, 832.0), &screenshot_covers(), &[entry]);
        assert_eq!(result.len(), 1);
    }
}

#[test]
fn angular_label_relocates_from_ring_tip_without_moving_the_ring() {
    let tip = pos2(1043.0, 358.0);
    let original = tip;
    let result = captions(
        viewport(1350.0, 832.0),
        &screenshot_covers(),
        &[(tip, "Pattern angle: 360.00 °")],
    );
    assert_eq!(tip, original);
    assert!(result[0].bounds.right() < 1052.0);
}

#[test]
fn gear_height_and_module_labels_avoid_each_other_and_bottom_navigation() {
    let result = captions(
        viewport(1350.0, 832.0),
        &screenshot_covers(),
        &[
            (pos2(561.0, 633.0), "Gear height: 8.20 mm"),
            (pos2(641.0, 818.0), "Gear module: 1.80 mm"),
        ],
    );
    assert!(!overlaps(result[0].bounds.expand(GAP), result[1].bounds));
    let crowded = captions(
        viewport(900.0, 560.0),
        &[],
        &[
            (pos2(450.0, 300.0), "Gear height: 8.20 mm"),
            (pos2(454.0, 302.0), "Gear module: 1.80 mm"),
        ],
    );
    assert!(!overlaps(crowded[0].bounds.expand(GAP), crowded[1].bounds));
}

#[test]
fn complete_labels_fit_all_viewport_corners_at_multiple_window_sizes() {
    for (width, height) in [
        (1350.0, 832.0),
        (900.0, 560.0),
        (640.0, 480.0),
        (420.0, 320.0),
    ] {
        let viewport = viewport(width, height);
        for tip in [
            viewport.left_top(),
            viewport.right_top(),
            viewport.right_bottom(),
            viewport.left_bottom(),
        ] {
            let result = captions(viewport, &[], &[(tip, "Pattern spacing: 50.00 mm")]);
            assert!(viewport.shrink(GAP).contains_rect(result[0].bounds));
        }
    }
}

#[test]
fn compact_free_region_wraps_complete_text_instead_of_hiding_or_clipping_it() {
    let viewport = viewport(900.0, 560.0);
    let covers = [
        Rect::from_min_max(pos2(0.0, 44.0), pos2(360.0, 560.0)),
        Rect::from_min_max(pos2(540.0, 44.0), pos2(900.0, 560.0)),
        Rect::from_min_max(pos2(360.0, 44.0), pos2(540.0, 220.0)),
        Rect::from_min_max(pos2(360.0, 360.0), pos2(540.0, 560.0)),
    ];
    let result = captions(
        viewport,
        &covers,
        &[(pos2(450.0, 300.0), "Mirror plane offset: -12345.67 mm")],
    );
    assert!(
        result[0].galley.rows.len() > 1,
        "wrap where the only free region is narrower than the full text"
    );
}

#[test]
fn chrome_edges_expose_remote_free_region_when_adjacent_candidates_are_blocked() {
    let viewport = viewport(1280.0, 820.0);
    let covers = [Rect::from_min_max(pos2(320.0, 44.0), pos2(1280.0, 820.0))];
    let result = captions(
        viewport,
        &covers,
        &[(pos2(900.0, 420.0), "Pattern spacing: 50.00 mm")],
    );
    assert!(result[0].bounds.right() < 320.0);
}

#[test]
fn label_layout_leaves_axis_projection_and_hit_testing_unchanged() {
    let view = super::super::test_support::view();
    let viewport = super::super::test_support::rect();
    for geometry in [
        super::super::Geometry::axis(onecad_scene::DVec3::ZERO, onecad_scene::DVec3::X, 1.0),
        super::super::Geometry::ring(
            onecad_scene::DVec3::ZERO,
            onecad_scene::DVec3::Z,
            onecad_scene::DVec3::X * 20.0,
        )
        .unwrap(),
    ] {
        let points = geometry.points(&view, viewport, 50.0).unwrap();
        let claimed = geometry.claims(&view, viewport, 50.0, points.1);
        let _ = captions(viewport, &[], &[(points.1, "Pattern dimension: 50.00 mm")]);
        assert_eq!(geometry.points(&view, viewport, 50.0).unwrap(), points);
        assert_eq!(geometry.claims(&view, viewport, 50.0, points.1), claimed);
    }
}

#[test]
fn mirror_caption_keeps_signed_plane_offset_before_during_and_after_drag() {
    use super::super::{Drag, Geometry, Kind, Target};
    use onecad_core::{document::record::MirrorBodyParams, ids::RecordId, math::Vec3};
    use onecad_scene::DVec3;
    let plane = MirrorBodyParams {
        source_body: None,
        plane_point: Vec3::new_unchecked(-4.0, 50.0, 60.0),
        plane_normal: Vec3::new_unchecked(1.0, 0.0, 0.0),
        fuse_with_original: false,
        extra: Default::default(),
    };
    let baseline = Target {
        record: RecordId::new(),
        kind: Kind::Mirror(plane.clone()),
        geometry: Geometry::axis(DVec3::ZERO, DVec3::X, 1.0),
        value: 0.0,
        original: String::new(),
    };
    assert_eq!(
        caption_text(&baseline, None, 0.0),
        "Mirror plane offset: -4.00 mm"
    );
    let view = super::super::test_support::view();
    let viewport = super::super::test_support::rect();
    let press = baseline.geometry.points(&view, viewport, 0.0).unwrap().1;
    let mut drag = Drag::start(baseline.clone(), press, &view, viewport, true).unwrap();
    drag.value = -2.0;
    let mut current = baseline;
    if let Kind::Mirror(p) = &mut current.kind {
        p.plane_point.x = -6.0;
    }
    assert_eq!(
        caption_text(&current, Some(&drag), drag.value),
        "Mirror plane offset: -6.00 mm",
        "use frozen origin so updated draft is not counted twice"
    );
    assert_eq!(
        caption_text(&current, None, 0.0),
        "Mirror plane offset: -6.00 mm",
        "release keeps authored plane offset visible"
    );
    let Kind::Mirror(original) = &drag.target.kind else {
        unreachable!()
    };
    assert_eq!(
        original, &plane,
        "label calculations never author placement"
    );
}
