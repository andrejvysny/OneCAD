//! File intents resolve on the bounded service actor; application services own mutations.
use crate::{
    bridge::{Backend, Message},
    document_flow::{ExportFormat, Outcome, Request},
};
use onecad_app::{api, host::FileDialog};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::mpsc,
};
use tokio::sync::OwnedSemaphorePermit;

pub async fn execute(
    backend: &mut Backend,
    request: Request,
    permit: OwnedSemaphorePermit,
    sender: &mpsc::Sender<Message>,
) {
    tracing::info!(?request, "Native document request started");
    if let Request::Extension(request) = request {
        let result = crate::native_extensions::execute(backend, request)
            .await
            .map(Outcome::Extension);
        let _ = sender.send(Message::Document {
            result,
            _permit: permit,
        });
        backend.shared.wake();
        return;
    }
    let mut result = run(backend, request).await;
    backend.events.finish();
    crate::document_state::refresh(backend).await;
    if backend.startup_opened {
        match crate::services::load_ready_display(backend).await {
            Ok(captured) => {
                tracing::info!(document_id = %captured.projection.document_id, runtime_session = %captured.projection.runtime_session,
                    revision = captured.projection.revision, snapshot_id = %captured.context.snapshot_id.0, "Native document captured");
                backend.shared.publish_display(captured);
            }
            Err(error) => {
                tracing::warn!(%error, "Native document capture failed");
                crate::services::invalidate(
                    &backend.shared,
                    format!("Scene refresh failed: {error}"),
                );
                if let Ok(Outcome::Replaced { warning } | Outcome::Inserted { warning }) =
                    &mut result
                {
                    *warning = Some(match warning.take() {
                        Some(previous) => format!("{previous}; {error}"),
                        None => error,
                    });
                }
            }
        }
    }
    tracing::info!(?result, "Native document request finished");
    refresh_recents(backend);
    let _ = sender.send(Message::Document {
        result,
        _permit: permit,
    });
    backend.shared.wake();
}

pub fn refresh_recents(backend: &Backend) {
    let root = backend.paths.root.join("documents");
    let mut entries = onecad_app::recents::list(&backend.host);
    entries.retain(|entry| {
        Path::new(&entry.path)
            .canonicalize()
            .is_ok_and(|path| path.starts_with(&root))
    });
    entries.truncate(10);
    for entry in &mut entries {
        entry.thumbnail = None;
        entry.has_recovery = backend
            .state
            .has_pending_recovery_for_path(Path::new(&entry.path));
    }
    if let Ok(mut recents) = backend.shared.recents.lock() {
        *recents = entries;
    }
}

async fn run(backend: &mut Backend, request: Request) -> Result<Outcome, String> {
    let recovery_ready = backend
        .shared
        .recovery
        .lock()
        .is_ok_and(|recovery| recovery.ready());
    if !recovery_ready {
        return Err("Resolve startup recovery before using file actions".into());
    }
    let (request, discard) = match request {
        Request::DiscardAndReplace(request)
            if matches!(*request, Request::New | Request::Open(_) | Request::Import) =>
        {
            let ticket = api::prepare_replacement_discard(&backend.state)
                .await
                .map_err(|error| error.to_string())?;
            (*request, Some(ticket))
        }
        Request::DiscardAndReplace(_) => {
            return Err("Discard requires a single document replacement intent".into());
        }
        request => (request, None),
    };
    let result = run_intent(backend, request).await;
    finish_discard(&backend.state, backend.host.clone(), discard, result).await
}

async fn finish_discard(
    state: &onecad_app::state::AppState,
    host: onecad_app::host::HostContext,
    discard: Option<api::ReplacementDiscard>,
    result: Result<Outcome, String>,
) -> Result<Outcome, String> {
    let mut outcome = result?;
    if let (Some(discard), Outcome::Replaced { warning }) = (discard, &mut outcome) {
        let projection = api::get_projection(state)
            .await
            .map_err(|error| error.to_string())?;
        if let Err(error) =
            api::discard_replaced_document(state, host, discard, &projection.runtime_session).await
        {
            let message = format!("Previous recovery retained: {error}");
            *warning = Some(warning.take().map_or_else(
                || message.clone(),
                |previous| format!("{previous}; {message}"),
            ));
        }
    }
    Ok(outcome)
}

async fn run_intent(backend: &mut Backend, request: Request) -> Result<Outcome, String> {
    if matches!(request, Request::Save { .. } | Request::Export(_))
        && api::get_document_state(&backend.state)
            .await
            .map_err(|error| error.to_string())?
            .runtime_session
            .is_empty()
    {
        return Err("Create or open a document before saving or exporting".into());
    }
    match request {
        Request::DiscardAndReplace(_) => Err("Nested discard replacement is unsupported".into()),
        Request::Extension(_) => Err("extension request reached file dispatcher".into()),
        Request::Save { save_as } => save(backend, save_as).await,
        Request::Export(format) => export(backend, format).await,
        Request::New => {
            api::new_document(&backend.state, backend.host.clone())
                .await
                .map_err(|e| e.to_string())?;
            backend.startup_opened = true;
            Ok(Outcome::Replaced { warning: None })
        }
        Request::Open(path) => {
            let path = match path {
                Some(path) => Some(path),
                None => api::open_file_dialog(backend.host.clone())
                    .await
                    .map_err(|e| e.to_string())?
                    .map(PathBuf::from),
            };
            let Some(path) = path else {
                return Ok(Outcome::Cancelled);
            };
            open(backend, &path, false).await
        }
        Request::Import => {
            let Some(path) = api::import_file_dialog(backend.host.clone())
                .await
                .map_err(|e| e.to_string())?
            else {
                return Ok(Outcome::Cancelled);
            };
            open(backend, Path::new(&path), true).await
        }
        Request::InsertStep => insert_step(backend).await,
    }
}

/// Mutates the active document through the application lane; its regen is awaited
/// like any edit so the display capture that follows sees the inserted bodies.
async fn insert_step(backend: &mut Backend) -> Result<Outcome, String> {
    let before = api::get_projection(&backend.state)
        .await
        .map_err(|e| e.to_string())?;
    if before.runtime_session.is_empty() {
        return Err("Open a document before inserting a STEP file".into());
    }
    backend.events.begin(crate::event_mailbox::Expected::Known {
        session: before.runtime_session,
        revision: before.revision + 1,
    });
    let Some(projection) = api::insert_step(&backend.state, backend.host.clone())
        .await
        .map_err(|e| e.to_string())?
    else {
        return Ok(Outcome::Cancelled);
    };
    backend
        .events
        .retarget(&projection.runtime_session, projection.revision);
    let warning = match crate::services::wait_regen(
        backend,
        &projection.runtime_session,
        projection.revision,
    )
    .await
    {
        Ok(completion) if matches!(completion.outcome.as_str(), "published" | "noop") => None,
        Ok(completion) => Some(format!(
            "STEP file inserted; regeneration {}",
            completion.outcome
        )),
        Err(error) => Some(format!("STEP file inserted; geometry unavailable: {error}")),
    };
    Ok(Outcome::Inserted { warning })
}

async fn open(backend: &mut Backend, source: &Path, import: bool) -> Result<Outcome, String> {
    let extension = source
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let step = import && matches!(extension.as_str(), "step" | "stp");
    if !step && extension != "onecad" {
        return Err("Open supports .onecad; Import supports .onecad, .step and .stp".into());
    }
    // Check the original as well as its QA copy so copying cannot evade recovery protection.
    if backend.state.has_pending_recovery_for_path(source) {
        return Err("Resolve pending recovery before opening this document".into());
    }
    // The isolated QA copy gets a fresh path, so ownership is decided on the source.
    if let Some(host) = backend
        .state
        .services()
        .reservations()
        .path_owner(source)
        .filter(|host| *host != backend.state.host_id())
    {
        return Ok(Outcome::AlreadyOpen { host });
    }
    let before = api::get_projection(&backend.state)
        .await
        .map_err(|e| e.to_string())?;
    backend
        .events
        .begin(crate::event_mailbox::Expected::Replacement {
            previous: before.runtime_session,
        });
    let path = if step {
        source.to_owned()
    } else {
        copy_for_open(&backend.paths.root.join("documents"), source)?
    };
    match replace_from_path(backend, &path, step).await {
        Err(onecad_app::error::ApiError::AlreadyOpen { host, .. })
        | Err(onecad_app::error::ApiError::DuplicateDocument { host, .. }) => {
            return Ok(Outcome::AlreadyOpen { host });
        }
        Err(error) => return Err(error.to_string()),
        Ok(()) => {}
    }
    if path != source {
        // The source is what the user opened: it stays owned while the copy is open.
        backend.state.add_claim(
            backend
                .state
                .claim_path(source)
                .map_err(|e| e.to_string())?,
        );
    }
    backend.startup_opened = true;
    replacement_outcome(backend).await
}

async fn replace_from_path(
    backend: &Backend,
    path: &Path,
    step: bool,
) -> Result<(), onecad_app::error::ApiError> {
    if step {
        api::import_step(
            &backend.state,
            backend.host.clone(),
            path.to_string_lossy().into_owned(),
        )
        .await
    } else {
        api::open_document(
            &backend.state,
            backend.host.clone(),
            path.to_string_lossy().into_owned(),
            None,
        )
        .await
    }?;
    Ok(())
}

async fn replacement_outcome(backend: &mut Backend) -> Result<Outcome, String> {
    let projection = api::get_projection(&backend.state)
        .await
        .map_err(|e| e.to_string())?;
    backend
        .events
        .retarget(&projection.runtime_session, projection.revision);
    let warning = match crate::services::wait_regen(
        backend,
        &projection.runtime_session,
        projection.revision,
    )
    .await
    {
        Ok(completion) if matches!(completion.outcome.as_str(), "published" | "noop") => None,
        Ok(completion) => Some(format!(
            "Document opened; regeneration {}",
            completion.outcome
        )),
        Err(error) => Some(format!("Document opened; geometry unavailable: {error}")),
    };
    Ok(Outcome::Replaced { warning })
}

async fn save(backend: &Backend, save_as: bool) -> Result<Outcome, String> {
    save_to_qa(
        &backend.state,
        backend.host.clone(),
        &backend.paths.root.join("documents"),
        save_as,
    )
    .await
}

async fn save_to_qa(
    state: &onecad_app::state::AppState,
    host: onecad_app::host::HostContext,
    documents: &Path,
    save_as: bool,
) -> Result<Outcome, String> {
    let current = state
        .runtime
        .lock()
        .await
        .as_ref()
        .and_then(|runtime| runtime.path().map(Path::to_owned));
    let needs_path = current
        .as_ref()
        .is_none_or(|path| confined_target(documents, path).is_err());
    let target = if save_as || needs_path {
        let Some(path) = api::save_file_dialog(host.clone())
            .await
            .map_err(|e| e.to_string())?
        else {
            return Ok(Outcome::Cancelled);
        };
        PathBuf::from(path)
    } else {
        current.ok_or("No save path")?
    };
    let target = confined_target(documents, &target)?;
    if target.extension().and_then(|s| s.to_str()) != Some("onecad") {
        return Err("Save requires a .onecad filename".into());
    }
    api::save_document(
        state,
        host.clone(),
        Some(target.to_string_lossy().into_owned()),
        None,
    )
    .await
    .map(Outcome::Saved)
    .map_err(|e| e.to_string())
}

async fn export(backend: &Backend, format: ExportFormat) -> Result<Outcome, String> {
    let (label, extension) = match format {
        ExportFormat::Step => ("STEP", "step"),
        ExportFormat::Stl => ("STL", "stl"),
        ExportFormat::Obj => ("OBJ", "obj"),
        ExportFormat::ThreeMf => ("3MF", "3mf"),
    };
    let paths = backend
        .host
        .dialogs
        .pick(FileDialog::new(label, &[extension], true))
        .await;
    let Some(path) = paths.first() else {
        return Ok(Outcome::Cancelled);
    };
    let path = confined_target(&backend.paths.root.join("documents"), Path::new(path))?;
    validate_export_extension(format, &path)?;
    let target = Some(path.to_string_lossy().into_owned());
    let result = match format {
        ExportFormat::Step => {
            api::export_step_file(&backend.state, backend.host.clone(), target).await
        }
        ExportFormat::Stl => {
            api::export_stl_file(&backend.state, backend.host.clone(), target).await
        }
        ExportFormat::Obj => {
            api::export_obj_file(&backend.state, backend.host.clone(), target).await
        }
        ExportFormat::ThreeMf => {
            api::export_3mf_file(&backend.state, backend.host.clone(), target).await
        }
    }
    .map_err(|e| e.to_string())?;
    Ok(result.map_or(Outcome::Cancelled, |path| {
        Outcome::Exported(PathBuf::from(path))
    }))
}

fn validate_export_extension(format: ExportFormat, path: &Path) -> Result<(), String> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let valid = match format {
        ExportFormat::Step => matches!(extension.as_str(), "step" | "stp"),
        ExportFormat::Stl => extension == "stl",
        ExportFormat::Obj => extension == "obj",
        ExportFormat::ThreeMf => extension == "3mf",
    };
    if valid {
        Ok(())
    } else {
        Err("Export filename extension must match the selected format".into())
    }
}

fn confined_target(documents: &Path, target: &Path) -> Result<PathBuf, String> {
    let documents = documents.canonicalize().map_err(|e| e.to_string())?;
    if !target.is_absolute() {
        return Err("Save and export require absolute QA paths".into());
    }
    if target
        .symlink_metadata()
        .is_ok_and(|m| m.file_type().is_symlink())
    {
        return Err("Save and export destinations must not be symlinks".into());
    }
    let parent = target
        .parent()
        .ok_or("Missing destination directory")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if !parent.starts_with(&documents) {
        return Err(
            "Save and export must remain inside the isolated QA documents directory".into(),
        );
    }
    Ok(parent.join(target.file_name().ok_or("Missing destination filename")?))
}

fn copy_for_open(documents: &Path, source: &Path) -> Result<PathBuf, String> {
    let source = source.canonicalize().map_err(|e| e.to_string())?;
    let documents = documents.canonicalize().map_err(|e| e.to_string())?;
    if source.starts_with(&documents) {
        return Ok(source);
    }
    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("document");
    let target = documents.join(format!("{stem}-{}.onecad", uuid::Uuid::new_v4()));
    let limit = onecad_core::io::container::MAX_TOTAL_BYTES;
    let input = fs::File::open(&source).map_err(|e| e.to_string())?;
    let metadata = input.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err("QA project copy exceeds container admission limits".into());
    }
    let mut output = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&target)
        .map_err(|e| e.to_string())?;
    let result =
        std::io::copy(&mut std::io::Read::take(input, limit + 1), &mut output).and_then(|bytes| {
            if bytes > limit {
                Err(std::io::Error::other(
                    "QA project copy exceeds container admission limits",
                ))
            } else {
                Ok(bytes)
            }
        });
    if let Err(error) = result {
        let _ = fs::remove_file(&target);
        return Err(error.to_string());
    }
    Ok(target)
}

#[cfg(test)]
#[path = "document_services_tests.rs"]
mod tests;
