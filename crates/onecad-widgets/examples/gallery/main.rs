//! Component gallery. Add a group: write `examples/gallery/<group>.rs` with
//! `pub fn show(ui, t, dark)`, add one `mod` and one line to `GROUPS`.
mod foundations;
mod g_actions;
mod g_inputs;
mod g_lists;
mod g_overlays;
mod g_shell;
mod g_start;
mod g_tabs;
mod icons;
mod screens;

use egui::RichText;
use onecad_widgets::{Theme, fonts, theme};

/// (section title, renderer) in display order.
type GroupFn = fn(&mut egui::Ui, &Theme, bool);

const GROUPS: &[(&str, GroupFn)] = &[
    ("g00 Foundations", foundations::show),
    ("g01 Actions", g_actions::show),
    ("g02 Shell", g_shell::show),
    ("g03 Viewport overlays", g_overlays::show),
    ("g04 Panels & lists", g_lists::show),
    ("g05 Inputs", g_inputs::show),
    ("g06 Start screen", g_start::show),
];

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Components,
    Screens,
    Icons,
}

struct Gallery {
    tab: Tab,
    dark: bool,
    applied: Option<bool>,
}

impl eframe::App for Gallery {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.applied != Some(self.dark) {
            if let Ok(t) = theme::load(self.dark) {
                theme::install(&ctx, t, self.dark);
            }
            self.applied = Some(self.dark);
        }
        let look = theme::look(&ctx);
        egui::Panel::top("bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, Tab::Components, "Components");
                ui.selectable_value(&mut self.tab, Tab::Screens, "Screens");
                ui.selectable_value(&mut self.tab, Tab::Icons, "Icons");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.selectable_value(&mut self.dark, true, "Dark");
                    ui.selectable_value(&mut self.dark, false, "Light");
                });
            });
        });
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::both().show(ui, |ui| match self.tab {
                Tab::Components => {
                    for (title, show) in GROUPS {
                        ui.label(RichText::new(*title).size(15.0).family(fonts::semibold()));
                        show(ui, &look.theme, look.dark);
                        ui.add_space(24.0);
                    }
                }
                Tab::Screens => {
                    screens::show(ui, &look.theme, look.dark);
                }
                Tab::Icons => icons::show(ui),
            });
        });
    }
}

fn main() -> eframe::Result {
    eframe::run_native(
        "OneCAD widgets gallery",
        eframe::NativeOptions::default(),
        Box::new(|cc| {
            fonts::install(&cc.egui_ctx);
            Ok(Box::new(Gallery {
                tab: Tab::Components,
                dark: false,
                applied: None,
            }))
        }),
    )
}
