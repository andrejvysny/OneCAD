//! Native gesture admission returns a handle before any worker request can settle.
use super::*;
use crate::document_runtime::native_gestures::{control::*, driver, NativeGestureOwner};
use crate::dto::{
    NativeGestureEnd, NativeGesturePreview, NativeGestureReceipt, NativeGestureScope,
};
use std::sync::Arc;
use tokio::sync::oneshot;

/// Last caller handle disappearing cancels the retained driver, including pending Begin.
#[derive(Clone)]
pub struct NativeGestureHandle {
    owner: Arc<HandleOwner>,
}
struct HandleOwner(Arc<Control>);
impl Drop for HandleOwner {
    fn drop(&mut self) {
        self.0.cancel();
    }
}
impl NativeGestureHandle {
    pub fn gesture_id(&self) -> u64 {
        self.owner.0.id
    }
    pub fn scope(&self) -> &NativeGestureScope {
        &self.owner.0.scope
    }
}
impl std::fmt::Debug for NativeGestureHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativeGestureHandle")
            .field("gesture_id", &self.gesture_id())
            .field("scope", self.scope())
            .finish()
    }
}

/// Admits the caller's expected document/edit visit without waiting for worker readiness.
/// Update, End and Cancel may use the handle immediately; dropping its last clone cancels.
pub async fn begin_native_gesture(
    state: &AppState,
    scope: NativeGestureScope,
    target: wire::GestureTarget,
) -> Result<NativeGestureHandle, ApiError> {
    let mut guard = state.runtime.lock().await;
    let runtime = guard
        .as_mut()
        .ok_or_else(|| ApiError::NoDocument("beginNativeGesture".into()))?;
    {
        let slot = state.native_gestures.lock().unwrap();
        if let Some(control) = slot.as_ref() {
            if control.poisoned_claim.lock().unwrap().is_some() {
                return Err(ApiError::Worker("Native gesture solver lane remains quarantined after uncertain cleanup; restart the application".into()));
            }
            return Err(refused("Native gesture driver or cleanup is still active"));
        }
    }
    let prepared = runtime.prepare_native_gesture(scope.clone(), target)?;
    let id = driver::next_id()?;
    let control = Control::new(id, scope);
    {
        let mut slot = state.native_gestures.lock().unwrap();
        if slot.is_some() {
            return Err(refused("Native gesture driver or cleanup is still active"));
        }
        *slot = Some(control.clone());
    }
    runtime.native_gesture = Some(NativeGestureOwner {
        id,
        sketch: prepared.scope.sketch_id,
        control: control.clone(),
    });
    driver::spawn(state, prepared, control.clone(), target);
    Ok(NativeGestureHandle {
        owner: Arc::new(HandleOwner(control)),
    })
}

pub async fn ready_native_gesture(
    handle: &NativeGestureHandle,
) -> Result<NativeGestureReceipt<NativeGesturePreview<BeginGestureDto>>, ApiError> {
    let control = &handle.owner.0;
    let cancel = CancelOnDrop::new(control.clone());
    let mut ready = control.ready.subscribe();
    let result = loop {
        if let Some(result) = ready.borrow_and_update().clone() {
            break result;
        }
        ready
            .changed()
            .await
            .map_err(|_| refused("Native gesture driver closed"))?;
    };
    cancel.disarm();
    result.map(|value| control.receipt(value))
}

/// Replaces only the queued target. `None` means that request was coalesced by a newer target.
pub async fn update_native_gesture(
    handle: &NativeGestureHandle,
    target: [f64; 2],
) -> Result<NativeGestureReceipt<Option<NativeGesturePreview<DragSolveDto>>>, ApiError> {
    let control = &handle.owner.0;
    let cancel = CancelOnDrop::new(control.clone());
    let (send, receive) = oneshot::channel();
    control.update(target, send)?;
    let result = receive
        .await
        .map_err(|_| refused("Native gesture driver closed"))?;
    cancel.disarm();
    result.map(|value| control.receipt(value))
}

pub async fn end_native_gesture_with_completion(
    state: &AppState,
    host: HostContext,
    handle: &NativeGestureHandle,
    final_target: Option<[f64; 2]>,
) -> Result<CommandCompletion<NativeGestureReceipt<NativeGestureEnd>>, ApiError> {
    let control = &handle.owner.0;
    let cancel = CancelOnDrop::new(control.clone());
    if !state
        .native_gestures
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|owner| Arc::ptr_eq(owner, control))
    {
        return Err(refused(
            "Native gesture belongs to another application or has settled",
        ));
    }
    let (send, receive) = oneshot::channel();
    control.end(EndRequest {
        target: final_target,
        host,
        reply: send,
    })?;
    let mut result = receive
        .await
        .map_err(|_| refused("Native gesture driver closed"))?;
    cancel.disarm();
    if let Err(error) = cancel_native_gesture(handle).await {
        if let Ok(completion) = &mut result {
            if let Ok(receipt) = &mut completion.result {
                receipt.value.cleanup_error = Some(error.to_string());
            }
        }
    }
    result
}

/// Retires ownership immediately; the bounded driver still owns remote cleanup and its lane.
pub async fn cancel_native_gesture(handle: &NativeGestureHandle) -> Result<(), ApiError> {
    let control = &handle.owner.0;
    control.cancel();
    let mut settled = control.settled.subscribe();
    loop {
        if let Some(result) = settled.borrow_and_update().clone() {
            return result;
        }
        settled
            .changed()
            .await
            .map_err(|_| refused("Native gesture driver closed"))?;
    }
}
