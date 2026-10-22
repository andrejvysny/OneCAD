//! The override bar beside the last committed stroke, the visible equivalent of
//! correcting a recognition. Switching Line/Arc undoes the stroke's history step
//! and commits the other kind once the undo has landed, so history still holds
//! one step per stroke; Undo reverts the stroke. Automation ids are stable.
use super::{LastStroke, commit, last_id};
use crate::app::App;
use egui::{Order, Ui};
use onecad_interaction::sketch::{
    SketchAction, SketchPhase, SketchSession, SketchTool, capture_sketch_pointer,
    recognition::{Recognition, StrokeKind, StrokeShape},
};
use onecad_scene::ViewSnapshot;
use onecad_widgets::components::{
    button::{Button, ButtonKind},
    lists::menu_frame,
    tooltip::Tip,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Choice {
    Kind(StrokeKind),
    Undo,
}

const CHOICES: [(Choice, &str, &str); 3] = [
    (
        Choice::Kind(StrokeKind::Line),
        "As line",
        "onecad.penStroke.line",
    ),
    (
        Choice::Kind(StrokeKind::Arc),
        "As arc",
        "onecad.penStroke.arc",
    ),
    (Choice::Undo, "Undo stroke", "onecad.penStroke.undo"),
];

enum Settled {
    /// A stroke, undo or override batch is in flight.
    Hidden(LastStroke),
    Retired,
    Shown(LastStroke),
}

pub(super) fn override_bar(app: &mut App, ui: &Ui, rect: egui::Rect, view: &ViewSnapshot) {
    let Some(last) = ui.ctx().data(|data| data.get_temp::<LastStroke>(last_id())) else {
        return;
    };
    let last = match settle(app, last, rect, view) {
        Settled::Hidden(last) => return store(ui.ctx(), Some(last)),
        Settled::Retired => return store(ui.ctx(), None),
        Settled::Shown(last) => last,
    };
    match show(ui, &last) {
        Some(Choice::Kind(kind)) => store(ui.ctx(), switch(app, last, kind)),
        Some(Choice::Undo) => {
            store(ui.ctx(), None);
            crate::input::submit_history(app, crate::bridge::Command::Undo);
        }
        None => store(ui.ctx(), Some(last)),
    }
}

fn store(ctx: &egui::Context, last: Option<LastStroke>) {
    ctx.data_mut(|data| match last {
        Some(last) => {
            data.insert_temp(last_id(), last);
        }
        None => data.remove::<LastStroke>(last_id()),
    });
}

fn show(ui: &Ui, last: &LastStroke) -> Option<Choice> {
    let mut chosen = None;
    egui::Area::new(egui::Id::unique("onecad-pen-stroke-override"))
        .order(Order::Foreground)
        .fixed_pos(last.anchor + egui::vec2(16.0, 16.0))
        .show(ui.ctx(), |ui| {
            menu_frame(ui.ctx()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (choice, text, automation) in CHOICES {
                        let kind = match choice {
                            Choice::Kind(_) => ButtonKind::Secondary,
                            Choice::Undo => ButtonKind::Ghost,
                        };
                        let reason = refusal(last, choice);
                        let mut button = Button::new(text).kind(kind).enabled(reason.is_none());
                        if let Some(reason) = reason {
                            button = button.tip(Tip::new(text).reason(Some(reason)));
                        }
                        let response = ui.add(button);
                        ui.ctx().accesskit_node_builder(response.id, |node| {
                            node.set_author_id(automation)
                        });
                        if response.clicked() {
                            chosen = Some(choice);
                        }
                    }
                });
            });
        });
    chosen
}

/// Why a choice is unavailable: the stroke already is that kind, or its samples
/// do not support it (a closed loop as a line, a straight stroke as an arc).
fn refusal(last: &LastStroke, choice: Choice) -> Option<&'static str> {
    match choice {
        Choice::Kind(kind) if kind == last.kind => Some("The stroke already is this kind"),
        Choice::Kind(kind) => last.recognizer.shape_as(kind).err(),
        Choice::Undo => None,
    }
}

fn present(session: &SketchSession, last: &LastStroke) -> bool {
    let entities = session.authoritative.entities();
    last.entities
        .iter()
        .all(|id| entities.iter().any(|entity| entity.id() == *id))
}

fn undo_depth(session: &SketchSession) -> usize {
    session
        .history
        .as_ref()
        .map_or(0, |history| history.undo_depth)
}

/// The bar acts only while its stroke is the newest history step of the same
/// session and tool; any other edit, tool or session retires it.
fn settle(app: &mut App, mut last: LastStroke, rect: egui::Rect, view: &ViewSnapshot) -> Settled {
    let Some(session) = app.doc.sketch.session.as_ref() else {
        return Settled::Retired;
    };
    if session.key != last.key || session.tool != SketchTool::PenStroke {
        return Settled::Retired;
    }
    match session.phase {
        SketchPhase::Submitting => return Settled::Hidden(last),
        SketchPhase::Active => {}
        _ => return Settled::Retired,
    }
    if let Some(shape) = last.pending.take() {
        if present(session, &last) {
            // The undo was refused or reverted something else; its session status says so.
            return Settled::Retired;
        }
        return match redraw(app, &last, shape, rect, view) {
            Some(next) => Settled::Hidden(next),
            None => Settled::Retired,
        };
    }
    let depth = undo_depth(session);
    if !present(session, &last) || last.depth.is_some_and(|recorded| recorded != depth) {
        return Settled::Retired;
    }
    last.depth = Some(depth);
    Settled::Shown(last)
}

/// Commits the override shape after its stroke's undo landed.
fn redraw(
    app: &mut App,
    last: &LastStroke,
    shape: StrokeShape,
    rect: egui::Rect,
    view: &ViewSnapshot,
) -> Option<LastStroke> {
    let session = app.doc.sketch.session.as_ref()?;
    // A resolved undo clears the accepted capture; the canvas re-accepts it only
    // under a hovering pointer, and the pointer is on this bar.
    let accept = SketchAction::AcceptView {
        key: session.key.clone(),
        document: session.document.clone(),
        capture: view.identity(),
    };
    app.sketch_dispatch(accept);
    let session = app.doc.sketch.session.as_ref()?;
    let logical = last.anchor - rect.min;
    let at = onecad_scene::DVec2::new(f64::from(logical.x), f64::from(logical.y));
    let point = capture_sketch_pointer(session, view, at).ok()?;
    let entities = commit(app, point, Recognition::Shape(shape));
    (!entities.is_empty()).then(|| LastStroke {
        kind: shape.kind(),
        entities,
        depth: None,
        pending: None,
        ..last.clone()
    })
}

/// Undoes the stroke's step; [`settle`] commits `kind` once the undo landed. A
/// refused undo reports through the session status and retires the bar.
fn switch(app: &mut App, mut last: LastStroke, kind: StrokeKind) -> Option<LastStroke> {
    let shape = last.recognizer.shape_as(kind).ok()?;
    crate::input::submit_history(app, crate::bridge::Command::Undo);
    let undoing = app
        .doc
        .sketch
        .session
        .as_ref()
        .is_some_and(|session| session.phase == SketchPhase::Submitting);
    if !undoing {
        return None;
    }
    last.pending = Some(shape);
    Some(last)
}
