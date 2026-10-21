//! Hosts the existing Display / Section menu (`viewport_options::contents`) from the chrome.
use crate::app::App;
use egui::{Align, Context, Layout, Rect, Response};
use onecad_widgets::components::lists::menu_frame;

fn layout() -> Layout {
    Layout::top_down_justified(Align::Min)
}

/// Menu toggled by `button`, anchored under it.
pub fn popup_for(app: &mut App, ctx: &Context, button: &Response) {
    egui::Popup::menu(button)
        .width(280.0)
        .frame(menu_frame(ctx))
        .show(|ui| crate::viewport_options::contents(app, ui));
}

/// Menu opened by `app.ui.display_menu` at a recorded anchor rect.
pub fn popup_at(app: &mut App, ctx: &Context) {
    let Some(anchor) = app.ui.display_menu else {
        return;
    };
    let mut open = true;
    egui::Popup::new(
        egui::Id::unique("display-menu"),
        ctx.clone(),
        anchor,
        egui::LayerId::background(),
    )
    .width(280.0)
    .kind(egui::PopupKind::Menu)
    .layout(layout())
    .align(egui::RectAlign::LEFT_START)
    .frame(menu_frame(ctx))
    .open_bool(&mut open)
    .show(|ui| crate::viewport_options::contents(app, ui));
    if !open {
        app.ui.display_menu = None;
    }
}

/// Open the menu at `anchor`, or close it if already open.
pub fn toggle(app: &mut App, anchor: Rect) {
    app.ui.display_menu = if app.ui.display_menu.is_some() {
        None
    } else {
        Some(anchor)
    };
}
