//! Inspector panel content: Properties (README 03a) and History (README 07a).

use egui::{CornerRadius, Margin, RichText, Sense, Ui, pos2, vec2};
use onecad_widgets::components::inputs::Select;
use onecad_widgets::components::lists::{
    HistoryRow, HistoryState, PropertyRow, RollbackMarker, SectionLabel, Segment, SegmentedControl,
    property_row_with,
};
use onecad_widgets::components::shell::{PanelToggle, Side};
use onecad_widgets::fonts;
use onecad_widgets::icon::{self, IconStyle};
use onecad_widgets::metrics::{PANEL_TITLE, SECONDARY};

use super::chrome::Cx;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum InspectorView {
    Properties,
    History,
}

pub fn show(ui: &mut Ui, cx: &Cx, view: InspectorView) {
    ui.spacing_mut().item_spacing.y = 0.0;
    header(ui, view);
    match view {
        InspectorView::Properties => properties(ui, cx),
        InspectorView::History => history(ui, cx),
    }
}

fn header(ui: &mut Ui, view: InspectorView) {
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.add_space(8.0);
        let mut tab = usize::from(view == InspectorView::History);
        let tabs = [
            Segment {
                label: "Properties",
                icon: None,
            },
            Segment {
                label: "History",
                icon: None,
            },
        ];
        ui.add(SegmentedControl::new("screens-insp-tabs", &mut tab, &tabs).width(230.0));
        ui.add(PanelToggle::new(Side::Right).open(true).compact(true));
    });
    ui.add_space(8.0);
}

/// Left-inset block so rows and labels align with the 8px panel padding.
fn padded(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .inner_margin(Margin::symmetric(10, 0))
        .show(ui, add);
}

fn properties(ui: &mut Ui, cx: &Cx) {
    padded(ui, |ui| {
        title_block(ui, cx);
        ui.add(SectionLabel::new("GEOMETRY"));
        for (k, v) in [
            ("Area", "3 093.14 mm²"),
            ("Perimeter", "306.83 mm"),
            ("Normal", "+Z"),
            ("Offset", "Z 8.00 mm"),
        ] {
            ui.add(PropertyRow::new(k).value(v));
        }
        ui.add_space(4.0);
        ui.separator();
        ui.add(SectionLabel::new("BODY"));
        let materials = ["Aluminium 6061", "Steel 1018", "ABS"];
        property_row_with(ui, "Material", |ui| {
            Select::new("screens-material", materials[0], &materials)
                .width(150.0)
                .show(ui);
        });
        ui.add(PropertyRow::new("Mass").value("96.85 g"));
        ui.add(PropertyRow::new("Volume").value("35 870 mm³"));
    });
    hint(ui, cx);
}

fn title_block(ui: &mut Ui, cx: &Cx) {
    let t = cx.t;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let (well, _) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::hover());
        ui.painter()
            .rect_filled(well, CornerRadius::same(8), t.well);
        let style = IconStyle {
            tint: t.ink4,
            bg: t.well,
            grey: false,
        };
        icon::paint(ui.painter(), well.shrink(6.0), "body", cx.dark, style, 1.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            let title = RichText::new("Face 7")
                .size(PANEL_TITLE)
                .family(fonts::semibold());
            ui.label(title.color(t.ink));
            ui.label(
                RichText::new("Planar · on Bracket")
                    .size(SECONDARY)
                    .color(t.ink4),
            );
        });
    });
    ui.add_space(8.0);
}

/// Footer hint pinned to the panel bottom.
fn hint(ui: &mut Ui, cx: &Cx) {
    let max = ui.max_rect();
    let bar = egui::Rect::from_min_max(pos2(max.left(), max.bottom() - 36.0), max.max);
    let p = ui.painter();
    p.hline(
        bar.x_range(),
        bar.top(),
        egui::Stroke::new(1.0, cx.t.border_subtle),
    );
    let style = IconStyle {
        tint: cx.t.ink5,
        bg: cx.t.floating,
        grey: false,
    };
    let glyph =
        egui::Rect::from_center_size(pos2(bar.left() + 20.0, bar.center().y), vec2(14.0, 14.0));
    icon::paint(p, glyph, "help", cx.dark, style, 1.0);
    p.text(
        pos2(bar.left() + 34.0, bar.center().y),
        egui::Align2::LEFT_CENTER,
        "⇧-click adds faces · Esc clears",
        egui::FontId::proportional(11.5),
        cx.t.ink5,
    );
}

fn history(ui: &mut Ui, cx: &Cx) {
    let t = cx.t;
    ui.horizontal(|ui| {
        ui.add(SectionLabel::new("7 FEATURES"));
    });
    let rows = [
        (1, "sketch", "Base profile", "", HistoryState::Default),
        (2, "extrude", "Extrude 1", "8 mm", HistoryState::Default),
        (3, "sketch", "Upright profile", "", HistoryState::Default),
        (4, "extrude", "Extrude 2", "", HistoryState::Editing),
        (5, "hole", "Hole 1", "⌀10 ×2", HistoryState::Default),
        (6, "hole", "Hole 2", "⌀18", HistoryState::Default),
    ];
    let mut edit = "45.00".to_owned();
    for (i, icon, label, meta, state) in rows {
        let meta = (!meta.is_empty()).then_some(meta);
        let field = (state == HistoryState::Editing).then_some(&mut edit);
        HistoryRow::new(i, icon, label)
            .meta(meta)
            .state(state)
            .show(ui, field);
    }
    ui.add(RollbackMarker::new());
    HistoryRow::new(7, "fillet", "Fillet 1")
        .meta(Some("R6"))
        .state(HistoryState::RolledBack)
        .show(ui, None);
    ui.add_space(12.0);
    info_box(ui, cx, t.chip);
}

fn info_box(ui: &mut Ui, cx: &Cx, fill: egui::Color32) {
    let text = "Changes regenerate everything below. Features that can no longer resolve are flagged here, not deleted.";
    padded(ui, |ui| {
        egui::Frame::new()
            .fill(fill)
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::same(10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(text).size(SECONDARY).color(cx.t.ink4));
            });
    });
}
