//! Validate worker-certified source identities against authored runtime authority.
use super::*;
use onecad_core::regen::WorkerElementEvidence;

impl DocumentRuntime {
    pub(crate) fn validate_project_import_namespace(
        &self,
        imported: &onecad_core::io::project_import::ImportedProject,
    ) -> Result<(), ApiError> {
        for (id, entry) in imported.elements.iter() {
            if self.regen.elements.contains(id) || self.staged.contains_key(id) {
                return Err(ApiError::InvalidCommand(format!(
                    "cannot import: source element id {id} already exists in this document"
                )));
            }
            if self.regen.bodies.get(entry.body).is_some()
                || self
                    .regen
                    .elements
                    .iter()
                    .any(|(_, other)| other.body == entry.body)
                || self.staged.values().any(|other| other.body == entry.body)
            {
                return Err(ApiError::InvalidCommand(format!(
                    "cannot import: source body id {} already exists in this document",
                    entry.body
                )));
            }
        }
        Ok(())
    }

    pub(crate) fn restore_imported_elements(
        &mut self,
        elements: &onecad_core::document::element_index::ElementIndex,
    ) {
        for (id, entry) in elements.iter() {
            self.regen.elements.insert(id.clone(), entry.clone());
        }
    }

    pub(crate) fn remove_imported_elements(
        &mut self,
        elements: &onecad_core::document::element_index::ElementIndex,
    ) {
        for (id, _) in elements.iter() {
            self.regen.elements.remove(id);
            self.staged.remove(id);
        }
        self.promoted
            .retain(|_, entry| !elements.contains(&entry.element_id));
    }

    pub(crate) fn discard_pending_import_elements(
        &mut self,
        elements: &onecad_core::document::element_index::ElementIndex,
    ) {
        for (id, _) in elements.iter() {
            self.staged.remove(id);
        }
        self.promoted
            .retain(|_, entry| !elements.contains(&entry.element_id));
    }

    pub(crate) fn validate_import_redo_namespace(
        &self,
        project: &onecad_core::edit::project_import::ProjectImportEdit,
    ) -> Result<(), DomainError> {
        let producers: std::collections::BTreeSet<_> = project
            .records
            .iter()
            .map(|record| record.record_id)
            .collect();
        for (id, entry) in project.elements.iter() {
            if self.regen.elements.contains(id)
                || self.staged.contains_key(id)
                || self
                    .regen
                    .elements
                    .iter()
                    .any(|(_, existing)| existing.body == entry.body)
                || self
                    .staged
                    .values()
                    .any(|existing| existing.body == entry.body)
                || self
                    .regen
                    .bodies
                    .get(entry.body)
                    .is_some_and(|body| !producers.contains(&body.created_by))
            {
                return Err(DomainError::Validation(
                    "project import redo identity namespace collision".into(),
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn validate_import_evidence(
        &self,
        evidence: &WorkerElementEvidence,
    ) -> Result<(), EngineError> {
        let Some(cert) = &evidence.import_source_identity else {
            if evidence
                .existing
                .as_ref()
                .and_then(|id| self.staged.get(id).or_else(|| self.regen.elements.get(id)))
                .is_some_and(|entry| entry.import_source_identity.is_some())
            {
                return Err(EngineError::Protocol {
                    message: "worker omitted existing import source identity".into(),
                });
            }
            return Ok(());
        };
        cert.validate().map_err(|message| EngineError::Protocol {
            message: message.into(),
        })?;
        let fail = || EngineError::Protocol {
            message: "import source identity does not match authored producer".into(),
        };
        if cert.body_id != evidence.body || evidence.kind != ElementKind::Face {
            return Err(fail());
        }
        let body = self.regen.bodies.get(evidence.body).ok_or_else(fail)?;
        if body.created_by != cert.producer_record_id {
            return Err(fail());
        }
        let record = self
            .session
            .document()
            .timeline
            .records()
            .iter()
            .find(|row| row.record_id == cert.producer_record_id)
            .ok_or_else(fail)?;
        let Operation::Known(KnownOperation::ImportStep(params)) = &record.op else {
            return Err(fail());
        };
        let codec_matches = match cert.codec {
            onecad_core::document::import_source_identity::ImportIdentityCodec::Step => {
                params.source_codec == ImportSourceCodec::Step
            }
            onecad_core::document::import_source_identity::ImportIdentityCodec::Xbf => {
                params.source_codec == ImportSourceCodec::Xbf && params.brep_format == Some(12)
            }
        };
        if !codec_matches
            || params.source_sha256 != cert.source_sha256
            || params.heal_policy != "v1"
            || params.unit_scale.expr.is_some()
            || format!("{:016x}", params.unit_scale.value.to_bits()) != cert.unit_scale_bits
        {
            return Err(fail());
        }
        if let Some(id) = &evidence.existing {
            if let Some(entry) = self.staged.get(id).or_else(|| self.regen.elements.get(id)) {
                if entry.body != evidence.body
                    || entry.kind != evidence.kind
                    || entry.import_source_identity != evidence.import_source_identity
                {
                    return Err(fail());
                }
            }
        }
        Ok(())
    }
}

impl DocumentRuntime {
    pub(crate) fn validate_minted_import_identities(
        &self,
        minted: &[(onecad_core::ids::ElementId, WorkerElementEvidence)],
    ) -> Result<(), EngineError> {
        for (index, (id, evidence)) in minted.iter().enumerate() {
            let Some(cert) = &evidence.import_source_identity else {
                continue;
            };
            let conflict =
                self.regen
                    .elements
                    .iter()
                    .chain(self.staged.iter())
                    .any(|(existing_id, entry)| {
                        existing_id != id && entry.import_source_identity.as_ref() == Some(cert)
                    })
                    || minted[..index].iter().any(|(other_id, other)| {
                        other_id != id && other.import_source_identity.as_ref() == Some(cert)
                    });
            if conflict {
                return Err(EngineError::Protocol {
                    message: "competing element ids share import source identity".into(),
                });
            }
        }
        Ok(())
    }
}

impl DocumentRuntime {
    pub(crate) fn validate_edit_import_identities(
        &self,
        command: &mut EditCommand,
    ) -> Result<(), DomainError> {
        self.validate_edit_identities_in(command, &self.regen.elements, true)
    }

    fn validate_edit_identities_in(
        &self,
        command: &mut EditCommand,
        entries: &onecad_core::document::element_index::ElementIndex,
        allow_staged: bool,
    ) -> Result<(), DomainError> {
        let check = |reference: &ElementRef| {
            if reference.import_source_identity.is_some() {
                let present = reference.primary.as_ref().is_some_and(|primary| {
                    (allow_staged && self.staged.contains_key(&primary.element))
                        || entries.get(&primary.element).is_some()
                });
                if !present {
                    return Err(DomainError::Validation(
                        "certified reference is not in the active definition".into(),
                    ));
                }
            }
            let index = reference.primary.as_ref().and_then(|primary| {
                if allow_staged && self.staged.contains_key(&primary.element) {
                    Some(&self.staged)
                } else {
                    None
                }
            });
            if let Some(staged) = index {
                let mut one = onecad_core::document::element_index::ElementIndex::default();
                let primary = reference.primary.as_ref().expect("checked above");
                if let Some(entry) = staged.get(&primary.element) {
                    one.insert(primary.element.clone(), entry.clone());
                }
                onecad_core::document::import_source_identity::validate_reference(reference, &one)
            } else {
                onecad_core::document::import_source_identity::validate_reference(
                    reference, entries,
                )
            }
            .map_err(DomainError::Validation)
        };
        match command {
            EditCommand::ImportProject { project } => project.validate()?,
            EditCommand::AddOperation { record, .. } => {
                if let Operation::Known(operation) = &mut record.op {
                    for reference in operation.stored_element_refs_mut() {
                        check(reference)?;
                    }
                }
            }
            EditCommand::UpdateOperationParams {
                op: Operation::Known(operation),
                ..
            } => {
                for reference in operation.stored_element_refs_mut() {
                    check(reference)?;
                }
            }
            EditCommand::EditOperationInput {
                reference: InputRef::Element(reference),
                ..
            } => check(reference)?,
            EditCommand::EditOperationInput {
                reference:
                    InputRef::Axis(onecad_core::document::refs::AxisRef::Element {
                        edge_ref: Some(reference),
                        ..
                    }),
                ..
            } => check(reference)?,
            EditCommand::AddSketch { sketch } => {
                if let Some(reference) = sketch.host_face() {
                    check(reference)?;
                }
            }
            EditCommand::UpdateSketchAttachment {
                attachment: onecad_core::sketch::SketchAttachment::HostFace { face, .. },
                ..
            } => check(face)?,
            EditCommand::Assembly { edit } => match edit.as_mut() {
                onecad_core::edit::AssemblyEdit::EditDefinition { id, command } => {
                    let definition =
                        self.session.document().definitions.get(id).ok_or_else(|| {
                            DomainError::Validation("component definition is unavailable".into())
                        })?;
                    self.validate_edit_identities_in(command, &definition.elements, false)?;
                }
                onecad_core::edit::AssemblyEdit::SetBoundJointsAndPoses { bindings, .. } => {
                    for binding in bindings {
                        let definition = self
                            .session
                            .document()
                            .definitions
                            .get(&binding.definition)
                            .ok_or_else(|| {
                                DomainError::Validation(
                                    "component definition is unavailable".into(),
                                )
                            })?;
                        onecad_core::document::import_source_identity::validate_reference(
                            &binding.element,
                            &definition.elements,
                        )
                        .map_err(DomainError::Validation)?;
                    }
                }
                _ => {}
            },
            _ => {}
        }
        Ok(())
    }
}
