//! Non-dirtying operand staging (ONECAD-315 S-315c). Unlike `promote_selection`
//! and `prepare_edge_op`, these never persist, dirty, emit `ProjectionUpdated` or
//! tick autosave: the commit that references the returned refs adopts their
//! evidence. Legacy IPC keeps its persist semantics.
use super::{
    edge_op_bodies, edge_op_mode, prepare_offset_face, wire_edge_op_picks, EdgeOpPickInput,
    OffsetFacePickInput,
};
use crate::error::ApiError;
use crate::read::PreviewContext;
use crate::state::AppState;
use crate::worker::wire;
use onecad_core::document::refs::{ElementKind, ElementRef};
use onecad_core::ids::{BodyId, ElementId, TopoKey};
use onecad_core::regen::{EngineError, OpFailureCode};

/// MESH1 labels a pick with a snapshot TopoKey or an already promoted ElementId.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StageAddress {
    TopoKey(String),
    Element(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StagePick {
    pub body: BodyId,
    pub kind: ElementKind,
    pub address: StageAddress,
}

/// An edge-op closure plus refs built in `prepared.edges` order; the binder uses
/// only `refs`, so preview and commit carry identical evidence.
#[derive(Clone, Debug, PartialEq)]
pub struct StagedEdgeOp {
    pub prepared: crate::dto::PrepareEdgeOpDto,
    pub refs: Vec<ElementRef>,
}

/// An OffsetFace closure plus refs staged in `prepared.faces` order; a refused
/// handshake carries no refs, so the binder can name the refusal.
#[derive(Clone, Debug, PartialEq)]
pub struct StagedOffsetFace {
    pub prepared: crate::dto::PrepareOffsetFaceDto,
    pub refs: Vec<ElementRef>,
}

/// Stages picks under one runtime lock. TopoKey picks promote per body in first
/// appearance order; refs return in input order only when every pick succeeded.
pub async fn stage_elements(
    state: &AppState,
    scope: &PreviewContext,
    picks: Vec<StagePick>,
) -> Result<Vec<ElementRef>, ApiError> {
    if picks.is_empty() {
        return Err(ApiError::InvalidCommand(
            "stageElements: at least one pick is required".into(),
        ));
    }
    let mut guard = state.runtime.lock().await;
    let runtime = guard
        .as_mut()
        .ok_or_else(|| ApiError::NoDocument("stageElements".into()))?;
    let ticket = runtime.stage_ticket(scope).map_err(stage_error)?;
    let mut refs: Vec<Option<ElementRef>> = vec![None; picks.len()];
    let mut bodies: Vec<BodyId> = Vec::new();
    for pick in &picks {
        if matches!(pick.address, StageAddress::TopoKey(_)) && !bodies.contains(&pick.body) {
            bodies.push(pick.body);
        }
    }
    for body in bodies {
        let (slots, keys): (Vec<usize>, Vec<TopoKey>) = picks
            .iter()
            .enumerate()
            .filter_map(|(index, pick)| match &pick.address {
                StageAddress::TopoKey(key) if pick.body == body => {
                    Some((index, TopoKey::new(key.clone())))
                }
                _ => None,
            })
            .unzip();
        let staged = runtime
            .stage_selection(&ticket, body, keys)
            .await
            .map_err(stage_error)?;
        for (index, reference) in slots.into_iter().zip(staged) {
            refs[index] = Some(reference);
        }
    }
    for (index, pick) in picks.iter().enumerate() {
        if let StageAddress::Element(id) = &pick.address {
            let reference = runtime
                .stage_existing(&ticket, pick.body, &ElementId::new(id.clone()), pick.kind)
                .await
                .map_err(stage_error)?;
            refs[index] = Some(reference);
        }
    }
    picks
        .iter()
        .zip(refs)
        .map(|(pick, reference)| {
            let reference = reference.ok_or_else(|| refused("a pick was not staged"))?;
            if reference.primary.as_ref().map(|p| p.kind) != Some(pick.kind) {
                return Err(refused("a pick resolved to a different element kind"));
            }
            Ok(reference)
        })
        .collect()
}

/// `prepare_edge_op` without persistence: the worker closure runs outside the
/// lock under a pre-checked scope, then every closure edge stages in order.
pub async fn stage_edge_op(
    state: &AppState,
    scope: &PreviewContext,
    mode: &str,
    picked_edges: Vec<EdgeOpPickInput>,
    chain_tangent_edges: bool,
) -> Result<StagedEdgeOp, ApiError> {
    if picked_edges.is_empty() {
        return Err(ApiError::InvalidCommand(
            "stageEdgeOp: at least one picked edge is required".into(),
        ));
    }
    let mode = edge_op_mode(mode)?;
    let bodies = edge_op_bodies(&picked_edges)?;
    let picks = wire_edge_op_picks(&picked_edges, &bodies);
    let ticket = {
        let guard = state.runtime.lock().await;
        guard
            .as_ref()
            .ok_or_else(|| ApiError::NoDocument("stageEdgeOp".into()))?
            .stage_ticket(scope)
            .map_err(stage_error)?
    };
    let mut prepared = state
        .face_projection()
        .prepare_edge_op(ticket.snapshot(), mode, &picks, chain_tangent_edges)
        .await
        .map_err(ApiError::from)?;
    if prepared.refusal.is_some() {
        return Ok(StagedEdgeOp {
            prepared,
            refs: Vec::new(),
        });
    }
    let body = wire::parse_body_id(&prepared.target_body_id).map_err(ApiError::Internal)?;
    let keys = prepared
        .edges
        .iter()
        .map(|edge| TopoKey::new(edge.topo_key.clone()))
        .collect();
    let refs = {
        let mut guard = state.runtime.lock().await;
        guard
            .as_mut()
            .ok_or_else(|| ApiError::NoDocument("stageEdgeOp".into()))?
            .stage_selection(&ticket, body, keys)
            .await
            .map_err(stage_error)?
    };
    for (edge, reference) in prepared.edges.iter_mut().zip(&refs) {
        let primary = reference
            .primary
            .as_ref()
            .ok_or_else(|| ApiError::Internal("staged edge has no identity".into()))?;
        if primary.kind != ElementKind::Edge {
            return Err(ApiError::Internal(
                "prepared edge staged as a non-edge element".into(),
            ));
        }
        edge.element_id = Some(primary.element.as_str().to_string());
        edge.body_id = Some(wire::body_id_wire(primary.body));
        edge.kind = Some("edge".into());
    }
    Ok(StagedEdgeOp { prepared, refs })
}

/// The `PrepareOffsetFace` closure (a pure read) answered against `scope`, then
/// every closure face staged in order. Nothing persists or dirties.
pub async fn stage_offset_face(
    state: &AppState,
    scope: &PreviewContext,
    picked_faces: Vec<OffsetFacePickInput>,
    chain_tangent_faces: bool,
) -> Result<StagedOffsetFace, ApiError> {
    let prepared = prepare_offset_face(
        state,
        scope.snapshot_id.0,
        picked_faces,
        chain_tangent_faces,
        None,
    )
    .await?;
    if prepared.refusal.is_some() {
        return Ok(StagedOffsetFace {
            prepared,
            refs: Vec::new(),
        });
    }
    if prepared.snapshot_id != scope.snapshot_id.0 {
        return Err(ApiError::StalePreview(format!(
            "stageOffsetFace: closure answered snapshot {}, staged under {}",
            prepared.snapshot_id, scope.snapshot_id.0
        )));
    }
    let body = wire::parse_body_id(&prepared.target_body_id).map_err(ApiError::Internal)?;
    let picks = prepared
        .faces
        .iter()
        .map(|face| StagePick {
            body,
            kind: ElementKind::Face,
            address: StageAddress::TopoKey(face.topo_key.clone()),
        })
        .collect();
    let refs = stage_elements(state, scope, picks).await?;
    Ok(StagedOffsetFace { prepared, refs })
}

fn stage_error(error: EngineError) -> ApiError {
    match error {
        EngineError::OpFailed {
            code: OpFailureCode::StalePreview,
            message,
            ..
        } => ApiError::StalePreview(message),
        other => other.into(),
    }
}

fn refused(reason: &str) -> ApiError {
    ApiError::OpFailed {
        message: format!("{reason}; re-pick"),
        diagnostics: Vec::new(),
    }
}
