//! Labels and witnesses share one captured view and the native theme tokens.
use super::{
    cache::{Cache, Drag, Placement, place},
    geometry::Witness,
};
use crate::sketch_numeric::format::{format_angle_degrees, format_measure};
use crate::theme::Theme;
use egui::{FontId, Pos2, Rect, Sense, Stroke, Ui};
use onecad_core::{
    ids::ConstraintId,
    math::Vec2 as SketchPoint,
    sketch::{Constraint, SketchPlane},
};
use onecad_scene::{DVec3, ViewSnapshot};

pub struct Badge {
    pub id: ConstraintId,
    pub text: String,
    pub accessible: String,
    pub witness: Witness,
}
struct Scope {
    key: onecad_interaction::sketch::SketchSessionKey,
    document: onecad_interaction::DocumentContext,
    capture: onecad_scene::CaptureIdentity,
}

#[derive(Default)]
pub struct Output {
    pub owns_pointer: bool,
    pub edit: Option<ConstraintId>,
    pub hover: Option<ConstraintId>,
}

pub fn project(
    rect: Rect,
    view: &ViewSnapshot,
    plane: SketchPlane,
    point: SketchPoint,
) -> Option<Pos2> {
    let world = plane.to_world(point);
    let screen = view.project(DVec3::new(world.x, world.y, world.z))?;
    let at = rect.min + egui::vec2(screen.logical.x as f32, screen.logical.y as f32);
    at.is_finite().then_some(at)
}

pub fn label(constraint: &Constraint) -> Option<(String, String)> {
    let scalar = constraint.value()?;
    let (kind, prefix, number, unit) = match constraint {
        Constraint::Distance { .. } => ("Distance", "", scalar.value, "mm"),
        Constraint::HorizontalDistance { .. } => ("Horizontal distance", "H ", scalar.value, "mm"),
        Constraint::VerticalDistance { .. } => ("Vertical distance", "V ", scalar.value, "mm"),
        Constraint::Angle { .. } => ("Angle", "", scalar.value.to_degrees(), "°"),
        Constraint::Radius { .. } => ("Radius", "R ", scalar.value, "mm"),
        Constraint::Diameter { .. } => ("Diameter", "Ø ", scalar.value, "mm"),
        _ => return None,
    };
    if !number.is_finite() {
        return None;
    }
    let number = if matches!(constraint, Constraint::Angle { .. }) {
        format_angle_degrees(number)
    } else {
        format_measure(number)
    };
    let value = format!("{prefix}{number} {unit}");
    let text = scalar.expr.as_ref().map_or_else(
        || value.clone(),
        |expression| format!("{} = {value}", short_expression(expression)),
    );
    let full = scalar.expr.as_ref().map_or_else(
        || value.clone(),
        |expression| format!("{expression} = {value}"),
    );
    Some((
        text,
        format!("Edit {kind}: {full}; constraint {}", constraint.id()),
    ))
}
fn short_expression(expression: &str) -> String {
    let mut chars = expression.chars();
    let prefix: String = chars.by_ref().take(48).collect();
    if chars.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}

#[derive(Clone, Copy)]
pub struct Canvas<'a> {
    pub rect: Rect,
    pub view: &'a ViewSnapshot,
    pub plane: SketchPlane,
    pub theme: &'a Theme,
    pub enabled: bool,
}

pub fn show(
    ui: &mut Ui,
    canvas: &Canvas<'_>,
    badges: &[Badge],
    cache: &mut Cache,
    mut occupied: Vec<Rect>,
) -> Output {
    let mut output = Output::default();
    if !canvas.enabled || ui.input(|input| !input.focused || input.key_pressed(egui::Key::Escape)) {
        cache.drag = None;
    }
    let scope = Scope {
        key: cache.key.clone(),
        document: cache.document.clone(),
        capture: cache.capture,
    };
    for badge in badges {
        let Some(index) = cache.entries.iter().position(|entry| entry.id == badge.id) else {
            continue;
        };
        let entry = &mut cache.entries[index];
        entry.last_rect = None;
        if entry.hidden {
            continue;
        }
        if let Some(result) = badge_ui(ui, canvas, badge, entry, &mut cache.drag, &scope, &occupied)
        {
            occupied.push(result.0);
            output.owns_pointer |= result.1;
            if result.1 {
                output.hover = Some(badge.id);
            }
            if result.2 {
                output.edit = Some(badge.id);
            }
        }
    }
    output.owns_pointer |= cache.drag.is_some();
    if cache.drag.is_some() && !ui.input(|input| input.pointer.primary_down()) {
        cache.drag = None;
    }
    output
}

fn badge_ui(
    ui: &mut Ui,
    canvas: &Canvas<'_>,
    badge: &Badge,
    entry: &mut Placement,
    drag: &mut Option<Drag>,
    scope: &Scope,
    occupied: &[Rect],
) -> Option<(Rect, bool, bool)> {
    let Canvas {
        rect,
        view,
        plane,
        theme,
        enabled,
    } = *canvas;
    let anchor = project(rect, view, plane, badge.witness.anchor)?;
    let galley = ui.painter().layout(
        badge.text.clone(),
        FontId::proportional(12.0),
        theme.ink,
        210.0,
    );
    let offset = drag
        .as_ref()
        .filter(|drag| drag.id == entry.id)
        .and_then(|drag| {
            ui.input(|input| input.pointer.interact_pos())
                .map(|at| drag.origin + (at - drag.pointer))
        })
        .unwrap_or(entry.offset);
    let bounds = place(
        anchor,
        galley.size() + egui::vec2(14.0, 10.0),
        offset,
        rect,
        occupied,
    )?;
    let id = egui::Id::unique((
        "sketch-dimension",
        scope.key.runtime_session.as_str(),
        scope.key.sketch_id,
        scope.key.generation,
        scope.document.revision,
        scope.capture.scene.session,
        scope.capture.scene.scene,
        scope.capture.view,
        entry.id,
    ));
    let response = ui.interact(
        bounds,
        id,
        if enabled {
            Sense::click_and_drag()
        } else {
            Sense::hover()
        },
    );
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, &badge.accessible));
    if drag.is_none() {
        entry.offset = bounds.center() - anchor;
    }
    if enabled {
        interact(ui, &response, entry, drag, scope, anchor, bounds);
    }
    visibility_menu(&response, entry);
    draw(ui, canvas, badge, bounds, &galley, response.hovered());
    entry.last_rect = Some(bounds);
    let owned = response.hovered()
        || response.is_pointer_button_down_on()
        || response.dragged()
        || response.drag_stopped()
        || response.context_menu_opened();
    let edit = enabled && response.clicked_by(egui::PointerButton::Primary);
    response.on_hover_text(format!(
        "{}\nClick to edit. Drag to move this label. Right-click for visibility.",
        badge.accessible
    ));
    Some((bounds, owned, edit))
}

fn interact(
    ui: &Ui,
    response: &egui::Response,
    entry: &mut Placement,
    drag: &mut Option<Drag>,
    scope: &Scope,
    anchor: Pos2,
    bounds: Rect,
) {
    if response.drag_started_by(egui::PointerButton::Primary)
        && let Some(pointer) = ui.input(|input| input.pointer.press_origin())
    {
        *drag = Some(Drag {
            id: entry.id,
            document: scope.document.clone(),
            capture: scope.capture,
            origin: entry.offset,
            pointer,
        });
    }
    if response.drag_stopped_by(egui::PointerButton::Primary)
        && drag.as_ref().is_some_and(|drag| {
            drag.id == entry.id && drag.document == scope.document && drag.capture == scope.capture
        })
    {
        entry.offset = bounds.center() - anchor;
        *drag = None;
    }
}

fn visibility_menu(response: &egui::Response, entry: &mut Placement) {
    response.context_menu(|ui| {
        if ui.button("Hide dimension label").clicked() {
            entry.hidden = true;
            ui.close();
        }
        if ui.button("Reset label position").clicked() {
            entry.offset = egui::vec2(0.0, -26.0);
            ui.close();
        }
    });
}

fn draw(
    ui: &Ui,
    canvas: &Canvas<'_>,
    badge: &Badge,
    bounds: Rect,
    galley: &std::sync::Arc<egui::Galley>,
    hovered: bool,
) {
    let Canvas {
        rect,
        view,
        plane,
        theme,
        ..
    } = *canvas;
    let painter = ui.painter().with_clip_rect(rect);
    let ink = if hovered { theme.accent } else { theme.ink4 };
    for segment in &badge.witness.segments {
        if let (Some(a), Some(b)) = (
            project(rect, view, plane, segment[0]),
            project(rect, view, plane, segment[1]),
        ) {
            painter.line_segment([a, b], Stroke::new(1.0, ink));
        }
    }
    for at in badge.witness.measured {
        if let Some(at) = project(rect, view, plane, at) {
            painter.circle_filled(at, 2.0, ink);
        }
    }
    if let Some(anchor) = project(rect, view, plane, badge.witness.anchor) {
        painter.line_segment([anchor, bounds.center()], Stroke::new(1.0, ink));
    }
    painter.rect_filled(bounds, 6.0, theme.chip);
    painter.rect_stroke(
        bounds,
        6.0,
        Stroke::new(1.0, if hovered { theme.accent } else { theme.border }),
        egui::StrokeKind::Inside,
    );
    painter.galley(
        bounds.center() - galley.size() * 0.5,
        galley.clone(),
        theme.ink,
    );
}
