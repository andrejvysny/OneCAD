//! Customize Design workspace modal (Extensions 19e). Toggles live in `ShellFacade`.
use crate::app::App;
use egui::{
    Color32, Context, CornerRadius, FontId, Frame, Margin, Pos2, Rect, Sense, Stroke, Ui, vec2,
};
use onecad_widgets::{
    components::{
        button::{Button, ButtonKind},
        inputs::Toggle,
        lists::SectionLabel,
    },
    fonts,
    icon::{self, IconStyle},
    metrics::{Elevation, RADIUS_XL, shadow},
    theme::look,
};

const WIDTH: f32 = 340.0;
const ROW_H: f32 = 48.0;

/// (icon, name, meta) per extension row.
const ROWS: [(&str, &str, &str); 3] = [
    ("body", "Sheet Metal", "5 tools · 1 section"),
    ("gear", "Gears & Threads", "4 generators · 1 tool"),
    ("panelLeft", "CNC Prep", "Only Dogbone, T-bone shown"),
];

pub fn dialog(app: &mut App, ctx: &Context) {
    if !app.ui.shell_facade.customize_open {
        return;
    }
    let l = look(ctx);
    let t = &l.theme;
    let card = Frame::new()
        .fill(t.floating)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CornerRadius::same(RADIUS_XL))
        .inner_margin(Margin::same(20))
        .shadow(shadow(Elevation::Card, t, l.dark));
    let modal = egui::Modal::new(egui::Id::unique("customize-workspace"))
        .frame(card)
        .show(ctx, |ui| {
            ui.set_width(WIDTH);
            contents(app, ui)
        });
    if modal.inner || modal.should_close() {
        app.ui.shell_facade.customize_open = false;
    }
}

/// Returns true when "Done" was pressed.
fn contents(app: &mut App, ui: &mut Ui) -> bool {
    let l = look(ui.ctx());
    let t = &l.theme;
    ui.spacing_mut().item_spacing.y = 0.0;
    ui.label(
        egui::RichText::new("Customize Design workspace")
            .font(FontId::new(15.0, fonts::semibold()))
            .color(t.ink),
    );
    ui.label(
        egui::RichText::new("Show extension contributions here")
            .size(12.5)
            .color(t.ink5),
    );
    ui.add_space(8.0);
    ui.add(SectionLabel::new("Extensions"));
    for (i, (icon_name, name, meta)) in ROWS.into_iter().enumerate() {
        let pick = i == 2;
        extension_row(
            ui,
            (icon_name, name, meta),
            &mut app.ui.shell_facade.extensions_on[i],
            pick,
            i > 0,
        );
    }
    ui.add_space(12.0);
    ui.label(
        egui::RichText::new("Hidden tools are still in ⌘K. Other workspaces are unaffected.")
            .size(12.0)
            .color(t.ink5),
    );
    ui.add_space(16.0);
    let mut done = false;
    ui.horizontal(|ui| {
        if text_link(ui, "Reset Design workspace") {
            app.ui.shell_facade.reset_extensions();
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            done = ui
                .add(Button::new("Done").kind(ButtonKind::Primary))
                .clicked();
        });
    });
    done
}

fn extension_row(
    ui: &mut Ui,
    (icon_name, name, meta): (&str, &str, &str),
    on: &mut bool,
    pick: bool,
    divider: bool,
) {
    let l = look(ui.ctx());
    let t = &l.theme;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::hover());
    let p = ui.painter();
    if divider {
        p.hline(
            rect.x_range(),
            rect.top() + 0.5,
            Stroke::new(1.0, t.border_subtle),
        );
    }
    let cy = rect.center().y;
    let at = Rect::from_center_size(Pos2::new(rect.left() + 12.0, cy), vec2(24.0, 24.0));
    let style = IconStyle {
        tint: t.ink3,
        bg: t.floating,
        grey: false,
    };
    icon::paint(p, at, icon_name, l.dark, style, 1.0);
    let g = p.layout_no_wrap(name.to_owned(), FontId::new(13.0, fonts::semibold()), t.ink);
    let m = p.layout_no_wrap(meta.to_owned(), FontId::proportional(11.5), t.ink5);
    let block = g.size().y + 1.0 + m.size().y;
    let top = cy - block / 2.0;
    let gh = g.size().y;
    p.galley(Pos2::new(rect.left() + 34.0, top), g, t.ink);
    p.galley(Pos2::new(rect.left() + 34.0, top + gh + 1.0), m, t.ink5);
    let toggle = Rect::from_center_size(Pos2::new(rect.right() - 15.0, cy), vec2(30.0, 18.0));
    ui.put(toggle, Toggle::new(on));
    if pick {
        let g = ui.painter().layout_no_wrap(
            "Pick tools".to_owned(),
            FontId::new(12.0, fonts::medium()),
            t.accent,
        );
        let r = Rect::from_center_size(
            Pos2::new(toggle.left() - 12.0 - g.size().x / 2.0, cy),
            g.size() + vec2(8.0, 8.0),
        );
        let resp = ui.interact(r, ui.scope_id().with("pick-tools"), Sense::click());
        ui.painter()
            .galley(r.center() - g.size() / 2.0, g, Color32::PLACEHOLDER);
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Pick tools"));
    }
    // `put` leaves the cursor at the toggle's bottom; resume below the full row.
    ui.advance_cursor_after_rect(rect);
}

/// Accent text link; returns true when clicked.
fn text_link(ui: &mut Ui, text: &str) -> bool {
    let t = look(ui.ctx()).theme;
    let g = ui.painter().layout_no_wrap(
        text.to_owned(),
        FontId::new(12.5, fonts::medium()),
        t.accent,
    );
    let (rect, r) = ui.allocate_exact_size(g.size() + vec2(0.0, 8.0), Sense::click());
    ui.painter()
        .galley(rect.center() - g.size() / 2.0, g, t.accent);
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, text));
    r.clicked()
}
