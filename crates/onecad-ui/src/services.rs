//! All worker I/O and checked CPU scene construction run on the service runtime.
use crate::bridge::{
    Backend, BackendEvent, Candidate, CandidateDelivery, Command, Message, command_name,
};
pub(crate) use crate::capture::load_display;
use onecad_app::{
    api::{self},
    dto::RegenFinished,
};
use onecad_core::regen::Lod;
use onecad_interaction::Action;
use onecad_scene::{MeshLimits, SceneBody, SceneLimits, SceneSnapshot};
use std::sync::{Arc, mpsc};
use tokio::sync::OwnedSemaphorePermit;

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
        other => other,
    };
    if let Command::Sketch(request) = command {
        crate::sketch_services::execute(backend, *request, permit, sender).await;
        return;
    }
    let name = command_name(&command);
    let tag = match &command {
        Command::Apply { tag, .. } => Some(tag.clone()),
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
                backend.state.retire_all();
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

pub async fn candidate(backend: &Backend, generation: u64, candidate: Candidate) {
    if !backend.shared.is_current(generation) {
        return;
    }
    let mut prepared = None;
    let mut retained_scene = None;
    let lease = if matches!(&candidate, Candidate::Preview { .. }) {
        match backend
            .shared
            .host_budget
            .reserve(crate::budget::PREVIEW_RESERVATION)
        {
            Ok(lease) => Some(lease),
            Err(error) => {
                let Candidate::Preview { tag, .. } = candidate else {
                    return;
                };
                backend.shared.publish_candidate(
                    generation,
                    CandidateDelivery {
                        action: Action::PreviewFinished {
                            tag,
                            result: Err(error),
                        },
                        prepared: None,
                        lease: None,
                        scene: None,
                    },
                );
                return;
            }
        }
    } else {
        None
    };
    let action = match candidate {
        Candidate::Evaluate {
            tag,
            expression,
            site,
        } => {
            let result = match check_tag(&backend.state, &tag).await {
                Ok(_) => api::evaluate_expression(&backend.state, expression, site.into())
                    .await
                    .map_err(|e| e.to_string()),
                Err(error) => Err(error),
            };
            Action::Evaluated { tag, result }
        }
        Candidate::Preview {
            tag,
            operation,
            sketch_id,
            snapshot,
        } => {
            let result = match preview(backend, &tag, *operation, sketch_id, snapshot).await {
                Ok(result) => match preview_scene(&tag, &result) {
                    Ok(scene) => {
                        match backend.shared.preparation_budget().and_then(|budget| {
                            onecad_render::PreparedScene::new(
                                &scene,
                                budget.limit_cpu_prepared_bytes(crate::budget::MAX_PREPARED_BYTES),
                            )
                            .map_err(|error| error.to_string())
                        }) {
                            Ok(packed) => {
                                prepared = Some(packed);
                                retained_scene = Some(Arc::new(scene));
                                Ok(result)
                            }
                            Err(error) => Err(error.to_string()),
                        }
                    }
                    Err(error) => Err(error),
                },
                Err(error) => Err(error),
            };
            Action::PreviewFinished { tag, result }
        }
    };
    backend.shared.publish_candidate(
        generation,
        CandidateDelivery {
            action,
            prepared,
            lease,
            scene: retained_scene,
        },
    );
}

async fn preview(
    backend: &Backend,
    tag: &onecad_interaction::RequestTag,
    operation: onecad_core::document::record::Operation,
    sketch: Option<String>,
    snapshot: onecad_core::ids::SnapshotId,
) -> Result<onecad_app::dto::PreviewResultDto, String> {
    let before = check_tag(&backend.state, tag).await?;
    let result = api::preview_operation(
        &backend.state,
        operation,
        tag.record_id,
        sketch,
        Some(snapshot),
        Lod::Coarse,
    )
    .await
    .map_err(|e| e.to_string())?;
    if api::preview_context(&backend.state)
        .await
        .map_err(|e| e.to_string())?
        != before
        || result.snapshot_id != before.snapshot_id.0
    {
        return Err("preview completion belongs to an obsolete worker head".into());
    }
    Ok(result)
}

fn preview_scene(
    tag: &onecad_interaction::RequestTag,
    result: &onecad_app::dto::PreviewResultDto,
) -> Result<SceneSnapshot, String> {
    if result.bodies.len() > crate::budget::MAX_BODIES {
        return Err("Native preview body count budget exceeded".into());
    }
    let mut bytes = 0;
    for body in &result.bodies {
        crate::budget::charge(
            &mut bytes,
            body.mesh.len(),
            crate::budget::MAX_SOURCE_BYTES,
            "preview mesh",
        )?;
    }
    let bodies = result
        .bodies
        .iter()
        .map(|body| {
            let id = body
                .body_id
                .parse()
                .map_err(|error| format!("preview body id: {error}"))?;
            SceneBody::new(
                id,
                tag.sequence,
                Arc::from(body.mesh.clone()),
                MeshLimits::default(),
            )
            .map_err(|error| error.to_string())
        })
        .collect::<Result<Vec<_>, String>>()?;
    SceneSnapshot::new(
        tag.context.scene,
        bodies,
        SceneLimits {
            max_resident_bytes: crate::budget::MAX_PREVIEW_SCENE_BYTES,
            max_bodies: crate::budget::MAX_BODIES,
            ..Default::default()
        },
    )
    .map_err(|error| error.to_string())
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
