//! NavPill (c-nav): bottom-left pill of 32px icon buttons.

use egui::{Margin, Ui, vec2};

use crate::components::shell::{Backdrop, IconButton};
use crate::components::tooltip::Tip;
use crate::frame;
use crate::metrics::{Elevation, RADIUS_SM, RADIUS_XL};
use crate::theme::look;

pub struct NavButton<'a> {
    pub icon: &'a str,
    pub enabled: bool,
    pub active: bool,
    pub tip: Tip<'a>,
}

pub struct NavPill<'a> {
    buttons: &'a [NavButton<'a>],
}

impl<'a> NavPill<'a> {
    pub fn new(buttons: &'a [NavButton<'a>]) -> Self {
        Self { buttons }
    }

    /// Index of the clicked button.
    pub fn show(self, ui: &mut Ui) -> Option<usize> {
        let l = look(ui.ctx());
        let mut clicked = None;
        frame::surface(&l.theme, l.dark, RADIUS_XL, Elevation::Pill)
            .inner_margin(Margin::same(3))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(2.0, 0.0);
                ui.horizontal(|ui| {
                    for (i, b) in self.buttons.iter().enumerate() {
                        let btn = IconButton::new(b.icon, b.tip.name, 32.0, 20.0)
                            .radius(RADIUS_SM)
                            .backdrop(Backdrop::None)
                            .gap_bg(l.theme.surface)
                            .on(b.active)
                            .enabled(b.enabled)
                            .tip(Some(b.tip));
                        if ui.add(btn).clicked() {
                            clicked = Some(i);
                        }
                    }
                });
            });
        clicked
    }
}
