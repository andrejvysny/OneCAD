use std::sync::{Arc, Mutex};

use super::*;
use crate::{document_runtime::DocumentRuntime, worker::PendingBackend};

fn target(rt: &DocumentRuntime) -> ExportTarget {
    ExportTarget {
        document_id: rt.document_id(),
        runtime_session: rt.runtime_session(),
        revision: rt.revision().0,
        snapshot_id: rt.head_snapshot_id().unwrap_or(SnapshotId(0)).0,
        worker_epoch: rt.fencing().get().1 .0,
        expected_base_hash: rt.export_history_hash().as_str().into(),
    }
}

#[tokio::test]
async fn missing_empty_and_stale_documents_refuse_before_writing() {
    let state = AppState::default();
    let backend = Arc::new(PendingBackend);
    let rt = DocumentRuntime::new_blank(backend.clone(), backend.clone(), backend);
    let mut expected = target(&rt);
    let directory = tempfile::tempdir().unwrap();
    let missing = export_3mf(
        &state,
        &expected,
        &ExportScope::Document,
        "x",
        directory.path(),
    )
    .await;
    assert!(matches!(missing, Err(ApiError::NoDocument(_))));
    *state.runtime.lock().await = Some(rt);
    let empty = export_3mf(
        &state,
        &expected,
        &ExportScope::Document,
        "x",
        directory.path(),
    )
    .await;
    assert!(
        matches!(empty, Err(ApiError::InvalidCommand(message)) if message.contains("no bodies"))
    );
    expected.snapshot_id = 42;
    let stale = export_3mf(
        &state,
        &expected,
        &ExportScope::Document,
        "x",
        directory.path(),
    )
    .await;
    assert!(matches!(stale, Err(ApiError::InvalidCommand(message)) if message.contains("stale")));
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn selection_deduplicates_in_document_order_and_rejects_unknown_bodies() {
    let a = BodyId(uuid::Uuid::from_u128(1));
    let b = BodyId(uuid::Uuid::from_u128(2));
    assert_eq!(
        select_bodies(vec![a, b], &ExportScope::Selection(vec![b, a, b])).unwrap(),
        vec![a, b]
    );
    assert!(select_bodies(vec![a], &ExportScope::Selection(vec![b])).is_err());
    assert!(select_bodies(vec![a], &ExportScope::Selection(vec![])).is_err());
}

struct FakeLauncher(Mutex<Vec<(PathBuf, PathBuf)>>);
impl ApplicationLauncher for FakeLauncher {
    fn launch(&self, executable: &Path, artifact: &Path) -> Result<(), String> {
        self.0
            .lock()
            .unwrap()
            .push((executable.into(), artifact.into()));
        Err("application failed to start".into())
    }
}

#[test]
fn unique_exports_and_failed_launch_preserve_files_and_literal_paths() {
    let root = tempfile::tempdir().unwrap();
    let name = safe_name("../../model;$(shell).3mf");
    let first = retain_export(root.path(), &name, b"completed export").unwrap();
    let second = retain_export(root.path(), &name, b"another export").unwrap();
    assert_ne!(first, second);
    assert_eq!(first.parent(), Some(root.path()));
    let executable = root.path().join("slicer path with spaces");
    std::fs::write(&executable, "").unwrap();
    let descriptor = ExternalApplicationDescriptor {
        id: "example-slicer".into(),
        display_name: "Example slicer".into(),
        macos_bundle_names: vec![],
        macos_executable_name: None,
    };
    let settings = ExternalApplicationSettings {
        executable: Some(executable.clone()),
    };
    let artifact = ExportArtifact {
        path: first.clone(),
        context: PreviewContext {
            document_id: "document".into(),
            runtime_session: "session".into(),
            revision: onecad_core::ids::DocumentRevision(3),
            worker_epoch: onecad_core::ids::WorkerEpoch(1),
            snapshot_id: SnapshotId(1),
            expected_base_hash: onecad_core::regen::HistoryPrefixHash::empty(),
        },
        body_count: 1,
        target: ExportTarget {
            document_id: "document".into(),
            runtime_session: "session".into(),
            revision: 3,
            snapshot_id: 1,
            worker_epoch: 1,
            expected_base_hash: onecad_core::regen::HistoryPrefixHash::empty()
                .as_str()
                .into(),
        },
    };
    let launcher = FakeLauncher(Mutex::new(vec![]));
    assert_eq!(
        launch_artifact(&descriptor, &settings, &artifact, &launcher),
        LaunchOutcome::Failed("application failed to start".into())
    );
    assert_eq!(
        *launcher.0.lock().unwrap(),
        vec![(executable, first.clone())]
    );
    assert_eq!(std::fs::read(first).unwrap(), b"completed export");
    assert!(second.is_file());
    let relative = ExternalApplicationSettings {
        executable: Some("slicer".into()),
    };
    assert!(matches!(
        launch_artifact(&descriptor, &relative, &artifact, &launcher),
        LaunchOutcome::Failed(_)
    ));
    assert_eq!(launcher.0.lock().unwrap().len(), 1);
}
