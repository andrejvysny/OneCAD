//! Parameters tab: the component's parameter table with a validated "Try parameters" draft.
use super::kit;
use crate::library_catalog::{
    Catalog, PreviewState, Status,
    draft::{Field, Kind, ParamDraft},
};
use egui::{FontFamily, Ui};
use onecad_widgets::{
    components::{
        button::{Button, ButtonKind},
        inputs::{NumberField, Select},
    },
    theme::look,
};

const VALUE_W: f32 = 150.0;

pub fn show(catalog: &mut Catalog, ui: &mut Ui) {
    let Some(view) = catalog.detail() else { return };
    if view.draft.is_none() {
        let note = match &view.status {
            Status::Failed(error) => format!("Parameters unavailable: {error}"),
            _ => "Loading parameters…".to_owned(),
        };
        kit::muted(ui, &note, 12.5);
        return;
    }
    let unit = view.component.as_ref().and_then(|c| c.unit.clone());
    let edited = catalog
        .detail_mut()
        .and_then(|v| v.draft.as_mut())
        .is_some_and(|draft| table(draft, unit.as_deref(), ui));
    if edited {
        catalog.draft_edited();
    }
    ui.add_space(14.0);
    footer(catalog, ui);
}

/// Draws every parameter row; true when a field changed.
fn table(draft: &mut ParamDraft, unit: Option<&str>, ui: &mut Ui) -> bool {
    let errors = draft.errors();
    let fields: Vec<Field> = draft.fields().to_vec();
    let mut edited = false;
    egui::Grid::new("params-grid")
        .num_columns(4)
        .spacing([18.0, 10.0])
        .show(ui, |ui| {
            for text in ["NAME", "VALUE", "ROLE", "ALLOWED"] {
                let colour = look(ui.ctx()).theme.ink5;
                kit::label(ui, text, 11.0, FontFamily::Proportional, colour);
            }
            ui.end_row();
            for field in &fields {
                let error = errors.get(&field.key).map(String::as_str);
                kit::label(
                    ui,
                    &field.key,
                    12.5,
                    FontFamily::Monospace,
                    look(ui.ctx()).theme.ink,
                );
                edited |= value_cell(draft, field, unit, error, ui);
                kit::muted(ui, field.role_label(), 12.0);
                kit::muted(ui, &allowed(field), 12.0);
                ui.end_row();
            }
        });
    edited
}

pub(crate) fn value_cell(
    draft: &mut ParamDraft,
    field: &Field,
    unit: Option<&str>,
    error: Option<&str>,
    ui: &mut Ui,
) -> bool {
    if !field.is_free() {
        locked_value(field, ui);
        return false;
    }
    let id = ("param", &field.key);
    let mut edited = false;
    ui.vertical(|ui| {
        match field.kind {
            Kind::Bool => edited = choose(draft, field, &["true", "false"], ui),
            _ if !field.domain.is_empty() => {
                let options: Vec<String> = field.domain.iter().map(|d| d.text()).collect();
                let options: Vec<&str> = options.iter().map(String::as_str).collect();
                edited = choose(draft, field, &options, ui);
            }
            Kind::Number => {
                if let Some(text) = draft.text_mut(&field.key) {
                    let before = text.clone();
                    let places = decimals(text);
                    let mut number = NumberField::new(id, text)
                        .width(VALUE_W)
                        .decimals(places)
                        .error(error);
                    if let Some(unit) = unit {
                        number = number.unit(unit);
                    }
                    ui.add(number);
                    edited = *text != before;
                }
            }
            Kind::Text => {
                if let Some(text) = draft.text_mut(&field.key) {
                    let before = text.clone();
                    ui.add(egui::TextEdit::singleline(text).desired_width(VALUE_W));
                    edited = *text != before;
                }
            }
        }
        if let Some(error) = error {
            let warn = look(ui.ctx()).theme.warn;
            kit::label(ui, error, 11.5, FontFamily::Proportional, warn);
        }
    });
    edited
}

fn choose(draft: &mut ParamDraft, field: &Field, options: &[&str], ui: &mut Ui) -> bool {
    let current = draft.text(&field.key).to_owned();
    let shown = if current.is_empty() {
        "Choose…"
    } else {
        &current
    };
    let out = Select::new(("param", &field.key), shown, options)
        .width(VALUE_W)
        .show(ui);
    match out.chosen {
        Some(i) => {
            draft.set_text(&field.key, options[i].to_owned());
            true
        }
        None => false,
    }
}

pub(crate) fn locked_value(field: &Field, ui: &mut Ui) {
    let text = match (&field.default, &field.from) {
        (Some(_), _) => field.default_text(),
        (None, Some(from)) => format!("from table: {from}"),
        (None, None) => "derived".to_owned(),
    };
    kit::muted(ui, &text, 12.5).on_hover_text("Set by the component; not editable");
}

fn allowed(field: &Field) -> String {
    if !field.is_free() {
        return String::new();
    }
    let mut parts = Vec::new();
    if !field.domain.is_empty() {
        let shown: Vec<String> = field.domain.iter().take(6).map(|d| d.text()).collect();
        let more = if field.domain.len() > 6 { ", …" } else { "" };
        parts.push(format!("{}{more}", shown.join(", ")));
    }
    if let Some(min) = field.min {
        parts.push(format!(
            "≥ {}",
            crate::library_catalog::draft::format_number(min)
        ));
    }
    if parts.is_empty() {
        parts.push(field.kind.label().to_owned());
    }
    parts.join(" · ")
}

/// Display precision that keeps the typed text as-is when the field loses focus.
fn decimals(text: &str) -> usize {
    if text.contains(['e', 'E']) {
        return 9;
    }
    text.split_once('.').map_or(0, |(_, fraction)| {
        fraction.chars().filter(char::is_ascii_digit).count()
    })
}

fn footer(catalog: &mut Catalog, ui: &mut Ui) {
    let Some(view) = catalog.detail() else { return };
    let Some(draft) = &view.draft else { return };
    let valid = draft.resolve().is_ok();
    let designation = draft.designation();
    let is_default = draft.is_default();
    if let Some(designation) = designation {
        ui.horizontal(|ui| {
            kit::muted(ui, "Designation", 12.0);
            let ink = look(ui.ctx()).theme.ink;
            kit::label(ui, &designation, 13.0, FontFamily::Monospace, ink);
            if !valid {
                kit::muted(
                    ui,
                    "· incomplete until the highlighted fields are fixed",
                    12.0,
                );
            }
        });
        ui.add_space(8.0);
    }
    ui.horizontal(|ui| {
        let running = matches!(catalog.preview(), PreviewState::Running);
        let preview = Button::new(if running { "Previewing…" } else { "Preview" })
            .kind(ButtonKind::Primary)
            .enabled(valid && !running);
        let r = ui.add(preview);
        let hint = if valid {
            "Builds a throwaway model of these parameters; nothing is placed"
        } else {
            "Fix the highlighted fields first"
        };
        if (if valid {
            r.on_hover_text(hint)
        } else {
            r.on_disabled_hover_text(hint)
        })
        .clicked()
        {
            catalog.request_preview();
        }
        let reset = Button::new("Reset to defaults")
            .kind(ButtonKind::Ghost)
            .enabled(!is_default);
        if ui.add(reset).clicked() {
            if let Some(draft) = catalog.detail_mut().and_then(|v| v.draft.as_mut()) {
                draft.reset();
            }
            catalog.draft_edited();
        }
    });
    preview_result(catalog, ui);
}

fn preview_result(catalog: &Catalog, ui: &mut Ui) {
    ui.add_space(8.0);
    match catalog.preview() {
        PreviewState::Idle | PreviewState::Running => {}
        PreviewState::Done(summary) => {
            let [x, y, z] = summary
                .size
                .map(crate::library_catalog::draft::format_dimension);
            let bodies = if summary.bodies == 1 {
                "body"
            } else {
                "bodies"
            };
            let text = format!(
                "Preview: {x} × {y} × {z} mm bounding box · {} {bodies} · {} triangles",
                summary.bodies, summary.triangles
            );
            kit::muted(ui, &text, 12.5);
        }
        PreviewState::Failed(error) => {
            let warn = look(ui.ctx()).theme.warn;
            kit::label(
                ui,
                &format!("Preview failed: {error}"),
                12.5,
                FontFamily::Proportional,
                warn,
            );
        }
    }
}
