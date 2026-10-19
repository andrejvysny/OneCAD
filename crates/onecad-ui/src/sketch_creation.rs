//! The visible winner and the authored inference share one accepted capture.
use crate::{
    app::App,
    sketch_snapping::{SnapRequest, SnapSettings, resolve_snap},
};
use egui::{Align2, FontId, Rect, Stroke, Ui};
use onecad_core::{
    ids::{ConstraintId, EntityId},
    sketch::CurvePosition,
};
use onecad_interaction::sketch::{
    PrimitiveIds, SketchAction, SketchTarget,
    inference::{AcceptedSnap, SnapIntent},
};
use onecad_scene::{DVec2, DVec3, ViewSnapshot};

pub(crate) fn viewport(
    app: &mut App,
    ui: &Ui,
    rect: Rect,
    response: &egui::Response,
    view: &ViewSnapshot,
) {
    let Some(session) = app.sketch.session.as_ref() else {
        return;
    };
    let Some(position) = response.hover_pos() else {
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
    let snap = winning_snap(app, ui, rect, view, position);
    if let Some(snap) = snap {
        point.point = snap.point;
        paint(app, ui, rect, view, snap);
    }
    if response.clicked() {
        app.sketch_dispatch(SketchAction::ClickInferred {
            point,
            ids: fresh_ids(),
            snap: snap.map(Box::new),
        });
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
    let session = app.sketch.session.as_ref()?;
    let cache = egui::Id::unique("sketch-winning-snap");
    let previous = ui.ctx().data(|data| data.get_temp::<AcceptedSnap>(cache));
    let snap = optional_snaps(app.ui.sketch_snap, ui.input(|input| input.modifiers))
        .then(|| {
            resolve_snap(&SnapRequest {
                sketch: &session.authoritative,
                plane: session.plane,
                view,
                rect,
                pointer: position,
                anchor: session.draft.as_ref().map(|draft| {
                    (
                        draft.anchor,
                        draft.anchor_id.map(|entity| SketchTarget::Point {
                            entity,
                            role: CurvePosition::Arbitrary,
                        }),
                    )
                }),
                previous: previous.as_ref(),
                generation: session.request.sequence,
                settings: SnapSettings {
                    grid_step: app.ui.sketch_grid_snap.then_some(1.0),
                    ..Default::default()
                },
            })
        })
        .flatten();
    ui.ctx().data_mut(|data| {
        data.remove::<AcceptedSnap>(cache);
        if let Some(snap) = snap {
            data.insert_temp(cache, snap);
        }
    });
    snap
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
    let Some(session) = app.sketch.session.as_ref() else {
        return;
    };
    let world = session.plane.to_world(snap.point);
    let Some(screen) = view.project(DVec3::new(world.x, world.y, world.z)) else {
        return;
    };
    let position = rect.min + egui::vec2(screen.logical.x as f32, screen.logical.y as f32);
    let painter = ui.painter().with_clip_rect(rect);
    painter.circle_stroke(position, 6.0, Stroke::new(1.5, app.theme.accent));
    let label = if snap.intent == SnapIntent::GeometryOnly {
        format!("{} guide", snap.kind.label())
    } else {
        format!("{} constraint", snap.kind.label())
    };
    painter.text(
        position + egui::vec2(10.0, -12.0),
        Align2::LEFT_BOTTOM,
        label,
        FontId::proportional(12.0),
        app.theme.accent,
    );
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
