//! Recovery checkpoints execute independently of document commands and native dialogs.
use crate::bridge::Shared;
use onecad_app::{autosave::AutosaveAttempt, host::HostContext, state::AppState};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(1);
const MAX_PENDING: usize = 16;

#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
#[serde(tag = "status", content = "error", rename_all = "camelCase")]
pub enum Status {
    Pending,
    Written,
    Skipped,
    Failed(String),
    Expired,
    Cancelled,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryResult {
    pub generation: u64,
    pub document_id: String,
    pub runtime_session: String,
    pub outcome: Status,
    pub durable_outcome: Option<Status>,
}

#[derive(Clone)]
pub struct Ticket(Arc<Mutex<RecoveryResult>>);

impl Ticket {
    pub(crate) fn request(shared: &Shared) -> Self {
        let (state, error) = match shared.document_state.lock() {
            Ok(slot) => (slot.latest(), None),
            Err(_) => (None, Some("Document state poisoned".to_owned())),
        };
        let ticket = Self(Arc::new(Mutex::new(RecoveryResult {
            generation: NEXT.fetch_add(1, Ordering::Relaxed),
            document_id: state
                .as_ref()
                .map_or_else(String::new, |(_, state)| state.document_id.clone()),
            runtime_session: state
                .as_ref()
                .map_or_else(String::new, |(_, state)| state.runtime_session.clone()),
            outcome: error.map_or_else(
                || {
                    if state.is_some() {
                        Status::Pending
                    } else {
                        Status::Skipped
                    }
                },
                Status::Failed,
            ),
            durable_outcome: None,
        })));
        if ticket.snapshot().outcome == Status::Pending {
            if let Ok(mut requests) = shared.recovery_requests.lock() {
                if shared.stopping.load(Ordering::Acquire) {
                    ticket.finish_waiting(false);
                    ticket.complete(Status::Skipped);
                } else if requests.len() < MAX_PENDING {
                    requests.push(ticket.clone());
                } else {
                    ticket.complete(Status::Failed("Recovery queue is full".into()));
                }
            } else {
                ticket.complete(Status::Failed("Recovery queue poisoned".into()));
            }
            shared.recovery_flush.notify_one();
        }
        ticket
    }

    pub fn snapshot(&self) -> RecoveryResult {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub fn finish_waiting(&self, expired: bool) {
        let mut result = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if result.outcome == Status::Pending {
            result.outcome = if expired {
                Status::Expired
            } else {
                Status::Cancelled
            };
        }
    }

    fn complete(&self, outcome: Status) {
        let mut result = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if result.outcome == Status::Pending {
            result.outcome = outcome.clone();
        }
        result.durable_outcome = Some(outcome);
    }
}

pub(crate) fn spawn(
    state: Arc<AppState>,
    host: HostContext,
    shared: Arc<Shared>,
) -> (
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let queue = shared.recovery_requests.clone();
    let notified = shared.recovery_flush.clone();
    let task = tokio::spawn(run(state, host, queue, notified, stopped, move || {
        shared.wake()
    }));
    (stop, task)
}

type Requests = Arc<Mutex<Vec<Ticket>>>;

fn cancel_queued(queue: &Requests) {
    let tickets = std::mem::take(&mut *queue.lock().unwrap_or_else(|error| error.into_inner()));
    for ticket in tickets {
        ticket.finish_waiting(false);
        ticket.complete(Status::Skipped);
    }
}

async fn run(
    state: Arc<AppState>,
    host: HostContext,
    queue: Requests,
    notified: Arc<tokio::sync::Notify>,
    mut stopped: tokio::sync::oneshot::Receiver<()>,
    wake: impl Fn(),
) {
    loop {
        tokio::select! {
            biased;
            _ = &mut stopped => break,
            _ = notified.notified() => {}
        }
        let tickets = std::mem::take(&mut *queue.lock().unwrap_or_else(|error| error.into_inner()));
        if tickets.is_empty() && !flush_idle(&state, &host, &mut stopped).await {
            break;
        }
        if !flush_tickets(&state, &host, tickets, &mut stopped, &wake).await {
            break;
        }
    }
    cancel_queued(&queue);
    wake();
}

async fn flush_idle(
    state: &AppState,
    host: &HostContext,
    stopped: &mut tokio::sync::oneshot::Receiver<()>,
) -> bool {
    let Some(root) = onecad_app::autosave::autosave_root(host) else {
        return true;
    };
    let cancel = tokio::sync::Notify::new();
    let work = onecad_app::autosave::checkpoint_until(
        &state.runtime,
        &root,
        &state.persistence,
        None,
        cancel.notified(),
    );
    tokio::pin!(work);
    let (outcome, keep_running) = tokio::select! {
        biased;
        _ = stopped => { cancel.notify_one(); (work.await, false) }
        outcome = &mut work => (outcome, true),
    };
    publish_checkpoint(host, &outcome);
    if let AutosaveAttempt::Failed(event) = outcome {
        tracing::warn!(error = ?event.error, "Recovery checkpoint failed");
    }
    keep_running
}

fn publish_checkpoint(host: &HostContext, outcome: &AutosaveAttempt) {
    if let AutosaveAttempt::Written(event) | AutosaveAttempt::Failed(event) = outcome {
        let _ = host.emit(onecad_app::host::AppEvent::Autosave(event.clone()));
    }
}

async fn flush_tickets(
    state: &AppState,
    host: &HostContext,
    tickets: Vec<Ticket>,
    stopped: &mut tokio::sync::oneshot::Receiver<()>,
    wake: &impl Fn(),
) -> bool {
    let mut remaining = tickets.into_iter();
    while let Some(ticket) = remaining.next() {
        let cancel = tokio::sync::Notify::new();
        let work = checkpoint_until(state, host, &ticket, cancel.notified());
        tokio::pin!(work);
        tokio::select! {
            biased;
            _ = &mut *stopped => {
                ticket.finish_waiting(false);
                cancel.notify_one();
                work.await;
                for ticket in remaining { ticket.finish_waiting(false); ticket.complete(Status::Skipped); }
                return false;
            }
            () = &mut work => {}
        }
        wake();
    }
    true
}

#[cfg(test)]
async fn checkpoint(state: &AppState, host: &HostContext, ticket: &Ticket) {
    checkpoint_until(state, host, ticket, std::future::pending()).await;
}

async fn checkpoint_until(
    state: &AppState,
    host: &HostContext,
    ticket: &Ticket,
    stop: impl std::future::Future<Output = ()>,
) {
    let requested = ticket.snapshot();
    if requested.outcome != Status::Pending {
        ticket.complete(Status::Skipped);
        return;
    }
    let Some(root) = onecad_app::autosave::autosave_root(host) else {
        ticket.complete(Status::Failed("Recovery storage is unavailable".into()));
        return;
    };
    let outcome = onecad_app::autosave::checkpoint_until(
        &state.runtime,
        &root,
        &state.persistence,
        Some(&requested.runtime_session),
        stop,
    )
    .await;
    publish_checkpoint(host, &outcome);
    ticket.complete(match outcome {
        AutosaveAttempt::Skipped => Status::Skipped,
        AutosaveAttempt::Written(_) => Status::Written,
        AutosaveAttempt::Failed(event) => {
            tracing::warn!(generation = requested.generation, document_id = %requested.document_id, error = ?event.error, "Recovery checkpoint failed");
            Status::Failed(event.error.unwrap_or_else(|| "Checkpoint failed".into()))
        }
    });
}

#[cfg(test)]
#[path = "recovery_flush_tests.rs"]
mod tests;
