//! Viewport model picks become typed selection items (ONECAD-315 S-315b). The
//! selection mode comes from the primary press event itself, so modifier changes
//! later in the same batch or before the release cannot alter the gesture.
use crate::{app::App, bridge::Display};
use onecad_core::ids::{ElementId, TopoKey};
use onecad_interaction::{
    ProfileSelection, SessionPhase,
    selection_set::{
        OperandRefusal, SelectionItem, SelectionKind, SelectionMode, StagedOperands, TopologyId,
        TopologyRef,
    },
};
use onecad_scene::{DVec2, PickHit, PickKind, ViewSnapshot};

/// Operand stages an active tool feeds from picks. This is the S-315c hook only:
/// the tool session takes ownership of the stages in a later slice.
#[derive(Debug)]
pub(crate) struct Staging {
    pub(crate) operands: StagedOperands,
    pub(crate) refusal: Option<OperandRefusal>,
}

pub(crate) fn mode_for(modifiers: egui::Modifiers) -> SelectionMode {
    match (modifiers.shift, modifiers.command) {
        (false, false) => SelectionMode::Replace,
        (true, false) => SelectionMode::Add,
        (false, true) => SelectionMode::Toggle,
        (true, true) => SelectionMode::Subtract,
    }
}

/// The mode of the last primary press inside `rect` in one event batch.
pub(crate) fn press_mode(events: &[egui::Event], rect: egui::Rect) -> Option<SelectionMode> {
    events.iter().rev().find_map(|event| match event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers,
        } if rect.contains(*pos) => Some(mode_for(*modifiers)),
        _ => None,
    })
}

/// A tree row clicks on release, possibly frames after its press: the mode is
/// recorded from the press on the row (any button, for the context-menu click)
/// and read when the click completes. Call every frame the row is shown.
pub(crate) fn row_mode(ui: &egui::Ui, response: &egui::Response) -> SelectionMode {
    let press = ui.input(|input| {
        input.events.iter().rev().find_map(|event| match event {
            egui::Event::PointerButton {
                pos,
                pressed: true,
                modifiers,
                ..
            } if response.rect.contains(*pos) => Some(mode_for(*modifiers)),
            _ => None,
        })
    });
    let id = response.id.with("row-press-mode");
    ui.ctx().data_mut(|data| {
        if let Some(mode) = press {
            data.insert_temp(id, mode);
        }
        data.get_temp::<SelectionMode>(id)
            .unwrap_or(SelectionMode::Replace)
    })
}

fn press_id() -> egui::Id {
    crate::documents::scoped_id("model-pick-press-mode")
}

/// Records this frame's press mode and returns the mode a click completing in
/// this frame uses; a release forgets it so a later click never inherits it.
pub(crate) fn track(ui: &egui::Ui, rect: egui::Rect) -> SelectionMode {
    let (press, released) = ui.input(|input| {
        let released = input.events.iter().any(|event| {
            matches!(
                event,
                egui::Event::PointerButton {
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    ..
                }
            )
        });
        (press_mode(&input.events, rect), released)
    });
    let id = press_id();
    ui.ctx().data_mut(|data| {
        if let Some(mode) = press {
            data.insert_temp(id, mode);
        }
        let mode = data
            .get_temp::<SelectionMode>(id)
            .unwrap_or(SelectionMode::Replace);
        if released {
            data.remove::<SelectionMode>(id);
        }
        mode
    })
}

/// MESH1 labels a sub-shape with a snapshot TopoKey or a promoted ElementId.
pub(crate) fn topology_id(raw: &str) -> TopologyId {
    let key = TopoKey::new(raw);
    if key.is_valid() {
        TopologyId::Snapshot(key)
    } else {
        TopologyId::Element(ElementId::new(raw))
    }
}

pub(crate) fn label(id: &TopologyId) -> &str {
    match id {
        TopologyId::Snapshot(key) => key.as_str(),
        TopologyId::Element(id) => id.as_str(),
    }
}

pub(crate) fn hit_item(hit: &PickHit<'_>) -> SelectionItem {
    let topology = TopologyRef {
        scene: hit.capture.scene,
        body: hit.body_id,
        id: topology_id(hit.topology_id),
    };
    match hit.kind {
        PickKind::Face => SelectionItem::Face(topology),
        PickKind::Edge => SelectionItem::Edge(topology),
    }
}

/// An active Editing tool that owns operand stages.
pub(crate) fn staging(app: &App) -> bool {
    app.doc.operand_staging.is_some()
        && app
            .doc
            .interaction
            .tool
            .as_ref()
            .is_some_and(|tool| tool.phase == SessionPhase::Editing)
}

/// Picking needs no tool, or a tool that feeds operand stages.
pub(crate) fn allowed(app: &App) -> bool {
    app.doc.interaction.tool.is_none() || staging(app)
}

pub(crate) fn policy(app: &App) -> onecad_scene::PickPolicy {
    let edges_only = staging(app)
        && app.doc.operand_staging.as_ref().is_some_and(|staging| {
            staging
                .operands
                .active()
                .accepts
                .iter()
                .eq([SelectionKind::Edge].iter())
        });
    if edges_only {
        onecad_scene::PickPolicy::EdgesOnly
    } else {
        crate::viewport::pick_policy(app.renderer.options().display_mode)
    }
}

pub(crate) fn pick(
    app: &mut App,
    display: &Display,
    view: &ViewSnapshot,
    logical: DVec2,
    whole_body: bool,
    mode: SelectionMode,
) {
    let profile = display
        .profiles
        .iter()
        .filter_map(|profile| profile.pick(view, logical).map(|hit| (profile, hit)))
        .min_by(|a, b| a.1.1.total_cmp(&b.1.1));
    let body =
        onecad_scene::pick_with_policy(&display.scene, view, view.identity(), logical, policy(app))
            .ok()
            .flatten();
    let profile = profile
        .filter(|(_, (_, distance))| {
            !whole_body && crate::viewport::profile_visible(body.as_ref(), *distance)
        })
        .zip(app.doc.interaction.context.clone())
        .map(|((profile, (anchor, _)), context)| {
            let mut reference = profile.reference.clone();
            reference.region_anchor = Some(anchor);
            SelectionItem::Profile(Box::new(ProfileSelection {
                profile: reference,
                context,
                capture: view.identity(),
            }))
        });
    let is_profile = profile.is_some();
    let item = profile.or_else(|| {
        body.as_ref().map(|hit| {
            if whole_body {
                SelectionItem::Body(hit.body_id)
            } else {
                hit_item(hit)
            }
        })
    });
    if staging(app) {
        // A miss never clears staged operands.
        if let Some(item) = item {
            stage(app, item, mode);
        }
        return;
    }
    match item {
        Some(item) => crate::item_selection::apply(app, mode, vec![item]),
        None if mode == SelectionMode::Replace => crate::item_selection::clear(app),
        None => {}
    }
    if is_profile {
        app.doc.status = "Profile selected".into();
    }
}

/// Feeds one pick into the active stage; a refused pick keeps the stage.
pub(crate) fn stage(app: &mut App, item: SelectionItem, mode: SelectionMode) {
    let Some(staging) = app.doc.operand_staging.as_mut() else {
        return;
    };
    let result = match mode {
        SelectionMode::Replace => replace_active(&mut staging.operands, item),
        SelectionMode::Add => staging.operands.offer(item).map(|_| ()),
        SelectionMode::Toggle => staging.operands.toggle(item).map(|_| ()),
        SelectionMode::Subtract => {
            staging.operands.remove(&item);
            Ok(())
        }
    };
    staging.refusal = result.err();
    app.doc.topology.changed();
    app.draw_dirty = true;
}

/// Replace trial-offers on a copy, so a refusal never wipes the active stage.
fn replace_active(
    operands: &mut StagedOperands,
    item: SelectionItem,
) -> Result<(), OperandRefusal> {
    let mut trial = operands.clone();
    for current in trial.active().items.clone() {
        trial.remove(&current);
    }
    trial.offer(item)?;
    *operands = trial;
    Ok(())
}

#[cfg(test)]
pub(crate) fn select_face_for_test(
    app: &mut App,
    scene: onecad_scene::SceneIdentity,
    body: onecad_core::ids::BodyId,
    face: &str,
) {
    let item = SelectionItem::Face(TopologyRef {
        scene,
        body,
        id: topology_id(face),
    });
    crate::item_selection::apply(app, SelectionMode::Replace, vec![item]);
}

#[cfg(test)]
#[path = "model_pick_tests.rs"]
mod tests;
