//! Isolated component replay and checked mesh verification.
use super::*;
mod promotion;

struct AbortReplayTask(tokio::task::AbortHandle);
impl Drop for AbortReplayTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn future_bytes<F: std::future::Future>(_: &impl FnOnce() -> F) -> usize {
    std::mem::size_of::<F>()
}

impl ComponentReplayInput {
    pub(crate) async fn drive(
        self,
        worker: crate::worker::WorkerManager,
        epoch: WorkerEpoch,
        promote: bool,
    ) -> Result<ComponentReplayOutput, ApiError> {
        let budget = self.archive_budget.clone();
        let make_work = move || self.drive_inner(worker, epoch, promote);
        let work_bytes = future_bytes(&make_work)
            .checked_add(std::mem::size_of_val(&make_work))
            .ok_or_else(|| ApiError::Io("component replay task size overflow".into()))?;
        let reservation = budget
            .reserve(work_bytes, crate::resource_admission::ArchiveUse::Scratch)
            .map_err(ApiError::Io)?;
        let child_reservation = reservation.clone();
        // Replay must not share the UI's deep poll stack. Cancellation aborts
        // only this isolated task; its worker lease still owns process reaping.
        let task = tokio::spawn(async move {
            let _reservation = child_reservation;
            // Construct the large future only on the isolated task's short
            // poll stack, then move its state to the admitted allocation.
            Box::pin(make_work()).await
        });
        let _abort = AbortReplayTask(task.abort_handle());
        task.await.map_err(|error| {
            ApiError::Internal(format!("component replay task stopped: {error}"))
        })?
    }

    async fn drive_inner(
        mut self,
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
        let continuation_retention = if let Some(base) = &self.translation_prefix {
            let continuation =
                super::translation::continue_translation(&self, &mut working, &worker, base);
            // An inline continuation enlarges deep native capture futures even
            // when their replay does not take this branch.
            let _future_retention = self
                .archive_budget
                .reserve(
                    std::mem::size_of_val(&continuation),
                    crate::resource_admission::ArchiveUse::Scratch,
                )
                .map_err(ApiError::Io)?;
            Some(Box::pin(continuation).await?)
        } else {
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
            None
        };
        if worker.current_epoch() != epoch {
            return Err(ApiError::InvalidCommand(
                "component worker epoch changed during replay".into(),
            ));
        }
        if let Some(proof) = &self.evidence_translation {
            proof.apply(&mut working)?;
        }
        let quarantine = self.rebind_component(&mut working).await?;
        if let Some(proof) = &self.evidence_translation {
            proof.refresh(&mut working, &worker).await?;
        }
        let (meshes, _) = collect_fresh_meshes(&working).await?;
        let snapshot = working
            .latest_snapshot
            .as_ref()
            .map_or(SnapshotId(0), |s| s.id);
        let minted = if promote {
            promotion::promote_anchored(&mut working, &worker, snapshot, &meshes, self.read_only)
                .await?
        } else {
            false
        };
        if let Some(proof) = &self.evidence_translation {
            proof.validate_inventory(&self.definition.elements, &working.regen.elements)?;
        }
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
        let replay_context = super::translation::replay_context(&self.settings, &self.imports)
            .ok_or_else(|| {
                ApiError::InvalidCommand("component replay context serialization failed".into())
            })?;
        let identity_signature =
            if promote || continuation_retention.is_some() || self.evidence_translation.is_some() {
                Some(mesh_signatures(&meshes)?)
            } else {
                None
            };
        drop(working);
        let translated = continuation_retention.is_some();
        drop(continuation_retention);
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
            minted: minted
                || translated
                || quarantine.is_some()
                || self.evidence_translation.is_some(),
            replay_context,
            identity_signature,
            quarantine,
            evidence_translation: self.evidence_translation,
            _retention: self._retention,
        })
    }

    async fn rebind_component(
        &mut self,
        working: &mut DocumentRuntime,
    ) -> Result<Option<quarantine::QuarantineCommit>, ApiError> {
        if !working.has_unbound_persisted_elements() {
            return Ok(None);
        }
        let Some(ticket) = working.prepare_persisted_rebind() else {
            // Whole-body deletion retains ordinary intent for Undo. It cannot
            // bind to a surviving partition and is not a face-quarantine answer.
            if working.regen.elements.iter().all(|(_, entry)| {
                !entry.has_rebind_evidence() || !working.regen.bodies.contains(entry.body)
            }) {
                return Ok(None);
            }
            return Err(ApiError::UnresolvedTopology(
                "missing component rebind head".into(),
            ));
        };
        let mut result = ticket.drive().await;
        let expected = result
            .counts
            .map_err(|()| ApiError::UnresolvedTopology("component rebind failed".into()))?;
        let rows = std::mem::take(&mut result.unresolved);
        if working.adopt_persisted_rebind(result) != expected {
            return Err(ApiError::StalePreview(
                "component rebind head changed".into(),
            ));
        }
        let proof = if rows.is_empty() {
            None
        } else {
            Some(
                self.quarantine
                    .take()
                    .ok_or_else(|| {
                        ApiError::UnresolvedTopology(
                            "component topology identities could not be rebound".into(),
                        )
                    })?
                    .apply(self.id, working, &rows)?,
            )
        };
        if working.has_unbound_persisted_elements() {
            return Err(ApiError::UnresolvedTopology(
                "component topology identities could not be rebound".into(),
            ));
        }
        Ok(proof)
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
        // This temporary root document contains geometry inputs only. Durable
        // reservations stay in the owned definition and the guarded regen mirror.
        for (id, entry) in def
            .elements
            .iter()
            .filter(|(_, entry)| !entry.is_quarantined())
        {
            document.elements.insert(id.clone(), entry.clone());
        }
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
        runtime.regen.elements = def.elements.clone();
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

pub(super) async fn collect_fresh_meshes(
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
            runtime.reject_quarantined_mesh(&bytes)?;
            meshes.insert(body.body, Arc::new(bytes));
        }
    }
    Ok((meshes, budget))
}

impl ComponentReplayOutput {
    pub(crate) fn verification_signature(&self) -> Result<BTreeMap<BodyId, Vec<u8>>, ApiError> {
        mesh_signatures(&self.meshes)
    }
}

pub(super) fn mesh_signatures(
    meshes: &BTreeMap<BodyId, Arc<Vec<u8>>>,
) -> Result<BTreeMap<BodyId, Vec<u8>>, ApiError> {
    meshes
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
