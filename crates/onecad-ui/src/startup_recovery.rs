//! Startup decisions precede opening any saved UUID and remain explicit.
use crate::bridge::{Backend, Command};
use onecad_app::{api, dto::RecoveryInfoDto};
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
        recovery.offer(id)?.clone()
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
    let destination = destination(&backend.paths.root, offer)?;
    match choice {
        Choice::Restore => {
            api::recover_document(&backend.state, backend.host.clone(), id.clone(), true)
                .await
                .map_err(|e| e.to_string())?
                .ok_or("Recovery offer is no longer pending")?;
            backend.paths.document = destination;
            backend.startup_opened = true;
        }
        Choice::OpenSaved => {
            open_saved(backend, offer, &destination).await?;
            backend.paths.document = destination;
            backend.startup_opened = true;
        }
        Choice::Discard => {
            api::recover_document(&backend.state, backend.host.clone(), id.clone(), false)
                .await
                .map_err(|e| e.to_string())?;
        }
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
    if Path::new(original)
        .canonicalize()
        .map_err(|e| e.to_string())?
        != destination
    {
        return Err("Saved version must remain inside the isolated QA documents directory".into());
    }
    api::open_document(
        &backend.state,
        backend.host.clone(),
        original.into(),
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

fn destination(root: &Path, offer: &RecoveryInfoDto) -> Result<PathBuf, String> {
    let documents = root
        .join("documents")
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if let Some(path) = offer
        .original_path
        .as_deref()
        .and_then(|path| Path::new(path).canonicalize().ok())
        && path.starts_with(&documents)
    {
        return Ok(path);
    }
    let id: onecad_core::ids::DocumentId = offer
        .document_id
        .parse()
        .map_err(|e| format!("Invalid recovery identity: {e}"))?;
    let fallback = documents.join(format!("recovered-{id}.onecad"));
    if fallback
        .symlink_metadata()
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err("Recovery save destination must not be a symlink".into());
    }
    Ok(fallback)
}

pub fn show(app: &mut crate::app::App, ctx: &egui::Context) -> bool {
    let Ok(recovery) = app.bridge.shared.recovery.lock() else {
        return true;
    };
    if recovery.opened || (recovery.checked && recovery.offers.is_empty()) {
        return false;
    }
    let offer = recovery.offers.first().cloned();
    let busy = recovery.busy;
    drop(recovery);
    let mut choice = None;
    egui::Modal::new(egui::Id::unique("startup-recovery")).show(ctx, |ui| {
        ui.heading("Recover unsaved work");
        let Some(offer) = &offer else { ui.label("Checking crash recovery…"); return; };
        ui.label(offer.title.as_deref().unwrap_or("Recovered document"));
        if let Some(path) = &offer.original_path { ui.label(path); }
        ui.label("Restore keeps recovered changes unsaved. Open saved and Discard permanently remove this recovery copy.");
        ui.add_enabled_ui(!busy, |ui| {
            if ui.button("Restore").clicked() { choice = Some(Choice::Restore); }
            if ui.add_enabled(offer.original_path.is_some(), egui::Button::new("Open saved — discard recovery")).clicked() { choice = Some(Choice::OpenSaved); }
            if ui.button("Discard recovery").clicked() { choice = Some(Choice::Discard); }
        });
        ui.label(&app.status);
    });
    if let (Some(offer), Some(choice)) = (offer, choice) {
        if let Ok(mut recovery) = app.bridge.shared.recovery.lock() {
            recovery.busy = true;
        }
        if let Err(error) = app.bridge.submit(Command::Recovery {
            document_id: offer.document_id,
            choice,
        }) {
            app.status = error;
            if let Ok(mut recovery) = app.bridge.shared.recovery.lock() {
                recovery.busy = false;
            }
        }
    }
    true
}

#[cfg(test)]
#[path = "startup_recovery_tests.rs"]
mod tests;
