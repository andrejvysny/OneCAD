//! Navigation settings (design 24b): how mouse, trackpad and touch move the camera.
use super::rows::{self, ROW_GAP, row, section};
use crate::navigation_prefs::{DragInput, MappingSlot, NavPreset, NavigationPrefs};
use egui::{Align, Layout, RichText, Stroke, Ui, vec2};
use onecad_widgets::{
    components::{
        button::Button,
        inputs::{Select, Toggle},
    },
    fonts,
    icon::Icon,
    metrics::{CONTROL, RADIUS_LG, SECONDARY, SECTION},
    theme::look,
};

const PRESETS: [(NavPreset, &str); 3] = [
    (NavPreset::ThreeButtonMouse, "Three-button mouse"),
    (NavPreset::Trackpad, "Trackpad"),
    (NavPreset::Custom, "Custom"),
];
const NONE: &str = "None";
const SELECT_W: f32 = 160.0;

pub fn show(ui: &mut Ui, prefs: &mut NavigationPrefs) {
    rows::page_header(
        ui,
        "Navigation",
        "How the mouse, trackpad and keyboard move the camera in the viewport.",
    );
    if rows::touch() {
        touch_section(ui);
    }
    let title = if rows::touch() {
        "MOUSE AND TRACKPAD"
    } else {
        "INPUT"
    };
    section(ui, title, |ui| {
        row(ui, "Preset", Some("Fills the mapping below"), |ui| {
            preset(ui, prefs)
        });
        mapping_table(ui, prefs);
    });
    section(ui, "CAMERA", |ui| camera(ui, prefs));
    footer(ui, prefs);
}

fn touch_section(ui: &mut Ui) {
    section(ui, "TOUCH", |ui| {
        for (label, value) in [
            ("Orbit", "One-finger drag"),
            ("Pan", "Two-finger drag"),
            ("Zoom", "Pinch"),
            ("Draw", "Apple Pencil (fingers never draw)"),
        ] {
            row(ui, label, None, |ui| rows::value(ui, value));
        }
    });
}

fn preset(ui: &mut Ui, prefs: &mut NavigationPrefs) {
    let labels = PRESETS.map(|(_, label)| label);
    let current = PRESETS.iter().position(|(p, _)| *p == prefs.preset);
    if let Some(index) = rows::segmented(ui, "nav-preset", current.unwrap_or(2), &labels) {
        prefs.apply_preset(PRESETS[index].0);
    }
}

fn mapping_table(ui: &mut Ui, prefs: &mut NavigationPrefs) {
    let t = look(ui.ctx()).theme;
    let width = ui.available_width();
    egui::Frame::new()
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(RADIUS_LG)
        .show(ui, |ui| {
            ui.set_width(width - 2.0);
            ui.spacing_mut().item_spacing = vec2(8.0, 0.0);
            header_row(ui);
            table_row(ui, "orbit", "Orbit", |ui| orbit_select(ui, prefs));
            table_row(ui, "pan", "Pan", |ui| pan_selects(ui, prefs));
            table_row(ui, "zoomIn", "Zoom", zoom_select);
            table_row(ui, "fit", "Fit all", |ui| fit_select(ui, prefs));
        });
    ui.add_space(ROW_GAP);
}

/// ACTION column: fixed and small in narrow columns so inputs keep the room.
fn action_width(width: f32) -> f32 {
    if rows::very_narrow(width) {
        96.0
    } else if rows::narrow(width) {
        120.0
    } else {
        rows::label_width(width)
    }
}

fn header_row(ui: &mut Ui) {
    let t = look(ui.ctx()).theme;
    let label_w = action_width(ui.available_width());
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), egui::Sense::hover());
    ui.painter().rect_filled(
        rect,
        egui::CornerRadius {
            nw: RADIUS_LG - 1,
            ne: RADIUS_LG - 1,
            sw: 0,
            se: 0,
        },
        t.panel,
    );
    for (x, text) in [(14.0, "ACTION"), (14.0 + label_w, "INPUT")] {
        ui.painter().text(
            rect.left_center() + vec2(x, 0.0),
            egui::Align2::LEFT_CENTER,
            text,
            egui::FontId::new(SECTION, fonts::medium()),
            t.ink5,
        );
    }
}

fn table_row(ui: &mut Ui, icon: &str, label: &str, inputs: impl FnOnce(&mut Ui)) {
    let t = look(ui.ctx()).theme;
    let height = rows::interactive_height() + 10.0;
    let width = ui.available_width();
    let label_w = action_width(width);
    let top = ui.cursor().top();
    ui.painter().hline(
        ui.max_rect().x_range(),
        top,
        Stroke::new(1.0, t.border_subtle),
    );
    ui.allocate_ui_with_layout(
        vec2(width, height),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.set_min_height(height);
            ui.add_space(14.0);
            ui.spacing_mut().item_spacing.x = 10.0;
            ui.allocate_ui_with_layout(
                vec2(label_w - 14.0, height),
                Layout::left_to_right(Align::Center),
                |ui| {
                    ui.set_min_width(label_w - 14.0);
                    ui.add(Icon::new(icon).size(18.0).tint(t.ink3));
                    ui.label(
                        RichText::new(label)
                            .size(CONTROL)
                            .family(fonts::medium())
                            .color(t.ink),
                    );
                },
            );
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
                inputs(ui);
            });
        },
    );
}

fn drag_labels() -> Vec<&'static str> {
    DragInput::ALL.iter().map(|input| input.label()).collect()
}

fn orbit_select(ui: &mut Ui, prefs: &mut NavigationPrefs) {
    if let Some(i) = pick(ui, "nav-orbit", prefs.orbit.label(), &drag_labels()) {
        prefs.assign(MappingSlot::Orbit, Some(DragInput::ALL[i]));
    }
}

fn pan_selects(ui: &mut Ui, prefs: &mut NavigationPrefs) {
    if let Some(i) = pick(ui, "nav-pan", prefs.pan.label(), &drag_labels()) {
        prefs.assign(MappingSlot::Pan, Some(DragInput::ALL[i]));
    }
    let t = look(ui.ctx()).theme;
    ui.label(RichText::new("or").size(12.0).color(t.ink6));
    let mut options = vec![NONE];
    options.extend(drag_labels());
    let current = prefs.pan_alt.map_or(NONE, DragInput::label);
    if let Some(i) = pick(ui, "nav-pan-alt", current, &options) {
        let input = i.checked_sub(1).map(|n| DragInput::ALL[n]);
        prefs.assign(MappingSlot::PanAlt, input);
    }
}

fn zoom_select(ui: &mut Ui) {
    let out = Select::new("nav-zoom", "Scroll", &["Scroll"])
        .width(SELECT_W)
        .hit_height(rows::hit())
        .enabled(false)
        .show(ui);
    rows::reason(out.response, "Zoom", "Scroll wheel and pinch always zoom");
}

fn fit_select(ui: &mut Ui, prefs: &mut NavigationPrefs) {
    let options = ["Middle double-click", NONE];
    let current = options[usize::from(!prefs.fit_on_middle_double_click)];
    if let Some(i) = pick(ui, "nav-fit", current, &options) {
        prefs.set_fit_on_middle_double_click(i == 0);
    }
}

fn pick(ui: &mut Ui, id: &str, current: &str, options: &[&str]) -> Option<usize> {
    Select::new(id, current, options)
        .width(SELECT_W)
        .hit_height(rows::hit())
        .show(ui)
        .chosen
}

fn camera(ui: &mut Ui, prefs: &mut NavigationPrefs) {
    row(ui, "Orbit around", None, |ui| {
        let labels = ["Selection", "Point under cursor", "Model centre"];
        let why = "Orbit pivot options are not available yet";
        rows::segmented_disabled(ui, "nav-pivot", 2, &labels, "Orbit around", why);
    });
    row(ui, "Zoom toward cursor", None, |ui| {
        ui.add(Toggle::new(&mut prefs.zoom_toward_cursor).hit_height(rows::hit()));
    });
    row(
        ui,
        "Invert scroll zoom",
        Some("Scroll up zooms out"),
        |ui| {
            ui.add(Toggle::new(&mut prefs.invert_scroll_zoom).hit_height(rows::hit()));
        },
    );
    row(
        ui,
        "View transitions",
        Some("Standard views and Fit all"),
        |ui| {
            let labels = ["Off", "Fast", "Smooth"];
            let why = "Animated view transitions are not available yet";
            rows::segmented_disabled(ui, "nav-transitions", 0, &labels, "View transitions", why);
        },
    );
}

fn footer(ui: &mut Ui, prefs: &mut NavigationPrefs) {
    let t = look(ui.ctx()).theme;
    ui.scope(|ui| {
        ui.set_max_width(ui.available_width().min(rows::CONTENT_MAX));
        let y = ui.cursor().top();
        ui.painter().hline(
            ui.max_rect().x_range(),
            y,
            Stroke::new(1.0, t.border_subtle),
        );
        ui.add_space(12.0);
        let stacked = rows::narrow(ui.available_width());
        let mut reset = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.add(Icon::new("help").size(14.0).tint(t.ink5));
            let text = RichText::new("Trackpad pinch and two-finger scroll always work.")
                .size(SECONDARY)
                .color(t.ink5);
            ui.add(egui::Label::new(text).wrap());
            if !stacked {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    reset = ui
                        .add(Button::new("Reset to preset").hit_height(rows::hit()))
                        .clicked();
                });
            }
        });
        if stacked {
            ui.add_space(8.0);
            reset = ui
                .add(Button::new("Reset to preset").hit_height(rows::hit()))
                .clicked();
        }
        if reset {
            prefs.reset_to_preset();
        }
    });
}
