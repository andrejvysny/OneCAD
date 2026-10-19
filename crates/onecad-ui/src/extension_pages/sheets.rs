//! Modal sheets: install consent (18a/18b), update review (21a) and the post-install toast.
use super::{
    catalog::{self, Adds, Ext, Permission, Trust},
    kit::{self, fid},
    state::{ExtensionsUi, Sheet},
};
use egui::{Align, Color32, Context, Layout, Order, Rect, RichText, Sense, Ui, pos2, vec2};
use onecad_widgets::{
    components::button::{Button, ButtonKind},
    fonts, frame,
    metrics::{Elevation, shadow},
    theme::{Look, look},
};

pub fn show(ctx: &Context, x: &mut ExtensionsUi) {
    let Some(sheet) = x.sheet else { return };
    let l = look(ctx);
    let (id, width) = match sheet {
        Sheet::Install(id) => (
            id,
            if catalog::find(id).is_some_and(|e| e.trust == Trust::Community) {
                420.0
            } else {
                440.0
            },
        ),
        Sheet::Update(id) => (id, 420.0),
    };
    let Some(ext) = catalog::find(id) else {
        x.close_sheet();
        return;
    };
    let card = egui::Frame::new()
        .fill(l.theme.surface)
        .stroke(egui::Stroke::new(1.0, l.theme.border))
        .corner_radius(12)
        .inner_margin(20)
        .shadow(shadow(Elevation::Card, &l.theme, l.dark));
    let mut confirm = false;
    let mut cancel = false;
    let modal = egui::Modal::new(egui::Id::unique(("ext-sheet", id)))
        .backdrop_color(Color32::from_black_alpha(102))
        .frame(card)
        .show(ctx, |ui| {
            ui.set_width(width - 40.0);
            ui.spacing_mut().item_spacing.y = 0.0;
            match sheet {
                Sheet::Install(_) => install_body(ui, &l, ext, &mut confirm, &mut cancel),
                Sheet::Update(_) => update_body(ui, &l, ext, &mut confirm, &mut cancel),
            }
        });
    if confirm {
        let now = ctx.input(|i| i.time);
        match sheet {
            Sheet::Install(_) => {
                if let Some(text) = x.confirm_install() {
                    x.show_toast(text, now);
                }
            }
            Sheet::Update(_) => x.confirm_update(),
        }
    } else if cancel || modal.should_close() {
        x.close_sheet();
    }
}

fn header(ui: &mut Ui, l: &Look, ext: &Ext, title: &str, tail: Option<&str>, trust_line: bool) {
    let th = &l.theme;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 48.0), Sense::hover());
    let p = ui.painter().clone();
    kit::well(
        &p,
        Rect::from_min_size(rect.min, vec2(48.0, 48.0)),
        ext.icon,
        10,
        28.0,
        th,
        l.dark,
    );
    let tx = rect.left() + 60.0;
    kit::text(
        &p,
        pos2(tx, rect.top() + 4.0),
        title,
        15.0,
        fonts::semibold(),
        th.ink,
    );
    let cy = rect.top() + 34.0;
    if trust_line {
        kit::publisher_line(&p, tx, cy, ext.publisher, ext.trust, tail, l);
    } else if let Some(t) = tail {
        p.text(
            pos2(tx, cy),
            egui::Align2::LEFT_CENTER,
            t,
            fid(12.0, fonts::mono()),
            th.ink5,
        );
    }
    ui.add_space(14.0);
}

fn adds_row(ui: &mut Ui, l: &Look, a: &Adds) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
    let p = ui.painter();
    kit::icon_at(
        p,
        pos2(rect.left() + 8.0, rect.center().y),
        16.0,
        a.icon,
        l.theme.ink3,
        l.theme.surface,
        l.dark,
    );
    p.text(
        pos2(rect.left() + 28.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        a.name,
        fid(13.0, egui::FontFamily::Proportional),
        l.theme.ink,
    );
    kit::text_right(
        p,
        rect.right(),
        rect.center().y,
        a.location,
        12.5,
        l.theme.ink5,
    );
}

fn permission_row(ui: &mut Ui, l: &Look, perm: &Permission) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
    let p = ui.painter();
    kit::well(
        p,
        Rect::from_min_size(rect.min + vec2(0.0, 6.0), vec2(28.0, 28.0)),
        perm.icon,
        6,
        14.0,
        &l.theme,
        l.dark,
    );
    kit::text(
        p,
        pos2(rect.left() + 36.0, rect.top() + 4.0),
        perm.title,
        13.0,
        fonts::medium(),
        l.theme.ink,
    );
    kit::text(
        p,
        pos2(rect.left() + 36.0, rect.top() + 21.0),
        perm.reason,
        12.5,
        egui::FontFamily::Proportional,
        l.theme.ink5,
    );
}

fn install_body(ui: &mut Ui, l: &Look, ext: &Ext, confirm: &mut bool, cancel: &mut bool) {
    let th = &l.theme;
    let tail = format!("{} · {}", ext.version, ext.size);
    header(
        ui,
        l,
        ext,
        &format!("Install {}?", ext.name),
        Some(&tail),
        true,
    );
    kit::caption(ui, "Adds");
    for a in ext.adds {
        adds_row(ui, l, a);
    }
    ui.add_space(10.0);
    kit::caption(ui, "Can");
    if ext.permissions.is_empty() {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::hover());
        kit::paragraph(
            ui.painter(),
            rect.min,
            "Nothing beyond adding materials. No document, file or network access.",
            13.0,
            18.0,
            rect.width(),
            th.ink4,
        );
    }
    for perm in ext.permissions {
        permission_row(ui, l, perm);
    }
    ui.add_space(8.0);
    if ext.trust == Trust::Community {
        warn_box(ui, l, ext.publisher);
        ui.add_space(14.0);
    }
    ui.add_space(8.0);
    footer(ui, l, "Install", ext.trust, confirm, cancel);
}

fn warn_box(ui: &mut Ui, l: &Look, publisher: &str) {
    let th = &l.theme;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 52.0), Sense::hover());
    kit::bordered(ui.painter(), rect, 8, th.warn_surface, th.warn_border);
    kit::icon_at(
        ui.painter(),
        pos2(rect.left() + 20.0, rect.top() + 18.0),
        14.0,
        "warning",
        th.warn,
        th.warn_surface,
        l.dark,
    );
    let text = format!(
        "OneCAD has not reviewed community extensions. Install only if you trust {publisher}."
    );
    kit::paragraph(
        ui.painter(),
        pos2(rect.left() + 34.0, rect.top() + 10.0),
        &text,
        12.5,
        17.0,
        rect.width() - 46.0,
        th.warn,
    );
}

fn footer(
    ui: &mut Ui,
    l: &Look,
    ok_label: &str,
    trust: Trust,
    confirm: &mut bool,
    cancel: &mut bool,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        match trust {
            Trust::Verified => {
                ui.add(
                    egui::Label::new(
                        RichText::new("Free · turn off any time")
                            .size(12.5)
                            .color(l.theme.ink5),
                    )
                    .selectable(false),
                );
            }
            Trust::Community => {
                kit::link(ui, "View source", l.theme.accent, 12.5);
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui
                .add(Button::new(ok_label).kind(ButtonKind::Primary))
                .clicked()
            {
                *confirm = true;
            }
            if ui.add(Button::new("Cancel")).clicked() {
                *cancel = true;
            }
        });
    });
}

fn update_body(ui: &mut Ui, l: &Look, ext: &Ext, confirm: &mut bool, cancel: &mut bool) {
    let th = &l.theme;
    let Some(info) = catalog::update_info(ext.id) else {
        return;
    };
    let tail = format!("{} → {}", info.from, info.to);
    header(
        ui,
        l,
        ext,
        &format!("Update {}", ext.name),
        Some(&tail),
        false,
    );
    for note in info.notes {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 23.0), Sense::hover());
        ui.painter()
            .circle_filled(pos2(rect.left() + 3.0, rect.center().y), 1.5, th.ink5);
        kit::text(
            ui.painter(),
            pos2(rect.left() + 14.0, rect.top() + 3.0),
            note,
            13.0,
            egui::FontFamily::Proportional,
            th.ink2,
        );
    }
    ui.add_space(12.0);
    kit::caption(ui, "New permission");
    ui.add_space(4.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 54.0), Sense::hover());
    kit::bordered(ui.painter(), rect, 8, th.sel_soft, th.sel_border);
    kit::icon_at(
        ui.painter(),
        pos2(rect.left() + 26.0, rect.center().y),
        16.0,
        info.permission.icon,
        th.ink3,
        th.sel_soft,
        l.dark,
    );
    kit::text(
        ui.painter(),
        pos2(rect.left() + 48.0, rect.top() + 9.0),
        info.permission.title,
        13.0,
        fonts::medium(),
        th.ink,
    );
    kit::text(
        ui.painter(),
        pos2(rect.left() + 48.0, rect.top() + 28.0),
        info.permission.reason,
        12.5,
        egui::FontFamily::Proportional,
        th.ink4,
    );
    ui.add_space(12.0);
    ui.add(
        egui::Label::new(RichText::new(info.unchanged).size(12.5).color(th.ink5)).selectable(false),
    );
    ui.add_space(18.0);
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if ui
            .add(Button::new("Allow & update").kind(ButtonKind::Primary))
            .clicked()
        {
            *confirm = true;
        }
        if ui.add(Button::new("Later")).clicked() {
            *cancel = true;
        }
    });
}

/// Transient bottom-centre toast; drops itself after its deadline.
pub fn toast(ctx: &Context, x: &mut ExtensionsUi) {
    let Some(t) = &x.toast else { return };
    let now = ctx.input(|i| i.time);
    if now >= t.until {
        x.toast = None;
        return;
    }
    ctx.request_repaint_after(std::time::Duration::from_millis(250));
    let l = look(ctx);
    let g = ctx.fonts_mut(|f| {
        f.layout_no_wrap(
            t.text.clone(),
            fid(13.0, egui::FontFamily::Proportional),
            l.theme.tooltip_text,
        )
    });
    let screen = ctx.content_rect();
    let w = g.size().x + 44.0;
    let pos = pos2(screen.center().x - w / 2.0, screen.bottom() - 70.0);
    frame::floating(egui::Id::unique("ext-toast"), pos, Order::Tooltip).show(ctx, |ui| {
        egui::Frame::new()
            .fill(l.theme.tooltip)
            .corner_radius(8)
            .inner_margin(egui::Margin::symmetric(14, 10))
            .shadow(shadow(Elevation::Tooltip, &l.theme, l.dark))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(&t.text)
                                .size(13.0)
                                .color(l.theme.tooltip_text),
                        )
                        .selectable(false),
                    );
                });
            });
    });
}
