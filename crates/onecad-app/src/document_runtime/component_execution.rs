//! Fenced, bounded replay for one embedded definition on an auxiliary worker.
use super::*;
use onecad_core::document::component::{ComponentDef, ComponentDefId, OccurrencePath};
use onecad_protocol::mesh::{MeshLimits, MeshView};
mod preflight;

const MAX_TRIANGLES: usize = 500_000;
const MAX_SEGMENTS: usize = 500_000;
const MAX_BODIES: usize = 4096;
const MAX_RESIDENT_BYTES: usize = 256 * 1024 * 1024;

fn check_joint_evidence_size(value: &impl serde::Serialize) -> Result<(), ApiError> {
    struct Budget(usize);
    impl std::io::Write for Budget {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self
                .0
                .checked_add(bytes.len())
                .ok_or_else(|| std::io::Error::other("joint evidence too large"))?;
            if self.0 > 64 * 1024 {
                return Err(std::io::Error::other("joint evidence too large"));
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(Budget(0), value)
        .map_err(|_| ApiError::InvalidCommand("joint origin evidence exceeds 64 KiB".into()))
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct MeshBudget {
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
    verified_head: Option<super::joint_execution::JointHead>,
    pub(super) meshes: BTreeMap<BodyId, Arc<Vec<u8>>>,
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
    pub(crate) _retention: crate::resource_admission::Retention,
}

pub(crate) struct ComponentReplayOutput {
    id: ComponentDefId,
    runtime_session: String,
    revision: DocumentRevision,
    pub epoch: WorkerEpoch,
    source_fingerprint: String,
    pub definition: ComponentDef,
    pub(crate) meshes: BTreeMap<BodyId, Arc<Vec<u8>>>,
    pub(crate) budget: MeshBudget,
    pub(crate) snapshot: SnapshotId,
    pub minted: bool,
    pub(crate) _retention: crate::resource_admission::Retention,
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
        let estimate = preflight::component_clone(def, &doc.settings, &self.imports)?;
        // Replay, verification and the owned working document can overlap.
        let retained = estimate.checked_mul(4).ok_or_else(|| {
            ApiError::InvalidCommand("component replay admission overflow".into())
        })?;
        let retention = self
            .archive_budget
            .reserve(retained, crate::resource_admission::ArchiveUse::Scratch)
            .map_err(ApiError::Io)?;
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
            _retention: retention,
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
        let new_fingerprint = fingerprint(&output.definition)?;
        let persisted_changed = new_fingerprint != output.source_fingerprint;
        if persisted_changed {
            self.require_generation_capacity()?;
            if self.read_only {
                return Err(ApiError::InvalidCommand(
                    "read-only component replay would change persisted state".into(),
                ));
            }
        }
        if !self
            .session
            .adopt_component_replay(output.id, output.definition)
        {
            return Err(ApiError::InvalidCommand(
                "component definition was removed".into(),
            ));
        }
        if persisted_changed {
            self.persisted_generation += 1;
            self.dirty = true;
        }
        self.component_generation = generation;
        self.component_cache.insert(
            output.id,
            ComponentCache {
                fingerprint: new_fingerprint,
                generation,
                verified_head: None,
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
        self.component_mesh_checked(id, body, generation, false)
    }

    fn component_mesh_checked(
        &self,
        id: ComponentDefId,
        body: BodyId,
        generation: u64,
        allow_pending: bool,
    ) -> Option<Arc<Vec<u8>>> {
        let def = self.session.document().definitions.get(&id)?;
        let cached = self.component_cache.get(&id)?;
        if (!allow_pending && cached.verified_head.as_ref() != Some(&self.joint_head()))
            || cached.generation != generation
            || fingerprint(def).ok()? != cached.fingerprint
        {
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

    pub(crate) fn component_mesh_for_validation(
        &self,
        path: &OccurrencePath,
        body: BodyId,
        generation: u64,
    ) -> Option<Arc<Vec<u8>>> {
        let definition = self.component_definition_for_path(path)?;
        self.component_mesh_checked(definition, body, generation, true)
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

    pub(crate) fn joint_face_reference(
        &self,
        path: &OccurrencePath,
        body: BodyId,
        element: &str,
    ) -> Result<(ComponentDefId, onecad_core::document::refs::ElementRef), ApiError> {
        use onecad_core::document::refs::{ElementKind, ElementRef, IntentQuery, PrimaryRef};
        let id = self
            .component_definition_for_path(path)
            .ok_or_else(|| ApiError::StalePreview("joint occurrence path is stale".into()))?;
        let element_id = onecad_core::ids::ElementId::new(element.to_owned());
        let entry = self
            .session
            .document()
            .definitions
            .get(&id)
            .and_then(|definition| definition.elements.get(&element_id))
            .filter(|entry| entry.body == body && entry.kind == ElementKind::Face)
            .ok_or_else(|| {
                ApiError::UnresolvedTopology("joint face identity is unavailable".into())
            })?;
        check_joint_evidence_size(entry)?;
        let anchor = entry
            .anchor
            .clone()
            .ok_or_else(|| ApiError::UnresolvedTopology("joint face lacks an anchor".into()))?;
        let descriptor = entry.descriptor.clone().ok_or_else(|| {
            ApiError::UnresolvedTopology("joint face lacks rebind evidence".into())
        })?;
        Ok((
            id,
            ElementRef {
                import_source_identity: entry.import_source_identity.clone(),
                primary: Some(PrimaryRef {
                    body,
                    element: element_id,
                    kind: ElementKind::Face,
                    extra: Default::default(),
                }),
                intent: Some(IntentQuery {
                    version: PolicyVersions::default().descriptor,
                    kind: ElementKind::Face,
                    descriptor,
                    extra: Default::default(),
                }),
                anchor: Some(anchor),
                extra: Default::default(),
            },
        ))
    }

    pub(crate) fn joint_bindings(
        &self,
    ) -> Result<Vec<onecad_core::document::component::JointBinding>, ApiError> {
        let bindings = &self.session.document().assembly.joint_bindings;
        if bindings.len() > 40 {
            return Err(ApiError::InvalidCommand(
                "too many joint origin bindings".into(),
            ));
        }
        for binding in bindings {
            check_joint_evidence_size(binding)?;
        }
        Ok(bindings.clone())
    }

    pub(crate) fn component_generation(&self, id: ComponentDefId) -> Option<u64> {
        let def = self.session.document().definitions.get(&id)?;
        let cached = self.component_cache.get(&id)?;
        (cached.verified_head.as_ref() == Some(&self.joint_head())
            && fingerprint(def).ok()? == cached.fingerprint)
            .then_some(cached.generation)
    }

    pub(crate) fn finish_component_validation(
        &mut self,
        session: &str,
        id: ComponentDefId,
        generation: u64,
        persisted: u64,
        head: &super::joint_execution::JointHead,
    ) -> Result<(), ApiError> {
        if !self.matches_runtime_session(session)
            || self.persisted_generation() != persisted
            || &self.joint_head() != head
        {
            return Err(ApiError::StalePreview(
                "component validation is stale".into(),
            ));
        }
        let definition = self
            .session
            .document()
            .definitions
            .get(&id)
            .ok_or_else(|| ApiError::StalePreview("component definition is missing".into()))?;
        let current_fingerprint = fingerprint(definition)?;
        let cached = self
            .component_cache
            .get_mut(&id)
            .filter(|cache| {
                cache.generation == generation && cache.fingerprint == current_fingerprint
            })
            .ok_or_else(|| {
                ApiError::StalePreview("component cache changed during validation".into())
            })?;
        cached.verified_head = Some(head.clone());
        Ok(())
    }

    pub(crate) fn validate_component_query(
        &self,
        output: &ComponentReplayOutput,
        epoch: WorkerEpoch,
        path: &OccurrencePath,
        body: BodyId,
        generation: u64,
        allow_pending: bool,
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
            || (if allow_pending {
                self.component_mesh_for_validation(path, body, generation)
            } else {
                self.component_mesh_for_path(path, body, generation)
            })
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

mod replay;

#[cfg(test)]
mod tests;
