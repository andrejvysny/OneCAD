use super::*;

fn installed(root: &Path, source: &Path) -> ExtensionRegistry {
    write_package(
        source,
        &manifest(ID, COMMAND),
        &component("{}", "i32.const 0", false),
    );
    let mut registry = ExtensionRegistry::open(root).unwrap();
    registry.install_package(source).unwrap();
    registry
        .set_enabled(ID, true, Some(approval(&registry)))
        .unwrap();
    registry
}

fn replacement(source: &Path) {
    let mut next = manifest(ID, &format!("{ID}.updated"));
    next.version = "2.0.0".into();
    write_package(
        source,
        &next,
        &component(r#"{"message":"updated"}"#, "i32.const 0", false),
    );
}

#[test]
fn invalid_or_wrong_id_replacement_preserves_enabled_package_and_approval() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let root = temp.path().join("installed");
    let mut registry = installed(&root, &source);
    let fingerprint = registry.packages()[0].fingerprint.clone();
    let original_grants = fs::read(root.join(".approvals.json")).unwrap();
    write_package(&source, &manifest(ID, COMMAND), b"\0asm");
    assert!(registry.update_package(ID, &fingerprint, &source).is_err());
    write_package(
        &source,
        &manifest("other.addon", "other.addon.send"),
        &component("{}", "i32.const 0", false),
    );
    assert!(registry.update_package(ID, &fingerprint, &source).is_err());
    assert_eq!(
        fs::read(root.join(".approvals.json")).unwrap(),
        original_grants
    );
    assert_eq!(registry.packages()[0].state, PackageState::Enabled);
    assert!(registry.execute(COMMAND, &context()).is_ok());
    assert!(ExtensionRegistry::open(&root)
        .unwrap()
        .execute(COMMAND, &context())
        .is_ok());
}

#[test]
fn successful_update_replaces_commands_and_revokes_grants_across_reopen() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let root = temp.path().join("installed");
    let mut registry = installed(&root, &source);
    let old_approval = approval(&registry);
    replacement(&source);
    registry
        .update_package(ID, &old_approval.fingerprint, &source)
        .unwrap();
    assert_eq!(registry.packages()[0].state, PackageState::Disabled);
    assert_eq!(registry.packages()[0].manifest.version, "2.0.0");
    assert!(matches!(
        registry.execute(COMMAND, &context()),
        Err(HostError::NotFound(_))
    ));
    let mut reopened = ExtensionRegistry::open(&root).unwrap();
    assert_eq!(reopened.packages()[0].state, PackageState::Disabled);
    assert_eq!(reopened.commands()[0].command.id, format!("{ID}.updated"));
    assert!(matches!(
        reopened.set_enabled(ID, true, None),
        Err(HostError::PermissionDenied(_))
    ));
    assert!(matches!(
        reopened.set_enabled(ID, true, Some(old_approval)),
        Err(HostError::ApprovalChanged)
    ));
    reopened
        .set_enabled(ID, true, Some(approval(&reopened)))
        .unwrap();
    assert_eq!(
        reopened
            .execute(&format!("{ID}.updated"), &context())
            .unwrap()
            .message
            .as_deref(),
        Some("updated")
    );
}

#[test]
fn stale_review_cannot_replace_a_newer_package() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let root = temp.path().join("installed");
    let mut registry = installed(&root, &source);
    let stale = registry.packages()[0].fingerprint.clone();
    replacement(&source);
    registry.update_package(ID, &stale, &source).unwrap();
    let current = registry.packages()[0].fingerprint.clone();
    assert!(matches!(
        registry.update_package(ID, &stale, &source),
        Err(HostError::ApprovalChanged)
    ));
    assert_eq!(registry.packages()[0].fingerprint, current);
    assert_eq!(registry.packages()[0].state, PackageState::Disabled);
}

#[test]
fn update_command_collisions_preserve_previous_enabled_version() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let root = temp.path().join("installed");
    let mut registry = installed(&root, &source);
    let fingerprint = registry.packages()[0].fingerprint.clone();
    let conflicting = "test.addon.child.send";
    write_package(
        &source,
        &manifest("test.addon.child", conflicting),
        &component("{}", "i32.const 0", false),
    );
    registry.install_package(&source).unwrap();
    write_package(
        &source,
        &manifest(ID, conflicting),
        &component("{}", "i32.const 0", false),
    );
    assert!(matches!(
        registry.update_package(ID, &fingerprint, &source),
        Err(HostError::InvalidPackage(_))
    ));
    assert!(registry.execute(COMMAND, &context()).is_ok());
}

#[test]
fn persistence_failure_rolls_back_replacement_and_retains_old_grants() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let root = temp.path().join("installed");
    let mut registry = installed(&root, &source);
    let fingerprint = registry.packages()[0].fingerprint.clone();
    let original_grants = fs::read(root.join(".approvals.json")).unwrap();
    replacement(&source);
    fs::create_dir(root.join(".approvals.tmp")).unwrap();
    assert!(registry.update_package(ID, &fingerprint, &source).is_err());
    assert_eq!(
        fs::read(root.join(".approvals.json")).unwrap(),
        original_grants
    );
    assert!(registry.execute(COMMAND, &context()).is_ok());
    fs::remove_dir(root.join(".approvals.tmp")).unwrap();
    let mut reopened = ExtensionRegistry::open(&root).unwrap();
    assert_eq!(reopened.packages()[0].fingerprint, fingerprint);
    assert_eq!(reopened.packages()[0].state, PackageState::Enabled);
    assert!(reopened.execute(COMMAND, &context()).is_ok());
    assert!(!fs::read_dir(&root).unwrap().any(|entry| entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".update-")));
}
