//! Move/Rotate gizmo drag lifecycle and multi-field feature re-edit through the
//! native host route; synthetic input with production worker and Metal, not
//! physical acceptance.
use super::extrude_handle_delivery_tests::{confirm, draft, extruding, undo_depth};
use super::point_center_delivery_tests::support::*;
use super::*;
use crate::bridge::Command;
use onecad_core::{
    document::{
        record::{KnownOperation, Operation, OperationRecord},
        variables::Scalar,
    },
    ids::{BodyId, RecordId},
};
use onecad_interaction::{Action, DraftStatus, NumericParameter, PreviewStatus};

const AUTHORED: &str = "6 * 2";

/// A published Extrude body under a fresh Move/Rotate session; returns the
/// screen position of the gizmo ball.
async fn moving(root: &std::path::Path) -> (Host, egui::Pos2) {
    let (mut host, _) = extruding(root, AUTHORED, 12.0).await;
    confirm(&mut host).await;
    host.redraw();
    let body: BodyId = host
        .app
        .doc
        .display
        .as_ref()
        .unwrap()
        .projection
        .bodies
        .keys()
        .next()
        .unwrap()
        .parse()
        .unwrap();
    host.app.doc.selected_body = Some(body);
    host.app.begin_transform();
    placed(&mut host).await;
    let ball = gizmo_ball(&host);
    (host, ball)
}

fn gizmo_ball(host: &Host) -> egui::Pos2 {
    let view = host.app.doc.view.unwrap();
    let rect = crate::layout::chrome(host.screen(), &host.app.ui).viewport;
    let triad = crate::transform_ui::triad(&host.app).unwrap();
    crate::gizmo::project(&view, rect, triad.center).unwrap()
}

/// Waits for an evaluated, previewed placement and redraws it.
async fn placed(host: &mut Host) {
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        app.doc.interaction.tool.as_ref().is_some_and(|tool| {
            matches!(tool.draft_status, DraftStatus::Valid(_))
                && tool.preview == PreviewStatus::Ready
        })
    })
    .await;
    host.redraw();
}

fn translation(host: &Host) -> [f64; 3] {
    let tool = host.app.doc.interaction.tool.as_ref().unwrap();
    let Operation::Known(KnownOperation::TransformBody(params)) = tool.effective_op() else {
        panic!("Move/Rotate session")
    };
    params.translate.map(|value| value.value)
}

fn gizmo_active(host: &Host) -> bool {
    host.app.doc.transform_gizmo.gizmo.active()
}

fn grab(host: &mut Host, ball: egui::Pos2) {
    host.ui(vec![
        egui::Event::PointerMoved(ball),
        button(ball, true),
        egui::Event::PointerMoved(ball + egui::vec2(60.0, 0.0)),
    ]);
    assert!(gizmo_active(host));
    assert_ne!(translation(host), [0.0; 3], "the drag authored a placement");
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn gizmo_pointer_loss_escape_and_tab_switch_restore_placement() {
    let root = tempfile::tempdir().unwrap();
    let (mut host, ball) = moving(root.path()).await;
    let undo = undo_depth(&host);
    for condition in 0..3 {
        grab(&mut host, ball);
        match condition {
            0 => host.ui(vec![egui::Event::PointerGone]),
            1 => host.ui(vec![key(egui::Key::Escape, egui::Modifiers::NONE)]),
            _ => {
                let tab = host.app.doc.tab;
                host.app.open_tab().unwrap();
                host.app.activate_tab(tab);
            }
        }
        assert!(!gizmo_active(&host), "condition {condition}");
        assert!(
            host.app.doc.interaction.tool.is_some(),
            "condition {condition}"
        );
        assert_eq!(translation(&host), [0.0; 3], "condition {condition}");
        assert_eq!(draft(&host), "0", "condition {condition}");
        host.ui(vec![button(ball, false)]);
        placed(&mut host).await;
        assert_eq!(undo_depth(&host), undo, "condition {condition}");
    }
    host.ui(vec![key(egui::Key::Escape, egui::Modifiers::NONE)]);
    assert!(
        host.app.doc.interaction.tool.is_none(),
        "Escape without a drag cancels the tool"
    );
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn gizmo_drags_then_one_apply_add_one_history_entry() {
    let root = tempfile::tempdir().unwrap();
    let (mut host, ball) = moving(root.path()).await;
    let undo = undo_depth(&host);
    grab(&mut host, ball);
    let end = ball + egui::vec2(60.0, 0.0);
    host.ui(vec![button(end, false)]);
    assert!(!gizmo_active(&host));
    let first = translation(&host);
    assert_ne!(first, [0.0; 3], "a changed release keeps the placement");
    placed(&mut host).await;
    // A click on the moved ball is not a drag: nothing more is authored.
    let moved = gizmo_ball(&host);
    let sequence = host.app.doc.interaction.tool.as_ref().unwrap().tag.sequence;
    host.ui(vec![
        egui::Event::PointerMoved(moved),
        button(moved, true),
        button(moved, false),
    ]);
    assert_eq!(
        host.app.doc.interaction.tool.as_ref().unwrap().tag.sequence,
        sequence
    );
    assert_eq!(translation(&host), first);
    assert_eq!(undo_depth(&host), undo, "dragging authors a draft only");
    confirm(&mut host).await;
    assert_eq!(undo_depth(&host), undo + 1, "one gesture, one entry");
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn gizmo_release_then_pointer_loss_in_one_batch_survives_a_tab_switch() {
    let root = tempfile::tempdir().unwrap();
    let (mut host, ball) = moving(root.path()).await;
    let undo = undo_depth(&host);
    grab(&mut host, ball);
    let dragged = translation(&host);
    let end = ball + egui::vec2(120.0, 0.0);
    host.ui(vec![
        egui::Event::PointerMoved(end),
        button(end, false),
        egui::Event::PointerGone,
    ]);
    assert!(!gizmo_active(&host));
    assert!(
        !host.app.doc.interaction.tool.as_ref().unwrap().dragging(),
        "the release closed the reducer drag"
    );
    let released = translation(&host);
    let length = |t: [f64; 3]| t.iter().map(|v| v * v).sum::<f64>().sqrt();
    assert!(
        length(released) > length(dragged) + 1e-6,
        "the release-frame motion is kept: {released:?} vs {dragged:?}"
    );
    let tab = host.app.doc.tab;
    host.app.open_tab().unwrap();
    host.app.activate_tab(tab);
    assert_eq!(translation(&host), released, "a released placement stays");
    placed(&mut host).await;
    assert_eq!(translation(&host), released);
    assert_eq!(undo_depth(&host), undo);
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn gizmo_camera_change_mid_drag_restores_placement() {
    let root = tempfile::tempdir().unwrap();
    let (mut host, ball) = moving(root.path()).await;
    let undo = undo_depth(&host);
    grab(&mut host, ball);
    host.app.doc.camera.offset *= 1.25;
    host.redraw();
    assert!(!gizmo_active(&host));
    assert_eq!(
        translation(&host),
        [0.0; 3],
        "a view change ends the drag as lost"
    );
    assert_eq!(draft(&host), "0");
    host.ui(vec![button(ball, false)]);
    placed(&mut host).await;
    assert_eq!(translation(&host), [0.0; 3]);
    assert_eq!(undo_depth(&host), undo);
    host.stop().await;
}

fn extrude_record(host: &Host) -> OperationRecord {
    host.app
        .doc
        .display
        .as_ref()
        .unwrap()
        .records
        .iter()
        .find(|record| matches!(record.op, Operation::Known(KnownOperation::Extrude(_))))
        .unwrap()
        .clone()
}

fn extrude_scalars(record: &OperationRecord) -> (Scalar, Scalar) {
    let Operation::Known(KnownOperation::Extrude(params)) = &record.op else {
        panic!("Extrude")
    };
    (params.distance.clone(), params.draft_angle_deg.clone())
}

/// The selector shows the focused label; the field caption below it does too,
/// so pick the one nearest a "Parameter" caption.
fn selector(host: &Host, focused: &str) -> egui::Pos2 {
    let captions: Vec<_> = host
        .widgets
        .iter()
        .filter(|(name, _)| name == "Parameter")
        .map(|(_, rect)| rect.center())
        .collect();
    host.widgets
        .iter()
        .filter(|(name, _)| name == focused)
        .map(|(_, rect)| rect.center())
        .min_by(|a, b| {
            let gap = |p: &egui::Pos2| {
                captions
                    .iter()
                    .map(|c| c.distance(*p))
                    .fold(f32::INFINITY, f32::min)
            };
            gap(a).total_cmp(&gap(b))
        })
        .expect("parameter selector")
}

async fn edit(host: &mut Host, record: RecordId) {
    host.app.doc.selected_feature = Some(record);
    crate::inspector_history::edit_selected_feature(&mut host.app);
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        app.doc
            .interaction
            .tool
            .as_ref()
            .is_some_and(|tool| tool.ready())
    })
    .await;
}

async fn evaluated(host: &mut Host, text: &str, value: f64) {
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        app.doc.interaction.tool.as_ref().is_some_and(|tool| {
            tool.draft == text
                && matches!(&tool.draft_status, DraftStatus::Valid(v) if (v.value - value).abs() < 1e-9)
                && tool.ready()
        })
    })
    .await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn reedit_two_fields_is_one_apply_and_one_undo_and_unchanged_apply_is_none() {
    let root = tempfile::tempdir().unwrap();
    let (mut host, _) = extruding(root.path(), AUTHORED, 12.0).await;
    confirm(&mut host).await;
    let before = extrude_record(&host);
    let undo = undo_depth(&host);
    let revision = host.app.doc.display.as_ref().unwrap().projection.revision;

    // Unchanged Apply closes the editor without a command.
    edit(&mut host, before.record_id).await;
    host.app.dispatch(Action::Confirm);
    assert!(host.app.doc.interaction.tool.is_none());
    host.redraw();
    let display = host.app.doc.display.as_ref().unwrap();
    assert_eq!(display.projection.undo_depth, undo);
    assert_eq!(display.projection.revision, revision);

    // Two fields, the selector switch between them, one Apply.
    edit(&mut host, before.record_id).await;
    host.app.dispatch(Action::DraftChanged("15".into()));
    evaluated(&mut host, "15", 15.0).await;
    // Switch through the selector, as a user does.
    host.ui(vec![]);
    let selector = selector(&host, NumericParameter::ExtrudeDistance.label());
    host.click(selector);
    host.control(NumericParameter::ExtrudeDraftAngle.label());
    assert_eq!(
        host.app
            .doc
            .interaction
            .tool
            .as_ref()
            .unwrap()
            .params
            .focus(),
        Some(NumericParameter::ExtrudeDraftAngle.into())
    );
    assert_eq!(
        draft(&host),
        "0",
        "switching keeps the session and its edits"
    );
    host.app.dispatch(Action::DraftChanged("3".into()));
    evaluated(&mut host, "3", 3.0).await;
    confirm(&mut host).await;
    assert_eq!(undo_depth(&host), undo + 1, "two fields, one entry");
    let (distance, draft_angle) = extrude_scalars(&extrude_record(&host));
    assert_eq!(distance, Scalar::with_expr(15.0, "15"));
    assert_eq!(draft_angle, Scalar::with_expr(3.0, "3"));

    crate::input::submit_history(&mut host.app, Command::Undo);
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        app.doc
            .display
            .as_ref()
            .is_some_and(|display| display.projection.undo_depth == undo)
    })
    .await;
    assert_eq!(
        extrude_scalars(&extrude_record(&host)),
        extrude_scalars(&before),
        "one Undo restores both fields"
    );
    host.stop().await;
}
