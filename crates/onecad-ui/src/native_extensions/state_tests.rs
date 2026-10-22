use super::*;

fn command() -> CommandContribution {
    CommandContribution {
        id: "test.send".into(),
        title: "Send".into(),
        description: String::new(),
        requires_document: true,
        requires_selection: false,
        menu: None,
        locations: vec!["export".into()],
    }
}

#[test]
fn command_contributions_respect_context_and_export_location() {
    let mut command = command();
    assert!(export_command(&command));
    assert!(!command_available(&command, false, false));
    assert!(command_available(&command, true, false));
    command.requires_selection = true;
    assert!(!command_available(&command, true, false));
    assert!(command_available(&command, true, true));
    command.locations.clear();
    assert!(!export_command(&command));
    command.menu = Some(CommandMenu::Export);
    assert!(export_command(&command));
}

#[test]
fn failed_management_does_not_claim_success_or_retry_each_frame() {
    let mut state = State {
        pending: true,
        attempted: true,
        ..Default::default()
    };
    state.failed("Invalid package".into());
    assert!(!state.pending);
    assert!(!state.loaded);
    assert!(state.attempted);
    assert_eq!(state.status, "Invalid package");
}

fn snapshot(generation: u64, enabled: bool) -> Reply {
    Reply {
        generation,
        packages: Vec::new(),
        commands: vec![RegisteredCommand {
            extension_id: "test".into(),
            command: command(),
            enabled,
        }],
        applications: BTreeMap::new(),
        status: format!("generation {generation}"),
        warning: false,
        discovery_errors: Vec::new(),
        review_package: None,
    }
}

#[test]
fn failed_guest_snapshot_removes_command_access() {
    let mut state = State::default();
    state.receive(snapshot(1, true));
    assert!(state.commands[0].enabled);
    let mut failed = snapshot(2, false);
    failed.status = "Extension sandbox failed".into();
    failed.warning = true;
    state.receive(failed);
    assert!(!state.commands[0].enabled);
    assert_eq!(state.status, "Extension sandbox failed");
}

#[test]
fn old_parked_tab_reply_cannot_reenable_commands() {
    let mut state = State::default();
    state.receive(snapshot(2, false));
    state.receive(snapshot(1, true));
    assert_eq!(state.generation, 2);
    assert!(!state.commands[0].enabled);
    assert_eq!(state.status, "generation 2");
}

fn package(fingerprint: &str) -> PackageSummary {
    PackageSummary {
        manifest: serde_json::from_value(serde_json::json!({
            "schema_version": 1, "id": "test", "name": "Test extension", "version": "1.0",
            "runtime": "portable_component", "entry": "addon.wasm", "capabilities": ["export_3mf"],
            "applications": [{"id": "app", "name": "Application"}],
            "contributions": {"commands": [{"id": "test.send", "title": "Send", "locations": ["export"]}]}
        })).unwrap(),
        fingerprint: fingerprint.into(), state: onecad_extension_host::PackageState::Disabled,
        path: "/tmp/test-extension".into(),
    }
}

fn package_snapshot(generation: u64, fingerprint: &str) -> Reply {
    let mut reply = snapshot(generation, false);
    reply.packages = vec![package(fingerprint)];
    reply
}

#[test]
fn menu_only_commands_have_a_distinct_menu_location() {
    let mut command = command();
    command.locations = vec!["menu".into()];
    assert!(menu_command(&command));
    assert!(!export_command(&command));
    command.locations.push("export".into());
    assert!(export_command(&command));
    assert!(!menu_command(&command));
    command.locations = vec!["palette".into()];
    assert!(!menu_command(&command));
    assert!(!export_command(&command));
}

#[test]
fn selection_commands_need_a_ready_document() {
    let mut command = command();
    command.requires_document = false;
    command.requires_selection = true;
    assert!(!command_available(&command, false, true));
    assert!(command_available(&command, true, true));
}

#[test]
fn changed_or_removed_package_closes_stale_permission_review() {
    let mut state = State {
        packages: vec![package("old")],
        review: Some(package("old")),
        ..Default::default()
    };
    state.receive(package_snapshot(1, "new"));
    assert!(state.review.is_none());
    state.review = Some(package("new"));
    state.receive(snapshot(2, false));
    assert!(state.review.is_none());
}

#[test]
fn update_opens_fresh_permission_review_for_exact_new_package() {
    let mut state = State {
        packages: vec![package("old")],
        pending: true,
        pending_request: Some(Request::UpdateFolder {
            id: "test".into(),
            fingerprint: "old".into(),
        }),
        ..Default::default()
    };
    let mut reply = package_snapshot(1, "new");
    reply.review_package = Some("test".into());
    state.receive(reply);
    assert_eq!(state.review.as_ref().unwrap().fingerprint, "new");
    assert!(!state.commands[0].enabled);
    assert!(!state.pending);
}

#[test]
fn unsaved_application_draft_survives_unrelated_snapshot() {
    let mut state = State::default();
    state.receive(package_snapshot(1, "package"));
    state
        .application_drafts
        .insert("test/app".into(), "/unsaved/path".into());
    state.receive(package_snapshot(2, "package"));
    assert_eq!(state.application_drafts["test/app"], "/unsaved/path");
    let mut saved = package_snapshot(3, "package");
    saved
        .applications
        .insert("test/app".into(), "/saved/path".into());
    state.receive(saved);
    assert_eq!(state.application_drafts["test/app"], "/saved/path");
}

#[test]
fn service_unavailable_retires_advertised_commands_until_refresh() {
    let mut state = State::default();
    state.receive(snapshot(1, true));
    state.pending = true;
    state.failed("Approval store cannot be read".into());
    assert!(!state.commands[0].enabled);
    assert!(state.status_warning);
    state.receive(snapshot(2, true));
    assert!(state.commands[0].enabled);
    assert!(!state.status_warning);
}

#[test]
fn manager_filters_real_manifest_fields_and_categories() {
    let mut state = State {
        packages: vec![package("one")],
        tab: ManagerTab::Discover,
        ..Default::default()
    };
    state.search = " TEST ".into();
    assert_eq!(manager::filtered(&state).len(), 1);
    state.category = 1;
    assert_eq!(manager::filtered(&state).len(), 1);
    state.search = "missing".into();
    assert!(manager::filtered(&state).is_empty());
    assert_eq!(manager::columns(580.0), 2);
    assert_eq!(manager::columns(280.0), 1);
}

#[test]
fn failed_update_never_inferrs_review_from_unrelated_content_change() {
    let mut state = State {
        packages: vec![package("old")],
        pending: true,
        pending_request: Some(Request::UpdateFolder {
            id: "test".into(),
            fingerprint: "old".into(),
        }),
        ..Default::default()
    };
    let mut reply = package_snapshot(1, "external-change");
    reply.warning = true;
    state.receive(reply);
    assert!(state.review.is_none());
}

#[test]
fn successful_identical_update_still_requires_fresh_approval() {
    let mut state = State {
        packages: vec![package("same")],
        pending: true,
        pending_request: Some(Request::UpdateFolder {
            id: "test".into(),
            fingerprint: "same".into(),
        }),
        ..Default::default()
    };
    let mut reply = package_snapshot(1, "same");
    reply.review_package = Some("test".into());
    state.receive(reply);
    assert_eq!(state.review.as_ref().unwrap().fingerprint, "same");
    assert!(!state.commands[0].enabled);
}
