//! Geometry-anchored model drafts. Pointer release keeps a draft; explicit Apply owns history.
mod anchors;
mod boolean;
mod drag;
mod geometry;
mod history;
mod labels;
mod operands;
mod surface;
pub(crate) use history::{eligible_bodies, poll as poll_history};
pub(crate) use operands::capture as capture_operands;
pub(crate) use surface::capture as capture_surface;
mod target;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;
use crate::{app::App, parameter_drag::Outcome};
use drag::Drag;
use egui::{Pos2, Rect};
use geometry::Geometry;
use onecad_core::{document::record::MirrorBodyParams, ids::RecordId};
use onecad_interaction::{Action, DragPhase, ModelingOptions, ParameterKey};
use onecad_scene::ViewSnapshot;

#[derive(Default)]
pub(crate) struct ModelHandle {
    drag: Option<Drag>,
    record: Option<RecordId>,
    values: Vec<(ParameterKey, f64)>,
    operands: Option<operands::Operands>,
    history: Option<history::History>,
    choice: Option<boolean::ChoicePress>,
}
impl ModelHandle {
    pub(crate) fn active(&self) -> bool {
        self.drag.is_some()
    }
    fn value(&self, key: ParameterKey) -> Option<f64> {
        self.values.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
    }
}
#[derive(Clone)]
enum Kind {
    Scalar(ParameterKey),
    Mirror(MirrorBodyParams),
}
#[derive(Clone)]
struct Target {
    record: RecordId,
    kind: Kind,
    geometry: Geometry,
    value: f64,
    original: String,
    caption: Option<&'static str>,
}
impl Target {
    fn label(&self, value: f64) -> String {
        let (name, shown) = match &self.kind {
            Kind::Scalar(key) => (key.parameter().label(), value),
            Kind::Mirror(plane) => {
                let offset = anchors::vector(plane.plane_normal)
                    .try_normalize()
                    .map_or(0.0, |normal| normal.dot(anchors::vector(plane.plane_point)));
                ("Mirror plane offset", offset + value)
            }
        };
        format!(
            "{}: {} {}",
            self.caption.unwrap_or(name),
            labels::number(shown),
            if self.geometry.angle() { "°" } else { "mm" }
        )
    }
}
pub(crate) fn available(app: &App) -> bool {
    !target::targets(app).is_empty() || boolean::parameters(app).is_some()
}
pub(crate) fn owns_pointer(app: &App, view: &ViewSnapshot, rect: Rect, pointer: Pos2) -> bool {
    app.doc.model_handle.active()
        || app.doc.model_handle.choice.is_some()
        || targets_at(app, view, rect)
            .iter()
            .any(|t| t.geometry.claims(view, rect, t.value, pointer))
        || boolean::bounds(app, view, rect).is_some_and(|r| r.contains(pointer))
}
pub(crate) fn abandon(app: &mut App, outcome: Outcome) {
    if let Some(drag) = app.doc.model_handle.drag.take() {
        drag.lifecycle.end(app, outcome);
    }
    app.doc.model_handle.values.clear();
    app.doc.model_handle.choice = None;
}
pub(crate) fn settle(app: &mut App, ctx: &egui::Context) {
    let (focused, down) = ctx.input(|i| (i.focused, i.pointer.primary_down()));
    boolean::settle(app, focused, down);
    if !app.doc.model_handle.active() {
        return;
    }
    if !focused || app.doc.interaction.tool.is_none() {
        abandon(app, Outcome::Lost);
    } else if !down {
        finish(app);
    }
}
fn flush(app: &mut App, drag: &mut Drag) {
    let Some(text) = drag.pending() else {
        return;
    };
    let action = match &drag.target.kind {
        Kind::Scalar(key) => Action::ParameterDrag(DragPhase::Update {
            key: *key,
            text: text.clone(),
        }),
        Kind::Mirror(p) => {
            let Geometry::Axis { direction, .. } = drag.target.geometry else {
                return;
            };
            let Some(tool) = app.doc.interaction.tool.as_ref() else {
                return;
            };
            Action::ModelingOptionsChanged {
                tag: tool.tag.clone(),
                options: ModelingOptions::Mirror {
                    point: anchors::core(anchors::vector(p.plane_point) + direction * drag.value),
                    normal: p.plane_normal,
                    fuse: p.fuse_with_original,
                },
            }
        }
    };
    drag.lifecycle.author(app, action);
    drag.sent = Some(text);
}
fn finish(app: &mut App) {
    let Some(mut drag) = app.doc.model_handle.drag.take() else {
        return;
    };
    let changed = drag.changed();
    if changed {
        flush(app, &mut drag);
    }
    if let Kind::Scalar(key) = drag.target.kind {
        remember(
            app,
            key,
            if changed {
                drag.value
            } else {
                drag.target.value
            },
        );
    }
    drag.lifecycle.end(app, Outcome::Finish { changed });
}
fn remember(app: &mut App, key: ParameterKey, value: f64) {
    app.doc.model_handle.values.retain(|(k, _)| *k != key);
    app.doc.model_handle.values.push((key, value));
}
fn targets_at(app: &App, view: &ViewSnapshot, rect: Rect) -> Vec<Target> {
    let mut targets = target::targets(app);
    let screen = Rect::from_min_max(Pos2::ZERO, rect.max);
    let chrome = crate::layout::chrome(screen, &app.ui).with_feature_card();
    let mut covers = Vec::new();
    if app
        .doc
        .interaction
        .tool
        .as_ref()
        .is_some_and(|t| !t.editing_existing)
    {
        covers.push(chrome.feature_card.expand(12.0));
    }
    if app.ui.inspector_open {
        covers.push(chrome.inspector.expand(8.0));
    }
    for target in &mut targets {
        target.geometry = target.geometry.avoid(view, rect, target.value, &covers);
    }
    targets
}
pub(crate) fn show(app: &mut App, ui: &mut egui::Ui, view: &ViewSnapshot, rect: Rect) -> bool {
    let record = app.doc.interaction.tool.as_ref().map(|t| t.tag.record_id);
    if app.doc.model_handle.record != record {
        abandon(app, Outcome::Lost);
        app.doc.model_handle.record = record;
    }
    let targets = targets_at(app, view, rect);
    let usable = crate::viewport_widgets::gesture_allowed(ui) && !targets.is_empty();
    if app.doc.model_handle.drag.as_ref().is_some_and(|d| {
        !usable
            || d.frame != (view.camera(), view.viewport(), rect)
            || Some(d.target.record) != record
    }) {
        abandon(app, Outcome::Lost);
    }
    for target in &targets {
        if let Kind::Scalar(key) = target.kind {
            remember(app, key, target.value);
        }
    }
    let events = ui.input(|i| i.events.clone());
    for event in events {
        event_input(app, ui, view, rect, &targets, event, usable);
    }
    if let Some(mut drag) = app.doc.model_handle.drag.take() {
        flush(app, &mut drag);
        app.doc.model_handle.drag = Some(drag);
    }
    let hot = paint_targets(app, ui, view, rect, &targets, usable);
    let active = app.doc.model_handle.active();
    if active {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        ui.ctx().request_repaint();
    } else if hot {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
    let controls = boolean::show(app, ui, view, rect);
    active || hot || controls
}
fn paint_targets(
    app: &App,
    ui: &egui::Ui,
    view: &ViewSnapshot,
    rect: Rect,
    targets: &[Target],
    usable: bool,
) -> bool {
    let pointer = ui.input(|i| i.pointer.hover_pos());
    let mut hot = false;
    let mut captions = Vec::new();
    for target in targets {
        let selected = app
            .doc
            .model_handle
            .drag
            .as_ref()
            .filter(|d| same_kind(&d.target.kind, &target.kind));
        let geometry = selected.map_or(target.geometry, |d| d.target.geometry);
        let value = selected.map_or(target.value, |d| d.value);
        let hovered = usable
            && pointer.is_some_and(|p| {
                crate::viewport_widgets::allows_pointer(ui, rect, p)
                    && geometry.claims(view, rect, value, p)
            });
        if matches!(target.kind, Kind::Mirror(_)) {
            geometry.paint_plane(ui, view, rect, value, app.theme.accent);
        }
        if let Some(tip) = geometry.paint(
            ui,
            rect,
            view,
            value,
            &app.theme,
            hovered || selected.is_some(),
        ) {
            captions.push((tip, labels::caption_text(target, selected, value)));
        }
        hot |= hovered;
    }
    labels::paint(app, ui, rect, &captions);
    hot
}
fn same_kind(a: &Kind, b: &Kind) -> bool {
    match (a, b) {
        (Kind::Scalar(a), Kind::Scalar(b)) => a == b,
        (Kind::Mirror(_), Kind::Mirror(_)) => true,
        _ => false,
    }
}
fn event_input(
    app: &mut App,
    ui: &egui::Ui,
    view: &ViewSnapshot,
    rect: Rect,
    targets: &[Target],
    event: egui::Event,
    usable: bool,
) {
    if !app.doc.model_handle.active() {
        if let egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers,
        } = event
            && usable
            && crate::viewport_widgets::allows_pointer(ui, rect, pos)
            && let Some(target) = targets
                .iter()
                .find(|t| t.geometry.claims(view, rect, t.value, pos))
        {
            app.doc.model_handle.drag = Drag::start(target.clone(), pos, view, rect, modifiers.alt);
        }
        return;
    }
    captured_input(app, ui, view, rect, event);
}
fn captured_input(
    app: &mut App,
    ui: &egui::Ui,
    view: &ViewSnapshot,
    rect: Rect,
    event: egui::Event,
) {
    match event {
        egui::Event::PointerGone | egui::Event::WindowFocused(false) => abandon(app, Outcome::Lost),
        egui::Event::ModifiersChanged(m) => {
            if let Some(drag) = app.doc.model_handle.drag.as_mut() {
                drag.free = m.alt;
            }
        }
        egui::Event::PointerMoved(pos) => {
            if let Some(drag) = app.doc.model_handle.drag.as_mut() {
                drag.track(pos, view, rect);
            }
        }
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers,
        } => {
            if crate::viewport_widgets::allows_pointer(ui, rect, pos)
                && let Some(drag) = app.doc.model_handle.drag.as_mut()
            {
                drag.free = modifiers.alt;
                drag.track(pos, view, rect);
            }
            finish(app);
        }
        _ => {}
    }
}

#[cfg(test)]
pub(crate) fn testing_tip(
    app: &App,
    rect: Rect,
    parameter: Option<onecad_interaction::NumericParameter>,
    value: f64,
) -> Option<Pos2> {
    let target = targets_at(app, app.doc.view.as_ref()?, rect)
        .into_iter()
        .find(|t| match (&t.kind, parameter) {
            (Kind::Scalar(key), Some(p)) => key.parameter() == p,
            (Kind::Mirror(_), None) => true,
            _ => false,
        })?;
    Some(
        target
            .geometry
            .points(app.doc.view.as_ref()?, rect, value)?
            .1,
    )
}
#[cfg(test)]
pub(crate) fn testing_axis(
    app: &App,
    parameter: onecad_interaction::NumericParameter,
) -> Option<(onecad_scene::DVec3, onecad_scene::DVec3, f64)> {
    target::targets(app).into_iter().find_map(|target| {
        let Kind::Scalar(key) = target.kind else {
            return None;
        };
        let Geometry::Axis {
            base,
            direction,
            scale,
            ..
        } = target.geometry
        else {
            return None;
        };
        (key.parameter() == parameter).then_some((base, direction, scale))
    })
}
#[cfg(test)]
pub(crate) fn testing_boolean_button(app: &App, rect: Rect, index: usize) -> Option<Pos2> {
    let bounds = boolean::bounds(app, app.doc.view.as_ref()?, rect)?;
    Some(bounds.min + egui::vec2(42.0 + index as f32 * 80.0, 16.0))
}
