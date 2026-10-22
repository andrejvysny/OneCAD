use super::*;
use std::time::Duration;

fn paths(root: &std::path::Path) -> QaPaths {
    QaPaths::from_args(["--qa-root".into(), root.join("qa").into_os_string()].into_iter()).unwrap()
}

fn wait(service: &mut Service) -> Result<Reply, String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(result) = service.poll() {
            return result;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "refresh did not reply"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn refresh_admission_stays_bounded_until_the_result_is_consumed() {
    let root = tempfile::tempdir().unwrap();
    let runtime = ServiceRuntime::start().unwrap();
    let mut service = Service::new(paths(root.path()));
    let context = egui::Context::default();
    let (release, gate) = oneshot::channel();
    let (finished, completed) = std::sync::mpsc::channel();
    assert!(service.spawn(&runtime, context.clone(), async move {
        gate.await.unwrap();
        finished.send(()).unwrap();
        Err("catalog unavailable".into())
    }));
    assert!(service.poll().is_none());
    assert!(!service.submit(&runtime, context.clone()));
    release.send(()).unwrap();
    completed.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(!service.submit(&runtime, context.clone()));
    assert_eq!(wait(&mut service).unwrap_err(), "catalog unavailable");
    assert!(service.spawn(&runtime, context, async { Err("retry accepted".into()) }));
    assert_eq!(wait(&mut service).unwrap_err(), "retry accepted");
}

#[test]
fn stopped_refresh_task_reports_failure_and_allows_retry() {
    let root = tempfile::tempdir().unwrap();
    let runtime = ServiceRuntime::start().unwrap();
    let mut service = Service::new(paths(root.path()));
    assert!(service.spawn(&runtime, egui::Context::default(), async {
        panic!("test refresh task stopped")
    }));
    assert_eq!(
        wait(&mut service).unwrap_err(),
        "Extension refresh service stopped before replying"
    );
    assert!(service.spawn(&runtime, egui::Context::default(), async {
        Err("retry accepted".into())
    }));
    assert_eq!(wait(&mut service).unwrap_err(), "retry accepted");
}

#[test]
fn catalog_filesystem_failure_clears_pending_and_disables_commands() {
    let root = tempfile::tempdir().unwrap();
    let paths = paths(root.path());
    std::fs::write(paths.data.join("extensions"), "not a directory").unwrap();
    let runtime = ServiceRuntime::start().unwrap();
    let mut service = Service::new(paths);
    let mut state = super::super::State::default();
    state
        .commands
        .push(onecad_extension_host::RegisteredCommand {
            extension_id: "test.addon".into(),
            command: serde_json::from_value(serde_json::json!({
                "id": "test.addon.run", "title": "Run"
            }))
            .unwrap(),
            enabled: true,
        });
    state.begin(super::super::Request::Refresh);
    assert!(service.submit(&runtime, egui::Context::default()));
    state.failed(wait(&mut service).unwrap_err());
    assert!(state.attempted);
    assert!(!state.pending);
    assert!(state.pending_request.is_none());
    assert!(state.status_warning);
    assert!(!state.status.is_empty());
    assert!(!state.commands[0].enabled);
}

#[test]
fn refresh_actor_finishes_during_runtime_shutdown() {
    let root = tempfile::tempdir().unwrap();
    let runtime = ServiceRuntime::start().unwrap();
    let exit = runtime.take_exit().unwrap();
    let mut service = Service::new(paths(root.path()));
    let (release, gate) = oneshot::channel();
    assert!(
        service.spawn(&runtime, egui::Context::default(), async move {
            gate.await.unwrap();
            Err("finished during shutdown".into())
        })
    );
    drop(runtime);
    release.send(()).unwrap();
    assert_eq!(wait(&mut service).unwrap_err(), "finished during shutdown");
    assert!(matches!(
        exit.recv_timeout(Duration::from_secs(5)),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
    ));
}
