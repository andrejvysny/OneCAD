use super::*;
use onecad_core::{
    document::Document,
    ids::DocumentId,
    io::container::{ContainerCaches, ContainerWriter, SaveMeta},
};
use std::path::{Path, PathBuf};

#[tokio::test]
#[ignore = "requires production worker and Metal; a fresh host and worker in the same test process"]
async fn finished_sketch_saves_in_place_and_reopens_with_same_region_and_identities() {
    let area = tempfile::tempdir().unwrap();
    let path = titled(area.path());
    let mut host = opened(area.path(), &path).await;
    host.app.new_sketch();
    open(&mut host).await;
    rectangle(&mut host).await;
    let sketch = host.sketch().id;
    let entities = ids(&host);
    host.control("Finish sketch");
    wait_closed(&mut host).await;
    let region = regions(&host);
    assert_eq!(region.len(), 1);
    assert!(host.app.document_dirty());
    let command = egui::Modifiers {
        command: true,
        mac_cmd: true,
        ..Default::default()
    };
    host.ui(vec![
        egui::Event::ModifiersChanged(command),
        key(egui::Key::S, command),
    ]);
    host.ui(vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)]);
    pump_until(&mut host.app, &host.context, &mut host.frame, |app| {
        !app.doc.documents.running && !app.document_dirty()
    })
    .await;
    assert!(
        host.app.doc.status.starts_with("Saved "),
        "{}",
        host.app.doc.status
    );
    host.stop().await;

    let mut host = opened(area.path(), &path).await;
    assert_eq!(regions(&host), region, "fresh host keeps the region id");
    assert!(!host.app.document_dirty());
    reenter(&mut host, sketch).await;
    assert_eq!(ids(&host), entities);
    host.control("Finish sketch");
    wait_closed(&mut host).await;
    assert_eq!(regions(&host), region);
    assert!(
        !host.app.document_dirty(),
        "no-op Finish keeps a clean file"
    );
    host.stop().await;
}

fn titled(root: &Path) -> PathBuf {
    let paths =
        QaPaths::from_args(["--qa-root".into(), root.join("qa").into_os_string()].into_iter())
            .unwrap();
    let destination = paths.root.join("documents/Finish reopen.onecad");
    std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
    ContainerWriter::save(
        &destination,
        &Document::new(DocumentId::new()),
        &ContainerCaches::none(),
        &SaveMeta {
            app_version: "sketch-finish-delivery".into(),
            created: "2026-10-21T00:00:00Z".into(),
            modified: "2026-10-21T00:00:00Z".into(),
            ..Default::default()
        },
    )
    .unwrap();
    destination
}

async fn opened(root: &Path, path: &Path) -> Host {
    let (mut app, context) = create_app(root).await;
    context.enable_accesskit();
    let mut frame = eframe::Frame::_new_kittest();
    pump_until(&mut app, &context, &mut frame, |app| {
        app.document_open() && app.doc.display.is_some() && !app.busy()
    })
    .await;
    app.document_request(Request::Open(Some(path.to_path_buf())));
    pump_until(&mut app, &context, &mut frame, |app| {
        app.doc
            .document_state
            .as_ref()
            .is_some_and(|state| state.path.is_some())
            && !app.busy()
    })
    .await;
    app.ui.start_open = false;
    app.ui.explorer_open = false;
    app.ui.inspector_open = false;
    let mut host = Host {
        app,
        context,
        frame,
        size: egui::vec2(1280.0, 820.0),
        widgets: vec![],
    };
    host.redraw();
    host
}
