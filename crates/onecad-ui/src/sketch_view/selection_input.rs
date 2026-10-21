use super::*;
use onecad_interaction::{
    DocumentContext,
    sketch::{SelectionMode, SketchAction, SketchSessionKey, SketchTarget},
};
use onecad_scene::CaptureIdentity;

#[derive(Clone)]
struct Marquee {
    key: SketchSessionKey,
    document: DocumentContext,
    capture: CaptureIdentity,
    start: Pos2,
    mode: SelectionMode,
}

fn cache_id() -> egui::Id {
    crate::documents::scoped_id("sketch-selection-marquee")
}

pub fn selection_input(
    ui: &egui::Ui,
    rect: Rect,
    response: &egui::Response,
    view: &ViewSnapshot,
    session: &SketchSession,
) -> Result<Option<SketchAction>, String> {
    let marquee = ui
        .ctx()
        .data_mut(|data| {
            let drag = data.get_temp::<Marquee>(cache_id());
            data.remove::<Marquee>(cache_id());
            drag
        })
        .filter(|drag| {
            drag.key == session.key
                && drag.document == session.document
                && drag.capture == view.identity()
                && session.selection_dragging
        });
    let press = primary_press(ui, rect, view, session)?;
    if response.drag_started_by(egui::PointerButton::Primary) {
        return Ok(press.map(|press| begin_drag(ui, view, session, press)));
    }
    if let Some(drag) = marquee {
        return continue_drag(ui, rect, response, view, session, drag);
    }
    if response.clicked_by(egui::PointerButton::Primary)
        && let Some(press) = press
    {
        return click_selection(ui, rect, view, session, press.start, press.mode);
    }
    Ok(None)
}

fn click_selection(
    ui: &egui::Ui,
    rect: Rect,
    view: &ViewSnapshot,
    session: &SketchSession,
    position: Pos2,
    mode: SelectionMode,
) -> Result<Option<SketchAction>, String> {
    let targets = match pick_entity(rect, view, session, position, DEFAULT_PICK_RADIUS) {
        PickResult::None => vec![],
        PickResult::Unique(target) => vec![target],
        PickResult::Ambiguous(targets) => {
            if targets.len() > onecad_interaction::sketch::MAX_SELECTION_TARGETS {
                return Err("Too many overlapping sketch targets; zoom in before selecting".into());
            }
            super::target_chooser::open(ui.ctx(), session, view, targets, mode, position);
            return Ok(None);
        }
    };
    Ok(Some(selection(session, view, targets, mode)))
}

fn begin_drag(
    ui: &egui::Ui,
    view: &ViewSnapshot,
    session: &SketchSession,
    press: Marquee,
) -> SketchAction {
    ui.ctx()
        .data_mut(|data| data.insert_temp(cache_id(), press));
    drag_action(session, view, true)
}

fn press_id() -> egui::Id {
    crate::documents::scoped_id("sketch-selection-primary-press")
}

pub(crate) fn cancel_pointer(ctx: &egui::Context) {
    ctx.data_mut(|data| {
        data.remove::<Marquee>(press_id());
        data.remove::<Marquee>(cache_id());
    });
}

fn primary_transitions(ui: &egui::Ui) -> Vec<(Pos2, bool, egui::Modifiers)> {
    ui.input(|input| {
        input
            .events
            .iter()
            .filter_map(|event| {
                if let egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers,
                } = event
                {
                    Some((*pos, *pressed, *modifiers))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
    })
}

fn take_press(ui: &egui::Ui, view: &ViewSnapshot, session: &SketchSession) -> Option<Marquee> {
    ui.ctx()
        .data_mut(|data| {
            let press = data.get_temp::<Marquee>(press_id());
            data.remove::<Marquee>(press_id());
            press
        })
        .filter(|press| {
            press.key == session.key
                && press.document == session.document
                && press.capture == view.identity()
        })
}

fn primary_press(
    ui: &egui::Ui,
    rect: Rect,
    view: &ViewSnapshot,
    session: &SketchSession,
) -> Result<Option<Marquee>, String> {
    let mut press = take_press(ui, view, session);
    if ui.input(|input| !input.focused || input.events.contains(&egui::Event::PointerGone)) {
        return Ok(None);
    }
    let transitions = primary_transitions(ui);
    let presses = transitions
        .iter()
        .filter(|(_, pressed, _)| *pressed)
        .count();
    let releases = transitions.len() - presses;
    if presses > 1 || releases > 1 || presses == 1 && press.is_some() {
        return Err("Multiple pointer cycles in one frame; select again".into());
    }
    let mut released = false;
    for (position, pressed, modifiers) in transitions {
        if pressed && rect.contains(position) {
            press = Some(Marquee {
                key: session.key.clone(),
                document: session.document.clone(),
                capture: view.identity(),
                start: position,
                mode: if modifiers.shift || modifiers.mac_cmd {
                    SelectionMode::Toggle
                } else {
                    SelectionMode::Replace
                },
            });
        } else if !pressed {
            released = true;
        }
    }
    if !released && let Some(press) = &press {
        ui.ctx()
            .data_mut(|data| data.insert_temp(press_id(), press.clone()));
    }
    Ok(press)
}

fn continue_drag(
    ui: &egui::Ui,
    rect: Rect,
    response: &egui::Response,
    view: &ViewSnapshot,
    session: &SketchSession,
    drag: Marquee,
) -> Result<Option<SketchAction>, String> {
    let release = primary_transitions(ui)
        .into_iter()
        .find_map(|(position, pressed, _)| (!pressed).then_some(position));
    let Some(end) = release
        .or_else(|| ui.input(|input| input.pointer.hover_pos()))
        .filter(|end| rect.contains(*end))
    else {
        return Ok(Some(drag_action(session, view, false)));
    };
    if response.drag_stopped_by(egui::PointerButton::Primary) {
        let targets = marquee_targets(rect, view, session, drag.start, end)
            .ok_or("Selection exceeds supported bounds or capture is stale")?;
        return Ok(Some(selection(session, view, targets, drag.mode)));
    }
    if !ui.input(|input| input.pointer.primary_down()) {
        return Ok(Some(drag_action(session, view, false)));
    }
    let color = if end.x < drag.start.x {
        Color32::from_rgb(80, 180, 130)
    } else {
        Color32::from_rgb(80, 140, 220)
    };
    ui.painter().rect_stroke(
        Rect::from_two_pos(drag.start, end),
        0.0,
        Stroke::new(1.0, color),
        egui::StrokeKind::Inside,
    );
    ui.ctx().data_mut(|data| data.insert_temp(cache_id(), drag));
    Ok(None)
}

fn drag_action(session: &SketchSession, view: &ViewSnapshot, active: bool) -> SketchAction {
    SketchAction::SelectionDrag {
        key: session.key.clone(),
        document: session.document.clone(),
        capture: view.identity(),
        active,
    }
}

fn selection(
    session: &SketchSession,
    view: &ViewSnapshot,
    targets: Vec<SketchTarget>,
    mode: SelectionMode,
) -> SketchAction {
    SketchAction::Select {
        key: session.key.clone(),
        document: session.document.clone(),
        capture: view.identity(),
        targets,
        mode,
    }
}

#[cfg(test)]
mod tests;
