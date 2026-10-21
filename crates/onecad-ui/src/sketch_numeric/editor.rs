use super::*;

fn field_id(draft: &Draft, field: CreationField) -> egui::Id {
    id().with((
        &draft.key.document_id,
        &draft.key.runtime_session,
        draft.key.generation,
        draft.key.sketch_id.to_string(),
        draft.generation,
        format!("{field:?}"),
    ))
}

fn label(field: CreationField) -> &'static str {
    match field {
        CreationField::X => "X",
        CreationField::Y => "Y",
        CreationField::Length => "Length",
        CreationField::Width => "Width",
        CreationField::Height => "Height",
        CreationField::Diameter => "Diameter",
        CreationField::Radius => "Radius",
        CreationField::Sweep => "Sweep (degrees)",
    }
}

pub(super) fn show(app: &App, ctx: &Context, draft: &mut Draft) -> (bool, bool) {
    let bounds = viewport_bounds(ctx).shrink(8.0);
    let width = 250.0_f32.min(bounds.width());
    let height = if draft.fields.len() > 1 { 230.0 } else { 190.0 };
    let position = draft
        .position
        .unwrap_or(bounds.center() - egui::vec2(width * 0.5, height * 0.5));
    let position = egui::pos2(
        position
            .x
            .clamp(bounds.left(), (bounds.right() - width).max(bounds.left())),
        position
            .y
            .clamp(bounds.top(), (bounds.bottom() - height).max(bounds.top())),
    );
    draft.position = Some(position);
    let mut result = (false, false);
    egui::Area::new(id())
        .order(egui::Order::Foreground)
        .fixed_pos(position)
        .constrain_to(bounds)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_width(width - 16.0);
                egui::ScrollArea::vertical()
                    .max_height(bounds.height().min(300.0))
                    .show(ui, |ui| {
                        result = contents(app, ui, draft);
                    });
            });
        });
    result
}

fn contents(app: &App, ui: &mut egui::Ui, draft: &mut Draft) -> (bool, bool) {
    let pending = draft.submission.is_some();
    ui.strong("Create precisely");
    ui.small("Only changed fields become dimensions.");
    ui.small("Tab changes fields. Enter creates. Escape cancels.");
    if draft.tool == SketchTool::Arc {
        ui.small("Sweep sets geometry only.");
    }
    edit_fields(ui, draft, pending);
    let parsed = fields::dimensions(draft, app).and_then(|dimensions| {
        let session = app.sketch.session.as_ref().ok_or("Open a sketch first")?;
        dimensions.preview_for_session(session)?;
        Ok(dimensions)
    });
    if let Some(error) = draft.error.as_ref().or_else(|| parsed.as_ref().err()) {
        ui.colored_label(ui.visuals().error_fg_color, error);
    }
    if pending {
        ui.label("Creating…");
    }
    let ready = !pending && parsed.is_ok() && !app.composing;
    let enter = ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, Key::Enter));
    let cancel = ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, Key::Escape));
    let mut result = (ready && enter, cancel && !app.composing);
    ui.horizontal(|ui| {
        result.0 |= ui.add_enabled(ready, egui::Button::new("Create")).clicked();
        result.1 |= ui.button("Cancel").clicked();
    });
    result
}

fn edit_fields(ui: &mut egui::Ui, draft: &mut Draft, pending: bool) {
    cycle(ui, draft, pending);
    let ids: Vec<_> = draft
        .fields
        .iter()
        .map(|field| field_id(draft, field.field))
        .collect();
    for (index, field) in draft.fields.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.label(label(field.field));
            let response = ui.add_enabled(
                !pending,
                egui::TextEdit::singleline(&mut field.text)
                    .id(ids[index])
                    .desired_width(135.0)
                    .char_limit(onecad_core::expr::MAX_SOURCE_LEN),
            );
            if draft.focus == Some(index) && !pending {
                response.request_focus()
            }
            if response.changed() {
                field.authored = true;
                draft.error = None;
            }
        });
    }
    draft.focus = None;
}

fn cycle(ui: &egui::Ui, draft: &mut Draft, pending: bool) {
    if pending || draft.fields.is_empty() {
        return;
    }
    let backwards = ui.input_mut(|input| input.consume_key(egui::Modifiers::SHIFT, Key::Tab));
    let forward =
        !backwards && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, Key::Tab));
    if !backwards && !forward {
        return;
    }
    let focused = ui.ctx().memory(|memory| memory.focused());
    let index = draft
        .fields
        .iter()
        .position(|field| Some(field_id(draft, field.field)) == focused)
        .unwrap_or(0);
    let count = draft.fields.len();
    draft.focus = Some(if backwards {
        (index + count - 1) % count
    } else {
        (index + 1) % count
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft() -> Draft {
        let key = SketchSessionKey {
            document_id: "numeric-test".into(),
            runtime_session: "visit".into(),
            sketch_id: onecad_core::ids::SketchId::new(),
            generation: 4,
        };
        Draft {
            key,
            document: DocumentContext {
                document_id: "numeric-test".into(),
                runtime_session: "visit".into(),
                revision: 0,
                snapshot_id: onecad_core::ids::SnapshotId(0),
                scene: onecad_scene::SceneIdentity {
                    session: 1,
                    scene: 0,
                },
            },
            tool: SketchTool::Rectangle,
            method: NumericCreationMethod::Tool(SketchTool::Rectangle),
            anchor: Vec2::new_unchecked(0.0, 0.0),
            secondary: None,
            generation: 7,
            sequence: 7,
            fields: vec![
                FieldDraft {
                    field: CreationField::Width,
                    text: "80".into(),
                    authored: false,
                },
                FieldDraft {
                    field: CreationField::Height,
                    text: "50".into(),
                    authored: false,
                },
            ],
            focus: Some(0),
            position: None,
            submission: None,
            error: None,
        }
    }

    fn frame(ctx: &Context, draft: &mut Draft, events: Vec<egui::Event>) {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 560.0),
                )),
                focused: true,
                events,
                ..Default::default()
            },
            |ui| {
                edit_fields(ui, draft, false);
            },
        );
        output.textures_delta.clear();
    }

    fn key(key: Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    #[test]
    fn tab_and_reverse_tab_cycle_only_fields_without_authorship() {
        let ctx = Context::default();
        let mut draft = draft();
        frame(&ctx, &mut draft, vec![]);
        frame(&ctx, &mut draft, vec![key(Key::Tab, egui::Modifiers::NONE)]);
        assert_eq!(
            ctx.memory(|memory| memory.focused()),
            Some(field_id(&draft, CreationField::Height))
        );
        frame(
            &ctx,
            &mut draft,
            vec![key(Key::Tab, egui::Modifiers::SHIFT)],
        );
        assert_eq!(
            ctx.memory(|memory| memory.focused()),
            Some(field_id(&draft, CreationField::Width))
        );
        assert!(draft.fields.iter().all(|field| !field.authored));
    }

    #[test]
    fn typing_authors_only_changed_field_and_clears_refusal() {
        let ctx = Context::default();
        let mut draft = draft();
        draft.error = Some("Previous solver refusal".into());
        frame(&ctx, &mut draft, vec![]);
        frame(&ctx, &mut draft, vec![egui::Event::Text("9".into())]);
        assert!(draft.fields[0].authored);
        assert!(!draft.fields[1].authored);
        assert!(draft.error.is_none());
    }

    #[test]
    fn field_identity_stays_stable_across_refusal_sequences() {
        let mut draft = draft();
        let before = field_id(&draft, CreationField::Width);
        draft.sequence += 1;
        draft.document.revision += 1;
        draft.position = Some(egui::pos2(100.0, 100.0));
        assert_eq!(before, field_id(&draft, CreationField::Width));
        draft.generation += 1;
        assert_ne!(before, field_id(&draft, CreationField::Width));
    }
}
