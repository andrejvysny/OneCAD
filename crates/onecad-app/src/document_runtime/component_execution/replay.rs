//! Isolated component replay and checked mesh verification.
use super::*;

impl ComponentReplayInput {
    pub(crate) async fn drive(
        self,
        worker: crate::worker::WorkerManager,
        epoch: WorkerEpoch,
        promote: bool,
    ) -> Result<ComponentReplayOutput, ApiError> {
        if self.definition.repair.first_seeded_step().is_some() {
            return Err(ApiError::InvalidCommand(
                "component contains unresolved repair gates".into(),
            ));
        }
        if worker.current_epoch() != epoch {
            return Err(ApiError::StalePreview(
                "component worker restarted before replay".into(),
            ));
        }
        let mut working = self.working_runtime(worker.clone(), epoch)?;
        let fence = working.fencing.clone();
        worker.set_restart_hook(Arc::new(move |epoch| fence.set_epoch(epoch.0)));
        if worker.current_epoch() != epoch {
            return Err(ApiError::StalePreview(
                "component worker restarted before replay".into(),
            ));
        }
        let original_ids: Vec<_> = self
            .definition
            .timeline
            .records()
            .iter()
            .map(|record| record.record_id)
            .collect();
        let working_ids: Vec<_> = working
            .session
            .document()
            .timeline
            .records()
            .iter()
            .map(|record| record.record_id)
            .collect();
        if original_ids != working_ids {
            return Err(ApiError::InvalidCommand(
                "component contains sketches without authored timeline records".into(),
            ));
        }
        let report = working
            .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
            .await;
        if !matches!(report.outcome, Outcome::Published(_) | Outcome::NoOp)
            || !report.failed_steps.is_empty()
            || !report.needs_repair.is_empty()
            || (matches!(report.outcome, Outcome::NoOp)
                && self
                    .definition
                    .timeline
                    .records()
                    .iter()
                    .take(self.definition.timeline.cursor())
                    .any(|record| !record.suppressed))
            || (matches!(report.outcome, Outcome::NoOp) && !working.regen.bodies.is_empty())
        {
            return Err(ApiError::InvalidCommand(
                "component replay did not produce valid geometry".into(),
            ));
        }
        if worker.current_epoch() != epoch {
            return Err(ApiError::InvalidCommand(
                "component worker epoch changed during replay".into(),
            ));
        }
        if working.has_unbound_persisted_elements() {
            let (_, unresolved) = working.rebind_persisted_elements().await;
            if unresolved != 0 || working.has_unbound_persisted_elements() {
                return Err(ApiError::UnresolvedTopology(
                    "component topology identities could not be rebound".into(),
                ));
            }
        }
        let (meshes, _) = collect_fresh_meshes(&working).await?;
        let snapshot = working
            .latest_snapshot
            .as_ref()
            .map_or(SnapshotId(0), |s| s.id);
        let minted = if promote {
            promote_anchored(&mut working, snapshot, &meshes, self.read_only).await?
        } else {
            false
        };
        drop(meshes);
        let (meshes, budget) = collect_fresh_meshes(&working).await?;
        let head = worker
            .get_worker_head()
            .await
            .map_err(|e| ApiError::Worker(e.to_string()))?;
        if head.snapshot_id != snapshot
            || head.worker_epoch != epoch
            || worker.current_epoch() != epoch
        {
            return Err(ApiError::StalePreview(
                "component worker head changed".into(),
            ));
        }
        let definition = replayed_definition(&self.definition, &working);
        let snapshot = working
            .latest_snapshot
            .as_ref()
            .map_or(SnapshotId(0), |snapshot| snapshot.id);
        Ok(ComponentReplayOutput {
            id: self.id,
            runtime_session: self.runtime_session,
            revision: self.revision,
            epoch,
            source_fingerprint: self.source_fingerprint,
            definition,
            meshes,
            budget,
            snapshot,
            minted,
            _retention: self._retention,
        })
    }

    fn working_runtime(
        &self,
        worker: crate::worker::WorkerManager,
        epoch: WorkerEpoch,
    ) -> Result<DocumentRuntime, ApiError> {
        let mut document = Document::new(self.namespace);
        let def = &self.definition;
        document.settings = self.settings.clone();
        document.variables = def.variables.clone();
        document.sketches = def.sketches.clone();
        document.sketch_visibility = def.sketch_visibility.clone();
        document.datum_planes = def.datum_planes.clone();
        document.timeline = def.timeline.clone();
        document.bodies = def.bodies.clone();
        document.elements = def.elements.clone();
        document.repair = def.repair.clone();
        document.named_selections = def.named_selections.clone();
        let engine = Arc::new(worker);
        let mut runtime = DocumentRuntime::from_document(
            document,
            def.name.clone(),
            None,
            false,
            engine.clone(),
            engine.clone(),
            engine,
        );
        runtime.fencing = Arc::new(FencingCell {
            revision: self.revision_fence.clone(),
            epoch: AtomicU64::new(epoch.0),
        });
        for (sha, blob) in &self.imports {
            runtime
                .import_workspace
                .materialize(sha, blob.codec, &blob.bytes)
                .map_err(|error| {
                    ApiError::Io(format!("component source materialization: {error}"))
                })?;
        }
        runtime.imports = self.imports.clone();
        Ok(runtime)
    }
}

async fn collect_fresh_meshes(
    runtime: &DocumentRuntime,
) -> Result<(BTreeMap<BodyId, Arc<Vec<u8>>>, MeshBudget), ApiError> {
    let mut meshes = BTreeMap::new();
    let mut budget = MeshBudget::default();
    if let Some(snapshot) = &runtime.latest_snapshot {
        for body in &snapshot.bodies {
            let bytes = runtime
                .meshes
                .fetch_mesh(body.body, body.mesh_key.lod, snapshot.id)
                .await
                .map_err(|e| ApiError::Worker(e.to_string()))?;
            budget.add(mesh_budget(&bytes)?);
            budget.validate()?;
            meshes.insert(body.body, Arc::new(bytes));
        }
    }
    Ok((meshes, budget))
}

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

async fn promote_anchored(
    runtime: &mut DocumentRuntime,
    snapshot: SnapshotId,
    meshes: &BTreeMap<BodyId, Arc<Vec<u8>>>,
    read_only: bool,
) -> Result<bool, ApiError> {
    let mut rows = Vec::new();
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
            } else if runtime.regen.elements.body_of(&ElementId::new(key)) != Some(*body) {
                return Err(ApiError::InvalidCommand(
                    "component face identity has no matching evidence".into(),
                ));
            }
        }
        rows.push((*body, picks));
    }
    let count = rows.iter().map(|(_, p)| p.len()).sum::<usize>();
    if count != 0 && read_only {
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
    Ok(count != 0)
}

impl ComponentReplayOutput {
    pub(crate) fn verification_signature(&self) -> Result<BTreeMap<BodyId, Vec<u8>>, ApiError> {
        self.meshes
            .iter()
            .map(|(body, bytes)| {
                let mesh = MeshView::decode(bytes, MeshLimits::default())
                    .map_err(|error| ApiError::InvalidCommand(error.to_string()))?;
                let mut hash = Sha256::new();
                hash.update(b"onecad.component-replay.mesh.v1\0");
                hash_floats(&mut hash, Some(mesh.positions()));
                hash_floats(&mut hash, mesh.normals());
                hash_floats(&mut hash, mesh.edge_positions());
                hash_floats(&mut hash, mesh.face_bboxes());
                hash.update(mesh.indices().len().to_le_bytes());
                for value in mesh.indices().iter() {
                    hash.update(value.to_le_bytes());
                }
                hash.update(mesh.header().face_count.to_le_bytes());
                hash.update(mesh.header().edge_count.to_le_bytes());
                for row in 0..mesh.header().face_count as usize {
                    let id = mesh
                        .face_id(row)
                        .filter(|id| id.starts_with("el_"))
                        .ok_or_else(|| {
                            ApiError::InvalidCommand(
                                "component face identity cannot be verified".into(),
                            )
                        })?;
                    hash.update(id.len().to_le_bytes());
                    hash.update(id.as_bytes());
                    hash.update(
                        serde_json::to_vec(
                            &mesh.face_range(row).map(|range| (range.first, range.count)),
                        )
                        .map_err(|error| ApiError::Internal(error.to_string()))?,
                    );
                }
                hash.update([u8::from(mesh.face_colors().is_some())]);
                if let Some(colors) = mesh.face_colors() {
                    hash.update(colors.len().to_le_bytes());
                    hash.update(colors);
                }
                hash_edges(&mut hash, &mesh)?;
                Ok((*body, hash.finalize().to_vec()))
            })
            .collect()
    }
}

pub(super) fn hash_edges(hash: &mut Sha256, mesh: &MeshView<'_>) -> Result<(), ApiError> {
    for row in 0..mesh.header().edge_count as usize {
        hash.update(
            serde_json::to_vec(&mesh.edge_range(row).map(|range| (range.first, range.count)))
                .map_err(|error| ApiError::Internal(error.to_string()))?,
        );
        // Transient worker TopoKeys are not cross-session identities.
        let id = mesh.edge_id(row).filter(|id| id.starts_with("el_"));
        hash.update([u8::from(id.is_some())]);
        if let Some(id) = id {
            hash.update(id.len().to_le_bytes());
            hash.update(id.as_bytes());
        }
        hash.update((mesh.edge_class(row) as u32).to_le_bytes());
        let faces = mesh.edge_incident_faces(row);
        hash.update(faces.len().to_le_bytes());
        for face in faces.iter() {
            hash.update(face.to_le_bytes());
        }
    }
    Ok(())
}

fn hash_floats(hash: &mut Sha256, values: Option<onecad_protocol::mesh::MeshFloats<'_>>) {
    hash.update([u8::from(values.is_some())]);
    if let Some(values) = values {
        hash.update(values.len().to_le_bytes());
        for value in values.iter() {
            hash.update(value.to_le_bytes());
        }
    }
}

fn replayed_definition(source: &ComponentDef, working: &DocumentRuntime) -> ComponentDef {
    let document = working.session.document();
    let mut result = source.clone();
    result.variables = document.variables.clone();
    result.sketches = document.sketches.clone();
    result.sketch_visibility = document.sketch_visibility.clone();
    result.datum_planes = document.datum_planes.clone();
    result.timeline = document.timeline.clone();
    result.bodies = working.merged_bodies();
    result.elements = working.regen.elements.clone();
    result.repair = working.regen.repair.clone();
    result.named_selections = document.named_selections.clone();
    result
}
