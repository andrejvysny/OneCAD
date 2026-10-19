//! 09b try-parameters sheet: live preview pane and a parameter column over a dimmed page.
use super::{
    LibraryUi,
    data::{COMPONENTS, DENSITY, HOLES, MATERIALS, PRESETS},
    kit::{self, ChipKind},
};
use crate::ui_state::{StartPage, UiState};
use egui::{
    Align2, CornerRadius, FontFamily, FontId, Rect, Sense, Stroke, Ui, UiBuilder, pos2, vec2,
};
use onecad_widgets::{
    components::{
        button::{Button, ButtonKind},
        inputs::{NumberField, Select, Toggle},
        lists::{Segment, SegmentedControl},
    },
    fonts,
    theme::look,
};

const SIZE: egui::Vec2 = vec2(940.0, 640.0);
const SIDE_W: f32 = 340.0;
const CUSTOM: usize = 3;
/// Mock allowance for the second leg and slots so the 72 x 40 x 4 Al bracket weighs 38.2 g.
const LEG_FACTOR: f32 = 1.228;

#[derive(Clone, Debug)]
pub struct SheetState {
    pub preset: usize,
    pub width: f32,
    pub height: f32,
    pub thickness: f32,
    pub hole: usize,
    pub fillets: bool,
    pub material: usize,
    pub width_text: String,
    pub height_text: String,
    base: (f32, f32),
}

impl SheetState {
    pub fn from_preset(preset: usize) -> Self {
        let (_, w, h, t) = PRESETS[preset.min(2)];
        Self {
            preset: preset.min(2),
            width: w,
            height: h,
            thickness: t,
            hole: 1,
            fillets: true,
            material: 0,
            width_text: w.to_string(),
            height_text: h.to_string(),
            base: (w, h),
        }
    }

    /// Picking Small/Standard/Heavy rewrites the fields; Custom keeps them.
    pub fn apply_preset(&mut self, preset: usize) {
        if preset < CUSTOM {
            let (_, w, h, t) = PRESETS[preset];
            (self.width, self.height, self.thickness) = (w, h, t);
            self.width_text = w.to_string();
            self.height_text = h.to_string();
        }
        self.preset = preset;
    }

    /// `typed` edits keep the user's text; slider edits rewrite it.
    pub fn set_width(&mut self, w: f32, typed: bool) {
        self.width = w.clamp(40.0, 120.0);
        if !typed {
            self.width_text = self.width.to_string();
        }
        self.preset = CUSTOM;
    }

    pub fn set_height(&mut self, h: f32, typed: bool) {
        self.height = h.clamp(30.0, 80.0);
        if !typed {
            self.height_text = self.height.to_string();
        }
        self.preset = CUSTOM;
    }

    pub fn mass_g(&self) -> f32 {
        let mm3 = self.width * self.height * self.thickness;
        mm3 * DENSITY[self.material] * 1e-3 * LEG_FACTOR
    }

    pub fn summary(&self) -> String {
        format!(
            "{} × {} × {} mm · {:.1} g",
            self.width,
            self.height,
            self.thickness,
            self.mass_g()
        )
    }

    /// Parameters whose value differs from the preset the sheet opened with.
    pub fn changed(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.width != self.base.0 {
            out.push("width changed");
        }
        if self.height != self.base.1 {
            out.push("height changed");
        }
        out
    }

    fn insert_label(&self) -> String {
        let name = PRESETS.get(self.preset).map_or("Custom", |p| p.0);
        format!("Insert as {name}")
    }
}

pub fn open(lib: &mut LibraryUi) {
    lib.sheet = Some(SheetState::from_preset(lib.preset));
}

enum Outcome {
    Close,
    Details,
    Insert,
}

pub fn show(state: &mut UiState, ctx: &egui::Context) {
    let Some(mut sheet) = state.library.sheet.take() else {
        return;
    };
    let name = COMPONENTS[state.library.selected].name;
    let frame = kit::modal_frame(ctx);
    let mut outcome = None;
    let response = egui::Modal::new(egui::Id::unique("library-try-sheet"))
        .frame(frame)
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            let (rect, _) = ui.allocate_exact_size(SIZE, Sense::hover());
            let left = Rect::from_min_max(rect.min, pos2(rect.right() - SIDE_W, rect.bottom()));
            let side = Rect::from_min_max(pos2(rect.right() - SIDE_W, rect.top()), rect.max);
            preview_pane(ui, left, &sheet);
            ui.scope_builder(UiBuilder::new().max_rect(side), |ui| {
                outcome = side_column(ui, side, name, &mut sheet);
            });
        });
    let closed = response.should_close() || matches!(outcome, Some(Outcome::Close));
    match outcome {
        Some(Outcome::Details) => state.start_page = StartPage::ComponentDetail,
        Some(Outcome::Insert) => {
            state.library.note = Some("Inserted with custom parameters (preview only).");
        }
        _ => {}
    }
    if !closed && !matches!(outcome, Some(Outcome::Details | Outcome::Insert)) {
        state.library.sheet = Some(sheet);
    }
}

fn preview_pane(ui: &mut Ui, rect: Rect, sheet: &SheetState) {
    let t = look(ui.ctx()).theme;
    let p = ui.painter();
    let radius = CornerRadius {
        nw: 12,
        sw: 12,
        ne: 0,
        se: 0,
    };
    p.rect_filled(rect, radius, t.well);
    let mut x = rect.left() + 14.0;
    let y = rect.top() + 14.0;
    x += kit::paint_chip(p, &t, pos2(x, y), "Live preview", ChipKind::Dark) + 6.0;
    for text in sheet.changed() {
        x += kit::paint_chip(p, &t, pos2(x, y), text, ChipKind::Warn) + 6.0;
    }
    let font = FontId::new(11.5, fonts::mono());
    let at = pos2(rect.left() + 14.0, rect.bottom() - 18.0);
    p.text(at, Align2::LEFT_CENTER, sheet.summary(), font, t.ink4);
}

fn side_column(ui: &mut Ui, side: Rect, name: &str, sheet: &mut SheetState) -> Option<Outcome> {
    let t = look(ui.ctx()).theme;
    let mut outcome = None;
    let body = side.shrink2(vec2(18.0, 0.0));
    let footer = Rect::from_min_max(pos2(side.left(), side.bottom() - 68.0), side.max);
    ui.painter()
        .hline(footer.x_range(), footer.top(), Stroke::new(1.0, t.border));
    ui.scope_builder(
        UiBuilder::new().max_rect(body.with_min_y(side.top() + 16.0)),
        |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            if header(ui, name) {
                outcome = Some(Outcome::Close);
            }
            ui.add_space(6.0);
            fields(ui, sheet);
        },
    );
    let hint = Rect::from_min_size(
        pos2(body.left(), footer.top() - 46.0),
        vec2(body.width(), 36.0),
    );
    ui.scope_builder(UiBuilder::new().max_rect(hint), hint_row);
    let foot = footer.shrink2(vec2(18.0, 0.0));
    ui.scope_builder(UiBuilder::new().max_rect(foot), |ui| {
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_height(footer.height());
            if kit::link(ui, "Full details", 12.5).clicked() {
                outcome = Some(Outcome::Details);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let label = sheet.insert_label();
                let b = Button::new(&label)
                    .kind(ButtonKind::Primary)
                    .icon(Some("plus"));
                if ui.add(b).clicked() {
                    outcome = Some(Outcome::Insert);
                }
            });
        });
    });
    outcome
}

fn header(ui: &mut Ui, name: &str) -> bool {
    let mut close = false;
    ui.horizontal(|ui| {
        kit::title(ui, name, 15.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            close = kit::icon_button(ui, "close", 28.0, false).clicked();
        });
    });
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        kit::chip(ui, "fx 7", ChipKind::Accent);
        kit::chip(ui, "v3", ChipKind::Plain);
    });
    close
}

fn param_label(ui: &mut Ui, text: &str) {
    let ink = look(ui.ctx()).theme.ink4;
    kit::label(ui, text, 12.0, fonts::mono(), ink);
}

fn fields(ui: &mut Ui, sheet: &mut SheetState) {
    let width = ui.available_width();
    let presets =
        ["Small", "Standard", "Heavy", "Custom"].map(|label| Segment { label, icon: None });
    let mut preset = sheet.preset;
    ui.add(SegmentedControl::new("sheet-preset", &mut preset, &presets).width(width));
    if preset != sheet.preset {
        sheet.apply_preset(preset);
    }
    ui.add_space(6.0);
    param_label(ui, "width");
    let off = sheet.width != sheet.base.0;
    if let Some((v, typed)) = slider_row(
        ui,
        "sheet-width",
        sheet.width,
        40.0..=120.0,
        &mut sheet.width_text,
        off,
    ) {
        sheet.set_width(v, typed);
    }
    param_label(ui, "height");
    let off = sheet.height != sheet.base.1;
    if let Some((v, typed)) = slider_row(
        ui,
        "sheet-height",
        sheet.height,
        30.0..=80.0,
        &mut sheet.height_text,
        off,
    ) {
        sheet.set_height(v, typed);
    }
    param_label(ui, "hole_size");
    let holes = HOLES.map(|label| Segment { label, icon: None });
    ui.add(SegmentedControl::new("sheet-hole", &mut sheet.hole, &holes).width(width));
    ui.horizontal(|ui| {
        param_label(ui, "fillets");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add(Toggle::new(&mut sheet.fillets));
        });
    });
    param_label(ui, "material");
    let out = Select::new("sheet-material", MATERIALS[sheet.material], &MATERIALS)
        .width(width)
        .show(ui);
    if let Some(i) = out.chosen {
        sheet.material = i;
    }
}

/// Slider plus numeric field; returns the new value and whether it was typed.
fn slider_row(
    ui: &mut Ui,
    id: &str,
    value: f32,
    range: std::ops::RangeInclusive<f32>,
    text: &mut String,
    changed_from_base: bool,
) -> Option<(f32, bool)> {
    let t = look(ui.ctx()).theme;
    let mut result = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        ui.spacing_mut().slider_width = ui.available_width() - 92.0 - 12.0;
        let mut v = value;
        let fill = if changed_from_base { t.warn } else { t.accent };
        ui.visuals_mut().selection.bg_fill = fill;
        ui.visuals_mut().widgets.hovered.bg_fill = fill;
        let slider = egui::Slider::new(&mut v, range)
            .show_value(false)
            .trailing_fill(true)
            .step_by(1.0);
        if ui.add(slider).changed() {
            result = Some((v, false));
        }
        let field = NumberField::new(id, text)
            .unit("mm")
            .width(92.0)
            .decimals(0);
        if ui.add(field).changed()
            && let Ok(v) = text.trim().parse::<f32>()
        {
            result = Some((v, true));
        }
    });
    result
}

fn hint_row(ui: &mut Ui) {
    let t = look(ui.ctx()).theme;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
        kit::paint_icon(ui.painter(), r, "help", t.surface, t.ink5);
        let text = "Changes here only affect the preview and the next insert.";
        kit::label(ui, text, 12.0, FontFamily::Proportional, t.ink5);
    });
}
