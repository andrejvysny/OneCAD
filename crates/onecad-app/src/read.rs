//! Typed, read-only application data for hosts that do not serialize commands.

use onecad_core::{
    document::record::OperationRecord,
    ids::{DocumentRevision, RecordId, SketchId, SnapshotId, WorkerEpoch},
    regen::HistoryPrefixHash,
    sketch::{Sketch, SketchPlane},
};

use crate::{error::ApiError, state::AppState};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewContext {
    pub document_id: String,
    pub runtime_session: String,
    pub revision: DocumentRevision,
    pub worker_epoch: WorkerEpoch,
    pub snapshot_id: SnapshotId,
    pub expected_base_hash: HistoryPrefixHash,
}

pub async fn get_sketch_plane(
    state: &AppState,
    sketch_id: SketchId,
) -> Result<SketchPlane, ApiError> {
    let guard = state.runtime.lock().await;
    let runtime = guard
        .as_ref()
        .ok_or_else(|| ApiError::NoDocument("sketchPlane".into()))?;
    Ok(runtime
        .sketch_snapshot(sketch_id, "sketchPlane")?
        .effective_plane())
}

/// Returns owned sketch data without opening a solver session or changing history.
pub async fn get_sketch_snapshot(
    state: &AppState,
    sketch_id: SketchId,
) -> Result<Sketch, ApiError> {
    let guard = state.runtime.lock().await;
    let runtime = guard
        .as_ref()
        .ok_or_else(|| ApiError::NoDocument("sketchSnapshot".into()))?;
    Ok(runtime.sketch_snapshot(sketch_id, "sketchSnapshot")?)
}

pub async fn get_operation_record(
    state: &AppState,
    record_id: RecordId,
) -> Result<OperationRecord, ApiError> {
    let guard = state.runtime.lock().await;
    let runtime = guard
        .as_ref()
        .ok_or_else(|| ApiError::NoDocument("operation".into()))?;
    runtime
        .operation_record(record_id)
        .ok_or_else(|| ApiError::InvalidCommand(format!("unknown record {record_id}")))
}

/// Captures one worker head without holding the document lock across worker I/O.
pub async fn preview_context(state: &AppState) -> Result<PreviewContext, ApiError> {
    let (session, revision, engine) = {
        let guard = state.runtime.lock().await;
        let runtime = guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("previewContext".into()))?;
        (
            runtime.runtime_session(),
            runtime.revision(),
            runtime.engine_arc(),
        )
    };
    let head = engine.get_worker_head().await?;
    let guard = state.runtime.lock().await;
    let runtime = guard
        .as_ref()
        .ok_or_else(|| ApiError::NoDocument("previewContext".into()))?;
    if !runtime.matches_runtime_session(&session)
        || runtime.revision() != revision
        || runtime.fencing().get().1 != head.worker_epoch
        || runtime.head_snapshot_id().unwrap_or(SnapshotId(0)) != head.snapshot_id
    {
        return Err(ApiError::StalePreview(
            "preview context belongs to a stale document head".into(),
        ));
    }
    Ok(PreviewContext {
        document_id: runtime.document_id(),
        runtime_session: session,
        revision,
        worker_epoch: head.worker_epoch,
        snapshot_id: head.snapshot_id,
        expected_base_hash: head.history_prefix_hash,
    })
}
