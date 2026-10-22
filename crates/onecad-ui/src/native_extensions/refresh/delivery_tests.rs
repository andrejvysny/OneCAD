use super::*;
use crate::native_extensions::Request as ExtensionRequest;

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn catalog_refresh_keeps_immediate_modeling_input_available() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, context) = create_app(root.path()).await;
    let mut frame = eframe::Frame::_new_kittest();
    pump_until(&mut app, &context, &mut frame, |app| {
        app.doc.display.is_some()
    })
    .await;
    assert!(!app.busy());
    app.extension_request(ExtensionRequest::Refresh);
    assert!(app.ui.extensions.native.pending);
    assert!(!app.doc.documents.running);
    assert!(!app.busy());
    app.new_sketch();
    pump_until(&mut app, &context, &mut frame, |app| app.sketch_active()).await;
    assert!(app.doc.sketch.session.is_some());
    app.stop_discard();
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal; synthetic native input is not physical acceptance"]
async fn management_waits_for_refresh_then_keeps_document_serialization() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, context) = create_app(root.path()).await;
    let mut frame = eframe::Frame::_new_kittest();
    pump_until(&mut app, &context, &mut frame, |app| {
        app.doc.display.is_some()
    })
    .await;
    app.extension_request(ExtensionRequest::Refresh);
    app.extension_request(ExtensionRequest::Refresh);
    app.extension_request(ExtensionRequest::ResetApplications);
    assert_eq!(
        app.ui.extensions.native.pending_request,
        Some(ExtensionRequest::Refresh)
    );
    assert!(!app.doc.documents.running);
    pump_until(&mut app, &context, &mut frame, |app| {
        !app.ui.extensions.native.pending
    })
    .await;
    app.extension_request(ExtensionRequest::ResetApplications);
    assert!(app.doc.documents.running);
    assert!(app.busy());
    app.extension_request(ExtensionRequest::Refresh);
    assert_eq!(
        app.ui.extensions.native.pending_request,
        Some(ExtensionRequest::ResetApplications)
    );
    pump_until(&mut app, &context, &mut frame, |app| {
        !app.ui.extensions.native.pending
    })
    .await;
    assert!(!app.doc.documents.running);
    assert_eq!(app.ui.extensions.native.status, "Application paths reset");
    app.stop_discard();
    pump_until(&mut app, &context, &mut frame, |app| app.may_close).await;
}
