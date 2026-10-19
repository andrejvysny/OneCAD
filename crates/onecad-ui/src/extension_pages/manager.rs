//! Extensions manager (16a): header, Discover / Installed / Updates tabs and the card grid.
use super::{
    catalog::{CATALOG, CATEGORY_CHIPS, Ext, SOURCES},
    developer, installed,
    kit::{self, TabLabel, fid},
    state::{ExtensionsUi, ManagerTab, Status},
};
use crate::ui_state::StartPage;
use egui::{Align, FontFamily, Layout, Pos2, Rect, RichText, Sense, Ui, UiBuilder, pos2, vec2};
use onecad_widgets::{
    components::{
        button::{Button, ButtonKind},
        inputs::{SearchField, Select},
    },
    fonts,
    theme::look,
};

const CARD_MIN_W: f32 = 280.0;
const CARD_H: f32 = 170.0;
const GAP: f32 = 16.0;

pub fn show(ui: &mut Ui, x: &mut ExtensionsUi) -> Option<StartPage> {
    let mut nav = None;
    header(ui, x);
    ui.add_space(18.0);
    tabs(ui, x);
    ui.add_space(16.0);
    match x.tab {
        ManagerTab::Discover => nav = discover(ui, x),
        ManagerTab::Installed => installed::list(ui, x, false),
        ManagerTab::Updates => installed::list(ui, x, true),
    }
    if x.developer && x.tab == ManagerTab::Installed {
        ui.add_space(24.0);
        developer::section(ui, x);
    }
    ui.add_space(24.0);
    nav
}

fn header(ui: &mut Ui, x: &mut ExtensionsUi) {
    let th = look(ui.ctx()).theme;
    let (installed, active, updates) = x.counts();
    let meta = format!(
        "{installed} installed · {active} active · {updates} update{}",
        if updates == 1 { "" } else { "s" }
    );
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 3.0;
            let title = RichText::new("Extensions")
                .size(20.0)
                .family(fonts::semibold())
                .color(th.ink);
            ui.add(egui::Label::new(title).selectable(false));
            ui.add(
                egui::Label::new(RichText::new(meta).size(12.5).color(th.ink5)).selectable(false),
            );
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let out = Select::new("ext-source", SOURCES[x.source], &SOURCES)
                .width(118.0)
                .show(ui);
            if let Some(i) = out.chosen {
                x.source = i;
            }
            ui.add_space(8.0);
            let width = (ui.available_width() - 8.0).clamp(160.0, 282.0);
            ui.add(
                SearchField::new("ext-search", &mut x.search)
                    .placeholder("Search extensions")
                    .width(width),
            );
        });
    });
}

fn tabs(ui: &mut Ui, x: &mut ExtensionsUi) {
    let (installed, _, updates) = x.counts();
    let items = [
        TabLabel {
            label: "Discover",
            count: None,
            accent_badge: false,
        },
        TabLabel {
            label: "Installed",
            count: Some(installed),
            accent_badge: false,
        },
        TabLabel {
            label: "Updates",
            count: (updates > 0).then_some(updates),
            accent_badge: true,
        },
    ];
    let selected = x.tab as usize;
    if let Some(i) = kit::tabs(ui, "ext-manager-tabs", &items, selected) {
        x.tab = [
            ManagerTab::Discover,
            ManagerTab::Installed,
            ManagerTab::Updates,
        ][i];
    }
}

fn discover(ui: &mut Ui, x: &mut ExtensionsUi) -> Option<StartPage> {
    chips(ui, x);
    ui.add_space(16.0);
    let visible = x.visible();
    let mut nav = None;
    if visible.is_empty() {
        let c = look(ui.ctx()).theme.ink5;
        ui.add(
            egui::Label::new(
                RichText::new("No extensions match your search.")
                    .size(13.0)
                    .color(c),
            )
            .selectable(false),
        );
    } else {
        nav = grid(ui, x, &visible);
    }
    ui.add_space(16.0);
    hint(ui, x);
    nav
}

fn chips(ui: &mut Ui, x: &mut ExtensionsUi) {
    let th = look(ui.ctx()).theme;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
        for (i, (label, _)) in CATEGORY_CHIPS.iter().enumerate() {
            let on = x.category == i;
            let family = if on {
                fonts::medium()
            } else {
                FontFamily::Proportional
            };
            let color = if on { th.on_accent } else { th.ink3 };
            let g = ui
                .painter()
                .layout_no_wrap((*label).to_owned(), fid(12.5, family), color);
            let (rect, resp) =
                ui.allocate_exact_size(vec2(g.size().x + 24.0, 28.0), Sense::click());
            let (fill, border) = match (on, resp.hovered()) {
                (true, _) => (th.ink, th.ink),
                (false, true) => (th.hover, th.border_strong),
                (false, false) => (th.surface, th.border_strong),
            };
            kit::bordered(ui.painter(), rect, 6, fill, border);
            ui.painter().galley(
                pos2(rect.left() + 12.0, rect.center().y - g.size().y / 2.0),
                g,
                color,
            );
            if resp
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                x.category = i;
            }
        }
    });
}

fn columns(width: f32) -> usize {
    (((width + GAP) / (CARD_MIN_W + GAP)).floor() as usize).clamp(1, 3)
}

fn grid(ui: &mut Ui, x: &mut ExtensionsUi, visible: &[usize]) -> Option<StartPage> {
    let cols = columns(ui.available_width());
    let w = (ui.available_width() - GAP * (cols - 1) as f32) / cols as f32;
    let rows = visible.len().div_ceil(cols);
    let height = rows as f32 * CARD_H + rows.saturating_sub(1) as f32 * GAP;
    let (area, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let mut nav = None;
    for (n, &i) in visible.iter().enumerate() {
        let (col, row) = (n % cols, n / cols);
        let min = area.min + vec2(col as f32 * (w + GAP), row as f32 * (CARD_H + GAP));
        let rect = Rect::from_min_size(min, vec2(w, CARD_H));
        if card(ui, x, &CATALOG[i], rect) {
            x.detail = CATALOG[i].id;
            x.detail_tab = Default::default();
            nav = Some(StartPage::ExtensionDetail);
        }
    }
    nav
}

/// Paints one card; returns true when its body was clicked.
fn card(ui: &mut Ui, x: &mut ExtensionsUi, ext: &Ext, rect: Rect) -> bool {
    let l = look(ui.ctx());
    let th = &l.theme;
    let body = ui.interact(
        rect,
        ui.scope_id().with(("ext-card", ext.id)),
        Sense::click(),
    );
    let hovered = ui.rect_contains_pointer(rect);
    let border = if x.detail == ext.id {
        th.sel_border
    } else if hovered {
        th.card_hover_border
    } else {
        th.border
    };
    let p = ui.painter().clone();
    kit::bordered(&p, rect, 10, th.surface, border);
    let well = Rect::from_min_size(rect.min + vec2(16.0, 16.0), vec2(44.0, 44.0));
    kit::well(&p, well, ext.icon, 10, 24.0, th, l.dark);
    let tx = well.right() + 12.0;
    kit::text(
        &p,
        pos2(tx, rect.top() + 18.0),
        ext.name,
        14.0,
        fonts::semibold(),
        th.ink,
    );
    kit::publisher_line(
        &p,
        tx,
        rect.top() + 49.0,
        ext.publisher,
        ext.trust,
        None,
        &l,
    );
    kit::paragraph(
        &p,
        pos2(rect.left() + 16.0, rect.top() + 72.0),
        ext.desc,
        13.0,
        19.0,
        rect.width() - 32.0,
        th.ink3,
    );
    footer(ui, x, ext, rect);
    body.on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

fn footer(ui: &mut Ui, x: &mut ExtensionsUi, ext: &Ext, card: Rect) {
    let th = look(ui.ctx()).theme;
    let action = Rect::from_min_max(
        pos2(card.right() - 136.0, card.bottom() - 46.0),
        pos2(card.right() - 16.0, card.bottom() - 16.0),
    );
    let meta_w = card.width() - 32.0 - action.width() - 4.0;
    let p = ui.painter().clone();
    let job = kit::mixed_job(&[(ext.meta, true)], 11.5, th.ink5, meta_w);
    let g = p.layout_job(job);
    p.galley(
        Pos2::new(card.left() + 16.0, action.center().y - g.size().y / 2.0),
        g,
        th.ink5,
    );
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(action)
            .layout(Layout::right_to_left(Align::Center)),
    );
    match x.status(ext.id) {
        Status::Installed => {
            let c = th.ok_text;
            child.add(
                egui::Label::new(RichText::new("Installed").size(12.5).color(c)).selectable(false),
            );
            child.add_space(2.0);
            let (r, _) = child.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
            kit::icon_at(
                &p,
                r.center(),
                12.0,
                "check",
                c,
                th.surface,
                look(ui.ctx()).dark,
            );
        }
        Status::Update(v) => {
            if kit::soft_button(&mut child, &format!("Update {v}")).clicked() {
                x.request_update(ext.id);
            }
        }
        Status::Available => {
            if child
                .add(Button::new("Install").kind(ButtonKind::Secondary))
                .clicked()
            {
                x.request_install(ext.id);
            }
        }
    }
}

fn hint(ui: &mut Ui, x: &mut ExtensionsUi) {
    let l = look(ui.ctx());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (r, _) = ui.allocate_exact_size(vec2(14.0, 20.0), Sense::hover());
        kit::icon_at(
            ui.painter(),
            r.center(),
            14.0,
            "help",
            l.theme.ink5,
            l.theme.surface,
            l.dark,
        );
        let muted = |s: &str| {
            egui::Label::new(RichText::new(s.to_owned()).size(12.0).color(l.theme.ink5))
                .selectable(false)
        };
        ui.add(muted(
            "All extensions are free. Sources: OneCAD catalogue, Workshop registry.",
        ));
        if kit::link(ui, "Developer mode", l.theme.accent, 12.0).clicked() {
            x.developer = !x.developer;
        }
        ui.add(muted("adds “Load from folder”."));
    });
}
