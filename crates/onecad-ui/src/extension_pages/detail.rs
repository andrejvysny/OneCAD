//! Extension detail page (17a): header, tabs and the Overview body.
use super::{
    catalog::{self, Ext},
    detail_tabs, kit,
    kit::{TabLabel, fid},
    state::{DetailTab, ExtensionsUi, Status},
};
use crate::ui_state::StartPage;
use egui::{Align, Align2, Layout, Rect, RichText, Sense, Ui, UiBuilder, pos2, vec2};
use onecad_widgets::{
    components::{
        button::{Button, ButtonKind},
        inputs::Toggle,
        lists::{MenuRow, menu_frame},
    },
    fonts,
    theme::look,
};

const SIDE_W: f32 = 300.0;

pub fn show(ui: &mut Ui, x: &mut ExtensionsUi) -> Option<StartPage> {
    let Some(ext) = catalog::find(x.detail) else {
        return Some(StartPage::Extensions);
    };
    let mut nav = breadcrumb(ui, ext);
    ui.add_space(14.0);
    if header(ui, x, ext) {
        nav = Some(StartPage::Extensions);
    }
    ui.add_space(18.0);
    let items = [
        TabLabel {
            label: "Overview",
            count: None,
            accent_badge: false,
        },
        TabLabel {
            label: "Settings",
            count: None,
            accent_badge: false,
        },
        TabLabel {
            label: "Changelog",
            count: None,
            accent_badge: false,
        },
    ];
    if let Some(i) = kit::tabs(ui, "ext-detail-tabs", &items, x.detail_tab as usize) {
        x.detail_tab = [
            DetailTab::Overview,
            DetailTab::Settings,
            DetailTab::Changelog,
        ][i];
    }
    ui.add_space(18.0);
    match x.detail_tab {
        DetailTab::Overview => overview(ui, ext),
        DetailTab::Settings => detail_tabs::settings(ui, x, ext),
        DetailTab::Changelog => detail_tabs::changelog(ui, ext),
    }
    ui.add_space(24.0);
    nav
}

fn breadcrumb(ui: &mut Ui, ext: &Ext) -> Option<StartPage> {
    let th = look(ui.ctx()).theme;
    let mut nav = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if kit::link(ui, "Extensions", th.accent, 12.5).clicked() {
            nav = Some(StartPage::Extensions);
        }
        ui.add(egui::Label::new(RichText::new("/").size(12.5).color(th.ink5)).selectable(false));
        ui.add(
            egui::Label::new(RichText::new(ext.name).size(12.5).color(th.ink)).selectable(false),
        );
    });
    nav
}

/// Returns true when the overflow menu uninstalled the extension.
fn header(ui: &mut Ui, x: &mut ExtensionsUi, ext: &Ext) -> bool {
    let l = look(ui.ctx());
    let th = &l.theme;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 66.0), Sense::hover());
    let p = ui.painter().clone();
    let well = Rect::from_min_size(rect.min, vec2(64.0, 64.0));
    kit::well(&p, well, ext.icon, 12, 36.0, th, l.dark);
    let tx = well.right() + 16.0;
    kit::text(
        &p,
        pos2(tx, rect.top() + 8.0),
        ext.name,
        20.0,
        fonts::semibold(),
        th.ink,
    );
    let version = format!("v{}", x.row(ext.id).map_or(ext.version, |r| r.version));
    kit::publisher_line(
        &p,
        tx,
        rect.top() + 46.0,
        ext.publisher,
        ext.trust,
        Some(&version),
        &l,
    );
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(Rect::from_min_max(
                pos2(rect.right() - 260.0, rect.top()),
                pos2(rect.right(), rect.top() + 64.0),
            ))
            .layout(Layout::right_to_left(Align::Center)),
    );
    controls(&mut child, x, ext)
}

fn controls(ui: &mut Ui, x: &mut ExtensionsUi, ext: &Ext) -> bool {
    let th = look(ui.ctx()).theme;
    let mut removed = false;
    let more = square_button(ui);
    egui::Popup::menu(&more)
        .frame(menu_frame(ui.ctx()))
        .width(170.0)
        .gap(4.0)
        .show(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.add(MenuRow::new(None, "Check for updates"));
            ui.add(MenuRow::new(None, "View source"));
            if ui.add(MenuRow::new(None, "Uninstall")).clicked() {
                removed = true;
            }
        });
    ui.add_space(8.0);
    if x.status(ext.id) == Status::Available {
        if ui
            .add(Button::new("Install").kind(ButtonKind::Primary))
            .clicked()
        {
            x.request_install(ext.id);
        }
    } else if let Some(row) = x.rows.iter_mut().find(|r| r.id == ext.id) {
        ui.add(Toggle::new(&mut row.enabled));
        ui.add_space(6.0);
        ui.add(
            egui::Label::new(RichText::new("Enabled").size(13.0).color(th.ink3)).selectable(false),
        );
    }
    if removed {
        x.uninstall(ext.id);
        x.tab = Default::default();
    }
    removed
}

fn square_button(ui: &mut Ui) -> egui::Response {
    let l = look(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::click());
    let fill = if resp.hovered() {
        l.theme.hover
    } else {
        l.theme.surface
    };
    kit::bordered(ui.painter(), rect, 8, fill, l.theme.border_strong);
    kit::icon_at(
        ui.painter(),
        rect.center(),
        16.0,
        "overflow",
        l.theme.ink3,
        fill,
        l.dark,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn overview(ui: &mut Ui, ext: &Ext) {
    let total = ui.available_width();
    let left_w = (total - SIDE_W - 32.0).max(320.0);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.allocate_ui_with_layout(vec2(left_w, 0.0), Layout::top_down(Align::Min), |ui| {
            ui.set_width(left_w);
            left_column(ui, ext);
        });
        ui.add_space(32.0);
        ui.allocate_ui_with_layout(vec2(SIDE_W, 0.0), Layout::top_down(Align::Min), |ui| {
            ui.set_width(SIDE_W);
            super::detail_side::column(ui, ext);
        });
    });
}

fn left_column(ui: &mut Ui, ext: &Ext) {
    let th = look(ui.ctx()).theme;
    shots(ui, ext);
    ui.add_space(16.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::hover());
    let size = kit::paragraph(
        ui.painter(),
        rect.min,
        ext.long_desc,
        13.0,
        21.0,
        rect.width(),
        th.ink3,
    );
    ui.allocate_space(vec2(0.0, (size.y - 44.0).max(0.0)));
    ui.add_space(12.0);
    kit::caption(ui, "What it adds · and where");
    ui.add_space(6.0);
    adds_table(ui, ext);
}

fn shots(ui: &mut Ui, ext: &Ext) {
    let l = look(ui.ctx());
    let th = &l.theme;
    let (area, _) = ui.allocate_exact_size(vec2(ui.available_width(), 222.0), Sense::hover());
    let big_w = ((area.width() - 12.0) * 410.0 / 614.0).round();
    let small_w = area.width() - 12.0 - big_w;
    let big = Rect::from_min_size(area.min, vec2(big_w, 222.0));
    let a = Rect::from_min_size(pos2(big.right() + 12.0, area.top()), vec2(small_w, 105.0));
    let b = Rect::from_min_size(pos2(a.left(), a.bottom() + 12.0), vec2(small_w, 105.0));
    let first = format!("screenshot · {} workspace", ext.name);
    let labels = [
        (big, first.as_str()),
        (a, "dogbone on inside corners"),
        (b, "tool reach layer"),
    ];
    for (rect, label) in labels {
        kit::bordered(ui.painter(), rect, 8, th.surface, th.border);
        kit::hatch(ui.painter(), rect.shrink(1.0), th.ink.gamma_multiply(0.035));
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            label,
            fid(11.0, fonts::mono()),
            th.ink6,
        );
    }
}

fn adds_table(ui: &mut Ui, ext: &Ext) {
    let l = look(ui.ctx());
    let th = &l.theme;
    let w = ui.available_width();
    let h = 39.0 * ext.adds.len() as f32;
    let (rect, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
    let p = ui.painter().clone();
    kit::bordered(&p, rect, 8, th.surface, th.border);
    for (i, a) in ext.adds.iter().enumerate() {
        let row = Rect::from_min_size(
            pos2(rect.left(), rect.top() + 39.0 * i as f32),
            vec2(w, 39.0),
        );
        if i > 0 {
            p.hline(
                row.x_range(),
                row.top() - 0.5,
                egui::Stroke::new(1.0, th.border_subtle),
            );
        }
        kit::icon_at(
            &p,
            pos2(row.left() + 22.0, row.center().y),
            16.0,
            a.icon,
            th.ink3,
            th.surface,
            l.dark,
        );
        kit::text(
            &p,
            pos2(row.left() + 40.0, row.center().y - 8.0),
            a.name,
            13.0,
            fonts::medium(),
            th.ink,
        );
        let desc_x = row.left() + w * 0.35;
        kit::paragraph_centered(
            &p,
            pos2(desc_x, row.center().y),
            a.desc,
            12.0,
            14.0,
            (w * 0.30).max(120.0),
            th.ink4,
        );
        kit::text_right(
            &p,
            row.right() - 12.0,
            row.center().y,
            a.location,
            12.0,
            th.ink5,
        );
    }
}
