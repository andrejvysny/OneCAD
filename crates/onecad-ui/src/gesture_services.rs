//! A gesture retains one admission permit, one operation task and one cancellation task.
use crate::bridge::{Backend, Display, Message};
use onecad_app::api;
use onecad_interaction::sketch::gesture::{GestureAction, GestureEffect, GestureTag};
use std::sync::{Arc, mpsc};
use tokio::sync::{OwnedSemaphorePermit, mpsc as channel};
mod jobs;
mod scope;
mod terminal;
#[cfg(test)]
pub(crate) mod testing;

pub(crate) const CAPACITY: usize = 2;

pub(crate) struct Request {
    pub effect: GestureEffect,
    pub expected: api::PreviewContext,
    pub permit: Option<OwnedSemaphorePermit>,
}

pub(crate) struct Delivery {
    pub tag: GestureTag,
    pub actions: Vec<GestureAction>,
    pub display: Option<Arc<Display>>,
    pub error: Option<String>,
    pub accepted: bool,
    pub accepted_revision: Option<u64>,
    pub moved: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Beginning,
    Idle,
    Updating,
    Ending,
    Cancelling,
}
struct Owner {
    tag: GestureTag,
    expected: api::PreviewContext,
    handle: api::NativeGestureHandle,
    permit: Option<OwnedSemaphorePermit>,
    task: tokio::task::JoinHandle<()>,
    cancellation: Option<tokio::task::JoinHandle<()>>,
    cancellation_tag: Option<GestureTag>,
    cancellation_error: Option<String>,
    phase: Phase,
}
impl Drop for Owner {
    fn drop(&mut self) {
        self.task.abort();
        if let Some(task) = &self.cancellation {
            task.abort();
        }
    }
}

pub(crate) struct Service {
    owner: Option<Owner>,
    sender: channel::Sender<jobs::ResultMessage>,
    receiver: channel::Receiver<jobs::ResultMessage>,
}
impl Service {
    pub fn new() -> Self {
        let (sender, receiver) = channel::channel(CAPACITY);
        Self {
            owner: None,
            sender,
            receiver,
        }
    }
    pub fn ending(&self) -> bool {
        self.owner
            .as_ref()
            .is_some_and(|owner| owner.phase == Phase::Ending)
    }
    pub async fn next(&mut self) -> Option<jobs::ResultMessage> {
        self.receiver.recv().await
    }

    pub async fn submit(
        &mut self,
        backend: &Backend,
        request: Request,
        sender: &mpsc::Sender<Message>,
    ) {
        let tag = tag(&request.effect).clone();
        if let GestureEffect::Begin { intent, .. } = request.effect {
            self.begin(
                backend,
                tag,
                intent,
                request.expected,
                request.permit,
                sender,
            )
            .await;
            return;
        }
        let Some(owner) = self
            .owner
            .as_mut()
            .filter(|owner| scope::owns(&owner.tag, &tag))
        else {
            return unowned(request.effect, sender, request.permit);
        };
        if tag.sequence <= owner.tag.sequence {
            return;
        }
        match request.effect {
            GestureEffect::Cancel { worker_id, .. }
                if worker_id.is_none_or(|id| id == owner.handle.gesture_id()) =>
            {
                if owner.phase == Phase::Ending {
                    if owner.cancellation.is_none() {
                        owner.cancellation_tag = Some(tag.clone());
                        owner.cancellation = Some(jobs::spawn(
                            tag,
                            self.sender.clone(),
                            jobs::cancel(owner.handle.clone()),
                        ));
                    }
                } else if owner.phase != Phase::Cancelling {
                    owner.task.abort();
                    owner.tag = tag.clone();
                    owner.phase = Phase::Cancelling;
                    owner.task =
                        jobs::spawn(tag, self.sender.clone(), jobs::cancel(owner.handle.clone()));
                }
            }
            GestureEffect::Update {
                worker_id, point, ..
            } if worker_id == owner.handle.gesture_id() && owner.phase == Phase::Idle => {
                owner.tag = tag.clone();
                owner.phase = Phase::Updating;
                owner.task = jobs::spawn(
                    tag,
                    self.sender.clone(),
                    jobs::update(owner.handle.clone(), point),
                );
            }
            GestureEffect::End {
                worker_id, point, ..
            } if worker_id == owner.handle.gesture_id() && owner.phase == Phase::Idle => {
                owner.tag = tag.clone();
                owner.phase = Phase::Ending;
                let events = match terminal::Reservation::new(
                    backend.shared.gesture_events.clone(),
                    &owner.expected,
                ) {
                    Ok(events) => events,
                    Err(error) => {
                        owner.phase = Phase::Idle;
                        return send(sender, failed(tag, error), None);
                    }
                };
                owner.task = jobs::spawn(
                    tag,
                    self.sender.clone(),
                    jobs::end(
                        backend.state.clone(),
                        backend.host.clone(),
                        backend.shared.clone(),
                        events,
                        owner.handle.clone(),
                        point,
                    ),
                );
            }
            _ => send(
                sender,
                failed(
                    tag,
                    "Gesture request is stale or another solve is in flight".into(),
                ),
                None,
            ),
        }
    }

    async fn begin(
        &mut self,
        backend: &Backend,
        tag: GestureTag,
        intent: onecad_interaction::sketch::gesture::GestureIntent,
        expected: api::PreviewContext,
        permit: Option<OwnedSemaphorePermit>,
        sender: &mpsc::Sender<Message>,
    ) {
        let prepared = async {
            if self.owner.is_some() || permit.is_none() {
                return Err("Another gesture owns native admission".into());
            }
            let scope = scope::native(&tag, &expected)?;
            let target = scope::target(&intent)?;
            api::begin_native_gesture(&backend.state, scope, target)
                .await
                .map_err(|error| error.to_string())
        }
        .await;
        match prepared {
            Ok(handle) => {
                let task = jobs::spawn(
                    tag.clone(),
                    self.sender.clone(),
                    jobs::ready(handle.clone()),
                );
                self.owner = Some(Owner {
                    tag,
                    expected,
                    handle,
                    permit,
                    task,
                    cancellation: None,
                    cancellation_tag: None,
                    cancellation_error: None,
                    phase: Phase::Beginning,
                });
            }
            Err(error) => send(sender, failed(tag, error), permit),
        }
    }

    pub async fn receive(
        &mut self,
        backend: &mut Backend,
        message: jobs::ResultMessage,
        sender: &mpsc::Sender<Message>,
    ) {
        let Some(owner) = self
            .owner
            .as_mut()
            .filter(|owner| scope::owns(&owner.tag, &message.tag))
        else {
            return;
        };
        if message.tag != owner.tag {
            if owner.phase == Phase::Ending && owner.cancellation_tag.as_ref() == Some(&message.tag)
            {
                #[cfg(test)]
                backend
                    .shared
                    .gesture_cancel_observed
                    .store(true, std::sync::atomic::Ordering::Release);
                owner.cancellation_error = message.result.err();
            }
            return;
        }
        match message.result {
            Ok(jobs::Reply::Ready(reply)) => {
                owner.phase = Phase::Idle;
                let action = GestureAction::Ready {
                    tag: message.tag.clone(),
                    worker_id: reply.gesture_id,
                    preview: Box::new(reply.value.sketch),
                };
                send(sender, delivery(message.tag, vec![action]), None);
            }
            Ok(jobs::Reply::Updated(reply)) => {
                owner.phase = Phase::Idle;
                let Some(preview) = reply.value else {
                    return send(
                        sender,
                        failed(
                            message.tag,
                            "Gesture update was unexpectedly coalesced".into(),
                        ),
                        None,
                    );
                };
                let action = GestureAction::Updated {
                    tag: message.tag.clone(),
                    worker_id: reply.gesture_id,
                    worker_sequence: preview.solve.seq,
                    preview: Box::new(preview.sketch),
                };
                send(sender, delivery(message.tag, vec![action]), None);
            }
            Ok(jobs::Reply::End {
                receipt,
                projection,
                completion,
            }) => {
                let Some(mut owner) = self.owner.take() else {
                    return;
                };
                let moved = receipt.value.moved;
                let action = GestureAction::Completed {
                    tag: message.tag.clone(),
                    worker_id: receipt.gesture_id,
                    moved,
                };
                let mut actions = vec![action];
                if let Some(tag) = &owner.cancellation_tag {
                    actions.push(GestureAction::Cancelled { tag: tag.clone() });
                }
                let mut delivery = delivery(message.tag, actions);
                delivery.accepted = true;
                delivery.accepted_revision = Some(projection.revision);
                delivery.moved = moved;
                delivery.error = completion.err().or_else(|| owner.cancellation_error.take());
                finish_capture(backend, &owner, &projection, &mut delivery).await;
                send(sender, delivery, owner.permit.take());
            }
            Ok(jobs::Reply::Cancelled) => {
                let Some(mut owner) = self.owner.take() else {
                    return;
                };
                send(
                    sender,
                    delivery(
                        message.tag.clone(),
                        vec![GestureAction::Cancelled { tag: message.tag }],
                    ),
                    owner.permit.take(),
                );
            }
            Err(error) if owner.phase == Phase::Ending || owner.phase == Phase::Cancelling => {
                let Some(mut owner) = self.owner.take() else {
                    return;
                };
                let mut result = failed(message.tag, error.clone());
                if let Some(tag) = &owner.cancellation_tag {
                    result.actions.push(GestureAction::Failed {
                        tag: tag.clone(),
                        message: error,
                    });
                }
                send(sender, result, owner.permit.take());
            }
            Err(error) => {
                owner.phase = Phase::Idle;
                send(sender, failed(message.tag, error), None);
            }
        }
    }

    pub async fn shutdown(&mut self) {
        if let Some(mut owner) = self.owner.take() {
            if owner.phase != Phase::Ending {
                owner.task.abort();
            }
            let _ = api::cancel_native_gesture(&owner.handle).await;
            if owner.phase == Phase::Ending {
                let _ =
                    tokio::time::timeout(std::time::Duration::from_secs(30), &mut owner.task).await;
            }
        }
    }
}

async fn finish_capture(
    backend: &mut Backend,
    owner: &Owner,
    projection: &onecad_app::dto::DocumentProjection,
    delivery: &mut Delivery,
) {
    crate::document_state::refresh(backend).await;
    let capture = async {
        let current = api::preview_context(&backend.state)
            .await
            .map_err(|error| error.to_string())?;
        if current.document_id != projection.document_id
            || current.runtime_session != projection.runtime_session
            || current.revision.0 != projection.revision
            || current.worker_epoch != owner.expected.worker_epoch
        {
            return Err(
                "Gesture committed, but its document or worker changed before capture".into(),
            );
        }
        #[cfg(test)]
        if backend
            .shared
            .fail_next_gesture_capture
            .swap(false, std::sync::atomic::Ordering::AcqRel)
        {
            return Err("Injected gesture final capture failure".into());
        }
        let display = crate::capture::load_display(backend).await?;
        if display.context != current {
            return Err("Gesture committed, but its final scene capture became stale".into());
        }
        Ok(Arc::new(display))
    }
    .await;
    match capture {
        Ok(display) => delivery.display = Some(display),
        Err(error) => {
            delivery.error = Some(match delivery.error.take() {
                Some(prior) => format!("{prior}; {error}"),
                None => error,
            })
        }
    }
    backend.events.finish();
}

pub(crate) fn tag(effect: &GestureEffect) -> &GestureTag {
    match effect {
        GestureEffect::Begin { tag, .. }
        | GestureEffect::Update { tag, .. }
        | GestureEffect::End { tag, .. }
        | GestureEffect::Cancel { tag, .. } => tag,
    }
}
fn delivery(tag: GestureTag, actions: Vec<GestureAction>) -> Delivery {
    Delivery {
        tag,
        actions,
        display: None,
        error: None,
        accepted: false,
        accepted_revision: None,
        moved: false,
    }
}
fn failed(tag: GestureTag, error: String) -> Delivery {
    let mut result = delivery(
        tag.clone(),
        vec![GestureAction::Failed {
            tag,
            message: error.clone(),
        }],
    );
    result.error = Some(error);
    result
}
fn send(sender: &mpsc::Sender<Message>, delivery: Delivery, permit: Option<OwnedSemaphorePermit>) {
    let _ = sender.send(Message::Gesture {
        delivery: Box::new(delivery),
        _permit: permit,
    });
}

fn unowned(
    effect: GestureEffect,
    sender: &mpsc::Sender<Message>,
    permit: Option<OwnedSemaphorePermit>,
) {
    let tag = tag(&effect).clone();
    let result = if matches!(effect, GestureEffect::Cancel { .. }) {
        delivery(tag.clone(), vec![GestureAction::Cancelled { tag }])
    } else {
        failed(tag, "Gesture service no longer owns this request".into())
    };
    send(sender, result, permit);
}

#[cfg(test)]
mod tests;

pub(crate) fn observe(shared: &crate::bridge::Shared, event: crate::bridge::BackendEvent) {
    let observer = shared
        .gesture_events
        .lock()
        .ok()
        .and_then(|slot| slot.clone());
    if let Some(observer) = observer {
        let _ = observer.emit(event);
    }
}
