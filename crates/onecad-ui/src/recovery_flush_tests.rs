use super::*;
use onecad_app::{
    api,
    host::{AppEvent, DialogPort, EventSink, FileDialog, HostPaths},
    worker::PendingBackend,
};
use std::{future::Future, path::Path, task::Poll};
use tokio::sync::{Notify, oneshot};

struct HeldDialog {
    started: Notify,
    answer: tokio::sync::Mutex<Option<oneshot::Receiver<()>>>,
    events: Mutex<Vec<onecad_app::autosave::AutosaveEvent>>,
}
impl EventSink for HeldDialog {
    fn emit(&self, event: AppEvent) -> std::result::Result<(), String> {
        if let AppEvent::Autosave(event) = event {
            self.events.lock().unwrap().push(event);
        }
        Ok(())
    }
}
#[async_trait::async_trait]
impl DialogPort for HeldDialog {
    async fn pick(&self, _: FileDialog) -> Vec<String> {
        self.started.notify_one();
        if let Some(answer) = self.answer.lock().await.take() {
            let _ = answer.await;
        }
        Vec::new()
    }
}

async fn fixture(
    root: &Path,
) -> (
    Arc<AppState>,
    HostContext,
    Arc<HeldDialog>,
    oneshot::Sender<()>,
) {
    let state = Arc::new(AppState::new(Arc::new(|| {
        let backend = Arc::new(PendingBackend);
        (
            backend.clone(),
            backend.clone(),
            backend.clone(),
            backend.clone(),
            backend.clone(),
            backend.clone(),
            backend.clone(),
            backend.clone(),
            backend.clone(),
            backend,
        )
    })));
    let (answer, waiting) = oneshot::channel();
    let dialogs = Arc::new(HeldDialog {
        started: Notify::new(),
        answer: tokio::sync::Mutex::new(Some(waiting)),
        events: Mutex::new(Vec::new()),
    });
    let host = HostContext {
        paths: HostPaths {
            app_data: Some(root.into()),
            app_config: Some(root.join("config")),
        },
        events: dialogs.clone(),
        dialogs: dialogs.clone(),
    };
    api::new_document(&state, host.clone()).await.unwrap();
    api::upsert_variable(&state, host.clone(), "accepted".into(), 42.0)
        .await
        .unwrap();
    (state, host, dialogs, answer)
}

async fn ticket(state: &AppState) -> Ticket {
    let state = api::get_document_state(state).await.unwrap();
    Ticket(Arc::new(Mutex::new(RecoveryResult {
        generation: NEXT.fetch_add(1, Ordering::Relaxed),
        document_id: state.document_id,
        runtime_session: state.runtime_session,
        outcome: Status::Pending,
        durable_outcome: None,
    })))
}

#[tokio::test]
async fn held_save_dialog_blocks_old_order_but_independent_checkpoint_completes() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, dialogs, answer) = fixture(area.path()).await;
    let documents = area.path().join("documents");
    std::fs::create_dir(&documents).unwrap();
    let checkpoint_ticket = ticket(&state).await;
    let mut old_order = Box::pin(async {
        crate::document_services::save_to_qa(&state, host.clone(), &documents, true)
            .await
            .unwrap();
        checkpoint(&state, &host, &checkpoint_ticket).await;
    });
    std::future::poll_fn(|context| {
        assert!(old_order.as_mut().poll(context).is_pending());
        Poll::Ready(())
    })
    .await;
    dialogs.started.notified().await;
    assert_eq!(
        checkpoint_ticket.snapshot().outcome,
        Status::Pending,
        "old actor ordering cannot reach recovery while dialog awaits"
    );
    let queue = Arc::new(Mutex::new(vec![checkpoint_ticket.clone()]));
    let notified = Arc::new(Notify::new());
    let completed = Arc::new(Notify::new());
    let signal = completed.clone();
    let (stop, stopped) = oneshot::channel();
    let service = tokio::spawn(run(
        state.clone(),
        host.clone(),
        queue,
        notified.clone(),
        stopped,
        move || signal.notify_one(),
    ));
    notified.notify_one();
    completed.notified().await;
    assert_eq!(checkpoint_ticket.snapshot().outcome, Status::Written);
    let id = checkpoint_ticket.snapshot().document_id.parse().unwrap();
    let path = onecad_core::io::recovery::autosave_path(area.path(), id);
    let saved = onecad_core::io::container::ContainerReader::open(&path).unwrap();
    assert_eq!(
        saved.document().variables.len(),
        1,
        "accepted edit reached real recovery container"
    );
    std::future::poll_fn(|context| {
        assert!(old_order.as_mut().poll(context).is_pending());
        Poll::Ready(())
    })
    .await;
    answer.send(()).unwrap();
    old_order.await;
    stop.send(()).unwrap();
    service.await.unwrap();
}

#[tokio::test]
async fn held_writer_expiry_keeps_terminal_generation_and_actual_durable_result() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, _, _) = fixture(area.path()).await;
    let first = ticket(&state).await;
    let lane = state.persistence.lock().await;
    let mut write = Box::pin(checkpoint(&state, &host, &first));
    std::future::poll_fn(|context| {
        assert!(write.as_mut().poll(context).is_pending());
        Poll::Ready(())
    })
    .await;
    assert_eq!(first.snapshot().outcome, Status::Pending);
    first.finish_waiting(true);
    let second = ticket(&state).await;
    drop(lane);
    write.await;
    assert_eq!(first.snapshot().outcome, Status::Expired);
    assert_eq!(first.snapshot().durable_outcome, Some(Status::Written));
    assert_eq!(second.snapshot().outcome, Status::Pending);
    checkpoint(&state, &host, &second).await;
    assert_eq!(second.snapshot().outcome, Status::Written);
    assert_ne!(first.snapshot().generation, second.snapshot().generation);
}

#[tokio::test]
async fn cancellation_failure_and_document_attribution_stay_distinct() {
    let area = tempfile::tempdir().unwrap();
    let (first, host, _, _) = fixture(area.path()).await;
    let cancelled = ticket(&first).await;
    cancelled.finish_waiting(false);
    checkpoint(&first, &host, &cancelled).await;
    assert_eq!(cancelled.snapshot().outcome, Status::Cancelled);
    let (second, second_host, _, _) = fixture(area.path()).await;
    let failed = ticket(&second).await;
    std::fs::write(area.path().join("autosave"), b"blocked directory").unwrap();
    checkpoint(&second, &second_host, &failed).await;
    assert!(matches!(failed.snapshot().outcome, Status::Failed(_)));
    assert_ne!(
        cancelled.snapshot().document_id,
        failed.snapshot().document_id
    );
}

#[tokio::test]
async fn stopping_before_queue_admission_cancels_every_ticket() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, _, _) = fixture(area.path()).await;
    let first = ticket(&state).await;
    let second = ticket(&state).await;
    let queue = Arc::new(Mutex::new(vec![first.clone(), second.clone()]));
    let notified = Arc::new(Notify::new());
    notified.notify_one();
    let (stop, stopped) = oneshot::channel();
    stop.send(()).unwrap();
    run(state, host, queue.clone(), notified, stopped, || {}).await;
    assert!(queue.lock().unwrap().is_empty());
    for ticket in [first, second] {
        assert_eq!(ticket.snapshot().outcome, Status::Cancelled);
        assert_eq!(ticket.snapshot().durable_outcome, Some(Status::Skipped));
    }
    assert!(!area.path().join("autosave").exists());
}

#[tokio::test]
async fn shutdown_cancels_waiting_lane_before_admission() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, _, _) = fixture(area.path()).await;
    let ticket = ticket(&state).await;
    let lane = state.persistence.lock().await;
    let queue = Arc::new(Mutex::new(vec![ticket.clone()]));
    let notified = Arc::new(Notify::new());
    let (stop, stopped) = oneshot::channel();
    let mut service = Box::pin(run(
        state.clone(),
        host,
        queue,
        notified.clone(),
        stopped,
        || {},
    ));
    notified.notify_one();
    std::future::poll_fn(|context| {
        assert!(service.as_mut().poll(context).is_pending());
        Poll::Ready(())
    })
    .await;
    assert_eq!(ticket.snapshot().outcome, Status::Pending);
    stop.send(()).unwrap();
    std::future::poll_fn(|context| {
        assert!(
            service.as_mut().poll(context).is_ready(),
            "stop must not wait for an unadmitted persistence lane"
        );
        Poll::Ready(())
    })
    .await;
    assert_eq!(ticket.snapshot().outcome, Status::Cancelled);
    assert_eq!(ticket.snapshot().durable_outcome, Some(Status::Skipped));
    assert!(!area.path().join("autosave").exists());
    drop(lane);
}

#[tokio::test]
async fn cancellation_after_lane_admission_keeps_lane_until_durable_write_finishes() {
    let area = tempfile::tempdir().unwrap();
    let (state, _, _, _) = fixture(area.path()).await;
    let requested = ticket(&state).await.snapshot();
    let runtime = state.runtime.lock().await;
    let cancel = Notify::new();
    let mut work = Box::pin(onecad_app::autosave::checkpoint_until(
        &state.runtime,
        area.path(),
        &state.persistence,
        Some(&requested.runtime_session),
        cancel.notified(),
    ));
    std::future::poll_fn(|context| {
        assert!(work.as_mut().poll(context).is_pending());
        Poll::Ready(())
    })
    .await;
    assert!(
        state.persistence.try_lock().is_err(),
        "checkpoint is admitted before snapshot"
    );
    cancel.notify_one();
    std::future::poll_fn(|context| {
        assert!(work.as_mut().poll(context).is_pending());
        Poll::Ready(())
    })
    .await;
    assert!(
        state.persistence.try_lock().is_err(),
        "cancellation cannot release an admitted transaction"
    );
    drop(runtime);
    assert!(matches!(work.await, AutosaveAttempt::Written(_)));
    assert!(state.persistence.try_lock().is_ok());
}

#[tokio::test]
async fn idle_and_ticket_flushes_emit_fenced_failure_success_and_skip_no_events() {
    for ticketed in [false, true] {
        let area = tempfile::tempdir().unwrap();
        let (state, host, sink, _) = fixture(area.path()).await;
        let identity = api::get_document_state(&state).await.unwrap();
        let blocker = area.path().join("autosave");
        std::fs::write(&blocker, b"blocked directory").unwrap();
        flush_once(&state, &host, ticketed).await;
        std::fs::remove_file(blocker).unwrap();
        flush_once(&state, &host, ticketed).await;
        {
            let events = sink.events.lock().unwrap();
            assert_eq!(events.len(), 2);
            assert!(events[0].error.is_some());
            assert!(events[1].error.is_none());
            assert!(events[1].attempt_id > events[0].attempt_id);
            for event in events.iter() {
                assert_eq!(event.document_id, identity.document_id);
                assert_eq!(event.runtime_session, identity.runtime_session);
                assert_eq!(event.document_revision, identity.revision);
            }
        }
        api::save_document(
            &state,
            host.clone(),
            Some(area.path().join("saved.onecad").to_string_lossy().into()),
            None,
        )
        .await
        .unwrap();
        flush_once(&state, &host, ticketed).await;
        assert_eq!(sink.events.lock().unwrap().len(), 2);
    }
}

#[tokio::test]
async fn stopping_idle_flush_still_emits_admitted_durable_completion() {
    let area = tempfile::tempdir().unwrap();
    let (state, host, sink, _) = fixture(area.path()).await;
    let runtime = state.runtime.lock().await;
    let (stop, mut stopped) = oneshot::channel();
    let mut work = Box::pin(flush_idle(&state, &host, &mut stopped));
    std::future::poll_fn(|context| {
        assert!(work.as_mut().poll(context).is_pending());
        Poll::Ready(())
    })
    .await;
    assert!(state.persistence.try_lock().is_err());
    stop.send(()).unwrap();
    std::future::poll_fn(|context| {
        assert!(work.as_mut().poll(context).is_pending());
        Poll::Ready(())
    })
    .await;
    drop(runtime);
    assert!(!work.await);
    let events = sink.events.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert!(events[0].error.is_none());
    assert!(Path::new(&events[0].path).is_file());
    assert!(state.persistence.try_lock().is_ok());
}

async fn flush_once(state: &AppState, host: &HostContext, ticketed: bool) {
    if ticketed {
        checkpoint(state, host, &ticket(state).await).await;
    } else {
        let (_stop, mut stopped) = oneshot::channel();
        assert!(flush_idle(state, host, &mut stopped).await);
    }
}
