//! Backend side of the wizard lane. Neither call edits the document (the save freezes it
//! without adopting a path), so there is no regeneration to wait for; both are fenced to
//! the document and head they were asked about.
use super::{Op, Reply, Request};
use crate::bridge::{Backend, Message};
use crate::library_catalog::guarded;
use onecad_app::{api, library};
use std::sync::mpsc;
use tokio::sync::OwnedSemaphorePermit;

const OTHER_DOCUMENT: &str = "The document changed; the request was dropped";
const STALE_HEAD: &str = "The model changed under the pick; click the body again";

pub(crate) async fn execute(
    backend: &mut Backend,
    request: Request,
    permit: OwnedSemaphorePermit,
    sender: &mpsc::Sender<Message>,
) {
    let sequence = request.sequence;
    let result = guarded(run(backend, request)).await;
    let _ = sender.send(Message::Author {
        sequence,
        result,
        _permit: permit,
    });
    backend.shared.wake();
}

async fn run(backend: &mut Backend, request: Request) -> Result<Reply, String> {
    let context = api::preview_context(&backend.state)
        .await
        .map_err(|e| e.to_string())?;
    if context.document_id != request.scope.document_id
        || context.runtime_session != request.scope.runtime_session
    {
        return Err(OTHER_DOCUMENT.into());
    }
    match request.op {
        Op::Pick {
            body_id,
            topo_key,
            name,
            snapshot_id,
        } => {
            if context.snapshot_id.0 != snapshot_id {
                return Err(STALE_HEAD.into());
            }
            let fence = onecad_app::dto::GeometryReadFenceDto {
                document_id: request.scope.document_id.clone(),
                runtime_session: request.scope.runtime_session.clone(),
                snapshot_id,
            };
            library::attachment_from_pick(&backend.state, body_id, topo_key, name, &fence)
                .await
                .map(|input| Reply::Picked(Box::new(input)))
                .map_err(|e| e.to_string())
        }
        Op::Save {
            body_id,
            spec,
            union_solids,
        } => library::save_as_component(
            backend.host.clone(),
            &backend.state,
            body_id,
            *spec,
            None,
            Some(union_solids),
        )
        .await
        .map(|dto| Reply::Saved(Box::new(dto)))
        .map_err(|e| e.to_string()),
        #[cfg(test)]
        Op::Variable { name, value } => variable(backend, name, value, context.revision.0).await,
    }
}

/// Test setup only: the UI has no variable editor yet, so the gated test adds them here.
#[cfg(test)]
async fn variable(
    backend: &mut Backend,
    name: String,
    value: f64,
    revision: u64,
) -> Result<Reply, String> {
    let before = api::get_projection(&backend.state)
        .await
        .map_err(|e| e.to_string())?;
    backend.events.begin(crate::event_mailbox::Expected::Known {
        session: before.runtime_session.clone(),
        revision: revision + 1,
    });
    let after = api::upsert_variable(&backend.state, backend.host.clone(), name, value)
        .await
        .map_err(|e| e.to_string())?;
    backend
        .events
        .retarget(&after.runtime_session, after.revision);
    crate::services::wait_regen(backend, &after.runtime_session, after.revision).await?;
    backend.events.finish();
    crate::document_state::refresh(backend).await;
    match crate::services::load_ready_display(backend).await {
        Ok(display) => backend.shared.publish_display(display),
        Err(error) => {
            crate::services::invalidate(&backend.shared, format!("Scene refresh failed: {error}"))
        }
    }
    Ok(Reply::Ready)
}
