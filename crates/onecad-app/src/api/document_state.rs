//! Lifecycle reads must remain available when geometry cannot be projected.

use crate::{dto::DocumentState, error::ApiError, state::AppState};

const MAX_TITLE_BYTES: usize = 4096;
const MAX_PATH_BYTES: usize = 32768;

/// Reads only authoritative lifecycle fields under the document runtime lock.
pub async fn get_document_state(state: &AppState) -> Result<DocumentState, ApiError> {
    let guard = state.runtime.lock().await;
    Ok(match guard.as_ref() {
        Some(runtime) => DocumentState {
            document_id: runtime.document_id(),
            runtime_session: runtime.runtime_session(),
            revision: runtime.revision().0,
            dirty: runtime.is_dirty(),
            title: bounded_text(runtime.title(), MAX_TITLE_BYTES),
            path: runtime
                .path()
                .map(|path| bounded_text(&path.to_string_lossy(), MAX_PATH_BYTES)),
        },
        None => DocumentState::default(),
    })
}

fn bounded_text(value: &str, limit: usize) -> String {
    let mut end = value.len().min(limit);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

#[cfg(test)]
mod tests;
