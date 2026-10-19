//! Read-only BRep evidence on the existing bounded host actor queue.
use onecad_app::{
    api::{self, PreviewContext},
    dto::{BodyTopologyDto, GeometryReadFenceDto, MassPropertiesDto},
    state::AppState,
};
use onecad_core::ids::BodyId;
use tokio::sync::oneshot;

#[derive(Debug)]
pub(crate) struct Request {
    pub expected: PreviewContext,
    pub body_id: BodyId,
    pub reply: oneshot::Sender<Result<Reading, String>>,
}

#[derive(Debug)]
pub(crate) struct Reading {
    pub topology: BodyTopologyDto,
    pub mass: MassPropertiesDto,
}

pub(crate) async fn execute(state: &AppState, request: Request) {
    let result = inspect(state, &request.expected, request.body_id).await;
    let _ = request.reply.send(result);
}

async fn inspect(
    state: &AppState,
    expected: &PreviewContext,
    body: BodyId,
) -> Result<Reading, String> {
    check(state, expected).await?;
    let body_id = onecad_app::worker::wire::body_id_wire(body);
    let topology = api::query_body_topology(state, body_id.clone())
        .await
        .map_err(|error| error.to_string())?;
    let mass = api::query_mass_properties(
        state,
        body_id,
        Some(GeometryReadFenceDto {
            document_id: expected.document_id.clone(),
            runtime_session: expected.runtime_session.clone(),
            snapshot_id: expected.snapshot_id.0,
        }),
    )
    .await
    .map_err(|error| error.to_string())?;
    check(state, expected).await?;
    Ok(Reading { topology, mass })
}

async fn check(state: &AppState, expected: &PreviewContext) -> Result<(), String> {
    let current = api::preview_context(state)
        .await
        .map_err(|error| error.to_string())?;
    if &current != expected {
        return Err("BRep inspection belongs to a stale document or worker head".into());
    }
    Ok(())
}
