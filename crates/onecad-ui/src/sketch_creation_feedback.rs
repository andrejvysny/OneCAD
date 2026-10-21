//! Placement feedback uses the captured candidate and winning inference only.
use crate::theme::Theme;
use egui::{Color32, Painter, Pos2, Rect, Stroke};
use onecad_core::{math::Vec2, sketch::SketchPlane};
use onecad_interaction::sketch::{
    SketchDraft, SketchPhase, SketchSession,
    inference::{AcceptedSnap, SnapIntent, SnapKind},
};
use onecad_scene::{DVec3, ViewSnapshot};

fn project(rect: Rect, view: &ViewSnapshot, plane: SketchPlane, point: Vec2) -> Option<Pos2> {
    let world = plane.to_world(point);
    let screen = view.project(DVec3::new(world.x, world.y, world.z))?;
    let position = rect.min + egui::vec2(screen.logical.x as f32, screen.logical.y as f32);
    position.is_finite().then_some(position)
}

pub(super) fn candidate(
    painter: &Painter,
    rect: Rect,
    view: &ViewSnapshot,
    session: &SketchSession,
    point: Vec2,
    theme: &Theme,
) {
    if let Some(position) = project(rect, view, session.plane, point) {
        let painter = painter.with_clip_rect(rect);
        painter.circle_filled(position, 4.5, theme.surface);
        painter.circle_filled(position, 2.5, theme.ink3);
    }
}

pub(crate) fn anchors(
    painter: &Painter,
    rect: Rect,
    view: &ViewSnapshot,
    session: &SketchSession,
    theme: &Theme,
) {
    if !matches!(
        session.phase,
        SketchPhase::Active | SketchPhase::Submitting | SketchPhase::Accepted
    ) || session.document.scene != view.identity().scene
    {
        return;
    }
    if session.tool == onecad_interaction::sketch::SketchTool::Point {
        return;
    }
    let Some(draft) = &session.draft else { return };
    let painter = painter.with_clip_rect(rect);
    for point in anchor_points(draft) {
        if let Some(position) = project(rect, view, session.plane, point) {
            let marker = Rect::from_center_size(position, egui::vec2(7.0, 7.0));
            painter.rect_filled(marker, 1.0, theme.surface);
            painter.rect_stroke(
                marker,
                1.0,
                Stroke::new(2.0, theme.accent),
                egui::StrokeKind::Inside,
            );
        }
    }
}

fn anchor_points(draft: &SketchDraft) -> Vec<Vec2> {
    let mut points = vec![draft.anchor];
    if draft.first != draft.anchor {
        points.push(draft.first);
    }
    if let Some(second) = &draft.secondary
        && !points.contains(&second.point)
    {
        points.push(second.point);
    }
    points
}

pub(super) fn snap(
    painter: &Painter,
    rect: Rect,
    view: &ViewSnapshot,
    session: &SketchSession,
    snap: AcceptedSnap,
    theme: &Theme,
) {
    let Some(position) = project(rect, view, session.plane, snap.point) else {
        return;
    };
    if snap.source
        != crate::sketch_snapping::snap_source(
            &session.authoritative,
            view,
            session.request.sequence,
        )
    {
        return;
    }
    let painter = painter.with_clip_rect(rect);
    let color = theme.sketch_inference;
    let guide = Stroke::new(1.25, color);
    let anchor_ray = snap.intent == SnapIntent::GeometryOnly;
    for source in guide_sources(session, &snap) {
        let Some(start) = project(rect, view, session.plane, source) else {
            continue;
        };
        if anchor_ray {
            if let Some(line) = ray_to_rect(start, position, rect) {
                painter.line_segment(line, guide);
            }
        } else {
            painter.line_segment([start, position], guide);
            painter.circle_filled(start, 3.0, color);
        }
    }
    painter.circle_filled(position, 10.0, color.gamma_multiply(0.35));
    if matches!(
        snap.kind,
        SnapKind::Endpoint
            | SnapKind::Midpoint
            | SnapKind::Center
            | SnapKind::Origin
            | SnapKind::Quadrant
            | SnapKind::Intersection
    ) {
        marker(&painter, position, snap.kind, color);
    }
    painter.circle_stroke(position, 4.0, Stroke::new(1.5, Color32::WHITE));
    painter.circle_filled(position, 2.5, color);
}

/// Ray from `origin` through `through`, clipped to the viewport rect edge.
fn ray_to_rect(origin: Pos2, through: Pos2, rect: Rect) -> Option<[Pos2; 2]> {
    let dir = through - origin;
    if dir.length() < 1.0 || !rect.contains(origin) {
        return None;
    }
    let axis = |o: f32, d: f32, lo: f32, hi: f32| {
        if d > 0.0 {
            (hi - o) / d
        } else if d < 0.0 {
            (lo - o) / d
        } else {
            f32::INFINITY
        }
    };
    let t = axis(origin.x, dir.x, rect.min.x, rect.max.x)
        .min(axis(origin.y, dir.y, rect.min.y, rect.max.y));
    t.is_finite().then(|| [origin, origin + dir * t.max(1.0)])
}

fn guide_sources(session: &SketchSession, snap: &AcceptedSnap) -> Vec<Vec2> {
    match snap.intent {
        SnapIntent::Guide { reference, .. } => reference
            .point_coordinates(&session.authoritative)
            .into_iter()
            .collect(),
        SnapIntent::Guides {
            horizontal,
            vertical,
        } => [horizontal, vertical]
            .into_iter()
            .filter_map(|reference| reference.point_coordinates(&session.authoritative))
            .collect(),
        SnapIntent::Quadrant { center, .. } => center
            .point_coordinates(&session.authoritative)
            .into_iter()
            .collect(),
        SnapIntent::GeometryOnly
            if matches!(
                snap.kind,
                SnapKind::HorizontalGuide | SnapKind::VerticalGuide | SnapKind::Polar
            ) =>
        {
            session
                .draft
                .as_ref()
                .map(|draft| draft.anchor)
                .into_iter()
                .collect()
        }
        _ => vec![],
    }
}

fn marker(painter: &Painter, at: Pos2, kind: SnapKind, color: Color32) {
    let stroke = Stroke::new(1.25, color);
    let rect = Rect::from_center_size(at, egui::vec2(8.4, 8.4));
    match kind {
        SnapKind::Endpoint => {
            painter.rect_stroke(rect, 0.0, stroke, egui::StrokeKind::Inside);
        }
        SnapKind::Midpoint => polygon(painter, at, &[(0.0, -4.9), (4.2, 3.5), (-4.2, 3.5)], stroke),
        SnapKind::Quadrant => polygon(
            painter,
            at,
            &[(0.0, -4.9), (4.9, 0.0), (0.0, 4.9), (-4.9, 0.0)],
            stroke,
        ),
        SnapKind::Intersection => {
            for sign in [-1.0, 1.0] {
                painter.line_segment(
                    [
                        at + egui::vec2(-3.5, -3.5 * sign),
                        at + egui::vec2(3.5, 3.5 * sign),
                    ],
                    stroke,
                );
            }
        }
        SnapKind::Center | SnapKind::Origin => {
            painter.circle_stroke(at, 4.2, stroke);
            painter.line_segment(
                [at - egui::vec2(5.6, 0.0), at + egui::vec2(5.6, 0.0)],
                stroke,
            );
            painter.line_segment(
                [at - egui::vec2(0.0, 5.6), at + egui::vec2(0.0, 5.6)],
                stroke,
            );
        }
        SnapKind::OnCurve => {
            painter.circle_stroke(at, 3.5, stroke);
        }
        _ => {
            painter.circle_stroke(at, 4.2, stroke);
        }
    }
}

fn polygon(painter: &Painter, at: Pos2, points: &[(f32, f32)], stroke: Stroke) {
    painter.add(egui::Shape::closed_line(
        points
            .iter()
            .map(|(x, y)| at + egui::vec2(*x, *y))
            .collect(),
        stroke,
    ));
}

#[cfg(test)]
#[path = "sketch_creation/feedback_tests.rs"]
mod tests;
