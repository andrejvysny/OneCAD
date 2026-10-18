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

pub async fn execute(
    backend: &mut Backend,
    command: Command,
    permit: OwnedSemaphorePermit,
    sender: &mpsc::Sender<Message>,
) {
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
    if name != "Quit" {
        match load_display(backend).await {
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
    backend.shared.wake();
}

async fn execute_command(
    backend: &mut Backend,
    command: Command,
    sender: &mpsc::Sender<Message>,
) -> Result<crate::outcome::CommandOutcome, String> {
    let state = &backend.state;
    let host = backend.host.clone();
    let before = api::get_projection(state)
        .await
        .map_err(|e| e.to_string())?;
    let expected = if matches!(&command, Command::Open) {
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
        Command::Sketch(_) => return Err("sketch command reached model dispatcher".into()),
        Command::Open => {
            api::open_document(
                state,
                host,
                backend.paths.document.to_string_lossy().into_owned(),
                None,
            )
            .await
            .map_err(|e| e.to_string())?;
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
        Command::Save => {
            let saved = api::save_document(
                state,
                host,
                Some(backend.paths.document.to_string_lossy().into_owned()),
                None,
            )
            .await
            .map_err(|e| e.to_string())?;
            if !saved.clean {
                return Err("Save finished with newer unsaved edits; document stays open".into());
            }
            return Ok(Default::default());
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
                            Ok(scene) => {
                                prepared = Some(scene);
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
