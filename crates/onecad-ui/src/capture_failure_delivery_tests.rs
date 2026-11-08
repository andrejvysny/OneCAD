use super::model_selection_delivery_tests::boxed;
use super::point_center_delivery_tests::support::Host;
use super::*;
use onecad_core::document::{
    Document,
    component::{
        ComponentDef, ComponentDefId, Occurrence, OccurrenceId, Quat, RigidTransform, Vec3d,
    },
};
use onecad_core::ids::DocumentId;
use onecad_core::io::container::{ContainerCaches, ContainerReader, ContainerWriter, SaveMeta};
use std::path::Path;

fn fixture(path: &Path, x: f64) {
    let source = ContainerReader::open(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../onecad-app/tests/fixtures/qa_plate_hole.onecad"),
    )
    .unwrap();
    let source = source.document();
    let mut document = Document::new(DocumentId::new());
    let definition = ComponentDefId(uuid::Uuid::new_v4());
    let mut part = ComponentDef::new(definition, "Plate");
    part.variables = source.variables.clone();
    part.sketches = source.sketches.clone();
    part.sketch_visibility = source.sketch_visibility.clone();
    part.datum_planes = source.datum_planes.clone();
    part.timeline = source.timeline.clone();
    document.definitions.insert(definition, part);
    document
        .assembly
        .occurrences
        .insert(Occurrence {
            id: OccurrenceId(uuid::Uuid::new_v4()),
            def: definition,
            parent: None,
            local: RigidTransform::new(Quat::IDENTITY, Vec3d::new(x, 0.0, 0.0)),
            grounded: false,
            name: "Far plate".into(),
        })
        .unwrap();
    ContainerWriter::save(
        path,
        &document,
        &ContainerCaches::none(),
        &SaveMeta::default(),
    )
    .unwrap();
}

async fn wait(host: &mut Host, done: impl Fn(&App) -> bool) {
    for _ in 0..4000 {
        logic_tick(&mut host.app, &host.context, &mut host.frame, false);
        host.ui(vec![]);
        if done(&host.app) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("host did not settle: {}", host.app.doc.status);
}

#[tokio::test]
#[ignore = "requires production worker and Metal"]
async fn capture_refusal_remains_visible_until_replacement() {
    assert_eq!(std::env::var("ONECAD_REQUIRE_WORKER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("ONECAD_REQUIRE_GPU").as_deref(), Ok("1"));
    let root = tempfile::tempdir().unwrap();
    let mut host = boxed(root.path()).await;
    let path = root.path().join("qa/documents/far.onecad");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    fixture(&path, 2048.0);
    host.app.document_request(Request::Open(Some(path.clone())));
    wait(&mut host, |app| {
        app.doc.display_error.is_some() && !app.busy()
    })
    .await;
    let error = host.app.doc.display_error.clone().unwrap();
    assert!(
        error.contains("occurrence exceeds 1/1024 mm GPU precision bound"),
        "{error}"
    );
    assert!(host.app.doc.display.is_none());
    assert!(host.app.document_open());
    let state = host.app.doc.document_state.as_ref().unwrap();
    let authoritative = host.app.doc.bridge.shared.test_document_state().await;
    eprintln!("UI state: {state:?}; authoritative state: {authoritative:?}");
    assert!(authoritative.dirty);
    assert!(state.dirty);
    let saved_path = std::path::PathBuf::from(state.path.as_ref().unwrap());
    host.app.ui.explorer_open = true;
    for _ in 0..10 {
        host.ui(vec![]);
        assert_eq!(host.app.doc.display_error.as_deref(), Some(error.as_str()));
    }
    assert!(
        host.widgets
            .iter()
            .any(|(label, _)| label == "Geometry unavailable")
    );
    assert!(
        host.widgets
            .iter()
            .any(|(label, _)| label == "Model unavailable")
    );
    assert!(
        !host
            .widgets
            .iter()
            .any(|(label, _)| label == "Opening project…")
    );
    assert!(host.widgets.iter().any(|(label, _)| label == &error));
    assert!(
        !host
            .widgets
            .iter()
            .any(|(label, _)| label == "Opening document…")
    );
    host.app.document_request(Request::Save { save_as: false });
    wait(&mut host, |app| !app.busy()).await;
    assert!(!host.app.doc.document_state.as_ref().unwrap().dirty);
    let saved = ContainerReader::open(&saved_path).unwrap();
    let document = saved.document();
    assert_eq!(document.definitions.len(), 1);
    assert_eq!(document.assembly.occurrences.iter().count(), 1);
    let part = document.definitions.values().next().unwrap();
    assert!(!part.timeline.records().is_empty());
    assert!(!part.elements.is_empty());
    assert!(!part.bodies.is_empty());
    assert!(host.app.doc.display_error.is_some());
    host.app.document_request(Request::New);
    wait(&mut host, |app| app.doc.display.is_some() && !app.busy()).await;
    assert!(host.app.doc.display_error.is_none());
    host.stop().await;
}

#[tokio::test]
#[ignore = "requires production worker and Metal"]
async fn accepted_scene_failure_exposes_error_and_blocks_stale_picks() {
    assert_eq!(std::env::var("ONECAD_REQUIRE_WORKER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("ONECAD_REQUIRE_GPU").as_deref(), Ok("1"));
    let root = tempfile::tempdir().unwrap();
    let mut host = boxed(root.path()).await;
    let path = root.path().join("qa/documents/near.onecad");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    fixture(&path, 0.0);
    host.app.document_request(Request::Open(Some(path)));
    wait(&mut host, |app| {
        app.doc
            .display
            .as_ref()
            .is_some_and(|d| d.assembly.occurrences.len() == 1)
            && !app.busy()
    })
    .await;
    let occurrence = OccurrenceId(
        uuid::Uuid::parse_str(&host.app.doc.display.as_ref().unwrap().assembly.occurrences[0].id)
            .unwrap(),
    );
    assert!(host.app.doc.bridge.valid());
    let pick =
        super::model_selection_delivery_tests::targets(&host, onecad_scene::PickKind::Face)[0].0;
    crate::assembly_controls::submit(
        &mut host.app,
        crate::assembly_controls::Action::Edit(onecad_core::edit::AssemblyEdit::SetPose {
            occurrence,
            pose: RigidTransform::new(Quat::IDENTITY, Vec3d::new(2048.0, 0.0, 0.0)),
        }),
    );
    wait(&mut host, |app| {
        app.doc.display_error.is_some() && !app.doc.assembly_controls.pending
    })
    .await;
    assert!(host.app.doc.display.is_some());
    assert!(!host.app.doc.bridge.valid());
    let error = host.app.doc.display_error.clone().unwrap();
    assert!(error.contains("occurrence exceeds 1/1024 mm GPU precision bound"));
    host.app.ui.explorer_open = true;
    for _ in 0..10 {
        host.ui(vec![]);
    }
    assert!(
        host.widgets
            .iter()
            .any(|(label, _)| label == "Geometry unavailable")
    );
    assert!(host.widgets.iter().any(|(label, _)| label == &error));
    super::model_selection_delivery_tests::click(&mut host, pick, egui::Modifiers::NONE);
    assert!(host.app.doc.selection.items().is_empty());
    assert!(host.app.doc.topology.occurrence.is_none());
    host.app.document_request(Request::New);
    wait(&mut host, |app| {
        app.doc.display.is_some() && app.doc.bridge.valid() && !app.busy()
    })
    .await;
    assert!(host.app.doc.display_error.is_none());
    host.stop().await;
}
