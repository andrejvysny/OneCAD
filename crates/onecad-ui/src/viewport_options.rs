//! Transient viewport settings, applied by the host after GPU work completes.

use crate::app::App;
use onecad_render::SolidCraftOptions;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    ui.menu_button("Display", |ui| {
        let mut options = app
            .pending_display_options
            .as_ref()
            .unwrap_or_else(|| app.renderer.options())
            .clone();
        let mut changed = ui.checkbox(&mut options.outline, "Silhouettes").changed();
        changed |= ui
            .checkbox(&mut options.hidden_edges, "Hidden edges")
            .changed();
        changed |= ui
            .add(egui::Slider::new(&mut options.opacity, 0.15..=1.0).text("Opacity"))
            .changed();
        changed |= supersampling(ui, &mut options);
        changed |= ui
            .add_enabled_ui(
                app.interaction.tool.is_none() && !app.sketch_active(),
                |ui| section(ui, app, &mut options),
            )
            .inner;
        if changed {
            app.pending_display_options = Some(options);
            app.draw_dirty = true;
            ui.ctx().request_repaint();
        }
    });
}

fn supersampling(ui: &mut egui::Ui, options: &mut SolidCraftOptions) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt("viewport-ssaa")
        .selected_text(format!("SSAA {}×", options.ssaa))
        .show_ui(ui, |ui| {
            for factor in [1.0, 2.0] {
                changed |= ui
                    .selectable_value(&mut options.ssaa, factor, format!("{factor}×"))
                    .changed();
            }
        });
    changed
}

fn section(ui: &mut egui::Ui, app: &App, options: &mut SolidCraftOptions) -> bool {
    let original = section_axis(options.section);
    let mut axis = original;
    egui::ComboBox::from_id_salt("viewport-section")
        .selected_text(format!("Section {}", section_label(axis)))
        .show_ui(ui, |ui| {
            for candidate in [None, Some(2), Some(1), Some(0)] {
                ui.selectable_value(&mut axis, candidate, section_label(candidate));
            }
        });
    let mut changed = axis != original;
    if changed {
        options.section = [0.0; 4];
        if let Some(axis) = axis {
            options.section[axis] = 1.0;
            options.section[3] = center_offset(app, axis);
        }
    }
    if axis.is_some() {
        let mut offset = options.section[3];
        let response = ui.add(
            egui::DragValue::new(&mut offset)
                .prefix("Offset ")
                .suffix(" mm"),
        );
        if response.changed() && offset.is_finite() {
            options.section[3] = offset;
            changed = true;
        }
        ui.label("Display only. Selection is disabled while a section is active.");
    }
    changed
}

fn section_axis(section: [f32; 4]) -> Option<usize> {
    section[..3].iter().position(|normal| *normal != 0.0)
}

fn section_label(axis: Option<usize>) -> &'static str {
    match axis {
        Some(2) => "XY",
        Some(1) => "XZ",
        Some(0) => "YZ",
        _ => "Off",
    }
}

fn center_offset(app: &App, axis: usize) -> f32 {
    let center = app
        .display
        .as_ref()
        .and_then(|display| display.scene.bounds())
        .map(|bounds| bounds.min * 0.5 + bounds.max * 0.5)
        .unwrap_or_default();
    let offset = center[axis] as f32;
    if offset.is_finite() { offset } else { 0.0 }
}
