mod settings;
use self::settings::*;
use super::{Reply, Request, application_key};
use crate::bridge::Backend;
use onecad_app::extension_handoff::{
    self as handoff, ExternalApplicationDescriptor, ExternalApplicationSettings, LaunchOutcome,
    ProcessApplicationLauncher,
};
use onecad_extension_api::{CommandContext, CommandExecution, ExportScope, HostRequest};
use onecad_extension_host::{ExtensionRegistry, PackageSummary};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

// A profile may have several document actors. Approval changes cannot interleave
// with an already-authorized plan's side effects across those actors.
static MANAGEMENT: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static NEXT_SNAPSHOT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
type Applications = BTreeMap<String, PathBuf>;
type Prepared = (Reply, Option<(PackageSummary, CommandExecution)>);

pub(crate) async fn execute(backend: &Backend, request: Request) -> Result<Reply, String> {
    let update = matches!(&request, Request::UpdateFolder { .. });
    let source = if matches!(
        &request,
        Request::InstallFolder | Request::UpdateFolder { .. }
    ) {
        match rfd::AsyncFileDialog::new()
            .set_title(if update {
                "Choose updated extension package folder"
            } else {
                "Choose extension package folder"
            })
            .pick_folder()
            .await
        {
            Some(folder) => Some(folder.path().to_path_buf()),
            None => {
                return snapshot(
                    backend,
                    if update {
                        "Package update cancelled"
                    } else {
                        "Package installation cancelled"
                    }
                    .into(),
                )
                .await;
            }
        }
    } else {
        None
    };
    let _guard = MANAGEMENT.lock().await;
    let root = backend.paths.data.join("extensions");
    let settings = backend.paths.config.join("extension-applications.json");
    let action = request.clone();
    let prepared =
        tokio::task::spawn_blocking(move || prepare(&root, &settings, action, source.as_deref()))
            .await
            .map_err(|error| format!("Extension service failed: {error}"))??;
    finish_execution(
        &backend.state,
        &backend.paths.data.join("extension-exports"),
        request,
        prepared,
    )
    .await
}

async fn finish_execution(
    state: &std::sync::Arc<onecad_app::state::AppState>,
    export_directory: &Path,
    request: Request,
    (mut reply, plan): Prepared,
) -> Result<Reply, String> {
    if let (
        Request::Execute {
            target, selection, ..
        },
        Some((package, plan)),
    ) = (request, plan)
    {
        match run_plan(
            state,
            export_directory,
            &package,
            plan,
            target,
            selection,
            &reply.applications,
        )
        .await
        {
            Ok((status, warning)) => {
                reply.status = status;
                reply.warning |= warning;
            }
            Err(error) => {
                reply.status = error;
                reply.warning = true;
            }
        }
    }
    Ok(reply)
}

async fn snapshot(backend: &Backend, status: String) -> Result<Reply, String> {
    let _guard = MANAGEMENT.lock().await;
    let root = backend.paths.data.join("extensions");
    let settings = backend.paths.config.join("extension-applications.json");
    tokio::task::spawn_blocking(move || {
        let registry = ExtensionRegistry::open(root).map_err(|e| e.to_string())?;
        let applications = read_settings(&settings);
        let issue = applications.as_ref().err().cloned();
        Ok(settings_warning(
            reply(&registry, applications.unwrap_or_default(), status),
            issue.as_deref(),
        ))
    })
    .await
    .map_err(|e| e.to_string())?
}

fn prepare(
    root: &Path,
    settings: &Path,
    request: Request,
    source: Option<&Path>,
) -> Result<Prepared, String> {
    let mut registry = ExtensionRegistry::open(root).map_err(|e| e.to_string())?;
    let applications = read_settings(settings);
    let mut settings_issue = applications.as_ref().err().cloned();
    let mut applications = applications.unwrap_or_default();
    if settings_issue.is_some() && !settings_independent(&request) {
        return Ok((
            settings_warning(
                reply(
                    &registry,
                    applications,
                    "Extension action was not performed".into(),
                ),
                settings_issue.as_deref(),
            ),
            None,
        ));
    }
    let mut plan = None;
    let mut review_package = None;
    let mut warning = false;
    let status = (|| -> Result<String, String> {
        Ok(match request {
            Request::Refresh => {
                install_bundled(
                    &mut registry,
                    &settings.with_file_name("extension-bundles.json"),
                )?;
                "Extensions refreshed".into()
            }
            Request::InstallFolder => {
                let id = registry
                    .install_package(source.ok_or("No package folder selected")?)
                    .map_err(|e| e.to_string())?;
                review_package = Some(id.clone());
                format!("Installed {id}. Review permissions before enabling it.")
            }
            Request::UpdateFolder { id, fingerprint } => {
                registry
                    .update_package(
                        &id,
                        &fingerprint,
                        source.ok_or("No update package folder selected")?,
                    )
                    .map_err(|e| e.to_string())?;
                review_package = Some(id.clone());
                format!("Updated {id}. Review permissions before enabling the new package.")
            }
            Request::Uninstall { id } => {
                registry.uninstall_package(&id).map_err(|e| e.to_string())?;
                applications.retain(|key, _| !key.starts_with(&format!("{id}/")));
                match settings_issue
                    .as_ref()
                    .map_or_else(|| write_settings(settings, &applications), |_| Ok(()))
                {
                    Ok(()) => format!("Uninstalled {id}"),
                    Err(error) => {
                        warning = true;
                        format!("Uninstalled {id}; application settings cleanup failed: {error}")
                    }
                }
            }
            Request::SetEnabled {
                id,
                enabled,
                approval,
            } => {
                registry
                    .set_enabled(&id, enabled, approval)
                    .map_err(|e| e.to_string())?;
                format!("{id} {}", if enabled { "enabled" } else { "disabled" })
            }
            Request::SetApplication {
                extension,
                application,
                executable,
            } => {
                let mut next = applications.clone();
                save_application(&registry, &mut next, &extension, &application, executable)?;
                write_settings(settings, &next)?;
                applications = next;
                "Application setting saved".into()
            }
            Request::ResetApplications => {
                let backup = settings.with_file_name(format!(
                    "extension-applications-{}.backup.json",
                    uuid::Uuid::new_v4()
                ));
                let preserved = reset_application_settings(settings, &backup)?;
                applications.clear();
                settings_issue = None;
                if preserved {
                    format!(
                        "Application paths reset. Previous settings retained at {}",
                        backup.display()
                    )
                } else {
                    "Application paths reset".into()
                }
            }
            Request::Execute {
                command,
                fingerprint,
                target,
                selection,
            } => {
                let entry = registry
                    .commands()
                    .into_iter()
                    .find(|entry| entry.command.id == command)
                    .ok_or("Extension command no longer exists")?;
                let package = registry
                    .packages()
                    .into_iter()
                    .find(|package| package.manifest.id == entry.extension_id)
                    .ok_or("Extension package no longer exists")?;
                if package.fingerprint != fingerprint {
                    return Err("Extension package changed since this command was selected; refresh and retry".into());
                }
                match registry.execute(
                    &command,
                    &CommandContext {
                        has_document: target.is_some(),
                        has_selection: target.is_some() && !selection.is_empty(),
                    },
                ) {
                    Ok(result) => {
                        plan = Some((package, result));
                        "Extension command completed".into()
                    }
                    Err(error) => {
                        warning = true;
                        error.to_string()
                    }
                }
            }
        })
    })();
    let status = match status {
        Ok(status) => status,
        Err(error) => {
            warning = true;
            // Failed persistence may have changed this registry in memory. Only
            // publish the state a subsequent execution would actually reload.
            registry = ExtensionRegistry::open(root).map_err(|e| e.to_string())?;
            error
        }
    };
    let mut reply = reply(&registry, applications, status);
    reply.review_package = review_package;
    reply.warning |= warning;
    Ok((settings_warning(reply, settings_issue.as_deref()), plan))
}

fn settings_independent(request: &Request) -> bool {
    matches!(
        request,
        Request::Refresh
            | Request::SetEnabled { enabled: false, .. }
            | Request::Uninstall { .. }
            | Request::ResetApplications
    )
}

fn settings_warning(mut reply: Reply, issue: Option<&str>) -> Reply {
    if let Some(issue) = issue {
        reply.warning = true;
        for command in &mut reply.commands {
            command.enabled = false;
        }
        reply.status = format!(
            "{}; application settings unavailable: {issue}. Existing settings were preserved.",
            reply.status
        );
    }
    reply
}

fn reply(registry: &ExtensionRegistry, applications: Applications, status: String) -> Reply {
    Reply {
        generation: NEXT_SNAPSHOT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        packages: registry.packages(),
        commands: registry.commands(),
        applications,
        status,
        warning: !registry.discovery_errors().is_empty(),
        discovery_errors: registry
            .discovery_errors()
            .iter()
            .map(|(path, error)| format!("{}: {error}", path.display()))
            .collect(),
        review_package: None,
    }
}

fn install_bundled(registry: &mut ExtensionRegistry, record: &Path) -> Result<(), String> {
    let bundled = std::env::current_exe().ok().and_then(|exe| {
        exe.parent()?
            .parent()
            .map(|contents| contents.join("Resources/addons"))
    });
    let root = bundled.filter(|path| path.is_dir()).or_else(|| {
        cfg!(debug_assertions)
            .then(|| {
                std::env::current_dir()
                    .ok()
                    .map(|root| root.join("packaging/binaries/addons"))
            })
            .flatten()
            .filter(|path| path.is_dir())
    });
    let Some(root) = root else {
        return Ok(());
    };
    seed_bundled(registry, &root, record)
}

fn seed_bundled(
    registry: &mut ExtensionRegistry,
    root: &Path,
    record: &Path,
) -> Result<(), String> {
    let mut seen: std::collections::BTreeSet<String> = match read_settings_bytes(record)? {
        Some(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| format!("Invalid bundled package record: {e}"))?,
        None => Default::default(),
    };
    for entry in fs::read_dir(root).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if !path.is_dir() {
            continue;
        }
        let manifest: onecad_extension_api::Manifest = serde_json::from_slice(
            &fs::read(path.join(onecad_extension_api::MANIFEST_FILE)).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        if !seen.contains(&manifest.id)
            && !registry
                .packages()
                .iter()
                .any(|package| package.manifest.id == manifest.id)
        {
            registry.install_package(&path).map_err(|e| e.to_string())?;
        }
        seen.insert(manifest.id);
    }
    write_settings(record, &seen)?;
    Ok(())
}

async fn run_plan(
    state: &std::sync::Arc<onecad_app::state::AppState>,
    export_directory: &Path,
    package: &PackageSummary,
    plan: CommandExecution,
    target: Option<onecad_app::api::PreviewContext>,
    selection: Vec<onecad_core::ids::BodyId>,
    settings: &Applications,
) -> Result<(String, bool), String> {
    let mut artifact = None;
    let mut status = plan
        .message
        .unwrap_or_else(|| "Extension command completed".into());
    for request in plan.requests {
        match request {
            HostRequest::Export3mf {
                scope,
                suggested_name,
            } => {
                let context = target
                    .as_ref()
                    .ok_or("Open a ready document before exporting")?;
                let target = handoff::ExportTarget::from(context);
                let scope = match scope {
                    ExportScope::Document => handoff::ExportScope::Document,
                    ExportScope::Selection => handoff::ExportScope::Selection(selection.clone()),
                };
                let result =
                    handoff::export_3mf(state, &target, &scope, &suggested_name, export_directory)
                        .await
                        .map_err(|e| e.to_string())?;
                status = format!("Exported {}", result.path().display());
                artifact = Some(result);
            }
            HostRequest::LaunchApplication { application, .. } => {
                let artifact = artifact
                    .as_ref()
                    .ok_or("Extension did not export a file before launching an application")?;
                if let Err(error) = handoff::validate_artifact_current(state, artifact).await {
                    return Ok((
                        format!(
                            "Exported {}; launch refused: {error}",
                            artifact.path().display()
                        ),
                        true,
                    ));
                }
                let descriptor = package
                    .manifest
                    .applications
                    .iter()
                    .find(|app| app.id == application)
                    .ok_or("Application is not declared by this package")?;
                let descriptor = ExternalApplicationDescriptor {
                    id: descriptor.id.clone(),
                    display_name: descriptor.name.clone(),
                    macos_bundle_names: descriptor.macos_bundle_name.iter().cloned().collect(),
                    macos_executable_name: None,
                };
                let settings = ExternalApplicationSettings {
                    executable: settings
                        .get(&application_key(&package.manifest.id, &application))
                        .cloned(),
                };
                let exported = artifact.clone();
                let state = state.clone();
                let runtime = tokio::runtime::Handle::current();
                let outcome = tokio::task::spawn_blocking(move || {
                    runtime.block_on(handoff::launch_current_artifact(
                        &state,
                        &descriptor,
                        &settings,
                        &exported,
                        &ProcessApplicationLauncher,
                    ))
                })
                .await
                .map_err(|e| e.to_string())?;
                status = match outcome {
                    LaunchOutcome::Launched => format!(
                        "Exported {} and launched application. Imported units and appearance require verification in the receiving application.",
                        artifact.path().display()
                    ),
                    LaunchOutcome::Failed(error) => {
                        return Ok((
                            format!(
                                "Exported {}; application launch failed: {error}",
                                artifact.path().display()
                            ),
                            true,
                        ));
                    }
                };
            }
        }
    }
    Ok((status, false))
}

#[cfg(all(test, unix))]
mod pipeline;
#[cfg(test)]
mod tests;
