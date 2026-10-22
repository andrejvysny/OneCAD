//! Pencil Stroke tool (ONECAD-458): one primary drag on the sketch canvas is
//! recognized as a Line or a three-point Arc and committed as one Upsert. Only
//! egui pointer events feed it, and hosts deliver confirmed samples only (iPad
//! predictions stay overlays), so recognition never sees a predicted point. A
//! `PointerGone` (a cancelled contact) or Escape drops the stroke uncommitted. After
//! a commit the override bar redraws the stroke as the other kind or undoes it.
use crate::app::App;
use egui::{Event, PointerButton, Pos2, Rect, Ui};
use onecad_core::ids::EntityId;
use onecad_interaction::sketch::{
    SketchAction, SketchPhase, SketchSession, SketchSessionKey, capture_sketch_pointer,
    recognition::{Recognition, StrokeKind, StrokeRecognizer, StrokeShape},
};
use onecad_scene::{DVec2, ViewSnapshot};

/// Screen travel below which a stroke is a stray touch and ignored.
const MIN_TRAVEL_PT: f32 = 6.0;
/// Painted samples per stroke; recognition keeps every sample.
const MAX_PAINTED: usize = 4096;

#[derive(Clone)]
struct Drawing {
    key: SketchSessionKey,
    recognizer: StrokeRecognizer,
    painted: Vec<Pos2>,
}

/// The committed stroke the override bar acts on.
#[derive(Clone)]
struct LastStroke {
    key: SketchSessionKey,
    recognizer: StrokeRecognizer,
    kind: StrokeKind,
    entities: Vec<EntityId>,
    anchor: Pos2,
    /// Undo depth once the stroke's batch landed: the stroke is the newest step.
    depth: Option<usize>,
    /// An override waiting for the stroke's undo to land.
    pending: Option<StrokeShape>,
}

fn drawing_id() -> egui::Id {
    crate::documents::scoped_id("pen-stroke-drawing")
}

fn last_id() -> egui::Id {
    crate::documents::scoped_id("pen-stroke-last")
}

/// Another tool is active: forget the stroke in progress and the override.
pub(crate) fn clear(ctx: &egui::Context) {
    ctx.data_mut(|data| {
        data.remove::<Drawing>(drawing_id());
        data.remove::<LastStroke>(last_id());
    });
}

pub(crate) fn viewport(app: &mut App, ui: &Ui, rect: Rect, view: &ViewSnapshot) {
    let Some(session) = app.doc.sketch.session.as_ref() else {
        return;
    };
    let key = session.key.clone();
    let drawing = ui
        .ctx()
        .data(|data| data.get_temp::<Drawing>(drawing_id()))
        .filter(|drawing| drawing.key == key);
    let (drawing, finished) = feed(session, ui, rect, view, drawing);
    paint(app, ui, drawing.as_ref());
    ui.ctx().data_mut(|data| match &drawing {
        Some(drawing) => {
            data.insert_temp(drawing_id(), drawing.clone());
        }
        None => data.remove::<Drawing>(drawing_id()),
    });
    if let Some((done, at)) = finished {
        finish(app, ui, rect, view, done, at);
    }
}

/// Applies this frame's pointer events: the stroke in progress after them,
/// and the stroke a release finished with its release position.
fn feed(
    session: &SketchSession,
    ui: &Ui,
    rect: Rect,
    view: &ViewSnapshot,
    mut drawing: Option<Drawing>,
) -> (Option<Drawing>, Option<(Drawing, Pos2)>) {
    let canvas = Canvas {
        session,
        ui,
        rect,
        view,
    };
    let mut finished = None;
    for event in ui.input(|input| input.events.clone()) {
        if let Some(done) = canvas.step(&event, &mut drawing) {
            finished = Some(done);
        }
    }
    // The release landed in a frame that never reached this canvas (an overlay
    // or dimension owned the pointer): drop the stroke, or later hover moves
    // would extend it and a later release commit it.
    if !ui.input(|input| input.pointer.primary_down()) {
        drawing = None;
    }
    (drawing, finished)
}

struct Canvas<'a> {
    session: &'a SketchSession,
    ui: &'a Ui,
    rect: Rect,
    view: &'a ViewSnapshot,
}

impl Canvas<'_> {
    fn step(&self, event: &Event, drawing: &mut Option<Drawing>) -> Option<(Drawing, Pos2)> {
        let (session, view, rect) = (self.session, self.view, self.rect);
        match event {
            // A press elsewhere (chrome) is another contact: the old stroke is over.
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: true,
                ..
            } => {
                *drawing = crate::viewport_widgets::allows_pointer(self.ui, rect, *pos)
                    .then(|| begin(session, view, rect, *pos))
                    .flatten();
            }
            Event::PointerMoved(pos) => {
                if let Some(drawing) = drawing.as_mut() {
                    extend(drawing, session, view, rect, *pos);
                }
            }
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: false,
                ..
            } => {
                let mut done = drawing.take()?;
                extend(&mut done, session, view, rect, *pos);
                return Some((done, *pos));
            }
            Event::PointerGone
            | Event::Key {
                key: egui::Key::Escape,
                pressed: true,
                ..
            } => *drawing = None,
            _ => {}
        }
        None
    }
}

/// A stroke is in progress: its moves and release must reach [`viewport`] even
/// while the pointer is over chrome, or the stroke would never finish.
pub(crate) fn drawing(ctx: &egui::Context) -> bool {
    ctx.data(|data| data.get_temp::<Drawing>(drawing_id()).is_some())
}

/// Drawn every sketch frame, not only while the pointer hovers the canvas: the
/// pointer is over the bar itself when it is pressed.
pub(crate) fn bar(app: &mut App, ui: &Ui, rect: Rect, view: &ViewSnapshot) {
    override_bar(app, ui, rect, view);
}

fn logical(rect: Rect, pos: Pos2) -> DVec2 {
    DVec2::new(f64::from(pos.x - rect.min.x), f64::from(pos.y - rect.min.y))
}

/// The movement threshold is screen-sized: its sketch length follows the zoom.
fn begin(session: &SketchSession, view: &ViewSnapshot, rect: Rect, pos: Pos2) -> Option<Drawing> {
    let at = capture_sketch_pointer(session, view, logical(rect, pos)).ok()?;
    let beside = pos + egui::vec2(MIN_TRAVEL_PT, 0.0);
    let reach = capture_sketch_pointer(session, view, logical(rect, beside))
        .map(|point| (point.point.x - at.point.x).hypot(point.point.y - at.point.y))
        .unwrap_or(0.0);
    let mut recognizer = StrokeRecognizer::new(reach);
    recognizer.push(at.point);
    Some(Drawing {
        key: session.key.clone(),
        recognizer,
        painted: vec![pos],
    })
}

fn extend(
    drawing: &mut Drawing,
    session: &SketchSession,
    view: &ViewSnapshot,
    rect: Rect,
    pos: Pos2,
) {
    if let Ok(point) = capture_sketch_pointer(session, view, logical(rect, pos)) {
        drawing.recognizer.push(point.point);
        if drawing.painted.len() < MAX_PAINTED {
            drawing.painted.push(pos);
        }
    }
}

fn paint(app: &App, ui: &Ui, drawing: Option<&Drawing>) {
    let Some(drawing) = drawing.filter(|drawing| drawing.painted.len() > 1) else {
        return;
    };
    let stroke = egui::Stroke::new(2.0, app.theme.accent);
    ui.painter()
        .add(egui::Shape::line(drawing.painted.clone(), stroke));
    let decision = match drawing.recognizer.finish() {
        Recognition::Shape(shape) => Some(label(shape.kind())),
        Recognition::TooShort | Recognition::Unrecognized(_) => None,
    };
    if let (Some(text), Some(last)) = (decision, drawing.painted.last()) {
        ui.painter().text(
            *last + egui::vec2(12.0, -12.0),
            egui::Align2::LEFT_BOTTOM,
            text,
            egui::FontId::proportional(12.0),
            app.theme.ink3,
        );
    }
}

fn label(kind: StrokeKind) -> &'static str {
    match kind {
        StrokeKind::Line => "Line",
        StrokeKind::Arc => "Arc",
    }
}

/// A recognized stroke commits; an unrecognized one is refused by the session
/// with its reason and leaves no history; a stray touch is ignored.
fn finish(app: &mut App, ui: &Ui, rect: Rect, view: &ViewSnapshot, done: Drawing, at: Pos2) {
    let stroke = done.recognizer.finish();
    if stroke == Recognition::TooShort {
        return;
    }
    let Some(session) = app.doc.sketch.session.as_ref() else {
        return;
    };
    let Ok(point) = capture_sketch_pointer(session, view, logical(rect, at)) else {
        return;
    };
    let kind = match &stroke {
        Recognition::Shape(shape) => shape.kind(),
        Recognition::TooShort | Recognition::Unrecognized(_) => StrokeKind::Line,
    };
    // A refused stroke (unrecognized, or an earlier batch still in flight)
    // leaves the previous stroke's bar and any override it awaits in place.
    let entities = commit(app, point, stroke);
    if !entities.is_empty() {
        let last = LastStroke {
            key: done.key,
            recognizer: done.recognizer,
            kind,
            entities,
            anchor: at,
            depth: None,
            pending: None,
        };
        ui.ctx().data_mut(|data| data.insert_temp(last_id(), last));
    }
}

/// Dispatches one stroke; the entities it adds when the session accepted it.
fn commit(
    app: &mut App,
    point: onecad_interaction::sketch::CapturedSketchPoint,
    stroke: Recognition,
) -> Vec<EntityId> {
    let ids = crate::sketch_creation::fresh_ids();
    let count = match &stroke {
        Recognition::Shape(StrokeShape::Line { .. }) => 3,
        Recognition::Shape(StrokeShape::Arc { .. }) => 2,
        Recognition::TooShort | Recognition::Unrecognized(_) => 0,
    };
    let entities = ids.entities[..count].to_vec();
    let phase = |app: &App| app.doc.sketch.session.as_ref().map(|session| session.phase);
    // While a batch is in flight the session refuses the stroke with a reason;
    // only a stroke dispatched from Active and now submitting was accepted.
    let was_active = phase(app) == Some(SketchPhase::Active);
    app.sketch_dispatch(SketchAction::CommitStroke { point, ids, stroke });
    let submitted = was_active && phase(app) == Some(SketchPhase::Submitting);
    if submitted { entities } else { Vec::new() }
}

#[path = "sketch_pen_stroke_bar.rs"]
mod bar;
use bar::override_bar;
