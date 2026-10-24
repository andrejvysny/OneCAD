use super::*;
use egui::{Context, Event, FullOutput, Modifiers, PointerButton, Pos2, Rect, pos2, vec2};

fn frame(ctx: &Context, diagnostics: &Diagnostics, events: Vec<Event>) -> FullOutput {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1180.0, 820.0))),
            events,
            ..Default::default()
        },
        |_| diagnostics.show(ctx),
    );
    output.textures_delta.clear();
    output
}

fn settled_frame(ctx: &Context, diagnostics: &Diagnostics) -> FullOutput {
    frame(ctx, diagnostics, vec![]);
    frame(ctx, diagnostics, vec![])
}

fn text_position(output: &FullOutput, label: &str) -> Pos2 {
    output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == label => {
                let bounds = Rect::from_min_size(text.pos, text.galley.size());
                assert!(shape.clip_rect.contains_rect(bounds), "{label} is clipped");
                Some(bounds.center())
            }
            _ => None,
        })
        .unwrap_or_else(|| {
            let labels: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) => Some(text.galley.text()),
                    _ => None,
                })
                .collect();
            panic!("missing rendered {label}; labels: {labels:?}")
        })
}

fn scroll_details(
    ctx: &Context,
    diagnostics: &Diagnostics,
    message: &str,
    pos: Pos2,
) -> FullOutput {
    let mut seen_rows = std::collections::BTreeSet::new();
    let mut row_count = 0;
    for _ in 0..40 {
        frame(
            ctx,
            diagnostics,
            vec![
                Event::PointerMoved(pos),
                Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    phase: egui::TouchPhase::Move,
                    source: egui::MouseWheelSource::Unknown,
                    delta: vec2(0.0, -20.0),
                    modifiers: Modifiers::NONE,
                },
            ],
        );
        let output = settled_frame(ctx, diagnostics);
        assert!(egui::Popup::is_any_open(ctx), "scroll closed Activity");
        for shape in &output.shapes {
            if let egui::Shape::Text(text) = &shape.shape {
                if text.galley.text() == message {
                    row_count = text.galley.rows.len();
                    for (index, row) in text.galley.rows.iter().enumerate() {
                        if shape
                            .clip_rect
                            .contains_rect(row.rect().translate(text.pos.to_vec2()))
                        {
                            seen_rows.insert(index);
                        }
                    }
                }
                if text.galley.text() == "Copy details"
                    && row_count > 0
                    && seen_rows.len() == row_count
                    && shape
                        .clip_rect
                        .contains_rect(Rect::from_min_size(text.pos, text.galley.size()))
                {
                    return output;
                }
            }
        }
    }
    panic!("full details not reachable by scrolling: {seen_rows:?}/{row_count} rows");
}

fn click(ctx: &Context, diagnostics: &Diagnostics, pos: Pos2) -> FullOutput {
    frame(ctx, diagnostics, vec![Event::PointerMoved(pos)]);
    let pointer = |pressed| {
        vec![
            Event::PointerMoved(pos),
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            },
        ]
    };
    frame(ctx, diagnostics, pointer(true));
    frame(ctx, diagnostics, pointer(false))
}

fn exercise_details(ctx: &Context, diagnostics: &Diagnostics, message: &str) {
    let output = settled_frame(ctx, diagnostics);
    click(
        ctx,
        diagnostics,
        text_position(&output, "Activity · Completed"),
    );
    let output = settled_frame(ctx, diagnostics);
    assert!(egui::Popup::is_any_open(ctx));
    assert!(!output.shapes.iter().any(|shape| matches!(
        &shape.shape, egui::Shape::Text(text) if text.galley.text() == message
    )));

    let details = text_position(&output, "Details");
    click(ctx, diagnostics, details);
    assert!(
        egui::Popup::is_any_open(ctx),
        "Details click closed Activity"
    );
    // The bounded Activity body scrolls; verify every full-message row becomes visible.
    let output = scroll_details(ctx, diagnostics, message, details);
    let output = click(ctx, diagnostics, text_position(&output, "Copy details"));
    assert!(
        output
            .platform_output
            .commands
            .iter()
            .any(|command| matches!(
                command, egui::OutputCommand::CopyText(text) if text == message
            ))
    );
    assert!(
        egui::Popup::is_any_open(ctx),
        "Copy details closed Activity"
    );
    click(ctx, diagnostics, pos2(20.0, 20.0));
    assert!(
        !egui::Popup::is_any_open(ctx),
        "outside click did not close Activity"
    );
}

#[test]
fn activity_details_and_copy_stay_open_until_outside_click() {
    for (action, summary, path) in [
        (
            FileAction::Save,
            "Document saved",
            "/Users/test/Documents/OneCAD/Release model.onecad",
        ),
        (
            FileAction::Export,
            "Export completed",
            "/Users/test/Documents/OneCAD/Release model.step",
        ),
    ] {
        let ctx = Context::default();
        ctx.style_mut_of(egui::Theme::Dark, |style| style.animation_time = 0.0);
        ctx.style_mut_of(egui::Theme::Light, |style| style.animation_time = 0.0);
        let mut diagnostics = Diagnostics::default();
        let message = format!("{summary}\nFull path: {path}");
        diagnostics.record(Source::File(action), State::Succeeded, &message);
        exercise_details(&ctx, &diagnostics, &message);
        assert!(diagnostics.contains(Source::File(action), State::Succeeded, path));
    }
}
