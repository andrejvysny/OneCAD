//! 08a library browser: header, source/type filters, recent cards, category tiles, grids.
use super::{
    LibraryUi,
    data::{CATEGORIES, COMPONENTS, Source, columns, sections},
    kit,
};
use crate::ui_state::{StartPage, UiState};
use egui::{Layout, Ui};
use onecad_widgets::components::{
    button::{Button, ButtonKind},
    inputs::{SearchField, Select},
    lists::{Segment, SegmentedControl},
};

const GAP: f32 = 16.0;
const SORTS: [&str; 3] = ["Recently used", "Name", "Last edited"];

pub fn show(state: &mut UiState, ui: &mut Ui) {
    header(state, ui);
    ui.add_space(22.0);
    filters(&mut state.library, ui);
    ui.add_space(22.0);
    let lib = &state.library;
    let s = sections(lib.source, lib.kind, &lib.search);
    let mut open = None;
    if !s.recent.is_empty() {
        grid(ui, "Recently used", &s.recent, &mut open);
        ui.add_space(26.0);
    }
    if s.tiles {
        tiles(ui, &mut open);
        ui.add_space(26.0);
    }
    if !s.own.is_empty() {
        grid(ui, s.own_title, &s.own, &mut open);
    }
    if s.recent.is_empty() && !s.tiles && s.own.is_empty() {
        kit::muted(ui, "No components match these filters.", 13.0);
    }
    if let Some(index) = open {
        open_component(state, index);
    }
}

/// Mine/Project cards open the detail page; standard parts open the spec table.
pub fn open_component(state: &mut crate::ui_state::UiState, index: usize) {
    if COMPONENTS[index].source == Source::Standard {
        state.start_page = StartPage::StandardPart;
    } else {
        state.library.selected = index;
        state.library.detail_tab = 0;
        state.start_page = StartPage::ComponentDetail;
    }
}

fn header(state: &mut UiState, ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 3.0;
            kit::title(ui, "Component library", 20.0);
            kit::muted(
                ui,
                "128 components · 14 yours, 5 in this project, 109 standard parts",
                12.5,
            );
        });
        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
            let new = Button::new("New component")
                .kind(ButtonKind::Primary)
                .icon(Some("plus"));
            if ui.add(new).clicked() {
                crate::component_wizard::open(state);
            }
            let search = SearchField::new("library-search", &mut state.library.search)
                .placeholder("Search names, standards, sizes")
                .shortcut(Some("/"))
                .width(300.0);
            ui.add(search);
        });
    });
}

fn filters(lib: &mut LibraryUi, ui: &mut Ui) {
    ui.horizontal(|ui| {
        let sources = [
            Segment {
                label: "All",
                icon: None,
            },
            Segment {
                label: "My components 14",
                icon: None,
            },
            Segment {
                label: "This project 5",
                icon: None,
            },
            Segment {
                label: "Standard parts 109",
                icon: None,
            },
        ];
        ui.add(SegmentedControl::new(
            "library-source",
            &mut lib.source,
            &sources,
        ));
        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
            let out = Select::new("library-sort", SORTS[lib.sort], &SORTS)
                .width(150.0)
                .show(ui);
            if let Some(i) = out.chosen {
                lib.sort = i;
            }
            let kinds = [
                Segment {
                    label: "All types",
                    icon: None,
                },
                Segment {
                    label: "Parametric",
                    icon: None,
                },
                Segment {
                    label: "Static",
                    icon: None,
                },
            ];
            ui.add(SegmentedControl::new("library-kind", &mut lib.kind, &kinds));
        });
    });
}

fn grid(ui: &mut Ui, heading: &str, items: &[usize], open: &mut Option<usize>) {
    kit::section_header(ui, heading, None);
    ui.add_space(12.0);
    let cols = columns(ui.available_width(), 200.0, GAP);
    let w = (ui.available_width() - GAP * (cols - 1) as f32) / cols as f32;
    for row in items.chunks(cols) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            for &index in row {
                if kit::component_card(ui, w, &COMPONENTS[index]).clicked() {
                    *open = Some(index);
                }
            }
        });
        ui.add_space(GAP);
    }
}

fn tiles(ui: &mut Ui, open: &mut Option<usize>) {
    ui.horizontal(|ui| {
        kit::title(ui, "Standard parts", 14.0);
        kit::muted(ui, "ISO and DIN, metric", 12.0);
        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
            if kit::link(ui, "Browse catalog", 12.5).clicked() {
                *open = COMPONENTS.iter().position(|c| c.source == Source::Standard);
            }
        });
    });
    ui.add_space(12.0);
    let gap = 12.0;
    let cols = columns(ui.available_width(), 170.0, gap).min(CATEGORIES.len());
    let w = (ui.available_width() - gap * (cols - 1) as f32) / cols as f32;
    for row in CATEGORIES.chunks(cols) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for cat in row {
                let count = format!("{} types", cat.types);
                if kit::category_tile(ui, w, cat.name, &count, cat.icon).clicked() {
                    *open = COMPONENTS.iter().position(|c| c.source == Source::Standard);
                }
            }
        });
        ui.add_space(gap);
    }
}
