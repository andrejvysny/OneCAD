//! Inspector panel for a selected placed component instance, and the detach confirmation.
use super::{
    Op,
    model::{self, ParamRow},
};
use crate::{
    app::App,
    library_pages::{kit, params},
};
use egui::{Context, FontFamily, Ui};
use onecad_app::dto::{ComponentInstanceDto, ComponentStatusDto};
use onecad_widgets::{
    components::{
        button::{Button, ButtonKind},
        lists::{PropertyRow, SectionLabel},
        overlays::{Dialog, DialogIntent},
    },
    theme::look,
};

pub(crate) const DETACH_WARNING: &str = "Detach breaks the link to the library. Parameters stop being editable and updates no longer apply. Geometry is kept.";

pub(crate) fn panel(app: &mut App, ui: &mut Ui) {
    let Some(record) = app.doc.selected_feature() else {
        return;
    };
    let key = record.to_string();
    let Some(dto) = app.doc.instances.find(&key).map(|v| v.dto.clone()) else {
        return;
    };
    ui.add_space(6.0);
    ui.add(SectionLabel::new("COMPONENT"));
    identity(app, ui, &dto);
    actions(app, ui, &dto);
    ui.add_space(6.0);
    ui.add(SectionLabel::new("PARAMETERS"));
    if model::editable(&dto) && app.doc.instances.edit_for(&key).is_some() {
        editable(app, ui, &dto);
    } else {
        read_only(app, ui, &dto);
    }
}

fn identity(app: &App, ui: &mut Ui, dto: &ComponentInstanceDto) {
    let source = if dto.linked {
        format!("{}@{}", dto.component_id, dto.component_version)
    } else {
        "Copy (no library link)".to_owned()
    };
    ui.add(PropertyRow::new("Source").value(&source));
    ui.add(PropertyRow::new("Kind").value(&dto.source_kind));
    if let Some(designation) = &dto.designation {
        ui.add(PropertyRow::new("Designation").value(designation));
    }
    let colour = if matches!(
        dto.status,
        ComponentStatusDto::Linked | ComponentStatusDto::Detached
    ) {
        app.theme.ink3
    } else {
        look(ui.ctx()).theme.warn
    };
    ui.add_space(4.0);
    kit::label(
        ui,
        &model::status_text(dto),
        12.0,
        FontFamily::Proportional,
        colour,
    );
}

fn actions(app: &mut App, ui: &mut Ui, dto: &ComponentInstanceDto) {
    ui.add_space(8.0);
    let reason = super::unavailable(app);
    ui.horizontal_wrapped(|ui| {
        if matches!(dto.status, ComponentStatusDto::UpgradeAvailable { .. })
            && ui.add(Button::new("Review updates")).clicked()
            && let Some(scope) = super::scope(app)
        {
            app.doc.instances.open_review(&scope);
        }
        if dto.linked {
            let detach = Button::new("Detach")
                .kind(ButtonKind::Danger)
                .enabled(reason.is_none());
            let response = ui.add(detach);
            if response.clicked() {
                app.doc.instances.confirm_detach = Some(dto.record_id.clone());
            }
            if let Some(reason) = reason {
                response.on_disabled_hover_text(reason);
            }
            let blocked = reason.or_else(|| crate::placement_gesture::gate(app));
            let edit = ui.add(Button::new("Edit placement").enabled(blocked.is_none()));
            if let Some(why) = blocked {
                edit.on_disabled_hover_text(why);
            } else if edit.clicked() {
                crate::placement_gesture::edit_existing(app, &dto.record_id);
            }
        }
    });
}

fn read_only(app: &App, ui: &mut Ui, dto: &ComponentInstanceDto) {
    let rows = model::param_rows(dto);
    if rows.is_empty() {
        kit::muted(ui, "This instance has no recorded parameters.", 12.0);
    }
    for ParamRow {
        key,
        effective,
        default,
        overridden,
    } in rows
    {
        let shown = match (&default, overridden) {
            (Some(default), true) => format!("{effective} (default {default})"),
            _ => effective,
        };
        ui.add(PropertyRow::new(&key).value(&shown));
    }
    if !dto.linked {
        return;
    }
    let _ = app;
    kit::muted(
        ui,
        "Parameters are read-only while the definition is unavailable.",
        11.5,
    );
}

fn editable(app: &mut App, ui: &mut Ui, dto: &ComponentInstanceDto) {
    let reason = super::unavailable(app);
    let Some(scope) = super::scope(app) else {
        return;
    };
    let revision = app.doc.document_state.as_ref().map_or(0, |s| s.revision);
    let settled = app.doc.instances.settled(revision);
    let unit = app
        .doc
        .instances
        .find(&dto.record_id)
        .and_then(|v| v.component.as_ref())
        .and_then(|c| c.unit.clone());
    let mut reset: Option<Option<Vec<String>>> = None;
    let mut commit = None;
    if let Some(edit) = app.doc.instances.edit_for(&dto.record_id) {
        let errors = edit.draft.errors();
        let fields = edit.draft.fields().to_vec();
        ui.add_enabled_ui(reason.is_none(), |ui| {
            for field in &fields {
                if !field.is_free() {
                    ui.horizontal(|ui| {
                        kit::label(
                            ui,
                            &field.key,
                            12.0,
                            FontFamily::Monospace,
                            look(ui.ctx()).theme.ink4,
                        );
                        params::locked_value(field, ui);
                    });
                    continue;
                }
                let overridden = dto.overrides.contains(&field.key);
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let ink = look(ui.ctx()).theme.ink;
                    kit::label(ui, &field.key, 12.5, FontFamily::Monospace, ink);
                    if overridden {
                        kit::chip(ui, "override", kit::ChipKind::Accent);
                    }
                });
                let error = errors.get(&field.key).map(String::as_str);
                params::value_cell(&mut edit.draft, field, unit.as_deref(), error, ui);
                if overridden {
                    override_line(ui, dto, &field.key, &mut reset);
                }
            }
        });
        if let Some(next) = model::pending_commit(&edit.draft, dto)
            && reason.is_none()
            && settled
            && !ui.ctx().text_edit_focused()
            && edit.attempted.as_ref() != Some(&next)
        {
            edit.attempted = Some(next.clone());
            commit = Some(next);
        }
    }
    if model::has_overrides(dto) {
        ui.add_space(6.0);
        let all = Button::new("Reset all")
            .kind(ButtonKind::Ghost)
            .enabled(reason.is_none());
        if ui.add(all).clicked() {
            reset = Some(None);
        }
    }
    if let Some(keys) = reset {
        let op = Op::Reset {
            record_id: dto.record_id.clone(),
            keys,
        };
        app.doc
            .instances
            .mutate(&scope, "Reset component parameters", op);
    } else if let Some(commit) = commit {
        let (what, op) = match commit {
            model::Commit::Set(params) => (
                "Edit component parameters",
                Op::SetParams {
                    record_id: dto.record_id.clone(),
                    params,
                },
            ),
            model::Commit::Reset(keys) => (
                "Reset component parameters",
                Op::Reset {
                    record_id: dto.record_id.clone(),
                    keys: Some(keys),
                },
            ),
        };
        app.doc.instances.mutate(&scope, what, op);
    }
}

fn override_line(
    ui: &mut Ui,
    dto: &ComponentInstanceDto,
    key: &str,
    reset: &mut Option<Option<Vec<String>>>,
) {
    ui.horizontal(|ui| {
        let default = dto.defaults.get(key).map(model::text).unwrap_or_default();
        kit::muted(ui, &format!("default {default}"), 11.5);
        if kit::link(ui, "Reset", 11.5).clicked() {
            *reset = Some(Some(vec![key.to_owned()]));
        }
    });
}

/// The detach confirmation; Confirm queues the mutation, Cancel changes nothing.
pub(crate) fn detach_dialog(app: &mut App, ctx: &Context) {
    let Some(record_id) = app.doc.instances.confirm_detach.clone() else {
        return;
    };
    let busy = app.doc.instances.blocks();
    let out = Dialog::new(DialogIntent::Destroy, "Detach component?")
        .icon("component")
        .body(DETACH_WARNING)
        .primary("Confirm", !busy)
        .secondary("Cancel", true)
        .focus_secondary(true)
        .show(ctx, egui::Id::unique("component-detach-confirm"), |_| {});
    if out.secondary || out.dismissed {
        app.doc.instances.confirm_detach = None;
    } else if out.primary {
        app.doc.instances.confirm_detach = None;
        if let Some(scope) = super::scope(app) {
            app.doc
                .instances
                .mutate(&scope, "Detach component", Op::Detach { record_id });
        }
    }
}
