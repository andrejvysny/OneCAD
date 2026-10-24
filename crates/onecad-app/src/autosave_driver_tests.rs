use super::*;
use onecad_core::{
    document::variables::{Scalar, Variable},
    edit::EditCommand,
    ids::VariableId,
};
use std::{future::Future, task::Poll};
use tokio::sync::{mpsc, oneshot};

fn dirty() -> DocumentRuntime {
    let backend = Arc::new(crate::worker::PendingBackend);
    let mut runtime = DocumentRuntime::new_blank(backend.clone(), backend.clone(), backend);
    runtime
        .apply(EditCommand::AddVariable {
            variable: Variable {
                id: VariableId::new(),
                name: "Depth".into(),
                value: Scalar::new(1.0),
                unit: Default::default(),
            },
        })
        .unwrap();
    runtime
}

struct Driver {
    runtime: Arc<Mutex<Option<DocumentRuntime>>>,
    tick: watch::Sender<u64>,
    events: mpsc::Receiver<AutosaveEvent>,
    stop: oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}

fn driver(root: &Path) -> Driver {
    driver_with_timing(
        root,
        (
            Duration::from_millis(20),
            Duration::from_millis(60),
            Duration::from_millis(120),
        ),
    )
}

fn driver_with_timing(root: &Path, timing: (Duration, Duration, Duration)) -> Driver {
    let runtime = Arc::new(Mutex::new(Some(dirty())));
    let (tick, changed) = watch::channel(0);
    let (stop, stopped) = oneshot::channel();
    let (emitted, events) = mpsc::channel(8);
    let task = tokio::spawn(run_driver(
        runtime.clone(),
        root.into(),
        Arc::new(Mutex::new(())),
        changed,
        timing,
        async {
            let _ = stopped.await;
        },
        move |event| emitted.try_send(event).unwrap(),
    ));
    Driver {
        runtime,
        tick,
        events,
        stop,
        task,
    }
}

async fn event(driver: &mut Driver) -> AutosaveEvent {
    tokio::time::timeout(Duration::from_secs(2), driver.events.recv())
        .await
        .unwrap()
        .unwrap()
}

async fn stop(driver: Driver) {
    let _ = driver.stop.send(());
    tokio::time::timeout(Duration::from_secs(1), driver.task)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn replacement_mutation_consumed_during_retry_is_checkpointed_without_another_tick() {
    let area = tempfile::tempdir().unwrap();
    let blocker = area.path().join("autosave");
    std::fs::write(&blocker, b"blocked").unwrap();
    let mut driver = driver_with_timing(
        area.path(),
        (
            Duration::from_secs(1),
            Duration::from_millis(400),
            Duration::from_millis(100),
        ),
    );
    driver.tick.send(1).unwrap();
    let previous = event(&mut driver).await;
    assert!(previous.error.is_some());
    let replacement = dirty();
    let session = replacement.runtime_session();
    *driver.runtime.lock().await = Some(replacement);
    driver.tick.send(2).unwrap();
    std::fs::remove_file(blocker).unwrap();
    let current = tokio::time::timeout(Duration::from_millis(300), driver.events.recv())
        .await
        .expect("replacement checkpoint must not restart the maximum-age clock after retry")
        .unwrap();
    assert!(current.error.is_none());
    assert_ne!(current.document_id, previous.document_id);
    assert_eq!(current.runtime_session, session);
    assert!(current.attempt_id > previous.attempt_id);
    assert!(!Path::new(&previous.path).exists());
    stop(driver).await;
}

#[tokio::test]
async fn clean_replacement_cancels_retry_without_new_mutation() {
    let area = tempfile::tempdir().unwrap();
    std::fs::write(area.path().join("autosave"), b"blocked").unwrap();
    let mut driver = driver(area.path());
    driver.tick.send(1).unwrap();
    assert!(event(&mut driver).await.error.is_some());
    let backend = Arc::new(crate::worker::PendingBackend);
    *driver.runtime.lock().await = Some(DocumentRuntime::new_blank(
        backend.clone(),
        backend.clone(),
        backend,
    ));
    tokio::time::sleep(Duration::from_millis(180)).await;
    assert!(driver.events.try_recv().is_err());
    assert_eq!(*driver.tick.borrow(), 1);
    stop(driver).await;
    assert!(std::fs::read_dir(area.path()).unwrap().count() == 1);
}

#[tokio::test]
async fn continuous_mutation_ticks_do_not_postpone_failed_checkpoint_retry() {
    let area = tempfile::tempdir().unwrap();
    let blocker = area.path().join("autosave");
    std::fs::write(&blocker, b"blocked").unwrap();
    let mut driver = driver(area.path());
    driver.tick.send(1).unwrap();
    assert!(event(&mut driver).await.error.is_some());
    std::fs::remove_file(blocker).unwrap();
    let mut burst = tokio::time::interval(Duration::from_millis(5));
    // This event follows blocking filesystem work; the scheduling deadline is
    // checked separately under virtual time rather than bounding durable IO here.
    let end = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(end) => panic!("continuous edits postponed retry"),
            _ = burst.tick() => { driver.tick.send_modify(|tick| *tick += 1); },
            event = driver.events.recv() => { assert!(event.unwrap().error.is_none()); break; },
        }
    }
    stop(driver).await;
}

#[tokio::test(start_paused = true)]
async fn ready_mutation_ticks_do_not_move_the_retry_deadline() {
    let (tick, mut changed) = watch::channel(0);
    let mut stopped = std::pin::pin!(std::future::pending());
    let deadline = tokio::time::Instant::now() + Duration::from_millis(120);
    let mut waiting = std::pin::pin!(wait_for_attempt(
        &mut changed,
        stopped.as_mut(),
        (Duration::from_millis(20), Duration::from_millis(60)),
        Some(deadline),
    ));
    let initial = std::future::poll_fn(|cx| Poll::Ready(waiting.as_mut().poll(cx))).await;
    assert_eq!(initial, Poll::Pending);
    for elapsed in (5..=120).step_by(5) {
        tick.send_modify(|value| *value += 1);
        tokio::time::advance(Duration::from_millis(5)).await;
        let outcome = std::future::poll_fn(|cx| Poll::Ready(waiting.as_mut().poll(cx))).await;
        let expected = if elapsed < 120 {
            Poll::Pending
        } else {
            Poll::Ready(true)
        };
        assert_eq!(outcome, expected, "retry scheduling at {elapsed} ms");
    }
    assert_eq!(tokio::time::Instant::now(), deadline);
}

#[tokio::test]
async fn persistent_failure_retries_on_delay_without_spinning_and_stops_promptly() {
    let area = tempfile::tempdir().unwrap();
    std::fs::write(area.path().join("autosave"), b"blocked").unwrap();
    let mut driver = driver(area.path());
    driver.tick.send(1).unwrap();
    let first = event(&mut driver).await;
    assert!(first.error.is_some());
    tokio::time::sleep(Duration::from_millis(40)).await;
    assert!(driver.events.try_recv().is_err());
    let second = event(&mut driver).await;
    assert!(second.error.is_some());
    assert!(second.attempt_id > first.attempt_id);
    stop(driver).await;
}
