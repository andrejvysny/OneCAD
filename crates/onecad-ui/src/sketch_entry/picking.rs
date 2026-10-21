use super::{App, Candidate, WorldPlane};
use onecad_scene::{DVec2, DVec3, ViewSnapshot};

pub(super) fn candidate_at(
    painter: &egui::Painter,
    rect: egui::Rect,
    view: &ViewSnapshot,
    display: &crate::bridge::Display,
    logical: Option<DVec2>,
    app: &App,
) -> Option<Candidate> {
    let mut closest = logical.and_then(|point| face_hit(view, display, point));
    let extent = (view.camera().offset.length() * 0.22).clamp(1.0, 1.0e6);
    for (name, plane, color) in [
        ("XY", WorldPlane::XY, app.theme.axis_z),
        ("XZ", WorldPlane::XZ, app.theme.axis_y),
        ("YZ", WorldPlane::YZ, app.theme.axis_x),
    ] {
        paint_patch(painter, rect, view, (name, plane, color), extent);
        if let Some(distance) = logical.and_then(|point| patch_distance(view, point, plane, extent))
            && closest.as_ref().is_none_or(|(depth, _)| distance < *depth)
        {
            closest = Some((distance, Candidate::Named(plane)));
        }
    }
    closest.map(|(_, candidate)| candidate)
}

fn face_hit(
    view: &ViewSnapshot,
    display: &crate::bridge::Display,
    point: DVec2,
) -> Option<(f64, Candidate)> {
    let hit = onecad_scene::pick_with_policy(
        &display.scene,
        view,
        view.identity(),
        point,
        onecad_scene::PickPolicy::FacesAndEdges,
    )
    .ok()??;
    (hit.kind == onecad_render::HighlightKind::Face).then(|| {
        (
            hit.ray_distance,
            Candidate::Face(hit.body_id, hit.topology_id.to_owned()),
        )
    })
}

fn paint_patch(
    painter: &egui::Painter,
    rect: egui::Rect,
    view: &ViewSnapshot,
    patch: (&str, WorldPlane, egui::Color32),
    extent: f64,
) {
    let (name, plane, color) = patch;
    let points: Option<Vec<_>> = [(0.15, 0.15), (1.0, 0.15), (1.0, 1.0), (0.15, 1.0)]
        .into_iter()
        .map(|(x, y)| {
            let point = plane
                .plane()
                .to_world(onecad_core::math::Vec2::new_unchecked(
                    x * extent,
                    y * extent,
                ));
            view.project(vector(point))
                .map(|point| rect.min + egui::vec2(point.logical.x as f32, point.logical.y as f32))
        })
        .collect();
    let Some(points) = points else {
        return;
    };
    let center = points
        .iter()
        .fold(egui::Vec2::ZERO, |sum, point| sum + point.to_vec2())
        / 4.0;
    painter.add(egui::Shape::convex_polygon(
        points,
        color.linear_multiply(0.12),
        egui::Stroke::new(1.0, color),
    ));
    painter.text(
        center.to_pos2(),
        egui::Align2::CENTER_CENTER,
        name,
        egui::FontId::proportional(14.0),
        color,
    );
}

pub(super) fn patch_distance(
    view: &ViewSnapshot,
    point: DVec2,
    plane: WorldPlane,
    extent: f64,
) -> Option<f64> {
    let ray = view.ray(point).ok()?;
    let frame = plane.plane();
    let denominator = ray.direction.dot(vector(frame.normal));
    if denominator.abs() < 1e-8 {
        return None;
    }
    let distance = (vector(frame.origin) - ray.origin).dot(vector(frame.normal)) / denominator;
    let world = ray.origin + ray.direction * distance;
    let local = frame.to_sketch(onecad_core::math::Vec3::new_unchecked(
        world.x, world.y, world.z,
    ));
    (distance > 0.0
        && (0.15 * extent..=extent).contains(&local.x)
        && (0.15 * extent..=extent).contains(&local.y))
    .then_some(distance)
}

fn vector(value: onecad_core::math::Vec3) -> DVec3 {
    DVec3::new(value.x, value.y, value.z)
}
