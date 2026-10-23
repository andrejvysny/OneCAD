use super::*;

fn option_frame(
    ctx: &egui::Context,
    gear_control: bool,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    onecad_widgets::text_session::begin_frame(ctx, "model-options".into());
    let mut out = ctx.run_ui(
        egui::RawInput {
            events,
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 900.0),
            )),
            ..Default::default()
        },
        |ui| {
            let mut fields = Fields::new("snapshot-test");
            FeatureCardRows::inline(ui, |rows| {
                if gear_control {
                    let record = modeling_records::gear(RecordId::new(), 0);
                    let Operation::Known(KnownOperation::Gear(p)) = record.op else {
                        unreachable!()
                    };
                    gear(rows, &mut fields, &p);
                } else {
                    pattern(
                        rows,
                        &mut fields,
                        3,
                        Vec3::new_unchecked(0.0, 0.0, 1.0),
                        Vec3::new_unchecked(0.0, 0.0, 0.0),
                        false,
                        true,
                    );
                }
            });
        },
    );
    out.textures_delta.clear();
    onecad_widgets::text_session::end_frame(ctx);
    out
}

#[test]
fn tapping_option_numbers_publishes_the_native_keyboard_session() {
    for suffix in ["patternCount", "patternDirectionX", "originZ", "gearTeeth"] {
        let ctx = egui::Context::default();
        onecad_widgets::theme::install(&ctx, onecad_widgets::theme::load(false).unwrap(), false);
        ctx.enable_accesskit();
        onecad_widgets::text_session::enable(&ctx);
        let gear_control = suffix == "gearTeeth";
        option_frame(&ctx, gear_control, Vec::new());
        let out = option_frame(&ctx, gear_control, Vec::new());
        let name = format!("onecad.option.{suffix}");
        let bounds = out
            .platform_output
            .accesskit_update
            .unwrap()
            .nodes
            .iter()
            .find_map(|(_, node)| {
                (node.author_id() == Some(name.as_str()))
                    .then(|| node.bounds())
                    .flatten()
            })
            .expect("named option field");
        let pos = egui::pos2(
            (bounds.x0 + 5.0) as f32,
            ((bounds.y0 + bounds.y1) / 2.0) as f32,
        );
        for pressed in [true, false] {
            option_frame(
                &ctx,
                gear_control,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        let snapshot = onecad_widgets::text_session::snapshot(&ctx)
            .unwrap_or_else(|| panic!("{suffix} did not publish a native keyboard session"));
        assert_eq!(
            snapshot.text,
            if suffix == "gearTeeth" {
                "20"
            } else if suffix == "patternCount" {
                "3"
            } else {
                "0"
            }
        );
    }
}
#[test]
fn vector_default_rounding_and_focus_blur_never_quantize_bound_values() {
    let ctx = egui::Context::default();
    let original = 50.00019918212638;
    let mut value = original;
    let mut id = None;
    for focused in [false, true, false, false] {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            if let Some(id) = id {
                ui.memory_mut(|memory| {
                    if focused {
                        memory.request_focus(id);
                    } else {
                        memory.surrender_focus(id);
                    }
                });
            }
            let response = Fields::new("precision").value(ui, "vector", "Vector X", &mut value);
            id = Some(response.id);
            assert!(!response.changed());
        });
        out.textures_delta.clear();
        assert_eq!(value, original);
        let expected = if focused {
            original.to_string()
        } else {
            "50.00".into()
        };
        assert!(out.shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::epaint::Shape::Text(text) if text.galley.job.text == expected)
        }), "missing shown value: {expected}");
    }
}

#[test]
fn pattern_controls_are_addressable_by_both_hosts() {
    let nodes = crate::automation::testing::author_ids(|ui| {
        FeatureCardRows::inline(ui, |rows| {
            pattern(
                rows,
                &mut Fields::new("test"),
                3,
                Vec3::new_unchecked(0.0, 0.0, 1.0),
                Vec3::new_unchecked(0.0, 0.0, 0.0),
                false,
                true,
            );
        });
    });
    for suffix in [
        "patternCount",
        "patternDirectionX",
        "patternDirectionY",
        "patternDirectionZ",
        "originX",
        "originY",
        "originZ",
        "patternFuse",
    ] {
        assert_eq!(
            nodes
                .iter()
                .filter(|(id, _, _)| id == &format!("onecad.option.{suffix}"))
                .count(),
            1,
            "{suffix}: {nodes:?}"
        );
    }
}
