//! Extensions manager pages (Extensions 16a–22a): UI facade over mock data; no extension host yet.
mod catalog;
mod detail;
mod detail_side;
mod detail_tabs;
mod developer;
mod installed;
mod kit;
mod manager;
mod sheets;
mod state;

use crate::{app::App, ui_state::StartPage};
use egui::Ui;

pub use state::{ExtensionsUi, ManagerTab};

pub fn show(app: &mut App, ui: &mut Ui) {
    let page = app.ui.start_page;
    let nav = render(ui, &mut app.ui.extensions, page);
    if let Some(next) = nav {
        app.ui.start_page = next;
    }
}

/// Draws the page for `page`; returns the page to navigate to, if any.
fn render(ui: &mut Ui, x: &mut ExtensionsUi, page: StartPage) -> Option<StartPage> {
    // The start screen already provides padding and the scroll area.
    let nav = match page {
        StartPage::ExtensionDetail => detail::show(ui, x),
        _ => manager::show(ui, x),
    };
    sheets::show(ui.ctx(), x);
    sheets::toast(ui.ctx(), x);
    nav
}

#[cfg(test)]
mod tests;
