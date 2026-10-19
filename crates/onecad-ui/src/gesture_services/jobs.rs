use super::*;
use onecad_app::{
    dto::{NativeGestureEnd, NativeGestureReceipt},
    state::AppState,
};
use onecad_core::math::Vec2;

pub(super) enum Reply {
    Ready(
        Box<
            onecad_app::dto::NativeGestureReceipt<
                onecad_app::dto::NativeGesturePreview<onecad_app::dto::BeginGestureDto>,
            >,
        >,
    ),
    Updated(
        Box<
            onecad_app::dto::NativeGestureReceipt<
                Option<onecad_app::dto::NativeGesturePreview<onecad_app::dto::DragSolveDto>>,
            >,
        >,
    ),
    End {
        receipt: Box<NativeGestureReceipt<NativeGestureEnd>>,
        projection: Box<onecad_app::dto::DocumentProjection>,
        completion: Result<(), String>,
    },
    Cancelled,
}
pub(crate) struct ResultMessage {
    pub(super) tag: GestureTag,
    pub(super) result: Result<Reply, String>,
}

pub(super) fn spawn(
    tag: GestureTag,
    sender: tokio::sync::mpsc::Sender<ResultMessage>,
    future: impl std::future::Future<Output = Result<Reply, String>> + Send + 'static,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let result = future.await;
        let _ = sender.send(ResultMessage { tag, result }).await;
    })
}

pub(super) async fn ready(handle: api::NativeGestureHandle) -> Result<Reply, String> {
    let reply = api::ready_native_gesture(&handle)
        .await
        .map_err(|error| error.to_string())?;
    scope::receipt(&handle, &reply)?;
    crate::record_budget::check_payload(&reply)?;
    Ok(Reply::Ready(Box::new(reply)))
}

pub(super) async fn update(handle: api::NativeGestureHandle, point: Vec2) -> Result<Reply, String> {
    let reply = api::update_native_gesture(&handle, [point.x, point.y])
        .await
        .map_err(|error| error.to_string())?;
    scope::receipt(&handle, &reply)?;
    crate::record_budget::check_payload(&reply)?;
    Ok(Reply::Updated(Box::new(reply)))
}

pub(super) async fn cancel(handle: api::NativeGestureHandle) -> Result<Reply, String> {
    api::cancel_native_gesture(&handle)
        .await
        .map_err(|error| error.to_string())?;
    Ok(Reply::Cancelled)
}

pub(super) async fn end(
    state: Arc<AppState>,
    host: onecad_app::host::HostContext,
    shared: Arc<crate::bridge::Shared>,
    events: terminal::Reservation,
    handle: api::NativeGestureHandle,
    point: Vec2,
) -> Result<Reply, String> {
    let receipt =
        api::end_native_gesture_with_completion(&state, host, &handle, Some([point.x, point.y]))
            .await
            .map_err(|error| {
                format!("Gesture End completion unavailable; authoring was not reverted: {error}")
            })?;
    let result = receipt.result.map_err(|error| error.to_string())?;
    scope::receipt(&handle, &result)?;
    let projection = receipt.projection;
    let cleanup_error = result.value.cleanup_error.clone();
    let mut completion = if receipt.regen_expected {
        events
            .mailbox
            .retarget(&projection.runtime_session, projection.revision);
        wait(
            &state,
            &shared,
            &events.mailbox,
            &projection.runtime_session,
            projection.revision,
        )
        .await
    } else {
        Ok(())
    };
    if let Some(error) = cleanup_error {
        completion = Err(match completion {
            Ok(()) => error,
            Err(prior) => format!("{prior}; {error}"),
        });
    }
    #[cfg(test)]
    testing::end_gate(&shared).await;
    Ok(Reply::End {
        receipt: Box::new(result),
        projection: Box::new(projection),
        completion,
    })
}

async fn wait(
    state: &AppState,
    shared: &crate::bridge::Shared,
    events: &crate::event_mailbox::EventMailbox,
    session: &str,
    revision: u64,
) -> Result<(), String> {
    let wait = async {
        loop {
            if shared.stopping.load(std::sync::atomic::Ordering::Acquire) {
                return Err(
                    "Host shutdown during gesture regeneration; accepted history was preserved"
                        .into(),
                );
            }
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {},
                event = events.next() => match event {
                    crate::bridge::BackendEvent::WorkerFailed if crate::services::current_worker_failed(state) => return Err("Geometry worker failed; accepted history was preserved".into()),
                    crate::bridge::BackendEvent::Regen(result) if result.runtime_session == session && result.source_revision == revision => {
                        return crate::sketch_completion::completion_error(&result).map_or(Ok(()), Err);
                    }
                    crate::bridge::BackendEvent::Regen(result) if result.runtime_session != session || result.source_revision > revision => return Err("Gesture regeneration was superseded; accepted history was preserved".into()),
                    _ => {},
                }
            }
        }
    };
    tokio::time::timeout(std::time::Duration::from_secs(30), wait)
        .await
        .map_err(|_| "Gesture regeneration timed out; accepted history was preserved".to_string())?
}
