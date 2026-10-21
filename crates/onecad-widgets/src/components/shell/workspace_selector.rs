//! WorkspaceSelector: explorer-header workspace switcher (full) and its collapsed-chip tile.

use egui::{
    Color32, CornerRadius, FontId, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, vec2,
};

use crate::fonts;
use crate::glyph;
use crate::icon::{self, IconStyle};
use crate::theme::look;

const TILE: f32 = 28.0;
const HEIGHT: f32 = 36.0;

/// Workspace switcher; the host owns the popover. Full: fills the row, tile + name +
/// chevron (h36, r8). Compact: tile + chevron only (`COMPACT_W` x `COMPACT_H`).
pub struct WorkspaceSelector<'a> {
    name: &'a str,
    icon: &'a str,
    open: bool,
    compact: bool,
}

impl<'a> WorkspaceSelector<'a> {
    pub const COMPACT_W: f32 = 44.0;
    pub const COMPACT_H: f32 = 32.0;

    pub fn new(name: &'a str, icon: &'a str) -> Self {
        Self {
            name,
            icon,
            open: false,
            compact: false,
        }
    }
    /// Menu is showing: active fill and flipped chevron.
    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }
    pub fn compact(mut self, compact: bool) -> Self {
        self.compact = compact;
        self
    }
}

impl egui::Widget for WorkspaceSelector<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let l = look(ui.ctx());
        let t = &l.theme;
        let size = if self.compact {
            vec2(Self::COMPACT_W, Self::COMPACT_H)
        } else {
            vec2(ui.available_width(), HEIGHT)
        };
        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
        let radius = if self.compact { 7 } else { 8 };
        let fill = if self.open {
            t.sel_soft
        } else if response.hovered() {
            t.hover
        } else {
            Color32::TRANSPARENT
        };
        let p = ui.painter();
        p.rect_filled(rect, CornerRadius::same(radius), fill);
        if response.has_focus() {
            let (w, c) = crate::metrics::focus_ring(t);
            p.rect_stroke(
                rect.expand(w * 0.5),
                CornerRadius::same(radius + 2),
                Stroke::new(w, c),
                StrokeKind::Outside,
            );
        }
        let inset = if self.compact { 3.0 } else { 4.0 };
        let tile = Rect::from_min_size(
            Pos2::new(rect.left() + inset, rect.center().y - TILE / 2.0),
            Vec2::splat(TILE),
        );
        p.rect(
            tile,
            CornerRadius::same(6),
            t.surface,
            Stroke::new(1.0, t.border_subtle),
            StrokeKind::Inside,
        );
        let style = IconStyle {
            tint: t.ink4,
            bg: t.surface,
            grey: false,
        };
        let glyph_box = Rect::from_center_size(tile.center(), Vec2::splat(18.0));
        icon::paint(p, glyph_box, self.icon, l.dark, style, 1.0);
        let chev = if self.compact {
            Pos2::new(rect.right() - 7.0, rect.center().y)
        } else {
            Pos2::new(rect.right() - 14.0, rect.center().y)
        };
        if self.open {
            glyph::chevron_up(p, chev, 14.0, t.ink4);
        } else {
            glyph::chevron_down(p, chev, 14.0, t.ink4);
        }
        if !self.compact {
            let name = p.layout_no_wrap(
                self.name.to_owned(),
                FontId::new(14.0, fonts::semibold()),
                t.ink,
            );
            let pos = Pos2::new(tile.right() + 10.0, rect.center().y - name.size().y / 2.0);
            p.galley(pos, name, t.ink);
        }
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::Role::Button, true, self.open, self.name)
        });
        response
    }
}
