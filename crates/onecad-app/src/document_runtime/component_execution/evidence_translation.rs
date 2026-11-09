//! Transport native evidence for a verified replacement of one trailing translation.
use super::*;
use onecad_core::{
    document::{
        element_index::ElementIndex,
        record::{KnownOperation, Operation, OperationRecord, TransformBodyParams},
    },
    ids::RecordId,
};
mod descriptor;

#[derive(Clone)]
pub(crate) struct EvidenceTranslation {
    generation: u64,
    persisted: u64,
    fingerprint: String,
    head: super::super::joint_execution::JointHead,
    engine_epoch: WorkerEpoch,
    context: [u8; 32],
    source_elements: [u8; 32],
    offsets: BTreeMap<BodyId, [f64; 3]>,
}

fn invalid(message: &str) -> ApiError {
    ApiError::UnresolvedTopology(message.into())
}

fn pure_translation(record: &OperationRecord) -> Option<&TransformBodyParams> {
    let Operation::Known(KnownOperation::TransformBody(params)) = &record.op else {
        return None;
    };
    (!record.suppressed
        && record.extra.is_empty()
        && params.extra.is_empty()
        && !params.copy
        && params.rotate.angle_deg.value == 0.0
        && params.rotate.angle_deg.expr.is_none()
        && params
            .translate
            .iter()
            .all(|value| value.expr.is_none() && value.value.is_finite())
        && params.validate().is_ok())
    .then_some(params)
}

fn offsets(prior: &ComponentDef, current: &ComponentDef) -> Option<BTreeMap<BodyId, [f64; 3]>> {
    let before = prior.timeline.records();
    let after = current.timeline.records();
    if before.len() < 2
        || before.len() != after.len()
        || prior.timeline.cursor() != before.len()
        || current.timeline.cursor() != after.len()
        || translation::digest(&&before[..before.len() - 1])?
            != translation::digest(&&after[..after.len() - 1])?
    {
        return None;
    }
    let old = pure_translation(before.last()?)?;
    let new = pure_translation(after.last()?)?;
    let mut normalized = before.last()?.clone();
    let Operation::Known(KnownOperation::TransformBody(params)) = &mut normalized.op else {
        return None;
    };
    params.translate = new.translate.clone();
    if translation::digest(&normalized)? != translation::digest(after.last()?)? {
        return None;
    }
    let delta = std::array::from_fn(|axis| new.translate[axis].value - old.translate[axis].value);
    if delta.iter().any(|value| !value.is_finite())
        || delta == [0.0; 3]
        || old.targets.iter().any(|body| !prior.bodies.contains(*body))
    {
        return None;
    }
    Some(old.targets.iter().map(|body| (*body, delta)).collect())
}

impl DocumentRuntime {
    pub(super) fn component_evidence_translation(
        &self,
        id: ComponentDefId,
        definition: &ComponentDef,
    ) -> Option<EvidenceTranslation> {
        let cached = self.component_cache.get(&id)?;
        let replay_context =
            translation::replay_context(&self.session.document().settings, &self.imports)?;
        if self.read_only
            || definition.elements.is_empty()
            || definition.elements.has_quarantined()
            || !translation::native_definition(definition)
            || definition.repair.first_seeded_step().is_some()
            || cached.verified_head.as_ref() != Some(&self.joint_head())
            || cached.replay_context != replay_context
            || self.engine.current_epoch() != self.fencing.get().1
        {
            return None;
        }
        let context = translation::context(definition)?;
        let mut change = None;
        self.session.find_prior_component_definition(id, |prior| {
            change = self.admit_prior_offsets(prior, definition, cached, context);
            change.is_some()
        })?;
        let proof = EvidenceTranslation {
            generation: cached.generation,
            persisted: self.persisted_generation(),
            fingerprint: cached.fingerprint.clone(),
            head: self.joint_head(),
            engine_epoch: self.engine.current_epoch(),
            context: replay_context,
            source_elements: translation::digest(&definition.elements)?,
            offsets: change?,
        };
        proof.translated_elements(&definition.elements).ok()?;
        Some(proof)
    }

    fn admit_prior_offsets(
        &self,
        prior: &ComponentDef,
        definition: &ComponentDef,
        cached: &ComponentCache,
        context: [u8; 32],
    ) -> Option<BTreeMap<BodyId, [f64; 3]>> {
        let estimate =
            preflight::component_clone(prior, &self.session.document().settings, &self.imports)
                .ok()?;
        let _retention = self
            .archive_budget
            .reserve(estimate, crate::resource_admission::ArchiveUse::Scratch)
            .ok()?;
        if fingerprint(prior).ok().as_ref() != Some(&cached.fingerprint)
            || translation::context(prior) != Some(context)
        {
            return None;
        }
        offsets(prior, definition)
    }

    pub(super) fn validate_evidence_translation(
        &self,
        id: ComponentDefId,
        proof: &EvidenceTranslation,
    ) -> Result<(), ApiError> {
        let cached = self
            .component_cache
            .get(&id)
            .ok_or_else(|| invalid("translation evidence cache unavailable"))?;
        if self.persisted_generation() != proof.persisted
            || self.joint_head() != proof.head
            || self.engine.current_epoch() != proof.engine_epoch
            || cached.generation != proof.generation
            || cached.fingerprint != proof.fingerprint
            || cached.verified_head.as_ref() != Some(&proof.head)
            || cached.replay_context != proof.context
            || translation::replay_context(&self.session.document().settings, &self.imports)
                != Some(proof.context)
        {
            return Err(ApiError::StalePreview(
                "translation evidence capture changed".into(),
            ));
        }
        Ok(())
    }
}

impl EvidenceTranslation {
    pub(super) fn validate_inventory(
        &self,
        source: &ElementIndex,
        actual: &ElementIndex,
    ) -> Result<(), ApiError> {
        if translation::digest(source) != Some(self.source_elements)
            || source.len() != actual.len()
            || source.iter().any(|(id, entry)| {
                actual.get(id).is_none_or(|next| {
                    next.body != entry.body
                        || next.kind != entry.kind
                        || next.binding_state != entry.binding_state
                        || next.import_source_identity != entry.import_source_identity
                })
            })
        {
            return Err(invalid(
                "translation evidence changed identity inventory or authority",
            ));
        }
        Ok(())
    }

    fn translated_elements(&self, source: &ElementIndex) -> Result<ElementIndex, ApiError> {
        if translation::digest(source) != Some(self.source_elements) {
            return Err(invalid("translation evidence source changed"));
        }
        let mut result = source.clone();
        for (id, entry) in source.iter() {
            if entry.is_quarantined() || entry.import_source_identity.is_some() {
                return Err(invalid("translation evidence requires native identities"));
            }
            if let Some(delta) = self.offsets.get(&entry.body) {
                if entry.anchor.as_ref().is_some_and(|anchor| {
                    !anchor.extra.is_empty()
                        || anchor
                            .local_frame
                            .as_ref()
                            .is_some_and(|frame| !frame.extra.is_empty())
                }) {
                    return Err(invalid("unsupported native translation anchor extension"));
                }
                let mut changed = entry.clone();
                changed.anchor = translation::translated_anchor(entry, *delta)?;
                changed.descriptor = Some(descriptor::translated(
                    entry
                        .descriptor
                        .as_ref()
                        .ok_or_else(|| invalid("translation evidence descriptor missing"))?,
                    entry.kind,
                    *delta,
                )?);
                result.insert(id.clone(), changed);
            }
        }
        Ok(result)
    }

    pub(super) fn apply(&self, runtime: &mut DocumentRuntime) -> Result<(), ApiError> {
        runtime.regen.elements = self.translated_elements(&runtime.regen.elements)?;
        Ok(())
    }

    pub(super) async fn refresh(
        &self,
        runtime: &mut DocumentRuntime,
        worker: &crate::worker::WorkerManager,
    ) -> Result<(), ApiError> {
        let snapshot = runtime
            .head_snapshot_id()
            .ok_or_else(|| invalid("translation evidence replay head missing"))?;
        let entries: Vec<_> = runtime
            .regen
            .elements
            .iter()
            .filter(|(_, entry)| self.offsets.contains_key(&entry.body))
            .map(|(id, entry)| (id.clone(), entry.clone()))
            .collect();
        let count = runtime.regen.elements.len();
        for (id, entry) in entries {
            translation::refresh_entry(runtime, worker, snapshot, &id, &entry, [0.0; 3]).await?;
        }
        if runtime.regen.elements.len() != count {
            return Err(invalid(
                "translation evidence refresh changed identity count",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
