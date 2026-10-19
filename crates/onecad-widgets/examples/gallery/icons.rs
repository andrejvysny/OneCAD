use egui::RichText;
use onecad_widgets::icon::{self, Icon};

const SIZES: [f32; 6] = [14.0, 16.0, 18.0, 20.0, 24.0, 32.0];

pub fn show(ui: &mut egui::Ui) {
    egui::Grid::new("icons").striped(true).show(ui, |ui| {
        for name in icon::names() {
            for s in SIZES {
                ui.add(Icon::new(name).size(s));
            }
            ui.label(RichText::new(name).size(12.0));
            ui.end_row();
        }
    });
}
