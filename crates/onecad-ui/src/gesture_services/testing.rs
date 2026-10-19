//! Test-only bounded gates and authoritative readback; production gesture paths remain unchanged.
use super::*;
use onecad_app::{dto::SketchHistoryAvailability, state::AppState};
use onecad_core::{ids::SketchId, sketch::Sketch};
use tokio::sync::oneshot;

pub(super) async fn end_gate(shared: &crate::bridge::Shared) {
    let gate = shared
        .gesture_end_gate
        .lock()
        .ok()
        .and_then(|mut slot| slot.take());
    if let Some(gate) = gate {
        shared
            .gesture_end_arrived
            .store(true, std::sync::atomic::Ordering::Release);
        let _ = tokio::time::timeout(std::time::Duration::from_secs(10), gate).await;
    }
}

#[derive(Debug)]
pub(crate) struct InspectionRequest {
    pub expected: api::PreviewContext,
    pub sketch: SketchId,
    pub reply: oneshot::Sender<Result<Inspection, String>>,
}
#[derive(Debug)]
pub(crate) struct Inspection {
    pub sketch: Sketch,
    pub history: Option<SketchHistoryAvailability>,
    pub undo_depth: usize,
}
pub(crate) async fn inspect(state: &AppState, request: InspectionRequest) {
    let result = read(state, &request.expected, request.sketch).await;
    let _ = request.reply.send(result);
}
async fn read(
    state: &AppState,
    expected: &api::PreviewContext,
    sketch: SketchId,
) -> Result<Inspection, String> {
    if api::preview_context(state)
        .await
        .map_err(|error| error.to_string())?
        != *expected
    {
        return Err("Gesture test inspection source is obsolete".into());
    }
    let snapshot = api::get_sketch_snapshot(state, sketch)
        .await
        .map_err(|error| error.to_string())?;
    let projection = api::get_projection(state)
        .await
        .map_err(|error| error.to_string())?;
    let session = api::get_sketch(state, sketch.to_string())
        .await
        .map_err(|error| error.to_string())?;
    if api::preview_context(state)
        .await
        .map_err(|error| error.to_string())?
        != *expected
    {
        return Err("Gesture test inspection changed during read".into());
    }
    Ok(Inspection {
        sketch: snapshot,
        history: session.history,
        undo_depth: projection.undo_depth,
    })
}
