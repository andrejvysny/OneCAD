use super::*;
use crate::{
    api,
    host::{AppEvent, DialogPort, EventSink, FileDialog, HostContext, HostPaths},
};
use onecad_core::{
    document::{
        component::{ComponentDef, Occurrence, OccurrenceId},
        Document,
    },
    edit::AssemblyEdit,
    ids::DocumentId,
    io::container::{ContainerCaches, ContainerReader, ContainerWriter, SaveMeta},
};

struct Quiet;
impl EventSink for Quiet {
    fn emit(&self, _: AppEvent) -> Result<(), String> {
        Ok(())
    }
}
#[async_trait::async_trait]
impl DialogPort for Quiet {
    async fn pick(&self, _: FileDialog) -> Vec<String> {
        Vec::new()
    }
}

fn host(root: &Path) -> HostContext {
    HostContext {
        paths: HostPaths {
            app_data: Some(root.into()),
            app_config: Some(root.join("config")),
        },
        events: Arc::new(Quiet),
        dialogs: Arc::new(Quiet),
    }
}

fn fixture(path: &Path) -> (ComponentDefId, OccurrenceId) {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/qa_plate_hole.onecad");
    let loaded = ContainerReader::open(&source).unwrap();
    let source = loaded.document();
    let id = ComponentDefId(uuid::Uuid::new_v4());
    let occurrence = OccurrenceId(uuid::Uuid::new_v4());
    let mut component = ComponentDef::new(id, "Part");
    component.variables = source.variables.clone();
    component.sketches = source.sketches.clone();
    component.sketch_visibility = source.sketch_visibility.clone();
    component.datum_planes = source.datum_planes.clone();
    component.timeline = source.timeline.clone();
    let mut document = Document::new(DocumentId::new());
    document.definitions.insert(id, component);
    document
        .assembly
        .occurrences
        .insert(Occurrence {
            id: occurrence,
            def: id,
            parent: None,
            local: RigidTransform::IDENTITY,
            grounded: false,
            name: "Part".into(),
        })
        .unwrap();
    ContainerWriter::save(
        path,
        &document,
        &ContainerCaches::none(),
        &SaveMeta::default(),
    )
    .unwrap();
    (id, occurrence)
}

fn barrier(
    slot: &std::sync::Mutex<Option<PublishHook>>,
    session: &str,
) -> (
    tokio::sync::oneshot::Receiver<()>,
    tokio::sync::oneshot::Sender<()>,
) {
    let (started, wait) = tokio::sync::oneshot::channel();
    let (release, gate) = tokio::sync::oneshot::channel();
    *slot.lock().unwrap() = Some((session.into(), started, gate));
    (wait, release)
}

#[tokio::test]
#[ignore = "requires production worker; run with ONECAD_REQUIRE_WORKER=1 and ONECAD_WORKER_PATH"]
async fn stale_and_canceled_exports_preserve_target_and_drain_worker() {
    assert_eq!(std::env::var("ONECAD_REQUIRE_WORKER").as_deref(), Ok("1"));
    let area = tempfile::tempdir().unwrap();
    let document = area.path().join("assembly.onecad");
    let (definition, occurrence) = fixture(&document);
    let host = host(area.path());
    let state = Arc::new(AppState::default());
    state.start(host.clone()).unwrap();
    let opened = api::open_assembly_document(
        &state,
        host.clone(),
        document.to_string_lossy().into_owned(),
        None,
    )
    .await
    .unwrap();
    assert!(
        state
            .readiness()
            .wait_ready(std::time::Duration::from_secs(15))
            .await
    );
    let session = opened.runtime_session;
    api::regenerate_component(&state, &session, definition)
        .await
        .unwrap();
    let target = area.path().join("protected.step");
    std::fs::write(&target, b"old target").unwrap();

    let empty_definition = ComponentDefId::new();
    let empty_occurrence = OccurrenceId::new();
    api::apply_assembly_edit(
        &state,
        &session,
        AssemblyEdit::CreateDefinition {
            id: empty_definition,
            name: "Empty".into(),
        },
    )
    .await
    .unwrap();
    api::apply_assembly_edit(
        &state,
        &session,
        AssemblyEdit::InsertOccurrence {
            occurrence: Occurrence {
                id: empty_occurrence,
                def: empty_definition,
                parent: None,
                local: RigidTransform::IDENTITY,
                grounded: false,
                name: "Empty visible".into(),
            },
        },
    )
    .await
    .unwrap();
    let error = export(&state, target.clone()).await.unwrap_err();
    assert!(error.to_string().contains(&empty_occurrence.to_string()));
    assert_eq!(std::fs::read(&target).unwrap(), b"old target");
    api::apply_assembly_edit(
        &state,
        &session,
        AssemblyEdit::SetVisibility {
            occurrence: empty_occurrence,
            visible: false,
        },
    )
    .await
    .unwrap();

    let (started, release) = barrier(&PUBLISH_HOOK, &session);
    let task = {
        let state = state.clone();
        let target = target.clone();
        tokio::spawn(async move { export(&state, target).await })
    };
    started.await.unwrap();
    api::apply_assembly_edit(
        &state,
        &session,
        AssemblyEdit::SetVisibility {
            occurrence,
            visible: false,
        },
    )
    .await
    .unwrap();
    release.send(()).unwrap();
    assert!(matches!(
        task.await.unwrap(),
        Err(ApiError::StalePreview(_))
    ));
    assert_eq!(std::fs::read(&target).unwrap(), b"old target");

    api::apply_assembly_edit(
        &state,
        &session,
        AssemblyEdit::SetVisibility {
            occurrence,
            visible: true,
        },
    )
    .await
    .unwrap();
    let (started, release) = barrier(&PUBLISH_HOOK, &session);
    let task = {
        let state = state.clone();
        let target = target.clone();
        tokio::spawn(async move { export(&state, target).await })
    };
    started.await.unwrap();
    task.abort();
    let _ = task.await;
    release.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            if state.component_worker.lock().unwrap().is_none() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"old target");
    let (started, release) = barrier(&COMMITTED_HOOK, &session);
    let task = {
        let state = state.clone();
        let target = target.clone();
        tokio::spawn(async move { export(&state, target).await })
    };
    started.await.unwrap();
    task.abort();
    let _ = task.await;
    release.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            if state.component_worker.lock().unwrap().is_none() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(std::fs::read(&target).unwrap().starts_with(b"ISO-10303-21"));
    api::close_document(&state, host).await.unwrap();
    state.retire_all();
}
