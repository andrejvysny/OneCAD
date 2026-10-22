//! Extrude distance handle through the native host event route; synthetic input
//! with production worker and Metal, not physical acceptance.
use super::point_center_delivery_tests::support::*;
use super::*;
use onecad_core::ids::RecordId;
use onecad_interaction::{
    Action, CommitStatus, DraftStatus, FeatureOutcome, PreviewStatus, ProfileSelection,
    sketch::SketchAction,
};
use onecad_scene::{Camera, DVec3, Projection, ViewSnapshot, Viewport};

const AUTHORED: &str = "6 * 2";

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn handle_click_and_sub_threshold_keep_authored_expression() {
    let root = tempfile::tempdir().unwrap();
    let (mut host, axis) = extruding(root.path(), AUTHORED, 12.0).await;
    let (before, undo) = (sequence(&host), undo_depth(&host));
    let tip = axis.at(&host, 12.0);
    host.ui(vec![
        egui::Event::PointerMoved(tip),
        button(tip, true),
        button(tip, false),
    ]);
    let nudge = tip + egui::vec2(3.0, -3.0);
    host.ui(vec![button(tip, true), egui::Event::PointerMoved(nudge)]);
    host.ui(vec![button(nudge, false)]);
    assert!(!host.app.doc.extrude_handle.active());
    assert_draft(&host, AUTHORED, 12.0);
    assert_eq!(sequence(&host), before, "no draft change may be dispatched");
    assert_eq!(undo_depth(&host), undo);
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn handle_batched_drag_uses_release_before_later_hover() {
    let root = tempfile::tempdir().unwrap();
    let (mut host, axis) = extruding(root.path(), AUTHORED, 12.0).await;
    let undo = undo_depth(&host);
    let (tip, end) = (axis.at(&host, 12.0), axis.at(&host, 20.0));
    host.ui(vec![
        egui::Event::PointerMoved(tip),
        button(tip, true),
        egui::Event::PointerMoved(end),
        button(end, false),
        egui::Event::PointerMoved(axis.at(&host, 30.0)),
    ]);
    assert!(!host.app.doc.extrude_handle.active());
    valid(&mut host, "20", 20.0).await;
    assert_eq!(undo_depth(&host), undo, "dragging authors a draft only");
    confirm(&mut host).await;
    assert_eq!(undo_depth(&host), undo + 1, "one accepted drag, one entry");
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn handle_press_move_then_move_release_next_frame_keeps_final_value() {
    let root = tempfile::tempdir().unwrap();
    let (mut host, axis) = extruding(root.path(), AUTHORED, 12.0).await;
    let tip = axis.at(&host, 12.0);
    let (middle, end) = (axis.at(&host, 16.0), axis.at(&host, 20.0));
    host.ui(vec![
        egui::Event::PointerMoved(tip),
        button(tip, true),
        egui::Event::PointerMoved(middle),
    ]);
    assert!(host.app.doc.extrude_handle.active());
    assert_eq!(draft(&host), "16");
    host.ui(vec![egui::Event::PointerMoved(end), button(end, false)]);
    assert!(!host.app.doc.extrude_handle.active());
    valid(&mut host, "20", 20.0).await;

    // Fallback: the viewport (and so the handle) is not shown when the release arrives.
    host.ui(vec![
        egui::Event::PointerMoved(end),
        button(end, true),
        egui::Event::PointerMoved(middle),
    ]);
    assert_eq!(draft(&host), "16");
    host.app.ui.start_open = true;
    host.ui(vec![button(middle, false)]);
    host.app.ui.start_open = false;
    assert!(!host.app.doc.extrude_handle.active());
    valid(&mut host, "16", 16.0).await;
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn handle_release_at_authored_value_restores_expression_and_precision() {
    let root = tempfile::tempdir().unwrap();
    let (mut host, axis) = extruding(root.path(), AUTHORED, 12.0).await;
    let undo = undo_depth(&host);
    let tip = axis.at(&host, 12.0);
    host.ui(vec![
        egui::Event::PointerMoved(tip),
        button(tip, true),
        egui::Event::PointerMoved(axis.at(&host, 20.0)),
    ]);
    assert_eq!(draft(&host), "20");
    host.ui(vec![egui::Event::PointerMoved(tip), button(tip, false)]);
    valid(&mut host, AUTHORED, 12.0).await;

    host.app.dispatch(Action::DraftChanged("12.3456".into()));
    valid(&mut host, "12.3456", 12.3456).await;
    host.redraw();
    let tip = axis.at(&host, 12.3456);
    let before = sequence(&host);
    let alt = egui::Modifiers {
        alt: true,
        ..Default::default()
    };
    // Alt is held only between its own events; the frame-final state (none)
    // would snap the release to "12" and lose the authored precision.
    host.ui(vec![
        egui::Event::PointerMoved(tip),
        button(tip, true),
        egui::Event::ModifiersChanged(alt),
        egui::Event::PointerMoved(axis.at(&host, 20.0)),
        egui::Event::PointerMoved(tip),
        egui::Event::PointerButton {
            pos: tip,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: alt,
        },
        egui::Event::ModifiersChanged(egui::Modifiers::NONE),
    ]);
    assert!(!host.app.doc.extrude_handle.active());
    assert_draft(&host, "12.3456", 12.3456);
    assert_eq!(sequence(&host), before, "net no-op dispatches nothing");
    assert_eq!(undo_depth(&host), undo);
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn handle_pointer_gone_focus_loss_and_escape_restore_without_history() {
    let root = tempfile::tempdir().unwrap();
    let (mut host, axis) = extruding(root.path(), AUTHORED, 12.0).await;
    let undo = undo_depth(&host);
    for condition in 0..3 {
        let tip = axis.at(&host, 12.0);
        host.ui(vec![
            egui::Event::PointerMoved(tip),
            button(tip, true),
            egui::Event::PointerMoved(axis.at(&host, 20.0)),
        ]);
        assert_eq!(draft(&host), "20", "condition {condition}");
        match condition {
            0 => host.ui(vec![egui::Event::PointerGone]),
            1 => host.ui_focused(vec![egui::Event::WindowFocused(false)], false),
            _ => host.ui(vec![key(egui::Key::Escape, egui::Modifiers::NONE)]),
        }
        host.ui(vec![button(tip, false)]);
        assert!(
            !host.app.doc.extrude_handle.active(),
            "condition {condition}"
        );
        // Escape included: the first one cancels only the drag.
        assert!(
            host.app.doc.interaction.tool.is_some(),
            "condition {condition}"
        );
        valid(&mut host, AUTHORED, 12.0).await;
        if condition == 2 {
            host.ui(vec![key(egui::Key::Escape, egui::Modifiers::NONE)]);
            assert!(
                host.app.doc.interaction.tool.is_none(),
                "the second Escape cancels the tool"
            );
        }
        assert_eq!(undo_depth(&host), undo, "condition {condition}");
    }
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn handle_second_press_in_batch_drags_from_the_first_release() {
    let root = tempfile::tempdir().unwrap();
    let (mut host, axis) = extruding(root.path(), AUTHORED, 12.0).await;
    let undo = undo_depth(&host);
    let (tip, first) = (axis.at(&host, 12.0), axis.at(&host, 20.0));
    host.ui(vec![
        egui::Event::PointerMoved(tip),
        button(tip, true),
        egui::Event::PointerMoved(first),
        button(first, false),
        button(first, true),
        egui::Event::PointerMoved(tip),
    ]);
    assert!(host.app.doc.extrude_handle.active());
    assert_eq!(
        draft(&host),
        "12",
        "second drag starts from the dispatched 20"
    );
    host.ui(vec![button(tip, false)]);
    assert!(!host.app.doc.extrude_handle.active());
    valid(&mut host, "12", 12.0).await;
    confirm(&mut host).await;
    assert_eq!(undo_depth(&host), undo + 1);
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn handle_cancelled_second_press_keeps_the_first_drag() {
    let root = tempfile::tempdir().unwrap();
    let (mut host, axis) = extruding(root.path(), AUTHORED, 12.0).await;
    let undo = undo_depth(&host);
    let (tip, first) = (axis.at(&host, 12.0), axis.at(&host, 20.0));
    host.ui(vec![
        egui::Event::PointerMoved(tip),
        button(tip, true),
        egui::Event::PointerMoved(first),
        button(first, false),
        button(first, true),
        egui::Event::PointerMoved(axis.at(&host, 26.0)),
    ]);
    assert_eq!(draft(&host), "26");
    host.ui(vec![egui::Event::PointerGone]);
    assert!(!host.app.doc.extrude_handle.active());
    valid(&mut host, "20", 20.0).await;
    assert_eq!(undo_depth(&host), undo);
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn handle_three_drags_then_confirm_add_one_history_entry() {
    let root = tempfile::tempdir().unwrap();
    let (mut host, axis) = extruding(root.path(), AUTHORED, 12.0).await;
    let undo = undo_depth(&host);
    let mut from = 12.0;
    for to in [20.0, 25.0, 30.0] {
        let (tip, end) = (axis.at(&host, from), axis.at(&host, to));
        host.ui(vec![
            egui::Event::PointerMoved(tip),
            button(tip, true),
            egui::Event::PointerMoved(end),
        ]);
        host.ui(vec![button(end, false)]);
        valid(&mut host, &format!("{to}"), to).await;
        assert_eq!(undo_depth(&host), undo, "drag {to} authors a draft only");
        from = to;
    }
    confirm(&mut host).await;
    assert_eq!(undo_depth(&host), undo + 1, "three drags, one entry");
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn handle_release_over_chrome_ends_drag_at_last_motion() {
    let root = tempfile::tempdir().unwrap();
    let (mut host, axis) = extruding(root.path(), AUTHORED, 12.0).await;
    let undo = undo_depth(&host);
    let tip = axis.at(&host, 12.0);
    let card = crate::layout::chrome(host.screen(), &host.app.ui)
        .feature_card
        .center();
    host.ui(vec![
        egui::Event::PointerMoved(tip),
        button(tip, true),
        egui::Event::PointerMoved(axis.at(&host, 20.0)),
        egui::Event::PointerMoved(card),
    ]);
    let dragged = draft(&host);
    let value: f64 = dragged.parse().unwrap();
    // Off the axis and away from the last motion: tracking it would change the value.
    let chrome = crate::layout::chrome(host.screen(), &host.app.ui).feature_card;
    let release = card + egui::vec2(0.0, -chrome.height() / 4.0);
    assert!(chrome.contains(release));
    let (last, off) = (axis.along(&host, card), axis.along(&host, release));
    assert!((off - last).abs() > 1.0, "release probe {last} vs {off}");
    host.ui(vec![button(release, false)]);
    assert!(!host.app.doc.extrude_handle.active());
    assert_eq!(draft(&host), dragged, "release adds no motion of its own");
    valid(&mut host, &dragged, value).await;
    assert_draft(&host, &dragged, value);
    confirm(&mut host).await;
    assert_eq!(undo_depth(&host), undo + 1);
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn handle_redrag_after_equal_value_release_keeps_expression() {
    let root = tempfile::tempdir().unwrap();
    let (mut host, axis) = extruding(root.path(), AUTHORED, 12.0).await;
    let undo = undo_depth(&host);
    let (tip, far) = (axis.at(&host, 12.0), axis.at(&host, 20.0));
    host.ui(vec![
        egui::Event::PointerMoved(tip),
        button(tip, true),
        egui::Event::PointerMoved(far),
    ]);
    assert_eq!(draft(&host), "20");
    // Release back at the authored value restores it; the re-press starts there.
    host.ui(vec![
        egui::Event::PointerMoved(tip),
        button(tip, false),
        button(tip, true),
        egui::Event::PointerMoved(far),
    ]);
    assert!(host.app.doc.extrude_handle.active());
    assert_eq!(draft(&host), "20");
    host.ui(vec![egui::Event::PointerMoved(tip), button(tip, false)]);
    assert!(!host.app.doc.extrude_handle.active());
    valid(&mut host, AUTHORED, 12.0).await;
    assert_eq!(undo_depth(&host), undo);
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn handle_press_after_pointer_loss_hits_the_restored_tip_only() {
    let root = tempfile::tempdir().unwrap();
    let (mut host, axis) = extruding(root.path(), AUTHORED, 12.0).await;
    let undo = undo_depth(&host);
    let tip = axis.at(&host, 12.0);
    let (lost, further) = (axis.at(&host, 26.0), axis.at(&host, 30.0));
    host.ui(vec![
        egui::Event::PointerMoved(tip),
        button(tip, true),
        egui::Event::PointerMoved(lost),
    ]);
    assert_eq!(draft(&host), "26");
    // The loss restores 12, so a press where the dragged tip was claims nothing.
    host.ui(vec![
        egui::Event::PointerGone,
        egui::Event::PointerMoved(lost),
        button(lost, true),
        egui::Event::PointerMoved(further),
    ]);
    assert!(!host.app.doc.extrude_handle.active());
    host.ui(vec![egui::Event::PointerMoved(tip), button(tip, false)]);
    assert!(!host.app.doc.extrude_handle.active());
    valid(&mut host, AUTHORED, 12.0).await;
    assert_eq!(undo_depth(&host), undo);
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn handle_tab_switch_mid_drag_restores_authored_value() {
    let root = tempfile::tempdir().unwrap();
    let (mut host, axis) = extruding(root.path(), AUTHORED, 12.0).await;
    let undo = undo_depth(&host);
    let (tip, far) = (axis.at(&host, 12.0), axis.at(&host, 20.0));
    host.ui(vec![
        egui::Event::PointerMoved(tip),
        button(tip, true),
        egui::Event::PointerMoved(far),
    ]);
    assert_eq!(draft(&host), "20");
    let tab = host.app.doc.tab;
    host.app.open_tab().unwrap();
    host.app.activate_tab(tab);
    assert!(!host.app.doc.extrude_handle.active());
    assert_eq!(draft(&host), AUTHORED, "deactivation restored the draft");
    host.ui(vec![button(far, false)]);
    valid(&mut host, AUTHORED, 12.0).await;
    assert_eq!(undo_depth(&host), undo);
    confirm(&mut host).await;
    assert_eq!(undo_depth(&host), undo + 1);
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn handle_release_then_escape_in_one_batch_reaches_the_tool() {
    let root = tempfile::tempdir().unwrap();
    let (mut host, axis) = extruding(root.path(), AUTHORED, 12.0).await;
    let undo = undo_depth(&host);
    let (tip, far) = (axis.at(&host, 12.0), axis.at(&host, 20.0));
    host.ui(vec![
        egui::Event::PointerMoved(tip),
        button(tip, true),
        egui::Event::PointerMoved(far),
    ]);
    assert_eq!(draft(&host), "20");
    // The release ends the capture first, so this Escape belongs to the tool.
    host.ui(vec![
        button(far, false),
        key(egui::Key::Escape, egui::Modifiers::NONE),
    ]);
    assert!(!host.app.doc.extrude_handle.active());
    assert!(
        host.app.doc.interaction.tool.is_none(),
        "an Escape after the release is not swallowed as a drag cancel"
    );
    assert_eq!(undo_depth(&host), undo);
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn handle_camera_change_mid_drag_restores_authored_value() {
    let root = tempfile::tempdir().unwrap();
    let (mut host, axis) = extruding(root.path(), AUTHORED, 12.0).await;
    let undo = undo_depth(&host);
    let (tip, far) = (axis.at(&host, 12.0), axis.at(&host, 20.0));
    host.ui(vec![
        egui::Event::PointerMoved(tip),
        button(tip, true),
        egui::Event::PointerMoved(far),
    ]);
    assert_eq!(draft(&host), "20");
    host.app.doc.camera.offset *= 1.25;
    host.redraw();
    assert!(!host.app.doc.extrude_handle.active());
    assert_eq!(
        draft(&host),
        AUTHORED,
        "a view change ends the drag as lost"
    );
    host.ui(vec![button(far, false)]);
    valid(&mut host, AUTHORED, 12.0).await;
    assert_eq!(undo_depth(&host), undo);
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn handle_drag_begun_while_file_action_runs_still_restores_on_escape() {
    let root = tempfile::tempdir().unwrap();
    let (mut host, axis) = extruding(root.path(), AUTHORED, 12.0).await;
    let undo = undo_depth(&host);
    let tip = axis.at(&host, 12.0);
    host.app.doc.documents.running = true;
    host.ui(vec![
        egui::Event::PointerMoved(tip),
        button(tip, true),
        egui::Event::PointerMoved(axis.at(&host, 20.0)),
    ]);
    assert!(host.app.doc.extrude_handle.active());
    assert_eq!(draft(&host), AUTHORED, "authoring is refused while it runs");
    host.app.doc.documents.running = false;
    host.ui(vec![egui::Event::PointerMoved(axis.at(&host, 25.0))]);
    assert_eq!(draft(&host), "25");
    assert!(
        host.app.doc.interaction.tool.as_ref().unwrap().dragging(),
        "the first accepted Begin opened the reducer drag"
    );
    host.ui(vec![key(egui::Key::Escape, egui::Modifiers::NONE)]);
    assert!(!host.app.doc.extrude_handle.active());
    assert!(host.app.doc.interaction.tool.is_some());
    valid(&mut host, AUTHORED, 12.0).await;
    assert_eq!(undo_depth(&host), undo);
    host.stop().await;
}

/// World axis of the extrude handle for screen projections.
pub(super) struct Axis {
    base: DVec3,
    normal: DVec3,
}

impl Axis {
    pub(super) fn at(&self, host: &Host, distance: f64) -> egui::Pos2 {
        let view = host.app.doc.view.unwrap();
        let rect = crate::layout::chrome(host.screen(), &host.app.ui).viewport;
        crate::gizmo::project(&view, rect, self.base + self.normal * distance).unwrap()
    }

    fn along(&self, host: &Host, pos: egui::Pos2) -> f64 {
        let view = host.app.doc.view.unwrap();
        let rect = crate::layout::chrome(host.screen(), &host.app.ui).viewport;
        crate::gizmo::along(&view, rect, pos, self.base, self.normal).unwrap()
    }
}

pub(super) async fn extruding(root: &std::path::Path, draft: &str, value: f64) -> (Host, Axis) {
    let mut host = Host::new(root).await;
    host.control("Rectangle");
    host.xy(0.0, 0.0);
    host.xy(80.0, 60.0);
    host.count(8).await;
    host.app.sketch_dispatch(SketchAction::Finish);
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        !app.sketch_active()
            && app
                .doc
                .display
                .as_ref()
                .is_some_and(|d| !d.profiles.is_empty())
    })
    .await;
    let display = host.app.doc.display.clone().unwrap();
    let profile = &display.profiles[0];
    let normal = profile.plane().normal;
    let axis = Axis {
        base: profile.centroid(),
        normal: DVec3::new(normal.x, normal.y, normal.z).normalize(),
    };
    host.app.doc.camera = Camera {
        target: axis.base,
        offset: DVec3::new(70.0, -110.0, 90.0),
        projection: Projection::Perspective,
        ..Default::default()
    };
    let context = crate::sketch_services::document_context(&display);
    let view = ViewSnapshot::new(
        context.scene,
        host.app.view_number + 1,
        host.app.doc.camera,
        Viewport {
            width: 900.0,
            height: 560.0,
            scale_factor: 1.0,
        },
    )
    .unwrap();
    host.app
        .dispatch(Action::SelectProfile(Box::new(ProfileSelection {
            profile: profile.reference.clone(),
            context,
            capture: view.identity(),
        })));
    host.app.dispatch(Action::BeginExtrude {
        record_id: RecordId::new(),
        gesture: 1,
        step_index: display.projection.applied_ops as u32,
    });
    host.app.dispatch(Action::DraftChanged(draft.into()));
    valid(&mut host, draft, value).await;
    host.redraw();
    let tip = axis.at(&host, value);
    assert!(
        host.screen().contains(tip),
        "handle tip {tip:?} must be on screen; status {}",
        host.app.doc.status
    );
    (host, axis)
}

pub(super) async fn valid(host: &mut Host, text: &str, value: f64) {
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        app.doc.interaction.tool.as_ref().is_some_and(|tool| {
            tool.draft == text
                && matches!(&tool.draft_status, DraftStatus::Valid(v) if (v.value - value).abs() < 1e-9)
                && tool.preview == PreviewStatus::Ready
        })
    })
    .await;
    host.redraw();
}

pub(super) async fn confirm(host: &mut Host) {
    host.app.dispatch(Action::Confirm);
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        matches!(
            app.doc.interaction.status,
            CommitStatus::Completed {
                outcome: FeatureOutcome::Published,
                ..
            }
        )
    })
    .await;
}

pub(super) fn draft(host: &Host) -> String {
    host.app
        .doc
        .interaction
        .tool
        .as_ref()
        .unwrap()
        .draft
        .clone()
}

fn sequence(host: &Host) -> u64 {
    host.app.doc.interaction.tool.as_ref().unwrap().tag.sequence
}

fn assert_draft(host: &Host, text: &str, value: f64) {
    let tool = host.app.doc.interaction.tool.as_ref().unwrap();
    assert_eq!(tool.draft, text);
    assert!(
        matches!(&tool.draft_status, DraftStatus::Valid(v) if (v.value - value).abs() < 1e-9),
        "{:?}",
        tool.draft_status
    );
}

pub(super) fn undo_depth(host: &Host) -> usize {
    host.app.doc.display.as_ref().unwrap().projection.undo_depth
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn reopened_consumed_extrude_profile_has_a_real_handle_and_draft_preview() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, context) = create_app(root.path()).await;
    context.enable_accesskit();
    let mut frame = eframe::Frame::_new_kittest();
    pump_until(&mut app, &context, &mut frame, |app| {
        app.doc.display.is_some()
    })
    .await;
    let path = root.path().join("qa/documents/QA-plate-hole.onecad");
    std::fs::write(
        &path,
        include_bytes!("../../onecad-app/tests/fixtures/qa_plate_hole.onecad"),
    )
    .unwrap();
    app.document_request(crate::document_flow::Request::Open(Some(path)));
    pump_until(&mut app, &context, &mut frame, |app| {
        !app.busy()
            && app
                .doc
                .display
                .as_ref()
                .is_some_and(|d| d.projection.features.len() >= 5)
    })
    .await;
    let mut host = Host {
        app,
        context,
        frame,
        size: egui::vec2(1280.0, 820.0),
        widgets: vec![],
    };
    host.app.ui.start_open = false;
    let record = host
        .app
        .doc
        .display
        .as_ref()
        .unwrap()
        .records
        .iter()
        .rev()
        .find(|r| {
            matches!(
                r.op,
                onecad_core::document::record::Operation::Known(
                    onecad_core::document::record::KnownOperation::Extrude(_)
                )
            )
        })
        .unwrap()
        .clone();
    let context = host.app.doc.interaction.context.clone().unwrap();
    host.app.doc.gesture += 1;
    host.app.dispatch(Action::BeginOperationEdit {
        record: Box::new(record),
        gesture: host.app.doc.gesture,
        context,
    });
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        app.doc.interaction.tool.as_ref().is_some_and(|t| t.ready())
    })
    .await;
    host.redraw();
    assert!(
        crate::extrude_handle::available(&host.app),
        "reopened consumed profile must retain a worker-derived anchor"
    );
    let tool = host.app.doc.interaction.tool.as_ref().unwrap();
    let onecad_core::document::record::Operation::Known(
        onecad_core::document::record::KnownOperation::Extrude(params),
    ) = tool.effective_op()
    else {
        panic!()
    };
    let wanted = params.profile.unwrap();
    let display = host.app.doc.display.as_ref().unwrap();
    assert!(
        !display
            .profiles
            .iter()
            .any(|p| p.reference.sketch == wanted.sketch && p.reference.region == wanted.region),
        "consumed profile must remain hidden and unpickable"
    );
    let anchor = display
        .profile_anchors
        .iter()
        .find(|p| p.reference.sketch == wanted.sketch && p.reference.region == wanted.region)
        .unwrap();
    host.app.doc.camera = Camera {
        target: anchor.base,
        offset: DVec3::new(70.0, -110.0, 90.0),
        projection: Projection::Perspective,
        ..Default::default()
    };
    host.app.doc.fit_pending = false;
    host.redraw();
    let rect = crate::layout::chrome(host.screen(), &host.app.ui).viewport;
    let start = crate::extrude_handle::testing_tip(&host.app, rect, 8.0).unwrap();
    let end = crate::extrude_handle::testing_tip(&host.app, rect, 20.0).unwrap();
    let undo = undo_depth(&host);
    let original = draft(&host);
    let sequence = host.app.doc.interaction.tool.as_ref().unwrap().tag.sequence;
    host.ui(vec![
        egui::Event::PointerMoved(start),
        button(start, true),
        button(start, false),
    ]);
    assert_eq!(draft(&host), original);
    assert_eq!(
        host.app.doc.interaction.tool.as_ref().unwrap().tag.sequence,
        sequence
    );
    host.ui(vec![
        egui::Event::PointerMoved(start),
        button(start, true),
        egui::Event::PointerMoved(end),
        button(end, false),
    ]);
    valid(&mut host, "20", 20.0).await;
    assert_eq!(
        undo_depth(&host),
        undo,
        "reopened edit drag authors a draft only"
    );
    assert!(crate::extrude_handle::available(&host.app));
    host.stop().await;
}
