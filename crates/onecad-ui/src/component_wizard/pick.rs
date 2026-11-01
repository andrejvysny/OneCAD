//! Viewport side of attachment picking: the next click on the source body names an
//! attachment. While it waits `App::busy` is true, so the click is not also a selection.
use crate::{app::App, component_wizard::model::Phase};
use onecad_interaction::selection_set::TopologyId;
use onecad_scene::{DVec2, PickPolicy, ViewSnapshot};

pub(crate) fn viewport(
    app: &mut App,
    ui: &egui::Ui,
    rect: egui::Rect,
    response: &egui::Response,
    (view, display): (&ViewSnapshot, &crate::bridge::Display),
    overlay_owns: bool,
) {
    if !super::picking(app) || overlay_owns || !response.clicked() {
        return;
    }
    let navigating = app.doc.navigation.active(ui.input(|input| input.time));
    let Some(position) = response
        .interact_pointer_pos()
        .filter(|point| rect.contains(*point))
        .filter(|point| crate::viewport_widgets::allows_pointer(ui, rect, *point))
        .filter(|_| !navigating)
    else {
        return;
    };
    let logical = DVec2::new(
        f64::from(position.x - rect.min.x),
        f64::from(position.y - rect.min.y),
    );
    let hit = onecad_scene::pick_with_tolerance(
        &display.scene,
        view,
        view.identity(),
        logical,
        PickPolicy::FacesAndEdges,
        crate::viewport_widgets::edge_tolerance(ui.ctx()),
    )
    .ok()
    .flatten();
    let snapshot = app
        .doc
        .interaction
        .context
        .as_ref()
        .map(|c| c.snapshot_id.0);
    let Some(wizard) = app
        .doc
        .wizard
        .as_mut()
        .filter(|w| w.phase == Phase::Picking)
    else {
        return;
    };
    let target = hit
        .filter(|hit| hit.body_id == wizard.body)
        .map(|hit| crate::model_pick::topology_id(hit.topology_id));
    match (target, snapshot) {
        (Some(TopologyId::Snapshot(key)), Some(snapshot)) => {
            wizard.resolve_pick(key.as_str().to_owned(), snapshot);
        }
        (Some(TopologyId::Element(_)), _) => {
            wizard.error = Some("That element has no stable key to attach to; pick another".into());
        }
        _ => {
            wizard.error = Some(format!(
                "Click {} to place the attachment",
                wizard.body_name
            ))
        }
    }
}

/// Esc cancels an attachment pick and is not also a deselect.
pub(crate) fn keys(app: &mut App, raw: &mut egui::RawInput) {
    if !super::picking(app) {
        return;
    }
    let before = raw.events.len();
    raw.events.retain(|event| {
        !matches!(
            event,
            egui::Event::Key {
                key: egui::Key::Escape,
                pressed: true,
                modifiers,
                ..
            } if modifiers.is_none()
        )
    });
    if raw.events.len() != before
        && let Some(wizard) = app.doc.wizard.as_mut()
    {
        wizard.cancel_pick();
    }
}
