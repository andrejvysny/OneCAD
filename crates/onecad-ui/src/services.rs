//! All worker I/O and checked CPU scene construction run on the service runtime.
use crate::bridge::{Backend, BackendEvent, Command, Message, command_name};
pub(crate) use crate::capture::load_display;
use onecad_app::{
    api::{self},
    dto::RegenFinished,
};
use std::sync::{Arc, mpsc};
use tokio::sync::OwnedSemaphorePermit;
#[path = "candidate_service.rs"]
mod candidate_service;
pub(crate) use candidate_service::{CandidateBackend, candidate};

pub fn invalidate(shared: &crate::bridge::Shared, error: String) {
    shared.invalidate(Some(error));
}

pub(crate) async fn load_ready_display(
    backend: &mut Backend,
) -> Result<crate::bridge::Display, String> {
    let stopping = backend
        .shared
        .stopping
        .load(std::sync::atomic::Ordering::Acquire);
    if stopping {
        return Err("Host shutdown requested before document capture".into());
    }
    let readiness = backend.state.readiness();
    await_capture_readiness(
        readiness.as_ref(),
        &backend.shared.shutdown,
        std::time::Duration::from_secs(8),
    )
    .await?;
    load_display(backend).await
}

async fn await_capture_readiness(
    readiness: &dyn onecad_app::worker::WorkerReadiness,
    shutdown: &tokio::sync::Notify,
    timeout: std::time::Duration,
) -> Result<(), String> {
    tokio::select! {
        biased;
        _ = shutdown.notified() => Err("Host shutdown requested during worker readiness".into()),
        ready = tokio::time::timeout(timeout, readiness.wait_ready(timeout)) => {
            if matches!(ready, Ok(true)) { Ok(()) }
            else { Err(format!("Geometry worker did not become ready within {} s; document remains open", timeout.as_secs_f64())) }
        }
    }
}

pub async fn execute(
    backend: &mut Backend,
    command: Command,
    permit: OwnedSemaphorePermit,
    sender: &mpsc::Sender<Message>,
) {
    let command = match command {
        Command::Items(request) => {
            crate::document_items::execute(backend, request, permit, sender).await;
            return;
        }
        Command::Operands(request) => {
            crate::operand_services::execute(backend, *request, permit).await;
            return;
        }
        #[cfg(test)]
        Command::InspectSketch(request) => {
            crate::gesture_services::testing::inspect(&backend.state, *request).await;
            return;
        }
        #[cfg(test)]
        Command::InspectBody(request) => {
            crate::body_inspection_tests_support::execute(&backend.state, *request).await;
            return;
        }
        Command::Document(request) => {
            crate::document_services::execute(backend, request, permit, sender).await;
            return;
        }
        Command::DescribeChanges => {
            let changes = api::unsaved_changes(&backend.state)
                .await
                .unwrap_or_default();
            let runtime_session = api::get_document_state(&backend.state)
                .await
                .map(|state| state.runtime_session)
                .unwrap_or_default();
            let _ = sender.send(Message::Changes {
                runtime_session,
                changes,
                _permit: permit,
            });
            backend.shared.wake();
            return;
        }
        Command::Prepare(source) => {
            let identity = source.scene.identity();
            let result = prepare(backend, source).await;
            if result.is_err() {
                // The retained capture cannot be drawn: publish a fresh one instead.
                match load_ready_display(backend).await {
                    Ok(display) => backend.shared.publish_display(display),
                    Err(error) => {
                        invalidate(&backend.shared, format!("Scene refresh failed: {error}"))
                    }
                }
            }
            let _ = sender.send(Message::Prepared {
                identity,
                result,
                _permit: permit,
            });
            backend.shared.wake();
            return;
        }
        other => other,
    };
    if let Command::Sketch(request) = command {
        crate::sketch_services::execute(backend, *request, permit, sender).await;
        return;
    }
    let name = command_name(&command);
    let tag = match &command {
        Command::Apply { tag, .. } | Command::Repair { tag, .. } => Some(tag.clone()),
        _ => None,
    };
    let result = execute_command(backend, command, sender).await;
    backend.events.finish();
    crate::document_state::refresh(backend).await;
    crate::document_services::refresh_recents(backend);
    if backend.startup_opened {
        match load_ready_display(backend).await {
            Ok(display) => backend.shared.publish_display(display),
            Err(error) => invalidate(&backend.shared, format!("Scene refresh failed: {error}")),
        }
    }
    let _ = sender.send(Message::Terminal {
        command: name,
        tag,
        result,
        _permit: permit,
    });
    if let Ok(mut recovery) = backend.shared.recovery.lock() {
        recovery.busy = false;
    }
    backend.shared.wake();
}

/// CPU preparation of an already-captured scene, under the same budgets as a
/// capture. Runs on the blocking pool: every document host shares the service
/// runtime, so a large preparation must not stall the others' actors.
async fn prepare(
    backend: &mut Backend,
    source: crate::viewport_updates::Source,
) -> Result<onecad_render::PreparedScene, String> {
    let epoch = backend
        .shared
        .display_epoch
        .load(std::sync::atomic::Ordering::Acquire);
    let preparation = Arc::new(
        backend
            .shared
            .host_budget
            .admit_while(
                crate::budget::MAX_PREPARED_BYTES as usize,
                crate::budget::Kind::Preparation,
                || {
                    !backend
                        .shared
                        .stopping
                        .load(std::sync::atomic::Ordering::Acquire)
                        && backend
                            .shared
                            .display_epoch
                            .load(std::sync::atomic::Ordering::Acquire)
                            == epoch
                },
            )
            .await?,
    );
    let retention = onecad_render::PreparationRetention {
        source: Some(source.lease),
        preparation: Some(preparation),
    };
    let scene = source.scene;
    let budget = backend
        .shared
        .preparation_budget()?
        .limit_cpu_prepared_bytes(crate::budget::MAX_PREPARED_BYTES);
    let mut cache = backend.shared.take_preparation_cache();
    let (prepared, cache) = tokio::task::spawn_blocking(move || {
        let prepared = onecad_render::PreparedScene::new_cached_retained(
            &scene, budget, &mut cache, retention,
        )
        .map_err(|e| e.to_string());
        (prepared, cache)
    })
    .await
    .map_err(|error| format!("scene preparation stopped: {error}"))?;
    let cache_bytes = (cache.source_bytes(), cache.cpu_bytes());
    backend.shared.restore_preparation_cache(cache);
    let prepared = prepared?;
    if prepared.cpu_bytes() > crate::budget::MAX_PREPARED_BYTES
        || cache_bytes.0 > crate::budget::MAX_SOURCE_BYTES as u64
        || cache_bytes.1 > crate::budget::MAX_PREPARED_BYTES
    {
        return Err("Native CPU preparation budget exceeded".into());
    }
    Ok(prepared)
}

async fn execute_command(
    backend: &mut Backend,
    command: Command,
    sender: &mpsc::Sender<Message>,
) -> Result<crate::outcome::CommandOutcome, String> {
    if matches!(command, Command::Open) && crate::startup_recovery::discover(backend).await? {
        return Ok(Default::default());
    }
    let state = &backend.state;
    let host = backend.host.clone();
    let before = api::get_projection(state)
        .await
        .map_err(|e| e.to_string())?;
    let expected = if matches!(&command, Command::Open | Command::Recovery { .. }) {
        crate::event_mailbox::Expected::Replacement {
            previous: before.runtime_session.clone(),
        }
    } else {
        crate::event_mailbox::Expected::Known {
            session: before.runtime_session.clone(),
            revision: before.revision + 1,
        }
    };
    backend.events.begin(expected);
    let projection = match command {
        #[cfg(test)]
        Command::InspectSketch(_) => {
            return Err("sketch inspection reached model dispatcher".into());
        }
        #[cfg(test)]
        Command::InspectBody(_) => return Err("inspection reached model dispatcher".into()),
        Command::Document(_) => return Err("file command reached model dispatcher".into()),
        Command::Items(_) => return Err("item command reached model dispatcher".into()),
        Command::Sketch(_) => return Err("sketch command reached model dispatcher".into()),
        Command::Operands(_) => return Err("operand command reached model dispatcher".into()),
        Command::Prepare(_) | Command::DescribeChanges => {
            return Err("display command reached model dispatcher".into());
        }
        Command::Recovery {
            document_id,
            choice,
        } => {
            if !crate::startup_recovery::resolve(backend, &document_id, choice).await? {
                return Ok(Default::default());
            }
            api::get_projection(&backend.state)
                .await
                .map_err(|e| e.to_string())?
        }
        Command::Open => {
            backend.startup_opened = open_requested_startup(state, host, &backend.paths).await?;
            if !backend.startup_opened {
                return Ok(Default::default());
            }
            api::get_projection(state)
                .await
                .map_err(|e| e.to_string())?
        }
        Command::Repair { tag, repair } => {
            return crate::provisional::repair(backend, tag, *repair, sender).await;
        }
        Command::Apply {
            tag,
            command,
            hide_sketch,
        } => {
            return crate::provisional::apply(backend, tag, *command, hide_sketch, sender).await;
        }
        Command::Undo => {
            if before.undo_depth == 0 {
                return Ok(Default::default());
            }
            {
                let outcome = api::undo_with_outcome(state, host)
                    .await
                    .map_err(|e| e.to_string())?;
                if !outcome.regen_expected {
                    return Ok(Default::default());
                }
                outcome.projection
            }
        }
        Command::Redo => {
            if before.redo_depth == 0 {
                return Ok(Default::default());
            }
            {
                let outcome = api::redo_with_outcome(state, host)
                    .await
                    .map_err(|e| e.to_string())?;
                if !outcome.regen_expected {
                    return Ok(Default::default());
                }
                outcome.projection
            }
        }
    };
    if projection.runtime_session == before.runtime_session
        && projection.revision == before.revision
    {
        return Ok(Default::default());
    }
    backend
        .events
        .retarget(&projection.runtime_session, projection.revision);
    wait_regen(backend, &projection.runtime_session, projection.revision)
        .await
        .map(|completion| crate::outcome::CommandOutcome {
            completion: Some(Box::new(completion)),
            rollback: None,
        })
}

pub(crate) async fn open_requested_startup(
    state: &onecad_app::state::AppState,
    host: onecad_app::host::HostContext,
    paths: &crate::qa::QaPaths,
) -> Result<bool, String> {
    if !paths.open_on_start {
        return Ok(false);
    }
    api::open_document(
        state,
        host,
        paths.document.to_string_lossy().into_owned(),
        None,
    )
    .await
    .map_err(|error| error.to_string())?;
    Ok(true)
}

pub(crate) async fn wait_regen(
    backend: &mut Backend,
    session: &str,
    revision: u64,
) -> Result<RegenFinished, String> {
    loop {
        tokio::select! {
            _=backend.shared.shutdown.notified()=>{
                backend.state.retire_host_workers();
                return Err("Host shutdown requested; accepted authoring history was preserved".into());
            }
            event=backend.events.next()=>match event {
                BackendEvent::WorkerFailed if current_worker_failed(&backend.state)=>return Err("Geometry worker failed; accepted history was preserved".into()),
                BackendEvent::Regen(result) if result.runtime_session==session && result.source_revision>=revision && result.outcome!="superseded"=>return Ok(result),
                _=>{},
            }
        }
    }
}

pub(crate) async fn check_tag(
    state: &onecad_app::state::AppState,
    tag: &onecad_interaction::RequestTag,
) -> Result<onecad_app::api::PreviewContext, String> {
    let context = api::preview_context(state)
        .await
        .map_err(|e| e.to_string())?;
    if context.document_id != tag.context.document_id
        || context.runtime_session != tag.context.runtime_session
        || context.revision.0 != tag.context.revision
        || context.snapshot_id != tag.context.snapshot_id
    {
        return Err("request belongs to an obsolete document/session/revision/head".into());
    }
    Ok(context)
}

pub(crate) fn current_worker_failed(state: &onecad_app::state::AppState) -> bool {
    worker_state_failed(state.live_worker().map(|worker| worker.state()))
}
fn worker_state_failed(state: Option<onecad_app::worker::WorkerState>) -> bool {
    use onecad_app::worker::WorkerState;
    matches!(
        state,
        None | Some(WorkerState::Failed | WorkerState::Retired)
    )
}
#[cfg(test)]
mod lifecycle_tests {
    struct Waiting;
    #[async_trait::async_trait]
    impl onecad_app::worker::WorkerReadiness for Waiting {
        async fn wait_ready(&self, _: std::time::Duration) -> bool {
            std::future::pending().await
        }
    }

    #[tokio::test]
    async fn capture_readiness_timeout_is_strict_and_shutdown_cancels_wait() {
        let shutdown = tokio::sync::Notify::new();
        let error = super::await_capture_readiness(
            &Waiting,
            &shutdown,
            std::time::Duration::from_millis(10),
        )
        .await
        .unwrap_err();
        assert!(error.contains("did not become ready"));
        shutdown.notify_one();
        let error = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            super::await_capture_readiness(&Waiting, &shutdown, std::time::Duration::from_secs(8)),
        )
        .await
        .unwrap()
        .unwrap_err();
        assert_eq!(error, "Host shutdown requested during worker readiness");
    }

    use super::*;
    use onecad_app::worker::WorkerState;
    #[test]
    fn stale_failure_hint_does_not_fail_healthy_replacement_but_absent_worker_fails() {
        for state in [
            WorkerState::Ready,
            WorkerState::Starting,
            WorkerState::Restarting,
        ] {
            assert!(!worker_state_failed(Some(state)));
        }
        for state in [None, Some(WorkerState::Failed), Some(WorkerState::Retired)] {
            assert!(worker_state_failed(state));
        }
    }
}
