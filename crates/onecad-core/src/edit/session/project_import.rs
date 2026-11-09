//! Atomic project imports retain the source's definition-local identity namespace.
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::{
    empty_outcome, merge_outcome, AppliedEdit, CommandOutcome, DocumentSession, EditCommand,
    HistoryDirection, Inverse,
};
use crate::{
    document::{
        element_index::{ElementEntry, ElementIndex},
        record::{KnownOperation, Operation, OperationRecord},
    },
    error::DomainError,
    ids::{ElementId, SketchId},
    sketch::Sketch,
};

/// Immutable, validated source authority prepared by application import admission.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectImportEdit {
    pub records: Vec<OperationRecord>,
    pub sketches: BTreeMap<SketchId, Sketch>,
    pub elements: ElementIndex,
}

impl ProjectImportEdit {
    /// Checks current identity ownership without publishing pending evidence.
    pub fn capture_elements<'a>(
        &self,
        entries: impl Iterator<Item = (&'a ElementId, &'a ElementEntry)>,
        bodies: &crate::document::body::BodyRegistry,
        target: &ElementIndex,
    ) -> Result<ElementIndex, DomainError> {
        let producers: BTreeSet<_> = self.records.iter().map(|record| record.record_id).collect();
        let source_bodies: BTreeSet<_> = self
            .elements
            .iter()
            .map(|(_, entry)| entry.body)
            .chain(
                self.records
                    .iter()
                    .flat_map(|record| record.outputs.iter().copied()),
            )
            .collect();
        let mut captured = self.elements.clone();
        for (id, entry) in entries {
            let body_producer = bodies.get(entry.body).map(|body| body.created_by);
            if self.validate_capture_owner(
                id,
                entry,
                body_producer,
                target,
                &producers,
                source_bodies.contains(&entry.body),
            )? {
                captured.insert(id.clone(), entry.clone());
            }
        }
        validate_unique_certificates(&captured)?;
        Ok(captured)
    }

    fn validate_capture_owner(
        &self,
        id: &ElementId,
        entry: &ElementEntry,
        body_producer: Option<crate::ids::RecordId>,
        target: &ElementIndex,
        producers: &BTreeSet<crate::ids::RecordId>,
        source_body: bool,
    ) -> Result<bool, DomainError> {
        let certified_producer = entry
            .import_source_identity
            .as_ref()
            .map(|cert| cert.producer_record_id);
        let associated = source_body || body_producer.is_some_and(|id| producers.contains(&id));
        if !associated && certified_producer.is_some_and(|id| producers.contains(&id)) {
            return Err(DomainError::Validation(
                "import history body ownership unavailable".into(),
            ));
        }
        if !associated {
            return Ok(false);
        }
        self.validate_entry(entry)?;
        if (target.contains(id) && !self.elements.contains(id))
            || body_producer.is_some_and(|id| !producers.contains(&id))
            || body_producer
                .zip(certified_producer)
                .is_some_and(|(body, cert)| body != cert)
            || self.elements.get(id).is_some_and(|prior| {
                prior.body != entry.body
                    || prior.kind != entry.kind
                    || prior.import_source_identity != entry.import_source_identity
            })
        {
            return Err(DomainError::Validation(
                "import history identity ownership conflict".into(),
            ));
        }
        Ok(true)
    }

    /// Certificates must name the exact imported producer, codec and scale.
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.records.is_empty() {
            return Err(DomainError::Validation(
                "project import has no source records".into(),
            ));
        }
        for (_, entry) in self.elements.iter() {
            self.validate_entry(entry)?;
        }
        for sketch in self.sketches.values() {
            if let Some(reference) = sketch.host_face() {
                self.validate_reference(reference)?;
            }
        }
        for record in &self.records {
            if let Operation::Known(mut operation) = record.op.clone() {
                for reference in operation.stored_element_refs_mut() {
                    self.validate_reference(reference)?;
                }
            }
        }
        Ok(())
    }

    fn validate_entry(
        &self,
        entry: &crate::document::element_index::ElementEntry,
    ) -> Result<(), DomainError> {
        let fail =
            || DomainError::Validation("import identity does not match source producer".into());
        let Some(cert) = &entry.import_source_identity else {
            return Ok(());
        };
        cert.validate().map_err(|_| fail())?;
        let record = self
            .records
            .iter()
            .find(|row| row.record_id == cert.producer_record_id)
            .ok_or_else(fail)?;
        let Operation::Known(KnownOperation::ImportStep(params)) = &record.op else {
            return Err(fail());
        };
        let codec = match cert.codec {
            crate::document::import_source_identity::ImportIdentityCodec::Step => {
                params.source_codec == crate::document::record::ImportSourceCodec::Step
            }
            crate::document::import_source_identity::ImportIdentityCodec::Xbf => {
                params.source_codec == crate::document::record::ImportSourceCodec::Xbf
                    && params.brep_format == Some(12)
            }
        };
        if !codec
            || entry.body != cert.body_id
            || entry.kind != crate::document::refs::ElementKind::Face
            || params.source_sha256 != cert.source_sha256
            || params.heal_policy != "v1"
            || params.unit_scale.expr.is_some()
            || format!("{:016x}", params.unit_scale.value.to_bits()) != cert.unit_scale_bits
        {
            return Err(fail());
        }
        Ok(())
    }

    fn validate_reference(
        &self,
        reference: &crate::document::refs::ElementRef,
    ) -> Result<(), DomainError> {
        if reference.import_source_identity.is_some()
            && reference.primary.as_ref().is_none_or(|primary| {
                self.elements
                    .get(&primary.element)
                    .is_none_or(|entry| entry.body != primary.body || entry.kind != primary.kind)
            })
        {
            return Err(DomainError::Validation(
                "certified import reference lacks source index evidence".into(),
            ));
        }
        crate::document::import_source_identity::validate_reference(reference, &self.elements)
            .map_err(DomainError::Validation)
    }
}

impl DocumentSession {
    /// The prepared import at the next history transition, without cloning ordinary history.
    pub fn project_import_history(
        &self,
        direction: HistoryDirection,
    ) -> Option<&ProjectImportEdit> {
        match self.undo.history_txn(direction)?.edits.as_slice() {
            [AppliedEdit {
                forward: EditCommand::ImportProject { project },
                ..
            }] => Some(project),
            _ => None,
        }
    }

    /// Retains later worker identities with their imported producers for exact redo.
    pub fn capture_project_import_identities<'a>(
        &mut self,
        entries: impl Iterator<Item = (&'a ElementId, &'a ElementEntry)>,
        bodies: &crate::document::body::BodyRegistry,
    ) -> Result<Option<ElementIndex>, DomainError> {
        let Some(project) = self.project_import_history(HistoryDirection::Undo) else {
            return Ok(None);
        };
        let captured = project.capture_elements(entries, bodies, &self.document.elements)?;
        let txn = self
            .undo
            .history_txn_mut(HistoryDirection::Undo)
            .expect("import history checked");
        let EditCommand::ImportProject { project } = &mut txn.edits[0].forward else {
            unreachable!("import history checked")
        };
        project.elements = captured.clone();
        Ok(Some(captured))
    }

    pub(super) fn apply_project_import(
        &mut self,
        project: &ProjectImportEdit,
    ) -> Result<(CommandOutcome, Inverse), DomainError> {
        project.validate()?;
        let prior = self.document.clone();
        let mut inverses = vec![Inverse::RestoreElements {
            elements: Box::new(prior.elements.clone()),
        }];
        let result = self.install_project_import(project, &mut inverses);
        match result {
            Ok(outcome) => Ok((outcome, Inverse::Composite(inverses))),
            Err(error) => {
                self.document = prior;
                self.rebuild_graph();
                Err(error)
            }
        }
    }

    fn install_project_import(
        &mut self,
        project: &ProjectImportEdit,
        inverses: &mut Vec<Inverse>,
    ) -> Result<CommandOutcome, DomainError> {
        for (id, entry) in project.elements.iter() {
            if self.document.elements.contains(id) {
                return Err(DomainError::Validation(
                    "import element identity collision".into(),
                ));
            }
            self.document.elements.insert(id.clone(), entry.clone());
        }
        let mut combined = empty_outcome();
        for record in &project.records {
            let (outcome, inverse) = self.add_operation(record.clone(), true)?;
            inverses.push(inverse);
            merge_outcome(&mut combined, &outcome);
        }
        for sketch in project.sketches.values() {
            let (outcome, inverse) = self.add_sketch(sketch.clone())?;
            inverses.push(inverse);
            merge_outcome(&mut combined, &outcome);
        }
        let encoded = serde_json::to_value(&self.document)
            .map_err(|error| DomainError::Validation(error.to_string()))?;
        serde_json::from_value::<crate::document::Document>(encoded)
            .map_err(|error| DomainError::Validation(error.to_string()))?;
        Ok(combined)
    }
}

fn validate_unique_certificates(elements: &ElementIndex) -> Result<(), DomainError> {
    let mut certificates = BTreeSet::new();
    for (_, entry) in elements.iter() {
        if let Some(cert) = &entry.import_source_identity {
            let key = serde_json::to_string(cert)
                .map_err(|error| DomainError::Validation(error.to_string()))?;
            if !certificates.insert(key) {
                return Err(DomainError::Validation(
                    "competing import history identities".into(),
                ));
            }
        }
    }
    Ok(())
}
