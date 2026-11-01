//! Wizard step bodies: Source, Details, Parameters, Attachments/Save.
use super::model::{Phase, WizardState, usable};
use crate::library_catalog::draft::format_number;
use crate::library_pages::kit;
use egui::{CornerRadius, FontFamily, FontId, Response, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use onecad_app::dto::VariableDto;
use onecad_widgets::{
    components::button::{Button, ButtonKind},
    fonts,
    theme::look,
};

pub(crate) fn show(ui: &mut Ui, w: &mut WizardState, variables: &[VariableDto]) {
    match w.phase {
        Phase::Saved(_) => return saved(ui, w),
        Phase::Saving => {
            ui.horizontal(|ui| {
                ui.spinner();
                kit::muted(ui, "Saving the component…", 12.5);
            });
            return;
        }
        _ => {}
    }
    match w.step {
        0 => source(ui, w),
        1 => details(ui, w),
        2 => parameters(ui, w, variables),
        _ => attachments(ui, w),
    }
}

fn caption(ui: &mut Ui, text: &str) {
    let ink = look(ui.ctx()).theme.ink4;
    kit::label(ui, text, 12.0, FontFamily::Proportional, ink);
}

fn warn(ui: &mut Ui, text: &str) {
    let ink = look(ui.ctx()).theme.warn;
    kit::label(ui, text, 12.0, FontFamily::Proportional, ink);
}

/// A single-line field with an optional refusal underneath; true when it changed.
fn field(ui: &mut Ui, id: &str, text: &mut String, width: f32, error: Option<&str>) -> bool {
    let id = ui.make_persistent_id(id);
    let before = text.clone();
    onecad_widgets::text_session::show(ui, id, text, "Component field", 16_384, |text| {
        egui::TextEdit::singleline(text)
            .font(FontId::new(12.5, fonts::mono()))
            .margin(vec2(10.0, 6.0))
            .desired_width(width)
    });
    if let Some(error) = error {
        warn(ui, error);
    }
    *text != before
}

fn source(ui: &mut Ui, w: &mut WizardState) {
    ui.spacing_mut().item_spacing.y = 10.0;
    kit::title(ui, "What goes into the component", 14.0);
    kit::muted(ui, &format!("1 body: {}", w.body_name), 12.5);
    kit::muted(
        ui,
        "The component freezes this document with its variables and bakes the body as it is now.",
        12.0,
    );
    ui.add_space(6.0);
    caption(ui, "Type");
    let width = ui.available_width();
    let half = (width - 12.0) / 2.0;
    let cards = [
        ("Static", "Fixed geometry. Inserts exactly as modeled."),
        ("Parametric", "Expose document variables as editable sizes."),
    ];
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        for (i, (title, text)) in cards.into_iter().enumerate() {
            if radio_card(ui, half, w.parametric == (i == 1), title, text).clicked() {
                w.parametric = i == 1;
            }
        }
    });
}

fn details(ui: &mut Ui, w: &mut WizardState) {
    use super::validate::{id_error, name_error, version_error};
    let width = ui.available_width();
    ui.spacing_mut().item_spacing.y = 5.0;
    caption(ui, "Name");
    let error = name_error(&w.name);
    if field(ui, "wizard-name", &mut w.name, width, error.as_deref()) {
        w.follow_name();
    }
    ui.add_space(6.0);
    let half = (width - 12.0) / 2.0;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        ui.vertical(|ui| {
            caption(ui, "Id (namespaced, lower-case)");
            let error = id_error(&w.id);
            if field(ui, "wizard-id", &mut w.id, half, error.as_deref()) {
                w.id_edited = true;
            }
        });
        ui.vertical(|ui| {
            caption(ui, "Version");
            let error = version_error(&w.version);
            field(ui, "wizard-version", &mut w.version, half, error.as_deref());
        });
    });
    ui.add_space(6.0);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        ui.vertical(|ui| {
            caption(ui, "Category (comma separated)");
            field(ui, "wizard-category", &mut w.category, half, None);
        });
        ui.vertical(|ui| {
            caption(ui, "Tags (comma separated)");
            field(ui, "wizard-tags", &mut w.tags, half, None);
        });
    });
    ui.add_space(6.0);
    caption(
        ui,
        "Designation template (optional, e.g. Bracket {width}x{thickness})",
    );
    field(ui, "wizard-designation", &mut w.designation, width, None);
    ui.add_space(10.0);
    kit::muted(ui, "Saved to: your library (the only destination)", 12.0);
}

fn radio_card(ui: &mut Ui, width: f32, on: bool, title: &str, text: &str) -> Response {
    let t = look(ui.ctx()).theme;
    let (rect, r) = ui.allocate_exact_size(vec2(width, 80.0), Sense::click());
    let (fill, border) = match (on, r.hovered()) {
        (true, _) => (t.sel_soft, t.accent),
        (false, true) => (t.hover, t.border_strong),
        (false, false) => (t.surface, t.border),
    };
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(9), fill);
    p.rect_stroke(
        rect,
        CornerRadius::same(9),
        Stroke::new(1.0, border),
        StrokeKind::Inside,
    );
    let c = pos2(rect.left() + 22.0, rect.top() + 22.0);
    p.circle_stroke(
        c,
        8.0,
        Stroke::new(1.0, if on { t.accent } else { t.border_strong }),
    );
    if on {
        p.circle_filled(c, 4.5, t.accent);
    }
    let x = rect.left() + 40.0;
    p.text(
        pos2(x, rect.top() + 22.0),
        egui::Align2::LEFT_CENTER,
        title,
        FontId::new(13.0, fonts::semibold()),
        t.ink,
    );
    let g = p.layout(
        text.to_owned(),
        FontId::proportional(12.0),
        t.ink4,
        rect.right() - 12.0 - x,
    );
    p.galley(pos2(x, rect.top() + 38.0), g, t.ink4);
    r.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn parameters(ui: &mut Ui, w: &mut WizardState, variables: &[VariableDto]) {
    ui.spacing_mut().item_spacing.y = 6.0;
    if !w.parametric {
        kit::muted(ui, "A static component exposes no parameters.", 12.5);
        return;
    }
    kit::title(ui, "Document variables", 14.0);
    kit::muted(
        ui,
        "An exposed variable becomes an editable size; its current value is the default.",
        12.0,
    );
    let list: Vec<VariableDto> = usable(variables).cloned().collect();
    if list.is_empty() {
        warn(
            ui,
            "This document has no variables. Add one, or save a static component.",
        );
    }
    for variable in &list {
        ui.horizontal(|ui| {
            let mut on = w.exposed.contains(&variable.name);
            if ui.checkbox(&mut on, "").changed() {
                w.set_exposed(variables, &variable.name, on);
            }
            let ink = look(ui.ctx()).theme.ink;
            kit::label(ui, &variable.name, 12.5, FontFamily::Monospace, ink);
            let shown = variable
                .expr
                .as_ref()
                .map_or_else(|| format_number(variable.value), |expr| format!("= {expr}"));
            kit::muted(
                ui,
                &format!("{shown}  ({})", format_number(variable.resolved_value)),
                12.0,
            );
        });
    }
    configurations(ui, w, variables);
}

fn configurations(ui: &mut Ui, w: &mut WizardState, variables: &[VariableDto]) {
    if w.exposed.is_empty() {
        return;
    }
    ui.add_space(12.0);
    kit::title(ui, "Configurations (optional)", 14.0);
    kit::muted(
        ui,
        "Named variants that set the exposed values together.",
        12.0,
    );
    let keys: Vec<String> = w.exposed.iter().cloned().collect();
    let mut remove = None;
    let mut rename = None;
    for i in 0..w.configs.len() {
        ui.horizontal(|ui| {
            let mut name = w.configs[i].name.clone();
            if field(ui, &format!("cfg-name-{i}"), &mut name, 150.0, None) {
                rename = Some((i, name));
            }
            for key in &keys {
                caption(ui, key);
                if let Some(text) = w.configs[i].values.get_mut(key) {
                    field(ui, &format!("cfg-{i}-{key}"), text, 80.0, None);
                }
            }
            if kit::link(ui, "Remove", 12.0).clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some((i, name)) = rename {
        w.rename_config(i, name);
    }
    if let Some(i) = remove {
        w.remove_config(i);
    }
    if ui
        .add(Button::new("Add configuration").kind(ButtonKind::Ghost))
        .clicked()
    {
        w.add_config(variables);
    }
    default_config(ui, w);
}

fn default_config(ui: &mut Ui, w: &mut WizardState) {
    if w.configs.is_empty() {
        return;
    }
    let names: Vec<&str> = w.configs.iter().map(|c| c.name.as_str()).collect();
    let current = w
        .default_config
        .clone()
        .unwrap_or_else(|| "None".to_owned());
    let mut options = vec!["None"];
    options.extend(names);
    caption(ui, "Default configuration");
    let out = onecad_widgets::components::inputs::Select::new(
        "wizard-default-config",
        &current,
        &options,
    )
    .width(200.0)
    .show(ui);
    if let Some(i) = out.chosen {
        w.default_config = (i > 0).then(|| options[i].to_owned());
    }
}

fn attachments(ui: &mut Ui, w: &mut WizardState) {
    ui.spacing_mut().item_spacing.y = 6.0;
    kit::title(ui, "Attachments", 14.0);
    kit::muted(
        ui,
        "Mate points a placement can snap to. Pick a planar face, a cylinder or a circular edge.",
        12.0,
    );
    let mut remove = None;
    let mut rename = None;
    for (i, row) in w.attachments.iter().enumerate() {
        ui.horizontal(|ui| {
            let mut name = row.name.clone();
            if field(ui, &format!("att-{i}"), &mut name, 160.0, None) {
                rename = Some((i, name));
            }
            kit::muted(
                ui,
                &format!("{}  accepts {}", row.input.on, row.input.accepts.join(", ")),
                12.0,
            );
            if kit::link(ui, "Remove", 12.0).clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some((i, name)) = rename
        && let Err(error) = w.rename_attachment(i, &name)
    {
        w.error = Some(error);
    }
    if let Some(i) = remove {
        w.remove_attachment(i);
    }
    if w.attachments.is_empty() {
        kit::muted(
            ui,
            "No attachments yet; the component can still be saved.",
            12.0,
        );
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        caption(ui, "Name");
        field(ui, "att-new", &mut w.new_attachment, 160.0, None);
        let pick = Button::new("Pick face/edge").enabled(w.phase == Phase::Editing);
        if ui.add(pick).clicked()
            && let Err(error) = w.begin_pick()
        {
            w.error = Some(error);
        }
    });
    if w.phase == Phase::Resolving {
        ui.horizontal(|ui| {
            ui.spinner();
            kit::muted(ui, "Resolving the pick…", 12.0);
        });
    }
    if let Some(error) = &w.error {
        warn(ui, error);
    }
    ui.add_space(10.0);
    summary(ui, w);
}

fn summary(ui: &mut Ui, w: &WizardState) {
    let kind = if w.parametric { "Parametric" } else { "Static" };
    let rows = [
        ("Name", w.name.trim().to_owned()),
        ("Id", format!("{}@{}", w.id, w.version)),
        ("Type", kind.to_owned()),
        ("Exposed parameters", w.exposed.len().to_string()),
        ("Configurations", w.configs.len().to_string()),
        ("Saved to", "Your library".to_owned()),
    ];
    for (name, value) in rows {
        ui.horizontal(|ui| {
            caption(ui, &format!("{name}:"));
            let ink = look(ui.ctx()).theme.ink;
            kit::label(ui, &value, 12.5, FontFamily::Proportional, ink);
        });
    }
    if !w.attachments.is_empty() {
        kit::muted(
            ui,
            "Thumbnail: not captured; the library shows the part icon.",
            11.5,
        );
    }
}

fn saved(ui: &mut Ui, w: &WizardState) {
    let Phase::Saved(saved) = &w.phase else {
        return;
    };
    let ok = look(ui.ctx()).theme.ok_text;
    kit::label(
        ui,
        &format!("Saved {} {} to the library", saved.name, saved.version),
        14.0,
        fonts::semibold(),
        ok,
    );
    ui.add_space(6.0);
    kit::muted(ui, &format!("{}@{}", saved.id, saved.version), 12.5);
}
