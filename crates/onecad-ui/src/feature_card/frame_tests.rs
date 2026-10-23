use super::*;
use onecad_core::ids::{BodyId, RecordId, SnapshotId};
use onecad_interaction::{DocumentContext, Interaction, ModelingOptions, modeling_records};
use onecad_scene::SceneIdentity;
use onecad_widgets::text_session;

fn session() -> Interaction {
    let context = DocumentContext {
        document_id: "document".into(),
        runtime_session: "runtime".into(),
        revision: 1,
        snapshot_id: SnapshotId(2),
        scene: SceneIdentity {
            session: 1,
            scene: 2,
        },
    };
    let mut interaction = Interaction::default();
    interaction.dispatch(Action::ContextChanged(Some(context.clone())));
    interaction.dispatch(Action::BeginOperationEdit {
        record: Box::new(modeling_records::linear_pattern(
            BodyId::new(),
            10.0,
            RecordId::new(),
            0,
        )),
        gesture: 1,
        context,
    });
    interaction
}

fn scalar_frame(ctx: &Context, interaction: &mut Interaction, shown: &ModelSession) {
    text_session::begin_frame(ctx, "card-session".into());
    ctx.run_ui(egui::RawInput::default(), |ui| {
        if let Some(current) = current_session(interaction, shown) {
            for field in current.params.active() {
                let salt = (
                    field.key.salt(),
                    current.tag.record_id.to_string(),
                    current.tag.gesture,
                );
                let id = ui.make_persistent_id(egui::IdSalt::new(salt.clone()));
                ui.memory_mut(|memory| memory.request_focus(id));
                let mut draft = field.text.clone();
                if ui.add(NumberField::new(salt, &mut draft)).changed() {
                    interaction.dispatch(Action::ParameterChanged {
                        key: field.key,
                        text: draft,
                    });
                }
            }
        }
    })
    .textures_delta
    .clear();
    text_session::end_frame(ctx);
}

#[test]
fn earlier_choice_preserves_acknowledged_scalar_callback_in_the_same_card_pass() {
    let ctx = Context::default();
    onecad_widgets::theme::install(&ctx, onecad_widgets::theme::load(false).unwrap(), false);
    text_session::enable(&ctx);
    let mut interaction = session();
    let shown = interaction.tool.clone().unwrap();
    scalar_frame(&ctx, &mut interaction, &shown);
    let snapshot = text_session::snapshot(&ctx).unwrap();
    assert!(text_session::apply(
        &ctx,
        text_session::Operation {
            generation: snapshot.generation,
            revision: snapshot.revision,
            sequence: snapshot.sequence + 1,
            edit: text_session::Edit::Replace(
                text_session::Range { start: 0, end: 2 },
                "27.123456789".into()
            ),
        }
    ));
    interaction.dispatch(Action::ModelingOptionsDraftChanged {
        tag: shown.tag.clone(),
        options: Some(ModelingOptions::Pattern {
            count: 5,
            direction: onecad_core::math::Vec3::new_unchecked(0.0, 1.0, 0.0),
            origin: onecad_core::math::Vec3::new_unchecked(0.0, 0.0, 0.0),
            fuse: true,
        }),
        error: None,
    });
    assert_ne!(interaction.tool.as_ref().unwrap().tag, shown.tag);
    scalar_frame(&ctx, &mut interaction, &shown);
    let current = interaction.tool.as_ref().unwrap();
    assert_eq!(current.params.active().next().unwrap().text, "27.123456789");
    assert_eq!(
        text_session::snapshot(&ctx).unwrap().generation,
        snapshot.generation
    );
}

#[test]
fn refreshed_scalar_rows_require_the_same_session_on_every_ownership_axis() {
    let interaction = session();
    let shown = interaction.tool.clone().unwrap();
    for axis in 0..9 {
        let mut previous = shown.clone();
        match axis {
            0 => previous.tag.context.document_id.push_str("-other"),
            1 => previous.tag.context.runtime_session.push_str("-other"),
            2 => previous.tag.context.revision += 1,
            3 => previous.tag.context.snapshot_id.0 += 1,
            4 => previous.tag.context.scene.session += 1,
            5 => previous.tag.context.scene.scene += 1,
            6 => previous.tag.record_id = RecordId::new(),
            7 => previous.tag.gesture += 1,
            8 => previous.tag.sequence += 1,
            _ => unreachable!(),
        }
        assert_eq!(
            current_session(&interaction, &previous).is_some(),
            axis == 8
        );
    }
}
