//! ViewCluster (c-view): view-cube slot over a column of 36×36 square buttons.

use egui::{Pos2, Rect, Response, Sense, Ui, Vec2};

use crate::components::shell::{Backdrop, IconButton};
use crate::components::tooltip::Tip;
use crate::metrics::{CHROME_GAP, Elevation, H_SQUARE, RADIUS_MD};

pub struct SquareButton<'a> {
    pub icon: &'a str,
    pub on: bool,
    pub enabled: bool,
    pub tip: Tip<'a>,
}

pub struct ViewCluster<'a> {
    buttons: &'a [SquareButton<'a>],
    cube: Option<f32>,
}

#[derive(Default)]
pub struct ViewClusterOutput {
    pub clicked: Option<usize>,
}

impl<'a> ViewCluster<'a> {
    pub fn new(buttons: &'a [SquareButton<'a>]) -> Self {
        Self {
            buttons,
            cube: Some(72.0),
        }
    }
    pub fn cube(mut self, size: f32) -> Self {
        self.cube = Some(size);
        self
    }
    /// No cube slot: the button column starts at the top and `paint_cube` is not called.
    pub fn no_cube(mut self) -> Self {
        self.cube = None;
        self
    }

    /// `paint_cube` draws the host's navigation cube into the reserved slot; the
    /// `Response` (click + drag over the slot) lets the host orbit / pick faces.
    pub fn show(
        self,
        ui: &mut Ui,
        paint_cube: impl FnOnce(&mut Ui, Rect, &Response),
    ) -> ViewClusterOutput {
        let n = self.buttons.len() as f32;
        let cube = self.cube.unwrap_or(0.0);
        let lead = if self.cube.is_some() {
            cube + CHROME_GAP
        } else {
            0.0
        };
        let height = lead + n * H_SQUARE + (n - 1.0).max(0.0) * CHROME_GAP;
        let (outer, _) =
            ui.allocate_exact_size(Vec2::new(cube.max(H_SQUARE), height), Sense::hover());
        if self.cube.is_some() {
            let slot = Rect::from_min_size(outer.min, Vec2::splat(cube));
            let resp = ui.interact(
                slot,
                ui.scope_id().with("view-cube"),
                Sense::click_and_drag(),
            );
            paint_cube(ui, slot, &resp);
        }
        let mut out = ViewClusterOutput::default();
        let x = outer.center().x - H_SQUARE / 2.0;
        let mut y = outer.top() + lead;
        for (i, b) in self.buttons.iter().enumerate() {
            let r = Rect::from_min_size(Pos2::new(x, y), Vec2::splat(H_SQUARE));
            let btn = IconButton::new(b.icon, b.tip.name, H_SQUARE, 20.0)
                .radius(RADIUS_MD)
                .backdrop(Backdrop::Floating)
                .shadow(Elevation::Pill)
                .on(b.on)
                .enabled(b.enabled)
                .tip(Some(b.tip));
            if ui.put(r, btn).clicked() {
                out.clicked = Some(i);
            }
            y += H_SQUARE + CHROME_GAP;
        }
        out
    }
}
