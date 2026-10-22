//! Production history-edit preview delivery; synthetic host adapters, not physical input.
use super::*;
use onecad_core::document::record::{KnownOperation, Operation};
use onecad_interaction::{Action, PreviewStatus};

fn thickness(scene: &onecad_scene::SceneSnapshot) -> f64 {
    let bounds = scene.bounds().unwrap();
    (bounds.max - bounds.min).min_element()
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic host adapters do not qualify physical input"]
async fn existing_extrude_numeric_preview_cancel_apply_undo_preserve_history() {
    let root = tempfile::tempdir().unwrap();
    let mut host = super::model_selection_delivery_tests::boxed(root.path()).await;
    let before = host.app.doc.display.clone().unwrap();
    let record = before
        .records
        .iter()
        .find(|record| matches!(record.op, Operation::Known(KnownOperation::Extrude(_))))
        .unwrap()
        .clone();
    assert!(
        (thickness(&before.scene) - 5.0).abs() < 1e-3,
        "baseline bounds: {:?}",
        before.scene.bounds()
    );
    open(&mut host, record.record_id).await;
    host.app.dispatch(Action::DraftChanged("8".into()));
    preview_ready(&mut host).await;
    let (_, preview) = host
        .app
        .doc
        .pending_preview
        .as_ref()
        .expect("checked replay geometry delivered");
    assert!((thickness(&preview.source.scene) - 8.0).abs() < 1e-3);
    assert_eq!(
        preview.suppressed,
        before
            .scene
            .bodies()
            .iter()
            .map(|body| body.body_id())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        host.app.doc.display.as_ref().unwrap().projection,
        before.projection
    );
    let dirty = host.app.document_dirty();
    let stale = host.app.doc.interaction.tool.as_ref().unwrap().tag.clone();
    host.redraw();
    assert_eq!(
        host.app.renderer.preview_identity().unwrap().generation,
        stale.sequence
    );
    host.app.dispatch(Action::Cancel);
    host.redraw();
    assert_eq!(host.app.renderer.preview_identity(), None);
    assert_eq!(
        host.app.doc.display.as_ref().unwrap().projection,
        before.projection
    );
    assert_eq!(host.app.document_dirty(), dirty);
    open(&mut host, record.record_id).await;
    host.app.dispatch(Action::DraftChanged("9".into()));
    preview_ready(&mut host).await;
    let current = host.app.doc.interaction.tool.as_ref().unwrap().tag.clone();
    assert_ne!(current, stale);
    host.app.dispatch(Action::EditPreviewFinished {
        tag: stale,
        result: Err(onecad_app::error::ApiError::Worker("late failure".into())),
    });
    assert_eq!(
        host.app.doc.interaction.tool.as_ref().unwrap().preview,
        PreviewStatus::Ready
    );
    host.app.dispatch(Action::Confirm);
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        app.doc.interaction.tool.is_none()
            && app.doc.display.as_ref().is_some_and(|display| {
                display.projection.undo_depth == before.projection.undo_depth + 1
            })
    })
    .await;
    assert!((thickness(&host.app.doc.display.as_ref().unwrap().scene) - 9.0).abs() < 1e-3);
    host.app.submit(crate::bridge::Command::Undo);
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        app.doc
            .display
            .as_ref()
            .is_some_and(|display| display.projection.undo_depth == before.projection.undo_depth)
    })
    .await;
    assert!((thickness(&host.app.doc.display.as_ref().unwrap().scene) - 5.0).abs() < 1e-3);
    host.stop().await;
}

async fn open(
    host: &mut super::point_center_delivery_tests::support::Host,
    record: onecad_core::ids::RecordId,
) {
    crate::pill::select_feature(&mut host.app, record);
    crate::inspector_history::edit_selected_feature(&mut host.app);
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        app.doc
            .interaction
            .tool
            .as_ref()
            .is_some_and(|tool| tool.editing_existing && tool.preview == PreviewStatus::Ready)
    })
    .await;
}

async fn preview_ready(host: &mut super::point_center_delivery_tests::support::Host) {
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        app.doc
            .interaction
            .tool
            .as_ref()
            .is_some_and(|tool| tool.preview == PreviewStatus::Ready)
    })
    .await;
}
