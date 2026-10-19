//! Panels and lists (g04): SectionLabel, TreeRow (c-tree), SegmentedControl (c-tabs),
//! MenuList (c-menu), PropertyRow (c-prop), HistoryRow + RollbackMarker (c-hist).

mod history;
mod menu;
mod segmented;
mod tabs;

pub use history::{HistoryOutput, HistoryRow, HistoryState, RollbackMarker};
pub use menu::{MenuRow, MenuSection, menu_divider, menu_frame};
pub use segmented::{Segment, SegmentedControl};
pub use tabs::{Tab, TabBar};

use egui::text::{LayoutJob, TextFormat};
use egui::{
    Color32, CornerRadius, FontFamily, FontId, Rect, Response, Sense, Stroke, Ui, Widget, pos2,
    vec2,
};

use crate::components::actions::common::{fitted, focus_ring, galley, inset, opacity, sense};
use crate::fonts;
use crate::glyph;
use crate::icon::{self, IconStyle};
use crate::metrics::{BODY, CONTROL, H_ROW, RADIUS_SM, SECTION, SECTION_TRACKING};
use crate::theme::look;

/// Caps label with letter-spacing; `left` is the text inset inside its row.
pub(crate) fn section_label(ui: &mut Ui, text: &str, left: f32) -> Response {
    let l = look(ui.ctx());
    let mut job = LayoutJob::default();
    job.append(
        &text.to_uppercase(),
        0.0,
        TextFormat {
            font_id: FontId::new(SECTION, fonts::medium()),
            color: l.theme.ink5,
            extra_letter_spacing: SECTION_TRACKING,
            ..TextFormat::default()
        },
    );
    let g = ui.painter().layout_job(job);
    let size = vec2(ui.available_width(), 6.0 + g.size().y + 4.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter()
        .galley(pos2(rect.left() + left, rect.top() + 6.0), g, l.theme.ink5);
    response
}

pub struct SectionLabel<'a> {
    text: &'a str,
}

impl<'a> SectionLabel<'a> {
    pub fn new(text: &'a str) -> Self {
        Self { text }
    }
}

impl Widget for SectionLabel<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        section_label(ui, self.text, 10.0)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RowState {
    #[default]
    Normal,
    Selected,
    Editing,
    Reference,
}

pub struct TreeRow<'a> {
    icon: &'a str,
    label: &'a str,
    meta: Option<&'a str>,
    trailing: Option<&'a str>,
    state: RowState,
    child: bool,
    expanded: bool,
    collapsed: bool,
    enabled: bool,
}

impl<'a> TreeRow<'a> {
    pub fn new(icon: &'a str, label: &'a str) -> Self {
        Self {
            icon,
            label,
            meta: None,
            trailing: None,
            state: RowState::Normal,
            child: false,
            expanded: false,
            collapsed: false,
            enabled: true,
        }
    }
    pub fn meta(mut self, meta: Option<&'a str>) -> Self {
        self.meta = meta;
        self
    }
    /// Right-most 15px icon, e.g. a visibility eye.
    pub fn trailing(mut self, icon: Option<&'a str>) -> Self {
        self.trailing = icon;
        self
    }
    pub fn state(mut self, state: RowState) -> Self {
        self.state = state;
        self
    }
    pub fn child(mut self, child: bool) -> Self {
        self.child = child;
        self
    }
    /// Draws the expanded disclosure chevron in the leading 10px slot.
    pub fn expanded(mut self, expanded: bool) -> Self {
        self.expanded = expanded;
        self
    }
    /// Draws the collapsed disclosure chevron (pointing right) in the leading slot.
    pub fn collapsed(mut self, collapsed: bool) -> Self {
        self.collapsed = collapsed;
        self
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

/// Child rows sit under a 1px rule: 28px high, content indented to 38.
const CHILD_H: f32 = 28.0;
const CHILD_RULE_X: f32 = 30.0;
const CHILD_INDENT: f32 = 38.0;

impl Widget for TreeRow<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let l = look(ui.ctx());
        let t = &l.theme;
        let (rect, response) = ui.allocate_exact_size(
            vec2(
                ui.available_width(),
                if self.child { CHILD_H } else { H_ROW },
            ),
            sense(self.enabled, Sense::click()),
        );
        let op = opacity(self.enabled);
        let hovered = response.hovered() && self.enabled;
        let (fill, label_col, meta_col, icon_tint) = match self.state {
            RowState::Normal if hovered => (t.hover2, t.tree_label, t.ink6, t.ink3),
            RowState::Normal => (Color32::TRANSPARENT, t.tree_label, t.ink6, t.ink3),
            RowState::Selected => (
                t.sel_soft,
                t.sel_text,
                t.sel_text.gamma_multiply(0.75),
                t.ink3,
            ),
            RowState::Editing => (
                t.sel_bg,
                t.sel_text,
                t.sel_text.gamma_multiply(0.75),
                t.ink3,
            ),
            RowState::Reference => (Color32::TRANSPARENT, t.ink5, t.ink6, t.ink5),
        };
        let p = ui.painter();
        let r = CornerRadius::same(RADIUS_SM);
        p.rect_filled(rect, r, fill.gamma_multiply(op));
        if self.state == RowState::Selected {
            inset(p, rect, RADIUS_SM, t.sel_border);
        }
        if response.has_focus() {
            focus_ring(p, t, rect, RADIUS_SM);
        }
        let cy = rect.center().y;
        if self.child {
            let x = rect.left() + CHILD_RULE_X;
            p.line_segment(
                [pos2(x, rect.top()), pos2(x, rect.bottom())],
                Stroke::new(1.0, t.border),
            );
        }
        let mut x = rect.left() + if self.child { CHILD_INDENT } else { 10.0 };
        let style = |tint| IconStyle {
            tint,
            bg: fill,
            grey: !self.enabled,
        };
        let chev = pos2(x + 5.0, cy);
        if self.expanded {
            glyph::chevron_down(p, chev, 12.0, t.ink7.gamma_multiply(op));
        } else if self.collapsed {
            glyph::chevron_right(p, chev, 12.0, t.ink7.gamma_multiply(op));
        }
        let (slot, icon_px) = if self.child {
            (0.0, 16.0)
        } else {
            (10.0 + 8.0, 18.0)
        };
        x += slot;
        let ic = Rect::from_center_size(pos2(x + icon_px / 2.0, cy), vec2(icon_px, icon_px));
        icon::paint(p, ic, self.icon, l.dark, style(icon_tint), op);
        x += icon_px + 8.0;
        let mut right = rect.right() - 10.0;
        if let Some(name) = self.trailing {
            let c = Rect::from_center_size(pos2(right - 7.5, cy), vec2(15.0, 15.0));
            if !glyph::by_name(p, name, c.center(), 12.0, t.ink7.gamma_multiply(op)) {
                icon::paint(p, c, name, l.dark, style(t.ink7), op);
            }
            right -= 15.0 + 8.0;
        }
        if let Some(meta) = self.meta {
            let g = galley(p, meta, SECTION, fonts::mono());
            p.galley(
                pos2(right - g.size().x, cy - g.size().y / 2.0),
                g.clone(),
                meta_col.gamma_multiply(op),
            );
            right -= g.size().x + 8.0;
        }
        let family = if self.state == RowState::Editing {
            fonts::medium()
        } else {
            FontFamily::Proportional
        };
        let size = if self.child { CONTROL } else { BODY };
        let g = fitted(
            p,
            self.label,
            FontId::new(size, family),
            Color32::PLACEHOLDER,
            right - x,
        );
        p.galley(
            pos2(x, cy - g.size().y / 2.0),
            g,
            label_col.gamma_multiply(op),
        );
        let selected = matches!(self.state, RowState::Selected | RowState::Editing);
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::Role::TreeItem, self.enabled, selected, self.label)
        });
        response
    }
}

pub struct PropertyRow<'a> {
    label: &'a str,
    value: &'a str,
}

impl<'a> PropertyRow<'a> {
    pub fn new(label: &'a str) -> Self {
        Self { label, value: "" }
    }
    pub fn value(mut self, value: &'a str) -> Self {
        self.value = value;
        self
    }
}

const LABEL_COL: f32 = 96.0;

impl Widget for PropertyRow<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let l = look(ui.ctx());
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
        let p = ui.painter();
        let label_width = LABEL_COL.min(rect.width() * 0.5);
        let g = fitted(
            p,
            self.label,
            FontId::proportional(CONTROL),
            l.theme.ink4,
            label_width,
        );
        p.galley(
            pos2(rect.left(), rect.center().y - g.size().y / 2.0),
            g,
            l.theme.ink4,
        );
        let value_rect = Rect::from_min_max(
            pos2(
                (rect.left() + label_width + 8.0).min(rect.right()),
                rect.top(),
            ),
            rect.max,
        );
        let v = fitted(
            p,
            self.value,
            FontId::new(CONTROL, fonts::mono()),
            l.theme.ink2,
            value_rect.width(),
        );
        let pos = pos2(
            rect.right() - v.size().x,
            rect.center().y - v.size().y / 2.0,
        );
        p.with_clip_rect(value_rect.intersect(p.clip_rect()))
            .galley(pos, v, l.theme.ink2);
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::Role::Label,
                true,
                format!("{}: {}", self.label, self.value),
            )
        });
        response.on_hover_text(self.value)
    }
}

/// Property row whose value is an editable widget placed in the value column.
pub fn property_row_with<R>(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    let l = look(ui.ctx());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let (rect, _) = ui.allocate_exact_size(vec2(LABEL_COL, 30.0), Sense::hover());
        let g = galley(ui.painter(), label, CONTROL, FontFamily::Proportional);
        let pos = pos2(rect.left(), rect.center().y - g.size().y / 2.0);
        ui.painter().galley(pos, g, l.theme.ink4);
        add(ui)
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::actions::common::headless;

    #[test]
    fn long_property_values_stay_in_their_column_at_both_display_scales() {
        const VALUE: &str = "Synthetic constrained 80 x 50 mm plate profile";
        for scale in [1.0, 2.0] {
            let ctx = egui::Context::default();
            crate::theme::install(&ctx, crate::theme::load(false).unwrap(), false);
            ctx.set_pixels_per_point(scale);
            for width in [160.0, 256.0, 280.0] {
                for _ in 0..3 {
                    let mut row = Rect::NOTHING;
                    let mut frame = ctx.run_ui(egui::RawInput::default(), |ui| {
                        ui.set_width(width);
                        row = ui.add(PropertyRow::new("Sketch").value(VALUE)).rect;
                        assert!(ui.min_rect().width() <= width + 1.0);
                    });
                    frame.textures_delta.clear();
                    let mut saw_value = false;
                    for shape in &frame.shapes {
                        if let egui::epaint::Shape::Text(text) = &shape.shape {
                            let bounds = Rect::from_min_size(text.pos, text.galley.size());
                            assert!(bounds.left() >= row.left() - 1.0);
                            assert!(bounds.right() <= row.right() + 1.0);
                            if text.galley.job.text == VALUE {
                                saw_value = true;
                                assert!(text.galley.elided);
                                assert!(
                                    bounds.left() >= row.left() + LABEL_COL.min(width * 0.5) + 7.0
                                );
                                assert!(
                                    shape.clip_rect.left()
                                        >= row.left() + LABEL_COL.min(width * 0.5) + 8.0
                                );
                            }
                        }
                    }
                    assert!(saw_value, "full value retained by text layout");
                }
            }
        }
    }

    #[test]
    fn every_component_renders_in_both_themes() {
        for dark in [false, true] {
            headless(dark, Vec::new(), |ui| {
                ui.add(SectionLabel::new("Bodies"));
                for s in [
                    RowState::Normal,
                    RowState::Selected,
                    RowState::Editing,
                    RowState::Reference,
                ] {
                    ui.add(
                        TreeRow::new("body", "Bracket")
                            .meta(Some("Al 6061"))
                            .state(s)
                            .trailing(Some("eyeOn")),
                    );
                }
                ui.add(TreeRow::new("rect", "Rectangle").child(true).enabled(false));
                ui.add(
                    TreeRow::new("sketch", "Sketch")
                        .collapsed(true)
                        .meta(Some("3 DOF")),
                );
                ui.add(PropertyRow::new("Area").value("3 093.14 mm²"));
                property_row_with(ui, "Material", |ui| ui.label("Aluminium"));
                let mut i = 0;
                let segs = [
                    Segment {
                        label: "A",
                        icon: None,
                    },
                    Segment {
                        label: "B",
                        icon: Some("fillet"),
                    },
                ];
                ui.add(SegmentedControl::new("seg", &mut i, &segs).width(240.0));
                ui.add(MenuSection::new("Geometric"));
                ui.add(MenuRow::new(Some("coincident"), "Coincident").shortcut(Some("C")));
                ui.add(MenuRow::new(None, "Disabled").enabled(false).checked(true));
                menu_divider(ui);
            });
        }
    }

    #[test]
    fn segmented_control_changes_value_on_click() {
        let ctx = egui::Context::default();
        crate::fonts::install(&ctx);
        if let Ok(t) = crate::theme::load(false) {
            crate::theme::install(&ctx, t, false);
        }
        let mut value = 0usize;
        let mut rect = Rect::NOTHING;
        let run = |ctx: &egui::Context, events, value: &mut usize, rect: &mut Rect| {
            let raw = egui::RawInput {
                events,
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 400.0))),
                ..Default::default()
            };
            let mut out = ctx.run_ui(raw, |ui| {
                let segs = [
                    Segment {
                        label: "One",
                        icon: None,
                    },
                    Segment {
                        label: "Two",
                        icon: None,
                    },
                ];
                *rect = ui
                    .add(SegmentedControl::new("seg", value, &segs).width(200.0))
                    .rect;
            });
            out.textures_delta.clear();
        };
        run(&ctx, Vec::new(), &mut value, &mut rect);
        run(&ctx, Vec::new(), &mut value, &mut rect);
        let at = pos2(rect.right() - 20.0, rect.center().y);
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        run(
            &ctx,
            vec![egui::Event::PointerMoved(at)],
            &mut value,
            &mut rect,
        );
        run(&ctx, vec![button(true)], &mut value, &mut rect);
        run(&ctx, vec![button(false)], &mut value, &mut rect);
        assert_eq!(value, 1);
    }
}
