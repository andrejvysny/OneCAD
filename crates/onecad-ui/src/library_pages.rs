//! Component library pages (Final 08a–10a): UI facade over mock data; no library services yet.
use crate::{
    app::App,
    ui_state::{StartPage, UiState},
};
use egui::{Rect, Ui, UiBuilder, pos2};

mod browser;
mod data;
mod detail;
pub(crate) mod kit;
mod sheet;
mod standard;
mod standard_data;
#[cfg(test)]
mod tests;

pub use sheet::SheetState;
pub use standard_data::StdState;

const PAD_X: f32 = 36.0;

/// Page-local state for the library facade.
#[derive(Clone, Debug)]
pub struct LibraryUi {
    /// 0 All, 1 My components, 2 This project, 3 Standard parts.
    pub source: usize,
    /// 0 All types, 1 Parametric, 2 Static.
    pub kind: usize,
    pub sort: usize,
    pub search: String,
    /// Index into the mock component list shown on the detail page.
    pub selected: usize,
    pub detail_tab: usize,
    /// Selected preset card on the detail page.
    pub preset: usize,
    /// Selected preview view button.
    pub view: usize,
    pub note: Option<&'static str>,
    pub sheet: Option<SheetState>,
    pub std: StdState,
    pub wizard: Option<crate::component_wizard::WizardState>,
}

impl Default for LibraryUi {
    fn default() -> Self {
        Self {
            source: 0,
            kind: 0,
            sort: 0,
            search: String::new(),
            selected: 0,
            detail_tab: 0,
            preset: 1,
            view: 1,
            note: None,
            sheet: None,
            std: StdState::default(),
            wizard: None,
        }
    }
}

/// Back to the browser with a source tab selected.
pub fn go_library(state: &mut UiState, source: usize) {
    state.start_page = StartPage::Library;
    state.library.source = source;
}

pub fn show(app: &mut App, ui: &mut Ui) {
    show_state(&mut app.ui, ui);
}

fn show_state(state: &mut UiState, ui: &mut Ui) {
    if state.start_page == StartPage::StandardPart {
        standard::show(state, ui);
    } else {
        let area = ui.max_rect();
        let main = Rect::from_min_max(
            pos2(area.left() + PAD_X, area.top() + 28.0),
            pos2(area.right() - PAD_X, area.bottom()),
        );
        ui.scope_builder(UiBuilder::new().max_rect(main), |ui| {
            egui::ScrollArea::vertical()
                .id_salt("library-content")
                .show(ui, |ui| match state.start_page {
                    StartPage::ComponentDetail => detail::show(state, ui),
                    _ => browser::show(state, ui),
                });
        });
    }
    sheet::show(state, ui.ctx());
}
