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

const NOT_YET: &str = "Not available yet";
const SQUARE: f32 = 36.0;

fn nav_button<'a>(icon: &'a str, name: &'a str, enabled: bool) -> NavButton<'a> {
    let reason = (!enabled).then_some(NOT_YET);
    NavButton {
        icon,
        enabled,
        active: false,
        tip: Tip::new(name).reason(reason),
    }
}

pub fn nav_pill(app: &mut App, ctx: &Context, chrome: &Chrome) {
    let ready = app.viewport_ready();
    let buttons = [
        nav_button("home", "Home view", ready),
        nav_button("fit", "Fit all", ready),
        nav_button("isolate", "Isolate", false),
        nav_button("section", "Section", false),
        nav_button("layers", "Layers", false),
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
        _ => {}
    }
}

/// The shared layout reserves a separate region for cube controls and parameters.
pub fn view_cluster(app: &mut App, ctx: &Context, chrome: &Chrome) {
    let ready = app.viewport_ready();
    let tip = |name| Tip::new(name);
    let off = |name| Tip::new(name).reason(Some(NOT_YET));
    let buttons = [
        SquareButton {
            icon: "shaded",
            on: app.ui.display_menu.is_some(),
            enabled: ready,
            tip: tip("Display"),
        },
        SquareButton {
            icon: "grid",
            on: true,
            enabled: false,
            tip: off("Grid"),
        },
        SquareButton {
            icon: "snap",
            on: false,
            enabled: false,
            tip: off("Snap"),
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
    display_menu::popup_at(app, ctx);
}
