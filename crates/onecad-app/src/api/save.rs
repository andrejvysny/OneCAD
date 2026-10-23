//! Explicit saves retain ownership through disk write and fenced completion.
use super::*;
use crate::{host_claims::Ownership, reservations::Claim};
use std::sync::Arc;
use tokio::sync::Mutex;

/// Saves the open document (`CadClient` save). `path` `None` reuses the last save
/// path; an unsaved document with no path is an error (the frontend's Save action
/// then falls back to Save As). Records the saved path in the recents store.
///
/// `preview_png` is an optional base64 PNG capture of the viewport, stored as the
/// container's `preview.png` and served to the start screen by
/// [`list_recents`](crate::api::list_recents). It is **entirely optional**: a
/// frontend that does not send it (and Tauri passes `None` for an argument the
/// caller omits entirely) keeps whatever thumbnail the document already had, and a
/// malformed or oversized capture is dropped with a warning. A thumbnail never
/// fails a save.
///
/// ## Off the single-writer lock (VF-B7 / plan F1a)
///
/// The two slow parts of a save — the worker `SaveCheckpoint` round-trip and the
/// container write (JSON + deflate of every import blob) — used to run with the
/// runtime lock held, stalling every edit for their whole duration. They now run
/// unlocked, in five phases:
///
/// 1. locked: resolve the target path, admit a checkpoint
///    ([`prepare_checkpoint`](DocumentRuntime::prepare_checkpoint)), clone the engine;
/// 2. **unlocked**: `SaveCheckpoint`; re-lock briefly to
///    [`adopt_checkpoint`](DocumentRuntime::adopt_checkpoint), whose fencing recheck
///    discards artifacts an interleaved edit or worker restart invalidated;
/// 3. locked: [`build_save_payload`](DocumentRuntime::build_save_payload) + the
///    revision it describes;
/// 4. **unlocked**, under the persistence lane: the container write;
/// 5. locked: [`mark_saved`](DocumentRuntime::mark_saved) — adopt the path, and go
///    clean only if no edit landed during the write. Clear recovery only for this
///    clean runtime, still under the persistence lane.
///
/// Every re-lock re-checks the runtime session: `close_document` / `open_document`
/// can interleave now that the lock is dropped, and the tail of a save must never be
/// applied to a replacement, even when it reopens the same document identity.
#[tracing::instrument(skip_all, fields(path = ?path), err(Display))]
pub async fn save_document(
    state: &AppState,
    app: HostContext,
    path: Option<String>,
    preview_png: Option<String>,
) -> Result<SaveOutcomeDto, ApiError> {
    let preview_png = decode_preview_png(preview_png);
    let ownership = Ownership::new(state);
    let (target, document_id, session, ticket, engine, target_claim, mut leases) = {
        let guard = state.runtime.lock().await;
        let rt = guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("save".into()))?;
        let target = path
            .map(PathBuf::from)
            .or_else(|| rt.path().map(Path::to_path_buf))
            .ok_or_else(|| ApiError::Io("no save path; provide one".into()))?;
        if let Some(root) = autosave::autosave_root(&app) {
            let recovery = onecad_core::io::recovery::autosave_dir(&root);
            if crate::reservations::canonical(&target)
                .starts_with(crate::reservations::canonical(&recovery))
            {
                return Err(ApiError::InvalidCommand(
                    "save target belongs to crash recovery; choose another location".into(),
                ));
            }
        }
        let target_claim = state.claim_path(&target)?;
        let leases = ownership.lease(rt.document_uuid())?;
        (
            target,
            rt.document_uuid(),
            rt.runtime_session(),
            rt.prepare_checkpoint(),
            rt.engine_arc(),
            target_claim,
            leases,
        )
    };
    if let Some(ticket) = ticket {
        if let Ok(artifacts) = engine.save_checkpoint(ticket.step()).await {
            let mut guard = state.runtime.lock().await;
            if let Some(rt) = guard
                .as_mut()
                .filter(|rt| rt.matches_runtime_session(&session))
            {
                rt.adopt_checkpoint(ticket, artifacts);
            }
        }
    }
    let (payload, revision) = {
        let mut guard = state.runtime.lock().await;
        let rt = guard
            .as_mut()
            .filter(|rt| rt.matches_runtime_session(&session))
            .ok_or_else(|| ApiError::NoDocument("save".into()))?;
        let revision = rt.revision();
        let caches = SaveCaches {
            meshes: true,
            preview_png,
        };
        (rt.build_save_payload(save_meta(), caches), revision)
    };
    // No write is started before admission; cancelling this wait releases staged leases.
    let lane = state.persistence.clone().lock_owned().await;
    leases.push(target_claim.clone());
    let completion = Completion {
        runtime: state.runtime.clone(),
        ownership,
        services: state.services().clone(),
        app,
        target,
        document_id,
        session,
        revision,
        target_claim,
    };
    #[cfg(test)]
    let hooks = state.save_hooks.lock().unwrap().take();
    crate::persistence::spawn(lane, leases, move |admission| async move {
        let target = completion.target.clone();
        let app = completion.app.clone();
        #[cfg(test)]
        let (write_hooks, finish_hooks) = hooks.map(crate::persistence::Hooks::split).unzip();
        tokio::task::spawn_blocking(move || {
            let _admission = admission;
            #[cfg(test)]
            if let Some(hooks) = write_hooks {
                hooks.wait();
            }
            DocumentRuntime::write_payload(&target, &payload)?;
            recents::record(&app, &target);
            Ok::<(), onecad_core::io::IoError>(())
        })
        .await
        .map_err(|error| ApiError::Io(format!("save task did not complete: {error}")))?
        .map_err(|error| ApiError::Io(error.to_string()))?;
        #[cfg(test)]
        if let Some(hooks) = finish_hooks {
            hooks.wait().await;
        }
        completion.finish().await
    })
    .await
    .map_err(|error| ApiError::Io(format!("save completion did not finish: {error}")))?
}

struct Completion {
    runtime: Arc<Mutex<Option<DocumentRuntime>>>,
    ownership: Ownership,
    services: Arc<crate::process::ProcessServices>,
    app: HostContext,
    target: PathBuf,
    document_id: DocumentId,
    session: String,
    revision: onecad_core::ids::DocumentRevision,
    target_claim: Claim,
}

impl Completion {
    async fn finish(self) -> Result<SaveOutcomeDto, ApiError> {
        let outcome = {
            let mut guard = self.runtime.lock().await;
            let rt = guard
                .as_mut()
                .filter(|rt| rt.matches_runtime_session(&self.session))
                .ok_or_else(|| {
                    ApiError::NoDocument("save completed after document close".into())
                })?;
            self.ownership.adopt_target(self.target_claim);
            let clean = rt.mark_saved(&self.target, self.revision);
            if let Some(root) = autosave::autosave_root(&self.app) {
                if clean
                    && !self
                        .services
                        .pending_recovery()
                        .contains_key(&self.document_id)
                {
                    autosave::clear_recovery_state(&root, self.document_id);
                } else {
                    autosave::retarget_recovery(&root, rt, self.ownership.source_paths());
                }
            }
            SaveOutcomeDto {
                document_id: rt.document_id(),
                saved_revision: self.revision.0,
                current_revision: rt.revision().0,
                clean,
                path: self.target.to_string_lossy().into_owned(),
                title: rt.title().to_string(),
            }
        };
        Ok(outcome)
    }
}
