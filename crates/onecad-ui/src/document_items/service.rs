use super::{Reply, Request};
use crate::bridge::{Backend, Message};
use onecad_app::api;
use std::sync::mpsc;
use tokio::sync::OwnedSemaphorePermit;

pub(crate) async fn execute(
    backend: &mut Backend,
    request: Request,
    permit: OwnedSemaphorePermit,
    sender: &mpsc::Sender<Message>,
) {
    let (sequence, result) = match request {
        Request::Plan {
            sequence,
            target,
            expected,
        } => {
            let result = api::plan_delete_document_item(&backend.state, target, expected)
                .await
                .map(|plan| Reply::Planned(Box::new(plan)))
                .map_err(|error| error.to_string());
            (sequence, result)
        }
        Request::Delete { sequence, plan } => {
            (sequence, delete(backend, sequence, *plan, sender).await)
        }
    };
    let _ = sender.send(Message::Items {
        sequence,
        result,
        _permit: Some(permit),
    });
    backend.shared.wake();
}

async fn delete(
    backend: &mut Backend,
    sequence: u64,
    plan: api::DeletionPlan,
    sender: &mpsc::Sender<Message>,
) -> Result<Reply, String> {
    backend.events.begin(crate::event_mailbox::Expected::Known {
        session: plan.expected.runtime_session.clone(),
        revision: plan.expected.revision.0 + 1,
    });
    let result = api::delete_document_item(&backend.state, backend.host.clone(), plan).await;
    let result = match result {
        Ok(accepted) => complete(backend, sequence, accepted, sender).await,
        Err(error) => Err(error.to_string()),
    };
    backend.events.finish();
    crate::document_state::refresh(backend).await;
    match crate::services::load_ready_display(backend).await {
        Ok(display) => backend.shared.publish_display(display),
        Err(error) => {
            crate::services::invalidate(&backend.shared, format!("Scene refresh failed: {error}"))
        }
    }
    result
}

async fn complete(
    backend: &mut Backend,
    sequence: u64,
    accepted: api::CommandCompletion<onecad_app::dto::DocumentProjection>,
    sender: &mpsc::Sender<Message>,
) -> Result<Reply, String> {
    accepted.result.map_err(|error| error.to_string())?;
    let _ = sender.send(Message::Items {
        sequence,
        result: Ok(Reply::Accepted),
        _permit: None,
    });
    backend.shared.wake();
    let completion = if accepted.regen_expected {
        backend.events.retarget(
            &accepted.projection.runtime_session,
            accepted.projection.revision,
        );
        Some(Box::new(
            crate::services::wait_regen(
                backend,
                &accepted.projection.runtime_session,
                accepted.projection.revision,
            )
            .await?,
        ))
    } else {
        None
    };
    Ok(Reply::Finished(Box::new(crate::outcome::CommandOutcome {
        completion,
        rollback: None,
    })))
}
