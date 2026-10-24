use std::{sync::Arc, time::Duration};

use onecad_app::{autosave, document_runtime::DocumentRuntime, worker::PendingBackend};
use onecad_core::{
    document::variables::{Scalar, Variable},
    edit::EditCommand,
    ids::VariableId,
};
use tokio::sync::{mpsc, oneshot, watch, Mutex};

fn dirty_runtime() -> DocumentRuntime {
    let backend = Arc::new(PendingBackend);
    let mut runtime = DocumentRuntime::new_blank(backend.clone(), backend.clone(), backend);
    runtime
        .apply(EditCommand::AddVariable {
            variable: Variable {
                id: VariableId::new(),
                name: "Depth".into(),
                value: Scalar::new(9.25),
                unit: Default::default(),
            },
        })
        .unwrap();
    runtime
}

#[tokio::test]
async fn transient_failure_retries_dirty_checkpoint_without_another_mutation() {
    let area = tempfile::tempdir().unwrap();
    let blocker = area.path().join("autosave");
    std::fs::write(&blocker, b"blocked directory").unwrap();
    let runtime = Arc::new(Mutex::new(Some(dirty_runtime())));
    let identity = {
        let runtime = runtime.lock().await;
        let runtime = runtime.as_ref().unwrap();
        (
            runtime.document_uuid().to_string(),
            runtime.runtime_session(),
            runtime.revision().0,
        )
    };
    let lane = Arc::new(Mutex::new(()));
    let (tick, changed) = watch::channel(0);
    let (stop, stopped) = oneshot::channel();
    let (emitted, mut events) = mpsc::channel(8);
    let driver = tokio::spawn(autosave::run_until(
        runtime.clone(),
        area.path().into(),
        lane,
        changed,
        (Duration::from_millis(10), Duration::from_millis(30)),
        async {
            let _ = stopped.await;
        },
        move |event| {
            emitted.try_send(event).unwrap();
        },
    ));
    tick.send(1).unwrap();
    let failure = tokio::time::timeout(Duration::from_secs(1), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(failure.error.is_some());
    assert_eq!(
        (
            &failure.document_id,
            &failure.runtime_session,
            failure.document_revision
        ),
        (&identity.0, &identity.1, identity.2)
    );
    std::fs::remove_file(&blocker).unwrap();
    let recovered = tokio::time::timeout(Duration::from_secs(7), events.recv())
        .await
        .expect("a failed dirty checkpoint must retry without requiring another edit")
        .unwrap();
    assert!(recovered.error.is_none());
    assert_eq!(
        (
            &recovered.document_id,
            &recovered.runtime_session,
            recovered.document_revision
        ),
        (&identity.0, &identity.1, identity.2)
    );
    assert!(recovered.attempt_id > failure.attempt_id);
    let wire = serde_json::to_value(&recovered).unwrap();
    assert_eq!(wire["documentId"], identity.0);
    assert_eq!(wire["runtimeSession"], identity.1);
    assert_eq!(wire["documentRevision"], identity.2);
    assert_eq!(wire["attemptId"], recovered.attempt_id);
    assert!(std::path::Path::new(&recovered.path).is_file());
    assert!(runtime.lock().await.as_ref().unwrap().is_dirty());
    let _ = stop.send(());
    driver.await.unwrap();
}

#[test]
fn autosave_scheduling_ceiling_is_at_most_sixty_seconds() {
    assert!(autosave::AUTOSAVE_MAX_AGE <= Duration::from_secs(60));
}
