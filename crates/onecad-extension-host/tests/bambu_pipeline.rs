//! Real Bambu component → approved registry → real worker export → desktop argv.
//! Set ONECAD_TEST_ADDON_COMPONENT and ONECAD_REQUIRE_WORKER=1 to require this gate.
#![cfg(all(unix, not(target_os = "ios")))]

#[path = "bambu_pipeline/model.rs"]
mod model;

use std::{
    io::Read,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::Duration,
};

use onecad_app::{
    extension_handoff::{
        export_3mf, launch_current_artifact, ExportArtifact, ExportScope, ExportTarget,
        ExternalApplicationDescriptor, ExternalApplicationSettings, LaunchOutcome,
        ProcessApplicationLauncher,
    },
    state::AppState,
};
use onecad_core::ids::BodyId;
use onecad_extension_api::{CommandContext, CommandExecution, HostRequest};
use onecad_extension_host::{Approval, ExtensionRegistry, HostError};
use tempfile::TempDir;

const PACKAGE: &str = "dev.onecad.bambu-studio";
const COMMAND: &str = "dev.onecad.bambu-studio.send";

fn component_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("ONECAD_TEST_ADDON_COMPONENT") {
        let path = PathBuf::from(path);
        assert!(path.is_file(), "ONECAD_TEST_ADDON_COMPONENT must exist");
        return Some(path);
    }
    let bundled = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../packaging/binaries/addons/dev.onecad.bambu-studio/extension.wasm");
    if bundled.is_file() {
        return Some(bundled);
    }
    assert_ne!(
        std::env::var("ONECAD_REQUIRE_WORKER").as_deref(),
        Ok("1"),
        "required Bambu pipeline gate needs ONECAD_TEST_ADDON_COMPONENT (build the addon first)"
    );
    None
}

fn package_source(component: &Path, directory: &Path) -> PathBuf {
    let source = directory.join("source package");
    std::fs::create_dir(&source).unwrap();
    std::fs::copy(component, source.join("extension.wasm")).unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../addons/bambu-studio/manifest.json"),
        source.join("manifest.json"),
    )
    .unwrap();
    source
}

fn fake_desktop(
    directory: &Path,
) -> (
    ExternalApplicationDescriptor,
    ExternalApplicationSettings,
    PathBuf,
) {
    let executable = directory.join("Fake Bambu Studio");
    // The fake is a desktop executable. The production process launcher supplies argv.
    std::fs::write(
        &executable,
        "#!/bin/sh\nprintf '%s\\n' \"$#\" \"$1\" > \"$0.args.tmp\"\nmv \"$0.args.tmp\" \"$0.args\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let descriptor = ExternalApplicationDescriptor {
        id: format!("{PACKAGE}.application"),
        display_name: "Fake Bambu Studio".into(),
        macos_bundle_names: Vec::new(),
        macos_executable_name: None,
    };
    let settings = ExternalApplicationSettings {
        executable: Some(executable.clone()),
    };
    let args = directory.join("Fake Bambu Studio.args");
    (descriptor, settings, args)
}

async fn export_plan(
    plan: &CommandExecution,
    state: &AppState,
    target: &ExportTarget,
    selected: &[BodyId],
    directory: &Path,
) -> Result<ExportArtifact, String> {
    let [HostRequest::Export3mf {
        scope,
        suggested_name,
    }, HostRequest::LaunchApplication { application, .. }] = plan.requests.as_slice()
    else {
        return Err("Bambu command must export before launch".into());
    };
    assert_eq!(application, &format!("{PACKAGE}.application"));
    let scope = match scope {
        onecad_extension_api::ExportScope::Document => ExportScope::Document,
        onecad_extension_api::ExportScope::Selection => ExportScope::Selection(selected.to_vec()),
    };
    export_3mf(state, target, &scope, suggested_name, directory)
        .await
        .map_err(|e| e.to_string())
}

async fn handoff_plan(
    plan: &CommandExecution,
    state: &AppState,
    target: &ExportTarget,
    selected: &[BodyId],
    directory: &Path,
    application: (&ExternalApplicationDescriptor, &ExternalApplicationSettings),
) -> Result<(ExportArtifact, LaunchOutcome), String> {
    let artifact = export_plan(plan, state, target, selected, directory).await?;
    let outcome = launch_current_artifact(
        state,
        application.0,
        application.1,
        &artifact,
        &ProcessApplicationLauncher,
    )
    .await;
    Ok((artifact, outcome))
}

fn model_xml(path: &Path) -> String {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    assert!(archive.by_name("[Content_Types].xml").is_ok());
    assert!(archive.by_name("_rels/.rels").is_ok());
    let mut xml = String::new();
    archive
        .by_name("3D/3dmodel.model")
        .unwrap()
        .read_to_string(&mut xml)
        .unwrap();
    assert!(xml.matches("<vertex ").count() >= 8);
    assert!(xml.matches("<triangle ").count() >= 12);
    xml
}

fn assert_y_bounds(xml: &str, expected: (f64, f64)) {
    let bounds = xml
        .split("<vertex ")
        .skip(1)
        .map(|vertex| vertex.split("y=\"").nth(1).unwrap())
        .map(|vertex| vertex.split('"').next().unwrap().parse::<f64>().unwrap())
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), x| {
            (lo.min(x), hi.max(x))
        });
    assert!(
        (bounds.0 - expected.0).abs() < 0.01 && (bounds.1 - expected.1).abs() < 0.01,
        "expected y bounds {expected:?}, got {bounds:?}"
    );
}

async fn assert_argv(path: &Path, artifact: &Path) {
    for _ in 0..100 {
        if path.is_file() {
            let argv = std::fs::read_to_string(path).unwrap();
            assert_eq!(argv, format!("1\n{}\n", artifact.display()));
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("fake desktop executable did not record its argv");
}

fn approval(registry: &ExtensionRegistry) -> Approval {
    let package = registry
        .packages()
        .into_iter()
        .find(|p| p.manifest.id == PACKAGE)
        .unwrap();
    Approval {
        fingerprint: package.fingerprint,
        capabilities: package.manifest.capabilities,
        applications: package
            .manifest
            .applications
            .into_iter()
            .map(|app| app.id)
            .collect(),
        trust_native: false,
    }
}

fn context(selection: bool) -> CommandContext {
    CommandContext {
        has_document: true,
        has_selection: selection,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_bambu_component_exports_real_model_and_hands_one_path_to_desktop() {
    let (Some(component), Some(worker_path)) = (component_path(), model::worker_path()) else {
        eprintln!("SKIP Bambu integration: build the addon and real worker; require with ONECAD_TEST_ADDON_COMPONENT and ONECAD_REQUIRE_WORKER=1");
        return;
    };
    let root = TempDir::new().unwrap();
    let source = package_source(&component, root.path());
    let mut registry = ExtensionRegistry::open(root.path().join("installed")).unwrap();
    assert_eq!(registry.install_package(&source).unwrap(), PACKAGE);
    registry
        .set_enabled(PACKAGE, true, Some(approval(&registry)))
        .unwrap();
    let (worker, state, target, bodies) = model::two_boxes(worker_path).await;
    let before =
        serde_json::to_value(state.runtime.lock().await.as_ref().unwrap().projection()).unwrap();
    let output = root.path().join("retained exports with spaces");
    let (descriptor, settings, args) = fake_desktop(root.path());

    let selection_plan = registry.execute(COMMAND, &context(true)).unwrap();
    let (selected, launched) = handoff_plan(
        &selection_plan,
        &state,
        &target,
        &bodies[1..],
        &output,
        (&descriptor, &settings),
    )
    .await
    .unwrap();
    assert_eq!(selected.body_count, 1);
    let selected_xml = model_xml(selected.path());
    assert_eq!(selected_xml.matches("<object ").count(), 1);
    // Canonical XY maps sketch X to world Y, so the second box is at Y=40..60.
    assert_y_bounds(&selected_xml, (40.0, 60.0));
    assert_eq!(launched, LaunchOutcome::Launched);
    assert_argv(&args, selected.path()).await;

    let document_plan = registry.execute(COMMAND, &context(false)).unwrap();
    let document = export_plan(&document_plan, &state, &target, &[], &output)
        .await
        .unwrap();
    assert_eq!(document.body_count, 2);
    let document_xml = model_xml(document.path());
    assert_eq!(document_xml.matches("<object ").count(), 2);
    assert_y_bounds(&document_xml, (0.0, 60.0));
    assert_ne!(
        selected.path(),
        document.path(),
        "exports never overwrite each other"
    );
    assert!(selected.path().is_file());

    // A real spawn error leaves the completed, valid file available for manual use.
    let not_executable = root.path().join("not an executable");
    std::fs::write(&not_executable, "not executable").unwrap();
    let failed_settings = ExternalApplicationSettings {
        executable: Some(not_executable),
    };
    assert!(matches!(
        launch_current_artifact(
            &state,
            &descriptor,
            &failed_settings,
            &document,
            &ProcessApplicationLauncher
        )
        .await,
        LaunchOutcome::Failed(_)
    ));
    assert!(document.path().is_file());
    assert_eq!(model_xml(document.path()).matches("<object ").count(), 2);

    // Failed export terminates the handoff before any launcher call can be made.
    std::fs::remove_file(&args).unwrap();
    let mut stale = target.clone();
    stale.revision += 1;
    assert!(handoff_plan(
        &document_plan,
        &state,
        &stale,
        &[],
        &output,
        (&descriptor, &settings)
    )
    .await
    .is_err());
    let unwritable_directory = root.path().join("a file rather than directory");
    std::fs::write(&unwritable_directory, "occupied").unwrap();
    assert!(handoff_plan(
        &document_plan,
        &state,
        &target,
        &[],
        &unwritable_directory,
        (&descriptor, &settings)
    )
    .await
    .is_err());
    assert!(!args.exists());
    assert_eq!(std::fs::read_dir(&output).unwrap().count(), 2);
    let after =
        serde_json::to_value(state.runtime.lock().await.as_ref().unwrap().projection()).unwrap();
    assert_eq!(
        before, after,
        "export and desktop handoff never mutate the document"
    );
    metadata_and_pending_exports(
        &document_plan,
        &state,
        &bodies,
        &output,
        (&descriptor, &settings, &args),
    )
    .await;
    worker.shutdown().await;
}

async fn metadata_and_pending_exports(
    plan: &CommandExecution,
    state: &AppState,
    bodies: &[BodyId],
    output: &Path,
    application: (
        &ExternalApplicationDescriptor,
        &ExternalApplicationSettings,
        &Path,
    ),
) {
    state
        .runtime
        .lock()
        .await
        .as_mut()
        .unwrap()
        .apply(onecad_core::edit::EditCommand::RenameBody {
            body: bodies[0],
            name: "Metadata only handoff".into(),
        })
        .unwrap();
    let target = model::capture_target(state).await;
    let artifact = export_plan(plan, state, &target, &bodies[..1], output)
        .await
        .unwrap();
    assert!(model_xml(artifact.path()).contains("Metadata only handoff"));
    let count = std::fs::read_dir(output).unwrap().count();
    model::add_pending_box(state).await;
    let pending = model::capture_target(state).await;
    let error = export_plan(plan, state, &pending, &bodies[..1], output)
        .await
        .unwrap_err();
    assert!(error.contains("pending regeneration"), "{error}");
    assert_eq!(std::fs::read_dir(output).unwrap().count(), count);
    assert!(artifact.path().is_file());
    assert!(matches!(
        launch_current_artifact(
            state,
            application.0,
            application.1,
            &artifact,
            &ProcessApplicationLauncher
        )
        .await,
        LaunchOutcome::Failed(_)
    ));
    assert!(
        !application.2.exists(),
        "a changed document cannot launch an older artifact"
    );
}

#[test]
fn disabled_denied_and_changed_bambu_packages_produce_no_export_or_launch() {
    let Some(component) = component_path() else {
        eprintln!(
            "SKIP Bambu registry integration: build addon or set ONECAD_TEST_ADDON_COMPONENT"
        );
        return;
    };
    let root = TempDir::new().unwrap();
    let source = package_source(&component, root.path());
    let output = root.path().join("exports");
    std::fs::create_dir(&output).unwrap();
    let mut registry = ExtensionRegistry::open(root.path().join("installed")).unwrap();
    registry.install_package(&source).unwrap();
    assert!(matches!(
        registry.execute(COMMAND, &context(false)),
        Err(HostError::Unavailable(_))
    ));
    let mut denied = approval(&registry);
    denied.applications.clear();
    registry.set_enabled(PACKAGE, true, Some(denied)).unwrap();
    assert!(matches!(
        registry.execute(COMMAND, &context(false)),
        Err(HostError::PermissionDenied(_))
    ));
    registry
        .set_enabled(PACKAGE, true, Some(approval(&registry)))
        .unwrap();
    assert_eq!(
        registry
            .execute(COMMAND, &context(false))
            .unwrap()
            .requests
            .len(),
        2
    );
    registry.set_enabled(PACKAGE, false, None).unwrap();
    assert!(matches!(
        registry.execute(COMMAND, &context(false)),
        Err(HostError::Unavailable(_))
    ));
    registry
        .set_enabled(PACKAGE, true, Some(approval(&registry)))
        .unwrap();
    let package = registry
        .packages()
        .into_iter()
        .find(|p| p.manifest.id == PACKAGE)
        .unwrap();
    // A changed non-code file still expires the content-bound approval.
    std::fs::write(
        package.path.join("changed.txt"),
        "content changed after approval",
    )
    .unwrap();
    assert!(matches!(
        registry.execute(COMMAND, &context(false)),
        Err(HostError::ApprovalChanged)
    ));
    assert_eq!(std::fs::read_dir(output).unwrap().count(), 0);
    assert!(!root.path().join("Fake Bambu Studio.args").exists());
}
