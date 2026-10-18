use super::{Handle, geometry::Geometry};
use crate::theme::Theme;
use egui::{Color32, Pos2, Shape, Stroke};

pub(super) fn draw(
    painter: &egui::Painter,
    geometry: &Geometry,
    active: Option<Handle>,
    theme: &Theme,
) {
    let axes = [theme.axis_x, theme.axis_y, theme.axis_z];
    let color = |handle, i: usize| {
        if active == Some(handle) {
            theme.accent
        } else {
            axes[i]
        }
    };
    for (i, points) in &geometry.rings {
        let handle = Handle::Ring(*i);
        painter.add(Shape::line(
            points.clone(),
            Stroke::new(
                if active == Some(handle) { 3.0 } else { 1.8 },
                color(handle, *i),
            ),
        ));
    }
    for (i, points) in &geometry.planes {
        let handle = Handle::Plane(*i);
        let c = color(handle, *i);
        painter.add(Shape::convex_polygon(
            points.clone(),
            c.gamma_multiply(if active == Some(handle) { 0.7 } else { 0.35 }),
            Stroke::new(1.0, c),
        ));
    }
    for (i, a, b) in &geometry.arrows {
        arrow(
            painter,
            *a,
            *b,
            color(Handle::Axis(*i), *i),
            active == Some(Handle::Axis(*i)),
        );
    }
    ball(painter, geometry, active, theme);
}

fn ball(painter: &egui::Painter, geometry: &Geometry, active: Option<Handle>, theme: &Theme) {
    if geometry.translate {
        let color = if active == Some(Handle::Ball) {
            theme.accent
        } else {
            theme.ink
        };
        painter.circle(
            geometry.ball,
            6.0,
            color,
            Stroke::new(1.5, theme.border_strong),
        );
    }
}
fn arrow(painter: &egui::Painter, a: Pos2, b: Pos2, color: Color32, hot: bool) {
    let v = b - a;
    let length = v.length();
    if length < 1.0 {
        return;
    }
    let u = v / length;
    let n = egui::vec2(-u.y, u.x);
    painter.line_segment([a, b], Stroke::new(if hot { 3.0 } else { 2.0 }, color));
    let head = 12.0_f32.min(length * 0.5);
    painter.add(Shape::convex_polygon(
        vec![
            b + u * 2.0,
            b - u * head + n * head * 0.4,
            b - u * head - n * head * 0.4,
        ],
        color,
        Stroke::NONE,
    ));
}
