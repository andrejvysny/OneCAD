//! ReferenceChip, AddChip and OptionIconGroup (c-ref).

use egui::{CornerRadius, Rect, Response, Sense, Ui, Vec2, Widget, pos2, vec2};

use crate::components::actions::common::{focus_ring, galley, inset};
use crate::components::tooltip::{self, Tip};
use crate::glyph;
use crate::icon::{self, IconStyle};
use crate::metrics::{H_CHIP, RADIUS_SM, SECONDARY};
use crate::theme::look;

pub struct ChipOutput {
    pub response: Response,
    pub remove: bool,
}

/// Selected-reference chip: sel colours, optional leading icon and remove button.
pub struct ReferenceChip<'a> {
    label: &'a str,
    icon: Option<&'a str>,
    removable: bool,
}

impl<'a> ReferenceChip<'a> {
    pub fn new(label: &'a str) -> Self {
        Self {
            label,
            icon: None,
            removable: false,
        }
    }
    pub fn icon(mut self, icon: Option<&'a str>) -> Self {
        self.icon = icon;
        self
    }
    pub fn removable(mut self, removable: bool) -> Self {
        self.removable = removable;
        self
    }

    pub fn show(self, ui: &mut Ui) -> ChipOutput {
        let l = look(ui.ctx());
        let t = &l.theme;
        let g = galley(
            ui.painter(),
            self.label,
            SECONDARY,
            egui::FontFamily::Proportional,
        );
        let icon_w = self.icon.map_or(0.0, |_| 15.0 + 6.0);
        let close_w = if self.removable { 6.0 + 11.0 } else { 0.0 };
        let width = 8.0 + icon_w + g.size().x + close_w + 8.0;
        let (rect, response) = ui.allocate_exact_size(vec2(width, H_CHIP), Sense::hover());
        let close = Rect::from_center_size(
            pos2(rect.right() - 8.0 - 5.5, rect.center().y),
            Vec2::splat(18.0),
        );
        let close_resp = self
            .removable
            .then(|| ui.interact(close, response.id.with("remove"), Sense::click()));
        let p = ui.painter();
        p.rect_filled(rect, CornerRadius::same(RADIUS_SM), t.sel_soft);
        inset(p, rect, RADIUS_SM, t.sel_border);
        let cy = rect.center().y;
        let mut x = rect.left() + 8.0;
        if let Some(name) = self.icon {
            let r = Rect::from_center_size(pos2(x + 7.5, cy), Vec2::splat(15.0));
            icon::paint(
                p,
                r,
                name,
                l.dark,
                IconStyle {
                    tint: t.sel_text,
                    bg: t.sel_soft,
                    grey: false,
                },
                1.0,
            );
            x += icon_w;
        }
        p.galley(pos2(x, cy - g.size().y / 2.0), g, t.sel_text);
        let mut remove = false;
        if let Some(c) = &close_resp {
            let tint = if c.hovered() { t.sel_text } else { t.ink5 };
            glyph::close(p, close.center(), 10.0, tint);
            c.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Remove"));
            remove = c.clicked();
        }
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, self.label));
        ChipOutput { response, remove }
    }
}

/// 28px square "+" chip that adds another reference.
pub struct AddChip;

impl AddChip {
    pub fn new() -> Self {
        Self
    }
}

impl Default for AddChip {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for AddChip {
    fn ui(self, ui: &mut Ui) -> Response {
        let l = look(ui.ctx());
        let t = &l.theme;
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(H_CHIP), Sense::click());
        let fill = if response.hovered() { t.hover2 } else { t.chip };
        let p = ui.painter();
        p.rect_filled(rect, CornerRadius::same(RADIUS_SM), fill);
        inset(p, rect, RADIUS_SM, t.border_strong);
        if response.has_focus() {
            focus_ring(p, t, rect, RADIUS_SM);
        }
        glyph::plus(p, rect.center(), 16.0, t.ink4);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Add"));
        response
    }
}

/// Row of 40x32 icon options (operation: join, cut, intersect, new body).
pub struct OptionIconGroup<'a> {
    selected: &'a mut usize,
    options: &'a [(&'a str, Tip<'a>)],
}

impl<'a> OptionIconGroup<'a> {
    pub fn new(selected: &'a mut usize, options: &'a [(&'a str, Tip<'a>)]) -> Self {
        Self { selected, options }
    }
}

impl Widget for OptionIconGroup<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let l = look(ui.ctx());
        let t = &l.theme;
        let mut changed = false;
        let mut group = ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for (i, (name, tip)) in self.options.iter().enumerate() {
                let (rect, r) = ui.allocate_exact_size(vec2(40.0, 32.0), Sense::click());
                let on = *self.selected == i;
                let fill = match (on, r.hovered()) {
                    (true, _) => t.sel_bg,
                    (false, true) => t.hover2,
                    (false, false) => t.chip,
                };
                let p = ui.painter();
                p.rect_filled(rect, CornerRadius::same(RADIUS_SM), fill);
                if on {
                    inset(p, rect, RADIUS_SM, t.sel_border);
                }
                if r.has_focus() {
                    focus_ring(p, t, rect, RADIUS_SM);
                }
                let tint = if on { t.sel_text } else { t.ink3 };
                let ic = Rect::from_center_size(rect.center(), Vec2::splat(22.0));
                icon::paint(
                    p,
                    ic,
                    name,
                    l.dark,
                    IconStyle {
                        tint,
                        bg: fill,
                        grey: false,
                    },
                    1.0,
                );
                r.widget_info(|| {
                    egui::WidgetInfo::selected(egui::Role::RadioButton, true, on, tip.name)
                });
                if tooltip::attach(r, *tip).clicked() && !on {
                    *self.selected = i;
                    changed = true;
                }
            }
        });
        if changed {
            group.response.mark_changed();
        }
        group.response
    }
}
