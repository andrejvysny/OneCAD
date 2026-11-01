//! Component library pages: browser, component detail and the ISO 4762 table, all
//! rendered from the application's library catalog (`library_catalog`).
use crate::{
    app::App,
    library_catalog::Catalog,
    ui_state::{StartPage, UiState},
};
use egui::{Rect, Ui, UiBuilder, pos2};

mod browser;
mod detail;
pub(crate) mod kit;
pub(crate) mod params;
mod standard;
#[cfg(test)]
mod tests;

const PAD_X: f32 = 36.0;

/// Why Insert is off, refreshed from the editor each frame (`None`: a placement can start).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InsertGate(pub Option<&'static str>);

impl Default for InsertGate {
    fn default() -> Self {
        Self(Some("Open a model first"))
    }
}

/// Why New component is off (`None`: the wizard can open on the selected body).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MakeGate(pub Option<&'static str>);

impl Default for MakeGate {
    fn default() -> Self {
        Self(Some("Open a model first"))
    }
}

/// Page-local state for the library pages.
#[derive(Clone, Debug, Default)]
pub struct LibraryUi {
    pub(crate) catalog: Catalog,
    pub detail_tab: usize,
    /// Why New component is off, refreshed from the editor each frame.
    pub(crate) make_gate: MakeGate,
    /// New component was pressed; the page owner (which has the editor) opens the wizard.
    pub(crate) make_requested: bool,
    pub(crate) insert_gate: InsertGate,
    /// Insert was pressed; the page owner (which has the editor) starts the placement.
    pub(crate) insert_requested: bool,
}

/// Back to the browser.
pub fn go_library(state: &mut UiState) {
    state.start_page = StartPage::Library;
}

/// Opens the browser from outside the library: every source, as the sidebar entry does.
pub fn open_library(state: &mut UiState) {
    go_library(state);
    let catalog = &mut state.library.catalog;
    if catalog.query.source != crate::library_catalog::Source::All {
        catalog.query.source = crate::library_catalog::Source::All;
        catalog.requery();
    }
}

/// Opens a catalog entry on its detail page.
pub(crate) fn open_component(state: &mut UiState, id: &str, version: &str) {
    state.library.catalog.open_detail(id, version);
    state.library.detail_tab = 0;
    state.start_page = StartPage::ComponentDetail;
}

pub fn show(app: &mut App, ui: &mut Ui) {
    crate::library_catalog::pump(app, ui.ctx());
    app.ui.library.insert_gate = InsertGate(crate::placement_gesture::gate(app));
    app.ui.library.make_gate = MakeGate(crate::component_wizard::availability(app));
    show_state(&mut app.ui, ui);
    if std::mem::take(&mut app.ui.library.make_requested) {
        crate::component_wizard::open(app);
    }
    if std::mem::take(&mut app.ui.library.insert_requested) {
        crate::placement_gesture::arm_from_detail(app);
    }
}

/// Why Insert is unavailable right now: the editor's reason, else an invalid draft.
pub(crate) fn insert_blocker(lib: &LibraryUi) -> Option<&'static str> {
    if let Some(reason) = lib.insert_gate.0 {
        return Some(reason);
    }
    let draft = lib.catalog.detail()?.draft.as_ref();
    match draft {
        Some(draft) if draft.resolve().is_ok() => None,
        Some(_) => Some("Fix the invalid parameters first"),
        None => Some("Loading parameters…"),
    }
}

fn show_state(state: &mut UiState, ui: &mut Ui) {
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
                StartPage::StandardPart => standard::show(state, ui),
                _ => browser::show(state, ui),
            });
    });
}
