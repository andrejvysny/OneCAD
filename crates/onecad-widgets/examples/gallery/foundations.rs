use egui::{Color32, RichText, Sense, Stroke, Vec2};
use onecad_widgets::frame::surface;
use onecad_widgets::metrics::{self as m, Elevation};
use onecad_widgets::{Theme, fonts};

fn swatches(ui: &mut egui::Ui, title: &str, items: &[(&str, Color32)]) {
    ui.label(RichText::new(title).size(m::SECTION).strong());
    ui.horizontal_wrapped(|ui| {
        for (name, c) in items {
            ui.vertical(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::new(88.0, 36.0), Sense::hover());
                ui.painter().rect(
                    r,
                    m::RADIUS_SM,
                    *c,
                    Stroke::new(1.0, Color32::from_black_alpha(40)),
                    egui::StrokeKind::Inside,
                );
                ui.label(RichText::new(*name).size(m::SECTION));
                let hex = format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b());
                ui.label(RichText::new(hex).size(m::SECTION).family(fonts::mono()));
            });
        }
    });
}

fn colours(ui: &mut egui::Ui, t: &Theme) {
    swatches(
        ui,
        "Ink",
        &[
            ("ink", t.ink),
            ("ink2", t.ink2),
            ("ink3", t.ink3),
            ("ink4", t.ink4),
            ("ink5", t.ink5),
            ("ink6", t.ink6),
            ("ink7", t.ink7),
        ],
    );
    swatches(
        ui,
        "Surfaces",
        &[
            ("panel", t.panel),
            ("surface", t.surface),
            ("floating", t.floating),
            ("canvas", t.canvas),
            ("chip", t.chip),
            ("well", t.well),
            ("titlebar", t.titlebar),
            ("statusbar", t.statusbar),
        ],
    );
    swatches(
        ui,
        "Borders",
        &[
            ("border", t.border),
            ("border_strong", t.border_strong),
            ("border_subtle", t.border_subtle),
            ("sel_border", t.sel_border),
        ],
    );
    swatches(
        ui,
        "Accent / selection",
        &[
            ("accent", t.accent),
            ("accent_hover", t.accent_hover),
            ("icon_accent", t.icon_accent),
            ("sel_bg", t.sel_bg),
            ("sel_soft", t.sel_soft),
            ("sel_text", t.sel_text),
        ],
    );
    swatches(
        ui,
        "State",
        &[
            ("created_fill", t.created_fill),
            ("created_stroke", t.created_stroke),
            ("created_text", t.created_text),
            ("running_bg", t.running_bg),
            ("running_border", t.running_border),
            ("ok_text", t.ok_text),
            ("expression", t.expression),
            ("warn", t.warn),
            ("danger_strong", t.danger_strong),
        ],
    );
}

fn type_scale(ui: &mut egui::Ui) {
    let rows = [
        ("Title", m::TITLE),
        ("Panel title", m::PANEL_TITLE),
        ("Body", m::BODY),
        ("Control", m::CONTROL),
        ("Secondary", m::SECONDARY),
        ("Section", m::SECTION),
    ];
    for (n, s) in rows {
        ui.label(RichText::new(format!("{n} {s}px — The quick brown fox")).size(s));
    }
    ui.label(
        RichText::new("Mono value 12.5 — 42.000 mm")
            .size(m::MONO_VALUE)
            .family(fonts::mono()),
    );
}

fn boxes(ui: &mut egui::Ui, t: &Theme, dark: bool) {
    ui.label(RichText::new("Radii").size(m::SECTION).strong());
    ui.horizontal(|ui| {
        for (n, r) in [
            ("sm", m::RADIUS_SM),
            ("md", m::RADIUS_MD),
            ("lg", m::RADIUS_LG),
            ("xl", m::RADIUS_XL),
        ] {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(48.0), Sense::hover());
            ui.painter().rect(
                rect,
                r,
                t.chip,
                Stroke::new(1.0, t.border),
                egui::StrokeKind::Inside,
            );
            ui.label(format!("{n} {r}"));
        }
    });
    ui.label(RichText::new("Control heights").size(m::SECTION).strong());
    ui.horizontal(|ui| {
        for h in [
            m::H_TOOLTIP,
            m::H_CHIP,
            m::H_FIELD,
            m::H_ROW,
            m::H_TOOL,
            m::H_SQUARE,
            m::H_TITLEBAR,
        ] {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(40.0, h), Sense::hover());
            ui.painter().rect(
                rect,
                m::RADIUS_SM,
                t.well,
                Stroke::new(1.0, t.border),
                egui::StrokeKind::Inside,
            );
            ui.label(format!("{h}"));
        }
    });
    ui.label(RichText::new("Elevation").size(m::SECTION).strong());
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        for (n, e) in [
            ("pill", Elevation::Pill),
            ("panel", Elevation::Panel),
            ("card", Elevation::Card),
            ("tooltip", Elevation::Tooltip),
        ] {
            surface(t, dark, m::RADIUS_LG, e)
                .inner_margin(16)
                .show(ui, |ui| ui.label(n));
            ui.add_space(24.0);
        }
    });
    ui.add_space(24.0);
    ui.label(RichText::new("Spacing").size(m::SECTION).strong());
    ui.horizontal(|ui| {
        for s in [m::S4, m::S6, m::S8, m::S10, m::S12, m::S16, m::S20, m::S24] {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(s, 16.0), Sense::hover());
            ui.painter().rect_filled(rect, 0.0, t.accent);
            ui.add_space(8.0);
        }
    });
}

pub fn show(ui: &mut egui::Ui, t: &Theme, dark: bool) {
    colours(ui, t);
    type_scale(ui);
    boxes(ui, t, dark);
}
