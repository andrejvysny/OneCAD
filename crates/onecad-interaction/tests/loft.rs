#[allow(dead_code)]
mod support;
use onecad_core::{
    document::{
        record::{KnownOperation, Operation, OperationRecord},
        refs::SketchRegionRef,
    },
    edit::EditCommand,
    ids::{RecordId, RegionId, SketchId},
};
use onecad_interaction::*;
use support::*;

fn sections() -> Vec<SketchRegionRef> {
    (0..3)
        .map(|i| SketchRegionRef {
            sketch: SketchId::new(),
            region: RegionId::new(format!("section-{i}")),
            region_identity_version: Some(2),
            region_anchor: Some([1.0, 2.0]),
            extra: Default::default(),
        })
        .collect()
}

fn begin() -> (Interaction, OperationRecord, RequestTag) {
    let record = modeling_records::loft(sections(), RecordId::new(), 3).unwrap();
    let mut state = Interaction::default();
    state.dispatch(Action::ContextChanged(Some(context())));
    let effects =
        state.dispatch(operand_binding::begin_bound(record.clone(), 1, context()).unwrap());
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::Preview {
            sketch_id: None,
            ..
        }
    )));
    let tag = state.tool.as_ref().unwrap().tag.clone();
    assert!(state.tool.as_ref().unwrap().params.fields().is_empty());
    (state, record, tag)
}

#[test]
fn loft_choices_invalidate_preview_and_commit_one_ordered_operation() {
    let (mut state, record, old) = begin();
    state.dispatch(Action::PreviewFinished {
        tag: old.clone(),
        result: Ok(preview(&old)),
    });
    let Operation::Known(KnownOperation::Loft(p)) = record.op else {
        panic!("Loft");
    };
    let mut profiles = p.profiles;
    profiles.reverse();
    let effects = state.dispatch(Action::ModelingOptionsChanged {
        tag: old.clone(),
        options: ModelingOptions::Loft {
            profiles: profiles.clone(),
            is_ruled: true,
        },
    });
    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::ClearPreview { .. })));
    let tag = state.tool.as_ref().unwrap().tag.clone();
    assert_ne!(old, tag);
    assert!(state
        .dispatch(Action::PreviewFinished {
            tag: old.clone(),
            result: Ok(preview(&old))
        })
        .is_empty());
    assert!(state.dispatch(Action::Confirm).is_empty());
    state.dispatch(Action::PreviewFinished {
        tag: tag.clone(),
        result: Ok(preview(&tag)),
    });
    let effects = state.dispatch(Action::Confirm);
    let command = effects
        .iter()
        .find_map(|e| match e {
            Effect::ApplyOperation {
                command,
                hide_sketch: None,
                ..
            } => Some(command),
            _ => None,
        })
        .unwrap();
    let EditCommand::AddOperation { record, .. } = command.as_ref() else {
        panic!("new Loft");
    };
    assert!(
        matches!(&record.op, Operation::Known(KnownOperation::Loft(p)) if p.profiles == profiles && p.is_ruled)
    );
    assert_eq!(record.inputs, record.op.derive_inputs());
    assert!(state.dispatch(Action::Confirm).is_empty());
}

#[test]
fn loft_invalid_sections_refuse_and_cancel_keeps_no_command() {
    let (mut state, _, tag) = begin();
    let effects = state.dispatch(Action::ModelingOptionsChanged {
        tag,
        options: ModelingOptions::Loft {
            profiles: sections()[..1].to_vec(),
            is_ruled: false,
        },
    });
    assert!(!effects.iter().any(|e| matches!(e, Effect::Preview { .. })));
    assert!(matches!(
        state.tool.as_ref().unwrap().params.readiness(),
        Readiness::Invalid { .. }
    ));
    assert!(state.dispatch(Action::Confirm).is_empty());
    let effects = state.dispatch(Action::Cancel);
    assert!(!effects
        .iter()
        .any(|e| matches!(e, Effect::ApplyOperation { .. })));
    assert!(state.tool.is_none());
}

#[test]
fn loft_history_edit_previews_reordered_sections_under_existing_record() {
    let (mut state, record, _) = begin();
    state.dispatch(Action::Cancel);
    state.dispatch(Action::BeginOperationEdit {
        record: Box::new(record.clone()),
        gesture: 2,
        context: context(),
    });
    let tag = state.tool.as_ref().unwrap().tag.clone();
    let Operation::Known(KnownOperation::Loft(p)) = &record.op else {
        panic!("Loft");
    };
    let effects = state.dispatch(Action::ModelingOptionsChanged {
        tag,
        options: ModelingOptions::Loft {
            profiles: p.profiles.iter().rev().cloned().collect(),
            is_ruled: false,
        },
    });
    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::PreviewExistingEdit { .. })));
    assert_eq!(state.tool.as_ref().unwrap().tag.record_id, record.record_id);
    let tag = state.tool.as_ref().unwrap().tag.clone();
    state.dispatch(Action::ContextChanged(None));
    assert!(state
        .dispatch(Action::PreviewFinished {
            tag: tag.clone(),
            result: Ok(preview(&tag))
        })
        .is_empty());
    assert!(state.tool.is_none());
}
