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
        if condition < 2 {
            valid(&mut host, AUTHORED, 12.0).await;
        } else {
            // Escape is routed to the Extrude session first: it ends with no draft at all.
            assert!(host.app.doc.interaction.tool.is_none());
        }
        assert_eq!(undo_depth(&host), undo, "condition {condition}");
    }
    host.stop().await;
}

/// World axis of the extrude handle for screen projections.
struct Axis {
    base: DVec3,
    normal: DVec3,
}

impl Axis {
    fn at(&self, host: &Host, distance: f64) -> egui::Pos2 {
        let view = host.app.doc.view.unwrap();
        let rect = crate::layout::chrome(host.screen(), &host.app.ui).viewport;
        crate::gizmo::project(&view, rect, self.base + self.normal * distance).unwrap()
    }
}

async fn extruding(root: &std::path::Path, draft: &str, value: f64) -> (Host, Axis) {
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

async fn valid(host: &mut Host, text: &str, value: f64) {
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

async fn confirm(host: &mut Host) {
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

fn draft(host: &Host) -> String {
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

fn undo_depth(host: &Host) -> usize {
    host.app.doc.display.as_ref().unwrap().projection.undo_depth
}
