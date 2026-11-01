//! Library browser: search, filters and the grid of catalog entries from the service.
use super::kit::{self, CardView, ChipKind, Thumb};
use crate::{
    library_catalog::{Catalog, ReindexState, Source, Status},
    ui_state::UiState,
};
use egui::{Layout, Ui, vec2};
use onecad_app::dto::{LibraryComponentDto, SearchSortDto};
use onecad_widgets::components::{
    button::{Button, ButtonKind},
    inputs::{SearchField, Select},
    lists::{Segment, SegmentedControl},
};

const GAP: f32 = 16.0;
const SORTS: [(&str, SearchSortDto); 2] =
    [("Name", SearchSortDto::Name), ("ID", SearchSortDto::Id)];
const ANY_CATEGORY: &str = "All categories";

pub fn show(state: &mut UiState, ui: &mut Ui) {
    header(&mut state.library, ui);
    ui.add_space(22.0);
    filters(&mut state.library.catalog, ui);
    reindex_notice(&state.library.catalog, ui);
    ui.add_space(22.0);
    if let Some((id, version)) = body(&mut state.library.catalog, ui) {
        super::open_component(state, &id, &version);
    }
}

fn header(lib: &mut super::LibraryUi, ui: &mut Ui) {
    let make_gate = lib.make_gate.0;
    let catalog = &mut lib.catalog;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 3.0;
            kit::title(ui, "Component library", 20.0);
            kit::muted(ui, &summary(catalog), 12.5);
        });
        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
            let make = Button::new("New component")
                .kind(ButtonKind::Primary)
                .icon(Some("plus"))
                .enabled(make_gate.is_none());
            let made = ui.add(make);
            match make_gate {
                None => {
                    if made
                        .on_hover_text("Make a component from the selected body")
                        .clicked()
                    {
                        lib.make_requested = true;
                    }
                }
                Some(reason) => {
                    made.on_disabled_hover_text(reason);
                }
            }
            let running = matches!(catalog.reindex_state(), ReindexState::Running);
            let reindex = Button::new(if running { "Reindexing…" } else { "Reindex" })
                .kind(ButtonKind::Ghost)
                .enabled(!running);
            let hint = "Rebuild the index from the component packages on disk";
            if ui.add(reindex).on_hover_text(hint).clicked() {
                catalog.reindex();
            }
            let before = catalog.query.text.clone();
            let search = SearchField::new("library-search", &mut catalog.query.text)
                .placeholder("Search names, standards, sizes")
                .shortcut(Some("/"))
                .width(300.0);
            ui.add(search);
            if catalog.query.text != before {
                catalog.text_edited();
            }
        });
    });
}

fn summary(catalog: &Catalog) -> String {
    match catalog.search_status() {
        Status::Failed(_) => "Library unavailable".into(),
        status => {
            let n = catalog.results().len();
            let noun = if n == 1 { "component" } else { "components" };
            let scope = if catalog.has_filters() { " match" } else { "" };
            let busy = if *status == Status::Loading {
                " · updating…"
            } else {
                ""
            };
            format!("{n} {noun}{scope}{busy}")
        }
    }
}

fn filters(catalog: &mut Catalog, ui: &mut Ui) {
    ui.horizontal(|ui| {
        let labels = ["All", "Generated", "Embedded", "Documents", "Profiles"];
        let segments = labels.map(|label| Segment { label, icon: None });
        let mut at = Source::ALL
            .iter()
            .position(|s| *s == catalog.query.source)
            .unwrap_or(0);
        ui.add(SegmentedControl::new("library-source", &mut at, &segments));
        if Source::ALL[at] != catalog.query.source {
            catalog.query.source = Source::ALL[at];
            catalog.requery();
        }
        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
            sort_select(catalog, ui);
            category_select(catalog, ui);
        });
    });
}

fn sort_select(catalog: &mut Catalog, ui: &mut Ui) {
    let labels = SORTS.map(|(label, _)| label);
    let current = SORTS
        .iter()
        .position(|(_, sort)| *sort == catalog.query.sort)
        .unwrap_or(0);
    let out = Select::new("library-sort", labels[current], &labels)
        .width(110.0)
        .show(ui);
    if let Some(i) = out.chosen {
        catalog.query.sort = SORTS[i].1;
        catalog.requery();
    }
}

fn category_select(catalog: &mut Catalog, ui: &mut Ui) {
    let mut options = vec![ANY_CATEGORY];
    options.extend(catalog.categories());
    let current = catalog.query.category.as_deref().unwrap_or(ANY_CATEGORY);
    let out = Select::new("library-category", current, &options)
        .width(170.0)
        .show(ui);
    if let Some(i) = out.chosen {
        catalog.query.category = (i > 0).then(|| options[i].to_owned());
        catalog.requery();
    }
}

fn reindex_notice(catalog: &Catalog, ui: &mut Ui) {
    let text = match catalog.reindex_state() {
        ReindexState::Idle | ReindexState::Running => return,
        ReindexState::Done(report) => {
            let skipped = report.skipped.len();
            let tail = if skipped == 0 {
                String::new()
            } else {
                format!(" · {skipped} skipped: {}", report.skipped.join("; "))
            };
            format!(
                "Reindexed {} of {} packages{tail}",
                report.indexed, report.total
            )
        }
        ReindexState::Failed(error) => format!("Reindex failed: {error}"),
    };
    ui.add_space(8.0);
    kit::muted(ui, &text, 12.0);
}

/// Grid or the honest empty/error state; returns the clicked entry.
fn body(catalog: &mut Catalog, ui: &mut Ui) -> Option<(String, String)> {
    if let Status::Failed(error) = catalog.search_status() {
        problem(ui, "The component library could not be read.", error);
        return None;
    }
    if catalog.results().is_empty() {
        empty(catalog, ui);
        return None;
    }
    grid(catalog, ui)
}

fn problem(ui: &mut Ui, headline: &str, detail: &str) {
    kit::title(ui, headline, 14.0);
    ui.add_space(4.0);
    kit::muted(ui, detail, 12.5);
}

fn empty(catalog: &mut Catalog, ui: &mut Ui) {
    if !catalog.loaded() {
        kit::muted(ui, "Loading components…", 13.0);
    } else if catalog.has_filters() {
        kit::muted(ui, "No components match these filters.", 13.0);
        ui.add_space(6.0);
        if kit::link(ui, "Clear filters", 12.5).clicked() {
            catalog.clear_filters();
        }
    } else {
        problem(
            ui,
            "The component library is empty.",
            "Reindex rebuilds the catalog from the component packages on disk.",
        );
    }
}

fn grid(catalog: &Catalog, ui: &mut Ui) -> Option<(String, String)> {
    let mut open = None;
    let cols = kit::columns(ui.available_width(), 200.0, GAP);
    let w = (ui.available_width() - GAP * (cols - 1) as f32) / cols as f32;
    for row in catalog.results().chunks(cols) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            for c in row {
                if card(catalog, ui, w, c).clicked() {
                    open = Some((c.id.clone(), c.version.clone()));
                }
            }
        });
        ui.add_space(GAP);
    }
    open
}

fn card(catalog: &Catalog, ui: &mut Ui, w: f32, c: &LibraryComponentDto) -> egui::Response {
    let meta = meta_line(c);
    let chips = chips(c);
    let texture = catalog
        .texture(&c.id, &c.version)
        .map(|(id, [w, h])| Thumb {
            id,
            size: vec2(w as f32, h as f32),
        });
    let view = CardView {
        name: &c.name,
        meta: &meta,
        icon: icon_for(c),
        texture,
        chips: &chips,
    };
    kit::component_card(ui, w, &view)
}

/// Designation (the name a drawing uses) when the package declares one, else the id.
pub(super) fn meta_line(c: &LibraryComponentDto) -> String {
    let lead = match (&c.standard, &c.designation) {
        (Some(standard), _) => standard.clone(),
        (None, Some(designation)) => designation.clone(),
        (None, None) => c.id.clone(),
    };
    format!("{lead} · v{}", c.version)
}

fn chips(c: &LibraryComponentDto) -> Vec<(String, ChipKind)> {
    let free = free_count(c);
    let mut chips = vec![(source_label(&c.source_kind).to_owned(), ChipKind::Plain)];
    if free > 0 {
        chips.insert(0, (format!("fx {free}"), ChipKind::Accent));
    }
    if let Some(category) = c.category.last() {
        chips.push((category.clone(), ChipKind::Plain));
    }
    chips
}

pub(super) fn free_count(c: &LibraryComponentDto) -> usize {
    c.parameters
        .values()
        .filter(|spec| spec.role == onecad_library::package::ParameterRole::Free)
        .count()
}

pub(super) fn source_label(kind: &str) -> &str {
    Source::ALL
        .iter()
        .find(|s| s.kind() == Some(kind))
        .map_or(kind, |s| s.label())
}

pub(super) fn icon_for(c: &LibraryComponentDto) -> &'static str {
    match c.category.first().map(String::as_str) {
        Some("fasteners") => "thread",
        Some("bearings") => "revolve",
        _ => "body",
    }
}
