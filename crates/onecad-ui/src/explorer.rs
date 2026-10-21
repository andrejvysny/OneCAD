//! Explorer: panel toggle and the Bodies / Sketches / Datums tree.
use crate::{app::App, layout::Chrome};
use egui::{Context, Margin, Order, Ui};
use onecad_widgets::{
    components::{
        inputs::SearchField,
        lists::{RowState, SectionLabel, TreeRow},
        shell::{FloatingPanel, PanelToggle, Side},
        tooltip::Tip,
    },
    frame,
};

fn matches(filter: &str, name: &str) -> bool {
    filter.is_empty() || name.to_lowercase().contains(&filter.to_lowercase())
}

pub fn show(app: &mut App, ctx: &Context, chrome: &Chrome) {
    if !app.ui.explorer_open {
        toggle(app, ctx, chrome);
    }
    let rect = chrome.explorer;
    let open = app.ui.explorer_open;
    FloatingPanel::new(egui::Id::unique("explorer"), Side::Left)
        .open(open)
        .rect(rect)
        .show(ctx, |ui| {
            egui::Frame::new()
                .inner_margin(Margin::same(12))
                .show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    header(app, ui);
                    ui.add_space(10.0);
                    crate::editor_facade::mode_switch(&mut app.ui.editor_facade, ui);
                    if app.ui.editor_facade.library_mode {
                        crate::editor_facade::library_list(app, ui);
                    } else {
                        tree(app, ui);
                    }
                    crate::editor_facade::footer(app, ui, rect);
                });
        });
}

fn toggle(app: &mut App, ctx: &Context, chrome: &Chrome) {
    let tip = Tip::new("Explorer").shortcut(Some("⌘1"));
    frame::floating(
        egui::Id::unique("explorer-toggle"),
        chrome.explorer_toggle.min,
        Order::Middle,
    )
    .show(ctx, |ui| {
        let button = PanelToggle::new(Side::Left).compact(true).tip(tip);
        if ui.add(button).clicked() {
            app.ui.explorer_open = !app.ui.explorer_open;
        }
    });
}

fn header(app: &mut App, ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.strong("Explorer");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let toggle = PanelToggle::new(Side::Left)
                .open(true)
                .compact(true)
                .tip(Tip::new("Hide explorer").shortcut(Some("⌘1")));
            if ui.add(toggle).clicked() {
                app.ui.explorer_open = false;
            }
        });
    });
}

fn tree(app: &mut App, ui: &mut Ui) {
    ui.spacing_mut().item_spacing.y = 0.0;
    let width = ui.available_width() - 4.0;
    ui.add(
        SearchField::new("explorer-filter", &mut app.ui.explorer_filter)
            .placeholder("Filter")
            .width(width),
    );
    let Some(display) = app.display.clone() else {
        ui.label("Opening document…");
        return;
    };
    let filter = app.ui.explorer_filter.clone();
    let scroll_h = (ui.available_height() - 36.0).max(0.0);
    egui::ScrollArea::vertical()
        .max_height(scroll_h)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add(SectionLabel::new("BODIES"));
            for body in display.projection.bodies.values() {
                if matches(&filter, &body.name) {
                    body_row(app, ui, &body.id, &body.name);
                }
            }
            ui.add(SectionLabel::new("SKETCHES"));
            for sketch in display.projection.sketches.values() {
                if matches(&filter, &sketch.name) {
                    sketch_row(app, ui, sketch);
                }
            }
            ui.add(SectionLabel::new("DATUMS"));
            for datum in display.projection.datums.values() {
                if matches(&filter, &datum.name) {
                    ui.add(TreeRow::new("datum", &datum.name));
                }
            }
        });
}

fn body_row(app: &mut App, ui: &mut Ui, id_text: &str, name: &str) {
    let id = id_text.parse().ok();
    let state = if app.sketch_active() {
        RowState::Reference
    } else if id.is_some() && id == app.selected_body {
        RowState::Selected
    } else {
        RowState::Normal
    };
    let enabled = app.viewport_ready() && app.bridge.valid() && !app.busy();
    let row = TreeRow::new("body", name).state(state).enabled(enabled);
    let response = ui.add(row);
    if response.clicked() || response.secondary_clicked() {
        if let Some(id) = id
            && crate::extrude_ui::select_target(app, id)
        {
            return;
        }
        if let Some(id) = id {
            crate::item_selection::body(app, id);
        }
    }
    if let Some(id) = id {
        response.context_menu(|ui| delete_menu(app, ui, onecad_app::api::DeleteTarget::Body(id)));
    }
}

fn sketch_row(app: &mut App, ui: &mut Ui, sketch: &onecad_app::dto::SketchDto) {
    let editing = app
        .sketch
        .session
        .as_ref()
        .is_some_and(|s| s.authoritative.id.to_string() == sketch.id)
        && app.sketch_active();
    let can_edit =
        app.viewport_ready() && app.bridge.valid() && !app.busy() && !app.sketch_active();
    let meta = sketch.dof.map(|dof| format!("{dof} DOF"));
    let row = TreeRow::new("sketch", &sketch.name)
        .meta(meta.as_deref())
        .state(if editing {
            RowState::Editing
        } else if app
            .ui
            .selected_sketch
            .is_some_and(|id| id.to_string() == sketch.id)
        {
            RowState::Selected
        } else {
            RowState::Normal
        })
        .enabled(can_edit || editing);
    let response = ui.add(row);
    if (response.clicked() || response.double_clicked() || response.secondary_clicked())
        && can_edit
        && let Ok(id) = sketch.id.parse()
    {
        crate::item_selection::sketch(app, id);
        if response.double_clicked() {
            app.edit_sketch(id);
        }
    }
    if let Ok(id) = sketch.id.parse() {
        response.context_menu(|ui| {
            if ui
                .add_enabled(can_edit, egui::Button::new("Edit sketch"))
                .clicked()
            {
                app.edit_sketch(id);
                ui.close();
            }
            delete_menu(app, ui, onecad_app::api::DeleteTarget::Sketch(id));
        });
    }
    if editing {
        sketch_children(app, ui);
    }
}

fn delete_menu(app: &mut App, ui: &mut Ui, target: onecad_app::api::DeleteTarget) {
    let reason = crate::document_items::unavailable(app);
    let response = ui.add_enabled(
        reason.is_none(),
        egui::Button::new("Delete").shortcut_text("Delete / Backspace"),
    );
    if response.clicked() {
        crate::document_items::request(app, target);
        ui.close();
    }
    if let Some(reason) = reason {
        response.on_disabled_hover_text(reason);
    }
}

/// Counts from the live sketch session; no extra queries.
fn sketch_children(app: &App, ui: &mut Ui) {
    use onecad_core::sketch::SketchEntity;
    let Some(session) = app.sketch.session.as_ref() else {
        return;
    };
    let (mut lines, mut circles, mut arcs) = (0usize, 0usize, 0usize);
    for entity in session.authoritative.entities() {
        match entity {
            SketchEntity::Line { .. } => lines += 1,
            SketchEntity::Circle { .. } => circles += 1,
            SketchEntity::Arc { .. } => arcs += 1,
            _ => {}
        }
    }
    let constraints = session.authoritative.constraints().len();
    let plural = |n: usize, one: &str| format!("{n} {one}{}", if n == 1 { "" } else { "s" });
    let rows = [
        ("line", plural(lines, "line"), lines),
        ("circle", plural(circles, "circle"), circles),
        ("arc", plural(arcs, "arc"), arcs),
        ("coincident", plural(constraints, "constraint"), constraints),
    ];
    for (icon, label, count) in rows {
        if count > 0 {
            ui.add(TreeRow::new(icon, &label).child(true));
        }
    }
}

pub(crate) fn selected_sketch_edit_button(app: &mut App, ui: &mut Ui) {
    if app.sketch_active() || app.interaction.tool.is_some() {
        return;
    }
    let Some(sketch) = app.ui.selected_sketch.and_then(|id| {
        app.display
            .as_ref()?
            .projection
            .sketches
            .values()
            .find(|sketch| sketch.id == id.to_string())
            .cloned()
    }) else {
        return;
    };
    let can_edit = app.viewport_ready() && app.bridge.valid() && !app.busy();
    if ui
        .add_enabled(can_edit, egui::Button::new("Edit sketch"))
        .clicked()
        && let Ok(id) = sketch.id.parse()
    {
        app.edit_sketch(id);
    }
}
