//! Explorer: panel toggle, workspace switcher and the Bodies / Sketches / Datums tree.
use crate::{app::App, layout::Chrome};
use egui::{Context, Margin, Order, Ui};
use onecad_widgets::{
    components::{
        inputs::SearchField,
        lists::{RowState, SectionLabel, TreeRow, menu_frame},
        shell::{FloatingPanel, PanelToggle, Side, WorkspaceButton, WorkspaceItem, workspace_menu},
        tooltip::Tip,
    },
    frame,
};

fn matches(filter: &str, name: &str) -> bool {
    filter.is_empty() || name.to_lowercase().contains(&filter.to_lowercase())
}

pub fn show(app: &mut App, ctx: &Context, chrome: &Chrome) {
    toggle(app, ctx, chrome);
    workspace(app, ctx, chrome);
    let rect = chrome.explorer;
    let open = app.ui.explorer_open;
    FloatingPanel::new(egui::Id::unique("explorer"), Side::Left)
        .open(open)
        .rect(rect)
        .show(ctx, |ui| {
            egui::Frame::new()
                .inner_margin(Margin::symmetric(6, 8))
                .show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    tree(app, ui);
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
        let button = PanelToggle::new(Side::Left)
            .open(app.ui.explorer_open)
            .tip(tip);
        if ui.add(button).clicked() {
            app.ui.explorer_open = !app.ui.explorer_open;
        }
    });
}

fn workspace(app: &mut App, ctx: &Context, chrome: &Chrome) {
    let mut response = None;
    frame::floating(
        egui::Id::unique("workspace-button"),
        chrome.workspace_button.min,
        Order::Middle,
    )
    .show(ctx, |ui| {
        let button =
            WorkspaceButton::new("Design", "Workspace", "body").open(app.ui.workspace_menu_open);
        response = Some(ui.add(button));
    });
    let Some(response) = response else {
        return;
    };
    let item = |label, icon, enabled, active| WorkspaceItem {
        label,
        icon,
        shortcut: None,
        enabled,
        active,
    };
    let items = [
        item("Design", "body", true, true),
        item("Assembly", "component", false, false),
        item("Simulation", "study", false, false),
        item("Drawing", "viewFront", false, false),
    ];
    egui::Popup::menu(&response)
        .frame(menu_frame(ctx))
        .show(|ui| {
            workspace_menu(ui, &items, &[]);
        });
    let id = egui::Popup::default_response_id(&response);
    app.ui.workspace_menu_open = egui::Popup::is_id_open(ctx, id);
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
    let muted = app.theme.ink5;
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        ui.label(
            egui::RichText::new("No extensions active")
                .size(12.0)
                .color(muted),
        );
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
    if ui.add(row).clicked() {
        crate::pill::cancel_tool(app);
        app.dispatch(onecad_interaction::Action::ClearSelection);
        app.selected_body = id;
        app.topology.select(None);
        app.draw_dirty = true;
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
        } else {
            RowState::Normal
        })
        .enabled(can_edit || editing);
    if ui.add(row).clicked()
        && can_edit
        && let Ok(id) = sketch.id.parse()
    {
        app.edit_sketch(id);
    }
    if editing {
        sketch_children(app, ui);
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
