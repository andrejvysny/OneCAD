//! A reserved identity cannot be rehydrated by caches, worker echoes or staging.
use super::*;
use onecad_core::regen::{OpFailureCode, WorkerElementEvidence};

impl DocumentRuntime {
    pub(super) fn reject_quarantined_mesh(
        &self,
        bytes: &[u8],
    ) -> Result<(), crate::error::ApiError> {
        use onecad_protocol::mesh::{MeshLimits, MeshView};
        if !self.regen.elements.has_quarantined() {
            return Ok(());
        }
        let mesh = MeshView::decode(bytes, MeshLimits::default())
            .map_err(|error| crate::error::ApiError::UnresolvedTopology(error.to_string()))?;
        let faces = (0..mesh.header().face_count as usize).filter_map(|index| mesh.face_id(index));
        let edges = (0..mesh.header().edge_count as usize).filter_map(|index| mesh.edge_id(index));
        for id in faces.chain(edges) {
            self.ensure_element_not_quarantined(&ElementId::new(id))?;
        }
        Ok(())
    }

    pub(super) fn ensure_element_not_quarantined(&self, id: &ElementId) -> Result<(), EngineError> {
        if self
            .regen
            .elements
            .get(id)
            .is_some_and(ElementEntry::is_quarantined)
            || self
                .staged
                .get(id)
                .is_some_and(ElementEntry::is_quarantined)
        {
            return Err(EngineError::OpFailed {
                code: OpFailureCode::RefUnresolved,
                recoverable: true,
                message: "topology identity requires explicit repair; choose a current face".into(),
                diagnostics: Vec::new(),
            });
        }
        Ok(())
    }

    pub(super) fn ensure_promotions_not_quarantined(
        &self,
        minted: &[(ElementId, WorkerElementEvidence)],
    ) -> Result<(), EngineError> {
        for (id, _) in minted {
            self.ensure_element_not_quarantined(id)?;
        }
        Ok(())
    }

    pub(super) fn quarantined_resolution(
        &self,
        snapshot: SnapshotId,
        request: &ResolveRef,
    ) -> Result<Option<RefResolution>, EngineError> {
        let hydrated = self.stored_input_ref(&request.ref_id);
        let reference = if element_ref_is_empty(&request.element) {
            hydrated.as_ref().unwrap_or(&request.element)
        } else {
            &request.element
        };
        let fallback = ElementId::new(request.ref_id.clone());
        let id = reference
            .primary
            .as_ref()
            .map_or(&fallback, |primary| &primary.element);
        let Some(entry) = self
            .regen
            .elements
            .get(id)
            .filter(|entry| entry.is_quarantined())
        else {
            return Ok(None);
        };
        if reference
            .primary
            .as_ref()
            .is_some_and(|primary| primary.body != entry.body || primary.kind != entry.kind)
        {
            self.ensure_element_not_quarantined(id)?;
        }
        Ok(Some(RefResolution {
            ref_id: request.ref_id.clone(),
            outcome: ResolveOutcome::NeedsRepair(quarantine_item(&request.ref_id, id, entry)),
            snapshot_id: snapshot,
            revision: self.projection().revision,
            body_id: Some(entry.body),
        }))
    }
}

fn quarantine_item(ref_id: &str, id: &ElementId, entry: &ElementEntry) -> RepairItem {
    use onecad_core::document::repair::{LadderLevel, RepairReason};
    RepairItem {
        step_index: 0,
        ref_id: ref_id.into(),
        element_id: Some(id.clone()),
        ladder_failed: LadderLevel::Descriptor,
        reason: RepairReason::NoCandidates,
        candidates: Vec::new(),
        scoring_version: None,
        anchor: entry.anchor.clone(),
        ui_label: "Source face requires explicit repair; choose a current face".into(),
        seeded: false,
        ordinal_anchor: None,
        seed_edge_id: None,
        resolved_axis: None,
        frozen_axis: None,
        resolved_sidedness: None,
    }
}
