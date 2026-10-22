mod parameter_support;
#[allow(dead_code)]
mod support;
use onecad_core::{
    document::{record::*, variables::Scalar},
    ids::{BodyId, RecordId},
    math::Vec3,
};
use onecad_interaction::*;
use support::{context, preview};

fn begin(record: OperationRecord) -> Interaction {
    let mut state = Interaction::default();
    state.dispatch(Action::ContextChanged(Some(context())));
    let action = operand_binding::begin_bound(record, 1, context()).unwrap();
    state.dispatch(action);
    assert!(state.tool.is_some(), "{:?}", state.status);
    state
}

fn change(state: &mut Interaction, options: ModelingOptions) -> Vec<Effect> {
    let tag = state.tool.as_ref().unwrap().tag.clone();
    state.dispatch(Action::ModelingOptionsChanged { tag, options })
}

#[test]
fn gear_bores_declare_only_their_active_fields_and_preserve_module_expression() {
    let mut state = begin(modeling_records::gear(RecordId::new(), 0));
    let module = NumericParameter::GearModule.into();
    state.dispatch(Action::ParameterChanged {
        key: module,
        text: "=module_size".into(),
    });
    let options = |enabled| ModelingOptions::Gear {
        teeth: 24,
        axle_hole: enabled,
        offset_hole: enabled,
        point: Vec3::new_unchecked(10.0, 0.0, 0.0),
    };
    change(&mut state, options(true));
    let tool = state.tool.as_ref().unwrap();
    assert_eq!(tool.params.field(module).unwrap().text, "=module_size");
    assert!(tool
        .params
        .field(NumericParameter::GearAxleDiameter.into())
        .is_some());
    assert!(tool
        .params
        .field(NumericParameter::GearOffsetDistance.into())
        .is_some());
    change(&mut state, options(false));
    let tool = state.tool.as_ref().unwrap();
    assert!(tool
        .params
        .field(NumericParameter::GearAxleDiameter.into())
        .is_none());
    assert!(tool
        .params
        .field(NumericParameter::GearOffsetDistance.into())
        .is_none());
    let Operation::Known(KnownOperation::Gear(p)) = tool.effective_op() else {
        panic!()
    };
    p.validate().unwrap();
    assert_eq!(
        p.placement.frame.as_ref().unwrap().origin,
        p.placement.point
    );
}

#[test]
fn choice_only_operations_preview_confirm_once_and_republish() {
    let a = BodyId::new();
    let b = BodyId::new();
    for record in [
        modeling_records::mirror(a, RecordId::new(), 0),
        modeling_records::boolean(&[a, b], RecordId::new(), 0).unwrap(),
    ] {
        let mut state = begin(record);
        assert_eq!(state.tool.as_ref().unwrap().preview, PreviewStatus::Pending);
        let tag = state.tool.as_ref().unwrap().tag.clone();
        assert!(state.dispatch(Action::Confirm).is_empty());
        state.dispatch(Action::PreviewFinished {
            tag,
            result: Ok(preview(&state.tool.as_ref().unwrap().tag)),
        });
        assert!(state.tool.as_ref().unwrap().ready());
        let effects = state.dispatch(Action::Republish);
        assert!(effects
            .iter()
            .any(|effect| matches!(effect, Effect::Preview { .. })));
        assert!(!state.tool.as_ref().unwrap().ready());
        let tag = state.tool.as_ref().unwrap().tag.clone();
        state.dispatch(Action::PreviewFinished {
            tag: tag.clone(),
            result: Ok(preview(&tag)),
        });
        let effects = state.dispatch(Action::Confirm);
        assert_eq!(
            effects
                .iter()
                .filter(|e| matches!(e, Effect::ApplyOperation { .. }))
                .count(),
            1
        );
        assert!(state.dispatch(Action::Confirm).is_empty());
    }
}

#[test]
fn pattern_options_preserve_invalid_text_and_fence_late_previews() {
    let mut state = begin(modeling_records::linear_pattern(
        BodyId::new(),
        10.0,
        RecordId::new(),
        0,
    ));
    let key = NumericParameter::LinearPatternSpacing.into();
    state.dispatch(Action::ParameterChanged {
        key,
        text: "=unfinished+".into(),
    });
    let old_tag = state.tool.as_ref().unwrap().tag.clone();
    let options = ModelingOptions::Pattern {
        count: 5,
        direction: Vec3::new_unchecked(0.0, 1.0, 0.0),
        origin: Vec3::new_unchecked(0.0, 0.0, 0.0),
        fuse: false,
    };
    change(&mut state, options.clone());
    let tool = state.tool.as_ref().unwrap();
    assert_eq!(tool.params.field(key).unwrap().text, "=unfinished+");
    assert!(tool.tag.sequence > old_tag.sequence);
    state.dispatch(Action::PreviewFinished {
        tag: old_tag.clone(),
        result: Ok(preview(&old_tag)),
    });
    assert!(!state.tool.as_ref().unwrap().ready());
    assert!(state
        .dispatch(Action::ModelingOptionsChanged {
            tag: old_tag,
            options
        })
        .is_empty());
    state.dispatch(Action::Cancel);
    assert!(state.tool.is_none());
}

#[test]
fn switching_extrude_extent_preserves_exact_distance_and_expression() {
    let mut record = parameter_support::record(
        parameter_support::operations()
            .into_iter()
            .find(|op| matches!(op, KnownOperation::Extrude(_)))
            .unwrap(),
    );
    if let Operation::Known(KnownOperation::Extrude(p)) = &mut record.op {
        p.mode = ExtrudeMode::Blind;
        p.two_directions = false;
        p.distance = Scalar {
            value: 1.23456,
            expr: Some("=thickness*2".into()),
        };
    }
    let mut state = begin(record);
    let key = NumericParameter::ExtrudeDistance.into();
    change(
        &mut state,
        ModelingOptions::ExtrudeExtent(ExtrudeMode::ThroughAll),
    );
    assert!(
        !state
            .tool
            .as_ref()
            .unwrap()
            .params
            .field(key)
            .unwrap()
            .active
    );
    change(
        &mut state,
        ModelingOptions::ExtrudeExtent(ExtrudeMode::Blind),
    );
    let field = state.tool.as_ref().unwrap().params.field(key).unwrap();
    assert!(field.active);
    assert_eq!(field.text, "=thickness*2");
}

#[test]
fn invalid_pattern_configuration_cannot_be_confirmed() {
    let mut state = begin(modeling_records::circular_pattern(
        BodyId::new(),
        RecordId::new(),
        0,
    ));
    change(
        &mut state,
        ModelingOptions::Pattern {
            count: 129,
            direction: Vec3::new_unchecked(0.0, 0.0, 0.0),
            origin: Vec3::new_unchecked(0.0, 0.0, 0.0),
            fuse: false,
        },
    );
    let tag = state.tool.as_ref().unwrap().tag.clone();
    let effects = state.dispatch(Action::Evaluated {
        tag,
        result: Ok(onecad_app::dto::EvaluatedExpressionDto {
            value: 360.0,
            dimension: "angle".into(),
            error: None,
        }),
    });
    assert!(!effects.iter().any(|e| matches!(e, Effect::Preview { .. })));
    assert!(state.dispatch(Action::Confirm).is_empty());
}
