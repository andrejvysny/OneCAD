//! Explorer footer (11a / 19b): extension count and the "Active in Design" popover.
use crate::{app::App, layout::Chrome, ui_state::StartPage};
use egui::{Align2, Color32, CornerRadius, FontId, Id, Order, Rect, Sense, Stroke, Ui, pos2, vec2};
use onecad_widgets::{
    components::lists::{MenuRow, MenuSection, menu_divider, menu_frame},
    fonts, frame,
    icon::{self, IconStyle},
    theme::{Theme, look},
};

const FOOTER_H: f32 = 36.0;
const POPOVER_W: f32 = 250.0;
/// Mock Updates feed entry.
const UPDATE: &str = "Print Prep 1.3.0 available";

/// Active-in-Design extensions: (icon, name, summary, chip, verified).
const EXTENSIONS: [(&str, &str, &str, &str, bool); 3] = [
    (
        "body",
        "Sheet Metal",
        "5 tools · 1 section",
        "OneCAD",
        false,
    ),
    (
        "gear",
        "Gears & Threads",
        "4 generators · 1 tool",
        "OneCAD",
        false,
    ),
    (
        "workspace",
        "CNC Prep",
        "2 of 4 tools shown here",
        "Verified",
        true,
    ),
];

fn active_count(on: &[bool; 3]) -> usize {
    on.iter().filter(|b| **b).count()
}

fn count_label(n: usize) -> String {
    match n {
        0 => "No extensions active".to_owned(),
        1 => "1 extension active".to_owned(),
        n => format!("{n} extensions active"),
    }
}

enum Pick {
    Review,
    Customize,
    Manage,
}

fn footer_rect(panel: Rect) -> Rect {
    Rect::from_min_max(pos2(panel.left(), panel.bottom() - FOOTER_H), panel.max)
}

/// 36px footer pinned to the panel bottom; its meaning follows the Explorer mode.
pub fn footer(app: &mut App, ui: &mut Ui, panel: Rect) {
    let bar = footer_rect(panel);
    let l = look(ui.ctx());
    let t = &l.theme;
    ui.painter()
        .hline(bar.x_range(), bar.top(), Stroke::new(1.0, t.border_subtle));
    let r = ui.interact(bar, Id::unique("explorer-footer"), Sense::click());
    let f = &app.ui.editor_facade;
    let lib = f.library_mode;
    let open = f.popover_open;
    let on = r.hovered() || open;
    let label = if lib {
        "Open full library".to_owned()
    } else {
        count_label(active_count(&app.ui.shell_facade.extensions_on))
    };
    paint_footer(ui, &l.theme, l.dark, bar, &label, lib, on);
    if !lib && update_pending(app) {
        let c = pos2(bar.right() - 34.0, bar.center().y);
        ui.painter().circle_filled(c, 3.0, t.accent);
    }
    if r.clicked() {
        if lib {
            super::open_start(app, StartPage::Library);
        } else {
            app.ui.editor_facade.popover_open = !open;
        }
    }
}

fn update_pending(app: &App) -> bool {
    app.ui.extensions.counts().2 > 0
}

fn paint_footer(ui: &Ui, t: &Theme, dark: bool, bar: Rect, label: &str, lib: bool, on: bool) {
    let p = ui.painter();
    let ink = if on && !lib { t.accent } else { t.ink5 };
    let name = if lib { "component" } else { "extension" };
    let style = IconStyle {
        tint: ink,
        bg: t.floating,
        grey: false,
    };
    let glyph = Rect::from_center_size(pos2(bar.left() + 20.0, bar.center().y), vec2(14.0, 14.0));
    icon::paint(p, glyph, name, dark, style, 1.0);
    p.text(
        pos2(bar.left() + 34.0, bar.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(11.5),
        ink,
    );
    let (tail, font) = if lib {
        ("⇧L", fonts::mono())
    } else {
        ("⋯", egui::FontFamily::Proportional)
    };
    p.text(
        pos2(bar.right() - 14.0, bar.center().y),
        Align2::RIGHT_CENTER,
        tail,
        FontId::new(11.5, font),
        t.ink5,
    );
}

pub fn popover(app: &mut App, ctx: &egui::Context, chrome: &Chrome) {
    let f = &app.ui.editor_facade;
    if !f.popover_open || f.library_mode || !app.ui.explorer_open {
        return;
    }
    let bar = footer_rect(chrome.explorer);
    let at = pos2(chrome.explorer.left() + 8.0, bar.top() - 6.0);
    let on = app.ui.shell_facade.extensions_on;
    let mut pick = None;
    let out = frame::floating(Id::unique("ext-popover"), at, Order::Foreground)
        .pivot(Align2::LEFT_BOTTOM)
        .show(ctx, |ui| {
            menu_frame(ctx).show(ui, |ui| {
                ui.set_width(POPOVER_W - 14.0);
                ui.spacing_mut().item_spacing.y = 0.0;
                pick = body(ui, &on);
            });
        });
    let rect = out.response.rect;
    let outside = ctx.input(|i| i.pointer.any_pressed())
        && ctx
            .input(|i| i.pointer.interact_pos())
            .is_some_and(|p| !rect.contains(p) && !bar.contains(p));
    if outside {
        app.ui.editor_facade.popover_open = false;
    }
    if let Some(pick) = pick {
        app.ui.editor_facade.popover_open = false;
        match pick {
            Pick::Review => {
                app.ui.extensions.tab = crate::extension_pages::ManagerTab::Updates;
                super::open_start(app, StartPage::Extensions);
            }
            Pick::Manage => super::open_start(app, StartPage::Extensions),
            Pick::Customize => app.ui.shell_facade.customize_open = true,
        }
    }
}

fn body(ui: &mut Ui, on: &[bool; 3]) -> Option<Pick> {
    let mut pick = None;
    ui.add(MenuSection::new("ACTIVE IN DESIGN"));
    for (row, shown) in EXTENSIONS.iter().zip(on) {
        if *shown {
            ext_row(ui, row);
        }
    }
    if update_row(ui) {
        pick = Some(Pick::Review);
    }
    menu_divider(ui);
    if ui
        .add(MenuRow::new(None, "Customize Design workspace…"))
        .clicked()
    {
        pick = Some(Pick::Customize);
    }
    if ui.add(MenuRow::new(None, "Manage extensions…")).clicked() {
        pick = Some(Pick::Manage);
    }
    pick
}

fn ext_row(ui: &mut Ui, row: &(&str, &str, &str, &str, bool)) {
    let (icon_name, name, meta, chip, verified) = *row;
    let l = look(ui.ctx());
    let t = &l.theme;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
    let p = ui.painter();
    let style = IconStyle {
        tint: t.ink3,
        bg: t.floating,
        grey: false,
    };
    let glyph = Rect::from_center_size(pos2(rect.left() + 18.0, rect.center().y), vec2(20.0, 20.0));
    icon::paint(p, glyph, icon_name, l.dark, style, 1.0);
    let x = rect.left() + 36.0;
    p.text(
        pos2(x, rect.center().y - 7.0),
        Align2::LEFT_CENTER,
        name,
        FontId::new(12.5, fonts::medium()),
        t.ink,
    );
    p.text(
        pos2(x, rect.center().y + 8.0),
        Align2::LEFT_CENTER,
        meta,
        FontId::proportional(11.0),
        t.ink5,
    );
    let (fill, ink) = if verified {
        (t.ok_text.gamma_multiply(0.14), t.ok_text)
    } else {
        (t.chip, t.ink4)
    };
    chip_right(
        ui,
        pos2(rect.right() - 8.0, rect.center().y),
        chip,
        fill,
        ink,
    );
}

fn chip_right(ui: &Ui, right_mid: egui::Pos2, text: &str, fill: Color32, ink: Color32) {
    let p = ui.painter();
    let g = p.layout_no_wrap(text.to_owned(), FontId::new(11.0, fonts::medium()), ink);
    let size = g.size() + vec2(12.0, 6.0);
    let rect = Rect::from_min_size(pos2(right_mid.x - size.x, right_mid.y - size.y / 2.0), size);
    p.rect_filled(rect, CornerRadius::same(5), fill);
    p.galley(rect.min + vec2(6.0, 3.0), g, ink);
}

/// Accent-soft "update available" row; true when clicked.
fn update_row(ui: &mut Ui) -> bool {
    let t = look(ui.ctx()).theme;
    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    let fill = if r.hovered() { t.sel_bg } else { t.sel_soft };
    let p = ui.painter();
    p.rect_filled(rect.shrink2(vec2(4.0, 0.0)), CornerRadius::same(6), fill);
    p.text(
        pos2(rect.left() + 12.0, rect.center().y),
        Align2::LEFT_CENTER,
        UPDATE,
        FontId::proportional(12.0),
        t.sel_text,
    );
    p.text(
        pos2(rect.right() - 12.0, rect.center().y),
        Align2::RIGHT_CENTER,
        "Review",
        FontId::new(12.0, fonts::semibold()),
        t.accent,
    );
    r.clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_label_pluralises() {
        assert_eq!(count_label(0), "No extensions active");
        assert_eq!(count_label(1), "1 extension active");
        assert_eq!(count_label(3), "3 extensions active");
        assert_eq!(active_count(&[true, false, true]), 2);
    }
}
