//! Pencil primitives capture confirmed samples locally; only a valid release authors.
use super::*;
use egui::{Event, PointerButton, Pos2};
use onecad_interaction::{
    DocumentContext,
    sketch::{
        CapturedSketchPoint, DraftPrimitive, NumericCreationMethod, PrimitiveDrag, SketchPhase,
    },
};

#[derive(Clone)]
struct Capture {
    document: DocumentContext,
    start: CapturedSketchPoint,
    end: CapturedSketchPoint,
    start_snap: Option<AcceptedSnap>,
    end_snap: Option<AcceptedSnap>,
    tool: SketchTool,
    generation: u64,
    sequence: u64,
    construction: bool,
    press: Pos2,
    promoted: bool,
    cancelled: bool,
    terminated: bool,
}

fn id() -> egui::Id {
    crate::documents::scoped_id("sketch-primitive-drag")
}

fn cancelled_pass_id() -> egui::Id {
    crate::documents::scoped_id("sketch-primitive-cancelled-pass")
}

pub(crate) fn pending(ctx: &egui::Context) -> bool {
    ctx.data(|data| data.get_temp::<Capture>(id()).is_some())
}

pub(crate) fn delivered(ui: &Ui) -> bool {
    crate::viewport_widgets::pointer_kind(ui.ctx()) == crate::viewport_widgets::PointerKind::Pencil
        && ui.input(|input| {
            input.raw.events.iter().any(|event| {
                matches!(
                    event,
                    Event::PointerButton {
                        button: PointerButton::Primary,
                        ..
                    }
                )
            })
        })
}

pub(crate) fn cancel(ctx: &egui::Context) {
    ctx.data_mut(|data| {
        if let Some(mut capture) = data.get_temp::<Capture>(id()) {
            capture.cancelled = true;
            capture.terminated = true;
            data.insert_temp(id(), capture);
        }
    });
}

pub(crate) fn route_raw(app: &App, ctx: &egui::Context, raw: &egui::RawInput) -> bool {
    if !raw.focused
        || app.composing
        || ctx.text_edit_focused()
        || crate::input::overlay_owns_keyboard(app, ctx)
        || raw
            .events
            .iter()
            .any(|event| matches!(event, Event::Ime(_)))
    {
        cancel(ctx);
    }
    let escape = (pending(ctx) || starts_primitive(app, ctx, raw))
        && raw.events.iter().any(|event| {
            matches!(event,
        Event::Key { key: egui::Key::Escape, pressed: true, modifiers, .. } if !modifiers.command)
        });
    if escape {
        cancel(ctx);
        let pass = ctx.cumulative_pass_nr();
        ctx.data_mut(|data| data.insert_temp(cancelled_pass_id(), pass));
    }
    escape
}

fn starts_primitive(app: &App, ctx: &egui::Context, raw: &egui::RawInput) -> bool {
    crate::viewport_widgets::pointer_kind(ctx)
        == crate::viewport_widgets::PointerKind::Pencil
        && raw.events.iter().rev().find_map(|event| match event {
            Event::ModifiersChanged(modifiers) => Some(*modifiers), _ => None,
        }).unwrap_or_else(|| ctx.input(|input| input.modifiers)) == egui::Modifiers::NONE
        && app.doc.sketch.session.as_ref().is_some_and(eligible)
        && raw.events.iter().any(|event| {
            matches!(
                event,
                Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers,
                    ..
                } if *modifiers == egui::Modifiers::NONE && !crate::viewport_widgets::over_control(ctx, *pos)
            )
        })
}

fn eligible(session: &SketchSession) -> bool {
    session.phase == SketchPhase::Active
        && session.numeric_method() != NumericCreationMethod::TangentArc
        && matches!(
            session.tool,
            SketchTool::Line
                | SketchTool::Rectangle
                | SketchTool::CenterRectangle
                | SketchTool::Circle
        )
}

#[cfg(test)]
pub(crate) fn preview(ctx: &egui::Context) -> Option<DraftPrimitive> {
    ctx.data(|data| data.get_temp::<Capture>(id()))
        .filter(|capture| capture.promoted && !capture.cancelled)
        .map(|capture| capture.preview())
}

impl Capture {
    fn current(&self, session: &SketchSession, view: &ViewSnapshot) -> bool {
        self.document == session.document
            && self.start.key == session.key
            && self.start.capture == view.identity()
            && session.accepted_capture == Some(view.identity())
            && session.phase == SketchPhase::Active
            && self.tool == session.tool
            && self.generation == session.draft_generation
            && self.sequence == session.request.sequence
            && self.construction == session.construction
            && session.numeric_method() != NumericCreationMethod::TangentArc
    }

    fn commit(self) -> SketchAction {
        SketchAction::CommitPrimitiveDrag(Box::new(PrimitiveDrag {
            start: self.start,
            end: self.end,
            start_snap: self.start_snap,
            end_snap: self.end_snap,
            tool: self.tool,
            draft_generation: self.generation,
            request_sequence: self.sequence,
            construction: self.construction,
            ids: fresh_ids(),
        }))
    }

    fn preview(&self) -> DraftPrimitive {
        let (first, opposite) = (self.start.point, self.end.point);
        match self.tool {
            SketchTool::Circle => DraftPrimitive::Circle {
                center: first,
                radius: (opposite.x - first.x).hypot(opposite.y - first.y),
            },
            SketchTool::Line => DraftPrimitive::Line {
                start: first,
                end: opposite,
            },
            SketchTool::CenterRectangle => DraftPrimitive::Rectangle {
                first: onecad_core::math::Vec2::new_unchecked(
                    2.0 * first.x - opposite.x,
                    2.0 * first.y - opposite.y,
                ),
                opposite,
            },
            _ => DraftPrimitive::Rectangle { first, opposite },
        }
    }
}

struct Canvas<'a> {
    app: &'a App,
    ui: &'a Ui,
    rect: Rect,
    view: &'a ViewSnapshot,
}

impl Canvas<'_> {
    fn sample(
        &self,
        position: Pos2,
        anchor: Option<onecad_core::math::Vec2>,
        previous: Option<&AcceptedSnap>,
    ) -> Option<(CapturedSketchPoint, Option<AcceptedSnap>)> {
        if !crate::viewport_widgets::allows_pointer(self.ui, self.rect, position) {
            return None;
        }
        let session = self.app.doc.sketch.session.as_ref()?;
        let logical = DVec2::new(
            f64::from(position.x - self.rect.min.x),
            f64::from(position.y - self.rect.min.y),
        );
        let mut point =
            onecad_interaction::sketch::capture_sketch_pointer(session, self.view, logical).ok()?;
        let snap = super::snap_at(
            self.app,
            self.ui,
            self.rect,
            self.view,
            position,
            anchor.map(|point| (point, None)),
            previous,
        );
        if let Some(snap) = snap {
            point.point = snap.point;
        }
        Some((point, snap))
    }

    fn begin(&self, press: Pos2) -> Option<Capture> {
        let session = self.app.doc.sketch.session.as_ref()?;
        if crate::viewport_widgets::pointer_kind(self.ui.ctx())
            != crate::viewport_widgets::PointerKind::Pencil
            || !eligible(session)
            || self
                .ui
                .input(|input| input.modifiers != egui::Modifiers::NONE)
        {
            return None;
        }
        let (start, snap) = self.sample(press, None, None)?;
        Some(Capture {
            document: session.document.clone(),
            end: start.clone(),
            start,
            start_snap: snap,
            end_snap: snap,
            tool: session.tool,
            generation: session.draft_generation,
            sequence: session.request.sequence,
            construction: session.construction,
            press,
            promoted: false,
            cancelled: false,
            terminated: false,
        })
    }

    fn extend(&self, capture: &mut Capture, position: Pos2) {
        let threshold = self
            .ui
            .ctx()
            .options(|options| options.input_options.max_click_dist);
        capture.promoted |= capture.press.distance(position) > threshold;
        if let Some((end, snap)) = self.sample(
            position,
            Some(capture.start.point),
            capture.end_snap.as_ref(),
        ) {
            capture.end = end;
            capture.end_snap = snap;
        } else {
            capture.cancelled = true;
        }
    }

    fn step(
        &self,
        event: &Event,
        capture: &mut Option<Capture>,
        consumed: &mut bool,
    ) -> Option<SketchAction> {
        match event {
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: true,
                ..
            } => {
                if capture.as_ref().is_some_and(|capture| !capture.terminated) {
                    if let Some(capture) = capture.as_mut() {
                        capture.cancelled = true;
                    }
                    *consumed = true;
                } else {
                    *capture = self.begin(*pos);
                }
            }
            Event::PointerMoved(position) => {
                if let Some(capture) = capture.as_mut().filter(|capture| !capture.cancelled) {
                    self.extend(capture, *position);
                }
            }
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: false,
                ..
            } => {
                if let Some(mut done) = capture.take() {
                    if !done.cancelled {
                        self.extend(&mut done, *pos);
                    }
                    *consumed |= done.promoted || done.cancelled;
                    if done.promoted && !done.cancelled {
                        return Some(done.commit());
                    }
                }
            }
            Event::PointerGone
            | Event::Key {
                key: egui::Key::Escape,
                pressed: true,
                ..
            } => {
                if let Some(capture) = capture.as_mut() {
                    capture.cancelled = true;
                    capture.terminated = matches!(event, Event::PointerGone);
                    *consumed = true;
                }
            }
            _ => {}
        }
        None
    }
}

fn take_capture(app: &App, ui: &Ui, view: &ViewSnapshot) -> Option<Capture> {
    let mut capture = ui.ctx().data_mut(|data| {
        let capture = data.get_temp::<Capture>(id());
        data.remove::<Capture>(id());
        capture
    });
    let pencil = crate::viewport_widgets::pointer_kind(ui.ctx())
        == crate::viewport_widgets::PointerKind::Pencil;
    if let Some(capture) = capture.as_mut() {
        capture.cancelled |= !pencil
            || app
                .doc
                .sketch
                .session
                .as_ref()
                .is_none_or(|session| !capture.current(session, view));
    }
    capture
}

fn conflict(ctx: &egui::Context, events: &[Event]) -> bool {
    let pass = ctx.cumulative_pass_nr();
    ctx.data(|data| data.get_temp::<u64>(cancelled_pass_id())) == Some(pass)
        || events
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    Event::PointerButton {
                        button: PointerButton::Primary,
                        pressed: true,
                        ..
                    }
                )
            })
            .count()
            > 1
}

impl Canvas<'_> {
    fn feed(
        &self,
        events: &[Event],
        capture: &mut Option<Capture>,
        conflict: bool,
    ) -> (bool, Option<SketchAction>) {
        let (mut consumed, mut action) = (false, None);
        for event in events {
            if let Some(next) = self.step(event, capture, &mut consumed) {
                action = Some(next);
            }
            if conflict && let Some(capture) = capture.as_mut() {
                capture.cancelled = true;
                consumed = true;
            }
        }
        (consumed || capture.is_some(), action.filter(|_| !conflict))
    }

    fn paint(&self, capture: &Capture) {
        if capture.promoted
            && !capture.cancelled
            && let Some(session) = self.app.doc.sketch.session.as_ref()
        {
            crate::sketch_view::paint_draft(
                self.ui.painter(),
                self.rect,
                self.view,
                session.plane,
                &capture.preview(),
                &self.app.theme,
            );
        }
    }
}

pub(crate) fn viewport(app: &mut App, ui: &Ui, rect: Rect, view: &ViewSnapshot) -> bool {
    let mut capture = take_capture(app, ui, view);
    let events = ui.input(|input| input.raw.events.clone());
    let conflict = conflict(ui.ctx(), &events);
    let canvas = Canvas {
        app,
        ui,
        rect,
        view,
    };
    let (consumed, action) = canvas.feed(&events, &mut capture, conflict);
    if let Some(capture) = &capture {
        canvas.paint(capture);
    }
    ui.ctx().data_mut(|data| {
        if let Some(capture) = capture {
            data.insert_temp(id(), capture);
        }
    });
    if let Some(action) = action {
        app.sketch_dispatch(action);
    }
    consumed
}
