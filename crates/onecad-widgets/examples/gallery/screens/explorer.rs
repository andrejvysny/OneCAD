//! Explorer panel content (README 02a / 04a): filter, BODIES / SKETCHES / DATUMS, footer.

use egui::{Rect, Ui, pos2, vec2};
use onecad_widgets::components::inputs::SearchField;
use onecad_widgets::components::lists::{RowState, SectionLabel, TreeRow};
use onecad_widgets::icon::{self, IconStyle};
use onecad_widgets::metrics::SECONDARY;

use super::chrome::Cx;

/// What the mock Explorer shows selected / editing.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ExplorerMode {
    Idle,
    /// Bracket selected with the given meta ("1 face", "1 edge").
    Picked(&'static str),
    /// Sketch 3 selected (extrude profile).
    Region,
    /// Sketch 3 open for editing, Bracket muted.
    Sketching,
}

pub fn show(ui: &mut Ui, cx: &Cx, mode: ExplorerMode) {
    ui.spacing_mut().item_spacing.y = 0.0;
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        let mut q = String::new();
        ui.add(
            SearchField::new("screens-explorer-filter", &mut q)
                .placeholder("Filter")
                .width(230.0),
        );
    });
    ui.add_space(4.0);
    // Design rows carry a 6px side margin inside the panel.
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(6, 0))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.add(SectionLabel::new("BODIES"));
            body_row(ui, mode);
            ui.add(SectionLabel::new("SKETCHES"));
            sketch_rows(ui, mode);
            ui.add(SectionLabel::new("DATUMS"));
            ui.add(TreeRow::new("origin", "Origin").trailing(Some("eyeOff")));
            ui.add(TreeRow::new("datum", "Mid plane").trailing(Some("eyeOn")));
        });
    footer(ui, cx);
}

fn body_row(ui: &mut Ui, mode: ExplorerMode) {
    let row = TreeRow::new("body", "Bracket");
    let row = match mode {
        ExplorerMode::Picked(meta) => row
            .meta(Some(meta))
            .state(RowState::Selected)
            .expanded(true),
        ExplorerMode::Sketching => row.meta(Some("reference")).state(RowState::Reference),
        ExplorerMode::Idle | ExplorerMode::Region => {
            row.meta(Some("Al 6061")).trailing(Some("eyeOn"))
        }
    };
    ui.add(row);
}

fn sketch_rows(ui: &mut Ui, mode: ExplorerMode) {
    ui.add(TreeRow::new("sketch", "Base profile").expanded(true));
    ui.add(TreeRow::new("sketch", "Upright profile").expanded(true));
    match mode {
        ExplorerMode::Sketching => {
            let editing = TreeRow::new("sketch", "Sketch 3")
                .meta(Some("editing"))
                .state(RowState::Editing)
                .expanded(true);
            ui.add(editing);
            for (icon, label) in [
                ("rect", "Rectangle · 4 lines"),
                ("circle", "Circle ⌀6"),
                ("constructionLine", "Centre line"),
                ("coincident", "7 constraints"),
            ] {
                ui.add(TreeRow::new(icon, label).child(true));
            }
        }
        ExplorerMode::Region => {
            let picked = TreeRow::new("sketch", "Sketch 3")
                .meta(Some("1 region"))
                .state(RowState::Selected)
                .expanded(true);
            ui.add(picked);
        }
        _ => {}
    }
}

/// 36px footer pinned to the panel bottom.
fn footer(ui: &mut Ui, cx: &Cx) {
    let max = ui.max_rect();
    let bar = Rect::from_min_max(pos2(max.left(), max.bottom() - 36.0), max.max);
    let t = cx.t;
    let p = ui.painter();
    p.hline(
        bar.x_range(),
        bar.top(),
        egui::Stroke::new(1.0, t.border_subtle),
    );
    let style = IconStyle {
        tint: t.ink4,
        bg: t.floating,
        grey: false,
    };
    let at = |x: f32| Rect::from_center_size(pos2(x, bar.center().y), vec2(16.0, 16.0));
    icon::paint(p, at(bar.left() + 20.0), "extension", cx.dark, style, 1.0);
    icon::paint(p, at(bar.right() - 20.0), "overflow", cx.dark, style, 1.0);
    p.text(
        pos2(bar.left() + 36.0, bar.center().y),
        egui::Align2::LEFT_CENTER,
        "3 extensions active",
        egui::FontId::proportional(SECONDARY),
        t.ink4,
    );
}
