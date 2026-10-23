//! Read-only prefix operands, with one retained task per document runtime.
use super::edit_preview::{ensure_context, ensure_runtime_fence};
use crate::worker::{
    OperandBodyBounds, OperandContextGeometry, OperandContextPreview, OperandGeometry,
};
use crate::{error::ApiError, read::PreviewContext, state::AppState};
use onecad_core::{
    ids::{BodyId, RecordId},
    regen::{CancelToken, EngineError, OpFailureCode, StepStatus, StoppedReason},
};
use std::sync::Arc;
use tokio::sync::oneshot;

#[derive(Debug, Clone)]
pub struct EditOperandContext {
    pub fence: PreviewContext,
    pub record: RecordId,
    pub bodies: Vec<BodyId>,
    pub operands: Vec<OperandGeometry>,
    pub body_bounds: Vec<OperandBodyBounds>,
    pub refusals: Vec<String>,
}
pub struct EditOperandRead {
    pub receiver: oneshot::Receiver<Result<EditOperandContext, ApiError>>,
    cancel: CancelToken,
}
impl Drop for EditOperandRead {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
struct CancelOnDrop(CancelToken);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

/// Starts one supervised read; replacing it or publishing cancels its worker job.
pub async fn start_edit_operand_context(
    state: Arc<AppState>,
    fence: PreviewContext,
    record: RecordId,
) -> Result<EditOperandRead, ApiError> {
    let (seq, cancel, request) = prepare(&state, &fence, record).await?;
    let expected: Vec<_> = request
        .prefix
        .plan
        .ops
        .iter()
        .map(|op| op.step_index)
        .collect();
    let (sender, receiver) = oneshot::channel();
    let engine = state.preview();
    let weak = Arc::downgrade(&state);
    let task_fence = fence.clone();
    let task_cancel = cancel.clone();
    let (start, started) = oneshot::channel();
    let task = tokio::spawn(async move {
        if started.await.is_err() {
            return;
        }
        let _cancel = CancelOnDrop(task_cancel.clone());
        let result = engine
            .edit_operand_context(request, task_cancel)
            .await
            .map_err(map_error);
        let Some(state) = weak.upgrade() else { return };
        let result = complete(&state, &task_fence, record, seq, &expected, result).await;
        let _ = sender.send(result);
    });
    retain(&state, &fence, seq, task).await;
    let _ = start.send(());
    Ok(EditOperandRead { receiver, cancel })
}

async fn prepare(
    state: &AppState,
    fence: &PreviewContext,
    record: RecordId,
) -> Result<(u64, CancelToken, crate::worker::EditOperandRequest), ApiError> {
    ensure_context(state, fence).await?;
    let mut guard = state.runtime.lock().await;
    let runtime = guard
        .as_mut()
        .ok_or_else(|| ApiError::NoDocument("edit operands".into()))?;
    ensure_runtime_fence(runtime, fence)?;
    runtime.plan_edit_operands(record, fence)
}
async fn retain(
    state: &AppState,
    fence: &PreviewContext,
    seq: u64,
    task: tokio::task::JoinHandle<()>,
) {
    let mut guard = state.runtime.lock().await;
    match guard
        .as_mut()
        .filter(|runtime| runtime.matches_runtime_session(&fence.runtime_session))
    {
        Some(runtime) => runtime.attach_operand_read(seq, task),
        None => task.abort(),
    }
    drop(guard);
}

async fn complete(
    state: &AppState,
    fence: &PreviewContext,
    record: RecordId,
    seq: u64,
    expected: &[usize],
    result: Result<OperandContextPreview, ApiError>,
) -> Result<EditOperandContext, ApiError> {
    {
        let mut guard = state.runtime.lock().await;
        let runtime = guard
            .as_mut()
            .ok_or_else(|| ApiError::NoDocument("edit operands".into()))?;
        if !runtime.matches_runtime_session(&fence.runtime_session)
            || !runtime.finish_operand_read(seq)
        {
            return Err(ApiError::StalePreview("operand read superseded".into()));
        }
        ensure_runtime_fence(runtime, fence)?;
    }
    ensure_context(state, fence).await?;
    context_from_preview(fence, record, expected, result?)
}

fn context_from_preview(
    fence: &PreviewContext,
    record: RecordId,
    expected: &[usize],
    result: OperandContextPreview,
) -> Result<EditOperandContext, ApiError> {
    let actual: Vec<_> = result
        .prefix
        .per_step
        .iter()
        .map(|step| step.step_index)
        .collect();
    if result.prefix.snapshot_id != fence.snapshot_id
        || result.prefix.stopped_reason != StoppedReason::Completed
        || actual != expected
        || result
            .prefix
            .per_step
            .iter()
            .any(|step| step.status != StepStatus::Ok)
    {
        return Err(ApiError::Unsupported(
            "authoritative feature prefix did not replay completely".into(),
        ));
    }
    let mut operands = Vec::new();
    let mut body_bounds = Vec::new();
    let mut refusals = Vec::new();
    for input in result.inputs {
        match input {
            Ok(OperandContextGeometry::Element(operand)) => operands.push(*operand),
            Ok(OperandContextGeometry::Body(bounds)) => body_bounds.push(bounds),
            Err(reason) => refusals.push(reason),
        }
    }
    Ok(EditOperandContext {
        fence: fence.clone(),
        record,
        bodies: result.eligible_bodies,
        operands,
        body_bounds,
        refusals,
    })
}
fn map_error(error: EngineError) -> ApiError {
    match error {
        EngineError::OpFailed {
            code: OpFailureCode::Unsupported,
            message,
            ..
        } => ApiError::Unsupported(message),
        EngineError::Cancelled => ApiError::Cancelled("operand read cancelled".into()),
        error => error.into(),
    }
}
