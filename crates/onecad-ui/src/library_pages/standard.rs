//! ISO 4762 / DIN 912 size table, read from the library's own dimension table.
//!
//! Reachable only from a component whose generator is `iso4762`; picking a row sets that
//! component's `thread` draft field, so the table, the Parameters tab and the designation
//! always describe the same size.
use super::kit;
use crate::{
    library_catalog::{Catalog, draft::format_dimension},
    ui_state::{StartPage, UiState},
};
use egui::{Align2, CornerRadius, CursorIcon, FontId, Sense, Stroke, Ui, pos2, vec2};
use onecad_library::tables::{Iso4762Row, Iso4762Table};
use onecad_widgets::{
    components::button::{Button, ButtonKind},
    fonts,
    theme::look,
};

const ROW_H: f32 = 34.0;
const GENERATOR: &str = "iso4762";
const COLUMNS: [&str; 8] = [
    "THREAD",
    "d1 SHAFT",
    "d2 HEAD Ø",
    "k HEAD H",
    "s SOCKET",
    "t MIN",
    "b THREAD",
    "PITCH",
];

fn values(row: &Iso4762Row) -> [f64; 7] {
    [row.d1, row.d2, row.k, row.s, row.t_min, row.b, row.pitch_mm]
}

enum Go {
    Library,
    Detail,
    Insert,
    Pick(&'static str),
}

pub fn show(state: &mut UiState, ui: &mut Ui) {
    let lib = &mut state.library;
    let blocker = super::insert_blocker(lib);
    let name = match lib.catalog.detail().and_then(|d| d.component.as_ref()) {
        Some(c) if c.generator_id.as_deref() == Some(GENERATOR) => c.name.clone(),
        // Another component (or none) has no ISO table: return to where it can be chosen.
        Some(_) => {
            state.start_page = StartPage::ComponentDetail;
            return;
        }
        None => {
            state.start_page = StartPage::Library;
            return;
        }
    };
    let go = match kit::breadcrumb(ui, &["Component library", &name, "Standard table"]) {
        Some(0) => Some(Go::Library),
        Some(1) => Some(Go::Detail),
        _ => None,
    };
    ui.add_space(14.0);
    kit::title(ui, "ISO 4762 · socket head cap screws", 20.0);
    ui.add_space(3.0);
    kit::muted(
        ui,
        "DIN 912 · metric, dimensions in mm, from the library's built-in table",
        12.5,
    );
    ui.add_space(16.0);
    let selected = selected_thread(&lib.catalog);
    let pick = table(ui, selected);
    ui.add_space(18.0);
    let panel = selection_panel(&lib.catalog, selected, blocker, ui);
    let go = go.or(pick.map(Go::Pick)).or(panel);
    match go {
        Some(Go::Library) => {
            lib.catalog.close_detail();
            super::go_library(state);
        }
        Some(Go::Detail) => {
            lib.detail_tab = 0;
            state.start_page = StartPage::ComponentDetail;
        }
        Some(Go::Insert) => lib.insert_requested = true,
        Some(Go::Pick(thread)) => set_thread(&mut lib.catalog, thread),
        None => {}
    }
}

fn selected_thread(catalog: &Catalog) -> Option<&'static str> {
    let draft = catalog.detail()?.draft.as_ref()?;
    Iso4762Table::threads()
        .into_iter()
        .find(|t| *t == draft.text("thread"))
}

pub(super) fn set_thread(catalog: &mut Catalog, thread: &str) {
    if let Some(draft) = catalog.detail_mut().and_then(|v| v.draft.as_mut()) {
        draft.set_text("thread", thread.to_owned());
        catalog.draft_edited();
    }
}

/// Header plus one clickable row per size; returns the clicked thread.
fn table(ui: &mut Ui, selected: Option<&str>) -> Option<&'static str> {
    let t = look(ui.ctx()).theme;
    let width = ui.available_width();
    let step = width / COLUMNS.len() as f32;
    let (head, _) = ui.allocate_exact_size(vec2(width, 28.0), Sense::hover());
    for (i, label) in COLUMNS.iter().enumerate() {
        let at = pos2(head.left() + 8.0 + step * i as f32, head.center().y);
        let font = FontId::new(11.0, fonts::medium());
        ui.painter()
            .text(at, Align2::LEFT_CENTER, label, font, t.ink5);
    }
    let mut picked = None;
    for thread in Iso4762Table::threads() {
        let Some(row) = Iso4762Table::row(thread) else {
            continue;
        };
        let (rect, r) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::click());
        let on = selected == Some(thread);
        let p = ui.painter();
        if on {
            p.rect_filled(rect, CornerRadius::same(6), t.sel_bg);
        } else if r.hovered() {
            p.rect_filled(rect, CornerRadius::same(6), t.hover);
        }
        p.hline(
            rect.x_range(),
            rect.top() + 0.5,
            Stroke::new(1.0, t.border_subtle),
        );
        let ink = if on { t.sel_text } else { t.ink };
        let cells = std::iter::once(thread.to_owned()).chain(values(row).map(format_dimension));
        for (i, text) in cells.enumerate() {
            let at = pos2(rect.left() + 8.0 + step * i as f32, rect.center().y);
            let font = FontId::new(12.5, fonts::mono());
            p.text(at, Align2::LEFT_CENTER, text, font, ink);
        }
        if r.on_hover_cursor(CursorIcon::PointingHand).clicked() {
            picked = Some(thread);
        }
    }
    picked
}

/// The selected size's designation and what the library does with it; true when the
/// user asks to continue in Try parameters.
fn selection_panel(
    catalog: &Catalog,
    selected: Option<&str>,
    blocker: Option<&'static str>,
    ui: &mut Ui,
) -> Option<Go> {
    let Some(thread) = selected else {
        kit::muted(ui, "Choose a size to set the component's thread.", 12.5);
        return None;
    };
    let designation = catalog
        .detail()
        .and_then(|d| d.draft.as_ref())
        .and_then(|d| d.designation())
        .unwrap_or_else(|| thread.to_owned());
    kit::title(ui, &designation, 14.0);
    ui.add_space(4.0);
    kit::muted(
        ui,
        "Length and thread detail are set on the Parameters tab.",
        12.0,
    );
    ui.add_space(10.0);
    let mut go = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if ui
            .add(Button::new("Try parameters").kind(ButtonKind::Primary))
            .clicked()
        {
            go = Some(Go::Detail);
        }
        let insert = ui.add(Button::new("Insert into open model").enabled(blocker.is_none()));
        if let Some(reason) = blocker {
            insert.on_disabled_hover_text(reason);
        } else if insert.clicked() {
            go = Some(Go::Insert);
        }
    });
    go
}
