//! Viewport overlays: nav pill (bottom-left) and the view-cluster buttons under the cube.
use crate::{app::App, display_menu, layout::Chrome};
use egui::{Align2, Context, Order, Rect, vec2};
use onecad_widgets::{
    components::{
        overlays::{NavButton, NavPill, SquareButton, ViewCluster},
        tooltip::Tip,
    },
    frame,
};

const SQUARE: f32 = 36.0;

fn nav_button<'a>(icon: &'a str, name: &'a str, enabled: bool, active: bool) -> NavButton<'a> {
    let reason = (!enabled).then_some(match icon {
        "isolate" => "Temporary body isolation is not available yet",
        "layers" => "Layer visibility controls are not available yet",
        _ => "Viewport unavailable",
    });
    NavButton {
        icon,
        enabled,
        active,
        tip: Tip::new(name).reason(reason),
    }
}

pub fn nav_pill(app: &mut App, ctx: &Context, chrome: &Chrome) {
    let ready = app.viewport_ready();
    let layers_open = app.ui.shell_facade.layers_open;
    let buttons = [
        nav_button("home", "Home view", ready, false),
        nav_button("fit", "Fit all", ready, false),
        nav_button("isolate", "Isolate", false, false),
        nav_button("section", "Section / display settings", ready, false),
        nav_button(
            "layers",
            "Layers",
            crate::editor_facade::PREVIEW_FACADES,
            layers_open,
        ),
    ];
    let mut clicked = None;
    frame::floating(egui::Id::unique("nav-pill"), chrome.nav_pill, Order::Middle)
        .pivot(Align2::LEFT_BOTTOM)
        .show(ctx, |ui| clicked = NavPill::new(&buttons).show(ui));
    match clicked {
        Some(0) => {
            app.camera = onecad_scene::Camera::default();
            app.fit_pending = true;
        }
        Some(1) => app.fit_pending = true,
        Some(3) => display_menu::toggle(
            app,
            Rect::from_min_size(chrome.nav_pill, vec2(SQUARE, SQUARE)),
        ),
        Some(4) => app.ui.shell_facade.layers_open = !layers_open,
        _ => {}
    }
    crate::workspace_ui::layers_popover(app, ctx, chrome);
}

/// The shared layout reserves a separate region for cube controls and parameters.
pub fn view_cluster(app: &mut App, ctx: &Context, chrome: &Chrome) {
    let ready = app.viewport_ready();
    let tip = |name| Tip::new(name);
    let buttons = [
        SquareButton {
            icon: "shaded",
            on: app.ui.display_menu.is_some(),
            enabled: ready,
            tip: tip("Display"),
        },
        SquareButton {
            icon: "grid",
            on: crate::support_view::grid_visible(ctx),
            enabled: ready,
            tip: tip("Grid"),
        },
        SquareButton {
            icon: "snap",
            on: app.ui.sketch_snap,
            enabled: crate::sketch_ui::tools_enabled(app),
            tip: tip("Snap"),
        },
    ];
    let pos = chrome.view_controls.min;
    let mut clicked = None;
    frame::floating(egui::Id::unique("view-cluster"), pos, Order::Middle).show(ctx, |ui| {
        clicked = ViewCluster::new(&buttons)
            .no_cube()
            .show(ui, |_, _, _| {})
            .clicked;
    });
    if clicked == Some(0) {
        let anchor = Rect::from_min_size(pos, vec2(SQUARE, SQUARE));
        display_menu::toggle(app, anchor);
    }
    if clicked == Some(1) {
        crate::support_view::set_grid_visible(ctx, !crate::support_view::grid_visible(ctx));
        app.draw_dirty = true;
    }
    if clicked == Some(2) {
        app.ui.sketch_snap = !app.ui.sketch_snap;
    }
    display_menu::popup_at(app, ctx);
}
