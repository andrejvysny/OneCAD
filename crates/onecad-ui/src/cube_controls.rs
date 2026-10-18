//! Navigation control layout adapted from SolidCraft's navigation cube.
use super::*;

fn button(
    ui: &mut egui::Ui,
    bounds: Rect,
    id: &str,
    label: &str,
    action: CubeAction,
    output: &mut CubeResponse,
) -> bool {
    let response = ui
        .interact(
            bounds,
            ui.scope_id().with(("cube control", id)),
            Sense::click(),
        )
        .on_hover_text(label);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    if response.clicked() {
        output.actions.push(action);
    }
    output.owns_pointer |= response.hovered() || response.is_pointer_button_down_on();
    response.hovered()
}

type OrbitChoice = (&'static str, egui::Vec2, egui::Vec2, DVec3, f64);

fn orbit_choices(basis: CameraBasis) -> [OrbitChoice; 4] {
    let quarter = std::f64::consts::FRAC_PI_2;
    let square_on = [basis.back.x, basis.back.y, basis.back.z]
        .into_iter()
        .filter(|v| v.abs() > 1e-5)
        .count()
        == 1;
    let distance = if square_on { 54.0 } else { 74.0 };
    [
        (
            "left",
            vec2(-distance, 0.0),
            vec2(1.0, 0.0),
            basis.up,
            -quarter,
        ),
        (
            "right",
            vec2(distance, 0.0),
            vec2(-1.0, 0.0),
            basis.up,
            quarter,
        ),
        (
            "up",
            vec2(0.0, -distance),
            vec2(0.0, 1.0),
            basis.right,
            -quarter,
        ),
        (
            "down",
            vec2(0.0, distance),
            vec2(0.0, -1.0),
            basis.right,
            quarter,
        ),
    ]
}

fn orbit_controls(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    center: Pos2,
    scale: f32,
    view: &ViewSnapshot,
    palette: CubePalette,
    output: &mut CubeResponse,
) {
    let size = if view.basis().back.abs().max_element() > 1.0 - 1e-5 {
        20.0
    } else {
        16.0
    };
    for (name, offset, direction, axis, radians) in orbit_choices(view.basis()) {
        let at = center + offset * scale;
        let label = format!("Orbit {name} 90°");
        let hot = button(
            ui,
            Rect::from_center_size(at, vec2(size, size) * scale),
            name,
            &label,
            CubeAction::QuarterOrbit { axis, radians },
            output,
        );
        painter::arrow(painter, at, direction * scale, hot, palette);
    }
}

fn roll_controls(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    center: Pos2,
    scale: f32,
    palette: CubePalette,
    output: &mut CubeResponse,
) {
    for (name, offset, size, clockwise) in [
        (
            "counterclockwise",
            vec2(42.0, -72.0),
            vec2(40.0, 20.0),
            false,
        ),
        ("clockwise", vec2(72.0, -42.0), vec2(20.0, 40.0), true),
    ] {
        let sign = if clockwise { 1.0 } else { -1.0 };
        let label = format!("Rotate view {name} 90°");
        let hot = button(
            ui,
            Rect::from_center_size(center + offset * scale, size * scale),
            name,
            &label,
            CubeAction::Roll(sign * std::f64::consts::FRAC_PI_2),
            output,
        );
        painter::curved_arrow(
            painter,
            center + vec2(42.0, -42.0) * scale,
            clockwise,
            hot,
            palette,
            scale,
        );
    }
}

fn home_fit_projection(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    center: Pos2,
    scale: f32,
    palette: CubePalette,
    output: &mut CubeResponse,
) {
    let home = Rect::from_center_size(
        center + vec2(-65.0, -65.0) * scale,
        vec2(26.0, 26.0) * scale,
    );
    let hot = button(
        ui,
        home,
        "home",
        "Home: isometric view and fit model",
        CubeAction::Home,
        output,
    );
    // OneCAD owns the home icon; navigation geometry and layout come from the donor.
    crate::icons::paint(
        painter,
        home.shrink(4.0 * scale),
        "home",
        if hot {
            palette.accent
        } else {
            palette.cube_text
        },
        palette.accent,
        1.5,
    );
}

fn fit_projection(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    center: Pos2,
    scale: f32,
    palette: CubePalette,
    output: &mut CubeResponse,
) {
    let fit = Rect::from_center_size(center + vec2(67.0, 67.0) * scale, vec2(22.0, 22.0) * scale);
    let hot = button(ui, fit, "fit", "Fit model in view", CubeAction::Fit, output);
    painter.circle(
        fit.center(),
        6.0 * scale,
        if hot {
            palette.accent_soft
        } else {
            palette.cube_face
        },
        Stroke::new(1.0, palette.cube_border),
    );
    let projection =
        Rect::from_center_size(center + vec2(0.0, 72.0) * scale, vec2(48.0, 20.0) * scale);
    let hot = button(
        ui,
        projection,
        "projection",
        "Toggle perspective / orthographic projection",
        CubeAction::ToggleProjection,
        output,
    );
    painter.text(
        projection.center(),
        Align2::CENTER_CENTER,
        "3D / 2D",
        FontId::proportional(10.0 * scale),
        if hot {
            palette.accent
        } else {
            palette.cube_text
        },
    );
}

pub(super) fn show(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    center: Pos2,
    scale: f32,
    view: &ViewSnapshot,
    palette: CubePalette,
    output: &mut CubeResponse,
) {
    orbit_controls(ui, painter, center, scale, view, palette, output);
    roll_controls(ui, painter, center, scale, palette, output);
    home_fit_projection(ui, painter, center, scale, palette, output);
    fit_projection(ui, painter, center, scale, palette, output);
}
