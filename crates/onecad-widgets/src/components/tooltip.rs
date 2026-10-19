//! Tooltip (c-tooltip): dark slab after 400 ms; name + mono shortcut, plus the reason a
//! disabled control is unavailable.

use egui::{Margin, Response, RichText};

use crate::components::actions::puzzle;
use crate::fonts;
use crate::metrics::{Elevation, RADIUS_SM, shadow};
use crate::theme::look;

/// What the tooltip says. `reason` is shown for disabled controls.
#[derive(Clone, Copy, Debug, Default)]
pub struct Tip<'a> {
    pub name: &'a str,
    pub shortcut: Option<&'a str>,
    pub reason: Option<&'a str>,
    /// Second line naming the extension that contributes the control, e.g. "CNC Prep · Millhouse".
    pub source: Option<&'a str>,
}

impl<'a> Tip<'a> {
    pub fn new(name: &'a str) -> Self {
        Self {
            name,
            ..Self::default()
        }
    }
    pub fn shortcut(mut self, key: Option<&'a str>) -> Self {
        self.shortcut = key;
        self
    }
    /// Why the control is unavailable. Pass it only while disabled: widgets drawn
    /// with `Sense::hover` still report `enabled()`, so the reason is shown whenever set.
    pub fn reason(mut self, reason: Option<&'a str>) -> Self {
        self.reason = reason;
        self
    }
    pub fn source(mut self, source: Option<&'a str>) -> Self {
        self.source = source;
        self
    }
}

/// Attach the design tooltip to `response`, for enabled and disabled widgets alike.
pub fn attach(response: Response, tip: Tip<'_>) -> Response {
    let l = look(&response.ctx);
    let frame = egui::Frame::new()
        .fill(l.theme.tooltip)
        .corner_radius(RADIUS_SM - 1)
        .inner_margin(Margin::symmetric(8, 5))
        .shadow(shadow(Elevation::Tooltip, &l.theme, l.dark));
    let mut tooltip = if response.enabled() {
        egui::Tooltip::for_enabled(&response)
    } else {
        egui::Tooltip::for_disabled(&response)
    };
    tooltip.popup = tooltip.popup.frame(frame).width(320.0);
    tooltip.show(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let text = match tip.reason {
                Some(reason) => format!("{} — {reason}", tip.name),
                _ => tip.name.to_owned(),
            };
            ui.label(RichText::new(text).size(11.5).color(l.theme.tooltip_text));
            if let Some(key) = tip.shortcut {
                ui.label(
                    RichText::new(key)
                        .size(11.0)
                        .family(fonts::mono())
                        .color(l.theme.tooltip_text.gamma_multiply(0.7)),
                );
            }
        });
        if let Some(source) = tip.source {
            let muted = l.theme.tooltip_text.gamma_multiply(0.7);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                let (r, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                puzzle(ui.painter(), r.center(), 10.0, muted, None);
                ui.label(RichText::new(source).size(11.0).color(muted));
            });
        }
    });
    response
}
