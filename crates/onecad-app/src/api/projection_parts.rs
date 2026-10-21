//! Projection planning shared by the headless and the lane-safe visit services.
//!
//! Each step is either pure or worker IO with the runtime lock RELEASED; the
//! callers own locking, fencing and the commit.
use super::*;
use onecad_core::sketch::{ProjectedGeometry, ProjectedSource, Sketch};
use std::collections::BTreeMap;

/// How one projection source is addressed: by the persistent id it already
/// carries, or by the snapshot-scoped key that has to be promoted to get one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ProjectionAddress {
    Promoted(ElementId),
    Pick(TopoKey),
}

/// One requested source after de-duplication.
pub(super) type AddressedSource = (BodyId, ProjectionAddress, Option<AnchorIntent>);

/// One source the kernel is asked for: body, `ElementId`, declared kind.
pub(super) type PlannedSource = (BodyId, String, wire::ProjectionSourceKind);

pub(super) fn refusal_dtos(
    refusals: Vec<wire::ProjectionRefusal>,
) -> Vec<crate::dto::ProjectionRefusalDto> {
    refusals
        .into_iter()
        .map(|r| crate::dto::ProjectionRefusalDto {
            body_id: r.body_id,
            element_id: r.element_id,
            topo_key: r.topo_key,
            code: r.code,
            message: r.message,
        })
        .collect()
}

/// Validates a `projectToSketch` request.
///
/// De-duplicates on (body, address), preserving request order: a doubled pick
/// would otherwise promote as a mismatched batch and project the same edge
/// twice into the sketch.
pub(super) fn parse_projection_request(
    mode: &str,
    sources: Vec<ProjectionSourceInput>,
) -> Result<(wire::ProjectionMode, Vec<AddressedSource>), ApiError> {
    let mode = wire::ProjectionMode::parse(mode).ok_or_else(|| {
        ApiError::InvalidCommand(format!(
            "projectToSketch: unknown mode {mode:?} (edges|faceOutline)"
        ))
    })?;
    if sources.is_empty() {
        return Err(ApiError::InvalidCommand(
            "projectToSketch: at least one source is required".into(),
        ));
    }
    let mut addressed: Vec<AddressedSource> = Vec::new();
    for source in sources {
        let body = wire::parse_body_id(&source.body_id).map_err(ApiError::InvalidCommand)?;
        let address = match source.element_id.filter(|id| !id.is_empty()) {
            Some(id) => ProjectionAddress::Promoted(ElementId::new(id)),
            None => {
                if source.topo_key.is_empty() {
                    return Err(ApiError::InvalidCommand(
                        "projectToSketch: every source needs a topoKey or an elementId".into(),
                    ));
                }
                ProjectionAddress::Pick(TopoKey::new(source.topo_key))
            }
        };
        if addressed
            .iter()
            .any(|(b, a, _)| *b == body && *a == address)
        {
            continue;
        }
        addressed.push((body, address, source.anchor));
    }
    Ok((mode, addressed))
}

/// Promotes the sources that carry no `ElementId` yet. Worker IO under the
/// runtime lock, exactly as `promote_selection` itself runs.
///
/// A source that already names its ElementId is bound in the worker's
/// partition already. Skipping its promotion also skips the stale-pick gate,
/// which costs nothing: an ElementId is durable across snapshots (Invariant 1),
/// and the kernel query plus the fenced commit still fence on the head.
pub(super) async fn promote_projection_picks(
    rt: &mut DocumentRuntime,
    snapshot: SnapshotId,
    addressed: &[AddressedSource],
) -> Result<Vec<PromotedElementDto>, ApiError> {
    let picks: Vec<(BodyId, TopoKey, Option<AnchorIntent>)> = addressed
        .iter()
        .filter_map(|(body, address, anchor)| match address {
            ProjectionAddress::Pick(key) => Some((*body, key.clone(), anchor.clone())),
            ProjectionAddress::Promoted(_) => None,
        })
        .collect();
    let mut promoted = Vec::new();
    for (body, group) in group_picks_by_body(&picks) {
        promoted.extend(rt.promote_selection(snapshot, body, group).await?);
    }
    Ok(promoted)
}

/// One pick, resolved to its body (`promote_selection` batches per body).
type BodyPick = (TopoKey, Option<AnchorIntent>);

/// Groups picks by body, preserving first-seen body order.
fn group_picks_by_body(
    picks: &[(BodyId, TopoKey, Option<AnchorIntent>)],
) -> Vec<(BodyId, Vec<BodyPick>)> {
    let mut out: Vec<(BodyId, Vec<BodyPick>)> = Vec::new();
    for (body, key, anchor) in picks {
        match out.iter_mut().find(|(b, _)| b == body) {
            Some((_, group)) => group.push((key.clone(), anchor.clone())),
            None => out.push((*body, vec![(key.clone(), anchor.clone())])),
        }
    }
    out
}

/// The kernel sources to request, plus the Rust-minted `alreadyProjected`
/// refusals for sources this sketch already carries.
pub(super) fn plan_projection_sources(
    mode: wire::ProjectionMode,
    addressed: &[AddressedSource],
    promoted: &[PromotedElementDto],
    already: &BTreeMap<EntityId, ProjectedSource>,
) -> Result<(Vec<PlannedSource>, Vec<wire::ProjectionRefusal>), ApiError> {
    // A source addressed by its ElementId declares the kind the MODE requires:
    // the worker refuses any other value anyway (`ProjectToSketchPlane` checks
    // `sources[].kind` against the mode's required kind), and there is no
    // promotion answer to read one off.
    let mode_kind = source_kind(mode);
    let mut planned = Vec::with_capacity(addressed.len());
    let mut refusals: Vec<wire::ProjectionRefusal> = Vec::new();
    for (body, address, _) in addressed {
        let body_wire = wire::body_id_wire(*body);
        let (element_id, key, kind) = match address {
            ProjectionAddress::Promoted(id) => (id.as_str().to_string(), "", mode_kind),
            ProjectionAddress::Pick(key) => {
                let hit = promoted
                    .iter()
                    .find(|p| p.body_id == body_wire && p.topo_key == key.as_str())
                    .ok_or_else(|| {
                        ApiError::Internal(format!(
                            "projectToSketch: promotion returned no identity for {body_wire} {}",
                            key.as_str()
                        ))
                    })?;
                let kind = match hit.kind.as_str() {
                    "face" => wire::ProjectionSourceKind::Face,
                    _ => wire::ProjectionSourceKind::Edge,
                };
                (hit.element_id.clone(), key.as_str(), kind)
            }
        };
        // A source this sketch has already projected is REFUSED, not projected
        // again: a second coincident outline would give one source two provenance
        // runs, and no later update could tell which rows belong to which
        // (WP-P F15). Per-source, so the rest of the batch still lands.
        if already
            .values()
            .any(|row| row.source_body == *body && row.source_element_id.as_str() == element_id)
        {
            refusals.push(wire::ProjectionRefusal {
                body_id: body_wire,
                element_id: element_id.clone(),
                topo_key: key.to_string(),
                code: sketch_projection::ALREADY_PROJECTED.to_string(),
                message: format!(
                    "{element_id} is already projected into this sketch — update or detach the \
                     existing geometry instead of projecting it a second time"
                ),
            });
            continue;
        }
        planned.push((*body, element_id, kind));
    }
    Ok((planned, refusals))
}

fn source_kind(mode: wire::ProjectionMode) -> wire::ProjectionSourceKind {
    match mode {
        wire::ProjectionMode::FaceOutline => wire::ProjectionSourceKind::Face,
        wire::ProjectionMode::Edges => wire::ProjectionSourceKind::Edge,
    }
}

pub(super) fn wire_projection_sources(
    planned: &[PlannedSource],
) -> Vec<wire::ProjectionSource<'_>> {
    planned
        .iter()
        .map(|(body, element_id, kind)| wire::ProjectionSource {
            body: *body,
            address: wire::FaceAddress::ElementId(element_id.as_str()),
            kind: *kind,
        })
        .collect()
}

/// Translates a kernel answer into ONE sketch edit: fresh locked entities, their
/// `Fixed` pins and their provenance rows. Pure; mints Rust ids only.
pub(super) fn projected_edit(
    geometry: &ProjectedGeometry,
) -> (Vec<SketchEditOp>, Vec<crate::dto::ProjectedEntityDto>, u32) {
    let (entities, constraints, projections) = onecad_core::sketch::projected_sketch_content(
        geometry,
        &mut EntityId::new,
        &mut ConstraintId::new,
    );
    let point_count = geometry.points.len().try_into().unwrap_or(u32::MAX);
    let type_by_id: HashMap<EntityId, &'static str> = entities
        .iter()
        .map(|e| (e.id(), sketch_projection::type_token(e)))
        .collect();
    let mut ops: Vec<SketchEditOp> = Vec::new();
    for entity in entities {
        ops.push(SketchEditOp::AddEntity { entity });
    }
    for constraint in constraints {
        ops.push(SketchEditOp::AddConstraint { constraint });
    }
    let mut dtos = Vec::with_capacity(projections.len());
    for (entity, source) in projections {
        dtos.push(crate::dto::ProjectedEntityDto {
            entity_id: entity.to_string(),
            entity_type: type_by_id.get(&entity).copied().unwrap_or("Line").into(),
            source_body_id: wire::body_id_wire(source.source_body),
            source_element_id: source.source_element_id.as_str().to_string(),
            projected_hash: source.projected_hash.clone(),
        });
        ops.push(SketchEditOp::SetEntityProjection {
            entity,
            source: Some(source),
        });
    }
    (ops, dtos, point_count)
}

/// An update plan: the ops that move geometry in place, and what it reports.
pub(super) struct Reprojection {
    pub ops: Vec<SketchEditOp>,
    pub entities: Vec<crate::dto::ProjectedEntityDto>,
    pub refusals: Vec<wire::ProjectionRefusal>,
}

/// Re-binds a sketch's mapped sources and re-projects them against `snapshot`,
/// then matches by (source element, ordinal). Runs with the runtime lock
/// RELEASED (WP-P F7); it mutates nothing in the runtime.
pub(super) async fn reproject_sketch(
    state: &AppState,
    sketch: &Sketch,
    snapshot: SnapshotId,
    rebind: Option<crate::document_runtime::ProjectionRebind>,
) -> Result<Reprojection, ApiError> {
    // The sources are snapshot-scoped in the WORKER even though the `ElementId`s
    // are Rust-owned: a regen renumbers sub-shapes, so a source promoted two
    // edits ago names nothing until the ladder re-binds it. Only an AutoBind
    // binds; anything else stays absent and comes back as a per-source refusal
    // rather than a guess.
    if let Some((engine, head, pending)) = rebind {
        let (bound, unresolved) = DocumentRuntime::rebind_entries_with(&engine, head, &pending)
            .await
            .unwrap_or((0, pending.len()));
        tracing::debug!("update_projection: rebind bound={bound} unresolved={unresolved}");
    }
    // One call per MODE, since a face source is re-requested as `faceOutline`
    // and an edge source as `edges`.
    let projector = state.face_projection();
    let mut fresh = Vec::new();
    for (mode, sources) in sketch_projection::projection_request_plan(sketch) {
        let planned: Vec<PlannedSource> = sources
            .into_iter()
            .map(|(body, element)| (body, element, source_kind(mode)))
            .collect();
        let result = projector
            .project_to_sketch_plane(
                snapshot,
                sketch.id,
                &sketch.plane,
                mode,
                &wire_projection_sources(&planned),
            )
            .await?;
        fresh.push((result.geometry, result.refusals));
    }
    let plan = sketch_projection::build_projection_update(sketch, &fresh);
    let mut refusals: Vec<wire::ProjectionRefusal> =
        fresh.iter().flat_map(|(_, r)| r.iter().cloned()).collect();
    refusals.extend(plan.refusals.iter().cloned());
    let entities = plan
        .replaced
        .iter()
        .map(|r| crate::dto::ProjectedEntityDto {
            entity_id: r.entity.to_string(),
            entity_type: r.entity_type.to_string(),
            source_body_id: r.source_body_id.clone(),
            source_element_id: r.source_element_id.clone(),
            projected_hash: r.projected_hash.clone(),
        })
        .collect();
    Ok(Reprojection {
        ops: plan.ops,
        entities,
        refusals,
    })
}

/// The projected entities a detach names; absent means every projected entity.
pub(super) fn detach_targets(
    sketch: &Sketch,
    entity_ids: Option<Vec<String>>,
) -> Result<Vec<EntityId>, ApiError> {
    let Some(ids) = entity_ids else {
        return Ok(sketch.projections.keys().copied().collect());
    };
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let entity = EntityId::from_str(&id).map_err(|e| {
            ApiError::InvalidCommand(format!("detachProjection: bad entityId {id:?}: {e}"))
        })?;
        if !sketch.projections.contains_key(&entity) {
            return Err(ApiError::InvalidCommand(format!(
                "detachProjection: entity {entity} is not projected in sketch {}",
                sketch.id
            )));
        }
        // A repeated id is the caller's, not a second projection: counting it
        // twice would make `remaining` underflow.
        if !out.contains(&entity) {
            out.push(entity);
        }
    }
    Ok(out)
}
