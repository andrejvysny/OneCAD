//! Placement feedback uses the captured candidate and winning inference only.
use crate::theme::Theme;
use egui::{Align2, FontId, Painter, Pos2, Rect, Stroke};
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
        painter.circle_filled(position, 3.0, theme.accent);
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
    for source in guide_sources(session, &snap) {
        if let Some(start) = project(rect, view, session.plane, source) {
            painter.line_segment(
                [start, position],
                Stroke::new(1.0, theme.accent.gamma_multiply(0.8)),
            );
            painter.circle_stroke(start, 3.0, Stroke::new(1.0, theme.accent));
        }
    }
    marker(&painter, position, snap.kind, theme);
    let label = if snap.intent == SnapIntent::GeometryOnly {
        format!("{} guide", snap.kind.label())
    } else {
        format!("{} constraint", snap.kind.label())
    };
    painter.text(
        position + egui::vec2(10.0, -12.0),
        Align2::LEFT_BOTTOM,
        label,
        FontId::proportional(12.0),
        theme.accent,
    );
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

fn marker(painter: &Painter, at: Pos2, kind: SnapKind, theme: &Theme) {
    let stroke = Stroke::new(1.5, theme.accent);
    let rect = Rect::from_center_size(at, egui::vec2(12.0, 12.0));
    match kind {
        SnapKind::Endpoint => {
            painter.rect_stroke(rect, 0.0, stroke, egui::StrokeKind::Inside);
        }
        SnapKind::Midpoint => polygon(painter, at, &[(0.0, -7.0), (6.0, 5.0), (-6.0, 5.0)], stroke),
        SnapKind::Quadrant => polygon(
            painter,
            at,
            &[(0.0, -7.0), (7.0, 0.0), (0.0, 7.0), (-7.0, 0.0)],
            stroke,
        ),
        SnapKind::Intersection => {
            for sign in [-1.0, 1.0] {
                painter.line_segment(
                    [
                        at + egui::vec2(-5.0, -5.0 * sign),
                        at + egui::vec2(5.0, 5.0 * sign),
                    ],
                    stroke,
                );
            }
        }
        SnapKind::Center | SnapKind::Origin => {
            painter.circle_stroke(at, 6.0, stroke);
            painter.line_segment(
                [at - egui::vec2(8.0, 0.0), at + egui::vec2(8.0, 0.0)],
                stroke,
            );
            painter.line_segment(
                [at - egui::vec2(0.0, 8.0), at + egui::vec2(0.0, 8.0)],
                stroke,
            );
        }
        SnapKind::OnCurve => {
            painter.circle_stroke(at, 5.0, stroke);
        }
        _ => {
            painter.circle_stroke(at, 6.0, stroke);
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
