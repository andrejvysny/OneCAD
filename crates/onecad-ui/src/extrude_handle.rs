//! Viewport arrow that drags a new Extrude's distance through the shared
//! parameter-drag lifecycle; the reducer owns and restores the authored text.
use crate::{
    app::App,
    gizmo::{along, arrow, project},
    parameter_drag::{Lifecycle, Outcome},
};
use egui::{Pos2, Rect, Stroke};
use onecad_core::{document::record::ExtrudeParams, ids::RecordId};
use onecad_interaction::{Action, DragPhase, ModelSession, NumericParameter, ParameterKey};
use onecad_scene::{Camera, DVec3, ViewSnapshot, Viewport};

const DOT_PX: f32 = 9.0;
const SEGMENT_PX: f32 = 6.0;
const NEAR_TIP_PX: f32 = 14.0;
/// Smallest magnitude the two-decimal draft text can still express.
const MIN_DISTANCE: f64 = 0.01;
const DEFAULT_DISTANCE: f64 = 10.0;
const DISTANCE: ParameterKey = ParameterKey::Scalar(NumericParameter::ExtrudeDistance);

#[derive(Default)]
pub(crate) struct ExtrudeHandle {
    record: Option<RecordId>,
    last_valid: Option<f64>,
    drag: Option<Drag>,
}

struct Drag {
    press: Pos2,
    t0: f64,
    d0: f64,
    d: f64,
    /// Shown value at press; the anchor `d0` moves at limits.
    start: f64,
    /// Reducer field at press: authored text and its evaluated value.
    original: String,
    evaluated: Option<f64>,
    /// Last text authored by this drag.
    sent: Option<String>,
    /// Pointer travel left the threshold box; nothing is authored before this.
    accepted: bool,
    free: bool,
    /// View at press; a camera or viewport change ends the drag.
    view: (Camera, Viewport, Rect),
    lifecycle: Lifecycle,
}

/// Per-drag constants for feeding events.
struct Rule {
    step: f64,
    two_directions: bool,
}

#[derive(Debug, PartialEq)]
enum Fed {
    Tracking,
    Released,
    Lost,
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
    tool.params
        .field(DISTANCE)
        .is_some_and(|field| field.active)
        .then_some(params)
}

/// The distance handle is drawn for the open tool.
pub(crate) fn available(app: &App) -> bool {
    target(app).is_some()
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
    let display = app.doc.display.as_ref()?;
    let (base, normal) = display
        .profiles
        .iter()
        .find(|p| p.reference.sketch == wanted.sketch && p.reference.region == wanted.region)
        .map(|p| {
            let normal = p.plane().normal;
            (p.centroid(), DVec3::new(normal.x, normal.y, normal.z))
        })
        .or_else(|| {
            display
                .profile_anchors
                .iter()
                .find(|a| {
                    a.reference.sketch == wanted.sketch && a.reference.region == wanted.region
                })
                .map(|a| (a.base, a.normal))
        })?;
    let normal = normal.try_normalize()?;
    let fallback = if params.distance.value.is_finite() && params.distance.value != 0.0 {
        params.distance.value
    } else {
        DEFAULT_DISTANCE
    };
    let (evaluated, draft) = authored(tool);
    base.is_finite().then_some(Target {
        record: tool.tag.record_id,
        base,
        normal,
        evaluated,
        fallback,
        two_directions: params.two_directions,
        draft,
    })
}

/// The reducer's current distance text and value; read again after every
/// terminal outcome, so a later press starts from what the reducer holds.
fn authored(tool: &ModelSession) -> (Option<f64>, String) {
    (
        tool.value(DISTANCE).filter(|value| value.is_finite()),
        tool.params
            .field(DISTANCE)
            .map(|field| field.text.clone())
            .unwrap_or_default(),
    )
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

/// Dragged distance: snapped unless free, never zero, positive for two-direction
/// legs. The flag reports a clamp, so the caller can re-anchor at the limit.
pub(crate) fn next_distance(
    d0: f64,
    delta: f64,
    previous: f64,
    step: f64,
    free: bool,
    two_directions: bool,
) -> (f64, bool) {
    let raw = d0 + delta;
    if !raw.is_finite() {
        return (previous, false);
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
        return if value < floor {
            (floor, true)
        } else {
            (value, false)
        };
    }
    if value.abs() < MIN_DISTANCE * 0.5 {
        return (if previous < 0.0 { -floor } else { floor }, true);
    }
    (value, false)
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

/// Ends a capture whose release or loss never reached `show` (viewport early
/// returns). Runs after the viewport, so an in-batch release that `show`
/// consumed has already ended the drag and cannot be preempted.
pub(crate) fn settle(app: &mut App, ctx: &egui::Context) {
    if !app.doc.extrude_handle.active() {
        return;
    }
    let (focused, down) = ctx.input(|i| (i.focused, i.pointer.primary_down()));
    if !focused || app.doc.interaction.tool.is_none() {
        abandon(app, Outcome::Lost);
    } else if !down {
        let drag = app.doc.extrude_handle.drag.take();
        finish(app, drag);
    }
}

/// Ends any handle drag and restores the authored draft exactly, for callers
/// that take the pointer from the document (Escape, tab deactivation).
pub(crate) fn abandon(app: &mut App, outcome: Outcome) {
    let drag = app.doc.extrude_handle.drag.take();
    end(app, drag, outcome);
}

pub(crate) fn show(app: &mut App, ui: &mut egui::Ui, view: &ViewSnapshot, rect: Rect) -> bool {
    let Some(mut target) = target(app) else {
        abandon(app, Outcome::Lost);
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
    let mut shown = drag
        .as_ref()
        .map(|drag| drag.d)
        .or(app.doc.extrude_handle.last_valid)
        .unwrap_or(target.fallback);
    let frame = (view.camera(), view.viewport(), rect);
    // A camera or viewport change mid-drag ends it like a pointer loss.
    let usable = drag.as_ref().is_none_or(|drag| drag.view == frame)
        && crate::viewport_widgets::gesture_allowed(ui);
    let Some((base_px, tip_px)) = points(view, rect, &target, shown).filter(|_| usable) else {
        end(app, drag, Outcome::Lost);
        return false;
    };
    let (pointer, events) = ui.input(|i| (i.pointer.hover_pos(), i.events.clone()));
    let hovered = pointer.is_some_and(|p| {
        crate::viewport_widgets::allows_pointer(ui, rect, p)
            && claims(view, rect, &target, shown, p)
    });
    let rule = Rule {
        step: crate::transform_ui::snap_step(view.world_per_pixel(view.depth(target.base))),
        two_directions: target.two_directions,
    };
    let (base, normal) = (target.base, target.normal);
    let axis = |pos| along(view, rect, pos, base, normal);
    let inside = |pos| crate::viewport_widgets::allows_pointer(ui, rect, pos);
    // Delivery order decides: a release ends the drag at that event, so later
    // motion in the same batch can never move the value.
    for event in &events {
        let Some(active) = drag.as_mut() else {
            if let egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers,
            } = event
                && inside(*pos)
                && claims(view, rect, &target, shown, *pos)
            {
                drag = axis(*pos)
                    .map(|t0| Drag::start(*pos, t0, shown, &target, modifiers.alt, frame));
            }
            continue;
        };
        match active.feed(event, &axis, &inside, &rule) {
            Fed::Tracking => continue,
            Fed::Released => finish(app, drag.take()),
            Fed::Lost => end(app, drag.take(), Outcome::Lost),
        }
        if let Some(tool) = app.doc.interaction.tool.as_ref() {
            (target.evaluated, target.draft) = authored(tool);
        }
        shown = app.doc.extrude_handle.last_valid.unwrap_or(shown);
    }
    // One draft change per frame keeps the evaluate/preview pipeline from flooding.
    if let Some(active) = drag.as_mut() {
        flush(app, active);
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

impl Drag {
    fn start(
        press: Pos2,
        t0: f64,
        shown: f64,
        target: &Target,
        free: bool,
        view: (Camera, Viewport, Rect),
    ) -> Self {
        Self {
            press,
            t0,
            d0: shown,
            d: shown,
            start: shown,
            original: target.draft.clone(),
            evaluated: target.evaluated,
            sent: None,
            accepted: false,
            free,
            view,
            lifecycle: Lifecycle::default(),
        }
    }

    /// Applies one event in delivery order, reading modifiers from the event stream.
    fn feed(
        &mut self,
        event: &egui::Event,
        axis: &dyn Fn(Pos2) -> Option<f64>,
        inside: &dyn Fn(Pos2) -> bool,
        rule: &Rule,
    ) -> Fed {
        match event {
            egui::Event::PointerGone | egui::Event::WindowFocused(false) => Fed::Lost,
            egui::Event::ModifiersChanged(modifiers) => {
                self.free = modifiers.alt;
                Fed::Tracking
            }
            egui::Event::PointerMoved(pos) => {
                self.track(*pos, axis, rule);
                Fed::Tracking
            }
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers,
            } => {
                // A release over chrome ends at the last motion; it adds none of its own.
                if inside(*pos) {
                    self.free = modifiers.alt;
                    self.track(*pos, axis, rule);
                }
                Fed::Released
            }
            _ => Fed::Tracking,
        }
    }

    fn track(&mut self, pos: Pos2, axis: &dyn Fn(Pos2) -> Option<f64>, rule: &Rule) {
        self.accepted |= crate::parameter_drag::beyond_threshold(
            self.press,
            pos,
            crate::parameter_drag::THRESHOLD_PX,
        );
        if !self.accepted {
            return;
        }
        let Some(t) = axis(pos) else {
            return;
        };
        // A negative start with two directions (invalid anyway) clamps to the floor on first move.
        let (d, clamped) = next_distance(
            self.d0,
            t - self.t0,
            self.d,
            rule.step,
            self.free,
            rule.two_directions,
        );
        self.d = d;
        // At a limit, re-anchor so the first reversing sample moves the value
        // instead of first paying back hidden travel.
        if clamped {
            (self.t0, self.d0) = (t, d);
        }
    }

    /// Draft text for the dragged value; the authored text survives when the
    /// value reads the same at display precision.
    fn text(&self) -> String {
        let text = crate::sketch_numeric::format::format_measure(self.d);
        let same = self
            .evaluated
            .is_some_and(|value| crate::sketch_numeric::format::format_measure(value) == text);
        if same { self.original.clone() } else { text }
    }

    fn changed(&self) -> bool {
        self.accepted && self.text() != self.original
    }

    /// Text to author now, if accepted motion produced a new one.
    fn pending(&self) -> Option<String> {
        if !self.accepted {
            return None;
        }
        let text = self.text();
        let last = self.sent.as_deref().unwrap_or(&self.original);
        (text != last).then_some(text)
    }
}

fn flush(app: &mut App, drag: &mut Drag) {
    if let Some(text) = drag.pending() {
        drag.sent = Some(text.clone());
        drag.lifecycle.author(
            app,
            Action::ParameterDrag(DragPhase::Update {
                key: DISTANCE,
                text,
            }),
        );
    }
}

/// Release: author the final value once and commit it; a release that reads
/// as the original restores it exactly; a click authors nothing.
fn finish(app: &mut App, drag: Option<Drag>) {
    let Some(mut drag) = drag else {
        return;
    };
    let changed = drag.changed();
    if changed {
        flush(app, &mut drag);
        app.doc.extrude_handle.last_valid = Some(drag.d);
    } else {
        restore_shown(app, &drag);
    }
    drag.lifecycle.end(app, Outcome::Finish { changed });
}

fn end(app: &mut App, drag: Option<Drag>, outcome: Outcome) {
    let Some(drag) = drag else {
        return;
    };
    restore_shown(app, &drag);
    drag.lifecycle.end(app, outcome);
}

/// The handle shows the restored original, so a later press must hit it there.
fn restore_shown(app: &mut App, drag: &Drag) {
    app.doc.extrude_handle.last_valid = Some(drag.evaluated.unwrap_or(drag.start));
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
pub(crate) fn testing_tip(app: &App, rect: Rect, distance: f64) -> Option<Pos2> {
    Some(points(app.doc.view.as_ref()?, rect, &target(app)?, distance)?.1)
}

#[cfg(test)]
#[path = "extrude_handle_tests.rs"]
mod tests;
