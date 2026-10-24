//! Offset datums are explicit document edits; sketches retain their datum attachment.
use crate::{app::App, bridge::Command, sketch_services::Request};
use onecad_core::{
    document::datum::DatumPlane,
    ids::DatumPlaneId,
    math::Vec3,
    sketch::{Sketch, SketchAttachment, SketchPlane, WorldPlane},
};
use onecad_widgets::components::inputs::NumberField;

#[derive(Clone, Debug)]
pub(crate) struct Draft {
    open: bool,
    base: WorldPlane,
    offset: String,
}

impl Default for Draft {
    fn default() -> Self {
        Self {
            open: false,
            base: WorldPlane::XY,
            offset: "10".into(),
        }
    }
}

impl Draft {
    pub(crate) fn open(&mut self) {
        self.open = true;
    }
}

pub(crate) fn show(app: &mut App, ui: &mut egui::Ui) {
    if app.doc.sketch_entry.pending.is_some() {
        return;
    }
    let datums: Vec<_> = app
        .doc
        .display
        .as_ref()
        .map(|display| display.projection.datums.values().cloned().collect())
        .unwrap_or_default();
    let mut selected = None;
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("sketch-datum-plane")
            .selected_text("Datum plane…")
            .show_ui(ui, |ui| {
                for datum in &datums {
                    let response = ui.add_enabled(
                        datum.resolved_valid && !app.busy(),
                        egui::Button::new(&datum.name),
                    );
                    if response.clicked() {
                        selected = datum.id.parse().ok();
                    }
                    if !datum.resolved_valid {
                        response.on_hover_text("Datum frame is unresolved");
                    }
                }
                if datums.is_empty() {
                    ui.label("No datum planes");
                }
            });
        if ui.small_button("Offset plane…").clicked() {
            app.doc.sketch_entry.datum.open = true;
        }
    });
    if let Some(id) = selected {
        app.new_sketch_on_datum(id);
        return;
    }
    if app.doc.sketch_entry.datum.open {
        editor(app, ui);
    }
}

fn editor(app: &mut App, ui: &mut egui::Ui) {
    let enabled = !app.busy();
    let (create, cancel) = offset_form(&mut app.doc.sketch_entry.datum, ui, enabled);
    if cancel {
        app.doc.sketch_entry.datum = Draft::default();
    } else if create {
        submit(app);
    }
}

fn offset_form(draft: &mut Draft, ui: &mut egui::Ui, enabled: bool) -> (bool, bool) {
    let mut create = false;
    let mut cancel = false;
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("offset-plane-base")
            .selected_text(plane_name(draft.base))
            .show_ui(ui, |ui| {
                for plane in [WorldPlane::XY, WorldPlane::XZ, WorldPlane::YZ] {
                    ui.selectable_value(&mut draft.base, plane, plane_name(plane));
                }
            });
        let input = ui.add(
            NumberField::new("offset-plane-distance", &mut draft.offset)
                .label("Offset (mm)")
                .unit("mm")
                .width(110.0)
                .enabled(enabled),
        );
        crate::automation::name(ui.ctx(), input.id, "onecad.datum.offset");
        create = ui
            .add_enabled(
                enabled && offset(&draft.offset).is_ok(),
                egui::Button::new("Create plane"),
            )
            .clicked();
        let button = ui.small_button("Cancel plane");
        crate::automation::name(ui.ctx(), button.id, "onecad.datum.cancel");
        cancel = button.clicked();
    });
    ui.small("Signed offset along the plane normal. Creates a frozen datum plane.");
    if let Err(error) = offset(&draft.offset) {
        ui.small(error);
    }
    (create, cancel)
}

fn submit(app: &mut App) {
    let Some(context) = app.doc.interaction.context.clone() else {
        return;
    };
    let draft = &app.doc.sketch_entry.datum;
    let Ok(datum) = definition(draft.base, &draft.offset) else {
        return;
    };
    match app
        .doc
        .bridge
        .submit(Command::Sketch(Box::new(Request::CreateDatum {
            context,
            datum: Box::new(datum),
        }))) {
        Ok(()) => {
            // Create is an explicit submission, so closing its editor cannot imply cancellation.
            app.doc.sketch_entry.dismiss();
            app.doc.command_status = true;
            app.doc.status = "Creating datum plane…".into();
        }
        Err(error) => app.doc.status = error,
    }
}

pub(crate) fn definition(base: WorldPlane, text: &str) -> Result<DatumPlane, String> {
    let offset = offset(text)?;
    let base = plane_name(base);
    let distance = crate::sketch_numeric::format::format_measure(offset);
    Ok(DatumPlane::offset_from_plane(
        DatumPlaneId::new(),
        format!("{base} offset {distance} mm"),
        base,
        offset,
    ))
}

fn offset(text: &str) -> Result<f64, String> {
    text.trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .ok_or_else(|| "Offset must be a finite number in mm".into())
}

pub(crate) fn sketch(datum: &onecad_app::dto::DatumDto) -> Result<Sketch, String> {
    if !datum.resolved_valid {
        return Err("Datum frame is unresolved; choose another support".into());
    }
    let datum_id = datum.id.parse().map_err(|_| "Datum identity is invalid")?;
    let v = |[x, y, z]: [f64; 3]| Vec3::new(x, y, z).ok_or("Datum frame is invalid");
    let plane = SketchPlane::new(
        v(datum.plane.origin)?,
        v(datum.plane.x_axis)?,
        v(datum.plane.y_axis)?,
        v(datum.plane.normal)?,
    )
    .ok_or("Datum frame is invalid")?;
    if !onecad_interaction::sketch::validate_sketch_plane(plane) {
        return Err("Datum frame is invalid".into());
    }
    let mut sketch = Sketch::new(
        onecad_core::ids::SketchId::new(),
        "Sketch",
        SketchAttachment::Datum { datum: datum_id },
    );
    sketch.plane = plane;
    Ok(sketch)
}

fn plane_name(plane: WorldPlane) -> &'static str {
    match plane {
        WorldPlane::XY => "XY",
        WorldPlane::XZ => "XZ",
        WorldPlane::YZ => "YZ",
    }
}

#[cfg(test)]
mod tests;
