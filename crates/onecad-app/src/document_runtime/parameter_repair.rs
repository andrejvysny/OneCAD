//! Undoable correction and live validation of verified generator inputs.
use super::*;
use onecad_core::document::component::ComponentDefId;
use onecad_core::document::record::{ComponentSourceRef, PlaceComponentParams};
use sha2::{Digest, Sha256};
mod schema_cache;
#[cfg(test)]
mod tests;

#[derive(Default)]
pub(super) struct ParameterRepairs {
    enabled: bool,
    schemas: std::sync::Mutex<schema_cache::SchemaCache>,
    detected: usize,
    corrected: usize,
}

fn fingerprint(value: &impl serde::Serialize) -> [u8; 32] {
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
    serde_json::to_writer(&mut writer, value).expect("validated component inputs");
    writer.0.finalize().into()
}

impl DocumentRuntime {
    pub(super) fn repair_loaded_parameters(&mut self) -> Result<(), IoError> {
        self.parameter_repairs.enabled = true;
        let commands = self.loaded_parameter_corrections()?;
        if !commands.is_empty() && !self.read_only {
            self.parameter_repairs.corrected = commands.len();
            self.commit_parameter_repairs(commands)?;
        }
        if self.parameter_repairs.detected != 0 {
            self.clear_parameter_caches();
            self.mark_parameter_repair_state();
        }
        Ok(())
    }

    fn loaded_parameter_corrections(&mut self) -> Result<Vec<EditCommand>, IoError> {
        let mut commands = Vec::new();
        let document = self.session.document();
        let timelines = std::iter::once((None, &document.timeline)).chain(
            document
                .definitions
                .iter()
                .map(|(id, definition)| (Some(*id), &definition.timeline)),
        );
        for (definition, timeline) in timelines {
            for record in timeline.records() {
                let Operation::Known(KnownOperation::PlaceComponent(params)) = &record.op else {
                    continue;
                };
                if !matches!(params.source, ComponentSourceRef::Generator { .. }) {
                    continue;
                }
                let corrected = self.resolved_generator(params);
                if corrected
                    .as_ref()
                    .is_ok_and(|source| *source == params.source)
                {
                    continue;
                }
                self.parameter_repairs
                    .schemas
                    .lock()
                    .unwrap()
                    .remember_invalid(&self.archive_budget, fingerprint(&params.source))
                    .map_err(|error| IoError::TooLarge(error.to_string()))?;
                self.parameter_repairs.detected += 1;
                if let Ok(source) = corrected {
                    let mut updated = params.clone();
                    updated.source = source;
                    let command = EditCommand::UpdateOperationParams {
                        record: record.record_id,
                        op: Operation::Known(KnownOperation::PlaceComponent(updated)),
                    };
                    commands.push(match definition {
                        None => command,
                        Some(id) => EditCommand::Assembly {
                            edit: Box::new(onecad_core::edit::AssemblyEdit::EditDefinition {
                                id,
                                command: Box::new(command),
                            }),
                        },
                    });
                }
            }
        }
        Ok(commands)
    }

    fn resolved_generator(
        &self,
        params: &PlaceComponentParams,
    ) -> Result<ComponentSourceRef, ApiError> {
        let package = self.generator_schema(params)?;
        crate::library::effective_parameters::source(
            &package,
            &params.component_id,
            params.source.clone(),
            &params.params,
        )
    }

    fn commit_parameter_repairs(&mut self, commands: Vec<EditCommand>) -> Result<(), IoError> {
        self.session
            .begin_transaction("Correct verified component defaults");
        for command in commands {
            if let Err(error) = self.session.apply(command) {
                self.session.cancel_transaction();
                return Err(IoError::UnsupportedFeature(format!(
                    "component parameter correction: {error}"
                )));
            }
        }
        if self.session.end_transaction().is_some() {
            self.after_mutation(Some(0));
        }
        Ok(())
    }

    fn invalid_generator_record(&self, record: &OperationRecord) -> bool {
        match &record.op {
            Operation::Known(KnownOperation::PlaceComponent(params))
                if matches!(params.source, ComponentSourceRef::Generator { .. }) =>
            {
                !self
                    .resolved_generator(params)
                    .is_ok_and(|source| source == params.source)
            }
            Operation::Known(KnownOperation::DetachComponent(params))
                if matches!(params.source, ComponentSourceRef::Generator { .. }) =>
            {
                self.parameter_repairs
                    .schemas
                    .lock()
                    .unwrap()
                    .invalid_source(&fingerprint(&params.source))
            }
            _ => false,
        }
    }

    pub(crate) fn require_component_parameters_ready(
        &self,
        record: RecordId,
    ) -> Result<(), ApiError> {
        let record = self
            .session
            .document()
            .timeline
            .record_by_id(record)
            .ok_or_else(|| ApiError::InvalidCommand("component record is unavailable".into()))?;
        if self.invalid_generator_record(record) {
            return Err(ApiError::InvalidCommand(
                "component generator inputs require verified parameter repair".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn parameter_repair_gate(
        &self,
        definition: Option<ComponentDefId>,
    ) -> Option<usize> {
        if !self.parameter_repairs.enabled {
            return None;
        }
        let document = self.session.document();
        let timeline = match definition {
            None => &document.timeline,
            Some(id) => &document.definitions.get(&id)?.timeline,
        };
        timeline
            .records()
            .iter()
            .enumerate()
            .find_map(|(index, record)| {
                (!record.suppressed && self.invalid_generator_record(record)).then_some(index)
            })
    }

    pub(crate) fn parameter_correction_notice(&self) -> Option<String> {
        let total = self.parameter_repairs.detected;
        if total == 0 {
            return None;
        }
        let corrected = self.parameter_repairs.corrected;
        Some(format!(
            "Reopen found {total} component parameter inconsistencies: corrected {corrected} from verified snapshots in one undoable edit; {} require repair. Save to keep corrections. Undo restores the repair requirement.", total - corrected,
        ))
    }

    pub(super) fn mark_parameter_repair_state(&mut self) {
        if let Some(index) = self.parameter_repair_gate(None) {
            let _ = self.regen.timeline.mark_state(
                index,
                StepState::Error {
                    reason: "Component generator inputs require verified parameter repair".into(),
                },
            );
        }
    }

    fn clear_parameter_caches(&mut self) {
        self.cached_meshes.clear();
        self.mesh_cache.clear();
        self.component_cache.clear();
        self.latest_snapshot = None;
        self.preview_png = None;
    }

    pub(crate) fn validate_embedded_component_edit(
        &self,
        edit: &onecad_core::edit::AssemblyEdit,
    ) -> Result<(), ApiError> {
        let onecad_core::edit::AssemblyEdit::EditDefinition { id, command } = edit else {
            return Ok(());
        };
        let definition = self
            .session
            .document()
            .definitions
            .get(id)
            .ok_or_else(|| ApiError::InvalidCommand("unknown component definition".into()))?;
        crate::library::commit_authorization::reject_unauthenticated_definition_edit(
            command,
            |record| {
                definition
                    .timeline
                    .record_by_id(record)
                    .map(|record| &record.op)
            },
        )
    }
}
