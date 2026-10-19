use super::*;
use onecad_core::{
    ids::{SketchId, SnapshotId},
    sketch::{SketchAttachment, WorldPlane},
};
use onecad_interaction::sketch::{SketchInteraction, SketchSnapshot};

fn active(sketch: Sketch) -> SketchInteraction {
    let context = DocumentContext {
        document_id: "document".into(),
        runtime_session: "runtime".into(),
        revision: 1,
        snapshot_id: SnapshotId(1),
        scene: onecad_scene::SceneIdentity {
            session: 1,
            scene: 1,
        },
    };
    let mut state = SketchInteraction::default();
    state.dispatch(SketchAction::ContextChanged(Some(context.clone())));
    let effect = state.dispatch(SketchAction::Begin {
        sketch: Box::new(sketch.clone()),
        create: false,
        context: context.clone(),
    });
    let onecad_interaction::sketch::SketchEffect::EnterSketch { tag, .. } = &effect[0] else {
        panic!()
    };
    let solve = onecad_app::dto::SketchSessionDto {
        edit_session: None,
        history: None,
        sketch_id: sketch.id.to_string(),
        plane: Default::default(),
        entities: Default::default(),
        constraints: Default::default(),
        dof: 1,
        status: onecad_app::dto::SketchSolveStatus::UnderConstrained,
        truth: Default::default(),
        solve_current: true,
        conflicting: vec![],
        entity_states: Default::default(),
        projections: Default::default(),
    };
    state.dispatch(SketchAction::Entered {
        tag: tag.clone(),
        snapshot: SketchSnapshot {
            history: None,
            document: context,
            sketch: Box::new(sketch),
        },
        solve: Box::new(solve),
    });
    state
}

fn sketch() -> Sketch {
    Sketch::new(
        SketchId::new(),
        "Sketch",
        SketchAttachment::World {
            plane: WorldPlane::XY,
        },
    )
}

#[test]
fn dimensional_input_preserves_precision_and_expression_source() {
    assert_eq!(scalar("0.1234", &[]).unwrap(), Scalar::new(0.1234));
    let expression = "0.1234mm * 2";
    let value = scalar(expression, &[]).unwrap();
    assert_eq!(value.value, 0.2468);
    assert_eq!(value.expr.as_deref(), Some(expression));
    assert_eq!(scalar("=2mm + 3mm", &[]).unwrap().value, 5.0);
    for invalid in ["0", "-0.1234", "NaN", "inf", "1/0", "1deg", "missing", ""] {
        assert!(scalar(invalid, &[]).is_err(), "{invalid}");
    }
}

#[test]
fn dimensional_input_uses_resolved_variables_and_rejects_failed_bindings() {
    let mut variable = VariableDto {
        id: "id".into(),
        name: "width".into(),
        value: 1.0,
        expr: None,
        resolved_value: 0.1234,
        dimension: "length".into(),
        error: None,
    };
    assert_eq!(
        scalar("width * 2", &[variable.clone()]).unwrap().value,
        0.2468
    );
    variable.error = Some("unresolved".into());
    assert!(scalar("width", &[variable]).is_err());
}

#[test]
fn line_length_uses_endpoint_identity_and_circle_uses_diameter() {
    let a = EntityId::new();
    let b = EntityId::new();
    let line = EntityId::new();
    let target = Target::Line {
        id: line,
        start: a,
        end: b,
        length: 5.0,
    };
    assert!(
        matches!(constraint(&target, ToolId::Dimension, Some(Scalar::new(0.1234))),
        Some(Constraint::Distance { entity1, entity2, value, .. }) if entity1 == a && entity2 == b && value.value == 0.1234)
    );
    assert!(
        matches!(constraint(&target, ToolId::Horizontal, None), Some(Constraint::Horizontal { line: id, .. }) if id == line)
    );
    assert!(
        matches!(constraint(&target, ToolId::Vertical, None), Some(Constraint::Vertical { line: id, .. }) if id == line)
    );
    let circle = Target::Circle {
        id: EntityId::new(),
        diameter: 12.0,
    };
    assert!(
        matches!(constraint(&circle, ToolId::Dimension, Some(Scalar::new(8.0))), Some(Constraint::Diameter { value, .. }) if value.value == 8.0)
    );
    assert!(!supports(&circle, ToolId::Horizontal));
    assert!(!supports(&circle, ToolId::Distance));
}

#[test]
fn fixed_uses_authoritative_coordinates_and_editor_rejects_selection_or_scope_changes() {
    let id = EntityId::new();
    let mut sketch = sketch();
    sketch
        .add_entity(SketchEntity::point(
            id,
            Vec2::new_unchecked(0.1234, -4.0),
            false,
            false,
        ))
        .unwrap();
    let mut state = active(sketch);
    let key = state.session.as_ref().unwrap().key.clone();
    state.dispatch(SketchAction::Select {
        key,
        entities: vec![id],
    });
    let session = state.session.as_mut().unwrap();
    let target = selected(session).unwrap();
    assert!(
        matches!(constraint(&target, ToolId::Fix, None), Some(Constraint::Fixed { point, at, .. }) if point == id && at == Vec2::new_unchecked(0.1234, -4.0))
    );
    let draft = Draft {
        key: session.key.clone(),
        document: session.document.clone(),
        target,
        text: "0.1234".into(),
        focus: false,
    };
    assert!(current(&draft, session));
    session.document.revision += 1;
    assert!(!current(&draft, session));
    session.document.revision -= 1;
    session.selection.clear();
    assert!(!current(&draft, session));
}

#[test]
fn dimension_editor_ime_enter_and_escape_never_apply_or_cancel() {
    let state = active(sketch());
    let session = state.session.as_ref().unwrap();
    for key in [egui::Key::Enter, egui::Key::Escape] {
        let ctx = egui::Context::default();
        let mut draft = Draft {
            key: session.key.clone(),
            document: session.document.clone(),
            target: Target::Circle {
                id: EntityId::new(),
                diameter: 1.0,
            },
            text: "1".into(),
            focus: true,
        };
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            editor(ui.ctx(), &mut draft, &[]);
        });
        output.textures_delta.clear();
        for ime in [
            egui::ImeEvent::Preedit {
                text: "候補".into(),
                active_range_chars: None,
            },
            egui::ImeEvent::Commit("2".into()),
        ] {
            let mut raw = egui::RawInput {
                events: vec![
                    egui::Event::Ime(ime),
                    egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                ..Default::default()
            };
            let mut composing = true;
            crate::input::protect_composition(&mut composing, &mut raw, true);
            let mut result = (true, true);
            let mut output = ctx.run_ui(raw, |ui| {
                result = editor(ui.ctx(), &mut draft, &[]);
            });
            output.textures_delta.clear();
            assert_eq!(result, (false, false));
        }
    }
}
