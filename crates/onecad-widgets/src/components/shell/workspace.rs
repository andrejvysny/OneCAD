//! WorkspaceButton + menu (c-ws).

use egui::{
    Color32, CornerRadius, FontId, Painter, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui,
    Vec2, vec2,
};

use crate::components::lists::section_label;

use crate::fonts;
use crate::glyph;
use crate::icon::{self, IconStyle};
use crate::metrics::{self, Elevation, RADIUS_SM, RADIUS_XL};
use crate::theme::{Look, Theme, look};

/// 196×44 button that opens the workspace menu (the host owns the popover).
pub struct WorkspaceButton<'a> {
    name: &'a str,
    subtitle: &'a str,
    icon: &'a str,
    open: bool,
    compact: bool,
}

/// Compact title-bar button: h30, 22px icon well, name, chevron.
const COMPACT_H: f32 = 30.0;

impl<'a> WorkspaceButton<'a> {
    /// Title-bar variant (h30, r7, no subtitle); the host owns the popover.
    pub fn compact(name: &'a str, icon: &'a str) -> Self {
        Self {
            compact: true,
            ..Self::new(name, "", icon)
        }
    }

    /// Width of the compact variant for `name`; lets a parent place it before drawing.
    pub fn compact_width(painter: &Painter, name: &str) -> f32 {
        let g = painter.layout_no_wrap(
            name.to_owned(),
            FontId::new(13.0, fonts::semibold()),
            Color32::PLACEHOLDER,
        );
        4.0 + 22.0 + 7.0 + g.size().x + 6.0 + 12.0 + 6.0
    }

    pub fn new(name: &'a str, subtitle: &'a str, icon: &'a str) -> Self {
        Self {
            name,
            subtitle,
            icon,
            open: false,
            compact: false,
        }
    }
    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }
}

impl egui::Widget for WorkspaceButton<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let l = look(ui.ctx());
        let t = &l.theme;
        if self.compact {
            return self.compact_ui(ui, &l);
        }
        let (rect, response) = ui.allocate_exact_size(vec2(196.0, 44.0), Sense::click());
        let cr = CornerRadius::same(RADIUS_XL);
        let fill = if response.hovered() {
            t.hover
        } else {
            t.floating
        };
        let border = if self.open { t.sel_border } else { t.border };
        let p = ui.painter();
        p.add(metrics::shadow(Elevation::Pill, t, l.dark).as_shape(rect, cr));
        p.rect(rect, cr, fill, Stroke::new(1.0, border), StrokeKind::Inside);
        if response.has_focus() {
            let (w, c) = metrics::focus_ring(t);
            p.rect_stroke(
                rect.expand(w * 0.5),
                CornerRadius::same(RADIUS_XL + 2),
                Stroke::new(w, c),
                StrokeKind::Outside,
            );
        }
        self.paint_content(p, rect, &l);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, self.name));
        response
    }
}

impl WorkspaceButton<'_> {
    fn compact_ui(&self, ui: &mut Ui, l: &Look) -> Response {
        let t = &l.theme;
        let w = Self::compact_width(ui.painter(), self.name);
        let (rect, response) = ui.allocate_exact_size(vec2(w, COMPACT_H), Sense::click());
        let cr = CornerRadius::same(7);
        let (fill, border) = if self.open {
            (t.sel_soft, t.sel_border)
        } else if response.hovered() {
            (t.hover, t.border_strong)
        } else {
            (Color32::TRANSPARENT, t.border_strong)
        };
        let p = ui.painter();
        p.rect(rect, cr, fill, Stroke::new(1.0, border), StrokeKind::Inside);
        if response.has_focus() {
            let (fw, c) = metrics::focus_ring(t);
            p.rect_stroke(
                rect.expand(fw * 0.5),
                CornerRadius::same(9),
                Stroke::new(fw, c),
                StrokeKind::Outside,
            );
        }
        let well = Rect::from_min_size(
            Pos2::new(rect.left() + 4.0, rect.center().y - 11.0),
            Vec2::splat(22.0),
        );
        p.rect_filled(well, CornerRadius::same(5), t.sel_soft);
        let style = IconStyle {
            tint: t.ink4,
            bg: t.sel_soft,
            grey: false,
        };
        icon::paint(
            p,
            Rect::from_center_size(well.center(), Vec2::splat(18.0)),
            self.icon,
            l.dark,
            style,
            1.0,
        );
        let name = p.layout_no_wrap(
            self.name.to_owned(),
            FontId::new(13.0, fonts::semibold()),
            t.titlebar_text,
        );
        let pos = Pos2::new(well.right() + 7.0, rect.center().y - name.size().y / 2.0);
        p.galley(pos, name, t.titlebar_text);
        let chev = Pos2::new(rect.right() - 6.0 - 6.0, rect.center().y);
        glyph::chevron_down(p, chev, 14.0, t.ink4);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, self.name));
        response
    }

    /// Icon well, name + subtitle block and trailing chevron.
    fn paint_content(&self, p: &Painter, rect: Rect, l: &Look) {
        let t = &l.theme;
        let well = Rect::from_min_size(
            Pos2::new(rect.left() + 6.0, rect.center().y - 16.0),
            Vec2::splat(32.0),
        );
        p.rect_filled(well, CornerRadius::same(RADIUS_SM), t.sel_soft);
        let glyph = Rect::from_center_size(well.center(), Vec2::splat(22.0));
        let style = IconStyle {
            tint: t.ink4,
            bg: t.sel_soft,
            grey: false,
        };
        icon::paint(p, glyph, self.icon, l.dark, style, 1.0);
        let name = p.layout_no_wrap(
            self.name.to_owned(),
            FontId::new(13.0, fonts::semibold()),
            t.ink,
        );
        let sub = p.layout_no_wrap(self.subtitle.to_owned(), FontId::proportional(11.0), t.ink5);
        let x = well.right() + 10.0;
        let block = name.size().y + 1.0 + sub.size().y;
        let top = rect.center().y - block / 2.0;
        let name_h = name.size().y;
        p.galley(Pos2::new(x, top), name, t.ink);
        p.galley(Pos2::new(x, top + name_h + 1.0), sub, t.ink5);
        let chev = Rect::from_center_size(
            Pos2::new(rect.right() - 12.0 - 6.0, rect.center().y),
            Vec2::splat(12.0),
        );
        glyph::chevron_down(p, chev.center(), 14.0, t.ink4);
    }
}

pub struct WorkspaceItem<'a> {
    pub label: &'a str,
    pub icon: &'a str,
    pub shortcut: Option<&'a str>,
    /// Second, smaller line under the label.
    pub subtitle: Option<&'a str>,
    pub enabled: bool,
    pub active: bool,
}

#[derive(Default)]
pub struct WorkspaceMenuOutput {
    /// Index into `items`.
    pub picked: Option<usize>,
    pub picked_extension: Option<usize>,
    pub customize: bool,
    pub manage: bool,
}

/// Menu rows (30, r6) for a caller-provided popover.
pub fn workspace_menu(
    ui: &mut Ui,
    items: &[WorkspaceItem],
    extensions: &[WorkspaceItem],
) -> WorkspaceMenuOutput {
    workspace_menu_with(
        ui,
        items,
        extensions,
        ("Customize workspace…", true),
        ("Manage extensions…", true),
        None,
    )
}

/// Like [`workspace_menu`] with caller-provided footer rows as `(label, enabled)`:
/// `customize` (reported as `customize`) and `last` (reported as `manage`).
/// `disabled_hint` is shown when hovering any disabled row.
pub fn workspace_menu_with(
    ui: &mut Ui,
    items: &[WorkspaceItem],
    extensions: &[WorkspaceItem],
    customize: (&str, bool),
    last: (&str, bool),
    disabled_hint: Option<&str>,
) -> WorkspaceMenuOutput {
    let mut out = WorkspaceMenuOutput::default();
    ui.set_min_width(208.0);
    ui.spacing_mut().item_spacing.y = 0.0;
    section_label(ui, "Workspaces", 8.0);
    out.picked = item_rows(ui, items, disabled_hint);
    if !extensions.is_empty() {
        section_label(ui, "From extensions", 8.0);
        out.picked_extension = item_rows(ui, extensions, disabled_hint);
    }
    divider(ui);
    let r = row(
        ui,
        Some("gear"),
        customize.0,
        None,
        None,
        customize.1,
        false,
    );
    out.customize = hinted(r, customize.1, disabled_hint).clicked();
    let r = row(ui, Some("extension"), last.0, None, None, last.1, false);
    out.manage = hinted(r, last.1, disabled_hint).clicked();
    out
}

pub(super) fn hinted(r: Response, enabled: bool, hint: Option<&str>) -> Response {
    match hint {
        Some(h) if !enabled => r.on_hover_text(h),
        _ => r,
    }
}

/// Index of the clicked item, if any.
fn item_rows(ui: &mut Ui, items: &[WorkspaceItem], hint: Option<&str>) -> Option<usize> {
    let mut picked = None;
    for (i, it) in items.iter().enumerate() {
        let r = row(
            ui,
            Some(it.icon),
            it.label,
            it.shortcut,
            it.subtitle,
            it.enabled,
            it.active,
        );
        if hinted(r, it.enabled, hint).clicked() {
            picked = Some(i);
        }
    }
    picked
}

pub(super) fn divider(ui: &mut Ui) {
    let t = look(ui.ctx()).theme;
    ui.add_space(4.0);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(r, 0.0, t.border_subtle);
    ui.add_space(4.0);
}

fn row(
    ui: &mut Ui,
    icon_name: Option<&str>,
    label: &str,
    shortcut: Option<&str>,
    subtitle: Option<&str>,
    enabled: bool,
    active: bool,
) -> Response {
    let l = look(ui.ctx());
    let t = &l.theme;
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(
        vec2(
            ui.available_width(),
            if subtitle.is_some() { 46.0 } else { 30.0 },
        ),
        sense,
    );
    let opacity = if enabled { 1.0 } else { 0.4 };
    let fill = if active {
        t.sel_bg
    } else if response.hovered() && enabled {
        t.hover
    } else {
        Color32::TRANSPARENT
    };
    let text = if active {
        t.sel_text
    } else if subtitle.is_some() {
        t.ink
    } else {
        t.ink2
    };
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(RADIUS_SM), fill);
    if response.has_focus() {
        focus_ring(p, rect, t);
    }
    let advance = match (icon_name, subtitle) {
        (Some(name), Some(_)) => row_tile(p, rect, name, enabled, &l),
        _ => row_icon(p, rect, icon_name, (text, fill, enabled), &l),
    };
    row_text(
        p,
        rect,
        rect.left() + 8.0 + advance,
        (label, subtitle),
        shortcut,
        [text, t.ink5, t.ink6].map(|c| c.gamma_multiply(opacity)),
    );
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, enabled, active, label));
    response
}

pub(super) fn focus_ring(p: &Painter, rect: Rect, t: &Theme) {
    let (w, c) = metrics::focus_ring(t);
    p.rect_stroke(
        rect.expand(w * 0.5),
        CornerRadius::same(RADIUS_SM + 2),
        Stroke::new(w, c),
        StrokeKind::Outside,
    );
}

/// Paints the optional leading icon; returns the x advance it consumed.
fn row_icon(
    p: &Painter,
    rect: Rect,
    name: Option<&str>,
    st: (Color32, Color32, bool),
    l: &Look,
) -> f32 {
    let Some(name) = name else {
        return 0.0;
    };
    let (tint, fill, enabled) = st;
    let r = Rect::from_center_size(
        Pos2::new(rect.left() + 8.0 + 10.0, rect.center().y),
        Vec2::splat(20.0),
    );
    let bg = if fill.a() == 255 {
        fill
    } else {
        l.theme.floating
    };
    let style = IconStyle {
        tint,
        bg,
        grey: !enabled,
    };
    icon::paint(p, r, name, l.dark, style, if enabled { 1.0 } else { 0.4 });
    30.0
}

/// 32px icon tile (r6, sel_soft, border_subtle); returns the x advance it consumed.
fn row_tile(p: &Painter, rect: Rect, name: &str, enabled: bool, l: &Look) -> f32 {
    let t = &l.theme;
    let opacity = if enabled { 1.0 } else { 0.4 };
    let tile = Rect::from_min_size(
        Pos2::new(rect.left() + 7.0, rect.center().y - 16.0),
        Vec2::splat(32.0),
    );
    p.rect(
        tile,
        CornerRadius::same(RADIUS_SM),
        t.sel_soft.gamma_multiply(opacity),
        Stroke::new(1.0, t.border_subtle.gamma_multiply(opacity)),
        StrokeKind::Inside,
    );
    let style = IconStyle {
        tint: t.ink4,
        bg: t.sel_soft,
        grey: !enabled,
    };
    let glyph = Rect::from_center_size(tile.center(), Vec2::splat(20.0));
    icon::paint(p, glyph, name, l.dark, style, opacity);
    41.0
}

/// Label at `x` and right-aligned mono shortcut; `[label, subtitle, shortcut]` colours.
fn row_text(
    p: &Painter,
    rect: Rect,
    x: f32,
    (label, subtitle): (&str, Option<&str>),
    key: Option<&str>,
    [c, sc, kc]: [Color32; 3],
) {
    let font = if subtitle.is_some() {
        FontId::new(13.0, fonts::semibold())
    } else {
        FontId::proportional(13.0)
    };
    let g = p.layout_no_wrap(label.to_owned(), font, c);
    let sub = subtitle.map(|s| p.layout_no_wrap(s.to_owned(), FontId::proportional(11.0), sc));
    let block = g.size().y + sub.as_ref().map_or(0.0, |s| 1.0 + s.size().y);
    let top = rect.center().y - block / 2.0;
    let gh = g.size().y;
    p.galley(Pos2::new(x, top), g, c);
    if let Some(sub) = sub {
        p.galley(Pos2::new(x, top + gh + 1.0), sub, sc);
    }
    if let Some(key) = key {
        let g = p.layout_no_wrap(key.to_owned(), FontId::new(11.0, fonts::mono()), kc);
        let pos = Pos2::new(
            rect.right() - 8.0 - g.size().x,
            rect.center().y - g.size().y / 2.0,
        );
        p.galley(pos, g, kc);
    }
}
