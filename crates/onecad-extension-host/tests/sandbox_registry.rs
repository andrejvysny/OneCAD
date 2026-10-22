use onecad_extension_host::{
    api::*, Approval, ExtensionRegistry, HostError, PackageState, SandboxLimits,
};
use std::{fs, path::Path};

#[path = "sandbox_registry/update.rs"]
mod update;

const ID: &str = "test.addon";
const COMMAND: &str = "test.addon.send";

fn component(response: &str, body: &str, err: bool) -> Vec<u8> {
    component_with_start(response, body, err, "")
}

fn component_with_start(response: &str, body: &str, err: bool, start: &str) -> Vec<u8> {
    let mut record = vec![u8::from(err), 0, 0, 0];
    record.extend_from_slice(&1024u32.to_le_bytes());
    record.extend_from_slice(&(response.len() as u32).to_le_bytes());
    let escaped = |bytes: &[u8]| {
        bytes
            .iter()
            .map(|b| format!("\\{b:02x}"))
            .collect::<String>()
    };
    wat::parse_str(format!(
        r#"
(component
  (core module $m
    (memory (export "memory") 1)
    (global $heap (mut i32) (i32.const 4096))
    (data (i32.const 0) "{}")
    (data (i32.const 1024) "{}")
    (func (export "realloc") (param i32 i32 i32 i32) (result i32)
      (local $p i32)
      global.get $heap local.tee $p local.get 3 i32.add global.set $heap local.get $p)
    (func (export "execute") (param i32 i32 i32 i32) (result i32) {}) {} )
  (core instance $i (instantiate $m))
  (alias core export $i "memory" (core memory $mem))
  (alias core export $i "realloc" (core func $realloc))
  (alias core export $i "execute" (core func $execute))
  (func (export "execute") (param "command" string) (param "context-json" string)
    (result (result string (error string)))
    (canon lift (core func $execute) (memory $mem) (realloc $realloc))))
"#,
        escaped(&record),
        escaped(response.as_bytes()),
        body,
        start
    ))
    .unwrap()
}

fn manifest(id: &str, command: &str) -> Manifest {
    Manifest {
        schema_version: SCHEMA_VERSION,
        id: id.into(),
        name: "Test addon".into(),
        version: "1.0.0".into(),
        description: String::new(),
        runtime: ExtensionRuntime::PortableComponent,
        entry: "extension.wasm".into(),
        capabilities: vec![Capability::Export3mf, Capability::LaunchApplication],
        applications: vec![ApplicationContribution {
            id: format!("{id}.application"),
            name: "Desktop application".into(),
            macos_bundle_name: None,
        }],
        contributions: Contributions {
            commands: vec![CommandContribution {
                id: command.into(),
                title: "Send model".into(),
                description: String::new(),
                requires_document: true,
                requires_selection: false,
                menu: Some(CommandMenu::Export),
                locations: vec!["export".into(), "palette".into()],
            }],
        },
    }
}

fn write_package(source: &Path, manifest: &Manifest, bytes: &[u8]) {
    fs::create_dir_all(source).unwrap();
    fs::write(
        source.join("manifest.json"),
        serde_json::to_vec_pretty(manifest).unwrap(),
    )
    .unwrap();
    fs::write(source.join("extension.wasm"), bytes).unwrap();
}

fn approval(registry: &ExtensionRegistry) -> Approval {
    let package = &registry.packages()[0];
    Approval {
        fingerprint: package.fingerprint.clone(),
        capabilities: package.manifest.capabilities.clone(),
        applications: package
            .manifest
            .applications
            .iter()
            .map(|a| a.id.clone())
            .collect(),
        trust_native: false,
    }
}

fn context() -> CommandContext {
    CommandContext {
        has_document: true,
        has_selection: true,
    }
}

fn plan() -> String {
    serde_json::to_string(&CommandExecution {
        requests: vec![
            HostRequest::Export3mf {
                scope: ExportScope::Selection,
                suggested_name: "model.3mf".into(),
            },
            HostRequest::LaunchApplication {
                application: format!("{ID}.application"),
                artifact: ArtifactRef::PreviousExport,
            },
        ],
        message: Some("Ready".into()),
    })
    .unwrap()
}

#[test]
fn real_component_dispatch_permissions_persistence_and_uninstall() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let root = temp.path().join("extensions");
    write_package(
        &source,
        &manifest(ID, COMMAND),
        &component(&plan(), "i32.const 0", false),
    );
    let mut registry = ExtensionRegistry::open(&root).unwrap();
    registry.install_package(&source).unwrap();
    assert!(!registry.commands()[0].enabled);
    assert!(matches!(
        registry.set_enabled(ID, true, None),
        Err(HostError::PermissionDenied(_))
    ));
    let approval = approval(&registry);
    registry.set_enabled(ID, true, Some(approval)).unwrap();
    assert_eq!(
        registry
            .execute(COMMAND, &context())
            .unwrap()
            .requests
            .len(),
        2
    );
    assert!(matches!(
        registry.execute("missing.command", &context()),
        Err(HostError::NotFound(_))
    ));
    assert!(registry
        .execute(COMMAND, &CommandContext::default())
        .is_err());
    assert_eq!(registry.packages()[0].state, PackageState::Enabled);
    let mut registry = ExtensionRegistry::open(&root).unwrap();
    assert!(registry.commands()[0].enabled);
    registry.set_enabled(ID, false, None).unwrap();
    assert!(registry.execute(COMMAND, &context()).is_err());
    registry.set_enabled(ID, true, None).unwrap();
    registry.uninstall_package(ID).unwrap();
    assert!(registry.commands().is_empty());
    assert!(source.exists());
    assert!(!root.join(ID).exists());
}

#[test]
fn undeclared_unapproved_capabilities_and_application_ids_are_denied() {
    for (capabilities, applications) in [
        (vec![], vec![]),
        (
            vec![Capability::Export3mf, Capability::LaunchApplication],
            vec![],
        ),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        write_package(
            &source,
            &manifest(ID, COMMAND),
            &component(&plan(), "i32.const 0", false),
        );
        let mut registry = ExtensionRegistry::open(temp.path().join("extensions")).unwrap();
        registry.install_package(&source).unwrap();
        let mut approval = approval(&registry);
        approval.capabilities = capabilities;
        approval.applications = applications;
        registry.set_enabled(ID, true, Some(approval)).unwrap();
        assert!(matches!(
            registry.execute(COMMAND, &context()),
            Err(HostError::PermissionDenied(_))
        ));
        assert!(matches!(
            registry.packages()[0].state,
            PackageState::Failed(_)
        ));
    }
}

#[test]
fn changed_content_revokes_approval_before_code_runs() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let root = temp.path().join("extensions");
    write_package(
        &source,
        &manifest(ID, COMMAND),
        &component(&plan(), "i32.const 0", false),
    );
    let mut registry = ExtensionRegistry::open(&root).unwrap();
    registry.install_package(&source).unwrap();
    let old_approval = approval(&registry);
    registry
        .set_enabled(ID, true, Some(old_approval.clone()))
        .unwrap();
    fs::write(
        root.join(ID).join("unloaded-resource.txt"),
        "also covered by approval",
    )
    .unwrap();
    assert!(matches!(
        registry.execute(COMMAND, &context()),
        Err(HostError::ApprovalChanged)
    ));
    assert!(matches!(
        registry.set_enabled(ID, true, Some(old_approval)),
        Err(HostError::ApprovalChanged)
    ));
    assert!(!ExtensionRegistry::open(root).unwrap().commands()[0].enabled);
}

#[test]
fn traps_fuel_memory_and_response_budgets_fail_closed() {
    const SPIN: &str = "(loop $spin br $spin) i32.const 0";
    for (body, response, limits) in [
        ("unreachable", "{}".into(), SandboxLimits::default()),
        (
            SPIN,
            "{}".into(),
            SandboxLimits {
                // Wasmtime charges the page-aligned startup data copy before
                // execute; leave headroom so this case reaches the loop.
                fuel: 64 * 1024,
                ..SandboxLimits::default()
            },
        ),
        (
            "i32.const 1000 memory.grow drop i32.const 0",
            "{}".into(),
            SandboxLimits::default(),
        ),
        (
            "i32.const 0",
            "x".repeat(500),
            SandboxLimits {
                max_response_bytes: 100,
                ..SandboxLimits::default()
            },
        ),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        write_package(
            &source,
            &manifest(ID, COMMAND),
            &component(&response, body, false),
        );
        let mut registry =
            ExtensionRegistry::with_limits(temp.path().join("extensions"), limits).unwrap();
        registry.install_package(&source).unwrap();
        registry
            .set_enabled(ID, true, Some(approval(&registry)))
            .unwrap();
        let error = registry.execute(COMMAND, &context()).unwrap_err();
        assert!(matches!(&error, HostError::Sandbox(_)), "{error:?}");
        if body == SPIN {
            assert_execute_exhausts_fuel(&component(&response, body, false), limits.fuel);
        }
        assert!(matches!(
            registry.packages()[0].state,
            PackageState::Failed(_)
        ));
    }
}

fn assert_execute_exhausts_fuel(bytes: &[u8], fuel: u64) {
    use wasmtime::{
        component::{Component, Linker},
        Config, Engine, Store, Trap,
    };

    let mut config = Config::new();
    config
        .wasm_component_model(true)
        .consume_fuel(true)
        .max_wasm_stack(512 * 1024);
    let engine = Engine::new(&config).unwrap();
    let component = Component::from_binary(&engine, bytes).unwrap();
    let mut store = Store::new(&engine, ());
    store.set_fuel(fuel).unwrap();
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &component)
        .unwrap();
    let execute = instance
        .get_typed_func::<(String, String), (Result<String, String>,)>(&mut store, "execute")
        .unwrap();
    let args = (COMMAND.into(), serde_json::to_string(&context()).unwrap());
    // The registry's string error omits Wasmtime's chained trap cause.
    let error = execute.call(&mut store, args).unwrap_err();
    assert_eq!(error.downcast_ref::<Trap>(), Some(&Trap::OutOfFuel));
}

#[test]
fn oversized_guest_error_is_capped() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    write_package(
        &source,
        &manifest(ID, COMMAND),
        &component(&"x".repeat(500), "i32.const 0", true),
    );
    let mut registry = ExtensionRegistry::with_limits(
        temp.path().join("extensions"),
        SandboxLimits {
            max_response_bytes: 100,
            ..SandboxLimits::default()
        },
    )
    .unwrap();
    registry.install_package(&source).unwrap();
    registry
        .set_enabled(ID, true, Some(approval(&registry)))
        .unwrap();
    assert!(registry
        .execute(COMMAND, &context())
        .unwrap_err()
        .to_string()
        .contains("response exceeds limit"));
}

#[test]
fn invalid_packages_imports_paths_and_versions_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let mut registry = ExtensionRegistry::open(temp.path().join("extensions")).unwrap();
    let mut manifest = manifest(ID, COMMAND);
    manifest.entry = "../escape.wasm".into();
    write_package(&source, &manifest, b"\0asm");
    assert!(registry.install_package(&source).is_err());
    manifest.entry = "extension.wasm".into();
    manifest.schema_version = 99;
    write_package(&source, &manifest, b"\0asm");
    assert!(registry.install_package(&source).is_err());
    manifest.schema_version = SCHEMA_VERSION;
    let imported = wat::parse_str("(component (import \"ambient-network\" (func)))").unwrap();
    write_package(&source, &manifest, &imported);
    assert!(matches!(
        registry.install_package(&source),
        Err(HostError::PermissionDenied(_))
    ));
    fs::write(source.join("extension.wasm"), "not wasm").unwrap();
    assert!(registry.install_package(&source).is_err());
}

#[test]
fn command_collisions_are_rejected_even_with_overlapping_package_namespaces() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let root = temp.path().join("extensions");
    let bytes = component("{}", "i32.const 0", false);
    write_package(
        &source,
        &manifest("test.addon", "test.addon.child.send"),
        &bytes,
    );
    let mut registry = ExtensionRegistry::open(&root).unwrap();
    registry.install_package(&source).unwrap();
    write_package(
        &source,
        &manifest("test.addon.child", "test.addon.child.send"),
        &bytes,
    );
    assert!(registry.install_package(&source).is_err());
    write_package(
        &root.join("test.addon.child"),
        &manifest("test.addon.child", "test.addon.child.send"),
        &bytes,
    );
    registry.refresh().unwrap();
    assert!(registry.commands().iter().all(|c| !c.enabled));
    assert!(registry
        .execute("test.addon.child.send", &context())
        .is_err());
    assert!(matches!(
        registry.set_enabled("test.addon", true, Some(approval(&registry))),
        Err(HostError::InvalidPackage(_))
    ));
    let failed_states = registry.packages().into_iter().map(|package| {
        assert!(matches!(&package.state, PackageState::Failed(message) if message.contains("conflicts")));
        (package.manifest.id, package.state)
    }).collect::<Vec<_>>();
    let reopened = ExtensionRegistry::open(&root).unwrap();
    assert_eq!(
        reopened
            .packages()
            .into_iter()
            .map(|package| (package.manifest.id, package.state))
            .collect::<Vec<_>>(),
        failed_states
    );
    assert!(reopened.commands().iter().all(|command| !command.enabled));
}

#[test]
fn native_code_never_loads_even_with_full_trust() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let mut manifest = manifest(ID, COMMAND);
    manifest.runtime = ExtensionRuntime::NativeTrusted;
    write_package(&source, &manifest, b"native code");
    let mut registry = ExtensionRegistry::open(temp.path().join("extensions")).unwrap();
    registry.install_package(&source).unwrap();
    let mut approval = approval(&registry);
    assert!(matches!(
        registry.set_enabled(ID, true, Some(approval.clone())),
        Err(HostError::PermissionDenied(_))
    ));
    approval.trust_native = true;
    assert!(matches!(
        registry.set_enabled(ID, true, Some(approval)),
        Err(HostError::NativeUnsupported)
    ));
}

#[cfg(unix)]
#[test]
fn links_and_large_empty_directory_trees_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    write_package(
        &source,
        &manifest(ID, COMMAND),
        &component("{}", "i32.const 0", false),
    );
    let mut registry = ExtensionRegistry::open(temp.path().join("extensions")).unwrap();
    std::os::unix::fs::symlink("/etc/passwd", source.join("external")).unwrap();
    assert!(registry.install_package(&source).is_err());
    fs::remove_file(source.join("external")).unwrap();
    for n in 0..260 {
        fs::create_dir(source.join(format!("empty-{n}"))).unwrap();
    }
    assert!(registry.install_package(&source).is_err());
}

#[test]
fn malformed_update_unloads_commands_and_requires_fresh_approval() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let root = temp.path().join("extensions");
    write_package(
        &source,
        &manifest(ID, COMMAND),
        &component(&plan(), "i32.const 0", false),
    );
    let mut registry = ExtensionRegistry::open(&root).unwrap();
    registry.install_package(&source).unwrap();
    registry
        .set_enabled(ID, true, Some(approval(&registry)))
        .unwrap();
    let manifest_file = root.join(ID).join("manifest.json");
    let original = fs::read(&manifest_file).unwrap();
    fs::write(&manifest_file, "{malformed").unwrap();
    registry.refresh().unwrap();
    assert!(matches!(
        registry.packages()[0].state,
        PackageState::Failed(_)
    ));
    assert!(!registry.commands()[0].enabled);
    assert_eq!(registry.discovery_errors().len(), 1);
    fs::write(manifest_file, original).unwrap();
    registry.refresh().unwrap();
    assert!(!registry.commands()[0].enabled);
    assert!(matches!(
        registry.set_enabled(ID, true, None),
        Err(HostError::PermissionDenied(_))
    ));
}

#[test]
fn installation_also_bounds_component_start_code() {
    for start in [
        "(func $start (loop $spin br $spin)) (start $start)",
        "(func $start i32.const 1000 memory.grow drop) (start $start)",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        write_package(
            &source,
            &manifest(ID, COMMAND),
            &component_with_start("{}", "i32.const 0", false, start),
        );
        let mut registry = ExtensionRegistry::with_limits(
            temp.path().join("extensions"),
            SandboxLimits {
                fuel: 1000,
                ..SandboxLimits::default()
            },
        )
        .unwrap();
        assert!(matches!(
            registry.install_package(&source),
            Err(HostError::Sandbox(_))
        ));
        assert!(registry.commands().is_empty());
    }
}

#[test]
fn relative_source_directory_is_supported() {
    let source = tempfile::tempdir_in(".").unwrap();
    write_package(
        source.path(),
        &manifest(ID, COMMAND),
        &component("{}", "i32.const 0", false),
    );
    let registry_root = tempfile::tempdir().unwrap();
    let mut registry = ExtensionRegistry::open(registry_root.path()).unwrap();
    let relative = Path::new(source.path().file_name().unwrap());
    assert_eq!(registry.install_package(relative).unwrap(), ID);
}

#[test]
fn guest_failures_cannot_make_the_approval_store_unreadable() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let root = temp.path().join("extensions");
    let bytes = component(&"\u{1}".repeat(50_000), "i32.const 0", true);
    let mut registry = ExtensionRegistry::open(&root).unwrap();
    for number in 0..4 {
        let id = format!("test.addon{number}");
        let command = format!("{id}.send");
        write_package(&source, &manifest(&id, &command), &bytes);
        registry.install_package(&source).unwrap();
        let package = registry
            .packages()
            .into_iter()
            .find(|package| package.manifest.id == id)
            .unwrap();
        registry
            .set_enabled(
                &id,
                true,
                Some(Approval {
                    fingerprint: package.fingerprint,
                    capabilities: package.manifest.capabilities,
                    applications: package
                        .manifest
                        .applications
                        .into_iter()
                        .map(|app| app.id)
                        .collect(),
                    trust_native: false,
                }),
            )
            .unwrap();
        assert!(matches!(
            registry.execute(&command, &context()),
            Err(HostError::Sandbox(_))
        ));
        registry = ExtensionRegistry::open(&root).unwrap();
        let package = registry
            .packages()
            .into_iter()
            .find(|package| package.manifest.id == id)
            .unwrap();
        assert!(matches!(package.state, PackageState::Failed(message) if message.len() <= 4096));
    }
}

#[test]
fn duplicate_grants_are_rejected_before_persistence() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    write_package(
        &source,
        &manifest(ID, COMMAND),
        &component("{}", "i32.const 0", false),
    );
    let mut registry = ExtensionRegistry::open(temp.path().join("extensions")).unwrap();
    registry.install_package(&source).unwrap();
    let mut duplicate = approval(&registry);
    duplicate.capabilities.push(duplicate.capabilities[0]);
    assert!(matches!(
        registry.set_enabled(ID, true, Some(duplicate)),
        Err(HostError::PermissionDenied(_))
    ));
    let mut duplicate = approval(&registry);
    duplicate
        .applications
        .push(duplicate.applications[0].clone());
    assert!(matches!(
        registry.set_enabled(ID, true, Some(duplicate)),
        Err(HostError::PermissionDenied(_))
    ));
    assert_eq!(registry.packages()[0].state, PackageState::Disabled);
}
