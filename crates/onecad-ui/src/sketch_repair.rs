//! Repair intents use the same application-owned, fenced sketch admission as dimensions.
use crate::app::App;
use onecad_core::{
    edit::SketchEditOp,
    ids::{ConstraintId, EntityId},
    sketch::{CurvePosition, SketchEntity},
};
use onecad_interaction::sketch::{SketchAction, SketchTarget};

#[derive(Clone)]
struct Hover {
    key: onecad_interaction::sketch::SketchSessionKey,
    document: onecad_interaction::DocumentContext,
    constraint: ConstraintId,
}

pub(crate) fn set_constraint_hover(
    app: &App,
    ctx: &egui::Context,
    constraint: Option<ConstraintId>,
) {
    let id = egui::Id::unique("sketch-constraint-hover");
    ctx.data_mut(|data| {
        data.remove::<Hover>(id);
        if let (Some(session), Some(constraint)) = (app.sketch.session.as_ref(), constraint) {
            data.insert_temp(
                id,
                Hover {
                    key: session.key.clone(),
                    document: session.document.clone(),
                    constraint,
                },
            );
        }
    });
}

pub(crate) fn hover_targets(
    ctx: &egui::Context,
    session: &onecad_interaction::sketch::SketchSession,
) -> Vec<SketchTarget> {
    let hover =
        ctx.data(|data| data.get_temp::<Hover>(egui::Id::unique("sketch-constraint-hover")));
    let Some(hover) =
        hover.filter(|hover| hover.key == session.key && hover.document == session.document)
    else {
        return Vec::new();
    };
    session
        .authoritative
        .constraints()
        .iter()
        .find(|constraint| constraint.id() == hover.constraint)
        .map(|constraint| {
            crate::constraint_authoring::constraint_targets(&session.authoritative, constraint)
        })
        .unwrap_or_default()
}

pub(crate) fn ready(app: &App) -> bool {
    !app.busy()
        && crate::sketch_ui::tools_enabled(app)
        && app
            .sketch
            .session
            .as_ref()
            .is_some_and(|session| session.draft.is_none())
}

fn entities(app: &App) -> Result<Vec<EntityId>, &'static str> {
    let session = app.sketch.session.as_ref().ok_or("No active sketch")?;
    let mut ids = Vec::new();
    for target in &session.selection {
        let id = match *target {
            SketchTarget::Entity(id) => id,
            SketchTarget::Point {
                entity,
                role: CurvePosition::Arbitrary,
            } if matches!(
                session.authoritative.get_entity(entity),
                Some(SketchEntity::Point { .. })
            ) =>
            {
                entity
            }
            _ => return Err("Select whole entities to change or delete geometry"),
        };
        let entity = session
            .authoritative
            .get_entity(id)
            .ok_or("Selection became stale")?;
        if entity.is_reference_locked() {
            return Err("Projected geometry is locked; detach its reference first");
        }
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    Ok(ids)
}

pub(crate) fn availability(app: &App, delete: bool) -> Result<(), &'static str> {
    if !ready(app) {
        return Err("Finish the current sketch command first");
    }
    let ids = entities(app)?;
    if delete && ids.is_empty() {
        return Err("Select geometry to delete");
    }
    Ok(())
}

pub(crate) fn toggle_construction(app: &mut App) {
    if availability(app, false).is_err() {
        return;
    }
    let Ok(ids) = entities(app) else {
        return;
    };
    let Some(session) = app.sketch.session.as_ref() else {
        return;
    };
    let key = session.key.clone();
    if ids.is_empty() {
        app.sketch_dispatch(SketchAction::SetConstructionMode {
            key,
            construction: !session.construction,
        });
        return;
    }
    let construction = !ids.iter().all(|id| {
        session
            .authoritative
            .get_entity(*id)
            .is_some_and(SketchEntity::is_construction)
    });
    let ops = ids
        .into_iter()
        .map(|entity| SketchEditOp::SetEntityConstruction {
            entity,
            construction,
        })
        .collect();
    app.sketch_dispatch(SketchAction::Repair { key, ops });
}

pub(crate) fn delete_selected(app: &mut App) {
    if availability(app, true).is_err() {
        return;
    }
    let Ok(ids) = entities(app) else {
        return;
    };
    let Some(session) = app.sketch.session.as_ref() else {
        return;
    };
    let key = session.key.clone();
    let ops = ids
        .into_iter()
        .map(|entity| SketchEditOp::RemoveEntity { entity })
        .collect();
    app.sketch_dispatch(SketchAction::Repair { key, ops });
}

pub(crate) fn remove_constraint(app: &mut App, constraint: ConstraintId) {
    if !ready(app) {
        return;
    }
    let Some(session) = app
        .sketch
        .session
        .as_ref()
        .filter(|session| session.authoritative.contains_constraint(constraint))
    else {
        return;
    };
    app.sketch_dispatch(SketchAction::Repair {
        key: session.key.clone(),
        ops: vec![SketchEditOp::RemoveConstraint { constraint }],
    });
}
