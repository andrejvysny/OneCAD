//! Closing detaches a runtime once; its persistence reconciliation survives the waiter.
use super::*;
use crate::host_claims::Ownership;

/// Closes the document, retaining dirty recovery work after queued writes drain.
/// Clean documents discard stale recovery state. Unresolved offers stay protected.
pub async fn close_document(state: &AppState, app: HostContext) -> Result<(), ApiError> {
    close_document_inner(state, app, None, true).await
}

/// Explicitly discards the runtime named by the user's close decision.
/// Session validation and runtime removal share one lock to reject stale consent.
pub async fn discard_and_close_document(
    state: &AppState,
    app: HostContext,
    expected_runtime_session: &str,
) -> Result<(), ApiError> {
    close_document_inner(state, app, Some(expected_runtime_session), true).await
}

/// Closes the document like [`close_document`] (or [`discard_and_close_document`]
/// when `discard_session` names the user's discard decision) and then tears the
/// whole host down: its regen scheduler and autosave lanes stop, and its workers
/// are retired and reaped. The process worker pool is left alone — other hosts
/// share it — so nothing is pre-warmed on this host's behalf.
///
/// This is the close path for one of several open documents, and the quit path of
/// a host whose runtime outlives it.
pub async fn shutdown_host(
    state: &AppState,
    app: HostContext,
    discard_session: Option<&str>,
) -> Result<(), ApiError> {
    let cleanup = start_close(state, app, discard_session, false).await?;
    // Detachment and consent validation have succeeded. Stop and retire before
    // awaiting disk cleanup: a cancelled waiter must not leave this host running.
    if let Some(scheduler) = state.scheduler.get() {
        scheduler.shutdown();
    }
    let tasks = state.tasks.begin_stop();
    let workers = state.retire_host_workers();
    finish_close(cleanup).await?;
    tasks
        .await
        .map_err(|error| ApiError::Io(format!("host tasks did not finish: {error}")))?;
    AppState::reap_host_workers(&workers).await;
    Ok(())
}

async fn close_document_inner(
    state: &AppState,
    app: HostContext,
    discard_session: Option<&str>,
    rewarm: bool,
) -> Result<(), ApiError> {
    finish_close(start_close(state, app, discard_session, rewarm).await?).await
}

async fn start_close(
    state: &AppState,
    app: HostContext,
    discard_session: Option<&str>,
    rewarm: bool,
) -> Result<tokio::task::JoinHandle<()>, ApiError> {
    let (closed, claims) = {
        let mut guard = state.runtime.lock().await;
        if discard_session.is_some_and(|expected| {
            !guard
                .as_ref()
                .is_some_and(|rt| rt.matches_runtime_session(expected))
        }) {
            return Err(ApiError::InvalidCommand(
                "discard close belongs to a stale runtime".into(),
            ));
        }
        if !rewarm {
            // Close admission under the same lock as detachment. Cancelling
            // before admission or refusing stale consent leaves the host intact.
            state.begin_closing();
        }
        let closed = guard.as_ref().map(|rt| (rt.document_uuid(), rt.is_dirty()));
        *guard = None;
        state.invalidate_backend_stage();
        state.clear_restart_owner();
        // Retire this runtime's worker while publication is excluded. Doing it
        // after a cancellable lane wait could retire a replacement or leak it.
        if rewarm {
            state.rewarm();
        }
        (closed, Ownership::new(state).take_claims())
    };
    // Never await the writer lane while holding the runtime lock. Admitted
    // autosaves finish before cleanup; queued old-session snapshots fail their fence.
    #[cfg(debug_assertions)]
    if let Some((id, _)) = closed {
        render_watchdog_retire_document(&id.to_string(), "document closed");
    }
    let runtime = state.runtime.clone();
    let lane = state.persistence.clone();
    let services = state.services().clone();
    let discard = discard_session.is_some();
    // Start before the next suspension: cancelling close must not strand a live
    // marker after an admitted autosave finishes, nor leak the closed leases.
    Ok(tokio::spawn(async move {
        let _claims = claims;
        let _lane = lane.lock_owned().await;
        let current = runtime.lock().await;
        reconcile_closed_recovery(&services, &app, closed, current.as_ref(), discard);
        if current.is_none() {
            #[cfg(debug_assertions)]
            render_watchdog_reconcile_projection(&DocumentProjection::empty(), "document closed");
            let _ = app.emit(AppEvent::ProjectionUpdated(DocumentProjection::empty()));
        }
    }))
}

async fn finish_close(cleanup: tokio::task::JoinHandle<()>) -> Result<(), ApiError> {
    cleanup
        .await
        .map_err(|error| ApiError::Io(format!("close cleanup did not finish: {error}")))
}

fn reconcile_closed_recovery(
    services: &crate::process::ProcessServices,
    app: &HostContext,
    closed: Option<(DocumentId, bool)>,
    current: Option<&DocumentRuntime>,
    discard: bool,
) {
    let (Some((id, dirty)), Some(root)) = (closed, autosave::autosave_root(app)) else {
        return;
    };
    if current.is_some_and(|runtime| runtime.document_uuid() == id)
        || services.pending_recovery().contains_key(&id)
    {
        return;
    }
    if dirty && !discard {
        if let Err(error) = onecad_core::io::recovery::release_marker(&root, id) {
            tracing::warn!(%error, "close: release recovery owner failed");
        }
    } else {
        autosave::clear_recovery_state(&root, id);
    }
}
