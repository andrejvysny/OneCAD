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
    /// `PrepareOffsetFace` is a pure read; its closure then stages like picks.
    OffsetFace {
        context: DocumentContext,
        picks: Vec<api::OffsetFacePickInput>,
        sender: mpsc::Sender<Result<api::StagedOffsetFace, String>>,
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
        Request::OffsetFace {
            context,
            picks,
            sender,
        } => {
            let result = match scope(state, &context).await {
                Ok(scope) => {
                    let staged = api::stage_offset_face(state, &scope, picks, true).await;
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
        return Err(format!(
            "operands belong to an obsolete head: requested {}, current {}",
            describe_request(context),
            describe_scope(&scope)
        ));
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
        Ok(now) => Err(format!(
            "staged operands went stale: staged under {}, current {}",
            describe_scope(scope),
            describe_scope(&now)
        )),
        Err(error) => Err(format!(
            "staged operands went stale: staged under {}, current head unavailable: {error}",
            describe_scope(scope)
        )),
    }
}

fn describe_request(context: &DocumentContext) -> String {
    format!(
        "document {} session {} revision {} snapshot {}",
        context.document_id, context.runtime_session, context.revision, context.snapshot_id.0
    )
}

fn describe_scope(scope: &PreviewContext) -> String {
    format!(
        "document {} session {} revision {} snapshot {} epoch {}",
        scope.document_id,
        scope.runtime_session,
        scope.revision.0,
        scope.snapshot_id.0,
        scope.worker_epoch.0
    )
}
