//! Sketch geometry painting: per-entity state styling, hover preselect, drafts.
use super::*;
use onecad_app::dto::EntityConstrainedState;
use onecad_interaction::sketch::{SketchTarget, SketchTool};

const WIDTH_NORMAL: f32 = 1.5;
const WIDTH_SELECTED: f32 = 2.5;
const WIDTH_HALO: f32 = 4.0;
const HALO_ALPHA: f32 = 0.6;

/// Color of one entity. Selection beats every state; construction and locked
/// reference geometry beat the solver state; unknown state is neutral ink.
pub(super) fn entity_color(
    theme: &Theme,
    entity: &SketchEntity,
    selected: bool,
    state: Option<EntityConstrainedState>,
) -> Color32 {
    if selected {
        theme.accent
    } else if entity.is_reference_locked() {
        theme.ink5.gamma_multiply(0.7)
    } else if entity.is_construction() {
        theme.ink5
    } else {
        match state {
            Some(EntityConstrainedState::FullyConstrained) => theme.dof_ok,
            Some(EntityConstrainedState::UnderConstrained) => theme.sketch_underconstrained,
            Some(EntityConstrainedState::Conflicting) => theme.danger_strong,
            None => theme.sketch_neutral,
        }
    }
}

/// Pure hover resolution for the Select tool. The caller owns pointer
/// ownership checks (chrome layers, active gestures, transient previews).
pub(crate) fn hover_target(
    rect: Rect,
    view: &ViewSnapshot,
    session: &SketchSession,
    pointer: Option<Pos2>,
) -> Option<SketchTarget> {
    if session.tool != SketchTool::Select || session.draft.is_some() || session.selection_dragging {
        return None;
    }
    match pick_entity(rect, view, session, pointer?, DEFAULT_PICK_RADIUS) {
        PickResult::Unique(target) => Some(target),
        PickResult::None | PickResult::Ambiguous(_) => None,
    }
}

fn paint_entity_path(
    painter: &Painter,
    rect: Rect,
    view: &ViewSnapshot,
    plane: SketchPlane,
    points: &[Vec2],
    stroke: Stroke,
    dash: Option<(f32, f32)>,
) {
    let Some((dash, gap)) = dash else {
        paint_path(painter, rect, view, plane, points, stroke);
        return;
    };
    let screen: Vec<Pos2> = points
        .iter()
        .filter_map(|point| project(rect, view, plane, *point))
        .collect();
    painter.extend(Shape::dashed_line(&screen, stroke, dash, gap));
}

pub fn paint(
    painter: &Painter,
    rect: Rect,
    view: &ViewSnapshot,
    session: &SketchSession,
    preview: Option<&Sketch>,
    hover: Option<SketchTarget>,
    theme: &Theme,
) {
    if session.document.scene != view.identity().scene
        || !matches_rect(rect, view)
        || matches!(
            session.phase,
            SketchPhase::Closed | SketchPhase::NeedsRefresh
        )
    {
        return;
    }
    let geometry = preview.unwrap_or(&session.authoritative);
    let painter = painter.with_clip_rect(rect);
    let halo = Stroke::new(WIDTH_HALO, theme.accent.gamma_multiply(HALO_ALPHA));
    for entity in geometry.entities() {
        let selected = session
            .selection
            .contains(&SketchTarget::Entity(entity.id()));
        // Drag previews keep the last solved state; unknown ids fall back to neutral.
        let state = crate::sketch_feedback::entity(&session.status, entity.id());
        let color = entity_color(theme, entity, selected, state);
        if let SketchEntity::Point { at, .. } = entity {
            if let Some(screen) = project(rect, view, session.plane, *at) {
                painter.circle_filled(screen, 2.5, color);
            }
        } else if let Some(points) = entity_points(geometry, entity, rect, view, session.plane) {
            let dash = entity.is_construction().then_some((5.0, 4.0));
            if hover == Some(SketchTarget::Entity(entity.id())) {
                paint_entity_path(&painter, rect, view, session.plane, &points, halo, dash);
            }
            let width = if selected {
                WIDTH_SELECTED
            } else {
                WIDTH_NORMAL
            };
            let stroke = Stroke::new(width, color);
            paint_entity_path(&painter, rect, view, session.plane, &points, stroke, dash);
        }
    }
    paint_constraint_hover(&painter, rect, view, session, geometry, theme);
    if let Some(target) = hover
        && let Some(at) = target.point_coordinates(geometry)
        && let Some(screen) = project(rect, view, session.plane, at)
    {
        painter.circle_filled(screen, 4.0, theme.accent.gamma_multiply(HALO_ALPHA));
        painter.circle_stroke(
            screen,
            6.0,
            Stroke::new(1.0, theme.accent.gamma_multiply(HALO_ALPHA)),
        );
    }
    for target in &session.selection {
        if let Some(at) = target.point_coordinates(geometry)
            && let Some(screen) = project(rect, view, session.plane, at)
        {
            painter.circle_filled(screen, 4.0, theme.accent);
            painter.circle_stroke(screen, 6.0, Stroke::new(1.0, theme.accent));
        }
    }
    crate::sketch_creation::paint_anchors(&painter, rect, view, session, theme);
    if let Some(draft) = &session.preview
        && !matches!(draft, DraftPrimitive::Point { .. })
    {
        paint_draft(&painter, rect, view, session.plane, draft, theme);
    }
}

pub(crate) fn paint_draft(
    painter: &Painter,
    rect: Rect,
    view: &ViewSnapshot,
    plane: SketchPlane,
    draft: &DraftPrimitive,
    theme: &Theme,
) {
    let points = draft_points(draft, rect, view, plane);
    if let [point] = points.as_slice()
        && let Some(screen) = project(rect, view, plane, *point)
    {
        painter.circle_stroke(screen, 4.0, Stroke::new(1.5, theme.accent));
    }
    let stroke = Stroke::new(WIDTH_NORMAL, theme.accent);
    paint_entity_path(
        painter,
        rect,
        view,
        plane,
        &points,
        stroke,
        Some((6.0, 4.0)),
    );
}

fn paint_constraint_hover(
    painter: &Painter,
    rect: Rect,
    view: &ViewSnapshot,
    session: &SketchSession,
    geometry: &Sketch,
    theme: &Theme,
) {
    let color = theme.accent.gamma_multiply(0.55);
    for target in crate::sketch_repair::hover_targets(painter.ctx(), session) {
        if let Some(at) = target.point_coordinates(geometry) {
            if let Some(screen) = project(rect, view, session.plane, at) {
                painter.circle_stroke(screen, 7.0, Stroke::new(2.0, color));
            }
        } else if let SketchTarget::Entity(id) = target
            && let Some(entity) = geometry.get_entity(id)
        {
            if let SketchEntity::Point { at, .. } = entity {
                if let Some(screen) = project(rect, view, session.plane, *at) {
                    painter.circle_stroke(screen, 7.0, Stroke::new(2.0, color));
                }
            } else if let Some(points) = entity_points(geometry, entity, rect, view, session.plane)
            {
                paint_path(
                    painter,
                    rect,
                    view,
                    session.plane,
                    &points,
                    Stroke::new(WIDTH_NORMAL, color),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests;
