use super::{Action, Reply, Request};
use crate::bridge::{Backend, Message};
use onecad_app::{api, joint_execution};
use std::sync::mpsc;
use tokio::sync::OwnedSemaphorePermit;
mod bound;

pub(crate) async fn execute(
    backend: &mut Backend,
    request: Request,
    permit: OwnedSemaphorePermit,
    sender: &mpsc::Sender<Message>,
) {
    let sequence = request.sequence;
    let session = request.session.clone();
    let result = crate::library_catalog::guarded(run(backend, request)).await;
    let _ = sender.send(Message::Assembly {
        sequence,
        session,
        result,
        _permit: permit,
    });
    backend.shared.wake();
}

pub(crate) async fn run(backend: &mut Backend, request: Request) -> Result<Reply, String> {
    let session = request.session.clone();
    let initial_generation = request.generation;
    let outcome = execute_action(backend, request).await;
    crate::document_state::refresh(backend).await;
    let (report, changed) = match outcome {
        Ok(result) => result,
        Err(error) => {
            let changed_during_prepare = api::get_assembly_projection(&backend.state, &session)
                .await
                .is_ok_and(|projection| projection.persisted_generation != initial_generation);
            if changed_during_prepare {
                match crate::services::load_ready_display(backend).await {
                    Ok(display) => backend.shared.publish_display(display),
                    Err(capture_error) => crate::services::invalidate(
                        &backend.shared,
                        format!(
                            "Assembly preparation changed the document; display refresh failed: {capture_error}"
                        ),
                    ),
                }
            }
            return Err(error);
        }
    };
    let capture_error = match capture_after_edit(backend).await {
        Ok(display) => {
            backend.shared.publish_display(display);
            None
        }
        Err(error) => {
            let message = if changed {
                format!(
                    "Assembly change accepted, but scene refresh failed: {error}. Undo is available."
                )
            } else {
                format!("Requested edit made no history change; scene refresh failed: {error}.")
            };
            crate::services::invalidate(&backend.shared, message);
            Some(error)
        }
    };
    Ok(Reply {
        report,
        session,
        capture_error,
        changed,
    })
}

async fn execute_action(
    backend: &Backend,
    request: Request,
) -> Result<(Option<joint_execution::JointSolveReport>, bool), String> {
    let session = request.session.clone();
    match request.action {
        Action::Edit(edit) => {
            let generation = prepare_target(backend, &session, request.generation, &edit).await?;
            let projection = api::apply_assembly_edit_for_display(
                &backend.state,
                &session,
                generation,
                edit,
                crate::budget::MAX_BODIES,
            )
            .await
            .map_err(|error| error.to_string())?;
            Ok((None, projection.persisted_generation != generation))
        }
        Action::Solve { joints, drives } => {
            let report = joint_execution::solve_joints_at_generation_cancellable(
                &backend.state,
                &session,
                request.generation,
                joints,
                drives,
                request
                    .cancellation
                    .ok_or("missing joint cancellation token")?,
            )
            .await
            .map_err(|error| error.to_string())?;
            let changed = api::get_assembly_projection(&backend.state, &session)
                .await
                .map_or(true, |projection| {
                    projection.persisted_generation != request.generation
                });
            Ok((Some(report), changed))
        }
        Action::SolveBound {
            joints,
            drives,
            joint,
            picks,
            unbound,
        } => {
            Box::pin(bound::solve(
                backend,
                bound::Request {
                    session,
                    generation: request.generation,
                    joints,
                    drives,
                    joint,
                    picks,
                    unbound,
                    cancellation: request.cancellation,
                },
            ))
            .await
        }
    }
}

async fn prepare_target(
    backend: &Backend,
    session: &str,
    generation: u64,
    edit: &onecad_core::edit::AssemblyEdit,
) -> Result<u64, String> {
    use onecad_core::edit::AssemblyEdit;
    let projection = api::get_assembly_projection(&backend.state, session)
        .await
        .map_err(|error| error.to_string())?;
    if projection.persisted_generation != generation {
        return Err("Assembly display is stale; retry after refresh".into());
    }
    let definition = match edit {
        AssemblyEdit::InsertOccurrence { occurrence } => Some(occurrence.def.to_string()),
        AssemblyEdit::SetVisibility {
            occurrence,
            visible: true,
        } => projection
            .occurrences
            .iter()
            .find(|item| item.id == occurrence.to_string())
            .map(|item| item.definition.clone()),
        _ => None,
    };
    let Some(definition) = definition else {
        return Ok(generation);
    };
    let entry = projection
        .definitions
        .iter()
        .find(|entry| entry.id == definition)
        .ok_or("Occurrence definition is unavailable")?;
    if entry.mesh_generation.is_some() {
        return Ok(generation);
    }
    let id = onecad_assembly::occurrence::ComponentDefId(
        uuid::Uuid::parse_str(&definition).map_err(|error| error.to_string())?,
    );
    let (_, updated) =
        api::regenerate_component_at_generation(&backend.state, session, id, generation)
            .await
            .map_err(|error| error.to_string())?;
    Ok(updated.persisted_generation)
}

async fn capture_after_edit(backend: &mut Backend) -> Result<crate::bridge::Display, String> {
    #[cfg(test)]
    if backend
        .shared
        .fail_next_assembly_capture
        .swap(false, std::sync::atomic::Ordering::AcqRel)
    {
        return Err("injected scene preparation failure".into());
    }
    crate::services::load_ready_display(backend).await
}
