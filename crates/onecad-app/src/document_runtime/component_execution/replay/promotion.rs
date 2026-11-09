//! Checked missing-evidence enrichment for current component faces.
use super::*;
use crate::worker::ElementQuery;

fn anchor(point: [f64; 3]) -> Result<AnchorIntent, ApiError> {
    Ok(AnchorIntent {
        world_point: Vec3::try_from(point).map_err(|e| ApiError::InvalidCommand(e.into()))?,
        surface_uv: None,
        local_frame: None,
        adjacency_hint: None,
        extra: Default::default(),
    })
}

fn sample_face(mesh: &MeshView<'_>, index: usize) -> Option<[f64; 3]> {
    let range = mesh.face_range(index)?;
    let mut best = None;
    for triangle in range.first..range.first.checked_add(range.count)? {
        let tri = mesh.triangle(triangle as usize)?;
        let a = mesh.position(tri[0] as usize)?.map(f64::from);
        let b = mesh.position(tri[1] as usize)?.map(f64::from);
        let c = mesh.position(tri[2] as usize)?.map(f64::from);
        let u = std::array::from_fn::<_, 3, _>(|i| b[i] - a[i]);
        let v = std::array::from_fn::<_, 3, _>(|i| c[i] - a[i]);
        let cross = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        let area2 = cross.into_iter().map(|x| x * x).sum::<f64>();
        if area2.is_finite() && area2 > 0.0 && best.as_ref().is_none_or(|(old, _)| area2 > *old) {
            best = Some((area2, std::array::from_fn(|i| (a[i] + b[i] + c[i]) / 3.0)));
        }
    }
    best.map(|(_, point)| point)
}

pub(super) async fn promote_anchored(
    runtime: &mut DocumentRuntime,
    worker: &crate::worker::WorkerManager,
    snapshot: SnapshotId,
    meshes: &BTreeMap<BodyId, Arc<Vec<u8>>>,
    read_only: bool,
) -> Result<bool, ApiError> {
    let mut rows = Vec::new();
    let mut enriched = Vec::new();
    let mut persistent = std::collections::HashSet::new();
    for (body, bytes) in meshes {
        let mesh = MeshView::decode(bytes, MeshLimits::default())
            .map_err(|e| ApiError::InvalidCommand(e.to_string()))?;
        let mut picks = Vec::new();
        for index in 0..mesh.header().face_count as usize {
            let key = mesh
                .face_id(index)
                .ok_or_else(|| ApiError::InvalidCommand("missing face identity".into()))?;
            if TopoKey::new(key).is_valid() {
                picks.push((
                    TopoKey::new(key),
                    Some(anchor(sample_face(&mesh, index).ok_or_else(|| {
                        ApiError::InvalidCommand("missing face sample".into())
                    })?)?),
                ));
            } else {
                let id = ElementId::new(key);
                if !persistent.insert(id.clone()) {
                    return Err(ApiError::InvalidCommand(
                        "duplicate component face identity".into(),
                    ));
                }
                let entry = runtime
                    .regen
                    .elements
                    .get(&id)
                    .filter(|entry| {
                        entry.body == *body
                            && entry.kind == ElementKind::Face
                            && !entry.is_quarantined()
                    })
                    .ok_or_else(|| {
                        ApiError::UnresolvedTopology(
                            "component face identity has no matching evidence".into(),
                        )
                    })?;
                if entry.anchor.is_none() || entry.descriptor.is_none() {
                    let sample = sample_face(&mesh, index).ok_or_else(|| {
                        ApiError::UnresolvedTopology(
                            "component face lacks a nondegenerate mesh sample".into(),
                        )
                    })?;
                    enriched.push((id, entry.clone(), anchor(sample)?));
                }
            }
        }
        rows.push((*body, picks));
    }
    let count = rows.iter().map(|(_, p)| p.len()).sum::<usize>();
    if (count != 0 || !enriched.is_empty()) && read_only {
        return Err(ApiError::InvalidCommand(
            "read-only component cannot persist topology identities".into(),
        ));
    }
    if runtime.regen.elements.len().saturating_add(count) > 4096 {
        return Err(ApiError::InvalidCommand(
            "component topology budget exceeded".into(),
        ));
    }
    for (body, picks) in rows {
        for chunk in picks.chunks(128) {
            runtime
                .promote_selection(snapshot, body, chunk.to_vec())
                .await
                .map_err(|e| ApiError::Worker(e.to_string()))?;
        }
    }
    let changed = count != 0 || !enriched.is_empty();
    for (id, entry, sample) in enriched {
        let next = enrich(runtime, worker, snapshot, &id, entry, sample).await?;
        runtime.regen.elements.insert(id, next);
    }
    Ok(changed)
}

async fn enrich(
    runtime: &mut DocumentRuntime,
    worker: &crate::worker::WorkerManager,
    snapshot: SnapshotId,
    id: &ElementId,
    entry: ElementEntry,
    sample: AnchorIntent,
) -> Result<ElementEntry, ApiError> {
    let info = worker
        .query_element(snapshot, entry.body, id.as_str())
        .await
        .map_err(|e| ApiError::Worker(e.to_string()))?
        .filter(|info| {
            info.element_id == id.as_str()
                && info.kind == "face"
                && crate::worker::wire::parse_body_id(&info.body_id).ok() == Some(entry.body)
        })
        .ok_or_else(|| {
            ApiError::UnresolvedTopology("component face evidence query mismatch".into())
        })?;
    let rows = runtime
        .promote_inner(
            snapshot,
            entry.body,
            vec![(TopoKey::new(info.topo_key), Some(sample))],
        )
        .await
        .map_err(|e| ApiError::Worker(e.to_string()))?;
    let [(actual, evidence)] = rows.as_slice() else {
        return Err(ApiError::UnresolvedTopology(
            "component face evidence inventory mismatch".into(),
        ));
    };
    merge_evidence(id, actual, entry, evidence)
}

fn merge_evidence(
    id: &ElementId,
    actual: &ElementId,
    mut entry: ElementEntry,
    evidence: &onecad_core::regen::WorkerElementEvidence,
) -> Result<ElementEntry, ApiError> {
    if actual != id
        || entry.is_quarantined()
        || evidence.body != entry.body
        || entry.kind != ElementKind::Face
        || evidence.kind != entry.kind
        || evidence.descriptor.is_none()
        || evidence.anchor.is_none()
        || entry.import_source_identity != evidence.import_source_identity
    {
        return Err(ApiError::UnresolvedTopology(
            "component face evidence identity mismatch".into(),
        ));
    }
    // Fill absent index evidence only. Operation inputs and every existing
    // anchor, descriptor and import certificate retain their original meaning.
    if entry.anchor.is_none() {
        entry.anchor = evidence.anchor.clone();
    }
    if entry.descriptor.is_none() {
        entry.descriptor = evidence.descriptor.clone();
    }
    Ok(entry)
}

#[cfg(test)]
mod tests;
