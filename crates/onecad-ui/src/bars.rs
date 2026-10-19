//! Title bar (document name, history, File menu) and status bar.
use crate::{app::App, bridge::Command};
use egui::{Context, Ui};
use onecad_widgets::components::{
    lists::{MenuRow, menu_divider, menu_frame},
    shell::{StatusBar, TitleBar},
};

/// Model history is blocked while a sketch is active, as in the input router.
fn ready(app: &App) -> bool {
    !app.sketch_active()
        && app.viewport_ready()
        && app.bridge.valid()
        && !app.busy()
        && !crate::viewport_selection::sectioned(app)
}

fn depth(app: &App, f: impl Fn(&onecad_app::dto::DocumentProjection) -> usize) -> bool {
    app.display.as_ref().is_some_and(|d| f(&d.projection) > 0)
}

pub fn title(app: &mut App, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let (can_undo, can_redo) = (
        ready(app) && depth(app, |p| p.undo_depth),
        ready(app) && depth(app, |p| p.redo_depth),
    );
    let (name, dirty) = app
        .display
        .as_ref()
        .map_or(("OneCAD".to_owned(), false), |d| {
            (d.projection.title.clone(), d.projection.dirty)
        });
    egui::Panel::top("title")
        .exact_size(44.0)
        .frame(egui::Frame::NONE)
        .resizable(false)
        .show(ui, |ui| {
            let out = TitleBar::new(&name)
                .dirty(dirty)
                .can_undo(can_undo)
                .can_redo(can_redo)
                .home_blocked((!home_allowed(app)).then_some("Finish the current command first"))
                .show(ui);
            if out.home && home_allowed(app) {
                app.ui.start_open = true;
            }
            if out.undo {
                app.submit(Command::Undo);
            }
            if out.redo {
                app.submit(Command::Redo);
            }
            file_menu(app, &ctx, &out.file, (can_undo, can_redo));
        });
}

fn home_allowed(app: &App) -> bool {
    !(app.sketch_active()
        || app.interaction.tool.is_some()
        || app.busy()
        || app.transform_gizmo.gizmo.active())
}

fn file_menu(app: &mut App, ctx: &Context, file: &egui::Response, (undo, redo): (bool, bool)) {
    egui::Popup::menu(file).frame(menu_frame(ctx)).show(|ui| {
        ui.set_min_width(180.0);
        let save = MenuRow::new(Some("save"), "Save").shortcut(Some("⌘S"));
        if ui.add(save).clicked() {
            app.submit(Command::Save);
        }
        menu_divider(ui);
        let row = |icon, label, key, enabled| {
            MenuRow::new(Some(icon), label)
                .shortcut(Some(key))
                .enabled(enabled)
        };
        if ui.add(row("undo", "Undo", "⌘Z", undo)).clicked() {
            app.submit(Command::Undo);
        }
        if ui.add(row("redo", "Redo", "⇧⌘Z", redo)).clicked() {
            app.submit(Command::Redo);
        }
        menu_divider(ui);
        ui.add(
            MenuRow::new(Some("search"), "Search commands")
                .shortcut(Some("⌘K"))
                .enabled(false),
        );
    });
}

pub fn status(app: &App, ui: &mut Ui) {
    let (text, dot) = match crate::feature_card::status_line(app) {
        Some((text, ready)) => (
            text,
            Some(if ready {
                app.theme.dof_ok
            } else {
                app.theme.ink5
            }),
        ),
        None => (app.status.as_str(), None),
    };
    let persp = app.camera.projection != onecad_scene::Projection::Orthographic;
    egui::Panel::bottom("status")
        .exact_size(28.0)
        .frame(egui::Frame::NONE)
        .resizable(false)
        .show(ui, |ui| {
            StatusBar::new()
                .sketch(app.sketch_active())
                .dot(dot)
                .text(text)
                .projection(Some(persp))
                .unit(Some("mm"))
                .show(ui);
        });
}
