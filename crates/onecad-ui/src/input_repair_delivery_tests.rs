//! Native repair selection/staging/card delivery with production worker and Metal.
use super::*;
use onecad_app::dto::FeatureStatus;
use onecad_interaction::{Action, NumericParameter, ParameterAxis};

async fn ready(host: &mut Host) {
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        app.doc
            .interaction
            .tool
            .as_ref()
            .is_some_and(|tool| tool.ready())
    })
    .await;
}
async fn committed(host: &mut Host, count: usize) {
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        app.doc.interaction.tool.is_none()
            && app
                .doc
                .display
                .as_ref()
                .is_some_and(|d| d.projection.features.len() == count)
    })
    .await;
    host.redraw();
}
fn repaired(host: &Host) -> bool {
    host.app
        .doc
        .display
        .as_ref()
        .unwrap()
        .projection
        .features
        .last()
        .unwrap()
        .status
        == FeatureStatus::Ok
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn repair_using_selection_stages_cancel_applies_undo_and_redo() {
    let root = tempfile::tempdir().unwrap();
    let mut host = boxed(root.path()).await;
    let faces = targets(&host, PickKind::Face);
    double_tap(&mut host, faces[0].0);
    let count = host.app.doc.display.as_ref().unwrap().records.len();
    tap(&mut host, &id::tool(ToolId::MoveRotate));
    enter(
        &mut host,
        &id::param(NumericParameter::TransformTranslation(ParameterAxis::Y)),
        "15",
    );
    ready(&mut host).await;
    tap(&mut host, id::OP_CONFIRM);
    committed(&mut host, count + 1).await;
    let moved = host
        .app
        .doc
        .display
        .as_ref()
        .unwrap()
        .records
        .last()
        .unwrap()
        .clone();
    host.app.doc.fit_pending = true;
    host.redraw();
    let edges = targets(&host, PickKind::Edge);
    click(&mut host, edges[0].0, egui::Modifiers::NONE);
    tap(&mut host, &id::tool(ToolId::Fillet));
    ready(&mut host).await;
    tap(&mut host, id::OP_CONFIRM);
    committed(&mut host, count + 2).await;
    let fillet = host
        .app
        .doc
        .display
        .as_ref()
        .unwrap()
        .records
        .last()
        .unwrap()
        .record_id;
    host.app.ui.inspector_open = true;
    host.app.ui.inspector_tab = crate::ui_state::InspectorTab::History;
    let context = host.app.doc.interaction.context.clone().unwrap();
    host.app.doc.gesture += 1;
    host.app.dispatch(Action::BeginParameterEdit {
        record: Box::new(moved),
        parameter: NumericParameter::TransformTranslation(ParameterAxis::Y),
        gesture: host.app.doc.gesture,
        context,
    });
    enter(
        &mut host,
        &id::param(NumericParameter::TransformTranslation(ParameterAxis::Y)),
        "40",
    );
    ready(&mut host).await;
    tap(&mut host, id::OP_CONFIRM);
    committed(&mut host, count + 2).await;
    assert!(!repaired(&host), "editing Move retains the safety gate");
    host.app.doc.fit_pending = true;
    host.redraw();
    let edges = targets(&host, PickKind::Edge);
    click(&mut host, edges[0].0, egui::Modifiers::NONE);
    host.control("Repair using selection");
    ready(&mut host).await;
    assert!(
        host.app
            .doc
            .interaction
            .tool
            .as_ref()
            .unwrap()
            .repair
            .is_some()
    );
    assert_eq!(
        host.app
            .doc
            .interaction
            .tool
            .as_ref()
            .unwrap()
            .params
            .active()
            .count(),
        0
    );
    assert!(!repaired(&host), "preview cannot clear authoritative gate");
    tap(&mut host, id::OP_CANCEL);
    assert!(host.app.doc.interaction.tool.is_none());
    assert!(!repaired(&host));
    host.control("Repair using selection");
    ready(&mut host).await;
    tap(&mut host, id::OP_CONFIRM);
    committed(&mut host, count + 2).await;
    assert!(repaired(&host));
    assert_eq!(
        host.app
            .doc
            .display
            .as_ref()
            .unwrap()
            .records
            .last()
            .unwrap()
            .record_id,
        fillet
    );
    host.app.submit(crate::bridge::Command::Undo);
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        !app.busy()
            && app.doc.display.as_ref().is_some_and(|d| {
                d.projection.features.last().unwrap().status == FeatureStatus::NeedsRepair
            })
    })
    .await;
    host.app.submit(crate::bridge::Command::Redo);
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        !app.busy()
            && app
                .doc
                .display
                .as_ref()
                .is_some_and(|d| d.projection.features.last().unwrap().status == FeatureStatus::Ok)
    })
    .await;
    host.stop().await;
}
