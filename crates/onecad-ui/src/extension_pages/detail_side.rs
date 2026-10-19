//! Right column of the detail Overview (17a): permissions and facts.
use super::{
    catalog::Ext,
    kit::{self, fid},
};
use egui::{Align2, Rect, RichText, Sense, Ui, pos2, vec2};
use onecad_widgets::{fonts, theme::look};

pub fn column(ui: &mut Ui, ext: &Ext) {
    let th = look(ui.ctx()).theme;
    kit::caption(ui, "Permissions");
    ui.add_space(4.0);
    for perm in ext.permissions {
        permission_row(ui, perm.icon, perm.title, perm.reason);
        ui.add_space(8.0);
    }
    ui.add(
        egui::Label::new(RichText::new(ext.no_access).size(12.0).color(th.ink5)).selectable(false),
    );
    ui.add_space(14.0);
    divider(ui);
    ui.add_space(14.0);
    facts(ui, ext);
    ui.add_space(14.0);
    info_box(ui);
}

fn permission_row(ui: &mut Ui, icon: &str, title: &str, reason: &str) {
    let l = look(ui.ctx());
    let th = &l.theme;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());
    let p = ui.painter();
    kit::well(
        p,
        Rect::from_min_size(rect.min + vec2(0.0, 3.0), vec2(28.0, 28.0)),
        icon,
        6,
        14.0,
        th,
        l.dark,
    );
    kit::text(
        p,
        pos2(rect.left() + 38.0, rect.top()),
        title,
        12.5,
        fonts::medium(),
        th.ink,
    );
    kit::text(
        p,
        pos2(rect.left() + 38.0, rect.top() + 17.0),
        reason,
        12.0,
        egui::FontFamily::Proportional,
        th.ink5,
    );
}

fn divider(ui: &mut Ui) {
    let th = look(ui.ctx()).theme;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        egui::Stroke::new(1.0, th.border),
    );
}

fn facts(ui: &mut Ui, ext: &Ext) {
    let th = look(ui.ctx()).theme;
    let rows: [(&str, &str, bool); 6] = [
        ("Version", ext.version, true),
        ("Updated", ext.updated, false),
        ("Works with", "OneCAD 0.2 and later", false),
        ("Size", ext.size, true),
        ("License", "MIT", false),
        ("Source", "OneCAD catalogue", false),
    ];
    for (key, value, mono) in rows {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 23.0), Sense::hover());
        let p = ui.painter();
        p.text(
            pos2(rect.left(), rect.center().y),
            Align2::LEFT_CENTER,
            key,
            fid(12.5, egui::FontFamily::Proportional),
            th.ink4,
        );
        let (family, color) = match (mono, key == "Source") {
            (true, _) => (fonts::mono(), th.ink),
            (_, true) => (egui::FontFamily::Proportional, th.accent),
            _ => (egui::FontFamily::Proportional, th.ink),
        };
        p.text(
            pos2(rect.left() + 110.0, rect.center().y),
            Align2::LEFT_CENTER,
            value,
            fid(12.5, family),
            color,
        );
    }
}

fn info_box(ui: &mut Ui) {
    let th = look(ui.ctx()).theme;
    let text = "Used in 4 documents. Disabling or uninstalling keeps its stock and setup data in them; it shows again when the extension is back.";
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 74.0), Sense::hover());
    ui.painter().rect_filled(rect, 8, th.well);
    kit::paragraph(
        ui.painter(),
        rect.min + vec2(12.0, 11.0),
        text,
        12.0,
        18.0,
        rect.width() - 24.0,
        th.ink4,
    );
}
