//! Primitive methods share one tool; choosing a method cancels only the local draft.
use crate::app::App;
use onecad_interaction::sketch::{SketchAction, SketchTool};

pub(crate) fn show(app: &mut App, ui: &mut egui::Ui) {
    rectangle(app, ui);
    arc(app, ui);
}

fn rectangle(app: &mut App, ui: &mut egui::Ui) {
    let Some(tool) = app
        .sketch
        .session
        .as_ref()
        .map(|session| session.tool)
        .filter(|tool| matches!(tool, SketchTool::Rectangle | SketchTool::CenterRectangle))
    else {
        return;
    };
    let mut centered = tool == SketchTool::CenterRectangle;
    ui.add_enabled_ui(crate::sketch_ui::tools_enabled(app), |ui| {
        egui::ComboBox::from_id_salt("rectangle-method")
            .selected_text(if centered {
                "Center rectangle"
            } else {
                "Corner rectangle"
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut centered, false, "Corner rectangle");
                ui.selectable_value(&mut centered, true, "Center rectangle");
            })
    });
    if centered != (tool == SketchTool::CenterRectangle) {
        let selected = if centered {
            SketchTool::CenterRectangle
        } else {
            SketchTool::Rectangle
        };
        app.sketch_dispatch(SketchAction::SetTool(selected));
        if app
            .sketch
            .session
            .as_ref()
            .is_some_and(|session| session.tool == selected)
        {
            app.ui.rectangle_centered = centered;
        }
    }
}

fn arc(app: &mut App, ui: &mut egui::Ui) {
    let Some(tool) = app
        .sketch
        .session
        .as_ref()
        .map(|session| session.tool)
        .filter(|tool| matches!(tool, SketchTool::Arc | SketchTool::ThreePointArc))
    else {
        return;
    };
    let mut three_point = tool == SketchTool::ThreePointArc;
    ui.add_enabled_ui(crate::sketch_ui::tools_enabled(app), |ui| {
        egui::ComboBox::from_id_salt("arc-method")
            .selected_text(if three_point {
                "Three-point arc"
            } else {
                "Center, start, end"
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut three_point, false, "Center, start, end");
                ui.selectable_value(&mut three_point, true, "Three-point arc");
            });
    });
    if three_point != (tool == SketchTool::ThreePointArc) {
        let selected = if three_point {
            SketchTool::ThreePointArc
        } else {
            SketchTool::Arc
        };
        app.sketch_dispatch(SketchAction::SetTool(selected));
        if app
            .sketch
            .session
            .as_ref()
            .is_some_and(|session| session.tool == selected)
        {
            app.ui.arc_three_point = three_point;
        }
    }
    ui.small(if three_point {
        "Click start, end, then a point on the arc."
    } else {
        "Click center, start, then end direction."
    });
}
