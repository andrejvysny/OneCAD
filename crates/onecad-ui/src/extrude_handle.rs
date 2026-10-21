//! Viewport arrow that drags a new Extrude's distance through the same typed draft.
use crate::{
    app::App,
    gizmo::{along, arrow, project},
};
use egui::{Pos2, Rect, Stroke};
use onecad_core::{document::record::ExtrudeParams, ids::RecordId};
use onecad_interaction::{Action, DraftStatus, ModelSession, NumericParameter};
use onecad_scene::{DVec3, ViewSnapshot};

const DOT_PX: f32 = 9.0;
const SEGMENT_PX: f32 = 6.0;
const NEAR_TIP_PX: f32 = 14.0;
/// Smallest magnitude the two-decimal draft text can still express.
const MIN_DISTANCE: f64 = 0.01;
const DEFAULT_DISTANCE: f64 = 10.0;

#[derive(Default)]
pub(crate) struct ExtrudeHandle {
    record: Option<RecordId>,
    last_valid: Option<f64>,
    drag: Option<Drag>,
}

struct Drag {
    t0: f64,
    d0: f64,
    d: f64,
    original: String,
    sent: String,
}

impl ExtrudeHandle {
    pub(crate) fn active(&self) -> bool {
        self.drag.is_some()
    }
}

struct Target {
    record: RecordId,
    base: DVec3,
    normal: DVec3,
    evaluated: Option<f64>,
    fallback: f64,
    two_directions: bool,
    draft: String,
}

pub(crate) fn distance_params(tool: &ModelSession) -> Option<&ExtrudeParams> {
    let params = crate::extrude_ui::params(tool)?;
    (tool.parameter == NumericParameter::ExtrudeDistance).then_some(params)
}

fn target(app: &App) -> Option<Target> {
    if !app.viewport_ready()
        || !app.doc.bridge.valid()
        || app.sketch_active()
        || crate::viewport_selection::sectioned(app)
    {
        return None;
    }
    let tool = app.doc.interaction.tool.as_ref()?;
    let params = distance_params(tool)?;
    let wanted = params.profile.as_ref()?;
    let profile = app
        .doc
        .display
        .as_ref()?
        .profiles
        .iter()
        .find(|p| p.reference.sketch == wanted.sketch && p.reference.region == wanted.region)?;
    let normal = profile.plane().normal;
    let normal = DVec3::new(normal.x, normal.y, normal.z).try_normalize()?;
    let base = profile.centroid();
    let fallback = if params.distance.value.is_finite() && params.distance.value != 0.0 {
        params.distance.value
    } else {
        DEFAULT_DISTANCE
    };
    (base.is_finite()).then(|| Target {
        record: tool.tag.record_id,
        base,
        normal,
        evaluated: match &tool.draft_status {
            DraftStatus::Valid(scalar) if scalar.value.is_finite() => Some(scalar.value),
            _ => None,
        },
        fallback,
        two_directions: params.two_directions,
        draft: tool.draft.clone(),
    })
}

/// Screen endpoints of the stem; the tip carries the grab dot.
fn points(view: &ViewSnapshot, rect: Rect, target: &Target, distance: f64) -> Option<(Pos2, Pos2)> {
    Some((
        project(view, rect, target.base)?,
        project(view, rect, target.base + target.normal * distance)?,
    ))
}

pub(crate) fn hit(pointer: Pos2, base: Pos2, tip: Pos2) -> bool {
    if pointer.distance(tip) <= DOT_PX {
        return true;
    }
    let stem = base - tip;
    let length = stem.length();
    if length < 1.0 {
        return false;
    }
    let end = tip + stem / length * length.min(NEAR_TIP_PX);
    let along = end - tip;
    let t = ((pointer - tip).dot(along) / along.length_sq().max(1e-6)).clamp(0.0, 1.0);
    (tip + along * t).distance(pointer) <= SEGMENT_PX
}

/// Dragged distance: snapped unless free, never zero, positive for two-direction legs.
pub(crate) fn next_distance(
    d0: f64,
    delta: f64,
    previous: f64,
    step: f64,
    free: bool,
    two_directions: bool,
) -> f64 {
    let raw = d0 + delta;
    if !raw.is_finite() {
        return previous;
    }
    let step = if step.is_finite() && step > 0.0 {
        step
    } else {
        1.0
    };
    let value = if free {
        raw
    } else {
        (raw / step).round() * step
    };
    let floor = if free {
        MIN_DISTANCE
    } else {
        step.max(MIN_DISTANCE)
    };
    if two_directions {
        return value.max(floor);
    }
    if value.abs() < MIN_DISTANCE * 0.5 {
        return if previous < 0.0 { -floor } else { floor };
    }
    value
}

/// A pointer claims the handle only where the drag axis is usable (view not parallel to it).
fn claims(view: &ViewSnapshot, rect: Rect, target: &Target, distance: f64, pointer: Pos2) -> bool {
    rect.contains(pointer)
        && points(view, rect, target, distance).is_some_and(|(b, tip)| hit(pointer, b, tip))
        && along(view, rect, pointer, target.base, target.normal).is_some()
}

pub(crate) fn owns_pointer(app: &App, view: &ViewSnapshot, rect: Rect, pointer: Pos2) -> bool {
    app.doc.extrude_handle.active()
        || target(app).is_some_and(|t| {
            let d = app
                .doc
                .extrude_handle
                .last_valid
                .or(t.evaluated)
                .unwrap_or(t.fallback);
            claims(view, rect, &t, d, pointer)
        })
}

/// Ends or cancels a capture whose release/focus events never reached `show`
/// (viewport early returns). Runs before the viewport is drawn.
pub(crate) fn settle(app: &mut App, ctx: &egui::Context) {
    if !app.doc.extrude_handle.active() {
        return;
    }
    let (focused, down) = ctx.input(|i| (i.focused, i.pointer.primary_down()));
    let drag = app.doc.extrude_handle.drag.take();
    if !focused || app.doc.interaction.tool.is_none() {
        cancel(app, drag);
    } else if !down {
        app.doc.extrude_handle.last_valid = drag.map(|d| d.d);
    } else {
        app.doc.extrude_handle.drag = drag;
    }
}

pub(crate) fn show(app: &mut App, ui: &mut egui::Ui, view: &ViewSnapshot, rect: Rect) -> bool {
    let Some(target) = target(app) else {
        let drag = app.doc.extrude_handle.drag.take();
        cancel(app, drag);
        app.doc.extrude_handle = ExtrudeHandle::default();
        return false;
    };
    if app.doc.extrude_handle.record != Some(target.record) {
        app.doc.extrude_handle = ExtrudeHandle {
            record: Some(target.record),
            ..Default::default()
        };
    }
    if app.doc.extrude_handle.drag.is_none()
        && let Some(value) = target.evaluated
    {
        app.doc.extrude_handle.last_valid = Some(value);
    }
    let mut drag = app.doc.extrude_handle.drag.take();
    let shown = drag
        .as_ref()
        .map(|drag| drag.d)
        .or(app.doc.extrude_handle.last_valid)
        .unwrap_or(target.fallback);
    let Some((base_px, tip_px)) = points(view, rect, &target, shown) else {
        cancel(app, drag);
        return false;
    };
    let (pointer, events, modifiers) =
        ui.input(|i| (i.pointer.hover_pos(), i.events.clone(), i.modifiers));
    if !crate::viewport_widgets::gesture_allowed(ui) {
        cancel(app, drag);
        return false;
    }
    let hovered = pointer.is_some_and(|p| {
        crate::viewport_widgets::allows_pointer(ui, rect, p)
            && claims(view, rect, &target, shown, p)
    });
    let step = crate::transform_ui::snap_step(view.world_per_pixel(view.depth(target.base)));
    let mut released = false;
    for event in events {
        match event {
            egui::Event::PointerGone | egui::Event::WindowFocused(false) => {
                cancel(app, drag.take());
            }
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                ..
            } if drag.is_none()
                && crate::viewport_widgets::allows_pointer(ui, rect, pos)
                && claims(view, rect, &target, shown, pos) =>
            {
                drag = along(view, rect, pos, target.base, target.normal).map(|t0| Drag {
                    t0,
                    d0: shown,
                    d: shown,
                    original: target.draft.clone(),
                    sent: target.draft.clone(),
                });
            }
            egui::Event::PointerMoved(pos)
            | egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                ..
            } if drag.is_some() => {
                if let Some(active) = drag.as_mut() {
                    update(view, rect, &target, active, pos, step, modifiers.alt);
                }
                released |= matches!(event, egui::Event::PointerButton { .. });
            }
            _ => {}
        }
    }
    // One draft change per frame keeps the evaluate/preview pipeline from flooding.
    if let Some(active) = drag.as_mut() {
        flush(app, active);
    }
    if released {
        app.doc.extrude_handle.last_valid = drag.take().map(|d| d.d);
    }
    let dragging = drag.is_some();
    app.doc.extrude_handle.drag = drag;
    let hot = hovered || dragging;
    let shown = app
        .doc
        .extrude_handle
        .drag
        .as_ref()
        .map_or(shown, |drag| drag.d);
    let (base_px, tip_px) = points(view, rect, &target, shown).unwrap_or((base_px, tip_px));
    paint(ui, rect, &app.theme, base_px, tip_px, hot);
    if dragging {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        ui.ctx().request_repaint();
    } else if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
    hot
}

fn update(
    view: &ViewSnapshot,
    rect: Rect,
    target: &Target,
    drag: &mut Drag,
    pos: Pos2,
    step: f64,
    free: bool,
) {
    let Some(t) = along(view, rect, pos, target.base, target.normal) else {
        return;
    };
    // A negative start with two directions (invalid anyway) clamps to the floor on first move.
    drag.d = next_distance(
        drag.d0,
        t - drag.t0,
        drag.d,
        step,
        free,
        target.two_directions,
    );
}

impl Drag {
    fn pending(&self) -> Option<String> {
        let text = crate::sketch_numeric::format::format_measure(self.d);
        (text != self.sent).then_some(text)
    }
    fn restore(&self) -> Option<String> {
        (self.sent != self.original).then(|| self.original.clone())
    }
}

fn flush(app: &mut App, drag: &mut Drag) {
    if let Some(text) = drag.pending() {
        drag.sent = text.clone();
        app.dispatch(Action::DraftChanged(text));
        app.draw_dirty = true;
    }
}

fn cancel(app: &mut App, drag: Option<Drag>) {
    app.doc.extrude_handle.drag = None;
    if let Some(text) = drag.and_then(|d| d.restore())
        && app.doc.interaction.tool.is_some()
    {
        app.dispatch(Action::DraftChanged(text));
        app.draw_dirty = true;
    }
}

fn paint(ui: &egui::Ui, rect: Rect, theme: &crate::theme::Theme, base: Pos2, tip: Pos2, hot: bool) {
    let color = if hot {
        theme.accent.lerp_to_gamma(egui::Color32::WHITE, 0.35)
    } else {
        theme.accent
    };
    let painter = ui.painter().with_clip_rect(rect);
    arrow(&painter, base, tip, color, hot);
    painter.circle(
        tip,
        if hot { 8.0 } else { 6.0 },
        color,
        Stroke::new(1.5, theme.border_strong),
    );
}

#[cfg(test)]
#[path = "extrude_handle_tests.rs"]
mod tests;
