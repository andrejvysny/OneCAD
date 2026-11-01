//! Feature-card rows of a running placement: live designation, free parameters,
//! Linked/Copy, flip and the snap status.
use super::{ComponentPlacement, autosize, snap};
use crate::{
    app::App,
    library_catalog::draft::{Field, Kind},
};
use onecad_core::document::record::ComponentParamValue as Value;
use onecad_interaction::{ModelSession, ModelingOptions, SessionPhase};
use onecad_widgets::{
    components::{inputs::NumberField, overlays::FeatureCardRows},
    theme::look,
};

/// What the user did this frame.
#[derive(Default)]
struct Edits {
    parameter: Option<String>,
    copy: Option<bool>,
    flip: bool,
    cycle: bool,
}

pub(crate) fn rows(app: &mut App, rows: &mut FeatureCardRows, tool: &ModelSession) {
    if !super::is_placement(tool) || tool.phase != SessionPhase::Editing {
        return;
    }
    let Some(mut p) = app.doc.placement.take() else {
        return;
    };
    let edits = draw(rows, &mut p);
    let params = edits.parameter.and_then(|key| accept(&mut p, key));
    if edits.flip {
        p.flipped = !p.flipped;
    }
    if edits.cycle {
        p.attachment_cursor += 1;
    }
    app.doc.placement = Some(p);
    if let Some(map) = params {
        super::send(app, ModelingOptions::ComponentParams(map));
    }
    if let Some(copy) = edits.copy {
        super::set_copy(app, copy);
    }
    if edits.flip || edits.cycle {
        super::reconcile::reconcile(app);
    }
}

/// A valid edit becomes the new full map and wins over automatic sizing.
fn accept(
    p: &mut ComponentPlacement,
    key: String,
) -> Option<std::collections::BTreeMap<String, Value>> {
    let map = p.draft.resolve().ok()?;
    p.manual_params.insert(key.clone());
    if key == autosize::THREAD {
        p.auto_note = None;
    }
    p.params = map.clone();
    Some(map)
}

fn draw(rows: &mut FeatureCardRows, p: &mut ComponentPlacement) -> Edits {
    let mut edits = Edits::default();
    let designation = p
        .draft
        .designation()
        .unwrap_or_else(|| p.component.name.clone());
    rows.row("Part", |ui| {
        ui.label(designation);
    });
    let fields: Vec<Field> = p
        .draft
        .fields()
        .iter()
        .filter(|f| f.is_free())
        .cloned()
        .collect();
    let errors = p.draft.errors();
    for field in &fields {
        let error = errors.get(&field.key).cloned();
        rows.row(&field.key, |ui| {
            if value_cell(&mut p.draft, field, error.as_deref(), ui) {
                edits.parameter = Some(field.key.clone());
            }
        });
    }
    if p.editing.is_none() {
        rows.row("Mode", |ui| {
            let mut copy = p.copy;
            ui.horizontal(|ui| {
                ui.selectable_value(&mut copy, false, "Linked");
                ui.selectable_value(&mut copy, true, "Copy");
            });
            if copy != p.copy {
                edits.copy = Some(copy);
            }
        });
    } else {
        rows.row("Mode", |ui| {
            ui.label("Linked");
        });
    }
    seat_rows(rows, p, &mut edits);
    let mut status = match (&p.sent, p.editing.is_some()) {
        (Some(_), _) => snap::describe(&p.snap, p.flipped),
        (None, true) => "Current placement; hover the model to re-place".to_owned(),
        (None, false) => "Hover the model to place".to_owned(),
    };
    if let Some(note) = &p.auto_note {
        status = format!("{status} \u{2014} {note}");
    }
    rows.row("Snap", |ui| {
        ui.add(egui::Label::new(status).wrap());
    });
    if let Some(notice) = p.notice.clone() {
        rows.row("Notice", |ui| {
            ui.add(egui::Label::new(notice).wrap());
        });
    }
    edits
}

fn seat_rows(rows: &mut FeatureCardRows, p: &ComponentPlacement, edits: &mut Edits) {
    let mated = matches!(p.snap, snap::SnapIntent::Mate(_));
    rows.row("Orientation", |ui| {
        edits.flip = ui
            .add_enabled(mated, egui::Button::new("Flip  A"))
            .clicked();
    });
    if let (snap::SnapIntent::Mate(c), true) = (&p.snap, p.compatible > 1) {
        let name = c.attachment.clone();
        rows.row("Attachment", |ui| {
            ui.label(name);
            edits.cycle = ui.button("Next  Tab").clicked();
        });
    }
}

fn value_cell(
    draft: &mut crate::library_catalog::draft::ParamDraft,
    field: &Field,
    error: Option<&str>,
    ui: &mut egui::Ui,
) -> bool {
    let before = draft.text(&field.key).to_owned();
    let mut text = before.clone();
    ui.vertical(|ui| {
        if field.kind == Kind::Bool || !field.domain.is_empty() {
            let options: Vec<String> = if field.kind == Kind::Bool {
                vec!["true".into(), "false".into()]
            } else {
                field.domain.iter().map(|d| d.text()).collect()
            };
            egui::ComboBox::from_id_salt(("place-param", &field.key))
                .selected_text(text.clone())
                .show_ui(ui, |ui| {
                    for option in &options {
                        ui.selectable_value(&mut text, option.clone(), option);
                    }
                });
        } else if field.kind == Kind::Number {
            ui.add(NumberField::new(("place-param", &field.key), &mut text).error(error));
        } else {
            ui.add(egui::TextEdit::singleline(&mut text));
        }
        if let Some(error) = error {
            ui.colored_label(look(ui.ctx()).theme.warn, error);
        }
    });
    draft.set_text(&field.key, text.clone());
    text != before
}
