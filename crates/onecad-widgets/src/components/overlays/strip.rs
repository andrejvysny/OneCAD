//! SessionStrip (c-strip): h38 r10 mode banner with status and exit actions.

use egui::{Align, Color32, CornerRadius, Layout, Margin, RichText, Stroke, Ui};

use crate::components::button::{Button, ButtonKind};
use crate::components::tooltip::Tip;
use crate::fonts;
use crate::frame;
use crate::icon::{self, IconStyle};
use crate::metrics::{CONTROL, Elevation, RADIUS_LG, SECONDARY};
use crate::theme::look;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StripVariant {
    Sketch,
    Neutral,
}

pub struct StripAction<'a> {
    pub label: &'a str,
    pub kind: ButtonKind,
    pub enabled: bool,
    pub tip: Option<Tip<'a>>,
}

struct Lead {
    title: Color32,
    gap: Color32,
}

/// (text, colour, optional icon)
pub type StripStatus<'a> = (&'a str, Color32, Option<&'a str>);

pub struct SessionStrip<'a> {
    variant: StripVariant,
    icon: Option<&'a str>,
    title: &'a str,
    subtitle: Option<&'a str>,
    status: Option<StripStatus<'a>>,
    actions: &'a [StripAction<'a>],
}

impl<'a> SessionStrip<'a> {
    pub fn new(variant: StripVariant) -> Self {
        Self {
            variant,
            icon: None,
            title: "",
            subtitle: None,
            status: None,
            actions: &[],
        }
    }
    pub fn icon(mut self, icon: Option<&'a str>) -> Self {
        self.icon = icon;
        self
    }
    pub fn title(mut self, title: &'a str) -> Self {
        self.title = title;
        self
    }
    pub fn subtitle(mut self, subtitle: Option<&'a str>) -> Self {
        self.subtitle = subtitle;
        self
    }
    pub fn status(mut self, status: Option<StripStatus<'a>>) -> Self {
        self.status = status;
        self
    }
    pub fn actions(mut self, actions: &'a [StripAction<'a>]) -> Self {
        self.actions = actions;
        self
    }

    /// Index of the clicked action.
    pub fn show(self, ui: &mut Ui) -> Option<usize> {
        self.show_actions(ui)
            .iter()
            .position(egui::Response::clicked)
    }

    /// Every action's response in order, so the host can name them for automation.
    pub fn show_actions(self, ui: &mut Ui) -> Vec<egui::Response> {
        let l = look(ui.ctx());
        let t = &l.theme;
        let (fill, border, title) = match self.variant {
            StripVariant::Sketch => (t.sketch_chrome, t.sketch_chrome_border, t.sel_text),
            StripVariant::Neutral => (t.surface, t.border, t.ink),
        };
        let lead = Lead { title, gap: fill };
        let mut frame = frame::surface(t, l.dark, RADIUS_LG, Elevation::Pill);
        frame.fill = fill;
        frame.stroke = Stroke::new(1.0, border);
        frame.inner_margin = Margin {
            left: 12,
            right: 5,
            top: 0,
            bottom: 0,
        };
        let mut responses = Vec::with_capacity(self.actions.len());
        frame.corner_radius = CornerRadius::same(RADIUS_LG);
        frame.show(ui, |ui| {
            ui.set_height(36.0);
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                self.leading(ui, &lead, &l);
                for a in self.actions {
                    let mut b = Button::new(a.label).kind(a.kind).enabled(a.enabled);
                    if let Some(tip) = a.tip {
                        b = b.tip(tip);
                    }
                    responses.push(ui.add(b));
                }
            });
        });
        responses
    }

    fn leading(&self, ui: &mut Ui, lead: &Lead, l: &crate::theme::Look) {
        let (title, gap) = (lead.title, lead.gap);
        if let Some(name) = self.icon {
            ui.add(icon::Icon::new(name).size(20.0).tint(title).bg(gap));
        }
        ui.label(
            RichText::new(self.title)
                .size(CONTROL)
                .family(fonts::semibold())
                .color(title),
        );
        if let Some(sub) = self.subtitle {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.label(RichText::new(sub).size(CONTROL).color(l.theme.ink2));
            ui.spacing_mut().item_spacing.x = 10.0;
        }
        if let Some((text, color, icon_name)) = self.status {
            let rule = match self.variant {
                StripVariant::Sketch => l.theme.sketch_chrome_border,
                StripVariant::Neutral => l.theme.border,
            };
            divider(ui, rule);
            ui.spacing_mut().item_spacing.x = 6.0;
            if let Some(n) = icon_name {
                let style = IconStyle {
                    tint: color,
                    bg: gap,
                    grey: false,
                };
                let (r, _) = ui.allocate_exact_size(egui::Vec2::splat(16.0), egui::Sense::hover());
                icon::paint(ui.painter(), r, n, l.dark, style, 1.0);
            }
            ui.label(
                RichText::new(text)
                    .size(SECONDARY)
                    .family(fonts::medium())
                    .color(color),
            );
            ui.spacing_mut().item_spacing.x = 10.0;
        }
    }
}

/// 1px × 18px rule before the status block.
fn divider(ui: &mut Ui, colour: Color32) {
    let (r, _) = ui.allocate_exact_size(egui::vec2(1.0, 18.0), egui::Sense::hover());
    ui.painter().rect_filled(r, 0.0, colour);
}
