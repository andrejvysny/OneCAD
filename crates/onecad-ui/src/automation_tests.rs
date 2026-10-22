use super::testing::{author_ids, find};
use super::*;
use egui::accesskit::Role;
use onecad_core::ids::{BodyId, RecordId, SnapshotId};
use onecad_interaction::{
    Action, DocumentContext, Interaction, ModelSession, operand_binding,
    operand_binding::EdgeOpKind,
};
use onecad_scene::SceneIdentity;
use onecad_widgets::components::{
    inputs::NumberField,
    overlays::FeatureCard,
    shell::{TabSpec, TabStrip, TitleBar},
};

fn context() -> DocumentContext {
    DocumentContext {
        document_id: "doc".into(),
        runtime_session: "runtime".into(),
        revision: 1,
        snapshot_id: SnapshotId(2),
        scene: SceneIdentity {
            session: 1,
            scene: 2,
        },
    }
}

/// A new Move / Rotate session: four declared fields, X first.
fn transform_session() -> ModelSession {
    let mut state = Interaction::default();
    state.dispatch(Action::ContextChanged(Some(context())));
    let record = operand_binding::transform_record(BodyId::new(), [0.0; 3], RecordId::new(), 1);
    let fields = operand_binding::transform_fields();
    state.dispatch(Action::BeginOperation {
        record: Box::new(record),
        focus: fields.first().copied(),
        fields,
        gesture: 1,
        context: context(),
    });
    state.tool.expect("transform session")
}

#[test]
fn every_parameter_and_tool_name_is_distinct() {
    let parameters: std::collections::BTreeSet<_> = [
        NumericParameter::ExtrudeDistance,
        NumericParameter::FilletRadius,
        NumericParameter::ChamferDistance,
        NumericParameter::ChamferSecondDistance,
        NumericParameter::OffsetFaceDistance,
        NumericParameter::TransformTranslation(ParameterAxis::X),
        NumericParameter::TransformTranslation(ParameterAxis::Y),
        NumericParameter::TransformTranslation(ParameterAxis::Z),
        NumericParameter::TransformRotationAngle,
    ]
    .into_iter()
    .map(param)
    .collect();
    assert_eq!(parameters.len(), 9);
    assert!(parameters.contains("onecad.param.filletRadius"));
    assert!(parameters.contains("onecad.param.rotationAngle"));
    assert_eq!(tool(ToolId::Fillet), "onecad.tool.fillet");
    assert_eq!(plane(WorldPlane::XZ), "onecad.plane.xz");
    assert_eq!(tab(1), "onecad.tab.1");
    assert_eq!(history(0), "onecad.history.0");
    assert_eq!(
        EdgeOpKind::Chamfer.parameter(),
        NumericParameter::ChamferDistance
    );
}

#[test]
fn automation_names_are_unique_registry_identifiers() {
    use crate::tool_registry::{PLANNED_MODEL, PLANNED_SKETCH, SelectionKind, pill_tools};
    let kinds = [
        SelectionKind::None,
        SelectionKind::Face,
        SelectionKind::Edge,
        SelectionKind::Body,
        SelectionKind::Region,
        SelectionKind::SketchIdle,
        SelectionKind::SketchEntity,
        SelectionKind::Feature,
        SelectionKind::Mixed,
    ];
    let mut seen = std::collections::BTreeSet::new();
    for def in kinds
        .iter()
        .flat_map(|kind| pill_tools(*kind).iter().flat_map(|group| group.iter()))
        .chain(PLANNED_SKETCH)
        .chain(PLANNED_MODEL)
    {
        let name = def.id.automation_name();
        assert!(name.starts_with(|c: char| c.is_ascii_lowercase()), "{name}");
        assert!(name.chars().all(|c| c.is_ascii_alphanumeric()), "{name}");
        seen.insert((name, format!("{:?}", def.id)));
    }
    let names: std::collections::BTreeSet<_> = seen.iter().map(|(name, _)| *name).collect();
    assert_eq!(
        names.len(),
        seen.len(),
        "two tools share an automation name"
    );
    assert_eq!(ToolId::OffsetFace.automation_name(), "offsetFace");
    assert_eq!(ToolId::MoveRotate.automation_name(), "moveRotate");
}

#[test]
fn sketch_creation_fields_follow_the_operation_card_rule() {
    assert_eq!(creation_field(0, CreationField::Width), DIMENSION_INPUT);
    assert_eq!(creation_field(0, CreationField::Diameter), DIMENSION_INPUT);
    assert_eq!(
        creation_field(1, CreationField::Height),
        "onecad.param.height"
    );
    assert_eq!(creation_field(1, CreationField::Y), "onecad.param.y");
    assert_eq!(
        creation_field(1, CreationField::Sweep),
        "onecad.param.sweep"
    );
}

#[test]
fn operation_card_names_primary_field_parameters_and_footer() {
    let tool = transform_session();
    let named = author_ids(|ui| {
        let out = FeatureCard::new("automation-card", "move", "Move / Rotate").show(ui, |rows| {
            for key in tool.params.active().map(|field| field.key) {
                rows.row(key.label(), |ui| {
                    let mut text = String::from("0");
                    let response = ui.add(NumberField::new(key.salt(), &mut text));
                    let id = crate::feature_card::field_automation(&tool, key);
                    name(ui.ctx(), response.id, &id);
                });
            }
        });
        crate::feature_card::name_footer(ui.ctx(), out.ok_id, out.cancel_id);
    });
    let (_, role, _) = find(&named, DIMENSION_INPUT);
    assert!(
        matches!(role, Role::TextInput | Role::NumberInput),
        "{role:?}"
    );
    for parameter in [
        NumericParameter::TransformTranslation(ParameterAxis::Y),
        NumericParameter::TransformTranslation(ParameterAxis::Z),
        NumericParameter::TransformRotationAngle,
    ] {
        find(&named, &param(parameter));
    }
    assert!(
        named.iter().all(
            |(id, _, _)| id != &param(NumericParameter::TransformTranslation(ParameterAxis::X))
        ),
        "the first field is the dimension input, not a second id"
    );
    assert_eq!(find(&named, OP_CONFIRM).1, Role::Button);
    assert_eq!(find(&named, OP_CANCEL).1, Role::Button);
}

#[test]
fn document_tabs_and_new_tab_are_named_in_strip_order() {
    let tabs = [
        TabSpec {
            id: 7,
            title: "Plate",
            icon: "body",
            dirty: false,
            active: true,
            busy: false,
        },
        TabSpec {
            id: 3,
            title: "Bracket",
            icon: "body",
            dirty: true,
            active: false,
            busy: false,
        },
    ];
    let named = author_ids(|ui| {
        let out = TitleBar::new("")
            .tabs(Some(TabStrip {
                tabs: &tabs,
                home_active: false,
                can_add: true,
            }))
            .show(ui);
        crate::bars::name_tabs(ui.ctx(), &out);
    });
    assert_eq!(find(&named, &tab(0)).1, Role::Tab);
    assert_eq!(find(&named, &tab(1)).1, Role::Tab);
    assert_eq!(find(&named, TAB_NEW).1, Role::Button);
}
