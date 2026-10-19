//! 09a component detail: preview with presets on the left, parameter tabs on the right.
use super::{
    data::{COMPONENTS, PARAMS, PRESETS, USED_IN, VERSIONS, dims_label},
    kit::{self, ChipKind},
    sheet,
};
use crate::ui_state::UiState;
use egui::{
    Align, CornerRadius, FontFamily, FontId, Layout, Rect, Sense, Stroke, StrokeKind, Ui, pos2,
    vec2,
};
use onecad_widgets::{
    components::{
        button::{Button, ButtonKind},
        lists::{MenuRow, Tab, TabBar, menu_frame},
    },
    fonts,
    theme::look,
};

const TABS: [Tab<'static>; 4] = [
    Tab {
        label: "Parameters",
        count: Some(7),
    },
    Tab {
        label: "Configurations",
        count: Some(3),
    },
    Tab {
        label: "Versions",
        count: Some(3),
    },
    Tab {
        label: "Used in",
        count: Some(3),
    },
];

pub fn show(state: &mut UiState, ui: &mut Ui) {
    let name = COMPONENTS[state.library.selected].name;
    let crumbs = ["Component library", "My components", name];
    match kit::breadcrumb(ui, &crumbs) {
        Some(0) => crate::library_pages::go_library(state, 0),
        Some(1) => crate::library_pages::go_library(state, 1),
        _ => {}
    }
    ui.add_space(18.0);
    let left_w = (ui.available_width() * 0.46).clamp(300.0, 440.0);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 28.0;
        ui.allocate_ui_with_layout(vec2(left_w, 400.0), Layout::top_down(Align::Min), |ui| {
            preview(state, ui);
        });
        let w = ui.available_width();
        ui.allocate_ui_with_layout(vec2(w, 400.0), Layout::top_down(Align::Min), |ui| {
            summary(state, ui, name);
            ui.add_space(22.0);
            tabs(state, ui);
        });
    });
}

fn preview(state: &mut UiState, ui: &mut Ui) {
    let l = look(ui.ctx());
    let t = &l.theme;
    let lib = &mut state.library;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 360.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(12), t.well);
    p.rect_stroke(
        rect,
        CornerRadius::same(12),
        Stroke::new(1.0, t.border),
        StrokeKind::Inside,
    );
    let tag = format!("{} preset", PRESETS[lib.preset].0);
    let g = p.layout_no_wrap(tag, FontId::new(11.0, fonts::medium()), t.ink3);
    let chip = Rect::from_min_size(rect.min + vec2(10.0, 10.0), vec2(g.size().x + 14.0, 20.0));
    p.rect_filled(chip, CornerRadius::same(5), t.chip);
    p.galley(
        pos2(chip.left() + 7.0, chip.center().y - g.size().y / 2.0),
        g,
        t.ink3,
    );
    kit::view_strip(ui, rect, &mut lib.view);
    ui.add_space(12.0);
    let gap = 10.0;
    let w = (ui.available_width() - 2.0 * gap) / 3.0;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        for i in 0..PRESETS.len() {
            if preset_card(ui, w, i, lib.preset == i).clicked() {
                lib.preset = i;
            }
        }
    });
}

fn preset_card(ui: &mut Ui, w: f32, i: usize, on: bool) -> egui::Response {
    let t = look(ui.ctx()).theme;
    let (rect, r) = ui.allocate_exact_size(vec2(w, 54.0), Sense::click());
    let (fill, border) = match (on, r.hovered()) {
        (true, _) => (t.sel_soft, t.accent),
        (false, true) => (t.hover, t.border_strong),
        (false, false) => (t.surface, t.border),
    };
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(9), fill);
    p.rect_stroke(
        rect,
        CornerRadius::same(9),
        Stroke::new(1.0, border),
        StrokeKind::Inside,
    );
    let name = FontId::new(12.5, fonts::semibold());
    p.text(
        rect.min + vec2(11.0, 10.0),
        egui::Align2::LEFT_TOP,
        PRESETS[i].0,
        name,
        t.ink,
    );
    let dims = FontId::new(11.5, fonts::mono());
    p.text(
        rect.min + vec2(11.0, 30.0),
        egui::Align2::LEFT_TOP,
        dims_label(i),
        dims,
        t.ink4,
    );
    r.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn summary(state: &mut UiState, ui: &mut Ui, name: &str) {
    let t = look(ui.ctx()).theme;
    ui.horizontal(|ui| {
        kit::title(ui, name, 22.0);
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            let more = kit::icon_button(ui, "overflow", 30.0, true);
            egui::Popup::menu(&more)
                .frame(menu_frame(ui.ctx()))
                .gap(4.0)
                .show(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for (icon, label) in [
                        ("edit", "Rename"),
                        ("export", "Export"),
                        ("delete", "Delete"),
                    ] {
                        ui.add(MenuRow::new(Some(icon), label));
                    }
                });
        });
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        kit::chip(ui, "fx 7", ChipKind::Accent);
        kit::chip(ui, "v3", ChipKind::Plain);
        kit::chip(ui, "Brackets", ChipKind::Plain);
        kit::label(
            ui,
            "Edited Oct 17 · from",
            12.0,
            FontFamily::Proportional,
            t.ink5,
        );
        kit::link(ui, "Mounting bracket.onecad", 12.0);
    });
    ui.add_space(14.0);
    let text = "L-bracket with two slotted mounting holes per leg. Hole size follows the bolt standard; fillet radius follows plate thickness.";
    kit::label(ui, text, 13.0, FontFamily::Proportional, t.ink2);
    ui.add_space(14.0);
    actions(state, ui);
}

fn actions(state: &mut UiState, ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let insert = Button::new("Insert into open model")
            .kind(ButtonKind::Primary)
            .icon(Some("plus"));
        if ui.add(insert).clicked() {
            sheet::open(&mut state.library);
        }
        let edit = Button::new("Edit source").icon(Some("edit"));
        if ui.add(edit).clicked() {
            state.library.note = Some("Source editing is not available yet.");
        }
        let dup = Button::new("Duplicate")
            .kind(ButtonKind::Ghost)
            .icon(Some("duplicate"));
        if ui.add(dup).clicked() {
            state.library.note = Some("Duplicated to My components (preview only).");
        }
    });
    if let Some(note) = state.library.note {
        ui.add_space(6.0);
        kit::muted(ui, note, 12.0);
    }
}

fn tabs(state: &mut UiState, ui: &mut Ui) {
    let lib = &mut state.library;
    ui.add(TabBar::new("detail-tabs", &mut lib.detail_tab, &TABS));
    ui.add_space(4.0);
    match lib.detail_tab {
        0 => params(ui),
        1 => config_rows(lib, ui),
        2 => rows(ui, &VERSIONS.map(|(a, b)| (a, b, ""))),
        _ => rows(ui, &USED_IN.map(|(a, b)| (a, b, ""))),
    }
}

fn columns(width: f32) -> [f32; 4] {
    let first = (width * 0.24).clamp(90.0, 120.0);
    let second = (width * 0.2).clamp(80.0, 100.0);
    [0.0, first, first + second, first + second + 110.0]
}

fn params(ui: &mut Ui) {
    let t = look(ui.ctx()).theme;
    let w = ui.available_width();
    let xs = columns(w);
    let (head, _) = ui.allocate_exact_size(vec2(w, 36.0), Sense::hover());
    let font = FontId::new(11.0, fonts::medium());
    for (x, text) in xs
        .iter()
        .zip(["NAME", "TYPE", "DEFAULT", "RANGE / OPTIONS"])
    {
        let pos = pos2(head.left() + 4.0 + x, head.center().y);
        ui.painter()
            .text(pos, egui::Align2::LEFT_CENTER, text, font.clone(), t.ink5);
    }
    for param in PARAMS {
        let (row, _) = ui.allocate_exact_size(vec2(w, 39.0), Sense::hover());
        let p = ui.painter();
        p.hline(
            row.x_range(),
            row.top() + 0.5,
            Stroke::new(1.0, t.border_subtle),
        );
        let cells = [
            (param.name, FontId::new(12.5, fonts::mono()), t.ink),
            (param.kind, FontId::proportional(12.5), t.ink4),
            (param.default, FontId::new(12.5, fonts::mono()), t.ink),
            (param.range, FontId::proportional(12.0), t.ink4),
        ];
        for (x, (text, font, colour)) in xs.iter().zip(cells) {
            let pos = pos2(row.left() + 4.0 + x, row.center().y);
            let max = row.right() - pos.x;
            let g = kit::fit(p, text, font, colour, max);
            p.galley(pos2(pos.x, pos.y - g.size().y / 2.0), g, colour);
        }
    }
}

fn config_rows(lib: &mut super::LibraryUi, ui: &mut Ui) {
    let t = look(ui.ctx()).theme;
    for (i, (name, ..)) in PRESETS.iter().enumerate() {
        let dims = dims_label(i);
        let tag = if lib.preset == i { "Default" } else { "" };
        let r = row(ui, name, &dims, tag, &t);
        if r.clicked() {
            lib.preset = i;
        }
    }
}

fn rows(ui: &mut Ui, items: &[(&str, &str, &str)]) {
    let t = look(ui.ctx()).theme;
    for (a, b, c) in items {
        row(ui, a, b, c, &t);
    }
}

fn row(ui: &mut Ui, a: &str, b: &str, tag: &str, t: &onecad_widgets::Theme) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 42.0), Sense::click());
    let p = ui.painter();
    if r.hovered() {
        p.rect_filled(rect, CornerRadius::same(6), t.hover);
    }
    p.hline(
        rect.x_range(),
        rect.top() + 0.5,
        Stroke::new(1.0, t.border_subtle),
    );
    let name = FontId::new(13.0, fonts::semibold());
    p.text(
        pos2(rect.left() + 4.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        a,
        name,
        t.ink,
    );
    let font = FontId::new(12.0, fonts::mono());
    p.text(
        pos2(rect.left() + 130.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        b,
        font,
        t.ink4,
    );
    if !tag.is_empty() {
        kit::paint_chip(
            p,
            t,
            pos2(rect.right() - 60.0, rect.center().y - 10.0),
            tag,
            ChipKind::Accent,
        );
    }
    r
}
