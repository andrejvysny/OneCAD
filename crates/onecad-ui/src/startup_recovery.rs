//! Startup decisions precede opening any saved UUID and remain explicit.
use crate::{
    bridge::{Backend, Command},
    path_policy::PathPolicy,
    qa::QaPaths,
};
use onecad_app::{api, dto::RecoveryInfoDto};
use onecad_widgets::components::overlays::{Dialog, DialogIntent};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug)]
pub enum Choice {
    Restore,
    OpenSaved,
    Discard,
}

#[derive(Default)]
pub struct State {
    pub checked: bool,
    pub offers: Vec<RecoveryInfoDto>,
    pub busy: bool,
    pub opened: bool,
    /// The user set the remaining offers aside; they stay pending on disk and in
    /// the process offer set, and are offered again at the next launch.
    pub deferred: bool,
}

impl State {
    fn offer(&self, id: &str) -> Result<&RecoveryInfoDto, String> {
        if self.opened {
            return Err(
                "A startup document is already open; other recovery offers are deferred".into(),
            );
        }
        self.offers
            .iter()
            .find(|offer| offer.document_id == id)
            .ok_or_else(|| "Recovery offer is no longer pending".into())
    }

    /// An offer to discard: allowed even when this host already has a document,
    /// since discarding replaces nothing.
    fn discardable(&self, id: &str) -> Result<&RecoveryInfoDto, String> {
        self.offers
            .iter()
            .find(|offer| offer.document_id == id)
            .ok_or_else(|| "Recovery offer is no longer pending".into())
    }

    /// Whether file actions may proceed (recovery resolved, opened or set aside).
    pub fn ready(&self) -> bool {
        self.opened || self.deferred || (self.checked && self.offers.is_empty())
    }
}

pub async fn discover(backend: &Backend) -> Result<bool, String> {
    let offers = api::check_recovery(&backend.state, backend.host.clone())
        .await
        .map_err(|error| error.to_string())?;
    let pending = !offers.is_empty();
    let mut recovery = backend.shared.recovery.lock().map_err(|e| e.to_string())?;
    recovery.checked = true;
    recovery.offers = offers;
    backend.shared.wake();
    Ok(pending)
}

pub async fn resolve(backend: &mut Backend, id: &str, choice: Choice) -> Result<bool, String> {
    let offer = {
        let recovery = backend.shared.recovery.lock().map_err(|e| e.to_string())?;
        match choice {
            Choice::Discard => recovery.discardable(id)?.clone(),
            Choice::Restore | Choice::OpenSaved => recovery.offer(id)?.clone(),
        }
    };
    apply_choice(backend, &offer, choice).await?;
    remove_offer(backend, id)?;
    if !backend.startup_opened && !has_offers(backend)? {
        backend.startup_opened = crate::services::open_requested_startup(
            &backend.state,
            backend.host.clone(),
            &backend.paths,
        )
        .await?;
    }
    if backend.startup_opened {
        backend
            .shared
            .recovery
            .lock()
            .map_err(|e| e.to_string())?
            .opened = true;
    }
    Ok(!matches!(choice, Choice::Discard)
        || (backend.startup_opened && backend.paths.open_on_start))
}

async fn apply_choice(
    backend: &mut Backend,
    offer: &RecoveryInfoDto,
    choice: Choice,
) -> Result<(), String> {
    let id = &offer.document_id;
    if matches!(choice, Choice::Discard) {
        return api::recover_document(&backend.state, backend.host.clone(), id.clone(), false)
            .await
            .map(|_| ())
            .map_err(|error| error.to_string());
    }
    let destination = recovery_destination(&backend.paths, offer)?;
    match choice {
        Choice::Restore => {
            api::recover_document_at_path(
                &backend.state,
                backend.host.clone(),
                id.clone(),
                true,
                Some(destination.clone()),
            )
            .await
            .map_err(|e| e.to_string())?
            .ok_or("Recovery offer is no longer pending")?;
            if let Some(original) = offer.original_path.as_deref() {
                onecad_app::recents::relocate(&backend.host, Path::new(original), &destination);
            }
            backend.paths.document = destination;
            backend.startup_opened = true;
        }
        Choice::OpenSaved => {
            open_saved(backend, offer, &destination).await?;
            backend.paths.document = destination;
            backend.startup_opened = true;
        }
        Choice::Discard => unreachable!("discard returns before resolving a save destination"),
    }
    Ok(())
}

async fn open_saved(
    backend: &Backend,
    offer: &RecoveryInfoDto,
    destination: &Path,
) -> Result<(), String> {
    let original = offer
        .original_path
        .as_deref()
        .ok_or("No saved version exists")?;
    let saved = if backend.paths.policy == PathPolicy::Desktop {
        backend.paths.open_source(Path::new(original))?
    } else {
        crate::imported_documents::migrate(&backend.paths.documents, Path::new(original))?
    };
    if saved != destination {
        return Err("Saved version is not an admitted recovery destination".into());
    }
    api::open_document(
        &backend.state,
        backend.host.clone(),
        destination.to_string_lossy().into_owned(),
        Some("openSaved".into()),
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn has_offers(backend: &Backend) -> Result<bool, String> {
    Ok(!backend
        .shared
        .recovery
        .lock()
        .map_err(|e| e.to_string())?
        .offers
        .is_empty())
}

fn remove_offer(backend: &Backend, id: &str) -> Result<(), String> {
    backend
        .shared
        .recovery
        .lock()
        .map_err(|e| e.to_string())?
        .offers
        .retain(|offer| offer.document_id != id);
    Ok(())
}

fn recovery_destination(paths: &QaPaths, offer: &RecoveryInfoDto) -> Result<PathBuf, String> {
    destination_in(
        paths.policy,
        &paths.documents,
        &paths.recovery_directory()?,
        offer,
    )
}

#[cfg(test)]
fn destination(documents: &Path, offer: &RecoveryInfoDto) -> Result<PathBuf, String> {
    destination_in(PathPolicy::Qa, documents, documents, offer)
}

fn destination_in(
    policy: PathPolicy,
    documents: &Path,
    recovered: &Path,
    offer: &RecoveryInfoDto,
) -> Result<PathBuf, String> {
    let documents = documents.canonicalize().map_err(|e| e.to_string())?;
    let id: onecad_core::ids::DocumentId = offer
        .document_id
        .parse()
        .map_err(|e| format!("Invalid recovery identity: {e}"))?;
    if let Some(path) = offer
        .original_path
        .as_deref()
        .and_then(|path| Path::new(path).canonicalize().ok())
        && (policy == PathPolicy::Desktop || path.starts_with(&documents))
    {
        verify_destination_identity(&path, id)?;
        let target = if policy == PathPolicy::Desktop {
            path
        } else {
            crate::imported_documents::migrate(&documents, &path)?
        };
        verify_destination_identity(&target, id)?;
        return Ok(target);
    }
    let fallback = recovered
        .canonicalize()
        .map_err(|error| error.to_string())?
        .join(format!("recovered-{id}.onecad"));
    if fallback
        .symlink_metadata()
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err("Recovery save destination must not be a symlink".into());
    }
    if fallback.exists() {
        verify_destination_identity(&fallback, id)?;
    }
    Ok(fallback)
}

fn verify_destination_identity(
    path: &Path,
    expected: onecad_core::ids::DocumentId,
) -> Result<(), String> {
    let document = onecad_core::io::container::ContainerReader::open(path)
        .map_err(|error| error.to_string())?;
    if document.document().id != expected {
        return Err(
            "Recovery path belongs to a different document; saved source and checkpoint retained"
                .into(),
        );
    }
    Ok(())
}

pub fn show(app: &mut crate::app::App, ctx: &egui::Context) -> bool {
    let Ok(recovery) = app.doc.bridge.shared.recovery.lock() else {
        return true;
    };
    if recovery.deferred || (recovery.checked && recovery.offers.is_empty()) {
        return false;
    }
    let document_open = recovery.opened;
    let offer = recovery.offers.first().cloned();
    let busy = recovery.busy;
    drop(recovery);
    let title = offer.as_ref().map_or("Checking crash recovery…", |offer| {
        offer.title.as_deref().unwrap_or("Recovered document")
    });
    let body = "Restore keeps recovered changes unsaved. Open saved and Discard permanently remove this recovery copy.";
    let open_saved = offer.as_ref().is_some_and(|o| o.original_path.is_some());
    let mut dialog = Dialog::new(DialogIntent::Decide, "Recover unsaved work")
        .icon("unsaved")
        .body(body)
        .primary("Restore", offer.is_some())
        .secondary("Open saved", open_saved)
        .tertiary("Discard recovery")
        .disabled(busy || offer.is_none());
    if offer.is_none() {
        dialog = dialog.body("Checking crash recovery…");
    }
    let out = dialog.show(ctx, egui::Id::unique("startup-recovery"), |ui| {
        if let Some(path) = offer.as_ref().and_then(|o| o.original_path.as_deref()) {
            ui.label(format!("{title} — {path}"));
        } else if offer.is_some() {
            ui.label(title);
        }
        if !app.doc.status.is_empty() {
            ui.add_space(6.0);
            ui.label(egui::RichText::new(&app.doc.status).small());
        }
    });
    let choice = if out.primary {
        Some(Choice::Restore)
    } else if out.secondary {
        Some(Choice::OpenSaved)
    } else if out.tertiary {
        Some(Choice::Discard)
    } else {
        None
    };
    if out.dismissed && choice.is_none() {
        if let Ok(mut recovery) = app.doc.bridge.shared.recovery.lock() {
            recovery.deferred = true;
        }
        return true;
    }
    if let (Some(offer), Some(choice)) = (offer, choice) {
        // A host with an open document never replaces it: restoring another
        // document opens a tab of its own, which takes over the remaining offers.
        if document_open
            && !matches!(choice, Choice::Discard)
            && let Err(error) = hand_offers_to_new_tab(app)
        {
            app.doc.status = error;
            return true;
        }
        if let Ok(mut recovery) = app.doc.bridge.shared.recovery.lock() {
            recovery.busy = true;
        }
        if let Err(error) = app.doc.bridge.submit(Command::Recovery {
            document_id: offer.document_id,
            choice,
        }) {
            app.doc.status = error;
            if let Ok(mut recovery) = app.doc.bridge.shared.recovery.lock() {
                recovery.busy = false;
            }
        }
    }
    true
}

#[cfg(test)]
pub(crate) fn tests_hand_offers(app: &mut crate::app::App) -> Result<(), String> {
    hand_offers_to_new_tab(app)
}

fn hand_offers_to_new_tab(app: &mut crate::app::App) -> Result<(), String> {
    if app.busy() {
        return Err("Wait for the current command before restoring another document".into());
    }
    let offers = std::mem::take(
        &mut app
            .doc
            .bridge
            .shared
            .recovery
            .lock()
            .map_err(|e| e.to_string())?
            .offers,
    );
    if let Err(error) = app.open_tab() {
        if let Ok(mut recovery) = app.doc.bridge.shared.recovery.lock() {
            recovery.offers = offers;
        }
        return Err(error);
    }
    let mut recovery = app
        .doc
        .bridge
        .shared
        .recovery
        .lock()
        .map_err(|e| e.to_string())?;
    recovery.checked = true;
    recovery.offers = offers;
    Ok(())
}

#[cfg(test)]
#[path = "startup_recovery_tests.rs"]
mod tests;
