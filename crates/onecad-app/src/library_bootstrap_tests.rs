use super::*;

struct Quiet;
impl crate::host::EventSink for Quiet {
    fn emit(&self, _: crate::host::AppEvent) -> Result<(), String> {
        Ok(())
    }
}
#[async_trait::async_trait]
impl crate::host::DialogPort for Quiet {
    async fn pick(&self, _: crate::host::FileDialog) -> Vec<String> {
        Vec::new()
    }
}

#[tokio::test]
async fn publishing_waits_for_bootstrap_and_propagates_failure_before_export() {
    let state = AppState::default();
    let (sender, receiver) = tokio::sync::watch::channel(None);
    state.library_initialization.set(receiver).unwrap();
    let host = HostContext {
        paths: Default::default(),
        events: Arc::new(Quiet),
        dialogs: Arc::new(Quiet),
    };
    let save = save_as_component(host, &state, String::new(), Default::default(), None, None);
    tokio::pin!(save);
    assert!(tokio::time::timeout(Duration::from_millis(20), &mut save)
        .await
        .is_err());
    sender.send_replace(Some(Err("seed failed".into())));
    assert!(matches!(save.await, Err(ApiError::Internal(message)) if message == "seed failed"));
}

fn expect_seed(root: &Path) {
    let detail = component_detail_at(root, "onecad.std.iso4762", "1.0.1").unwrap();
    assert_eq!(detail.component.version, "1.0.1");
}

#[test]
fn seed_marker_without_index_is_repaired_before_return() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    crate::library_seed::seed_library(root).unwrap();
    let manifest = root.join("onecad.std.iso4762/component.toml");
    let original = std::fs::read(&manifest).unwrap();
    assert!(!root.join(onecad_library::index::INDEX_FILE).exists());
    seed_and_reindex_at(root).unwrap();
    expect_seed(root);
    assert_eq!(std::fs::read(manifest).unwrap(), original);
}

#[test]
fn unchanged_seed_with_empty_index_is_repaired_without_rewriting_packages() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    crate::library_seed::seed_library(root).unwrap();
    let manifest = root.join("onecad.std.iso4762/component.toml");
    let original = std::fs::read(&manifest).unwrap();
    LibraryIndex::default()
        .save(&root.join(onecad_library::index::INDEX_FILE))
        .unwrap();
    seed_and_reindex_at(root).unwrap();
    expect_seed(root);
    assert_eq!(std::fs::read(manifest).unwrap(), original);
}

#[test]
fn concurrent_bootstraps_return_a_complete_readable_index() {
    let directory = tempfile::tempdir().unwrap();
    let start = std::sync::Barrier::new(8);
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                start.wait();
                seed_and_reindex_at(directory.path()).unwrap();
                expect_seed(directory.path());
            });
        }
    });
}

#[test]
fn future_index_refuses_before_seeding_and_preserves_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(onecad_library::index::INDEX_FILE);
    let index = LibraryIndex {
        index_version: 2,
        ..LibraryIndex::default()
    };
    index.save(&path).unwrap();
    let original = std::fs::read(&path).unwrap();
    assert!(seed_and_reindex_at(directory.path())
        .unwrap_err()
        .contains("version 2"));
    assert_eq!(std::fs::read(path).unwrap(), original);
    assert!(!directory.path().join("onecad.std.iso4762").exists());
}

#[test]
fn failed_marker_write_keeps_an_existing_readable_index_available() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    seed_and_reindex_at(root).unwrap();
    let path = root.join(onecad_library::index::INDEX_FILE);
    let original = std::fs::read(&path).unwrap();
    let marker = root.join(".seed-version");
    std::fs::remove_file(&marker).unwrap();
    std::fs::create_dir(marker).unwrap();
    seed_and_reindex_at(root).unwrap();
    expect_seed(root);
    assert_eq!(std::fs::read(path).unwrap(), original);
}
