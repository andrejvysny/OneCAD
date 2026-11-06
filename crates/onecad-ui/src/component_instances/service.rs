//! Backend side of the instance lane. Reads call the instance services directly; a
//! mutation follows the committed-edit tail every other edit uses: fence the regen
//! mailbox, apply, wait for the regen, then refresh document state and the display.
use super::{InstanceView, Mutated, Op, Reply, Request};
use crate::bridge::{Backend, Message};
use crate::library_catalog::guarded;
use onecad_app::{api, dto::LibraryComponentDto, host::HostContext, library, library_instances};
use std::{collections::BTreeMap, sync::mpsc, time::Duration};
use tokio::sync::OwnedSemaphorePermit;

/// A mutation whose regeneration never reports must not hold the actor forever.
const REGEN_TIMEOUT: Duration = Duration::from_secs(120);
const OTHER_DOCUMENT: &str = "The document changed; the request was dropped";

pub(crate) async fn execute(
    backend: &mut Backend,
    request: Request,
    permit: OwnedSemaphorePermit,
    sender: &mpsc::Sender<Message>,
) {
    let sequence = request.sequence;
    let result = guarded(run(backend, request)).await;
    let _ = sender.send(Message::Instance {
        sequence,
        result,
        _permit: permit,
    });
    backend.shared.wake();
}

async fn run(backend: &mut Backend, request: Request) -> Result<Reply, String> {
    let before = api::get_projection(&backend.state)
        .await
        .map_err(|e| e.to_string())?;
    if before.document_id != request.scope.document_id
        || before.runtime_session != request.scope.runtime_session
    {
        return Err(OTHER_DOCUMENT.into());
    }
    let host = backend.host.clone();
    match request.op {
        Op::List => list(&host, &backend.state).await,
        Op::Plan { record_ids } => {
            backend.state.await_library_initialization().await?;
            library_instances::plan_component_upgrades(host, &backend.state, record_ids, None)
                .await
                .map(Reply::Planned)
                .map_err(|e| e.to_string())
        }
        op => mutate(backend, op, before.revision, before.runtime_session).await,
    }
}

async fn list(host: &HostContext, state: &onecad_app::state::AppState) -> Result<Reply, String> {
    let dtos = library_instances::component_instances(host.clone(), state)
        .await
        .map_err(|e| e.to_string())?;
    let mut records: BTreeMap<(String, String), Option<LibraryComponentDto>> = BTreeMap::new();
    let mut items = Vec::with_capacity(dtos.len());
    for dto in dtos {
        let component = if dto.linked {
            let key = (dto.component_id.clone(), dto.component_version.clone());
            if !records.contains_key(&key) {
                let found = library::component_detail(host.clone(), key.0.clone(), key.1.clone())
                    .await
                    .ok()
                    .map(|detail| detail.component);
                records.insert(key.clone(), found);
            }
            records.get(&key).cloned().flatten()
        } else {
            None
        };
        items.push(InstanceView { dto, component });
    }
    Ok(Reply::Listed(items))
}

fn label(op: &Op) -> &'static str {
    match op {
        Op::SetParams { .. } => "Edit component parameters",
        Op::Reset { .. } => "Reset component parameters",
        Op::Detach { .. } => "Detach component",
        _ => "Update components",
    }
}

async fn mutate(
    backend: &mut Backend,
    op: Op,
    revision: u64,
    session: String,
) -> Result<Reply, String> {
    let what = label(&op);
    backend.events.begin(crate::event_mailbox::Expected::Known {
        session,
        revision: revision + 1,
    });
    let applied = apply(&backend.host, &backend.state, op).await;
    let done = match applied {
        Ok(after) if after.revision == revision => Ok(Mutated {
            what,
            noop: true,
            failure: None,
        }),
        Ok(after) => settle(backend, what, &after.runtime_session, after.revision).await,
        Err(error) => Err(error),
    };
    // The same tail as every committed edit: document state (undo depth, dirty) and the
    // scene follow, whether or not the regeneration succeeded.
    backend.events.finish();
    crate::document_state::refresh(backend).await;
    match crate::services::load_ready_display(backend).await {
        Ok(display) => backend.shared.publish_display(display),
        Err(error) => {
            crate::services::invalidate(&backend.shared, format!("Scene refresh failed: {error}"))
        }
    }
    done.map(|done| Reply::Mutated(Box::new(done)))
}

async fn apply(
    host: &HostContext,
    state: &onecad_app::state::AppState,
    op: Op,
) -> Result<onecad_app::dto::DocumentProjection, String> {
    let result = match op {
        Op::SetParams { record_id, params } => {
            library::set_component_params(host.clone(), state, record_id, params).await
        }
        Op::Reset { record_id, keys } => {
            library_instances::reset_component_overrides(host.clone(), state, record_id, keys).await
        }
        Op::Detach { record_id } => library::detach_component(host.clone(), state, record_id).await,
        Op::Upgrade { approved } => {
            state.await_library_initialization().await?;
            library_instances::apply_component_upgrades(host.clone(), state, approved).await
        }
        Op::List | Op::Plan { .. } => return Err("not a mutation".into()),
    };
    result.map_err(|e| e.to_string())
}

/// Waits for the edit's regeneration; a failed one is reported, not hidden.
async fn settle(
    backend: &mut Backend,
    what: &'static str,
    session: &str,
    revision: u64,
) -> Result<Mutated, String> {
    backend.events.retarget(session, revision);
    let waited = tokio::time::timeout(
        REGEN_TIMEOUT,
        crate::services::wait_regen(backend, session, revision),
    )
    .await;
    let failure = match waited {
        Err(_) => Some("regeneration did not report in time".to_owned()),
        Ok(Err(error)) => Some(error),
        Ok(Ok(result)) => regen_failure(&result),
    };
    Ok(Mutated {
        what,
        noop: false,
        failure,
    })
}

fn regen_failure(result: &onecad_app::dto::RegenFinished) -> Option<String> {
    let clean = matches!(result.outcome.as_str(), "published" | "noop")
        && result.failed_steps.is_empty()
        && result.repair_steps.is_empty();
    (!clean).then(|| {
        result
            .message
            .clone()
            .unwrap_or_else(|| format!("regeneration {}", result.outcome))
    })
}
