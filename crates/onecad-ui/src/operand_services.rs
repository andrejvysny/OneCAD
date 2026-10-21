//! Non-dirtying operand staging on the Command lane (ONECAD-315 S-315c). The lane
//! is FIFO with `Apply`, so a staging request always resolves before a later
//! commit; the reply is accepted only while the captured head is still current.
use crate::bridge::Backend;
use onecad_app::api::{self, PreviewContext};
use onecad_core::document::refs::ElementRef;
use onecad_interaction::DocumentContext;
use std::sync::mpsc;
use tokio::sync::OwnedSemaphorePermit;

#[derive(Debug)]
#[allow(dead_code, reason = "the operand session slice (S-315c-ui) submits it")]
pub enum Request {
    Elements {
        context: DocumentContext,
        picks: Vec<api::StagePick>,
        sender: mpsc::Sender<Result<Vec<ElementRef>, String>>,
    },
    EdgeOp {
        context: DocumentContext,
        mode: String,
        picks: Vec<api::EdgeOpPickInput>,
        chain_tangent: bool,
        sender: mpsc::Sender<Result<api::StagedEdgeOp, String>>,
    },
}

pub(crate) async fn execute(backend: &Backend, request: Request, permit: OwnedSemaphorePermit) {
    let state = &backend.state;
    match request {
        Request::Elements {
            context,
            picks,
            sender,
        } => {
            let result = match scope(state, &context).await {
                Ok(scope) => {
                    let staged = api::stage_elements(state, &scope, picks).await;
                    current(state, &scope, staged.map_err(|e| e.to_string())).await
                }
                Err(error) => Err(error),
            };
            let _ = sender.send(result);
        }
        Request::EdgeOp {
            context,
            mode,
            picks,
            chain_tangent,
            sender,
        } => {
            let result = match scope(state, &context).await {
                Ok(scope) => {
                    let staged =
                        api::stage_edge_op(state, &scope, &mode, picks, chain_tangent).await;
                    current(state, &scope, staged.map_err(|e| e.to_string())).await
                }
                Err(error) => Err(error),
            };
            let _ = sender.send(result);
        }
    }
    drop(permit);
    backend.shared.wake();
}

/// The head the UI picked against, or a refusal when it already moved.
async fn scope(
    state: &onecad_app::state::AppState,
    context: &DocumentContext,
) -> Result<PreviewContext, String> {
    let scope = api::preview_context(state)
        .await
        .map_err(|error| error.to_string())?;
    if scope.document_id != context.document_id
        || scope.runtime_session != context.runtime_session
        || scope.revision.0 != context.revision
        || scope.snapshot_id != context.snapshot_id
    {
        return Err("operands belong to an obsolete document/session/revision/head".into());
    }
    Ok(scope)
}

async fn current<T>(
    state: &onecad_app::state::AppState,
    scope: &PreviewContext,
    result: Result<T, String>,
) -> Result<T, String> {
    let value = result?;
    match api::preview_context(state).await {
        Ok(now) if now == *scope => Ok(value),
        _ => Err("stale".into()),
    }
}
