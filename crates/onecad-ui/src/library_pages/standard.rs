//! 10a standard part page: family list, ISO size table with length chips, derived data panel.
use super::{
    kit,
    standard_data::{
        FAMILIES, MATERIALS, SIZES, StdState, derived, lengths_for, part_code, part_summary,
        subtitle,
    },
};
use crate::ui_state::UiState;
use egui::{
    Align, Align2, CornerRadius, CursorIcon, FontFamily, FontId, Layout, Rect, Sense, Stroke,
    StrokeKind, Ui, UiBuilder, pos2, vec2,
};
use onecad_widgets::{
    components::{
        button::{Button, ButtonKind},
        lists::{SectionLabel, Segment, SegmentedControl},
    },
    fonts,
    theme::look,
};

const LIST_W: f32 = 250.0;
const SIDE_W: f32 = 300.0;

pub fn show(state: &mut UiState, ui: &mut Ui) {
    let t = look(ui.ctx()).theme;
    let area = ui.max_rect();
    let list = Rect::from_min_max(area.min, pos2(area.left() + LIST_W, area.bottom()));
    let side = Rect::from_min_max(pos2(area.right() - SIDE_W, area.top()), area.max);
    let mid = Rect::from_min_max(
        pos2(list.right(), area.top()),
        pos2(side.left(), area.bottom()),
    );
    ui.painter().rect_filled(side, 0.0, t.panel);
    ui.painter().vline(
        list.right() - 0.5,
        list.y_range(),
        Stroke::new(1.0, t.border),
    );
    ui.painter().vline(
        side.left() + 0.5,
        side.y_range(),
        Stroke::new(1.0, t.border),
    );
    ui.scope_builder(
        UiBuilder::new().max_rect(list.shrink2(vec2(10.0, 0.0))),
        |ui| {
            family_list(state, ui);
        },
    );
    ui.scope_builder(
        UiBuilder::new().max_rect(mid.shrink2(vec2(28.0, 0.0))),
        |ui| {
            egui::ScrollArea::vertical()
                .id_salt("std-mid")
                .show(ui, |ui| middle(state, ui));
        },
    );
    side_panel(state, ui, side);
}

fn family_list(state: &mut UiState, ui: &mut Ui) {
    let t = look(ui.ctx()).theme;
    ui.add_space(22.0);
    if kit::breadcrumb(ui, &["Library", "Standard parts"]) == Some(0) {
        super::go_library(state, 0);
    }
    ui.add_space(6.0);
    ui.spacing_mut().item_spacing.y = 2.0;
    let std = &mut state.library.std;
    let mut group = "";
    for (i, f) in FAMILIES.iter().enumerate() {
        if f.group != group {
            group = f.group;
            ui.add(SectionLabel::new(group));
        }
        let h = if f.two_line { 40.0 } else { 30.0 };
        let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
        let on = std.family == i;
        let p = ui.painter();
        if on {
            p.rect_filled(rect, CornerRadius::same(6), t.sel_bg);
        } else if r.hovered() {
            p.rect_filled(rect, CornerRadius::same(6), t.hover);
        }
        let (ink, family) = if on {
            (t.sel_text, fonts::semibold())
        } else {
            (t.ink2, FontFamily::Proportional)
        };
        let x = rect.left() + 10.0;
        if f.two_line {
            let name = FontId::new(12.5, family);
            p.text(
                pos2(x, rect.center().y - 8.0),
                Align2::LEFT_CENTER,
                f.name,
                name,
                ink,
            );
            let code = FontId::new(11.0, fonts::mono());
            p.text(
                pos2(x, rect.center().y + 8.0),
                Align2::LEFT_CENTER,
                f.standard,
                code,
                t.ink5,
            );
        } else {
            let text = format!("{} · {}", f.name, f.standard);
            let font = FontId::new(12.5, family);
            p.text(
                pos2(x, rect.center().y),
                Align2::LEFT_CENTER,
                text,
                font,
                ink,
            );
        }
        if r.on_hover_cursor(CursorIcon::PointingHand).clicked() {
            std.family = i;
            std.inserted = false;
        }
    }
}

fn middle(state: &mut UiState, ui: &mut Ui) {
    let std = &mut state.library.std;
    ui.add_space(22.0);
    kit::title(ui, FAMILIES[std.family].name, 20.0);
    ui.add_space(4.0);
    kit::muted(ui, &subtitle(std.family), 12.5);
    ui.add_space(16.0);
    spec_table(ui, std);
    ui.add_space(16.0);
    let t = look(ui.ctx()).theme;
    let heading = format!("Length L for {} (mm)", SIZES[std.size].size);
    kit::label(ui, &heading, 12.0, FontFamily::Proportional, t.ink4);
    ui.add_space(8.0);
    length_chips(ui, std);
    ui.add_space(18.0);
    options(ui, std);
    ui.add_space(24.0);
}

fn spec_table(ui: &mut Ui, std: &mut StdState) {
    let t = look(ui.ctx()).theme;
    let w = ui.available_width();
    let (frame, _) =
        ui.allocate_exact_size(vec2(w, 34.0 + 36.0 * SIZES.len() as f32), Sense::hover());
    let weights = [1.0, 1.0, 1.0, 1.0, 1.0, 1.4];
    let total: f32 = weights.iter().sum();
    let inner = frame.shrink2(vec2(14.0, 0.0));
    let mut xs = [0.0; 6];
    let mut acc = 0.0;
    for (x, wt) in xs.iter_mut().zip(weights) {
        *x = inner.left() + acc;
        acc += inner.width() * wt / total;
    }
    let p = ui.painter();
    let head = Rect::from_min_size(frame.min, vec2(w, 34.0));
    let top = CornerRadius {
        nw: 10,
        ne: 10,
        sw: 0,
        se: 0,
    };
    p.rect_filled(head, top, t.panel);
    let font = FontId::new(11.0, fonts::medium());
    for (x, text) in xs.iter().zip(["Size", "Pitch", "d", "k", "s", "Lengths"]) {
        p.text(
            pos2(*x, head.center().y),
            Align2::LEFT_CENTER,
            text,
            font.clone(),
            t.ink5,
        );
    }
    // Subscript "k" after d, as in the design.
    p.text(
        pos2(xs[2] + 7.0, head.center().y + 3.0),
        Align2::LEFT_CENTER,
        "k",
        FontId::new(8.5, fonts::medium()),
        t.ink5,
    );
    for (i, row) in SIZES.iter().enumerate() {
        let rect = Rect::from_min_size(
            pos2(frame.left(), head.bottom() + 36.0 * i as f32),
            vec2(w, 36.0),
        );
        let r = ui.interact(rect, ui.scope_id().with(("size", i)), Sense::click());
        let on = std.size == i;
        let p = ui.painter();
        if on {
            p.rect_filled(rect, 0.0, t.sel_soft);
        } else if r.hovered() {
            p.rect_filled(rect, 0.0, t.hover);
        }
        p.hline(
            rect.x_range(),
            rect.top() + 0.5,
            Stroke::new(1.0, t.border_subtle),
        );
        if on {
            let bar = Rect::from_min_size(rect.min, vec2(3.0, rect.height()));
            p.rect_filled(bar, 0.0, t.accent);
        }
        let range = format!("{}–{}", row.lo, row.hi);
        let cells = [
            row.size,
            row.pitch,
            &fmt_num(row.dk),
            &fmt_num(row.k),
            row.s,
            &range,
        ];
        let font = FontId::new(12.5, fonts::mono());
        let ink = if on { t.sel_text } else { t.ink };
        for (x, text) in xs.iter().zip(cells) {
            p.text(
                pos2(*x, rect.center().y),
                Align2::LEFT_CENTER,
                text,
                font.clone(),
                ink,
            );
        }
        if r.on_hover_cursor(CursorIcon::PointingHand).clicked() {
            std.select_size(i);
        }
    }
    let p = ui.painter();
    p.rect_stroke(
        frame,
        CornerRadius::same(10),
        Stroke::new(1.0, t.border),
        StrokeKind::Inside,
    );
}

fn fmt_num(v: f32) -> String {
    v.to_string()
}

fn length_chips(ui: &mut Ui, std: &mut StdState) {
    let t = look(ui.ctx()).theme;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        for len in lengths_for(std.size) {
            let (rect, r) = ui.allocate_exact_size(vec2(42.0, 30.0), Sense::click());
            let on = std.length == len;
            let fill = match (on, r.hovered()) {
                (true, _) => t.accent,
                (false, true) => t.hover,
                (false, false) => t.surface,
            };
            let p = ui.painter();
            p.rect_filled(rect, CornerRadius::same(6), fill);
            if !on {
                p.rect_stroke(
                    rect,
                    CornerRadius::same(6),
                    Stroke::new(1.0, t.border_strong),
                    StrokeKind::Inside,
                );
            }
            let (ink, family) = if on {
                (t.on_accent, fonts::mono())
            } else {
                (t.ink2, fonts::mono())
            };
            p.text(
                rect.center(),
                Align2::CENTER_CENTER,
                len.to_string(),
                FontId::new(12.5, family),
                ink,
            );
            if r.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                std.length = len;
                std.inserted = false;
            }
        }
    });
}

fn options(ui: &mut Ui, std: &mut StdState) {
    let t = look(ui.ctx()).theme;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 24.0;
        ui.vertical(|ui| {
            kit::label(
                ui,
                "Material / class",
                12.0,
                FontFamily::Proportional,
                t.ink4,
            );
            ui.add_space(4.0);
            let segs = MATERIALS.map(|(label, _)| Segment { label, icon: None });
            ui.add(SegmentedControl::new(
                "std-material",
                &mut std.material,
                &segs,
            ));
        });
        ui.vertical(|ui| {
            kit::label(ui, "Thread", 12.0, FontFamily::Proportional, t.ink4);
            ui.add_space(4.0);
            let segs = ["Cosmetic", "Modeled"].map(|label| Segment { label, icon: None });
            ui.add(SegmentedControl::new("std-thread", &mut std.thread, &segs));
        });
    });
}

fn side_panel(state: &mut UiState, ui: &mut Ui, side: Rect) {
    let t = look(ui.ctx()).theme;
    let std = &mut state.library.std;
    let well = Rect::from_min_size(side.min, vec2(side.width(), 230.0));
    let p = ui.painter();
    p.rect_filled(well, 0.0, t.well);
    p.hline(side.x_range(), well.bottom(), Stroke::new(1.0, t.border));
    let icon = Rect::from_center_size(well.center(), vec2(96.0, 96.0));
    kit::paint_icon(p, icon, "thread", t.well, t.ink4);
    let body = Rect::from_min_max(
        pos2(side.left() + 16.0, well.bottom() + 14.0),
        pos2(side.right() - 16.0, side.bottom() - 110.0),
    );
    ui.scope_builder(UiBuilder::new().max_rect(body), |ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        let code = part_code(std.family, std.size, std.length);
        kit::label(ui, &code, 14.0, fonts::mono(), t.ink);
        let summary = part_summary(std.size, std.length, std.material, std.thread == 1);
        kit::muted(ui, &summary, 12.0);
        ui.add_space(12.0);
        for (name, value) in derived(std.size) {
            derived_row(ui, name, &value);
        }
    });
    let foot = Rect::from_min_max(pos2(side.left(), side.bottom() - 100.0), side.max);
    ui.painter()
        .hline(foot.x_range(), foot.top(), Stroke::new(1.0, t.border));
    ui.scope_builder(
        UiBuilder::new().max_rect(foot.shrink2(vec2(16.0, 14.0))),
        |ui| {
            footer(ui, std);
        },
    );
}

fn derived_row(ui: &mut Ui, name: &str, value: &str) {
    let t = look(ui.ctx()).theme;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::hover());
    let p = ui.painter();
    p.text(
        rect.left_center(),
        Align2::LEFT_CENTER,
        name,
        FontId::proportional(12.5),
        t.ink4,
    );
    p.text(
        rect.right_center(),
        Align2::RIGHT_CENTER,
        value,
        FontId::new(12.0, fonts::mono()),
        t.ink,
    );
}

fn footer(ui: &mut Ui, std: &mut StdState) {
    let t = look(ui.ctx()).theme;
    ui.spacing_mut().item_spacing = vec2(8.0, 10.0);
    ui.horizontal(|ui| {
        kit::label(ui, "Insert as", 12.0, FontFamily::Proportional, t.ink4);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let segs = ["Linked", "Copy"].map(|label| Segment { label, icon: None });
            ui.add(SegmentedControl::new(
                "std-insert-as",
                &mut std.insert_as,
                &segs,
            ));
        });
    });
    ui.horizontal(|ui| {
        let (label, icon) = if std.added {
            ("Added", "check")
        } else {
            ("Add to My components", "starred")
        };
        if ui.add(Button::new(label).icon(Some(icon))).clicked() {
            std.added = !std.added;
        }
        let (label, kind) = if std.inserted {
            ("Inserted", ButtonKind::Secondary)
        } else {
            ("Insert", ButtonKind::Primary)
        };
        let icon = if std.inserted { "check" } else { "plus" };
        if ui
            .add(Button::new(label).kind(kind).icon(Some(icon)))
            .clicked()
        {
            std.inserted = true;
        }
    });
}
