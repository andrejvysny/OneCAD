//! A history move solves outside the runtime lock, then adopts under the same visit fence.
use super::*;
use crate::dto::{SketchHistoryDto, SketchHistoryScope};
use onecad_core::edit::HistoryDirection;

pub async fn sketch_history_with_completion(
    state: &AppState,
    app: HostContext,
    scope: SketchHistoryScope,
    direction: HistoryDirection,
) -> Result<CommandCompletion<SketchHistoryDto>, ApiError> {
    let prepared = {
        let guard = state.runtime.lock().await;
        let runtime = guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("sketchHistory".into()))?;
        match runtime
            .prepare_sketch_history(scope.clone(), direction)
            .await?
        {
            Some(prepared) => prepared,
            None => {
                return Ok(CommandCompletion {
                    result: Ok(runtime.sketch_history_result(&scope, direction, false)?),
                    projection: runtime.projection(),
                    regen_expected: false,
                });
            }
        }
    };
    let driven = prepared.drive().await?;
    let (result, report, solve, projection) = {
        let mut guard = state.runtime.lock().await;
        let runtime = guard
            .as_mut()
            .ok_or_else(|| ApiError::NoDocument("sketchHistory".into()))?;
        let (result, report, solve) = runtime.commit_sketch_history(driven)?;
        (result, report, solve, runtime.projection())
    };
    let _ = app.emit(AppEvent::SketchSolved(solve));
    let _ = app.emit(AppEvent::ProjectionUpdated(projection.clone()));
    let mut regen_expected = false;
    if let (Some(scheduler), Some(request)) = (state.scheduler.get(), report.regen) {
        scheduler.request(request);
        regen_expected = true;
    }
    state.note_mutation();
    Ok(CommandCompletion {
        result: Ok(result),
        projection,
        regen_expected,
    })
}
