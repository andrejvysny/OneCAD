//! Explicit discard is finalized only after its replacement runtime is published.
use super::*;

/// Opaque consent bound to the active runtime, not merely its persistent UUID.
#[derive(Debug)]
pub struct ReplacementDiscard {
    document: DocumentId,
    session: String,
}

pub async fn prepare_replacement_discard(state: &AppState) -> Result<ReplacementDiscard, ApiError> {
    let guard = state.runtime.lock().await;
    let runtime = guard
        .as_ref()
        .ok_or_else(|| ApiError::NoDocument("discard replacement".into()))?;
    Ok(ReplacementDiscard {
        document: runtime.document_uuid(),
        session: runtime.runtime_session(),
    })
}

/// Refuses stale completion, unresolved offers, and a dirty reopened copy sharing
/// the previous UUID. Autosave's under-lane session fence prevents queued writes
/// from recreating this discarded runtime's recovery after cleanup.
pub async fn discard_replaced_document(
    state: &AppState,
    app: HostContext,
    ticket: ReplacementDiscard,
    expected_session: &str,
) -> Result<(), ApiError> {
    let _lane = state.persistence.lock().await;
    let guard = state.runtime.lock().await;
    let runtime = guard
        .as_ref()
        .ok_or_else(|| ApiError::NoDocument("discard replacement".into()))?;
    if runtime.runtime_session() != expected_session || ticket.session == expected_session {
        return Err(ApiError::InvalidCommand(
            "Discard completion belongs to an obsolete or unchanged runtime".into(),
        ));
    }
    if state.peek_pending_recovery(ticket.document).is_some()
        || (runtime.document_uuid() == ticket.document && runtime.is_dirty())
    {
        return Err(ApiError::RecoveryPending(
            "Discard cannot remove recovery owned by a pending offer or active dirty document"
                .into(),
        ));
    }
    if let Some(root) = autosave::autosave_root(&app) {
        onecad_core::io::recovery::remove_marker(&root, ticket.document)
            .map_err(|error| ApiError::Io(error.to_string()))?;
        match std::fs::remove_file(onecad_core::io::recovery::autosave_path(
            &root,
            ticket.document,
        )) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(ApiError::Io(error.to_string())),
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "replacement_tests.rs"]
mod tests;
