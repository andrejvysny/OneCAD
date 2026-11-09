//! Retain unresolved identities only after proving the entire consumer set inactive.
use super::*;
use onecad_core::document::element_index::ElementBindingState;
#[cfg(test)]
mod tests;

#[derive(Clone)]
pub(super) struct QuarantineInput {
    document: Box<Document>,
    generation: u64,
    retention: crate::resource_admission::Retention,
}

pub(super) struct QuarantineCommit {
    generation: u64,
    _retention: crate::resource_admission::Retention,
}

fn inactive(runtime: &DocumentRuntime) -> bool {
    runtime.staged.is_empty()
        && runtime.active_gesture.is_none()
        && runtime.native_gesture.is_none()
        && runtime.sketch_session.is_none()
        && !runtime.session.transaction_open()
}

impl DocumentRuntime {
    pub(super) fn prepare_component_quarantine(
        &self,
        id: ComponentDefId,
    ) -> Result<Option<QuarantineInput>, ApiError> {
        if self.read_only
            || !inactive(self)
            || !self
                .session
                .document()
                .assembly
                .joint_bindings
                .iter()
                .any(|binding| binding.definition == id && binding.needs_repair)
        {
            return Ok(None);
        }
        let size = preflight::runtime_clone(self)?
            .checked_mul(4)
            .ok_or_else(|| ApiError::InvalidCommand("quarantine admission overflow".into()))?;
        let retention = self
            .archive_budget
            .reserve(size, crate::resource_admission::ArchiveUse::Scratch)
            .map_err(ApiError::Io)?;
        Ok(Some(QuarantineInput {
            document: Box::new(self.document_for_persistence()),
            generation: self.persisted_generation(),
            retention,
        }))
    }

    pub(super) fn check_quarantine_commit(&self, proof: &QuarantineCommit) -> Result<(), ApiError> {
        if self.read_only || self.persisted_generation() != proof.generation || !inactive(self) {
            return Err(ApiError::StalePreview(
                "topology repair consumer set changed during replay".into(),
            ));
        }
        Ok(())
    }
}

impl QuarantineInput {
    pub(super) fn apply(
        mut self,
        id: ComponentDefId,
        runtime: &mut DocumentRuntime,
        rows: &[RefResolution],
    ) -> Result<QuarantineCommit, ApiError> {
        let candidate = self.document.definitions.get_mut(&id).ok_or_else(|| {
            ApiError::UnresolvedTopology("repair definition is unavailable".into())
        })?;
        if rows.is_empty()
            || candidate.elements.iter().any(|(_, entry)| {
                entry.has_rebind_evidence() && !runtime.regen.bodies.contains(entry.body)
            })
        {
            return Err(ApiError::UnresolvedTopology(
                "quarantine requires complete surviving-body identity answers".into(),
            ));
        }
        for row in rows {
            if !matches!(row.outcome, ResolveOutcome::NeedsRepair(_)) {
                return Err(ApiError::UnresolvedTopology(
                    "worker did not return explicit NeedsRepair".into(),
                ));
            }
            let key = ElementId::new(&row.ref_id);
            let old = candidate
                .elements
                .get(&key)
                .filter(|entry| {
                    entry.kind == ElementKind::Face
                        && entry.import_source_identity.is_none()
                        && entry.has_rebind_evidence()
                        && runtime.regen.bodies.get(entry.body).is_some_and(|body| {
                            matches!(
                                body.health,
                                onecad_core::document::body::BodyHealth::Healthy
                            )
                        })
                        && runtime.regen.elements.get(&key) == Some(*entry)
                })
                .ok_or_else(|| {
                    ApiError::UnresolvedTopology("repair identity evidence changed".into())
                })?;
            let mut reserved = old.clone();
            reserved.binding_state = Some(ElementBindingState::Quarantined);
            candidate.elements.insert(key, reserved);
        }
        self.document
            .validate_topology_binding_states()
            .map_err(ApiError::UnresolvedTopology)?;
        for row in rows {
            let key = ElementId::new(&row.ref_id);
            runtime.regen.elements.insert(
                key.clone(),
                self.document.definitions[&id]
                    .elements
                    .get(&key)
                    .unwrap()
                    .clone(),
            );
        }
        Ok(QuarantineCommit {
            generation: self.generation,
            _retention: self.retention,
        })
    }
}
