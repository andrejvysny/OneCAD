//! Compact workspace menu: 34px rows, 24px icon tile, shortcut + active check, no sections.

use egui::{Color32, CornerRadius, FontId, Pos2, Rect, Response, Sense, Ui, Vec2, pos2, vec2};

use super::workspace::{WorkspaceItem, WorkspaceMenuOutput, divider, focus_ring, hinted};
use crate::fonts;
use crate::glyph;
use crate::icon::{self, IconStyle};
use crate::metrics::RADIUS_SM;
use crate::theme::{Look, look};

/// Row height of the compact menu.
pub const ROW_H: f32 = 34.0;
const TILE: f32 = 24.0;
const ICON: f32 = 16.0;
const PAD: f32 = 6.0;

/// Compact workspace rows for a caller-provided popover: `items`, divider, `extensions`,
/// divider, footer `customize` as `(label, enabled)`. Reports `picked`, `picked_extension`
/// and `customize`. `disabled_hint` is shown when hovering any disabled row.
pub fn workspace_menu_compact(
    ui: &mut Ui,
    items: &[WorkspaceItem],
    extensions: &[WorkspaceItem],
    customize: (&str, bool),
    disabled_hint: Option<&str>,
) -> WorkspaceMenuOutput {
    let mut out = WorkspaceMenuOutput::default();
    ui.spacing_mut().item_spacing.y = 0.0;
    out.picked = rows(ui, items, disabled_hint);
    if !extensions.is_empty() {
        divider(ui);
        out.picked_extension = rows(ui, extensions, disabled_hint);
    }
    divider(ui);
    let r = row(ui, "gear", customize.0, None, customize.1, false, false);
    out.customize = hinted(r, customize.1, disabled_hint).clicked();
    out
}

fn rows(ui: &mut Ui, items: &[WorkspaceItem], hint: Option<&str>) -> Option<usize> {
    let mut picked = None;
    for (i, it) in items.iter().enumerate() {
        let r = row(
            ui,
            it.icon,
            it.label,
            it.shortcut,
            it.enabled,
            it.active,
            true,
        );
        if hinted(r, it.enabled, hint).clicked() {
            picked = Some(i);
        }
    }
    picked
}

fn row(
    ui: &mut Ui,
    icon_name: &str,
    label: &str,
    shortcut: Option<&str>,
    enabled: bool,
    active: bool,
    tile: bool,
) -> Response {
    let l = look(ui.ctx());
    let t = &l.theme;
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), sense);
    let fill = if active {
        t.sel_bg
    } else if response.hovered() && enabled {
        t.hover
    } else {
        Color32::TRANSPARENT
    };
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(RADIUS_SM), fill);
    if response.has_focus() {
        focus_ring(p, rect, t);
    }
    let slot = Rect::from_center_size(
        pos2(rect.left() + PAD + TILE / 2.0, rect.center().y),
        Vec2::splat(TILE),
    );
    paint_icon(p, slot, icon_name, enabled, tile, &l);
    let (color, font) = if active {
        (t.accent, FontId::new(13.0, fonts::semibold()))
    } else if !enabled {
        (t.ink3, FontId::proportional(13.0))
    } else if tile {
        (t.ink, FontId::proportional(13.0))
    } else {
        (t.ink2, FontId::proportional(13.0))
    };
    let g = p.layout_no_wrap(label.to_owned(), font, color);
    let pos = Pos2::new(slot.right() + 10.0, rect.center().y - g.size().y / 2.0);
    p.galley(pos, g, color);
    let mut right = rect.right() - 8.0;
    if let Some(key) = shortcut {
        let g = p.layout_no_wrap(key.to_owned(), FontId::new(11.0, fonts::mono()), t.ink5);
        right -= g.size().x;
        p.galley(
            Pos2::new(right, rect.center().y - g.size().y / 2.0),
            g,
            t.ink5,
        );
    }
    if active {
        glyph::check(p, pos2(right - 8.0 - 7.0, rect.center().y), 14.0, t.accent);
    }
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, enabled, active, label));
    response
}

fn paint_icon(p: &egui::Painter, slot: Rect, name: &str, enabled: bool, tile: bool, l: &Look) {
    let t = &l.theme;
    if tile {
        p.rect_filled(slot, CornerRadius::same(RADIUS_SM), t.sel_soft);
    }
    let style = IconStyle {
        tint: if enabled { t.ink4 } else { t.ink5 },
        bg: if tile { t.sel_soft } else { t.floating },
        grey: false,
    };
    let opacity = if enabled { 1.0 } else { 0.6 };
    let glyph = Rect::from_center_size(slot.center(), Vec2::splat(ICON));
    icon::paint(p, glyph, name, l.dark, style, opacity);
}
