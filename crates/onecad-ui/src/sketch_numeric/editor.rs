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
        CreationField::Length => "Length mm",
        CreationField::Width => "Width mm",
        CreationField::Height => "Height mm",
        CreationField::Diameter => "Ø mm",
        CreationField::Radius => "R mm",
        CreationField::Sweep => "Sweep °",
    }
}

pub(super) fn show(app: &App, ctx: &Context, draft: &mut Draft) -> (bool, bool) {
    let bounds = crate::dimension_chip::usable(ctx, &app.ui).intersect(viewport_bounds(ctx));
    let point = if draft.engaged {
        draft.placement_anchor
    } else {
        app.sketch
            .session
            .as_ref()
            .and_then(|session| session.draft.as_ref())
            .map_or(draft.anchor, |draft| draft.cursor)
    };
    if !draft.engaged {
        draft.placement_anchor = point;
    }
    let anchor = project(app, ctx, point).unwrap_or(bounds.center());
    let height = 12.0 + draft.fields.len() as f32 * 26.0 + if draft.engaged { 52.0 } else { 0.0 };
    let size = egui::vec2(crate::dimension_chip::WIDTH.min(bounds.width()), height);
    let hovering = draft.position.is_some_and(|position| {
        ctx.input(|input| input.pointer.hover_pos())
            .is_some_and(|at| egui::Rect::from_min_size(position, size).contains(at))
    });
    let position = if !draft.engaged && hovering {
        draft
            .position
            .unwrap_or_else(|| crate::dimension_chip::place(anchor, size, bounds))
    } else {
        crate::dimension_chip::place(anchor, size, bounds)
    };
    draft.position = Some(position);
    let mut result = (false, false);
    let response = egui::Area::new(id())
        .order(egui::Order::Foreground)
        .fixed_pos(position)
        .default_size(size)
        .constrain_to(bounds)
        .movable(false)
        .show(ctx, |ui| {
            crate::dimension_chip::frame(ui.style()).show(ui, |ui| {
                ui.set_width((size.x - 12.0).max(1.0));
                result = contents(app, ui, draft);
            });
        });
    crate::dimension_chip::witness(ctx, id().with("witness"), anchor, response.response.rect);
    result
}

fn contents(app: &App, ui: &mut egui::Ui, draft: &mut Draft) -> (bool, bool) {
    let pending = draft.submission.is_some();
    edit_fields(ui, draft, pending);
    if !draft.engaged {
        return (false, false);
    }
    let parsed = fields::dimensions(draft, app).and_then(|dimensions| {
        let session = app.sketch.session.as_ref().ok_or("Open a sketch first")?;
        dimensions.preview_for_session(session)?;
        Ok(dimensions)
    });
    crate::dimension_chip::error(ui, draft.error.as_ref().or_else(|| parsed.as_ref().err()));
    let ready = !pending && parsed.is_ok() && !app.composing;
    let enter = ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, Key::Enter));
    let cancel = ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, Key::Escape));
    let mut result = (ready && enter, cancel && !app.composing);
    ui.horizontal(|ui| {
        if pending {
            ui.spinner();
            ui.small("Creating…");
        } else {
            result.0 |= ui
                .add_enabled(ready, egui::Button::new("Create").small())
                .clicked();
        }
        result.1 |= ui.small_button("Cancel").clicked();
    });
    result
}

fn edit_fields(ui: &mut egui::Ui, draft: &mut Draft, pending: bool) {
    if draft.engaged {
        cycle(ui, draft, pending);
    }
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
                    .desired_width(ui.available_width().clamp(24.0, 104.0))
                    .char_limit(onecad_core::expr::MAX_SOURCE_LEN),
            );
            if draft.focus == Some(index) && !pending {
                response.request_focus()
            }
            if response.has_focus() || response.clicked() {
                draft.engaged = true;
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
            engaged: true,
            placement_anchor: Vec2::new_unchecked(80.0, 50.0),
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
    fn measurements_stay_unfocused_and_do_not_author_fields() {
        let ctx = Context::default();
        let mut draft = draft();
        draft.engaged = false;
        draft.focus = None;
        frame(&ctx, &mut draft, vec![]);
        assert!(!draft.engaged);
        assert!(ctx.memory(|memory| memory.focused()).is_none());
        assert!(draft.fields.iter().all(|field| !field.authored));
        ctx.data_mut(|data| data.insert_temp(id(), draft));
        assert!(!editor_open(&ctx));
    }

    #[test]
    fn pending_fields_cannot_change_authorship_or_focus() {
        let ctx = Context::default();
        let mut draft = draft();
        draft.focus = None;
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![
                    egui::Event::Text("123".into()),
                    key(Key::Tab, egui::Modifiers::NONE),
                ],
                ..Default::default()
            },
            |ui| edit_fields(ui, &mut draft, true),
        );
        output.textures_delta.clear();
        assert!(draft.fields.iter().all(|field| !field.authored));
        assert!(draft.focus.is_none());
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
