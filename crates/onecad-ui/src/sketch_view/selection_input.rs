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
    egui::Id::unique("sketch-selection-marquee")
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
    let mode = ui.input(|input| {
        if input.modifiers.shift || input.modifiers.mac_cmd {
            SelectionMode::Toggle
        } else {
            SelectionMode::Replace
        }
    });
    if response.drag_started_by(egui::PointerButton::Primary) {
        return Ok(begin_drag(ui, rect, view, session, mode));
    }
    if let Some(drag) = marquee {
        return continue_drag(ui, rect, response, view, session, drag);
    }
    if response.clicked_by(egui::PointerButton::Primary)
        && let Some(position) = response.interact_pointer_pos()
    {
        return click_selection(ui, rect, view, session, position, mode);
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
    rect: Rect,
    view: &ViewSnapshot,
    session: &SketchSession,
    mode: SelectionMode,
) -> Option<SketchAction> {
    let start = ui
        .input(|input| input.pointer.press_origin())
        .filter(|start| rect.contains(*start))?;
    ui.ctx().data_mut(|data| {
        data.insert_temp(
            cache_id(),
            Marquee {
                key: session.key.clone(),
                document: session.document.clone(),
                capture: view.identity(),
                start,
                mode,
            },
        )
    });
    Some(drag_action(session, view, true))
}

fn continue_drag(
    ui: &egui::Ui,
    rect: Rect,
    response: &egui::Response,
    view: &ViewSnapshot,
    session: &SketchSession,
    drag: Marquee,
) -> Result<Option<SketchAction>, String> {
    let Some(end) = ui
        .input(|input| input.pointer.hover_pos())
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
