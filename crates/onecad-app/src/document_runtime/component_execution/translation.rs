//! Continue a verified native prefix without resolving old evidence on moved geometry.
use super::*;
use crate::worker::{ElementQuery, WorkerManager};
#[path = "translation_lifecycle.rs"]
mod lifecycle;
use onecad_core::document::element_index::ElementEntry;
use onecad_core::document::record::{KnownOperation, Operation};

pub(super) fn digest(value: &impl serde::Serialize) -> Option<[u8; 32]> {
    struct Writer(Sha256);
    impl std::io::Write for Writer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.update(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = Writer(Sha256::new());
    serde_json::to_writer(&mut writer, value).ok()?;
    Some(writer.0.finalize().into())
}

#[derive(Clone)]
pub(crate) struct TranslationBase {
    pub prefix: usize,
    pub signature: BTreeMap<BodyId, Vec<u8>>,
}

pub(super) fn replay_context(
    settings: &onecad_core::document::DocumentSettings,
    imports: &ImportBlobs,
) -> Option<[u8; 32]> {
    let mut hash = Sha256::new();
    hash.update(digest(settings)?);
    for (sha, blob) in imports {
        hash.update(digest(&(sha, blob.codec, blob.bytes.len()))?);
    }
    Some(hash.finalize().into())
}

fn context(def: &ComponentDef) -> Option<[u8; 32]> {
    digest(&(
        &def.id,
        &def.name,
        &def.variables,
        &def.sketches,
        &def.sketch_visibility,
        &def.datum_planes,
        &def.bodies,
        &def.elements,
        &def.repair,
        &def.named_selections,
        &def.extra,
    ))
}

fn translations(def: &ComponentDef, from: usize) -> Option<BTreeMap<BodyId, [f64; 3]>> {
    let mut offsets = BTreeMap::<BodyId, [f64; 3]>::new();
    for record in &def.timeline.records()[from..] {
        let Operation::Known(KnownOperation::TransformBody(params)) = &record.op else {
            return None;
        };
        if record.suppressed
            || !record.extra.is_empty()
            || params.copy
            || params.is_identity()
            || !params.extra.is_empty()
            || params.rotate.angle_deg.value != 0.0
            || params.rotate.angle_deg.expr.is_some()
            || params.validate().is_err()
            || params
                .translate
                .iter()
                .any(|s| s.expr.is_some() || !s.value.is_finite())
        {
            return None;
        }
        for body in &params.targets {
            let offset = offsets.entry(*body).or_default();
            for (value, scalar) in offset.iter_mut().zip(&params.translate) {
                *value += scalar.value;
                if !value.is_finite() {
                    return None;
                }
            }
        }
    }
    Some(offsets)
}

fn native_definition(def: &ComponentDef) -> bool {
    !def.elements
        .iter()
        .any(|(_, entry)| entry.import_source_identity.is_some())
        && !def.timeline.records().iter().any(|record| {
            matches!(
                record.op,
                Operation::Known(KnownOperation::ImportStep(_) | KnownOperation::PlaceComponent(_))
                    | Operation::Opaque(_)
            )
        })
}

impl DocumentRuntime {
    pub(super) fn component_translation_prefix(
        &self,
        id: ComponentDefId,
        def: &ComponentDef,
    ) -> Option<TranslationBase> {
        let cached = self.component_cache.get(&id)?;
        if cached.verified_head.as_ref() != Some(&self.joint_head())
            || replay_context(&self.session.document().settings, &self.imports)?
                != cached.replay_context
        {
            return None;
        }
        if def.elements.is_empty()
            || def.timeline.cursor() != def.timeline.len()
            || !native_definition(def)
        {
            return None;
        }
        let current_context = context(def)?;
        self.session
            .find_prior_component_definition(id, |prior| {
                let prefix = prior.timeline.len();
                if prefix == 0 || prefix >= def.timeline.len() || prior.timeline.cursor() != prefix
                {
                    return false;
                }
                let Ok(estimate) = preflight::component_clone(
                    prior,
                    &self.session.document().settings,
                    &self.imports,
                ) else {
                    return false;
                };
                let Ok(_retention) = self
                    .archive_budget
                    .reserve(estimate, crate::resource_admission::ArchiveUse::Scratch)
                else {
                    return false;
                };
                fingerprint(prior).is_ok_and(|hash| hash == cached.fingerprint)
                    && context(prior) == Some(current_context)
                    && digest(&prior.timeline.records()).is_some_and(|hash| {
                        digest(&&def.timeline.records()[..prefix]) == Some(hash)
                    })
                    && translations(def, prefix).is_some()
            })
            .map(|prior| TranslationBase {
                prefix: prior.timeline.len(),
                signature: cached.identity_signature.clone(),
            })
    }
}

fn invalid(message: &str) -> ApiError {
    ApiError::UnresolvedTopology(message.into())
}

fn ensure_valid(report: &RegenReport) -> Result<(), ApiError> {
    if !matches!(report.outcome, Outcome::Published(_))
        || !report.failed_steps.is_empty()
        || !report.needs_repair.is_empty()
    {
        return Err(invalid(
            "component identity continuation did not publish valid geometry",
        ));
    }
    Ok(())
}

async fn face_ids(
    runtime: &DocumentRuntime,
) -> Result<BTreeMap<BodyId, BTreeSet<String>>, ApiError> {
    let (meshes, _) = super::replay::collect_fresh_meshes(runtime).await?;
    let mut all = BTreeMap::new();
    for (body, bytes) in meshes {
        let mesh = MeshView::decode(&bytes, MeshLimits::default())
            .map_err(|e| ApiError::InvalidCommand(e.to_string()))?;
        let mut ids = BTreeSet::new();
        for row in 0..mesh.header().face_count as usize {
            let id = mesh
                .face_id(row)
                .filter(|id| id.starts_with("el_"))
                .ok_or_else(|| {
                    invalid("component continuation face lacks a persistent identity")
                })?;
            if !ids.insert(id.to_owned()) {
                return Err(invalid(
                    "component continuation has duplicate face identities",
                ));
            }
        }
        all.insert(body, ids);
    }
    Ok(all)
}

fn translated_anchor(
    entry: &ElementEntry,
    offset: [f64; 3],
) -> Result<Option<AnchorIntent>, ApiError> {
    let mut anchor = entry.anchor.clone();
    if let Some(anchor) = &mut anchor {
        anchor.world_point = translated_point(anchor.world_point, offset)?;
        if let Some(frame) = &mut anchor.local_frame {
            frame.origin = translated_point(frame.origin, offset)?;
        }
    }
    Ok(anchor)
}

fn translated_point(point: Vec3, offset: [f64; 3]) -> Result<Vec3, ApiError> {
    let point: [f64; 3] = point.into();
    Vec3::try_from(std::array::from_fn(|i| point[i] + offset[i]))
        .map_err(|_| invalid("component translated anchor is not finite"))
}

async fn refresh_entry(
    runtime: &mut DocumentRuntime,
    worker: &WorkerManager,
    snapshot: SnapshotId,
    id: &ElementId,
    entry: &ElementEntry,
    offset: [f64; 3],
) -> Result<(), ApiError> {
    let info = worker
        .query_element(snapshot, entry.body, id.as_str())
        .await
        .map_err(|e| ApiError::Worker(e.to_string()))?
        .ok_or_else(|| invalid("component transform lost a persisted identity"))?;
    if info.element_id != id.as_str()
        || info.kind != kind_str(entry.kind)
        || crate::worker::wire::parse_body_id(&info.body_id) != Ok(entry.body)
    {
        return Err(invalid(
            "component transform changed identity ownership or kind",
        ));
    }
    let key = TopoKey::new(&info.topo_key);
    if !key.is_valid() {
        return Err(invalid("component query returned an invalid topology key"));
    }
    let anchor = translated_anchor(entry, offset)?;
    let promoted = runtime
        .promote_selection(snapshot, entry.body, vec![(key, anchor.clone())])
        .await
        .map_err(|e| ApiError::Worker(e.to_string()))?;
    let after = runtime
        .regen
        .elements
        .get(id)
        .ok_or_else(|| invalid("component refreshed identity disappeared"))?;
    if promoted.len() != 1
        || promoted[0].element_id != id.as_str()
        || after.body != entry.body
        || after.kind != entry.kind
        || after.import_source_identity != entry.import_source_identity
        || after.anchor != anchor
        || after.descriptor.is_none()
    {
        return Err(invalid(
            "component evidence refresh changed identity or source intent",
        ));
    }
    Ok(())
}

struct BoundPrefix {
    faces: BTreeMap<BodyId, BTreeSet<String>>,
    bodies: onecad_core::document::body::BodyRegistry,
}

async fn replay_bound_prefix(
    runtime: &mut DocumentRuntime,
    base: &TranslationBase,
) -> Result<BoundPrefix, ApiError> {
    let prefix = base.prefix;
    runtime
        .session
        .apply(EditCommand::SetRollback { cursor: prefix })
        .map_err(ApiError::from)?;
    runtime.sync_regen_timeline(None);
    ensure_valid(
        &runtime
            .run_regen(RegenRequest::ToEnd { from: 0 }, CancelToken::new())
            .await,
    )?;
    if runtime.has_unbound_persisted_elements() {
        let (_, unresolved) = runtime.rebind_persisted_elements().await;
        if unresolved != 0 || runtime.has_unbound_persisted_elements() {
            return Err(invalid("component prefix identities could not be rebound"));
        }
    }
    let (prefix_meshes, _) = super::replay::collect_fresh_meshes(runtime).await?;
    if super::replay::mesh_signatures(&prefix_meshes)? != base.signature {
        return Err(invalid(
            "component rebound prefix differs from its verified identity mapping",
        ));
    }
    drop(prefix_meshes);
    Ok(BoundPrefix {
        faces: face_ids(runtime).await?,
        bodies: runtime.regen.bodies.clone(),
    })
}

async fn save_bound_checkpoint(
    input: &ComponentReplayInput,
    runtime: &mut DocumentRuntime,
    worker: &WorkerManager,
) -> Result<crate::resource_admission::Retention, ApiError> {
    let checkpoint_bytes = crate::worker::MAX_COMPONENT_CHECKPOINT_BYTES as usize;
    let retention = input
        .archive_budget
        .reserve(
            4 * (checkpoint_bytes + crate::worker::MAX_PREVIEW_RESPONSE_JSON_BYTES as usize),
            crate::resource_admission::ArchiveUse::Scratch,
        )
        .map_err(ApiError::Io)?;
    let ticket = runtime
        .prepare_checkpoint()
        .ok_or_else(|| invalid("component checkpoint is not current"))?;
    let artifacts = worker
        .save_component_checkpoint(ticket.step)
        .await
        .map_err(|e| ApiError::Worker(e.to_string()))?;
    runtime.adopt_checkpoint(ticket, artifacts);
    Ok(retention)
}

async fn refresh_translated_entries(
    input: &ComponentReplayInput,
    runtime: &mut DocumentRuntime,
    worker: &WorkerManager,
    offsets: &BTreeMap<BodyId, [f64; 3]>,
    before_faces: BTreeMap<BodyId, BTreeSet<String>>,
) -> Result<(), ApiError> {
    if face_ids(runtime).await? != before_faces {
        return Err(invalid(
            "component transform changed the live face identity set",
        ));
    }
    let snapshot = runtime
        .latest_snapshot
        .as_ref()
        .ok_or_else(|| invalid("missing transformed snapshot"))?
        .id;
    for (id, entry) in input.definition.elements.iter() {
        let actual = runtime
            .regen
            .elements
            .get(id)
            .ok_or_else(|| invalid("component transform lost a partition entry"))?;
        if actual.body != entry.body
            || actual.kind != entry.kind
            || actual.import_source_identity != entry.import_source_identity
        {
            return Err(invalid("component transform changed a partition entry"));
        }
        refresh_entry(
            runtime,
            worker,
            snapshot,
            id,
            entry,
            offsets.get(&entry.body).copied().unwrap_or_default(),
        )
        .await?;
    }
    if runtime.regen.elements.len() != input.definition.elements.len() {
        return Err(invalid("component transform changed partition size"));
    }
    Ok(())
}

pub(super) async fn continue_translation(
    input: &ComponentReplayInput,
    runtime: &mut DocumentRuntime,
    worker: &WorkerManager,
    base: &TranslationBase,
) -> Result<crate::resource_admission::Retention, ApiError> {
    if input.read_only {
        return Err(invalid(
            "read-only component cannot refresh translated evidence",
        ));
    }
    let offsets = translations(&input.definition, base.prefix)
        .ok_or_else(|| invalid("unsupported component identity continuation"))?;
    let proven = replay_bound_prefix(runtime, base).await?;
    let retention = save_bound_checkpoint(input, runtime, worker).await?;
    runtime
        .session
        .apply(EditCommand::SetRollback {
            cursor: input.definition.timeline.cursor(),
        })
        .map_err(ApiError::from)?;
    runtime.sync_regen_timeline(Some(base.prefix));
    let report = runtime
        .run_regen_requiring_checkpoint(
            RegenRequest::ToEnd { from: base.prefix },
            CancelToken::new(),
        )
        .await;
    ensure_valid(&report)?;
    lifecycle::restore(runtime, &input.definition, base.prefix, proven.bodies)?;
    refresh_translated_entries(input, runtime, worker, &offsets, proven.faces).await?;
    Ok(retention)
}

#[cfg(test)]
#[path = "translation_tests.rs"]
mod tests;
