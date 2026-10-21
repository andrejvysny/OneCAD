use super::*;
use onecad_core::regen::WorkerElementEvidence;

/// Scoped ownership of an unlocked face-entry query. Dropping it changes nothing.
pub struct FaceEntryTicket {
    instance: Uuid,
    revision: DocumentRevision,
    snapshot: SnapshotId,
    epoch: WorkerEpoch,
    pub(crate) engine: Arc<dyn GeometryEngine>,
}

impl DocumentRuntime {
    pub fn prepare_face_entry(
        &self,
        session: &str,
        revision: DocumentRevision,
        snapshot: SnapshotId,
        epoch: WorkerEpoch,
    ) -> Result<FaceEntryTicket, EngineError> {
        self.gate_stale_pick(snapshot)?;
        if !self.matches_runtime_session(session)
            || self.revision() != revision
            || self.head_epoch != epoch
        {
            return Err(face_entry_stale());
        }
        Ok(FaceEntryTicket {
            instance: self.instance,
            revision,
            snapshot,
            epoch: self.head_epoch,
            engine: self.engine.clone(),
        })
    }

    pub fn face_entry_is_current(&self, ticket: &FaceEntryTicket) -> bool {
        self.instance == ticket.instance
            && self.revision() == ticket.revision
            && self.head_snapshot_id() == Some(ticket.snapshot)
            && self.head_epoch == ticket.epoch
            && self.fencing.get().1 == ticket.epoch
            && self.engine.current_epoch() == ticket.epoch
            && Arc::ptr_eq(&self.engine, &ticket.engine)
    }

    pub(crate) fn face_entry_identity(
        &self,
        ticket: &FaceEntryTicket,
        mut evidence: WorkerElementEvidence,
    ) -> Result<(ElementId, WorkerElementEvidence), EngineError> {
        if !self.face_entry_is_current(ticket) {
            return Err(face_entry_stale());
        }
        if let Some(hit) =
            self.promoted
                .get(&(ticket.snapshot, evidence.body, evidence.topo_key.clone()))
        {
            evidence.existing = Some(hit.element_id.clone());
        }
        if evidence.existing.is_none() {
            let previous = self
                .previous_promotion_generation(ticket.snapshot)
                .and_then(|previous| {
                    self.promoted
                        .get(&(previous, evidence.body, evidence.topo_key.clone()))
                })
                .filter(|hit| {
                    hit.kind == evidence.kind
                        && hit.descriptor.is_some()
                        && hit.descriptor == evidence.descriptor
                });
            if let Some(hit) = previous {
                evidence.existing = Some(hit.element_id.clone());
            }
        }
        mint_element_ids(vec![evidence])
            .pop()
            .ok_or_else(face_entry_stale)
    }

    pub(crate) fn commit_face_entry(
        &mut self,
        ticket: &FaceEntryTicket,
        sketch: Sketch,
        identity: Option<(ElementId, WorkerElementEvidence)>,
    ) -> Result<CommandOutcome, crate::error::ApiError> {
        if !self.face_entry_is_current(ticket) {
            return Err(face_entry_stale().into());
        }
        let outcome = self.apply(EditCommand::AddSketch { sketch })?;
        if let Some((id, evidence)) = identity {
            // Reuse the existing fenced rebind lane at the next published regen.
            // Preparation never installs worker bindings on cancellation.
            self.rebind_attempted = false;
            self.regen.elements.insert(
                id.clone(),
                ElementEntry::new(evidence.body, evidence.kind)
                    .with_evidence(evidence.anchor, evidence.descriptor.clone()),
            );
            self.promoted.insert(
                (ticket.snapshot, evidence.body, evidence.topo_key),
                PromotionEntry {
                    element_id: id,
                    kind: evidence.kind,
                    descriptor: evidence.descriptor,
                },
            );
        }
        Ok(outcome)
    }
}

fn face_entry_stale() -> EngineError {
    EngineError::OpFailed {
        code: onecad_core::regen::OpFailureCode::RefUnresolved,
        recoverable: true,
        message: "Sketch support changed during entry; select the planar face again".into(),
        diagnostics: Vec::new(),
    }
}
