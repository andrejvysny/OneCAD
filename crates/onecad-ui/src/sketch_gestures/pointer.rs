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
    if ui.input(|input| input.pointer.button_pressed(PointerButton::Primary)) {
        armed = arm(app, ui, rect, response, view, scope);
    }
    let Some(armed) = armed else {
        return false;
    };
    if response.drag_started_by(PointerButton::Primary) {
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
            }
            ArmedTarget::Cancelled => {}
        }
    }
    if ui.input(|input| input.pointer.primary_down()) {
        app.armed_sketch_gesture = Some(armed);
        return true;
    }
    matches!(armed.target, ArmedTarget::Stale | ArmedTarget::Cancelled)
}

fn arm(
    app: &mut App,
    ui: &Ui,
    rect: Rect,
    response: &Response,
    view: &ViewSnapshot,
    scope: GestureScope,
) -> Option<Armed> {
    let position = response
        .interact_pointer_pos()
        .filter(|position| rect.contains(*position))?;
    if ui.input(|input| input.modifiers.shift || input.modifiers.command) {
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
            })
        }
        crate::sketch_view::PickResult::Ambiguous(_) => Some(Armed {
            scope,
            target: ArmedTarget::Refused(
                "Gesture target is ambiguous; click to choose an explicit point role or entity"
                    .into(),
            ),
            grab,
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
    let position = ui.input(|input| input.pointer.hover_pos());
    let Some(point) = position
        .filter(|position| rect.contains(*position))
        .and_then(|position| point(app, rect, view, position))
    else {
        app.cancel_sketch_gesture();
        return true;
    };
    if response.drag_stopped_by(PointerButton::Primary) {
        app.gesture_dispatch(GestureAction::End { tag, point });
    } else if ui.input(|input| input.pointer.primary_down()) {
        app.gesture_dispatch(GestureAction::Move { tag, point });
    } else {
        app.cancel_sketch_gesture();
    }
    true
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
