//! PlaceComponent through the normal operation session: preview, snap, Copy.
#[allow(dead_code)]
mod support;
use onecad_core::{
    document::{
        record::*,
        refs::{ElementKind, ElementRef, PrimaryRef},
        variables::Scalar,
    },
    edit::EditCommand,
    ids::{BodyId, ElementId, RecordId},
};
use onecad_interaction::*;
use std::collections::BTreeMap;
use support::{context, preview};

fn params() -> PlaceComponentParams {
    PlaceComponentParams {
        component_id: "onecad.std.iso4762".into(),
        component_version: "1.0.0".into(),
        component_revision: format!("sha256:{}", "0".repeat(64)),
        params: BTreeMap::new(),
        source: ComponentSourceRef::Generator {
            generator_id: "iso4762".into(),
            generator_version: 1,
            params: BTreeMap::new(),
            extra: Default::default(),
        },
        mate: None,
        placement: FrozenPlacement {
            translate: [0.0; 3].map(Scalar::new),
            rotate: TransformRotation::default(),
        },
        extra: Default::default(),
    }
}

fn mate() -> ComponentMate {
    ComponentMate {
        self_attachment: "shankAxis".into(),
        target: ElementRef {
            primary: Some(PrimaryRef {
                body: BodyId::new(),
                element: ElementId::new("el_top"),
                kind: ElementKind::Face,
                extra: Default::default(),
            }),
            intent: None,
            anchor: None,
            extra: Default::default(),
        },
        kind: MateKind::Coincident,
        flipped: false,
        self_frame: None,
        target_axis: None,
        target_sidedness: None,
        extra: Default::default(),
    }
}

fn begin(params: PlaceComponentParams) -> (Interaction, Vec<Effect>) {
    let mut state = Interaction::default();
    state.dispatch(Action::ContextChanged(Some(context())));
    let record = modeling_records::component(RecordId::new(), 0, "Place Component", params);
    let effects = state.dispatch(Action::BeginOperation {
        record: Box::new(record),
        fields: Vec::new(),
        focus: None,
        gesture: 1,
        context: context(),
    });
    (state, effects)
}

fn tag(state: &Interaction) -> RequestTag {
    state.tool.as_ref().unwrap().tag.clone()
}

fn change(state: &mut Interaction, options: ModelingOptions) -> Vec<Effect> {
    let tag = tag(state);
    state.dispatch(Action::ModelingOptionsChanged { tag, options })
}

fn previews(effects: &[Effect]) -> Vec<&PlaceComponentParams> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Preview { operation, .. } => match &**operation {
                Operation::Known(KnownOperation::PlaceComponent(p)) => Some(p),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

fn resolved() -> FrozenPlacement {
    FrozenPlacement {
        translate: [1.0, 2.0, 5.0].map(Scalar::new),
        rotate: TransformRotation::default(),
    }
}

fn finish_preview(state: &mut Interaction, mate_placement: Option<FrozenPlacement>) {
    let tag = tag(state);
    let mut result = preview(&tag);
    result.mate_placement = mate_placement;
    state.dispatch(Action::PreviewFinished {
        tag,
        result: Ok(result),
    });
}

fn snap(state: &mut Interaction) -> Vec<Effect> {
    change(
        state,
        ModelingOptions::ComponentSnap {
            translate: [10.0, 20.0, 0.0],
            rotate: TransformRotation::default(),
            mate: Some(Box::new(mate())),
        },
    )
}

fn committed(effects: &[Effect]) -> Vec<&EditCommand> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::ApplyOperation { command, .. } => Some(&**command),
            _ => None,
        })
        .collect()
}

#[test]
fn begin_previews_and_params_change_reprevews_with_the_new_source_params() {
    let (mut state, effects) = begin(params());
    assert_eq!(previews(&effects).len(), 1);
    let sized: BTreeMap<_, _> = [("length".to_string(), ComponentParamValue::Number(25.0))].into();
    let effects = change(&mut state, ModelingOptions::ComponentParams(sized.clone()));
    let previewed = previews(&effects);
    assert_eq!(previewed.len(), 1);
    assert_eq!(previewed[0].params, sized);
    let ComponentSourceRef::Generator { params, .. } = &previewed[0].source else {
        panic!("generator source")
    };
    assert_eq!(params, &sized);
}

#[test]
fn invalid_component_params_surface_an_error_without_a_preview() {
    let mut baked = params();
    baked.source = ComponentSourceRef::Embedded {
        sha256: "0".repeat(64),
        codec: Default::default(),
        brep_format: None,
        extra: Default::default(),
    };
    let (mut state, _) = begin(baked);
    let sized: BTreeMap<_, _> = [("length".to_string(), ComponentParamValue::Number(25.0))].into();
    let effects = change(&mut state, ModelingOptions::ComponentParams(sized));
    assert!(previews(&effects).is_empty());
    let tool = state.tool.as_ref().unwrap();
    assert!(matches!(tool.params.readiness(), Readiness::Invalid { .. }));
    assert_eq!(tool.preview, PreviewStatus::Empty);
}

#[test]
fn snap_sets_placement_and_mate_and_reprevews() {
    let (mut state, _) = begin(params());
    let effects = snap(&mut state);
    let previewed = previews(&effects);
    assert_eq!(previewed.len(), 1);
    assert_eq!(
        previewed[0].placement.translate,
        [10.0, 20.0, 0.0].map(Scalar::new)
    );
    let placed = previewed[0].mate.as_ref().expect("mate set");
    assert_eq!(placed.self_attachment, "shankAxis");
    assert_eq!(placed.kind, MateKind::Coincident);
    let effects = change(
        &mut state,
        ModelingOptions::ComponentSnap {
            translate: [f64::NAN, 0.0, 0.0],
            rotate: TransformRotation::default(),
            mate: None,
        },
    );
    assert!(previews(&effects).is_empty());
    assert!(matches!(
        state.tool.as_ref().unwrap().params.readiness(),
        Readiness::Invalid { .. }
    ));
}

#[test]
fn a_stale_preview_is_ignored() {
    let (mut state, _) = begin(params());
    let old = tag(&state);
    snap(&mut state);
    state.dispatch(Action::PreviewFinished {
        tag: old.clone(),
        result: Ok(preview(&old)),
    });
    assert!(!state.tool.as_ref().unwrap().ready());
}

#[test]
fn confirm_linked_adds_the_place_component_record() {
    let (mut state, _) = begin(params());
    finish_preview(&mut state, None);
    let effects = state.dispatch(Action::Confirm);
    let commands = committed(&effects);
    assert_eq!(commands.len(), 1);
    let EditCommand::AddOperation { record, .. } = commands[0] else {
        panic!("AddOperation")
    };
    assert!(matches!(
        record.op,
        Operation::Known(KnownOperation::PlaceComponent(_))
    ));
}

#[test]
fn confirm_copy_commits_a_detach_at_the_resolved_placement() {
    let (mut state, _) = begin(params());
    snap(&mut state);
    change(&mut state, ModelingOptions::ComponentCopy(true));
    finish_preview(&mut state, Some(resolved()));
    let effects = state.dispatch(Action::Confirm);
    let commands = committed(&effects);
    assert_eq!(commands.len(), 1);
    let EditCommand::AddOperation { record, .. } = commands[0] else {
        panic!("AddOperation")
    };
    let Operation::Known(KnownOperation::DetachComponent(detached)) = &record.op else {
        panic!("DetachComponent")
    };
    assert_eq!(detached.placement, resolved());
}

#[test]
fn confirm_copy_without_a_fresh_preview_is_refused() {
    let (mut state, _) = begin(params());
    snap(&mut state);
    change(&mut state, ModelingOptions::ComponentCopy(true));
    assert!(committed(&state.dispatch(Action::Confirm)).is_empty());
    // A resolved placement from before a later change is never reused.
    finish_preview(&mut state, Some(resolved()));
    change(
        &mut state,
        ModelingOptions::ComponentSnap {
            translate: [11.0, 20.0, 0.0],
            rotate: TransformRotation::default(),
            mate: Some(Box::new(mate())),
        },
    );
    assert!(committed(&state.dispatch(Action::Confirm)).is_empty());
    // Linked placement needs no resolved placement.
    change(&mut state, ModelingOptions::ComponentCopy(false));
    finish_preview(&mut state, None);
    assert_eq!(committed(&state.dispatch(Action::Confirm)).len(), 1);
}

#[test]
fn mated_copy_without_an_echo_is_refused() {
    let (mut state, _) = begin(params());
    snap(&mut state);
    change(&mut state, ModelingOptions::ComponentCopy(true));
    finish_preview(&mut state, None);
    assert!(committed(&state.dispatch(Action::Confirm)).is_empty());
}

#[test]
fn unmated_copy_keeps_the_draft_placement() {
    let mut draft = params();
    draft.placement.translate = [3.0, 4.0, 5.0].map(Scalar::new);
    let (mut state, _) = begin(draft);
    change(&mut state, ModelingOptions::ComponentCopy(true));
    finish_preview(&mut state, None);
    let effects = state.dispatch(Action::Confirm);
    let EditCommand::AddOperation { record, .. } = committed(&effects)[0] else {
        panic!("AddOperation")
    };
    let Operation::Known(KnownOperation::DetachComponent(detached)) = &record.op else {
        panic!("DetachComponent")
    };
    assert_eq!(
        detached.placement.translate,
        [3.0, 4.0, 5.0].map(Scalar::new)
    );
}

#[test]
fn cancel_applies_nothing() {
    let (mut state, _) = begin(params());
    finish_preview(&mut state, None);
    let effects = state.dispatch(Action::Cancel);
    assert!(committed(&effects).is_empty());
    assert!(state.tool.is_none());
}

#[test]
fn creation_rejects_an_invalid_component_record() {
    let mut bad = params();
    bad.component_id = "nodot".into();
    let (state, effects) = begin(bad);
    assert!(state.tool.is_none());
    assert!(previews(&effects).is_empty());
}

fn begin_edit(params: PlaceComponentParams) -> (Interaction, RecordId) {
    let mut state = Interaction::default();
    state.dispatch(Action::ContextChanged(Some(context())));
    let id = RecordId::new();
    let record = modeling_records::component(id, 0, "Place Component", params);
    state.dispatch(Action::BeginOperationEdit {
        record: Box::new(record),
        gesture: 1,
        context: context(),
    });
    (state, id)
}

fn edit_ready(state: &mut Interaction) {
    use onecad_app::api::{EditPreview, EditPreviewOutcome};
    let tag = tag(state);
    state.dispatch(Action::EditPreviewFinished {
        tag: tag.clone(),
        result: Ok(EditPreview {
            record: tag.record_id,
            edited_step: Some(0),
            snapshot_id: tag.context.snapshot_id,
            outcome: EditPreviewOutcome::Ready,
            bodies: Vec::new(),
            meshes: Vec::new(),
            suppressed: Vec::new(),
            needs_repair: Vec::new(),
            step_signatures: Vec::new(),
            isolated: false,
        }),
    });
}

#[test]
fn an_existing_placement_snaps_and_commits_an_update_on_the_same_record() {
    let (mut state, id) = begin_edit(params());
    assert!(state.tool.as_ref().unwrap().editing_existing);
    let effects = snap(&mut state);
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::PreviewExistingEdit { operation, .. }
            if matches!(&**operation, Operation::Known(KnownOperation::PlaceComponent(p)) if p.mate.is_some())
    )));
    let sized: BTreeMap<_, _> = [("length".to_string(), ComponentParamValue::Number(25.0))].into();
    let effects = change(&mut state, ModelingOptions::ComponentParams(sized.clone()));
    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::PreviewExistingEdit { .. })));
    edit_ready(&mut state);
    let effects = state.dispatch(Action::Confirm);
    let commands = committed(&effects);
    assert_eq!(commands.len(), 1);
    let EditCommand::UpdateOperationParams { record, op } = commands[0] else {
        panic!("UpdateOperationParams")
    };
    assert_eq!(*record, id);
    let Operation::Known(KnownOperation::PlaceComponent(p)) = op else {
        panic!("PlaceComponent")
    };
    assert_eq!(p.params, sized);
    assert_eq!(p.placement.translate, [10.0, 20.0, 0.0].map(Scalar::new));
    assert!(p.mate.is_some());
}

#[test]
fn an_unchanged_existing_placement_closes_without_a_command() {
    let (mut state, _) = begin_edit(params());
    let effects = state.dispatch(Action::Confirm);
    assert!(committed(&effects).is_empty());
    assert!(state.tool.is_none());
}

#[test]
fn copy_never_converts_an_existing_placement() {
    let (mut state, _) = begin_edit(params());
    change(&mut state, ModelingOptions::ComponentCopy(true));
    snap(&mut state);
    edit_ready(&mut state);
    let effects = state.dispatch(Action::Confirm);
    let EditCommand::UpdateOperationParams { op, .. } = committed(&effects)[0] else {
        panic!("still an update, not a detach")
    };
    assert!(matches!(
        op,
        Operation::Known(KnownOperation::PlaceComponent(_))
    ));
}

#[test]
fn a_placement_never_mates_to_its_own_body() {
    let (mut state, id) = begin_edit(params());
    let mut own = mate();
    own.target.primary.as_mut().unwrap().body = BodyId(id.0);
    let effects = change(
        &mut state,
        ModelingOptions::ComponentSnap {
            translate: [1.0, 2.0, 3.0],
            rotate: TransformRotation::default(),
            mate: Some(Box::new(own)),
        },
    );
    assert!(effects
        .iter()
        .all(|e| !matches!(e, Effect::PreviewExistingEdit { .. })));
    let Readiness::Invalid { message, .. } = state.tool.as_ref().unwrap().params.readiness() else {
        panic!("refused")
    };
    assert!(message.contains("its own geometry"), "{message}");
    // Another body is fine.
    assert!(!snap(&mut state).is_empty());
}
