//! Fenced, bounded replay for one embedded definition on an auxiliary worker.
use super::*;
use onecad_core::document::component::{ComponentDef, ComponentDefId, OccurrencePath};
use onecad_protocol::mesh::{MeshLimits, MeshView};

const MAX_TRIANGLES: usize = 500_000;
const MAX_SEGMENTS: usize = 500_000;
const MAX_BODIES: usize = 4096;
const MAX_RESIDENT_BYTES: usize = 256 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Default)]
struct MeshBudget {
    triangles: usize,
    segments: usize,
    bodies: usize,
    bytes: usize,
}

impl MeshBudget {
    fn add(&mut self, other: Self) {
        self.triangles = self.triangles.saturating_add(other.triangles);
        self.segments = self.segments.saturating_add(other.segments);
        self.bodies = self.bodies.saturating_add(other.bodies);
        self.bytes = self.bytes.saturating_add(other.bytes);
    }

    fn validate(self) -> Result<(), ApiError> {
        if self.triangles > MAX_TRIANGLES
            || self.segments > MAX_SEGMENTS
            || self.bodies > MAX_BODIES
            || self.bytes > MAX_RESIDENT_BYTES
        {
            return Err(ApiError::InvalidCommand(
                "assembly geometry exceeds the scene resource budget".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ComponentCache {
    fingerprint: String,
    pub generation: u64,
    meshes: BTreeMap<BodyId, Arc<Vec<u8>>>,
    budget: MeshBudget,
}

#[derive(Clone)]
pub(crate) struct ComponentReplayInput {
    pub id: ComponentDefId,
    pub namespace: DocumentId,
    pub runtime_session: String,
    pub revision: DocumentRevision,
    pub source_fingerprint: String,
    pub definition: ComponentDef,
    pub settings: onecad_core::document::DocumentSettings,
    pub imports: ImportBlobs,
    pub revision_fence: Arc<AtomicU64>,
    pub read_only: bool,
}

pub(crate) struct ComponentReplayOutput {
    id: ComponentDefId,
    runtime_session: String,
    revision: DocumentRevision,
    pub epoch: WorkerEpoch,
    source_fingerprint: String,
    pub definition: ComponentDef,
    meshes: BTreeMap<BodyId, Arc<Vec<u8>>>,
    budget: MeshBudget,
    snapshot: SnapshotId,
    pub minted: bool,
}

fn fingerprint(definition: &ComponentDef) -> Result<String, ApiError> {
    let bytes = serde_json::to_vec(definition)
        .map_err(|error| ApiError::InvalidCommand(format!("component serialization: {error}")))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn namespace(document: DocumentId, definition: ComponentDefId) -> DocumentId {
    let mut hash = Sha256::new();
    hash.update(b"onecad.component-worker-session.v1\0");
    hash.update(document.0.as_bytes());
    hash.update(definition.0.as_bytes());
    let digest = hash.finalize();
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&digest[..16]);
    DocumentId(Uuid::from_bytes(bytes))
}

impl DocumentRuntime {
    pub(crate) fn prepare_component_replay(
        &self,
        id: ComponentDefId,
        session: &str,
    ) -> Result<ComponentReplayInput, ApiError> {
        if !self.matches_runtime_session(session) {
            return Err(ApiError::InvalidCommand(
                "component replay belongs to a stale runtime".into(),
            ));
        }
        let doc = self.session.document();
        let def = doc
            .definitions
            .get(&id)
            .ok_or_else(|| ApiError::InvalidCommand("unknown component definition".into()))?;
        Ok(ComponentReplayInput {
            id,
            namespace: namespace(doc.id, id),
            runtime_session: self.runtime_session(),
            revision: self.revision(),
            source_fingerprint: fingerprint(def)?,
            definition: def.clone(),
            settings: doc.settings.clone(),
            imports: self.imports.clone(),
            revision_fence: self.fencing.revision.clone(),
            read_only: self.read_only,
        })
    }

    pub(crate) fn commit_component_replay(
        &mut self,
        output: ComponentReplayOutput,
        current_epoch: WorkerEpoch,
    ) -> Result<u64, ApiError> {
        if !self.matches_runtime_session(&output.runtime_session)
            || self.revision() != output.revision
            || output.epoch != current_epoch
        {
            return Err(ApiError::InvalidCommand("component replay is stale".into()));
        }
        let current = self
            .session
            .document()
            .definitions
            .get(&output.id)
            .ok_or_else(|| ApiError::InvalidCommand("component definition was removed".into()))?;
        if fingerprint(current)? != output.source_fingerprint {
            return Err(ApiError::InvalidCommand(
                "component definition changed during replay".into(),
            ));
        }
        self.check_component_budget(output.id, output.budget)?;
        let generation = self
            .component_generation
            .checked_add(1)
            .ok_or_else(|| ApiError::InvalidCommand("component generation exhausted".into()))?;
        self.require_generation_capacity()?;
        let new_fingerprint = fingerprint(&output.definition)?;
        if !self
            .session
            .adopt_component_replay(output.id, output.definition)
        {
            return Err(ApiError::InvalidCommand(
                "component definition was removed".into(),
            ));
        }
        self.persisted_generation += 1;
        self.dirty |= output.minted;
        self.component_generation = generation;
        self.component_cache.insert(
            output.id,
            ComponentCache {
                fingerprint: new_fingerprint,
                generation,
                meshes: output.meshes,
                budget: output.budget,
            },
        );
        Ok(generation)
    }

    fn check_component_budget(
        &self,
        id: ComponentDefId,
        budget: MeshBudget,
    ) -> Result<(), ApiError> {
        let mut total = budget;
        for (other, cached) in &self.component_cache {
            if *other != id {
                total.add(cached.budget);
            }
        }
        let root = self.latest_snapshot.as_ref();
        if root.is_none() && !self.regen.bodies.is_empty() {
            return Err(ApiError::InvalidCommand(
                "root geometry must be regenerated before admitting component meshes".into(),
            ));
        }
        if let Some(root) = root {
            for body in &root.bodies {
                let mesh = self.mesh_cache.peek(&body.mesh_key).ok_or_else(|| {
                    ApiError::InvalidCommand("root mesh unavailable for assembly admission".into())
                })?;
                total.add(mesh_budget(&mesh)?);
            }
        }
        total.validate()
    }

    /// Current checked definition-local mesh; stale definition data fails closed.
    pub fn component_mesh(
        &self,
        id: ComponentDefId,
        body: BodyId,
        generation: u64,
    ) -> Option<Arc<Vec<u8>>> {
        let def = self.session.document().definitions.get(&id)?;
        let cached = self.component_cache.get(&id)?;
        if cached.generation != generation || fingerprint(def).ok()? != cached.fingerprint {
            return None;
        }
        cached.meshes.get(&body).cloned()
    }

    /// Resolves a checked local mesh through an exact occurrence path and generation.
    pub fn component_mesh_for_path(
        &self,
        path: &OccurrencePath,
        body: BodyId,
        generation: u64,
    ) -> Option<Arc<Vec<u8>>> {
        let definition = self.component_definition_for_path(path)?;
        self.component_mesh(definition, body, generation)
    }

    pub(crate) fn component_definition_for_path(
        &self,
        path: &OccurrencePath,
    ) -> Option<ComponentDefId> {
        let id = *path.0.last()?;
        let assembly = &self.session.document().assembly;
        if assembly.occurrences.path(id).ok()?.0 != path.0 {
            return None;
        }
        Some(assembly.occurrences.get(id)?.def)
    }

    pub(crate) fn component_generation(&self, id: ComponentDefId) -> Option<u64> {
        let def = self.session.document().definitions.get(&id)?;
        let cached = self.component_cache.get(&id)?;
        (fingerprint(def).ok()? == cached.fingerprint).then_some(cached.generation)
    }

    pub(crate) fn validate_component_query(
        &self,
        output: &ComponentReplayOutput,
        epoch: WorkerEpoch,
        path: &OccurrencePath,
        body: BodyId,
        generation: u64,
    ) -> Result<SnapshotId, ApiError> {
        if !self.matches_runtime_session(&output.runtime_session)
            || self.revision() != output.revision
            || output.epoch != epoch
            || output.snapshot.0 == 0
        {
            return Err(ApiError::StalePreview(
                "component query replay is stale".into(),
            ));
        }
        let occurrence = path
            .0
            .last()
            .and_then(|id| self.session.document().assembly.occurrences.get(*id))
            .ok_or_else(|| ApiError::StalePreview("component occurrence is stale".into()))?;
        if occurrence.def != output.id
            || self
                .component_mesh_for_path(path, body, generation)
                .is_none()
        {
            return Err(ApiError::StalePreview(
                "component path or mesh is stale".into(),
            ));
        }
        let definition = self
            .session
            .document()
            .definitions
            .get(&output.id)
            .ok_or_else(|| ApiError::StalePreview("component definition is missing".into()))?;
        if fingerprint(definition)? != output.source_fingerprint {
            return Err(ApiError::StalePreview(
                "component definition changed".into(),
            ));
        }
        Ok(output.snapshot)
    }
}

fn mesh_budget(bytes: &[u8]) -> Result<MeshBudget, ApiError> {
    let limits = MeshLimits {
        max_blob_bytes: MAX_RESIDENT_BYTES,
        max_triangles: MAX_TRIANGLES as u32,
        max_edges: MAX_SEGMENTS as u32,
        ..MeshLimits::default()
    };
    let mesh = MeshView::decode(bytes, limits)
        .map_err(|error| ApiError::InvalidCommand(format!("invalid component mesh: {error}")))?;
    let mut segments = 0usize;
    for index in 0..mesh.header().edge_count {
        let range = mesh
            .edge_range(index as usize)
            .ok_or_else(|| ApiError::InvalidCommand("invalid component edge range".into()))?;
        segments = segments.saturating_add(range.count.saturating_sub(1) as usize);
    }
    let triangles = mesh.header().triangle_count as usize;
    // Conservative allowance for checked metadata, index primitives and nodes.
    let estimated = bytes
        .len()
        .saturating_add(16 * 1024)
        .saturating_add(triangles.saturating_add(segments).saturating_mul(256));
    Ok(MeshBudget {
        triangles,
        segments,
        bodies: 1,
        bytes: estimated,
    })
}

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
                return Err(ApiError::InvalidCommand(
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
    pub(crate) fn validate_rehydration(&self, other: &Self) -> Result<(), ApiError> {
        if self.meshes.len() != other.meshes.len() {
            return Err(ApiError::InvalidCommand(
                "component rehydration body mismatch".into(),
            ));
        }
        for (body, bytes) in &self.meshes {
            let other = other.meshes.get(body).ok_or_else(|| {
                ApiError::InvalidCommand("component rehydration body missing".into())
            })?;
            let a = MeshView::decode(bytes, MeshLimits::default())
                .map_err(|e| ApiError::InvalidCommand(e.to_string()))?;
            let b = MeshView::decode(other, MeshLimits::default())
                .map_err(|e| ApiError::InvalidCommand(e.to_string()))?;
            if a.header().face_count != b.header().face_count {
                return Err(ApiError::InvalidCommand(
                    "component rehydration face count mismatch".into(),
                ));
            }
            for i in 0..a.header().face_count as usize {
                let id = a
                    .face_id(i)
                    .ok_or_else(|| ApiError::InvalidCommand("missing face id".into()))?;
                if !id.starts_with("el_") || b.face_id(i) != Some(id) {
                    return Err(ApiError::InvalidCommand(
                        "component face identity cannot be verified".into(),
                    ));
                }
            }
        }
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use onecad_core::edit::AssemblyEdit;

    fn runtime() -> DocumentRuntime {
        let backend = Arc::new(crate::worker::PendingBackend);
        DocumentRuntime::new_blank(backend.clone(), backend.clone(), backend)
    }

    fn output(input: ComponentReplayInput, epoch: WorkerEpoch) -> ComponentReplayOutput {
        ComponentReplayOutput {
            id: input.id,
            runtime_session: input.runtime_session,
            revision: input.revision,
            epoch,
            source_fingerprint: input.source_fingerprint,
            definition: input.definition,
            meshes: BTreeMap::new(),
            budget: MeshBudget::default(),
            snapshot: SnapshotId(1),
            minted: false,
        }
    }

    #[test]
    fn stale_worker_epoch_and_same_uuid_replacement_refuse_replay() {
        let mut original = runtime();
        let id = ComponentDefId(Uuid::from_u128(7));
        original
            .apply_assembly_edit(AssemblyEdit::CreateDefinition {
                id,
                name: "Part".into(),
            })
            .unwrap();
        let input = original
            .prepare_component_replay(id, &original.runtime_session())
            .unwrap();
        assert!(original
            .commit_component_replay(output(input, WorkerEpoch(2)), WorkerEpoch(3))
            .is_err());
        assert!(original.component_cache.is_empty());
        let input = original
            .prepare_component_replay(id, &original.runtime_session())
            .unwrap();
        let backend = Arc::new(crate::worker::PendingBackend);
        let mut replacement = DocumentRuntime::from_document(
            original.session.document().clone(),
            "Replacement".into(),
            None,
            false,
            backend.clone(),
            backend.clone(),
            backend,
        );
        assert_eq!(replacement.document_uuid(), original.document_uuid());
        assert_ne!(replacement.runtime_session(), original.runtime_session());
        assert!(replacement
            .commit_component_replay(output(input, WorkerEpoch(2)), WorkerEpoch(2))
            .is_err());
        assert!(replacement.component_cache.is_empty());
    }

    #[test]
    fn root_publication_evicts_component_caches_before_new_scene() {
        let mut runtime = runtime();
        let id = ComponentDefId(Uuid::from_u128(123));
        runtime.component_cache.insert(
            id,
            ComponentCache {
                fingerprint: String::new(),
                generation: 1,
                meshes: BTreeMap::new(),
                budget: MeshBudget {
                    triangles: MAX_TRIANGLES,
                    ..MeshBudget::default()
                },
            },
        );
        let snapshot = Arc::new(ModelSnapshot {
            id: SnapshotId(1),
            generation: 1,
            step_index: None,
            bodies: Vec::new(),
            stopped_reason: onecad_core::regen::StoppedReason::Completed,
            step_states: Vec::new(),
            signatures: None,
            diagnostics: Vec::new(),
            diagnostics_by_step: BTreeMap::new(),
            repair_summary: Default::default(),
            sketch_placement_by_step: BTreeMap::new(),
        });
        runtime.commit_snapshot(runtime.clone_regen_session(), &snapshot, Lod::Coarse, &[]);
        assert!(runtime.component_cache.is_empty());
    }

    #[test]
    fn aggregate_budget_refusal_keeps_existing_component_cache() {
        let mut runtime = runtime();
        let existing = ComponentDefId(Uuid::from_u128(1));
        let incoming = ComponentDefId(Uuid::from_u128(2));
        runtime.component_cache.insert(
            existing,
            ComponentCache {
                fingerprint: String::new(),
                generation: 1,
                meshes: BTreeMap::new(),
                budget: MeshBudget {
                    triangles: 1,
                    segments: 1,
                    bodies: 1,
                    bytes: 1,
                },
            },
        );
        assert!(runtime
            .check_component_budget(
                incoming,
                MeshBudget {
                    triangles: MAX_TRIANGLES,
                    segments: 0,
                    bodies: 1,
                    bytes: 1,
                }
            )
            .is_err());
        for over in [
            MeshBudget {
                segments: MAX_SEGMENTS,
                ..MeshBudget::default()
            },
            MeshBudget {
                bodies: MAX_BODIES,
                ..MeshBudget::default()
            },
            MeshBudget {
                bytes: MAX_RESIDENT_BYTES,
                ..MeshBudget::default()
            },
        ] {
            assert!(runtime.check_component_budget(incoming, over).is_err());
        }
        assert!(runtime.component_cache.contains_key(&existing));
    }
}
