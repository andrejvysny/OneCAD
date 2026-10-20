//! Pickable native dimensions use accepted captures.
//! Label offsets and visibility remain transient until a versioned placement contract exists.
//! They survive camera/value changes within a visit, then reset on re-entry or fresh reopen.
//! Hidden or collision-suppressed labels remain editable through the inspector.
mod cache;
mod geometry;
mod paint;
use crate::app::App;
use cache::{Cache, cache_id};
use egui::{Rect, Ui};
use onecad_interaction::sketch::{SketchAction, SketchPhase, SketchSession, SketchTool};
use onecad_scene::ViewSnapshot;

fn current(session: &SketchSession, rect: Rect, view: &ViewSnapshot) -> bool {
    session.phase == SketchPhase::Active
        && session.document.scene == view.identity().scene
        && session.accepted_capture == Some(view.identity())
        && rect.is_positive()
        && (f64::from(rect.width()) - view.viewport().width).abs() < 0.001
        && (f64::from(rect.height()) - view.viewport().height).abs() < 0.001
}

fn prepare_capture(app: &mut App, rect: Rect, view: &ViewSnapshot) -> bool {
    if app.view.as_ref().map(ViewSnapshot::identity) != Some(view.identity())
        || !app.viewport_ready()
        || !app.bridge.valid()
    {
        return false;
    }
    let Some(session) = app.sketch.session.as_ref() else {
        return false;
    };
    if session.phase != SketchPhase::Active
        || session.document.scene != view.identity().scene
        || session.draft.is_some()
    {
        return false;
    }
    if app.document_state.as_ref().is_some_and(|state| {
        state.runtime_session != session.document.runtime_session
            || state.revision != session.document.revision
    }) {
        return false;
    }
    app.sketch_dispatch(SketchAction::AcceptView {
        key: session.key.clone(),
        document: session.document.clone(),
        capture: view.identity(),
    });
    app.sketch
        .session
        .as_ref()
        .is_some_and(|session| current(session, rect, view))
}

pub fn show(app: &mut App, ui: &mut Ui, rect: Rect, view: &ViewSnapshot) -> bool {
    if !prepare_capture(app, rect, view) {
        cancel_drag(ui.ctx());
        return false;
    }
    let Some(session) = app.sketch.session.as_ref() else {
        return false;
    };
    let previous = ui.ctx().data_mut(|data| {
        let previous = data.get_temp::<Cache>(cache_id());
        data.remove::<Cache>(cache_id());
        previous
    });
    let mut cache = Cache::sync(previous, session, view.identity());
    let badges: Vec<_> = session
        .authoritative
        .constraints()
        .iter()
        .filter_map(|constraint| {
            let (text, accessible) = paint::label(constraint)?;
            Some(paint::Badge {
                id: constraint.id(),
                text,
                accessible,
                witness: geometry::witness(&session.authoritative, constraint)?,
            })
        })
        .take(cache::MAX_BADGES)
        .collect();
    let occupied = exclusions(app, ui, rect);
    let enabled = !app.busy() && crate::viewport_widgets::gesture_allowed(ui);
    let canvas = paint::Canvas {
        rect,
        view,
        plane: session.plane,
        theme: &app.theme,
        enabled,
    };
    let output = paint::show(ui, &canvas, &badges, &mut cache, occupied);
    let edit = output.edit.filter(|_| {
        app.sketch.session.as_ref().is_some_and(|session| {
            cache.current(session, view.identity()) && current(session, rect, view)
        })
    });
    ui.ctx()
        .data_mut(|data| data.insert_temp(cache_id(), cache));
    if let Some(id) = edit {
        app.sketch_dispatch(SketchAction::SetTool(SketchTool::Select));
        crate::constraint_authoring::edit_dimension(app, ui.ctx(), id);
    }
    output.owns_pointer
}

fn exclusions(app: &App, ui: &Ui, rect: Rect) -> Vec<Rect> {
    let layers = ui.ctx().memory(|memory| memory.areas().visible_layer_ids());
    let mut result: Vec<_> = layers
        .into_iter()
        .filter(|layer| layer.order > ui.layer_id().order)
        .filter_map(|layer| {
            let bounds = egui::AreaState::load(ui.ctx(), layer.id)?.rect();
            Some(
                ui.ctx()
                    .layer_transform_to_global(layer)
                    .map_or(bounds, |transform| transform * bounds),
            )
        })
        .collect();
    let chrome = crate::layout::chrome(ui.ctx().content_rect(), &app.ui);
    let host = Rect::from_min_max(
        egui::pos2(rect.left(), chrome.view_cluster.y),
        egui::pos2(chrome.view_cluster.x, rect.bottom()),
    );
    result.push(crate::navigation_cube::rect(host));
    result.push(chrome.view_controls);
    result
}

pub fn owns_pointer(app: &App, ui: &Ui) -> bool {
    owns_position(app, ui, ui.input(|input| input.pointer.hover_pos()))
}

pub(crate) fn owns_position(app: &App, ui: &Ui, position: Option<egui::Pos2>) -> bool {
    let Some(session) = app.sketch.session.as_ref() else {
        return false;
    };
    let Some(view) = app.view.as_ref() else {
        return false;
    };
    let Some(cache) = ui
        .ctx()
        .data(|data| data.get_temp::<Cache>(cache_id()))
        .filter(|cache| cache.current(session, view.identity()))
    else {
        return false;
    };
    cache.drag.is_some()
        || position.is_some_and(|at| {
            cache
                .entries
                .iter()
                .any(|entry| !entry.hidden && entry.last_rect.is_some_and(|rect| rect.contains(at)))
        })
}

fn cancel_drag(ctx: &egui::Context) {
    ctx.data_mut(|data| {
        if let Some(mut cache) = data.get_temp::<Cache>(cache_id()) {
            cache.drag = None;
            for entry in &mut cache.entries {
                entry.last_rect = None;
            }
            data.insert_temp(cache_id(), cache);
        }
    });
}

pub fn show_all(app: &App, ctx: &egui::Context) {
    let Some(session) = app.sketch.session.as_ref() else {
        return;
    };
    ctx.data_mut(|data| {
        if let Some(mut cache) = data
            .get_temp::<Cache>(cache_id())
            .filter(|cache| cache.key == session.key)
        {
            for entry in &mut cache.entries {
                entry.hidden = false;
            }
            data.insert_temp(cache_id(), cache);
        }
    });
}

#[cfg(test)]
pub(crate) fn badge_rect(ctx: &egui::Context, id: onecad_core::ids::ConstraintId) -> Option<Rect> {
    ctx.data(|data| data.get_temp::<Cache>(cache_id()))?
        .entries
        .iter()
        .find(|entry| entry.id == id)?
        .last_rect
}

#[cfg(test)]
mod tests;
