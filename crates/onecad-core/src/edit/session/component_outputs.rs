//! Preserve only observed non-copy transform output writeback across component Redo.
use super::*;
use crate::document::record::{KnownOperation, Operation};
use serde::{Serialize, Serializer};

#[derive(Debug, Clone)]
pub(super) struct OutputReplay {
    pub record: RecordId,
    pub before: Vec<BodyId>,
    pub after: Vec<BodyId>,
    owners: Vec<(BodyId, RecordId)>,
}

pub(super) struct RecordsView<'a> {
    pub records: &'a [OperationRecord],
    pub outputs: &'a [OutputReplay],
}
struct RecordView<'a>(&'a OperationRecord, &'a [BodyId]);
impl Serialize for RecordView<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize_with_outputs(serializer, self.1)
    }
}
impl Serialize for RecordsView<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.records.iter().map(|record| {
            let outputs = self
                .outputs
                .iter()
                .find(|saved| saved.record == record.record_id)
                .map_or(record.outputs.as_slice(), |saved| saved.before.as_slice());
            RecordView(record, outputs)
        }))
    }
}

fn added_outputs(txn: &Txn, id: ComponentDefId, record: RecordId) -> Option<&[BodyId]> {
    txn.edits.iter().find_map(|edit| match &edit.forward {
        EditCommand::Assembly { edit } => match edit.as_ref() {
            crate::edit::AssemblyEdit::EditDefinition {
                id: actual,
                command,
            } if *actual == id => match command.as_ref() {
                EditCommand::AddOperation { record: added, .. } if added.record_id == record => {
                    Some(added.outputs.as_slice())
                }
                _ => None,
            },
            _ => None,
        },
        _ => None,
    })
}

fn modified_targets(record: &OperationRecord) -> Option<&[BodyId]> {
    let Operation::Known(KnownOperation::TransformBody(params)) = &record.op else {
        return None;
    };
    if params.copy || record.suppressed || params.validate().is_err() {
        return None;
    }
    Some(&params.targets)
}

pub(super) fn capture(
    before: Option<&ComponentDef>,
    after: &ComponentDef,
    txn: &Txn,
) -> Vec<OutputReplay> {
    let Some(prior) = before else {
        return Vec::new();
    };
    let mut changes = Vec::new();
    for record in after.timeline.records() {
        let old = prior
            .timeline
            .records()
            .iter()
            .find(|r| r.record_id == record.record_id)
            .map(|r| r.outputs.as_slice())
            .or_else(|| added_outputs(txn, after.id, record.record_id));
        let Some(old) = old.filter(|old| old.is_empty() && *old != record.outputs) else {
            continue;
        };
        let Some(targets) = modified_targets(record) else {
            continue;
        };
        if targets.len() != record.outputs.len()
            || targets.iter().copied().collect::<BTreeSet<_>>()
                != record.outputs.iter().copied().collect()
        {
            continue;
        }
        let owners: Option<Vec<_>> = targets
            .iter()
            .map(|body| {
                let old = prior.bodies.get(*body)?;
                let current = after.bodies.get(*body)?;
                (old.created_by == current.created_by).then_some((*body, current.created_by))
            })
            .collect();
        let Some(owners) = owners else {
            continue;
        };
        changes.push(OutputReplay {
            record: record.record_id,
            before: old.to_vec(),
            after: record.outputs.clone(),
            owners,
        });
    }
    changes
}

pub(super) fn validate(
    definition: &ComponentDef,
    outputs: &[OutputReplay],
) -> Result<(), DomainError> {
    for saved in outputs {
        let record = definition
            .timeline
            .records()
            .iter()
            .find(|r| r.record_id == saved.record)
            .ok_or_else(|| DomainError::Validation("redo transform record is missing".into()))?;
        if record.outputs != saved.before
            || modified_targets(record).is_none()
            || saved.owners.iter().any(|(body, owner)| {
                definition
                    .bodies
                    .get(*body)
                    .is_none_or(|meta| meta.created_by != *owner)
            })
        {
            return Err(DomainError::Validation(
                "redo transform output ownership changed".into(),
            ));
        }
    }
    Ok(())
}

pub(super) fn restore(definition: &mut ComponentDef, outputs: &[OutputReplay]) {
    for saved in outputs {
        let position = definition
            .timeline
            .records()
            .iter()
            .position(|r| r.record_id == saved.record)
            .expect("preflighted record");
        definition
            .timeline
            .set_record_outputs(position, saved.after.clone());
    }
}
