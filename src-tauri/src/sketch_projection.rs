//! Pure planning for projected sketch geometry (WP-P; SCHEMA §7.6
//! `ProjectToSketchPlane`).
//!
//! **No worker, no lock, no document mutation.** Everything here turns a
//! [`Sketch`] plus a fresh projection result into a batch of [`SketchEditOp`]s the
//! caller applies as ONE undoable transaction, or into the list of entities whose
//! projected geometry has gone stale. The commands in [`crate::api`] own the IO
//! and the fencing; this module owns the reasoning, so it is unit-testable
//! without a worker.
//!
//! ## Re-association is by `(source element, ordinal)`, never by shape
//!
//! SCHEMA §7.6 makes emission order normative, and
//! [`ProjectedSource::source_ordinal`] freezes each entity's index within its
//! source's run at commit time. Matching on that pair is what lets an update keep
//! the existing `EntityId`s when a box wall merely moved: every user constraint
//! hung off the projected line survives, and no derived `regionId` changes.
//!
//! Matching on GEOMETRY instead would be a guess, and a wrong guess here is the
//! silent-wrong-bind class this migration exists to remove. So a row whose
//! re-projection came back a different entity TYPE, or did not come back at all,
//! is reported as a `topologyChanged` refusal and left stale — the user detaches
//! or re-projects, and nothing is rebound behind their back.
//!
//! ## …but the RUN is verified before any ordinal in it is trusted
//!
//! An ordinal only means the same physical edge while the source's emission run
//! is unchanged. A boundary that gained or lost an edge renumbers everything
//! after the change, so the guard is per SOURCE, not per row: a run whose length
//! moved, whose committed ordinals no longer line up, or that came back entirely
//! stale with its hashes merely permuted, is refused WHOLE and every one of its
//! rows stays stale ([`refused_sources`]). The same rule covers a shared corner
//! two sources stop agreeing about ([`claim_points`]). Deterministic refusal
//! beats a silent wrong bind, everywhere.

use std::collections::BTreeMap;

use onecad_core::document::refs::ElementKind;
use onecad_core::edit::SketchEditOp;
use onecad_core::ids::{BodyId, EntityId};
use onecad_core::math::Vec2;
use onecad_core::sketch::{
    Constraint, ProjectedEntity, ProjectedGeometry, ProjectedSource, Sketch, SketchEntity,
};

use crate::dto::ProjectedSourceDto;
use crate::worker::wire::{body_id_wire, ProjectionMode, ProjectionRefusal, ProjectionSourceKind};

/// Rust's own per-source refusal code, minted when a re-projection no longer
/// matches the committed run. It sits in the same `refusals` list as the worker's
/// six codes because it is the same KIND of answer — "this one source did not come
/// through, here is why" — and never becomes a whole-call error.
pub const TOPOLOGY_CHANGED: &str = "topologyChanged";

/// Rust's own per-source refusal code for a pick the sketch has ALREADY
/// projected. Minting a second coincident outline for the same element would
/// give the sketch two provenance rows for one source, which no update can ever
/// tell apart again.
pub const ALREADY_PROJECTED: &str = "alreadyProjected";

/// A projected entity's committed geometry, keyed for re-association.
type SourceKey = (BodyId, String, u32);

/// One SOURCE, without its ordinal.
///
/// Re-association is positional *within* a source's emission run, so the run has
/// to be verified as a whole before any of its rows is trusted — see
/// [`refused_sources`].
type GroupKey = (BodyId, String);

fn key_of(source: &ProjectedSource) -> SourceKey {
    (
        source.source_body,
        source.source_element_id.as_str().to_string(),
        source.source_ordinal,
    )
}

fn group_of(source: &ProjectedSource) -> GroupKey {
    (
        source.source_body,
        source.source_element_id.as_str().to_string(),
    )
}

/// The `(mode, sources)` calls needed to re-project a sketch's provenance rows.
///
/// An EDGE source is re-requested with `mode:"edges"` and a FACE source with
/// `mode:"faceOutline"`; sending the other one comes back as a `kindMismatch`
/// refusal, which is why the kind is persisted rather than assumed. Sources are
/// de-duplicated (a `faceOutline` face owns several entities) and ordered
/// deterministically by the sketch's own map order.
#[must_use]
pub fn projection_request_plan(sketch: &Sketch) -> Vec<(ProjectionMode, Vec<(BodyId, String)>)> {
    let mut out = Vec::new();
    for (mode, kind) in [
        (ProjectionMode::Edges, ElementKind::Edge),
        (ProjectionMode::FaceOutline, ElementKind::Face),
    ] {
        let mut seen: Vec<(BodyId, String)> = Vec::new();
        for source in sketch.projections.values() {
            if source.source_kind != kind {
                continue;
            }
            let entry = (
                source.source_body,
                source.source_element_id.as_str().to_string(),
            );
            if !seen.contains(&entry) {
                seen.push(entry);
            }
        }
        if !seen.is_empty() {
            out.push((mode, seen));
        }
    }
    out
}

/// The wire `kind` token for an [`ElementKind`] on this verb.
#[must_use]
pub fn source_kind_wire(kind: ElementKind) -> ProjectionSourceKind {
    match kind {
        ElementKind::Face => ProjectionSourceKind::Face,
        _ => ProjectionSourceKind::Edge,
    }
}

/// One fresh projection result, flattened for lookup.
struct Fresh<'a> {
    by_key: BTreeMap<SourceKey, (&'a ProjectedGeometry, usize)>,
    /// How many entities each source emitted in THIS result — the run length the
    /// committed rows are checked against.
    runs: BTreeMap<GroupKey, usize>,
    refused: Vec<(BodyId, String)>,
}

impl<'a> Fresh<'a> {
    fn build(results: &'a [(ProjectedGeometry, Vec<ProjectionRefusal>)]) -> Self {
        let mut by_key = BTreeMap::new();
        let mut runs: BTreeMap<GroupKey, usize> = BTreeMap::new();
        let mut refused = Vec::new();
        for (geometry, refusals) in results {
            for (index, source) in geometry.entity_sources.iter().enumerate() {
                by_key.insert(key_of(source), (geometry, index));
                *runs.entry(group_of(source)).or_insert(0) += 1;
            }
            for refusal in refusals {
                if let Ok(body) = crate::worker::wire::parse_body_id(&refusal.body_id) {
                    refused.push((body, refusal.element_id.clone()));
                }
            }
        }
        Self {
            by_key,
            runs,
            refused,
        }
    }

    fn group_refused(&self, group: &GroupKey) -> bool {
        self.refused
            .iter()
            .any(|(b, e)| *b == group.0 && *e == group.1)
    }

    fn was_refused(&self, source: &ProjectedSource) -> bool {
        self.group_refused(&group_of(source))
    }
}

/// The sources whose committed run no longer lines up with the run just emitted
/// for them, keyed by source and refused WHOLE (WP-P F1/F2).
///
/// Positional matching inside a source is only sound while the RUN is unchanged.
/// A boundary that gained or lost an edge renumbers every ordinal after the
/// change, so ordinal 2 of the new run is a DIFFERENT physical edge from ordinal
/// 2 of the committed one — rewriting the entity's geometry from it is exactly
/// the silent wrong bind this migration exists to remove. The same trap with no
/// count to catch it is a run that came back the same LENGTH but merely rotated,
/// so an entirely-stale run whose fresh hashes are a permutation of the committed
/// ones is refused too.
///
/// Sources ABSENT from the fresh result are deliberately not here: they are
/// already reported per row (a ladder miss, or a worker `absent`).
fn refused_sources(
    rows: &[(EntityId, ProjectedSource)],
    fresh: &Fresh<'_>,
) -> BTreeMap<GroupKey, String> {
    let mut committed: BTreeMap<GroupKey, Vec<&ProjectedSource>> = BTreeMap::new();
    for (_, source) in rows {
        committed.entry(group_of(source)).or_default().push(source);
    }
    let mut out = BTreeMap::new();
    for (group, sources) in committed {
        let Some(&emitted) = fresh.runs.get(&group) else {
            continue;
        };
        let reason = run_mismatch(&group.1, &sources, emitted)
            .or_else(|| run_reordered(&group.1, &sources, fresh));
        if let Some(reason) = reason {
            out.insert(group, reason);
        }
    }
    out
}

/// Why a committed run no longer lines up with the `emitted` one, or `None` when
/// every committed ordinal still names a row of the same-length fresh run.
fn run_mismatch(element: &str, sources: &[&ProjectedSource], emitted: usize) -> Option<String> {
    let committed = sources.len();
    if committed != emitted {
        return Some(format!(
            "{element}: boundary changed: {committed} edges were projected, {emitted} are \
             emitted now — detach it or re-project the source"
        ));
    }
    let mut ordinals: Vec<u32> = sources.iter().map(|s| s.source_ordinal).collect();
    ordinals.sort_unstable();
    ordinals.dedup();
    let in_range = ordinals
        .last()
        .is_some_and(|last| (*last as usize) < emitted);
    if ordinals.len() != committed || !in_range {
        return Some(format!(
            "{element}: boundary changed: the committed run no longer lines up with the \
             {emitted} edges emitted now — detach it or re-project the source"
        ));
    }
    None
}

/// A same-length run in which at least one row moved and whose fresh hashes are a
/// permutation of the committed ones: the same curves, re-ordered (a rotation, or
/// a reversed walk that keeps its start edge and swaps the others).
///
/// Updating it would swap geometry between entity ids — every constraint and
/// region hung off them would silently start describing a different edge — so it
/// is refused like any other run change.
fn run_reordered(element: &str, sources: &[&ProjectedSource], fresh: &Fresh<'_>) -> Option<String> {
    let mut committed: Vec<&str> = Vec::with_capacity(sources.len());
    let mut emitted: Vec<&str> = Vec::with_capacity(sources.len());
    let mut any_changed = false;
    for source in sources {
        let (geometry, index) = fresh.by_key.get(&key_of(source)).copied()?;
        let hash = geometry.entity_sources[index].projected_hash.as_str();
        // A row that still agrees stays in the multiset: a REVERSED wire walk keeps
        // its start edge in place and swaps the rest, and that swap is exactly the
        // silent id/geometry exchange this guard exists to refuse.
        any_changed |= hash != source.projected_hash;
        committed.push(source.projected_hash.as_str());
        emitted.push(hash);
    }
    if !any_changed {
        return None; // nothing moved — nothing to update, nothing to refuse.
    }
    committed.sort_unstable();
    emitted.sort_unstable();
    (committed == emitted).then(|| {
        format!(
            "{element}: boundary re-ordered: the same {} projected curves came back in a \
             different order — detach it or re-project the source",
            sources.len()
        )
    })
}

/// The entities whose re-projected geometry no longer matches the hash committed
/// with them (WP-P B4).
///
/// A source that VANISHED (a ladder miss, or a per-source `absent` refusal) counts
/// as stale too, and deliberately so: the projection is reported, never re-bound
/// to whatever now occupies that ordinal.
///
/// The comparison is on `projectedHash` alone — the projected UV geometry — so a
/// source body that merely slid along the sketch normal is NOT stale. That is
/// correct for a projection: the question is whether the picture in this sketch
/// changed, not whether the model did.
#[must_use]
pub fn stale_projected_entities(
    rows: &[(EntityId, ProjectedSource)],
    results: &[(ProjectedGeometry, Vec<ProjectionRefusal>)],
) -> Vec<EntityId> {
    let fresh = Fresh::build(results);
    let refused = refused_sources(rows, &fresh);
    rows.iter()
        .filter(|(_, source)| {
            // A source whose RUN changed is stale wholesale: not one of its rows
            // can still be trusted to describe the same physical edge, however
            // many of the hashes happen to line up (WP-P F1/F2).
            refused.contains_key(&group_of(source))
                || match fresh.by_key.get(&key_of(source)) {
                    Some((geometry, index)) => {
                        geometry.entity_sources[*index].projected_hash != source.projected_hash
                    }
                    None => true,
                }
        })
        .map(|(entity, _)| *entity)
        .collect()
}

/// One replaced projected entity, for the command's DTO.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplacedEntity {
    /// The entity that was updated in place (its id is UNCHANGED).
    pub entity: EntityId,
    /// The SCHEMA §7.6 type token.
    pub entity_type: &'static str,
    /// The source body, `body_<uuid>` wire form.
    pub source_body_id: String,
    /// The source element id.
    pub source_element_id: String,
    /// The refreshed hash now serving as the staleness baseline.
    pub projected_hash: String,
}

/// The batch an `update_projection` should apply.
#[derive(Debug, Default)]
pub struct ProjectionUpdatePlan {
    /// The ops, in apply order. Empty when nothing moved.
    pub ops: Vec<SketchEditOp>,
    /// The entities whose geometry was replaced.
    pub replaced: Vec<ReplacedEntity>,
    /// Rows that could not be re-associated, as per-source refusals.
    pub refusals: Vec<ProjectionRefusal>,
}

/// Plans the in-place replacement of every projected entity whose geometry moved.
///
/// Returns an EMPTY plan when nothing changed — a re-projection that agrees with
/// the committed hashes must not enter the undo stack.
#[must_use]
pub fn build_projection_update(
    sketch: &Sketch,
    results: &[(ProjectedGeometry, Vec<ProjectionRefusal>)],
) -> ProjectionUpdatePlan {
    let fresh = Fresh::build(results);
    let rows: Vec<(EntityId, ProjectedSource)> = sketch
        .projections
        .iter()
        .map(|(entity, source)| (*entity, source.clone()))
        .collect();
    let mut refused = refused_sources(&rows, &fresh);
    let mut plan = ProjectionUpdatePlan::default();
    let (staged, split) = stage_rows(sketch, &fresh, &refused, &mut plan);
    refused.extend(split);
    for (group, message) in &refused {
        if fresh.group_refused(group) {
            continue; // the worker already said it.
        }
        plan.refusals.push(source_refused(group, message));
    }

    // Target positions per projected POINT, de-duplicated: a corner point is
    // shared by the two lines that meet there, and both would name it.
    let mut moves: BTreeMap<EntityId, Vec2> = BTreeMap::new();
    // Curves whose OWN scalars (radius / angles / radii) changed and therefore
    // cannot be updated by moving a point.
    let mut rebuilt: Vec<SketchEntity> = Vec::new();
    let mut sources: Vec<(EntityId, ProjectedSource)> = Vec::new();
    for row in staged
        .into_iter()
        .filter(|row| !refused.contains_key(&row.group))
    {
        // Only points that ACTUALLY moved: a re-projection that shifts one wall
        // still names both endpoints of every line it touches, and unlocking,
        // re-pinning and re-locking a point at the coordinate it already has is
        // three ops of pure churn in the undo entry.
        moves.extend(row.point_moves.into_iter().filter(|(id, at)| {
            sketch
                .get_entity(*id)
                .is_none_or(|e| !matches!(e, SketchEntity::Point { at: now, .. } if now == at))
        }));
        if let Some(entity) = row.rebuild {
            rebuilt.push(entity);
        }
        plan.replaced.push(ReplacedEntity {
            entity: row.entity,
            entity_type: type_token(row.existing),
            source_body_id: crate::worker::wire::body_id_wire(row.source.source_body),
            source_element_id: row.source.source_element_id.as_str().to_string(),
            projected_hash: row.source.projected_hash.clone(),
        });
        sources.push((row.entity, row.source));
    }

    if moves.is_empty() && rebuilt.is_empty() {
        return plan;
    }
    plan.ops = point_move_ops(sketch, &moves);
    plan.ops.extend(rebuild_ops(sketch, rebuilt));
    plan.ops.extend(sources.into_iter().map(|(entity, source)| {
        SketchEditOp::SetEntityProjection {
            entity,
            source: Some(source),
        }
    }));
    plan
}

/// One row that CAN be updated, staged before the F3 collision check decides
/// whether its whole source survives.
struct Staged<'a> {
    entity: EntityId,
    existing: &'a SketchEntity,
    group: GroupKey,
    point_moves: Vec<PointMove>,
    rebuild: Option<SketchEntity>,
    source: ProjectedSource,
}

/// Stages every row whose source survived [`refused_sources`] and that
/// re-projected to the same KIND of shape, recording each source's claim on the
/// points it moves. Rows that cannot be re-associated push their own per-entity
/// refusal onto `plan`; the returned map holds the F3 shared-corner refusals.
fn stage_rows<'a>(
    sketch: &'a Sketch,
    fresh: &Fresh<'_>,
    refused: &BTreeMap<GroupKey, String>,
    plan: &mut ProjectionUpdatePlan,
) -> (Vec<Staged<'a>>, BTreeMap<GroupKey, String>) {
    let mut staged = Vec::new();
    let mut claims: BTreeMap<EntityId, (Vec2, GroupKey)> = BTreeMap::new();
    let mut split: BTreeMap<GroupKey, String> = BTreeMap::new();
    for (entity, old) in &sketch.projections {
        let group = group_of(old);
        if refused.contains_key(&group) {
            continue; // the source is refused whole; none of its rows is touched.
        }
        let Some(existing) = sketch.get_entity(*entity) else {
            continue; // the map cannot name a missing entity; defence in depth.
        };
        let Some((geometry, index)) = fresh.by_key.get(&key_of(old)).copied() else {
            if !fresh.was_refused(old) {
                plan.refusals.push(topology_changed(*entity, old));
            }
            continue;
        };
        let source = geometry.entity_sources[index].clone();
        if source.projected_hash == old.projected_hash {
            continue;
        }
        let Some((point_moves, rebuild)) =
            replace_in_place(existing, &geometry.entities[index], &geometry.points)
        else {
            plan.refusals.push(topology_changed(*entity, old));
            continue;
        };
        claim_points(&mut claims, &mut split, &group, &point_moves);
        staged.push(Staged {
            entity: *entity,
            existing,
            group,
            point_moves,
            rebuild,
            source,
        });
    }
    (staged, split)
}

/// Records this source's claim on each point it moves, and refuses BOTH sources
/// when two of them pull one shared point to different places (WP-P F3).
///
/// Two projected curves meeting at a corner name ONE point, and the update moves
/// it once. When the sources stop agreeing on where it goes, the corner has come
/// apart in the model: whichever claim happened to win would drag the other
/// curve's endpoint somewhere its own source never put it — a silent wrong bind
/// with no refusal to explain it.
///
/// The claim is recorded BEFORE the no-op filter, so a move that happens to land
/// on the coordinate the point already holds still counts as a claim.
fn claim_points(
    claims: &mut BTreeMap<EntityId, (Vec2, GroupKey)>,
    split: &mut BTreeMap<GroupKey, String>,
    group: &GroupKey,
    moves: &[PointMove],
) {
    for (point, at) in moves {
        let Some((claimed, owner)) = claims.get(point).cloned() else {
            claims.insert(*point, (*at, group.clone()));
            continue;
        };
        if owner == *group || same_point(claimed, *at) {
            continue;
        }
        let message = format!(
            "{} and {}: shared corner split — the two sources no longer project their shared \
             point to the same place; detach one or re-project both",
            owner.1, group.1
        );
        split.insert(owner, message.clone());
        split.insert(group.clone(), message);
    }
}

/// Two claimed positions that agree to within the shared-corner tolerance.
fn same_point(a: Vec2, b: Vec2) -> bool {
    (a.x - b.x).abs() <= 1e-9 && (a.y - b.y).abs() <= 1e-9
}

/// Points: unlock → move → re-pin the `Fixed` at the new UV → re-lock.
fn point_move_ops(sketch: &Sketch, moves: &BTreeMap<EntityId, Vec2>) -> Vec<SketchEditOp> {
    let mut ops = Vec::new();
    let locked_points: Vec<EntityId> = moves
        .keys()
        .copied()
        .filter(|id| {
            sketch
                .get_entity(*id)
                .is_some_and(SketchEntity::is_reference_locked)
        })
        .collect();
    for point in &locked_points {
        ops.push(SketchEditOp::SetEntityReferenceLocked {
            entity: *point,
            locked: false,
        });
    }
    if !moves.is_empty() {
        ops.push(SketchEditOp::SetEntityPositions {
            positions: moves.iter().map(|(id, at)| (*id, *at)).collect(),
        });
    }
    // The pin holds the point at its OWN UV, so a moved point whose pin still
    // names the old coordinate would be dragged straight back by the solver. The
    // constraint id is reused: the batch removes it before re-adding it.
    for constraint in sketch.constraints() {
        let Constraint::Fixed {
            id,
            point,
            point_position,
            ..
        } = constraint
        else {
            continue;
        };
        let Some(at) = moves.get(point) else { continue };
        ops.push(SketchEditOp::RemoveConstraint { constraint: *id });
        ops.push(SketchEditOp::AddConstraint {
            constraint: Constraint::Fixed {
                id: *id,
                point: *point,
                point_position: *point_position,
                at: *at,
            },
        });
    }
    for point in &locked_points {
        ops.push(SketchEditOp::SetEntityReferenceLocked {
            entity: *point,
            locked: true,
        });
    }
    ops
}

/// Curves whose own scalars moved: remove + re-add under the SAME id.
///
/// There is no op that rewrites a curve's radius or angles, and minting a new id
/// would break every constraint and region that names this one. The constraints
/// the cascade drops are re-added verbatim after.
fn rebuild_ops(sketch: &Sketch, rebuilt: Vec<SketchEntity>) -> Vec<SketchEditOp> {
    let mut ops = Vec::new();
    for entity in rebuilt {
        let id = entity.id();
        let dependents = sketch.dependents_of(id);
        ops.push(SketchEditOp::SetEntityReferenceLocked {
            entity: id,
            locked: false,
        });
        ops.push(SketchEditOp::RemoveEntity { entity: id });
        ops.push(SketchEditOp::AddEntity { entity });
        for constraint in dependents.constraints {
            if let Some(c) = sketch.get_constraint(constraint) {
                ops.push(SketchEditOp::AddConstraint {
                    constraint: c.clone(),
                });
            }
        }
    }
    ops
}

/// One whole-source `topologyChanged` refusal (F1/F2/F3): the source is named,
/// none of its rows is touched, and every one of them stays stale.
fn source_refused(group: &GroupKey, message: &str) -> ProjectionRefusal {
    ProjectionRefusal {
        body_id: crate::worker::wire::body_id_wire(group.0),
        element_id: group.1.clone(),
        topo_key: String::new(),
        code: TOPOLOGY_CHANGED.to_string(),
        message: message.to_string(),
    }
}

fn topology_changed(entity: EntityId, source: &ProjectedSource) -> ProjectionRefusal {
    ProjectionRefusal {
        body_id: crate::worker::wire::body_id_wire(source.source_body),
        element_id: source.source_element_id.as_str().to_string(),
        topo_key: String::new(),
        code: TOPOLOGY_CHANGED.to_string(),
        message: format!(
            "the re-projection of {} no longer matches the entity {entity} committed from it — \
             detach it or re-project the source",
            source.source_element_id.as_str()
        ),
    }
}

/// A point move (`entity`, new UV).
type PointMove = (EntityId, Vec2);

/// The point moves (and, when the curve's own scalars changed, the rebuilt curve)
/// that turn `existing` into `projected`. `None` when the shapes are not the same
/// KIND — that is a topology change, not an update.
fn replace_in_place(
    existing: &SketchEntity,
    projected: &ProjectedEntity,
    points: &[Vec2],
) -> Option<(Vec<PointMove>, Option<SketchEntity>)> {
    let at = |i: usize| points.get(i).copied();
    match (existing, projected) {
        (SketchEntity::Line { start, end, .. }, ProjectedEntity::Line { p0, p1 }) => {
            Some((vec![(*start, at(*p0)?), (*end, at(*p1)?)], None))
        }
        (
            SketchEntity::Circle {
                id,
                center,
                radius,
                construction,
                ..
            },
            ProjectedEntity::Circle {
                center: c,
                radius: r,
            },
        ) => {
            let moves = vec![(*center, at(*c)?)];
            let rebuild = (radius != r)
                .then(|| SketchEntity::circle(*id, *center, *r, *construction))
                .flatten()
                .map(|e| e.with_reference_locked(true));
            Some((moves, rebuild))
        }
        (
            SketchEntity::Arc {
                id,
                center,
                radius,
                start_angle,
                end_angle,
                construction,
                ..
            },
            ProjectedEntity::Arc {
                center: c,
                radius: r,
                start_angle: s,
                end_angle: e,
                ..
            },
        ) => {
            let moves = vec![(*center, at(*c)?)];
            let rebuild = (radius != r || start_angle != s || end_angle != e)
                .then(|| SketchEntity::arc(*id, *center, *r, *s, *e, *construction))
                .flatten()
                .map(|e| e.with_reference_locked(true));
            Some((moves, rebuild))
        }
        (
            SketchEntity::Ellipse {
                id,
                center,
                major_r,
                minor_r,
                rotation,
                construction,
                ..
            },
            ProjectedEntity::Ellipse {
                center: c,
                major_r: ma,
                minor_r: mi,
                rotation: rot,
            },
        ) => {
            let moves = vec![(*center, at(*c)?)];
            let rebuild = (major_r != ma || minor_r != mi || rotation != rot)
                .then(|| SketchEntity::ellipse(*id, *center, *ma, *mi, *rot, *construction))
                .flatten()
                .map(|e| e.with_reference_locked(true));
            Some((moves, rebuild))
        }
        _ => None,
    }
}

/// The SCHEMA §7.6 entity-type token for a committed sketch entity.
#[must_use]
pub fn type_token(entity: &SketchEntity) -> &'static str {
    match entity {
        SketchEntity::Point { .. } => "Point",
        SketchEntity::Line { .. } => "Line",
        SketchEntity::Arc { .. } => "Arc",
        SketchEntity::Circle { .. } => "Circle",
        SketchEntity::Ellipse { .. } => "Ellipse",
    }
}

/// Plans a detach: drop each target's provenance row, release the `Fixed` pins
/// that hold it, and unlock it and the points it exclusively owns.
///
/// Returns `(ops, released_pin_count)`.
///
/// A projected POINT is released only when every still-projected curve has let go
/// of it. Two projected lines meeting at a corner share one point, and freeing it
/// while one of them is still reference-locked would leave locked geometry hanging
/// off a movable vertex.
#[must_use]
pub fn build_projection_detach(sketch: &Sketch, targets: &[EntityId]) -> (Vec<SketchEditOp>, u32) {
    let mut ops = Vec::new();

    // Points still held by a projected curve that is NOT being detached.
    let mut held: Vec<EntityId> = Vec::new();
    for entity in sketch.projections.keys() {
        if targets.contains(entity) {
            continue;
        }
        if let Some(e) = sketch.get_entity(*entity) {
            held.extend(e.referenced_entities());
        }
    }

    let mut free_points: Vec<EntityId> = Vec::new();
    for target in targets {
        let Some(entity) = sketch.get_entity(*target) else {
            continue;
        };
        for point in entity.referenced_entities() {
            if held.contains(&point) || free_points.contains(&point) {
                continue;
            }
            if sketch
                .get_entity(point)
                .is_some_and(SketchEntity::is_reference_locked)
            {
                free_points.push(point);
            }
        }
        ops.push(SketchEditOp::SetEntityProjection {
            entity: *target,
            source: None,
        });
        ops.push(SketchEditOp::SetEntityReferenceLocked {
            entity: *target,
            locked: false,
        });
    }

    let mut released = 0u32;
    for constraint in sketch.constraints() {
        let Constraint::Fixed { id, point, .. } = constraint else {
            continue;
        };
        if !free_points.contains(point) {
            continue;
        }
        ops.push(SketchEditOp::RemoveConstraint { constraint: *id });
        released += 1;
    }
    for point in free_points {
        ops.push(SketchEditOp::SetEntityReferenceLocked {
            entity: point,
            locked: false,
        });
    }
    (ops, released)
}

/// Re-checks every sketch whose projected geometry could have moved under a
/// just-published snapshot, and records the verdict (WP-P B4).
///
/// **Runs with the runtime lock RELEASED**, and must never be called from
/// `finish_regen`: it does a worker round-trip per affected sketch, and holding
/// the single writer across worker IO is the anti-pattern R-WP11 removed for
/// regen. The shape is `mint_rollback_checkpoint`'s — capture under the lock,
/// drive unlocked, re-lock and adopt — and
/// [`DocumentRuntime::adopt_projection_staleness`] DROPS the verdict if the head
/// moved while the probe was out, because staleness reported against a snapshot
/// that no longer exists is worse than no report: the next publish re-probes
/// anyway.
///
/// A publish that moved none of a sketch's source bodies costs ZERO round-trips —
/// [`DocumentRuntime::projection_probes`] filters on the source bodies' geometry
/// signatures. Nothing here is on the drag or preview path.
pub async fn refresh_projection_staleness(
    runtime: &std::sync::Arc<tokio::sync::Mutex<Option<crate::document_runtime::DocumentRuntime>>>,
    projector: &std::sync::Arc<dyn crate::worker::FaceBoundaryProjection>,
    cancel: &onecad_core::regen::CancelToken,
) {
    let (probes, document_id) = {
        let guard = runtime.lock().await;
        let Some(rt) = guard.as_ref() else {
            return;
        };
        (rt.projection_probes(), rt.document_uuid())
    };
    for probe in probes {
        // A superseding regen is already queued: its own publish re-probes, so
        // paying for this one's worker round-trips would only delay it (WP-P F6).
        if cancel.is_cancelled() {
            return;
        }
        if !rebind_sources_unlocked(runtime, document_id, probe.sketch).await {
            return;
        }
        let sketch = {
            let guard = runtime.lock().await;
            let Some(rt) = guard
                .as_ref()
                .filter(|rt| rt.document_uuid() == document_id)
            else {
                return;
            };
            rt.sketch_snapshot(probe.sketch, "projectionStaleness").ok()
        };
        // The sketch was deleted while the pass ran. That is this probe's problem
        // alone — the rest still answer against the same head (WP-P F10).
        let Some(sketch) = sketch else { continue };
        let Some(fresh) = probe_projections(projector, &probe, &sketch, cancel).await else {
            if cancel.is_cancelled() {
                return;
            }
            continue;
        };
        let stale = stale_projected_entities(&probe.sources, &fresh);
        let mut guard = runtime.lock().await;
        let Some(rt) = guard
            .as_mut()
            .filter(|rt| rt.document_uuid() == document_id)
        else {
            return;
        };
        if !rt.adopt_projection_staleness(&probe, stale)
            && rt.head_snapshot_id() != Some(probe.snapshot)
        {
            return; // the head moved; every remaining probe answers against it too.
        }
    }
}

/// Re-binds one sketch's projection sources through the §7.5 ladder with the
/// runtime lock RELEASED (WP-P F7). Returns `false` when the document went away.
///
/// A `TopoKey` is a snapshot ordinal, so after the regen that just published, a
/// source promoted earlier names nothing until the ladder says otherwise — and
/// without this every projection would report stale forever after its first edit.
/// Only an AutoBind binds; anything else stays absent and IS the stale verdict.
async fn rebind_sources_unlocked(
    runtime: &std::sync::Arc<tokio::sync::Mutex<Option<crate::document_runtime::DocumentRuntime>>>,
    document_id: onecad_core::ids::DocumentId,
    sketch: onecad_core::ids::SketchId,
) -> bool {
    let ticket = {
        let guard = runtime.lock().await;
        let Some(rt) = guard
            .as_ref()
            .filter(|rt| rt.document_uuid() == document_id)
        else {
            return false;
        };
        rt.projection_rebind_ticket(sketch)
    };
    let Some((engine, snapshot, pending)) = ticket else {
        return true;
    };
    let (bound, unresolved) =
        crate::document_runtime::DocumentRuntime::rebind_entries_with(&engine, snapshot, &pending)
            .await
            .unwrap_or((0, pending.len()));
    tracing::debug!(
        target: "onecad_lib::regen",
        %sketch,
        bound,
        unresolved,
        "projection staleness: sources re-bound"
    );
    true
}

/// One probe's worker round-trips, one per MODE. `None` records NOTHING —
/// leaving the previous verdict standing is honest, and the next publish retries;
/// writing "fresh" on a transport hiccup would clear a real warning.
async fn probe_projections(
    projector: &std::sync::Arc<dyn crate::worker::FaceBoundaryProjection>,
    probe: &crate::document_runtime::ProjectionProbe,
    sketch: &Sketch,
    cancel: &onecad_core::regen::CancelToken,
) -> Option<Vec<(ProjectedGeometry, Vec<ProjectionRefusal>)>> {
    let mut fresh = Vec::new();
    for (mode, sources) in projection_request_plan(sketch) {
        if cancel.is_cancelled() {
            return None;
        }
        let wire_sources: Vec<crate::worker::wire::ProjectionSource<'_>> = sources
            .iter()
            .map(|(body, element)| crate::worker::wire::ProjectionSource {
                body: *body,
                address: crate::worker::wire::FaceAddress::ElementId(element.as_str()),
                kind: match mode {
                    ProjectionMode::FaceOutline => ProjectionSourceKind::Face,
                    ProjectionMode::Edges => ProjectionSourceKind::Edge,
                },
            })
            .collect();
        match projector
            .project_to_sketch_plane(
                probe.snapshot,
                probe.sketch,
                &probe.plane,
                mode,
                &wire_sources,
            )
            .await
        {
            Ok(result) => fresh.push((result.geometry, result.refusals)),
            Err(e) => {
                tracing::debug!(
                    target: "onecad_lib::regen",
                    sketch = %probe.sketch,
                    "projection staleness probe failed: {e}"
                );
                return None;
            }
        }
    }
    Some(fresh)
}

/// The `SketchSessionDto.projections` map for a sketch: every provenance row,
/// keyed by the entity uuid the wire uses for that entity, ids rendered exactly
/// as `ProjectedEntityDto` renders them (WP-P P2b). Pure; no worker, no lock.
pub fn projections_dto(sketch: &Sketch) -> BTreeMap<String, ProjectedSourceDto> {
    sketch
        .projections
        .iter()
        .map(|(entity, source)| {
            let kind = match source.source_kind {
                ElementKind::Edge => "edge",
                ElementKind::Face => "face",
                ElementKind::Vertex => "vertex",
            };
            (
                entity.to_string(),
                ProjectedSourceDto {
                    source_body_id: body_id_wire(source.source_body),
                    source_element_id: source.source_element_id.as_str().to_string(),
                    source_kind: kind.to_string(),
                    source_ordinal: source.source_ordinal,
                    projected_hash: source.projected_hash.clone(),
                },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use onecad_core::ids::{BodyId, ConstraintId, ElementId, SketchId};
    use onecad_core::sketch::{CurvePosition, SketchAttachment, WorldPlane};
    use uuid::Uuid;

    fn eid(n: u128) -> EntityId {
        EntityId(Uuid::from_u128(n))
    }
    fn cid(n: u128) -> ConstraintId {
        ConstraintId(Uuid::from_u128(n))
    }
    fn body() -> BodyId {
        BodyId(Uuid::from_u128(0xB0D1))
    }
    fn v2(x: f64, y: f64) -> Vec2 {
        Vec2::new(x, y).expect("finite")
    }

    fn source(element: &str, ordinal: u32, hash: &str) -> ProjectedSource {
        ProjectedSource {
            source_body: body(),
            source_element_id: ElementId::new(element),
            source_kind: ElementKind::Edge,
            source_ordinal: ordinal,
            projected_hash: hash.to_string(),
        }
    }

    #[test]
    fn projections_dto_renders_every_row_keyed_by_entity_uuid() {
        let sketch = projected_sketch();
        assert!(
            !sketch.projections.is_empty(),
            "fixture must carry provenance"
        );
        let dto = projections_dto(&sketch);
        assert_eq!(dto.len(), sketch.projections.len());
        for (entity, source) in &sketch.projections {
            let row = dto
                .get(&entity.to_string())
                .expect("row per projected entity");
            assert_eq!(row.source_body_id, body_id_wire(source.source_body));
            assert_eq!(row.source_element_id, source.source_element_id.as_str());
            assert_eq!(row.source_kind, "edge");
            assert_eq!(row.source_ordinal, source.source_ordinal);
            assert_eq!(row.projected_hash, source.projected_hash);
        }
        // Serialized on the session DTO under camelCase keys; omitted when empty.
        let session = crate::dto::SketchSessionDto {
            sketch_id: "sk".into(),
            plane: serde_json::json!({}),
            entities: serde_json::json!([]),
            constraints: serde_json::json!([]),
            dof: 0,
            status: crate::dto::SketchSolveStatus::UnderConstrained,
            conflicting: vec![],
            entity_states: crate::dto::EntityStates::new(),
            projections: dto.clone(),
        };
        let v = serde_json::to_value(&session).unwrap();
        let (first_entity, first_row) = dto.iter().next().unwrap();
        assert_eq!(v["projections"][first_entity]["sourceKind"], "edge");
        assert_eq!(
            v["projections"][first_entity]["sourceElementId"],
            serde_json::json!(first_row.source_element_id)
        );
        let empty = crate::dto::SketchSessionDto {
            projections: Default::default(),
            ..session
        };
        assert!(serde_json::to_value(&empty)
            .unwrap()
            .get("projections")
            .is_none());
    }

    /// Two projected lines sharing the corner point `p1`, each from its own edge.
    fn projected_sketch() -> Sketch {
        let mut s = Sketch::new(
            SketchId(Uuid::from_u128(1)),
            "S",
            SketchAttachment::World {
                plane: WorldPlane::XY,
            },
        );
        for (id, at) in [
            (1u128, v2(0.0, 0.0)),
            (2, v2(10.0, 0.0)),
            (3, v2(10.0, 10.0)),
        ] {
            s.add_entity(SketchEntity::point(eid(id), at, false, true))
                .unwrap();
        }
        s.add_entity(
            SketchEntity::line(eid(10), eid(1), eid(2), false).with_reference_locked(true),
        )
        .unwrap();
        s.add_entity(
            SketchEntity::line(eid(11), eid(2), eid(3), false).with_reference_locked(true),
        )
        .unwrap();
        for (c, p, at) in [
            (1u128, 1u128, v2(0.0, 0.0)),
            (2, 2, v2(10.0, 0.0)),
            (3, 3, v2(10.0, 10.0)),
        ] {
            s.add_constraint(Constraint::Fixed {
                id: cid(c),
                point: eid(p),
                point_position: CurvePosition::Arbitrary,
                at,
            })
            .unwrap();
        }
        s.projections.insert(eid(10), source("el_a", 0, "aaaa"));
        s.projections.insert(eid(11), source("el_b", 0, "bbbb"));
        s
    }

    fn geometry(
        points: &[[f64; 2]],
        entities: Vec<ProjectedEntity>,
        sources: Vec<ProjectedSource>,
    ) -> ProjectedGeometry {
        ProjectedGeometry {
            points: points.iter().map(|p| v2(p[0], p[1])).collect(),
            entities,
            entity_sources: sources,
        }
    }

    /// The whole point of hashing the PROJECTED UV: a source that moved along the
    /// sketch normal re-projects to the same picture and is NOT stale.
    #[test]
    fn an_unchanged_hash_is_not_stale_and_plans_nothing() {
        let s = projected_sketch();
        let fresh = vec![(
            geometry(
                &[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]],
                vec![
                    ProjectedEntity::Line { p0: 0, p1: 1 },
                    ProjectedEntity::Line { p0: 1, p1: 2 },
                ],
                vec![source("el_a", 0, "aaaa"), source("el_b", 0, "bbbb")],
            ),
            Vec::new(),
        )];
        let rows: Vec<_> = s.projections.iter().map(|(e, p)| (*e, p.clone())).collect();
        assert!(stale_projected_entities(&rows, &fresh).is_empty());
        let plan = build_projection_update(&s, &fresh);
        assert!(
            plan.ops.is_empty(),
            "an unchanged projection is not an edit"
        );
        assert!(plan.replaced.is_empty());
    }

    /// A moved wall: both lines re-hash, the shared corner point moves ONCE, its
    /// pin is re-issued at the new UV, and the entity ids are unchanged.
    #[test]
    fn a_moved_line_updates_in_place_and_repins_the_shared_point() {
        let s = projected_sketch();
        let fresh = vec![(
            geometry(
                &[[0.0, 0.0], [30.0, 0.0], [30.0, 10.0]],
                vec![
                    ProjectedEntity::Line { p0: 0, p1: 1 },
                    ProjectedEntity::Line { p0: 1, p1: 2 },
                ],
                vec![source("el_a", 0, "cccc"), source("el_b", 0, "dddd")],
            ),
            Vec::new(),
        )];
        let rows: Vec<_> = s.projections.iter().map(|(e, p)| (*e, p.clone())).collect();
        assert_eq!(stale_projected_entities(&rows, &fresh).len(), 2);

        let plan = build_projection_update(&s, &fresh);
        assert_eq!(plan.replaced.len(), 2);
        assert!(plan.refusals.is_empty());
        assert_eq!(
            plan.replaced.iter().map(|r| r.entity).collect::<Vec<_>>(),
            vec![eid(10), eid(11)],
            "the entity ids are preserved — an update is not a re-mint"
        );

        let moves: Vec<_> = plan
            .ops
            .iter()
            .filter_map(|op| match op {
                SketchEditOp::SetEntityPositions { positions } => Some(positions.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(moves.len(), 1, "one batched move op");
        assert_eq!(moves[0].len(), 2, "p1 and p2 moved; p0 did not");
        assert!(moves[0].iter().any(|(id, at)| *id == eid(2)
            && (at.x - 30.0).abs() < f64::EPSILON
            && at.y.abs() < f64::EPSILON));

        // Every moved point is unlocked before and re-locked after.
        let unlocks = plan.ops.iter().filter(|op| {
            matches!(
                op,
                SketchEditOp::SetEntityReferenceLocked { locked: false, .. }
            )
        });
        assert_eq!(unlocks.count(), 2);
        // …and the pin follows the point.
        let repin = plan.ops.iter().any(|op| {
            matches!(
                op,
                SketchEditOp::AddConstraint { constraint: Constraint::Fixed { point, at, .. } }
                    if *point == eid(2) && (at.x - 30.0).abs() < f64::EPSILON
            )
        });
        assert!(repin, "the Fixed pin must move with its point");
    }

    /// A source that vanished from the re-projection is STALE and refused by name
    /// — never re-associated with whatever now occupies that ordinal.
    #[test]
    fn a_vanished_source_is_stale_and_refused_as_topology_changed() {
        let s = projected_sketch();
        let fresh = vec![(
            geometry(
                &[[0.0, 0.0], [10.0, 0.0]],
                vec![ProjectedEntity::Line { p0: 0, p1: 1 }],
                vec![source("el_a", 0, "aaaa")],
            ),
            Vec::new(),
        )];
        let rows: Vec<_> = s.projections.iter().map(|(e, p)| (*e, p.clone())).collect();
        assert_eq!(stale_projected_entities(&rows, &fresh), vec![eid(11)]);

        let plan = build_projection_update(&s, &fresh);
        assert_eq!(plan.refusals.len(), 1);
        assert_eq!(plan.refusals[0].code, TOPOLOGY_CHANGED);
        assert_eq!(plan.refusals[0].element_id, "el_b");
        assert!(
            plan.ops.is_empty(),
            "nothing else changed, so nothing is applied"
        );
    }

    /// A face source whose boundary GAINED an edge: the committed run and the
    /// fresh one are different lengths, so ordinal 1 of the new run is a
    /// different physical edge from ordinal 1 of the old one. Every row is stale
    /// and the SOURCE gets one refusal — nothing is rewritten from the wrong edge
    /// (WP-P F1, the blocker).
    #[test]
    fn a_run_that_changed_length_refuses_the_whole_source() {
        let mut s = projected_sketch();
        // One FACE source owning both lines: ordinals 0 and 1 of one run.
        let mut face_a = source("el_face", 0, "aaaa");
        face_a.source_kind = ElementKind::Face;
        let mut face_b = face_a.clone();
        face_b.source_ordinal = 1;
        face_b.projected_hash = "bbbb".into();
        s.projections.insert(eid(10), face_a.clone());
        s.projections.insert(eid(11), face_b.clone());

        // The face now emits THREE edges. Ordinals 0 and 1 still exist and even
        // still hash the same — a per-row check would have called this clean.
        let mut third = face_a.clone();
        third.source_ordinal = 2;
        third.projected_hash = "cccc".into();
        let fresh = vec![(
            geometry(
                &[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
                vec![
                    ProjectedEntity::Line { p0: 0, p1: 1 },
                    ProjectedEntity::Line { p0: 1, p1: 2 },
                    ProjectedEntity::Line { p0: 2, p1: 3 },
                ],
                vec![face_a, face_b, third],
            ),
            Vec::new(),
        )];
        let rows: Vec<_> = s.projections.iter().map(|(e, p)| (*e, p.clone())).collect();
        let stale = stale_projected_entities(&rows, &fresh);
        assert_eq!(
            stale.len(),
            2,
            "every row of a re-numbered run is stale, matching hashes included"
        );

        let plan = build_projection_update(&s, &fresh);
        assert_eq!(plan.refusals.len(), 1, "ONE refusal for the source");
        assert_eq!(plan.refusals[0].code, TOPOLOGY_CHANGED);
        assert_eq!(plan.refusals[0].element_id, "el_face");
        assert!(
            plan.refusals[0]
                .message
                .contains("2 edges were projected, 3 are emitted now"),
            "the message names the counts, got {:?}",
            plan.refusals[0].message
        );
        assert!(plan.ops.is_empty(), "not one row of it is touched");
        assert!(plan.replaced.is_empty());
    }

    /// A run that came back the same LENGTH but rotated: the same curves, at
    /// different ordinals. Updating would swap geometry between entity ids, so it
    /// is refused instead (WP-P F2).
    #[test]
    fn a_re_ordered_run_is_refused_rather_than_swapped() {
        let mut s = projected_sketch();
        let mut face_a = source("el_face", 0, "aaaa");
        face_a.source_kind = ElementKind::Face;
        let mut face_b = face_a.clone();
        face_b.source_ordinal = 1;
        face_b.projected_hash = "bbbb".into();
        s.projections.insert(eid(10), face_a.clone());
        s.projections.insert(eid(11), face_b.clone());

        let mut rotated_a = face_a.clone();
        rotated_a.projected_hash = "bbbb".into();
        let mut rotated_b = face_b.clone();
        rotated_b.projected_hash = "aaaa".into();
        let fresh = vec![(
            geometry(
                &[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]],
                vec![
                    ProjectedEntity::Line { p0: 1, p1: 2 },
                    ProjectedEntity::Line { p0: 0, p1: 1 },
                ],
                vec![rotated_a, rotated_b],
            ),
            Vec::new(),
        )];
        let plan = build_projection_update(&s, &fresh);
        assert_eq!(plan.refusals.len(), 1);
        assert_eq!(plan.refusals[0].code, TOPOLOGY_CHANGED);
        assert!(
            plan.refusals[0].message.contains("re-ordered"),
            "got {:?}",
            plan.refusals[0].message
        );
        assert!(plan.ops.is_empty());
    }

    /// A REVERSED wire walk keeps its start edge (row 0 agrees) and swaps the rest.
    /// One agreeing row must not disarm the permutation guard: updating would swap
    /// the geometry of rows 1 and 2 under their ids.
    #[test]
    fn a_reversed_walk_with_a_fixed_start_edge_is_still_refused() {
        let mut s = projected_sketch();
        let mut face_a = source("el_face", 0, "aaaa");
        face_a.source_kind = ElementKind::Face;
        let mut face_b = face_a.clone();
        face_b.source_ordinal = 1;
        face_b.projected_hash = "bbbb".into();
        let mut face_c = face_a.clone();
        face_c.source_ordinal = 2;
        face_c.projected_hash = "cccc".into();
        s.projections.insert(eid(10), face_a.clone());
        s.projections.insert(eid(11), face_b.clone());
        s.projections.insert(eid(12), face_c.clone());

        let mut swapped_b = face_b.clone();
        swapped_b.projected_hash = "cccc".into();
        let mut swapped_c = face_c.clone();
        swapped_c.projected_hash = "bbbb".into();
        let fresh = vec![(
            geometry(
                &[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
                vec![
                    ProjectedEntity::Line { p0: 0, p1: 1 },
                    ProjectedEntity::Line { p0: 2, p1: 3 },
                    ProjectedEntity::Line { p0: 1, p1: 2 },
                ],
                vec![face_a.clone(), swapped_b, swapped_c],
            ),
            Vec::new(),
        )];
        let plan = build_projection_update(&s, &fresh);
        assert_eq!(plan.refusals.len(), 1, "got {:?}", plan.refusals);
        assert_eq!(plan.refusals[0].code, TOPOLOGY_CHANGED);
        assert!(
            plan.refusals[0].message.contains("re-ordered"),
            "got {:?}",
            plan.refusals[0].message
        );
        assert!(plan.ops.is_empty());
        let stale = stale_projected_entities(
            &s.projections
                .iter()
                .map(|(e, p)| (*e, p.clone()))
                .collect::<Vec<_>>(),
            &fresh,
        );
        assert_eq!(
            stale.len(),
            3,
            "every row of a refused source is stale, got {stale:?}"
        );
    }

    /// An `edges`-mode source is one row at ordinal 0, so neither run guard can
    /// fire on it: a moved edge still updates in place.
    #[test]
    fn a_single_edge_source_is_never_caught_by_the_run_guards() {
        let s = projected_sketch();
        let fresh = vec![(
            geometry(
                &[[0.0, 0.0], [12.0, 0.0], [10.0, 10.0]],
                vec![
                    ProjectedEntity::Line { p0: 0, p1: 1 },
                    ProjectedEntity::Line { p0: 1, p1: 2 },
                ],
                vec![source("el_a", 0, "bbbb"), source("el_b", 0, "aaaa")],
            ),
            Vec::new(),
        )];
        let plan = build_projection_update(&s, &fresh);
        assert!(
            plan.refusals.is_empty(),
            "swapped hashes across DIFFERENT sources are not a re-ordering: {:?}",
            plan.refusals
        );
        assert_eq!(plan.replaced.len(), 2);
    }

    /// Two sources sharing a corner that stop agreeing about where it goes: BOTH
    /// are refused and neither is applied, rather than one silently dragging the
    /// other's endpoint (WP-P F3).
    #[test]
    fn a_shared_corner_split_refuses_both_sources() {
        let s = projected_sketch();
        // el_a puts the shared corner (eid 2) at (30,0); el_b puts it at (31,0).
        let fresh = vec![
            (
                geometry(
                    &[[0.0, 0.0], [30.0, 0.0]],
                    vec![ProjectedEntity::Line { p0: 0, p1: 1 }],
                    vec![source("el_a", 0, "cccc")],
                ),
                Vec::new(),
            ),
            (
                geometry(
                    &[[31.0, 0.0], [30.0, 10.0]],
                    vec![ProjectedEntity::Line { p0: 0, p1: 1 }],
                    vec![source("el_b", 0, "dddd")],
                ),
                Vec::new(),
            ),
        ];
        let plan = build_projection_update(&s, &fresh);
        assert_eq!(plan.refusals.len(), 2, "both contributing sources refused");
        assert!(plan.refusals.iter().all(|r| r.code == TOPOLOGY_CHANGED));
        assert!(plan
            .refusals
            .iter()
            .all(|r| r.message.contains("shared corner split")));
        let named: Vec<&str> = plan
            .refusals
            .iter()
            .map(|r| r.element_id.as_str())
            .collect();
        assert!(
            named.contains(&"el_a") && named.contains(&"el_b"),
            "{named:?}"
        );
        assert!(
            plan.ops.is_empty() && plan.replaced.is_empty(),
            "neither source is applied"
        );
    }

    /// The collision check runs BEFORE the no-op filter: a claim that happens to
    /// land on the point's current position still counts, so a genuine split is
    /// not hidden by one side asking for no movement at all.
    #[test]
    fn a_no_op_claim_still_counts_as_a_claim() {
        let s = projected_sketch();
        let fresh = vec![
            (
                geometry(
                    // el_a leaves the shared corner exactly where it is (10,0)…
                    &[[0.0, 0.0], [10.0, 0.0]],
                    vec![ProjectedEntity::Line { p0: 0, p1: 1 }],
                    vec![source("el_a", 0, "cccc")],
                ),
                Vec::new(),
            ),
            (
                geometry(
                    // …while el_b moves it.
                    &[[40.0, 0.0], [10.0, 10.0]],
                    vec![ProjectedEntity::Line { p0: 0, p1: 1 }],
                    vec![source("el_b", 0, "dddd")],
                ),
                Vec::new(),
            ),
        ];
        let plan = build_projection_update(&s, &fresh);
        assert_eq!(plan.refusals.len(), 2, "the no-op claim is still a claim");
        assert!(plan.ops.is_empty());
    }

    /// A worker refusal for the source is not doubled up with a Rust one.
    #[test]
    fn a_worker_refused_source_does_not_also_get_a_rust_refusal() {
        let s = projected_sketch();
        let fresh = vec![(
            geometry(
                &[[0.0, 0.0], [10.0, 0.0]],
                vec![ProjectedEntity::Line { p0: 0, p1: 1 }],
                vec![source("el_a", 0, "aaaa")],
            ),
            vec![ProjectionRefusal {
                body_id: crate::worker::wire::body_id_wire(body()),
                element_id: "el_b".into(),
                topo_key: String::new(),
                code: "absent".into(),
                message: "gone".into(),
            }],
        )];
        let plan = build_projection_update(&s, &fresh);
        assert!(plan.refusals.is_empty(), "the worker already said it");
    }

    /// A re-projection that came back a DIFFERENT SHAPE is refused, not guessed at.
    #[test]
    fn a_changed_entity_type_is_refused_not_reshaped() {
        let s = projected_sketch();
        let fresh = vec![(
            geometry(
                &[[0.0, 0.0]],
                vec![ProjectedEntity::Circle {
                    center: 0,
                    radius: 5.0,
                }],
                vec![source("el_a", 0, "cccc")],
            ),
            Vec::new(),
        )];
        let plan = build_projection_update(&s, &fresh);
        assert_eq!(plan.refusals.len(), 2, "el_a reshaped, el_b vanished");
        assert!(plan.refusals.iter().all(|r| r.code == TOPOLOGY_CHANGED));
        assert!(plan.replaced.is_empty());
    }

    /// Detaching ONE of two lines frees only the point the other has let go of.
    #[test]
    fn detach_releases_only_the_points_no_projection_still_holds() {
        let s = projected_sketch();
        let (ops, released) = build_projection_detach(&s, &[eid(10)]);

        assert_eq!(released, 1, "only p0 is freed; p1 is still held by line 11");
        assert!(ops.iter().any(|op| matches!(
            op,
            SketchEditOp::SetEntityProjection { entity, source: None } if *entity == eid(10)
        )));
        assert!(ops.iter().any(|op| matches!(
            op,
            SketchEditOp::SetEntityReferenceLocked { entity, locked: false } if *entity == eid(1)
        )));
        assert!(
            !ops.iter().any(|op| matches!(
                op,
                SketchEditOp::SetEntityReferenceLocked { entity, locked: false } if *entity == eid(2)
            )),
            "the shared corner stays locked while line 11 is still projected"
        );
    }

    /// Detaching everything frees every point and drops every pin.
    #[test]
    fn detach_all_frees_every_point_and_pin() {
        let s = projected_sketch();
        let targets: Vec<EntityId> = s.projections.keys().copied().collect();
        let (ops, released) = build_projection_detach(&s, &targets);
        assert_eq!(released, 3);
        let unlocked: Vec<EntityId> = ops
            .iter()
            .filter_map(|op| match op {
                SketchEditOp::SetEntityReferenceLocked {
                    entity,
                    locked: false,
                } => Some(*entity),
                _ => None,
            })
            .collect();
        for id in [1u128, 2, 3, 10, 11] {
            assert!(unlocked.contains(&eid(id)), "entity {id} must be unlocked");
        }
    }

    /// The re-request plan splits by kind, because an edge source and a face
    /// source are asked for in different MODES.
    #[test]
    fn the_request_plan_splits_edge_and_face_sources_by_mode() {
        let mut s = projected_sketch();
        let mut face = source("el_face", 0, "eeee");
        face.source_kind = ElementKind::Face;
        s.projections.insert(eid(12), face.clone());
        let mut face2 = face.clone();
        face2.source_ordinal = 1;
        s.projections.insert(eid(13), face2);

        let plan = projection_request_plan(&s);
        assert_eq!(plan.len(), 2);
        assert_eq!(plan[0].0, ProjectionMode::Edges);
        assert_eq!(plan[0].1.len(), 2);
        assert_eq!(plan[1].0, ProjectionMode::FaceOutline);
        assert_eq!(
            plan[1].1.len(),
            1,
            "one face source, however many entities it owns"
        );
    }
}
