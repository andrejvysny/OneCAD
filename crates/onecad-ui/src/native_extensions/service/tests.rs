use super::*;

#[test]
fn application_paths_round_trip_per_profile_and_report_corruption() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.json");
    assert!(read_settings(&path).unwrap().is_empty());
    let settings = BTreeMap::from([("extension/app".into(), PathBuf::from("/Applications/App"))]);
    write_settings(&path, &settings).unwrap();
    assert_eq!(read_settings(&path).unwrap(), settings);
    fs::write(path, "broken json").unwrap();
    assert!(read_settings(&directory.path().join("settings.json")).is_err());
}

#[test]
fn uninstalling_a_bundled_package_survives_refresh() {
    let directory = tempfile::tempdir().unwrap();
    let bundled = directory.path().join("bundled");
    let package = bundled.join("test.addon");
    fs::create_dir_all(&package).unwrap();
    let manifest = serde_json::json!({
        "schema_version": 1, "id": "test.addon", "name": "Test", "version": "1.0",
        "runtime": "native_trusted", "entry": "extension.bin"
    });
    fs::write(
        package.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    fs::write(package.join("extension.bin"), b"fixture").unwrap();
    let mut registry = ExtensionRegistry::open(directory.path().join("installed")).unwrap();
    let record = directory.path().join("bundles.json");
    seed_bundled(&mut registry, &bundled, &record).unwrap();
    assert_eq!(registry.packages().len(), 1);
    assert!(matches!(
        registry.packages()[0].state,
        onecad_extension_host::PackageState::Disabled
    ));
    registry.set_enabled("test.addon", false, None).unwrap();
    seed_bundled(&mut registry, &bundled, &record).unwrap();
    assert_eq!(registry.packages().len(), 1);
    registry.uninstall_package("test.addon").unwrap();
    seed_bundled(&mut registry, &bundled, &record).unwrap();
    assert!(registry.packages().is_empty());
}

#[test]
fn malformed_installed_package_is_visible_as_discovery_error() {
    let directory = tempfile::tempdir().unwrap();
    fs::create_dir(directory.path().join("broken")).unwrap();
    let registry = ExtensionRegistry::open(directory.path()).unwrap();
    let snapshot = reply(&registry, BTreeMap::new(), "Refreshed".into());
    assert!(snapshot.warning);
    assert_eq!(snapshot.discovery_errors.len(), 1);
    assert!(snapshot.discovery_errors[0].contains("broken"));
}

fn installed_package(directory: &Path) -> (PathBuf, PathBuf) {
    let source = directory.join("source");
    fs::create_dir(&source).unwrap();
    let manifest = serde_json::json!({
        "schema_version": 1, "id": "test.addon", "name": "Test", "version": "1.0",
        "runtime": "native_trusted", "entry": "extension.bin",
        "applications": [{"id":"test.addon.app", "name":"Test application"}],
        "contributions": {"commands": [{"id":"test.addon.send", "title":"Send", "locations":["menu"]}]}
    });
    fs::write(
        source.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    fs::write(source.join("extension.bin"), b"fixture").unwrap();
    let installed = directory.join("installed");
    ExtensionRegistry::open(&installed)
        .unwrap()
        .install_package(&source)
        .unwrap();
    (source, installed)
}

#[test]
fn oversized_application_settings_preserve_the_previous_readable_file() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.json");
    let original = BTreeMap::from([("app/path".to_string(), PathBuf::from("/some/app"))]);
    write_settings(&path, &original).unwrap();
    let bytes = fs::read(&path).unwrap();
    let oversized = BTreeMap::from([("app/path".to_string(), "x".repeat(MAX_SETTINGS_BYTES))]);
    assert!(
        write_settings(&path, &oversized)
            .unwrap_err()
            .contains("size limit")
    );
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(read_settings(&path).unwrap(), original);
}

#[test]
fn corrupt_application_settings_cannot_block_disable_or_uninstall() {
    let directory = tempfile::tempdir().unwrap();
    let (source, installed) = installed_package(directory.path());
    let settings = directory.path().join("settings.json");
    fs::write(&settings, "broken json").unwrap();
    let (disabled, plan) = prepare(
        &installed,
        &settings,
        Request::SetEnabled {
            id: "test.addon".into(),
            enabled: false,
            approval: None,
        },
        None,
    )
    .unwrap();
    assert!(plan.is_none());
    assert!(disabled.warning && disabled.status.contains("disabled"));
    assert!(disabled.status.contains("Existing settings were preserved"));
    assert_eq!(
        disabled.packages[0].state,
        onecad_extension_host::PackageState::Disabled
    );
    assert_eq!(fs::read_to_string(&settings).unwrap(), "broken json");
    let (removed, _) = prepare(
        &installed,
        &settings,
        Request::Uninstall {
            id: "test.addon".into(),
        },
        None,
    )
    .unwrap();
    assert!(removed.warning && removed.status.contains("Uninstalled"));
    assert!(removed.packages.is_empty() && removed.commands.is_empty());
    assert!(!installed.join("test.addon").exists());
    assert!(source.is_dir());
    assert_eq!(fs::read_to_string(&settings).unwrap(), "broken json");
}

#[test]
fn corrupt_settings_refuse_execution_and_saves_without_destroying_configuration() {
    let directory = tempfile::tempdir().unwrap();
    let (_, installed) = installed_package(directory.path());
    let settings = directory.path().join("settings.json");
    fs::write(&settings, "broken json").unwrap();
    for request in [
        Request::Execute {
            command: "test.addon.send".into(),
            fingerprint: "reviewed content".into(),
            target: None,
            selection: vec![],
        },
        Request::SetApplication {
            extension: "test.addon".into(),
            application: "test.addon.app".into(),
            executable: None,
        },
    ] {
        let (reply, plan) = prepare(&installed, &settings, request, None).unwrap();
        assert!(reply.warning && reply.status.contains("not performed"));
        assert_eq!(reply.packages.len(), 1);
        assert!(plan.is_none());
        assert_eq!(fs::read_to_string(&settings).unwrap(), "broken json");
    }
}

#[test]
fn failed_management_returns_current_discovery_instead_of_stale_commands() {
    let directory = tempfile::tempdir().unwrap();
    let (_, installed) = installed_package(directory.path());
    let settings = directory.path().join("settings.json");
    assert_eq!(
        ExtensionRegistry::open(&installed)
            .unwrap()
            .commands()
            .len(),
        1
    );
    fs::write(installed.join("test.addon/manifest.json"), "broken json").unwrap();
    let (reply, plan) = prepare(
        &installed,
        &settings,
        Request::SetEnabled {
            id: "test.addon".into(),
            enabled: true,
            approval: None,
        },
        None,
    )
    .unwrap();
    assert!(reply.warning);
    assert!(reply.status.contains("not found"));
    assert!(reply.review_package.is_none());
    assert!(reply.commands.is_empty() && reply.packages.is_empty());
    assert_eq!(reply.discovery_errors.len(), 1);
    assert!(plan.is_none());
}

#[test]
fn resetting_corrupt_settings_retains_original_bytes_and_recovers_reads() {
    let directory = tempfile::tempdir().unwrap();
    let settings = directory.path().join("settings.json");
    let original = b"broken json that must remain recoverable";
    fs::write(&settings, original).unwrap();
    let (reply, plan) = prepare(
        &directory.path().join("installed"),
        &settings,
        Request::ResetApplications,
        None,
    )
    .unwrap();
    assert!(!reply.warning && plan.is_none());
    assert!(reply.status.contains("Previous settings retained"));
    assert!(read_settings(&settings).unwrap().is_empty());
    let backups = fs::read_dir(directory.path())
        .unwrap()
        .filter_map(|entry| {
            let path = entry.unwrap().path();
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with("backup.json")
                .then_some(path)
        })
        .collect::<Vec<_>>();
    assert_eq!(backups.len(), 1);
    assert_eq!(fs::read(&backups[0]).unwrap(), original);
}

#[test]
fn reset_backup_collision_preserves_both_existing_files() {
    let directory = tempfile::tempdir().unwrap();
    let settings = directory.path().join("settings.json");
    let backup = directory.path().join("backup.json");
    fs::write(&settings, "original").unwrap();
    fs::write(&backup, "previous backup").unwrap();
    assert!(reset_application_settings(&settings, &backup).is_err());
    assert_eq!(fs::read_to_string(settings).unwrap(), "original");
    assert_eq!(fs::read_to_string(backup).unwrap(), "previous backup");
}

#[test]
fn failed_settings_save_does_not_publish_an_unpersisted_application_path() {
    let directory = tempfile::tempdir().unwrap();
    let (_, installed) = installed_package(directory.path());
    let settings = directory.path().join("settings.json");
    let empty =
        serde_json::to_vec_pretty(&BTreeMap::from([("padding".to_string(), PathBuf::new())]))
            .unwrap()
            .len();
    let original = BTreeMap::from([(
        "padding".to_string(),
        PathBuf::from("x".repeat(MAX_SETTINGS_BYTES - empty - 1)),
    )]);
    write_settings(&settings, &original).unwrap();
    let executable = directory.path().join("slicer");
    fs::write(&executable, "fake executable").unwrap();
    let (reply, plan) = prepare(
        &installed,
        &settings,
        Request::SetApplication {
            extension: "test.addon".into(),
            application: "test.addon.app".into(),
            executable: Some(executable),
        },
        None,
    )
    .unwrap();
    assert!(reply.warning && reply.status.contains("size limit"));
    assert!(plan.is_none());
    assert_eq!(reply.applications, original);
    assert_eq!(read_settings(&settings).unwrap(), original);
}

#[test]
fn unavailable_settings_retire_command_access_without_forging_package_state() {
    let directory = tempfile::tempdir().unwrap();
    let (_, installed) = installed_package(directory.path());
    let registry = ExtensionRegistry::open(installed).unwrap();
    let mut snapshot = reply(&registry, Applications::new(), "Refreshed".into());
    snapshot.commands[0].enabled = true;
    let original_state = snapshot.packages[0].state.clone();
    let snapshot = settings_warning(snapshot, Some("Invalid application settings"));
    assert!(snapshot.warning);
    assert!(!snapshot.commands[0].enabled);
    assert_eq!(snapshot.packages[0].state, original_state);
}

#[test]
fn local_package_update_returns_new_disabled_snapshot_and_fences_stale_review() {
    let directory = tempfile::tempdir().unwrap();
    let (source, installed) = installed_package(directory.path());
    let settings = directory.path().join("settings.json");
    let old = ExtensionRegistry::open(&installed)
        .unwrap()
        .packages()
        .remove(0);
    let mut manifest: onecad_extension_api::Manifest =
        serde_json::from_slice(&fs::read(source.join("manifest.json")).unwrap()).unwrap();
    manifest.version = "2.0".into();
    fs::write(
        source.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    fs::write(source.join("extension.bin"), "updated fixture").unwrap();
    let request = Request::UpdateFolder {
        id: old.manifest.id,
        fingerprint: old.fingerprint.clone(),
    };
    let (updated, plan) = prepare(&installed, &settings, request.clone(), Some(&source)).unwrap();
    assert!(!updated.warning && plan.is_none());
    assert_eq!(updated.review_package.as_deref(), Some("test.addon"));
    assert!(updated.status.contains("Updated"));
    assert_eq!(updated.packages[0].manifest.version, "2.0");
    assert_ne!(updated.packages[0].fingerprint, old.fingerprint);
    assert_eq!(
        updated.packages[0].state,
        onecad_extension_host::PackageState::Disabled
    );
    assert!(updated.commands.iter().all(|command| !command.enabled));
    let fingerprint = updated.packages[0].fingerprint.clone();
    let (stale, plan) = prepare(&installed, &settings, request, Some(&source)).unwrap();
    assert!(stale.warning && plan.is_none());
    assert!(stale.review_package.is_none());
    assert!(!stale.status.starts_with("Updated"));
    assert_eq!(stale.packages[0].fingerprint, fingerprint);
    assert_eq!(stale.packages[0].manifest.version, "2.0");
}
