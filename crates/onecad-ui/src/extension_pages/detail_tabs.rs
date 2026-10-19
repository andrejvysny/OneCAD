//! Detail Settings (20a) and Changelog tabs.
use super::{
    catalog::Ext,
    kit::{self, fid},
    state::ExtensionsUi,
};
use egui::{Align2, Rect, RichText, Sense, Ui, UiBuilder, pos2, vec2};
use onecad_widgets::{
    components::{
        inputs::{NumberField, Select, Toggle},
        lists::{Segment, SegmentedControl},
    },
    fonts,
    theme::look,
};

const ROW_H: f32 = 48.0;
const CONTROL_X: f32 = 245.0;
const RESOLUTION: [&str; 3] = ["Low", "Medium", "High"];

pub fn settings(ui: &mut Ui, x: &mut ExtensionsUi, ext: &Ext) {
    let th = look(ui.ctx()).theme;
    if ext.id != "cnc-prep" {
        ui.add(
            egui::Label::new(
                RichText::new("This extension has no settings.")
                    .size(13.0)
                    .color(th.ink5),
            )
            .selectable(false),
        );
        return;
    }
    let w = ui.available_width().min(780.0);
    let (rect, _) = ui.allocate_exact_size(vec2(w, 4.0 * ROW_H), egui::Sense::hover());
    let labels = [
        ("Relief style", "Used when Dogbone starts"),
        ("Default tool diameter", "Flat end mill"),
        (
            "Mark unreachable areas",
            "Red overlay in the Tool reach layer",
        ),
        ("Check resolution", "Higher is slower to update"),
    ];
    for (i, (label, sub)) in labels.iter().enumerate() {
        let top = rect.top() + i as f32 * ROW_H;
        kit::text(
            ui.painter(),
            pos2(rect.left(), top + 4.0),
            label,
            13.0,
            fonts::medium(),
            th.ink,
        );
        kit::text(
            ui.painter(),
            pos2(rect.left(), top + 22.0),
            sub,
            12.0,
            egui::FontFamily::Proportional,
            th.ink5,
        );
        let area = Rect::from_min_size(
            pos2(rect.left() + CONTROL_X, top),
            vec2(w - CONTROL_X, ROW_H - 6.0),
        );
        let mut child = ui.new_child(UiBuilder::new().max_rect(area));
        control(&mut child, x, i);
    }
    ui.add_space(8.0);
    footer(ui, x, w);
}

fn control(ui: &mut Ui, x: &mut ExtensionsUi, i: usize) {
    match i {
        0 => {
            let segs = [
                Segment {
                    label: "Dogbone",
                    icon: None,
                },
                Segment {
                    label: "T-bone",
                    icon: None,
                },
                Segment {
                    label: "Minimal",
                    icon: None,
                },
            ];
            ui.add(SegmentedControl::new("ext-relief", &mut x.settings.relief, &segs).width(218.0));
        }
        1 => {
            ui.add(
                NumberField::new("ext-diameter", &mut x.settings.diameter)
                    .unit("mm")
                    .width(162.0),
            );
        }
        2 => {
            ui.add(Toggle::new(&mut x.settings.mark_unreachable));
        }
        _ => {
            let current = RESOLUTION[x.settings.resolution.min(2)];
            if let Some(n) = Select::new("ext-resolution", current, &RESOLUTION)
                .width(202.0)
                .show(ui)
                .chosen
            {
                x.settings.resolution = n;
            }
        }
    }
}

fn footer(ui: &mut Ui, x: &mut ExtensionsUi, w: f32) {
    let l = look(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(w, 34.0), Sense::hover());
    let p = ui.painter().clone();
    p.hline(
        rect.x_range(),
        rect.top(),
        egui::Stroke::new(1.0, l.theme.border),
    );
    kit::icon_at(
        &p,
        pos2(rect.left() + 7.0, rect.center().y + 2.0),
        14.0,
        "help",
        l.theme.ink5,
        l.theme.surface,
        l.dark,
    );
    p.text(
        pos2(rect.left() + 22.0, rect.center().y + 2.0),
        Align2::LEFT_CENTER,
        "These apply to every document. Each relief is a regular Cut feature you can edit in History.",
        fid(12.0, egui::FontFamily::Proportional),
        l.theme.ink5,
    );
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(rect.shrink2(vec2(0.0, 6.0)))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    if kit::link(&mut child, "Reset to defaults", l.theme.accent, 12.0).clicked() {
        x.reset_settings();
    }
}

pub fn changelog(ui: &mut Ui, ext: &Ext) {
    let th = look(ui.ctx()).theme;
    for rel in ext.log {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let mono = RichText::new(rel.version)
                .size(13.0)
                .family(fonts::mono())
                .color(th.ink);
            ui.add(egui::Label::new(mono).selectable(false));
            ui.add(
                egui::Label::new(RichText::new(rel.date).size(12.5).color(th.ink5))
                    .selectable(false),
            );
        });
        for note in rel.notes {
            let (rect, _) =
                ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
            ui.painter()
                .circle_filled(pos2(rect.left() + 3.0, rect.center().y), 1.5, th.ink5);
            kit::text(
                ui.painter(),
                pos2(rect.left() + 14.0, rect.top() + 2.0),
                note,
                13.0,
                egui::FontFamily::Proportional,
                th.ink3,
            );
        }
        ui.add_space(16.0);
    }
}
