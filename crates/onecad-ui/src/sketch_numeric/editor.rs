use super::callouts::{self, Callout};
use super::*;
use crate::dimension_chip::annotation;

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

/// Prefix and unit sit inside the box; position and these carry field identity.
fn affixes(field: CreationField) -> (&'static str, &'static str) {
    match field {
        CreationField::X => ("X", "mm"),
        CreationField::Y => ("Y", "mm"),
        CreationField::Length | CreationField::Width | CreationField::Height => ("", "mm"),
        CreationField::Diameter => ("Ø", "mm"),
        CreationField::Radius => ("R", "mm"),
        CreationField::Sweep => ("", "°"),
    }
}

fn accessible_name(field: CreationField) -> &'static str {
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

fn validated(app: &App, draft: &Draft) -> Result<(), String> {
    let dimensions = fields::dimensions(draft, app)?;
    let session = app
        .doc
        .sketch
        .session
        .as_ref()
        .ok_or("Open a sketch first")?;
    dimensions.preview_for_session(session)?;
    Ok(())
}

pub(super) fn show(app: &App, ctx: &Context, draft: &mut Draft) -> (bool, bool) {
    let bounds = crate::dimension_chip::usable(ctx, &app.ui).intersect(viewport_bounds(ctx));
    if !draft.engaged {
        draft.placement_anchor = app
            .doc
            .sketch
            .session
            .as_ref()
            .and_then(|session| session.draft.as_ref())
            .map_or(draft.anchor, |draft| draft.cursor);
    }
    let callouts = callouts::build(app, ctx, draft, bounds);
    let painter = ctx
        .layer_painter(egui::LayerId::new(
            egui::Order::Middle,
            id().with("graphics"),
        ))
        .with_clip_rect(bounds);
    let stroke = egui::Stroke::new(1.0, ctx.global_style().visuals.weak_text_color());
    for graphic in callouts
        .iter()
        .filter_map(|callout| callout.graphic.as_ref())
    {
        annotation::paint(&painter, graphic, stroke);
    }
    let pending = draft.submission.is_some();
    let message = draft.error.clone().or_else(|| validated(app, draft).err());
    edit_fields(ctx, draft, pending, &callouts, bounds, message.as_deref());
    if !draft.engaged {
        return (false, false);
    }
    let ready = !pending && validated(app, draft).is_ok() && !app.composing;
    let enter = ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, Key::Enter));
    let cancel = ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, Key::Escape));
    (ready && enter, cancel && !app.composing)
}

fn text_width(ui: &egui::Ui, text: &str) -> f32 {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let color = ui.visuals().text_color();
    ui.painter()
        .layout_no_wrap(text.to_owned(), font, color)
        .size()
        .x
}

fn edit_fields(
    ctx: &Context,
    draft: &mut Draft,
    pending: bool,
    callouts: &[Callout],
    bounds: egui::Rect,
    message: Option<&str>,
) {
    if draft.engaged {
        cycle(ctx, draft, pending);
    }
    let ids: Vec<_> = draft
        .fields
        .iter()
        .map(|field| field_id(draft, field.field))
        .collect();
    let message_at = ids
        .iter()
        .position(|id| ctx.memory(|memory| memory.has_focus(*id)))
        .or(draft.focus)
        .unwrap_or(0);
    for (index, field) in draft.fields.iter_mut().enumerate() {
        let center = callouts
            .get(index)
            .map_or(bounds.center(), |callout| callout.center);
        let (prefix, unit) = affixes(field.field);
        let chip = egui::Area::new(ids[index].with("chip"))
            .order(egui::Order::Foreground)
            .pivot(egui::Align2::CENTER_CENTER)
            .fixed_pos(center)
            .constrain_to(bounds)
            .movable(false)
            .show(ctx, |ui| {
                let focused = ui.memory(|memory| memory.has_focus(ids[index]));
                crate::dimension_chip::value_frame(ui, focused).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        if !prefix.is_empty() {
                            ui.label(egui::RichText::new(prefix).small().weak());
                        }
                        let width = (text_width(ui, &field.text) + 8.0).max(40.0);
                        let response = ui.add_enabled(
                            !pending,
                            egui::TextEdit::singleline(&mut field.text)
                                .id(ids[index])
                                .frame(egui::Frame::NONE)
                                .desired_width(width)
                                .char_limit(onecad_core::expr::MAX_SOURCE_LEN),
                        );
                        // Visible affixes are terse; assistive tech still needs the full name.
                        response.widget_info(|| {
                            let mut info =
                                egui::WidgetInfo::text_edit(!pending, &field.text, &field.text, "");
                            info.label = Some(accessible_name(field.field).into());
                            info
                        });
                        ui.label(egui::RichText::new(unit).small().weak());
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
                });
            });
        if index == message_at && draft.engaged {
            note(
                ctx,
                ids[index],
                chip.response.rect,
                bounds,
                pending,
                message,
            );
        }
    }
    draft.focus = None;
}

/// A refusal or pending state hangs under the focused chip instead of widening it.
fn note(
    ctx: &Context,
    id: egui::Id,
    chip: egui::Rect,
    bounds: egui::Rect,
    pending: bool,
    message: Option<&str>,
) {
    if !pending && message.is_none() {
        return;
    }
    egui::Area::new(id.with("note"))
        .order(egui::Order::Foreground)
        .pivot(egui::Align2::CENTER_TOP)
        .fixed_pos(chip.center_bottom() + egui::vec2(0.0, 4.0))
        .constrain_to(bounds)
        .movable(false)
        .show(ctx, |ui| {
            ui.set_max_width(220.0);
            if pending {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.small("Creating…");
                });
            } else if let Some(message) = message {
                crate::dimension_chip::error(ui, Some(&message.to_owned()));
            }
        });
}

fn cycle(ctx: &Context, draft: &mut Draft, pending: bool) {
    if pending || draft.fields.is_empty() {
        return;
    }
    let backwards = ctx.input_mut(|input| input.consume_key(egui::Modifiers::SHIFT, Key::Tab));
    let forward =
        !backwards && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, Key::Tab));
    if !backwards && !forward {
        return;
    }
    let focused = ctx.memory(|memory| memory.focused());
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
pub(super) mod tests {
    use super::*;

    pub(in crate::sketch_numeric) fn draft() -> Draft {
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
                    measured: 80.0,
                    authored: false,
                },
                FieldDraft {
                    field: CreationField::Height,
                    text: "50".into(),
                    measured: 50.0,
                    authored: false,
                },
            ],
            focus: Some(0),
            engaged: true,
            placement_anchor: Vec2::new_unchecked(80.0, 50.0),
            submission: None,
            error: None,
        }
    }

    fn bounds() -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(600.0, 400.0))
    }

    fn callouts() -> Vec<Callout> {
        [300.0, 450.0]
            .map(|x| Callout {
                center: egui::pos2(x, 300.0),
                graphic: None,
            })
            .into()
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
            |ui| edit_fields(ui.ctx(), draft, false, &callouts(), bounds(), None),
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
            |ui| edit_fields(ui.ctx(), &mut draft, true, &callouts(), bounds(), None),
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
        assert_eq!(before, field_id(&draft, CreationField::Width));
        draft.generation += 1;
        assert_ne!(before, field_id(&draft, CreationField::Width));
    }
}
