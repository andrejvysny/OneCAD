//! The visible winner and the authored inference share one accepted capture.
use crate::{
    app::App,
    sketch_snapping::{SnapRequest, SnapSettings, resolve_snap},
};
use egui::{Rect, Ui};
use onecad_core::{
    ids::{ConstraintId, EntityId},
    sketch::CurvePosition,
};
use onecad_interaction::sketch::{
    PrimitiveIds, SketchAction, SketchSession, SketchSessionKey, SketchTarget, SketchTool,
    inference::AcceptedSnap,
};
use onecad_scene::{DVec2, ViewSnapshot};

#[path = "sketch_chain_input.rs"]
pub(crate) mod chain_input;
#[path = "sketch_creation_feedback.rs"]
mod feedback;
pub(crate) use feedback::anchors as paint_anchors;

#[derive(Clone)]
struct CachedSnap {
    snap: AcceptedSnap,
    key: SketchSessionKey,
    tool: SketchTool,
    draft_generation: u64,
}

impl CachedSnap {
    fn current(&self, session: &SketchSession) -> bool {
        self.key == session.key
            && self.tool == session.tool
            && self.draft_generation == session.draft_generation
    }
}

pub(crate) fn clear_candidate(ctx: &egui::Context) {
    ctx.data_mut(|data| {
        data.remove::<CachedSnap>(crate::documents::scoped_id("sketch-winning-snap"))
    });
}

pub(crate) fn viewport(
    app: &mut App,
    ui: &Ui,
    rect: Rect,
    response: &egui::Response,
    view: &ViewSnapshot,
) {
    if chain_input::viewport(app, ui, rect, response, view) {
        return;
    }
    let Some(session) = app.doc.sketch.session.as_ref() else {
        return;
    };
    let Some(position) = response
        .hover_pos()
        .filter(|position| crate::viewport_widgets::allows_pointer(ui, rect, *position))
    else {
        clear_candidate(ui.ctx());
        return;
    };
    let logical = DVec2::new(
        f64::from(position.x - rect.min.x),
        f64::from(position.y - rect.min.y),
    );
    let Ok(mut point) = onecad_interaction::sketch::capture_sketch_pointer(session, view, logical)
    else {
        clear_candidate(ui.ctx());
        return;
    };
    let snap = winning_snap(app, ui, rect, view, position);
    if let Some(snap) = snap {
        point.point = snap.point;
        paint(app, ui, rect, view, snap);
    }
    feedback::candidate(ui.painter(), rect, view, session, point.point, &app.theme);
    if response.clicked() {
        app.sketch_dispatch(SketchAction::ClickInferred {
            point,
            ids: fresh_ids(),
            snap: snap.map(Box::new),
        });
        if let Some(session) = app.doc.sketch.session.as_ref() {
            paint_anchors(ui.painter(), rect, view, session, &app.theme);
        }
    } else if ui.input(|input| input.pointer.delta() != egui::Vec2::ZERO)
        || session
            .draft
            .as_ref()
            .is_some_and(|draft| draft.cursor != point.point)
    {
        app.sketch_dispatch(SketchAction::Move(point));
    }
}

fn winning_snap(
    app: &App,
    ui: &Ui,
    rect: Rect,
    view: &ViewSnapshot,
    position: egui::Pos2,
) -> Option<AcceptedSnap> {
    let session = app.doc.sketch.session.as_ref()?;
    let cache = crate::documents::scoped_id("sketch-winning-snap");
    let previous = ui
        .ctx()
        .data(|data| data.get_temp::<CachedSnap>(cache))
        .filter(|previous| previous.current(session));
    let snap = optional_snaps(app.ui.sketch_snap, ui.input(|input| input.modifiers))
        .then(|| {
            resolve_snap(&SnapRequest {
                sketch: &session.authoritative,
                plane: session.plane,
                view,
                rect,
                pointer: position,
                anchor: session.draft.as_ref().map(snap_anchor),
                previous: previous.as_ref().map(|previous| &previous.snap),
                generation: session.request.sequence,
                settings: SnapSettings {
                    grid_step: app.ui.sketch_grid_snap.then(|| {
                        f64::from(onecad_render::grid_step(
                            view.camera().offset.length() as f32
                        ))
                    }),
                    ..Default::default()
                },
            })
        })
        .flatten();
    ui.ctx().data_mut(|data| {
        data.remove::<CachedSnap>(cache);
        if let Some(snap) = snap {
            data.insert_temp(
                cache,
                CachedSnap {
                    snap,
                    key: session.key.clone(),
                    tool: session.tool,
                    draft_generation: session.draft_generation,
                },
            );
        }
    });
    snap
}

pub(crate) fn snap_anchor(
    draft: &onecad_interaction::sketch::SketchDraft,
) -> (onecad_core::math::Vec2, Option<SketchTarget>) {
    let reference = draft
        .anchor_id
        .map(|entity| SketchTarget::Point {
            entity,
            role: CurvePosition::Arbitrary,
        })
        .or_else(|| {
            draft.preceding.map(|curve| SketchTarget::Point {
                entity: curve.entity,
                role: curve.role,
            })
        })
        .or_else(|| match draft.anchor_snap.map(|snap| snap.intent) {
            // A first click welded onto existing geometry aligns later points to that geometry.
            Some(onecad_interaction::sketch::inference::SnapIntent::Coincident(target)) => {
                Some(target)
            }
            _ => None,
        });
    (draft.anchor, reference)
}

fn optional_snaps(enabled: bool, modifiers: egui::Modifiers) -> bool {
    enabled && !modifiers.alt
}

fn fresh_ids() -> PrimitiveIds {
    PrimitiveIds {
        entities: std::array::from_fn(|_| EntityId::new()),
        constraints: std::array::from_fn(|_| ConstraintId::new()),
    }
}

fn paint(app: &App, ui: &Ui, rect: Rect, view: &ViewSnapshot, snap: AcceptedSnap) {
    if let Some(session) = app.doc.sketch.session.as_ref() {
        feedback::snap(ui.painter(), rect, view, session, snap, &app.theme);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn alt_suppresses_optional_inference_without_changing_the_snap_setting() {
        assert!(optional_snaps(true, egui::Modifiers::NONE));
        assert!(!optional_snaps(true, egui::Modifiers::ALT));
        assert!(!optional_snaps(false, egui::Modifiers::NONE));
    }
}
