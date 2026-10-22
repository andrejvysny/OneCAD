//! FeatureCard (c-card): parameters for the running feature. Header · field grid · footer.
//! The card only reports clicks; the host owns Enter / Escape.

use egui::{FontId, Margin, Pos2, Rect, RichText, Sense, Ui, Vec2, vec2};

use crate::components::button::{Button, ButtonKind};
use crate::components::shell::IconButton;
use crate::components::tooltip::Tip;
use crate::fonts;
use crate::frame;
use crate::icon::{self, IconStyle};
use crate::metrics::{CONTROL, Elevation, PANEL_TITLE, RADIUS_XL};
use crate::theme::look;

const HEADER_H: f32 = 44.0;
const LABEL_W: f32 = 76.0;

#[derive(Default)]
pub struct FeatureCardOutput {
    pub close: bool,
    pub cancel: bool,
    pub ok: bool,
    /// Widget ids of the footer buttons, so the host can name them for automation.
    pub cancel_id: Option<egui::Id>,
    pub ok_id: Option<egui::Id>,
}

pub struct FeatureCard<'a> {
    id: egui::IdSalt,
    icon: &'a str,
    title: &'a str,
    shortcut: Option<&'a str>,
    width: f32,
    ok_enabled: bool,
    ok_label: &'a str,
    cancel_enabled: bool,
}

/// Body helper: label column 76, row gap 10. The closure passed to `row` gets the
/// remaining width (`ui.available_width()`) for its field.
pub struct FeatureCardRows<'u> {
    ui: &'u mut Ui,
    /// Label column width: 76, or the widest label seen last frame.
    label_w: f32,
    seen_w: f32,
}

impl FeatureCardRows<'_> {
    pub fn row(&mut self, label: &str, add: impl FnOnce(&mut Ui)) {
        let ink4 = look(self.ui.ctx()).theme.ink4;
        let label_w = self.label_w;
        let mut seen = 0.0_f32;
        self.ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let font = FontId::proportional(CONTROL);
            let g = ui.painter().layout_no_wrap(label.to_owned(), font, ink4);
            seen = g.size().x;
            // Exact size: a laid-out child would shrink to the label and misalign fields.
            let (rect, _) = ui.allocate_exact_size(vec2(label_w, 30.0), Sense::hover());
            let pos = Pos2::new(rect.left(), rect.center().y - g.size().y / 2.0);
            ui.painter().galley(pos, g, ink4);
            add(ui);
        });
        self.seen_w = self.seen_w.max(seen);
    }
    pub fn caption(&mut self, text: &str) {
        let ink5 = look(self.ui.ctx()).theme.ink5;
        let label_w = self.label_w;
        // Sits under the field column, tucked against the row above.
        self.ui.add_space(-6.0);
        self.ui.horizontal(|ui| {
            ui.add_space(label_w + 8.0);
            ui.label(RichText::new(text).size(11.5).color(ink5));
        });
    }
}

impl<'a> FeatureCard<'a> {
    pub fn new(id_salt: impl egui::AsIdSalt, icon: &'a str, title: &'a str) -> Self {
        Self {
            id: egui::IdSalt::new(id_salt),
            icon,
            title,
            shortcut: None,
            width: 280.0,
            ok_enabled: true,
            ok_label: "OK ↵",
            cancel_enabled: true,
        }
    }
    pub fn shortcut(mut self, key: Option<&'a str>) -> Self {
        self.shortcut = key;
        self
    }
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }
    pub fn ok_enabled(mut self, enabled: bool) -> Self {
        self.ok_enabled = enabled;
        self
    }

    pub fn ok_label(mut self, label: &'a str) -> Self {
        self.ok_label = label;
        self
    }
    pub fn cancel_enabled(mut self, enabled: bool) -> Self {
        self.cancel_enabled = enabled;
        self
    }

    pub fn show(self, ui: &mut Ui, rows: impl FnOnce(&mut FeatureCardRows)) -> FeatureCardOutput {
        let l = look(ui.ctx());
        let mut out = FeatureCardOutput::default();
        let inner_w = self.width - 2.0;
        ui.push_id(self.id, |ui| {
            frame::surface(&l.theme, l.dark, RADIUS_XL, Elevation::Card).show(ui, |ui| {
                ui.set_width(inner_w);
                ui.spacing_mut().item_spacing.y = 0.0;
                out.close = self.header(ui);
                divider(ui);
                egui::Frame::new()
                    .inner_margin(Margin::same(12))
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 10.0;
                        let key = ui.scope_id().with("label-w");
                        let stored = ui.data(|d| d.get_temp::<f32>(key));
                        let mut body = FeatureCardRows {
                            ui,
                            label_w: stored.unwrap_or(LABEL_W).max(LABEL_W),
                            seen_w: 0.0,
                        };
                        rows(&mut body);
                        let next = (body.seen_w.ceil() + 2.0).max(LABEL_W);
                        if stored != Some(next) {
                            ui.data_mut(|d| d.insert_temp(key, next));
                            ui.ctx().request_repaint();
                        }
                    });
                divider(ui);
                self.footer(ui, &mut out);
            });
        });
        out
    }

    fn header(&self, ui: &mut Ui) -> bool {
        let l = look(ui.ctx());
        let t = &l.theme;
        let (rect, _) =
            ui.allocate_exact_size(vec2(ui.available_width(), HEADER_H), Sense::hover());
        let cy = rect.center().y;
        let p = ui.painter();
        let glyph =
            Rect::from_center_size(Pos2::new(rect.left() + 12.0 + 11.0, cy), Vec2::splat(22.0));
        let style = IconStyle {
            tint: t.ink4,
            bg: t.floating,
            grey: false,
        };
        icon::paint(p, glyph, self.icon, l.dark, style, 1.0);
        let g = p.layout_no_wrap(
            self.title.to_owned(),
            FontId::new(PANEL_TITLE, fonts::semibold()),
            t.ink,
        );
        let gh = g.size().y;
        p.galley(Pos2::new(glyph.right() + 9.0, cy - gh / 2.0), g, t.ink);
        let close =
            Rect::from_center_size(Pos2::new(rect.right() - 10.0 - 12.0, cy), Vec2::splat(24.0));
        if let Some(key) = self.shortcut {
            let g = p.layout_no_wrap(key.to_owned(), FontId::new(11.0, fonts::mono()), t.ink5);
            let pos = Pos2::new(close.left() - 9.0 - g.size().x, cy - g.size().y / 2.0);
            p.galley(pos, g, t.ink5);
        }
        let x = IconButton::new("close", "Close", 24.0, 14.0)
            .enabled(self.cancel_enabled)
            .gap_bg(t.surface)
            .tip(Some(Tip::new("Close")));
        ui.put(close, x).clicked()
    }

    fn footer(&self, ui: &mut Ui, out: &mut FeatureCardOutput) {
        egui::Frame::new()
            .inner_margin(Margin::symmetric(12, 10))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let w = (ui.available_width() - 8.0) / 2.0;
                    let cancel = ui.add(
                        Button::new("Cancel")
                            .enabled(self.cancel_enabled)
                            .min_width(w),
                    );
                    out.cancel = cancel.clicked();
                    out.cancel_id = Some(cancel.id);
                    let ok = Button::new(self.ok_label)
                        .kind(ButtonKind::Primary)
                        .enabled(self.ok_enabled)
                        .min_width(w);
                    let ok = ui.add(ok);
                    out.ok = ok.clicked();
                    out.ok_id = Some(ok.id);
                });
            });
    }
}

fn divider(ui: &mut Ui) {
    let t = look(ui.ctx()).theme;
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(r, 0.0, t.border_subtle);
}
