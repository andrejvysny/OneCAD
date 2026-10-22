//! Exercises the native adapter with its real guest, worker export, and process launcher.
#[path = "../../../../onecad-extension-host/tests/bambu_pipeline/model.rs"]
mod model;

use super::*;
use onecad_app::state::AppState;
use onecad_extension_host::{Approval, PackageState};
use std::{io::Read, os::unix::fs::PermissionsExt, sync::Arc, time::Duration};

const PACKAGE: &str = "dev.onecad.bambu-studio";
const COMMAND: &str = "dev.onecad.bambu-studio.send";

fn component_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("ONECAD_TEST_ADDON_COMPONENT") {
        let path = PathBuf::from(path);
        assert!(path.is_file(), "ONECAD_TEST_ADDON_COMPONENT must exist");
        return Some(path);
    }
    let staged = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../packaging/binaries/addons/dev.onecad.bambu-studio/extension.wasm");
    if staged.is_file() {
        return Some(staged);
    }
    assert_ne!(
        std::env::var("ONECAD_REQUIRE_WORKER").as_deref(),
        Ok("1"),
        "required native Bambu adapter gate needs the compiled addon"
    );
    None
}

fn source_package(component: &Path, directory: &Path) -> PathBuf {
    let source = directory.join("source");
    fs::create_dir(&source).unwrap();
    fs::copy(component, source.join("extension.wasm")).unwrap();
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../addons/bambu-studio/manifest.json"),
        source.join("manifest.json"),
    )
    .unwrap();
    source
}

async fn dispatch(
    root: &Path,
    settings: &Path,
    state: &Arc<AppState>,
    output: &Path,
    request: Request,
) -> Result<Reply, String> {
    let prepared = prepare(root, settings, request.clone(), None)?;
    finish_execution(state, output, request, prepared).await
}

async fn recorded_artifact(arguments: &Path) -> PathBuf {
    for _ in 0..100 {
        if arguments.is_file() {
            let text = fs::read_to_string(arguments).unwrap();
            let lines = text.lines().collect::<Vec<_>>();
            assert_eq!(lines.len(), 2);
            assert_eq!(
                lines[0], "1",
                "native launch passes exactly one literal path"
            );
            return PathBuf::from(lines[1]);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the controlled executable did not record its arguments");
}

fn exported_model(path: &Path, objects: usize) -> String {
    assert_eq!(path.extension().unwrap(), "3mf");
    let mut archive = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    let mut model = String::new();
    archive
        .by_name("3D/3dmodel.model")
        .unwrap()
        .read_to_string(&mut model)
        .unwrap();
    assert_eq!(model.matches("<object ").count(), objects);
    assert!(model.matches("<triangle ").count() >= 12);
    model
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_adapter_dispatches_real_bambu_selection_and_document_and_retires_registration() {
    let (Some(component), Some(worker_path)) = (component_path(), model::worker_path()) else {
        eprintln!(
            "SKIP native Bambu adapter: build addon/worker; require with ONECAD_TEST_ADDON_COMPONENT and ONECAD_REQUIRE_WORKER=1"
        );
        return;
    };
    let _guard = MANAGEMENT.lock().await;
    let area = tempfile::tempdir().unwrap();
    let root = area.path().join("installed");
    let settings = area.path().join("applications.json");
    let output = area.path().join("exports with spaces");
    let source = source_package(&component, area.path());
    let (worker, state, _, bodies) = model::two_boxes(worker_path).await;
    let state = Arc::new(state);
    let (installed, plan) =
        prepare(&root, &settings, Request::InstallFolder, Some(&source)).unwrap();
    assert!(!installed.warning && plan.is_none());
    let package = &installed.packages[0];
    assert_eq!(package.state, PackageState::Disabled);
    assert!(installed.commands.iter().all(|command| !command.enabled));
    let mut approval = Approval {
        fingerprint: package.fingerprint.clone(),
        capabilities: package.manifest.capabilities.clone(),
        applications: package
            .manifest
            .applications
            .iter()
            .map(|application| application.id.clone())
            .collect(),
        trust_native: false,
    };
    let enabled = dispatch(
        &root,
        &settings,
        &state,
        &output,
        Request::SetEnabled {
            id: PACKAGE.into(),
            enabled: true,
            approval: Some(approval.clone()),
        },
    )
    .await
    .unwrap();
    assert!(!enabled.warning && enabled.commands[0].enabled);
    let executable = area.path().join("Controlled Bambu executable");
    fs::write(&executable, "#!/bin/sh\nprintf '%s\\n' \"$#\" \"$1\" > \"$0.args.tmp\"\nmv \"$0.args.tmp\" \"$0.args\"\n").unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    let arguments = area.path().join("Controlled Bambu executable.args");
    let configured = dispatch(
        &root,
        &settings,
        &state,
        &output,
        Request::SetApplication {
            extension: PACKAGE.into(),
            application: format!("{PACKAGE}.application"),
            executable: Some(executable),
        },
    )
    .await
    .unwrap();
    assert!(!configured.warning);
    let before =
        serde_json::to_value(state.runtime.lock().await.as_ref().unwrap().projection()).unwrap();
    let target = onecad_app::read::preview_context(&state).await.unwrap();
    let selected_request = Request::Execute {
        command: COMMAND.into(),
        fingerprint: approval.fingerprint.clone(),
        target: Some(target.clone()),
        selection: bodies[1..].to_vec(),
    };
    let selected = dispatch(&root, &settings, &state, &output, selected_request.clone())
        .await
        .unwrap();
    assert!(!selected.warning && selected.status.contains("launched"));
    let selected_path = recorded_artifact(&arguments).await;
    assert_eq!(selected_path.parent().unwrap(), output);
    let model = exported_model(&selected_path, 1);
    let ys = model
        .split("<vertex ")
        .skip(1)
        .map(|vertex| {
            vertex
                .split("y=\"")
                .nth(1)
                .unwrap()
                .split('"')
                .next()
                .unwrap()
                .parse::<f64>()
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert!(
        ys.iter().all(|y| (39.99..=60.01).contains(y)),
        "the second selected body must be exported"
    );
    fs::remove_file(&arguments).unwrap();
    let document = dispatch(
        &root,
        &settings,
        &state,
        &output,
        Request::Execute {
            command: COMMAND.into(),
            fingerprint: approval.fingerprint.clone(),
            target: Some(target.clone()),
            selection: vec![],
        },
    )
    .await
    .unwrap();
    assert!(!document.warning);
    let document_path = recorded_artifact(&arguments).await;
    exported_model(&document_path, 2);
    assert_ne!(document_path, selected_path);
    assert_eq!(
        before,
        serde_json::to_value(state.runtime.lock().await.as_ref().unwrap().projection()).unwrap()
    );
    fs::remove_file(&arguments).unwrap();
    let mut manifest: onecad_extension_api::Manifest =
        serde_json::from_slice(&fs::read(source.join("manifest.json")).unwrap()).unwrap();
    manifest.version = "0.2.0".into();
    fs::write(
        source.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let update = Request::UpdateFolder {
        id: PACKAGE.into(),
        fingerprint: approval.fingerprint.clone(),
    };
    let updated = finish_execution(
        &state,
        &output,
        update.clone(),
        prepare(&root, &settings, update, Some(&source)).unwrap(),
    )
    .await
    .unwrap();
    assert!(!updated.warning && updated.review_package.as_deref() == Some(PACKAGE));
    assert_eq!(updated.packages[0].state, PackageState::Disabled);
    approval.fingerprint = updated.packages[0].fingerprint.clone();
    dispatch(
        &root,
        &settings,
        &state,
        &output,
        Request::SetEnabled {
            id: PACKAGE.into(),
            enabled: true,
            approval: Some(approval.clone()),
        },
    )
    .await
    .unwrap();
    let stale = dispatch(&root, &settings, &state, &output, selected_request)
        .await
        .unwrap();
    assert!(
        stale.warning
            && stale
                .status
                .contains("changed since this command was selected")
    );
    assert!(
        stale.commands[0].enabled,
        "a stale queued request must not disable the new approved package"
    );
    assert!(!arguments.exists());
    let selected_request = Request::Execute {
        command: COMMAND.into(),
        fingerprint: approval.fingerprint.clone(),
        target: Some(target),
        selection: bodies[1..].to_vec(),
    };
    assert!(
        prepare(&root, &settings, selected_request.clone(), None)
            .unwrap()
            .1
            .is_some()
    );
    model::add_pending_box(&state).await;
    let pending_target = onecad_app::read::preview_context(&state).await.unwrap();
    let pending = dispatch(
        &root,
        &settings,
        &state,
        &output,
        Request::Execute {
            command: COMMAND.into(),
            fingerprint: approval.fingerprint.clone(),
            target: Some(pending_target),
            selection: bodies[1..].to_vec(),
        },
    )
    .await
    .unwrap();
    assert!(
        pending.warning && pending.status.contains("pending regeneration"),
        "{}",
        pending.status
    );
    assert!(
        pending.commands[0].enabled,
        "pending document geometry does not revoke package approval"
    );
    assert!(!arguments.exists());
    assert_eq!(fs::read_dir(&output).unwrap().count(), 2);
    dispatch(
        &root,
        &settings,
        &state,
        &output,
        Request::SetEnabled {
            id: PACKAGE.into(),
            enabled: false,
            approval: None,
        },
    )
    .await
    .unwrap();
    let disabled = dispatch(&root, &settings, &state, &output, selected_request.clone())
        .await
        .unwrap();
    assert!(disabled.warning && disabled.commands.iter().all(|command| !command.enabled));
    assert!(!arguments.exists());
    dispatch(
        &root,
        &settings,
        &state,
        &output,
        Request::SetEnabled {
            id: PACKAGE.into(),
            enabled: true,
            approval: Some(approval),
        },
    )
    .await
    .unwrap();
    fs::write(
        root.join(PACKAGE).join("changed-resource.txt"),
        "approval expired",
    )
    .unwrap();
    let changed = dispatch(&root, &settings, &state, &output, selected_request)
        .await
        .unwrap();
    assert!(changed.warning && changed.commands.iter().all(|command| !command.enabled));
    assert!(!arguments.exists());
    assert_eq!(fs::read_dir(&output).unwrap().count(), 2);
    worker.shutdown().await;
}
