//! ONECAD-369 edit preview service: preview an edit of an existing record by a
//! non-publishing full replay (SCHEMA §7.6 `PreviewPlan`).
//!
//! Latest-wins (a new preview cancels the previous one), cancellable (dropping
//! the returned future cancels the worker job; a publishing regen preempts it)
//! and fenced twice: `api::preview_context` is compared OUTSIDE the runtime lock
//! before and after the worker call (it does worker IO itself), and the
//! runtime-local fields are compared INSIDE the lock at both phases.

use onecad_core::document::record::Operation;
use onecad_core::ids::{RecordId, SnapshotId};
use onecad_core::regen::{CancelToken, Lod};

use crate::document_runtime::edit_preview::{drive_edit_preview, EditPreviewPrep};
pub use crate::document_runtime::edit_preview::{EditPreview, EditPreviewOutcome};
use crate::document_runtime::DocumentRuntime;
use crate::error::ApiError;
use crate::read::{preview_context, PreviewContext};
use crate::state::AppState;

/// Cancels the preview's worker job if the caller abandons this future.
struct CancelOnDrop(CancelToken);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

/// Previews replacing `record`'s operation with `op`, fenced to `fence`.
///
/// Never mutates the document: no revision bump, no dirty flag, no undo entry,
/// no checkpoint and no mesh-cache write.
///
/// # Errors
/// `StalePreview` when the head or document moved (or a newer preview
/// superseded this one), `Cancelled`, `Unsupported` (the worker lacks
/// `plan.preview`, a mesh budget refusal), `InvalidCommand` for a candidate the
/// commit would reject, or the worker-class error as-is.
pub async fn preview_existing_edit(
    state: &AppState,
    fence: PreviewContext,
    record: RecordId,
    op: Operation,
    lod: Lod,
) -> Result<EditPreview, ApiError> {
    ensure_context(state, &fence).await?;
    let (job, engine) = {
        let mut guard = state.runtime.lock().await;
        let runtime = guard
            .as_mut()
            .ok_or_else(|| ApiError::NoDocument("previewExistingEdit".into()))?;
        ensure_runtime_fence(runtime, &fence)?;
        match runtime.plan_edit_preview(record, op, lod)? {
            EditPreviewPrep::Done(preview) => {
                return Ok(EditPreview {
                    snapshot_id: fence.snapshot_id,
                    ..*preview
                });
            }
            EditPreviewPrep::Run(job) => (job, state.preview()),
        }
    };
    let _abandon = CancelOnDrop(job.cancel_token());
    let run = drive_edit_preview(&job, engine.as_ref()).await;
    let preview = {
        let mut guard = state.runtime.lock().await;
        let runtime = guard
            .as_mut()
            .ok_or_else(|| ApiError::NoDocument("previewExistingEdit".into()))?;
        if !runtime.matches_runtime_session(&fence.runtime_session) {
            return Err(stale());
        }
        let preview = runtime.finish_edit_preview(*job, run);
        ensure_runtime_fence(runtime, &fence)?;
        preview?
    };
    ensure_context(state, &fence).await?;
    if preview.snapshot_id != fence.snapshot_id {
        return Err(stale());
    }
    Ok(preview)
}

fn stale() -> ApiError {
    ApiError::StalePreview("edit preview belongs to an obsolete document head".into())
}

async fn ensure_context(state: &AppState, fence: &PreviewContext) -> Result<(), ApiError> {
    if preview_context(state).await? == *fence {
        Ok(())
    } else {
        Err(stale())
    }
}

/// The runtime-local half of the fence; no worker IO, safe under the lock.
fn ensure_runtime_fence(runtime: &DocumentRuntime, fence: &PreviewContext) -> Result<(), ApiError> {
    let (revision, epoch) = runtime.fencing().get();
    if runtime.matches_runtime_session(&fence.runtime_session)
        && revision == fence.revision
        && epoch == fence.worker_epoch
        && runtime.head_snapshot_id().unwrap_or(SnapshotId(0)) == fence.snapshot_id
    {
        Ok(())
    } else {
        Err(stale())
    }
}
