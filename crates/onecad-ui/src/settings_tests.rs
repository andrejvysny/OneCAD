use super::*;

fn load(directory: &tempfile::TempDir) -> (Store, UiState, Option<String>) {
    Store::load_at(directory.path(), egui::Context::default())
}

#[test]
fn roundtrip_preferences_without_transient_editor_state() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, mut ui, warning) = load(&directory);
    assert!(warning.is_none());
    ui.appearance = Appearance::System;
    ui.explorer_open = false;
    ui.inspector_open = false;
    ui.inspector_tab = InspectorTab::History;
    ui.recent_view = 1;
    ui.explorer_filter = "private filter".into();
    ui.start_open = true;
    ui.constraints_open = true;
    ui.workspace_menu_open = true;
    ui.theme_failed = Some(true);
    store.save_if_changed(&ui).unwrap();
    drop(store);
    let (_, actual, warning) = load(&directory);
    assert!(warning.is_none());
    assert_eq!(Preferences::capture(&actual), Preferences::capture(&ui));
    assert!(actual.explorer_filter.is_empty());
    assert!(!actual.start_open && !actual.constraints_open && !actual.workspace_menu_open);
    assert!(actual.theme_failed.is_none() && actual.display_menu.is_none());
}

#[test]
fn unchanged_or_transient_changes_never_create_settings() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, mut ui, _) = load(&directory);
    store.save_if_changed(&ui).unwrap();
    ui.explorer_filter = "search".into();
    ui.start_open = true;
    store.save_if_changed(&ui).unwrap();
    drop(store);
    assert!(!directory.path().join(FILE_NAME).exists());
}

#[test]
fn valid_loaded_preferences_are_not_rewritten_without_change() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(FILE_NAME);
    let mut preferences = Preferences::capture(&UiState::default());
    preferences.appearance = Appearance::Light;
    let bytes = serde_json::to_vec(&preferences).unwrap();
    fs::write(&path, &bytes).unwrap();
    let (mut store, mut ui, warning) = load(&directory);
    assert!(warning.is_none());
    ui.explorer_filter = "transient".into();
    store.save_if_changed(&ui).unwrap();
    assert!(store.writer.is_none());
    drop(store);
    assert_eq!(fs::read(path).unwrap(), bytes);
}

#[test]
fn corrupt_future_oversize_and_invalid_layout_stay_untouched() {
    let mut invalid = Preferences::capture(&UiState::default());
    invalid.recent_view = 2;
    for bytes in [
        b"invalid json".to_vec(),
        b"{\"version\":2,\"future\":true}".to_vec(),
        vec![b' '; MAX_BYTES as usize + 1],
        serde_json::to_vec(&invalid).unwrap(),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(FILE_NAME);
        fs::write(&path, &bytes).unwrap();
        let (mut store, mut ui, warning) = load(&directory);
        assert!(warning.is_some());
        ui.appearance = Appearance::Light;
        store.save_if_changed(&ui).unwrap();
        drop(store);
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
}

#[test]
fn final_change_survives_shutdown_while_prior_write_runs() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, mut ui, _) = load(&directory);
    ui.appearance = Appearance::Light;
    store.save_if_changed(&ui).unwrap();
    ui.appearance = Appearance::System;
    ui.inspector_tab = InspectorTab::History;
    store.save_if_changed(&ui).unwrap();
    drop(store);
    let (_, actual, _) = load(&directory);
    assert_eq!(Preferences::capture(&actual), Preferences::capture(&ui));
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn write_failure_is_reported_and_not_retried_each_frame() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, mut ui, _) = load(&directory);
    fs::create_dir(directory.path().join(FILE_NAME)).unwrap();
    ui.appearance = Appearance::Light;
    store.save_if_changed(&ui).unwrap();
    assert!(
        store
            .finish()
            .unwrap_err()
            .contains("Native settings save failed")
    );
    store.save_if_changed(&ui).unwrap();
    assert!(store.writer.is_none());
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn linked_settings_are_preserved_and_never_followed() {
    let directory = tempfile::tempdir().unwrap();
    let outside = tempfile::NamedTempFile::new().unwrap();
    fs::write(outside.path(), b"external data").unwrap();
    std::os::unix::fs::symlink(outside.path(), directory.path().join(FILE_NAME)).unwrap();
    let (mut store, mut ui, warning) = load(&directory);
    assert!(warning.is_some());
    ui.explorer_open = false;
    store.save_if_changed(&ui).unwrap();
    drop(store);
    assert_eq!(fs::read(outside.path()).unwrap(), b"external data");
    assert!(
        fs::symlink_metadata(directory.path().join(FILE_NAME))
            .unwrap()
            .is_symlink()
    );
}
