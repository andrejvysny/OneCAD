use super::*;
use egui::{PointerButton, Pos2, Rect, Response, Ui};
use onecad_core::math::Vec2;
use onecad_interaction::sketch::{
    SketchTarget, SketchTool, capture_sketch_pointer,
    gesture::{GestureScope, classify_target},
};
use onecad_scene::{DVec2, ViewSnapshot};

#[derive(Clone)]
pub(crate) struct Armed {
    scope: GestureScope,
    target: ArmedTarget,
    grab: Vec2,
    press: Pos2,
    moved: bool,
}

impl Armed {
    pub(crate) fn cancel(&mut self) {
        self.target = ArmedTarget::Cancelled;
    }
}

#[derive(Clone)]
enum ArmedTarget {
    Drag(SketchTarget),
    Refused(String),
    Stale,
    Cancelled,
}

pub(crate) fn delivered_inside(app: &App, ui: &Ui, rect: Rect) -> bool {
    if !app
        .sketch
        .session
        .as_ref()
        .is_some_and(|session| session.tool == SketchTool::Select)
    {
        return false;
    }
    let position = ui.input(|input| {
        press(input).map(|(position, _)| position).or_else(|| {
            app.armed_sketch_gesture
                .as_ref()
                .and_then(|_| release_position(input))
        })
    });
    position.is_some_and(|position| crate::viewport_widgets::allows_pointer(ui, rect, position))
}

pub(crate) fn viewport(
    app: &mut App,
    ui: &Ui,
    rect: Rect,
    response: &Response,
    view: &ViewSnapshot,
) -> bool {
    app.sync_sketch_gesture();
    if app.sketch_gesture.busy() {
        return move_active(app, ui, rect, response, view);
    }
    let Some(session) = app
        .sketch
        .session
        .as_ref()
        .filter(|session| session.tool == SketchTool::Select)
    else {
        app.armed_sketch_gesture = None;
        return false;
    };
    let Ok(scope) = GestureScope::from_session(session) else {
        return false;
    };
    let mut armed = app.armed_sketch_gesture.take().map(|mut armed| {
        if armed.scope != scope {
            armed.scope = scope.clone();
            armed.target = ArmedTarget::Stale;
        }
        armed
    });
    let pressed = ui.input(|input| input.pointer.button_pressed(PointerButton::Primary));
    if pressed {
        armed = arm(app, ui, rect, response, view, scope);
    }
    let Some(mut armed) = armed else {
        return false;
    };
    let max_click_dist = ui
        .ctx()
        .options(|options| options.input_options.max_click_dist);
    let motion = ui.input(|input| sample(input, armed.press, pressed, max_click_dist));
    if motion.conflict {
        crate::sketch_view::cancel_selection_pointer(ui.ctx());
        crate::input_trace::app(
            app,
            "gesture.refused",
            serde_json::json!({"reason": "Multiple primary presses in one input batch"}),
        );
        app.cancel_sketch_gesture();
        return true;
    }
    armed.moved |= motion.moved;
    let released = ui.input(|input| input.pointer.button_released(PointerButton::Primary));
    let drag_started = response.drag_started_by(PointerButton::Primary) || released && armed.moved;
    if drag_started {
        crate::sketch_view::cancel_selection_pointer(ui.ctx());
        match &armed.target {
            ArmedTarget::Drag(target) => {
                app.gesture_dispatch(GestureAction::Begin {
                    scope: armed.scope,
                    target: *target,
                    grab: armed.grab,
                });
                return move_active(app, ui, rect, response, view);
            }
            ArmedTarget::Refused(message) => {
                crate::input_trace::app(
                    app,
                    "gesture.refused",
                    serde_json::json!({"reason": crate::input_trace::bounded_id(message)}),
                );
                app.status = message.clone();
                app.diagnostics.record(
                    crate::diagnostics::Source::Sketch,
                    crate::diagnostics::State::Rejected,
                    message,
                );
            }
            ArmedTarget::Stale => {
                app.status =
                    "Gesture start belongs to an obsolete captured view; press again".into();
                crate::input_trace::app(
                    app,
                    "gesture.refused",
                    serde_json::json!({"reason": app.status}),
                );
            }
            ArmedTarget::Cancelled => {}
        }
    }
    let consumed = drag_started
        || armed.moved
        || matches!(armed.target, ArmedTarget::Stale | ArmedTarget::Cancelled);
    if consumed {
        crate::sketch_view::cancel_selection_pointer(ui.ctx());
    }
    if ui.input(|input| input.pointer.primary_down()) {
        app.armed_sketch_gesture = Some(armed);
    }
    // Selection must capture the press until movement actually claims the gesture.
    consumed
}

fn arm(
    app: &mut App,
    ui: &Ui,
    rect: Rect,
    response: &Response,
    view: &ViewSnapshot,
    scope: GestureScope,
) -> Option<Armed> {
    let (position, modifiers) = ui.input(press).filter(|(position, _)| {
        model_position(app, ui, rect, *position)
            && ui
                .ctx()
                .layer_id_at(*position)
                .is_none_or(|layer| layer == response.layer_id)
    })?;
    if modifiers.shift || modifiers.command {
        return None;
    }
    let session = app.sketch.session.as_ref()?;
    let grab = point(app, rect, view, position)?;
    match crate::sketch_view::pick_entity(
        rect,
        view,
        session,
        position,
        crate::sketch_view::DEFAULT_PICK_RADIUS,
    ) {
        crate::sketch_view::PickResult::Unique(target) => {
            let target = match classify_target(&session.authoritative, target, grab) {
                Ok(_) => ArmedTarget::Drag(target),
                Err(message) => ArmedTarget::Refused(message),
            };
            Some(Armed {
                scope,
                target,
                grab,
                press: position,
                moved: false,
            })
        }
        crate::sketch_view::PickResult::Ambiguous(_) => Some(Armed {
            scope,
            target: ArmedTarget::Refused(
                "Gesture target is ambiguous; click to choose an explicit point role or entity"
                    .into(),
            ),
            grab,
            press: position,
            moved: false,
        }),
        crate::sketch_view::PickResult::None => None,
    }
}

fn move_active(
    app: &mut App,
    ui: &Ui,
    rect: Rect,
    response: &Response,
    view: &ViewSnapshot,
) -> bool {
    let Some(tag) = app.sketch_gesture.tag().cloned() else {
        return false;
    };
    if !app.sketch_gesture.active() {
        return true;
    }
    let release = ui.input(release_position);
    let position = release.or_else(|| ui.input(|input| input.pointer.hover_pos()));
    let Some(point) = position
        .filter(|position| {
            rect.contains(*position)
                && (release.is_none() || model_position(app, ui, rect, *position))
        })
        .and_then(|position| point(app, rect, view, position))
    else {
        app.cancel_sketch_gesture();
        return true;
    };
    if response.drag_stopped_by(PointerButton::Primary)
        || ui.input(|input| input.pointer.button_released(PointerButton::Primary))
    {
        app.gesture_dispatch(GestureAction::End { tag, point });
    } else if ui.input(|input| input.pointer.primary_down()) {
        app.gesture_dispatch(GestureAction::Move { tag, point });
    } else {
        app.cancel_sketch_gesture();
    }
    true
}

fn model_position(app: &App, ui: &Ui, rect: Rect, position: Pos2) -> bool {
    crate::viewport_widgets::allows_pointer(ui, rect, position)
        && !crate::viewport_widgets::owns_position(app, ui, rect, position)
        && !crate::sketch_dimensions::owns_position(app, ui, Some(position))
}

fn press(input: &egui::InputState) -> Option<(Pos2, egui::Modifiers)> {
    input.raw.events.iter().find_map(|event| match event {
        egui::Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: true,
            modifiers,
        } => Some((*pos, *modifiers)),
        _ => None,
    })
}

#[derive(Default)]
struct Motion {
    moved: bool,
    conflict: bool,
}

fn sample(input: &egui::InputState, press: Pos2, new_press: bool, max_click_dist: f32) -> Motion {
    let mut motion = Motion::default();
    let mut tracking = !new_press;
    let mut started = !new_press;
    // Respect the press/release boundaries that egui's final pointer state loses.
    for event in &input.raw.events {
        match event {
            egui::Event::PointerButton {
                button: PointerButton::Primary,
                pressed: true,
                ..
            } => {
                if started {
                    motion.conflict = true;
                    break;
                }
                started = true;
                tracking = true;
            }
            egui::Event::PointerMoved(position) if tracking => {
                motion.moved |= press.distance(*position) > max_click_dist;
            }
            egui::Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: false,
                ..
            } if tracking => {
                motion.moved |= press.distance(*pos) > max_click_dist;
                tracking = false;
            }
            _ => {}
        }
    }
    motion
}

fn release_position(input: &egui::InputState) -> Option<Pos2> {
    input.raw.events.iter().find_map(|event| match event {
        egui::Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: false,
            ..
        } => Some(*pos),
        _ => None,
    })
}

fn point(app: &App, rect: Rect, view: &ViewSnapshot, position: Pos2) -> Option<Vec2> {
    let session = app.sketch.session.as_ref()?;
    capture_sketch_pointer(
        session,
        view,
        DVec2::new(
            f64::from(position.x - rect.min.x),
            f64::from(position.y - rect.min.y),
        ),
    )
    .ok()
    .map(|point| point.point)
}
