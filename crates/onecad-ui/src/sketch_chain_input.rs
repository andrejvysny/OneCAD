//! A primary press captures one chain visit; only its release may commit a drag arc.
use super::*;
use egui::{PointerButton, Pos2};
use onecad_interaction::{
    DocumentContext,
    sketch::{ChainCurve, SketchPhase, SketchSession, SketchSessionKey},
};
use onecad_scene::CaptureIdentity;

#[derive(Clone)]
struct Capture {
    key: SketchSessionKey,
    document: DocumentContext,
    view: CaptureIdentity,
    sequence: u64,
    anchor: onecad_core::math::Vec2,
    preceding: Option<ChainCurve>,
    press: Pos2,
    promoted: bool,
    cancelled: bool,
}

fn cache_id() -> egui::Id {
    egui::Id::unique("sketch-tangent-chain-drag")
}

impl Capture {
    fn owns_draft(&self, session: &SketchSession) -> bool {
        self.key == session.key
            && self.document == session.document
            && self.sequence == session.request.sequence
            && session.phase == SketchPhase::Active
            && session.tool == onecad_interaction::sketch::SketchTool::Line
            && session.draft.as_ref().is_some_and(|draft| {
                draft.anchor == self.anchor && draft.preceding == self.preceding
            })
    }

    fn current(&self, session: &SketchSession, view: CaptureIdentity) -> bool {
        self.owns_draft(session)
            && self.view == view
            && session.accepted_capture == Some(view)
            && session.can_tangent_arc()
    }
}

fn drag_threshold(press: Pos2, position: Pos2) -> bool {
    (position.x - press.x).abs() > 4.0 || (position.y - press.y).abs() > 4.0
}

pub(crate) fn pointer_owned(ctx: &egui::Context) -> bool {
    ctx.data(|data| {
        data.get_temp::<Capture>(cache_id())
            .is_some_and(|capture| !capture.cancelled)
    })
}

pub(crate) fn cancel(app: &mut App, ctx: &egui::Context) {
    let capture = ctx.data(|data| data.get_temp::<Capture>(cache_id()));
    if let Some(mut capture) = capture {
        if capture.promoted
            && !capture.cancelled
            && app
                .sketch
                .session
                .as_ref()
                .is_some_and(|session| capture.owns_draft(session))
        {
            app.sketch_dispatch(SketchAction::SetTangentArc {
                key: capture.key.clone(),
                enabled: false,
            });
        }
        capture.cancelled = true;
        ctx.data_mut(|data| data.insert_temp(cache_id(), capture));
    }
}

pub(crate) fn route_raw(app: &mut App, ctx: &egui::Context, raw: &egui::RawInput) {
    let lost = !raw.focused
        || app.composing
        || ctx.text_edit_focused()
        || raw
            .events
            .iter()
            .any(|event| matches!(event, egui::Event::Ime(_)))
        || crate::input::overlay_owns_keyboard(app, ctx)
        || raw.events.iter().any(|event| {
            matches!(
                event,
                egui::Event::PointerGone
                    | egui::Event::Key {
                        key: egui::Key::Escape,
                        pressed: true,
                        ..
                    }
            )
        });
    if lost {
        cancel(app, ctx);
    }
}

pub(super) fn viewport(
    app: &mut App,
    ui: &Ui,
    rect: Rect,
    response: &egui::Response,
    view: &ViewSnapshot,
) -> bool {
    let mut capture = ui.ctx().data_mut(|data| {
        let capture = data.get_temp::<Capture>(cache_id());
        data.remove::<Capture>(cache_id());
        capture
    });
    if ui.input(|input| input.pointer.button_pressed(PointerButton::Primary)) {
        capture = arm(app, ui, rect, response, view);
    }
    let Some(mut capture) = capture else {
        return false;
    };
    let current = app
        .sketch
        .session
        .as_ref()
        .is_some_and(|session| capture.current(session, view.identity()));
    let owns_draft = app
        .sketch
        .session
        .as_ref()
        .is_some_and(|session| capture.owns_draft(session));
    if !current {
        cancel_capture(app, &mut capture, owns_draft);
    }
    let position = response
        .hover_pos()
        .filter(|position| rect.contains(*position));
    let down = ui.input(|input| input.pointer.primary_down());
    let released = ui.input(|input| input.pointer.button_released(PointerButton::Primary));
    if position.is_none() {
        cancel_capture(app, &mut capture, owns_draft);
    }
    if let Some(position) = position.filter(|_| !capture.cancelled) {
        sample(app, ui, rect, view, &mut capture, position, released);
    }
    let consumed = capture.promoted || capture.cancelled;
    if down {
        ui.ctx()
            .data_mut(|data| data.insert_temp(cache_id(), capture));
    } else if !released && capture.promoted {
        cancel_capture(app, &mut capture, owns_draft);
    }
    consumed
}

fn sample(
    app: &mut App,
    ui: &Ui,
    rect: Rect,
    view: &ViewSnapshot,
    capture: &mut Capture,
    position: Pos2,
    released: bool,
) {
    let down = ui.input(|input| input.pointer.primary_down());
    if (down || released) && drag_threshold(capture.press, position) && !capture.promoted {
        capture.promoted = true;
        app.sketch_dispatch(SketchAction::SetTangentArc {
            key: capture.key.clone(),
            enabled: true,
        });
    }
    if capture.promoted {
        deliver(app, ui, rect, view, position, released);
    }
}

fn cancel_capture(app: &mut App, capture: &mut Capture, current: bool) {
    if current && capture.promoted && !capture.cancelled {
        app.sketch_dispatch(SketchAction::SetTangentArc {
            key: capture.key.clone(),
            enabled: false,
        });
    }
    capture.cancelled = true;
}

fn arm(
    app: &App,
    ui: &Ui,
    rect: Rect,
    response: &egui::Response,
    view: &ViewSnapshot,
) -> Option<Capture> {
    let session = app
        .sketch
        .session
        .as_ref()
        .filter(|session| session.can_tangent_arc())?;
    let draft = session.draft.as_ref()?;
    let press = ui.input(press_position).filter(|position| {
        crate::viewport_widgets::allows_pointer(ui, rect, *position)
            && ui
                .ctx()
                .layer_id_at(*position)
                .is_none_or(|layer| layer == response.layer_id)
    })?;
    if ui.input(|input| input.modifiers != egui::Modifiers::NONE) {
        return None;
    }
    Some(Capture {
        key: session.key.clone(),
        document: session.document.clone(),
        view: view.identity(),
        sequence: session.request.sequence,
        anchor: draft.anchor,
        preceding: draft.preceding,
        press,
        promoted: false,
        cancelled: false,
    })
}

fn press_position(input: &egui::InputState) -> Option<Pos2> {
    input
        .raw
        .events
        .iter()
        .find_map(|event| match event {
            egui::Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: true,
                ..
            } => Some(*pos),
            _ => None,
        })
        .or_else(|| input.pointer.press_origin())
}

fn deliver(
    app: &mut App,
    ui: &Ui,
    rect: Rect,
    view: &ViewSnapshot,
    position: Pos2,
    released: bool,
) {
    let Some(session) = app.sketch.session.as_ref() else {
        return;
    };
    let logical = DVec2::new(
        f64::from(position.x - rect.min.x),
        f64::from(position.y - rect.min.y),
    );
    let Ok(mut point) = onecad_interaction::sketch::capture_sketch_pointer(session, view, logical)
    else {
        return;
    };
    let snap = super::winning_snap(app, ui, rect, view, position);
    if let Some(snap) = snap {
        point.point = snap.point;
        super::paint(app, ui, rect, view, snap);
    }
    super::feedback::candidate(ui.painter(), rect, view, session, point.point, &app.theme);
    if released {
        let key = point.key.clone();
        app.sketch_dispatch(SketchAction::ClickInferred {
            point,
            ids: super::fresh_ids(),
            snap: snap.map(Box::new),
        });
        app.sketch_dispatch(SketchAction::SetTangentArc {
            key,
            enabled: false,
        });
    } else {
        app.sketch_dispatch(SketchAction::Move(point));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn batched_press_move_release_keeps_original_press() {
        let context = egui::Context::default();
        let press = Pos2::new(100.0, 200.0);
        let release = Pos2::new(130.0, 230.0);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let raw = egui::RawInput {
            events: vec![
                egui::Event::PointerMoved(press),
                button(press, true),
                egui::Event::PointerMoved(release),
                button(release, false),
            ],
            ..Default::default()
        };
        let mut output = context.run_ui(raw, |ui| {
            assert_eq!(ui.input(press_position), Some(press));
            assert!(ui.input(|input| input.pointer.button_pressed(PointerButton::Primary)));
            assert!(ui.input(|input| input.pointer.button_released(PointerButton::Primary)));
            assert!(!ui.input(|input| input.pointer.primary_down()));
            assert!(drag_threshold(ui.input(press_position).unwrap(), release));
        });
        output.textures_delta.clear();
    }

    #[test]
    fn threshold_is_strict_per_logical_axis() {
        let press = Pos2::new(100.0, 200.0);
        assert!(!drag_threshold(press, press + egui::vec2(4.0, -4.0)));
        assert!(!drag_threshold(press, press + egui::vec2(3.0, 3.0)));
        assert!(drag_threshold(press, press + egui::vec2(4.01, 0.0)));
        assert!(drag_threshold(press, press + egui::vec2(0.0, -4.01)));
    }
}
