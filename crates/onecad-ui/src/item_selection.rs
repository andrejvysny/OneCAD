//! Tree and viewport selections share identities; hidden-sketch previews stay transient.
use crate::{app::App, bridge::Display, tool_registry::SelectionKind};
use onecad_app::api::PreviewContext;
use onecad_core::{
    ids::{BodyId, SketchId},
    sketch::Sketch,
};
use onecad_interaction::{
    Action,
    selection_set::{SelectionItem, SelectionKind as ItemKind, SelectionMode, SelectionSet},
};
use std::sync::mpsc;

#[derive(Default)]
pub(crate) struct Preview {
    expected: Option<PreviewContext>,
    pending: Option<mpsc::Receiver<Result<Sketch, String>>>,
    sketch: Option<Sketch>,
}

#[cfg(test)]
impl Preview {
    pub(crate) fn ready(&self) -> bool {
        self.sketch.is_some()
    }
}

/// Applies one gesture to the shared selection and mirrors a lone profile into
/// the interaction state Extrude starts from.
pub(crate) fn apply(app: &mut App, mode: SelectionMode, items: Vec<SelectionItem>) {
    match app.doc.selection.apply(mode, items) {
        Ok(true) => changed(app),
        Ok(false) => {}
        Err(_) => app.doc.status = "Too many items selected; selection unchanged".into(),
    }
}

/// Tools follow one homogeneous kind; a mixed set offers no single-kind tools.
/// No tool takes a vertex or datum, so those select without a tool set.
pub(crate) fn model_kind(selection: &SelectionSet) -> SelectionKind {
    if selection.is_empty() {
        return SelectionKind::None;
    }
    match selection.single_kind() {
        Some(ItemKind::Face) => SelectionKind::Face,
        Some(ItemKind::Edge) => SelectionKind::Edge,
        Some(ItemKind::Body) => SelectionKind::Body,
        Some(ItemKind::Feature) => SelectionKind::Feature,
        Some(ItemKind::Profile) => SelectionKind::Region,
        // A sketch row keeps the idle tool set; the Inspector shows its block.
        Some(ItemKind::Sketch | ItemKind::Vertex | ItemKind::Datum) => SelectionKind::None,
        None => SelectionKind::Mixed,
    }
}

/// Profiles in the shared selection; Extrude takes exactly one.
pub(crate) fn profile_count(app: &App) -> usize {
    app.doc
        .selection
        .items()
        .iter()
        .filter(|item| matches!(item, SelectionItem::Profile(_)))
        .count()
}

pub(crate) fn clear(app: &mut App) {
    if app.doc.selection.clear() || app.doc.interaction.selection.is_some() {
        changed(app);
    }
}

/// Drops items that no longer exist or belong to another scene or context.
pub(crate) fn retain(app: &mut App, keep: impl FnMut(&SelectionItem) -> bool) {
    if !app.doc.selection.retain(keep).is_empty() {
        changed(app);
    }
}

fn changed(app: &mut App) {
    let profile = match app.doc.selection.items() {
        [SelectionItem::Profile(profile)] => Some(profile.clone()),
        _ => None,
    };
    if profile.as_deref() != app.doc.interaction.selection.as_ref() {
        app.dispatch(match profile {
            Some(profile) => Action::SelectProfile(profile),
            None => Action::ClearSelection,
        });
    }
    if app.doc.selected_sketch().is_none() {
        app.doc.selection_preview = Preview::default();
    }
    app.doc.topology.changed();
    app.draw_dirty = true;
}

pub(crate) fn body(app: &mut App, id: BodyId, mode: SelectionMode) {
    if crate::model_pick::staging(app) {
        crate::model_pick::stage(app, SelectionItem::Body(id), mode);
        return;
    }
    crate::pill::cancel_tool(app);
    apply(app, mode, vec![SelectionItem::Body(id)]);
}

pub(crate) fn sketch(app: &mut App, id: SketchId) {
    apply(app, SelectionMode::Replace, vec![SelectionItem::Sketch(id)]);
    app.ui.inspector_open = true;
    app.ui.inspector_tab = crate::ui_state::InspectorTab::Properties;
    request_preview(app, id);
    app.draw_dirty = true;
}

fn request_preview(app: &mut App, id: SketchId) {
    app.doc.selection_preview = Preview::default();
    let Some(display) = &app.doc.display else {
        return;
    };
    let expected = display.context.clone();
    let (sender, receiver) = mpsc::channel();
    let request = crate::sketch_services::Request::SelectionPreview {
        expected: expected.clone(),
        id,
        sender,
    };
    match app
        .doc
        .bridge
        .submit(crate::bridge::Command::Sketch(Box::new(request)))
    {
        Ok(()) => {
            app.doc.selection_preview = Preview {
                expected: Some(expected),
                pending: Some(receiver),
                sketch: None,
            }
        }
        Err(error) => app.doc.status = error,
    }
}

/// Interim policy: items scoped to a replaced scene or context are dropped, and
/// persistent items survive only while the projection still has them.
pub(crate) fn reconcile(app: &mut App, display: &Display) {
    let scene = display.scene.identity();
    let context = app.doc.interaction.context.clone();
    retain(app, |item| match item {
        SelectionItem::Body(id) => display.projection.bodies.contains_key(&id.to_string()),
        SelectionItem::Sketch(id) => display.projection.sketches.contains_key(&id.to_string()),
        SelectionItem::Feature(id) => display.records.iter().any(|r| r.record_id == *id),
        SelectionItem::Profile(profile) => Some(&profile.context) == context.as_ref(),
        other => !other.is_stale_for(scene),
    });
    if app.doc.selection_preview.expected.as_ref() != Some(&display.context) {
        app.doc.selection_preview = Preview::default();
    }
}

pub(crate) fn paint(
    app: &mut App,
    ui: &egui::Ui,
    rect: egui::Rect,
    view: &onecad_scene::ViewSnapshot,
) {
    let current = app.doc.display.as_ref().is_some_and(|display| {
        app.doc.selection_preview.expected.as_ref() == Some(&display.context)
    });
    if !current || app.doc.selected_sketch().is_none() {
        app.doc.selection_preview = Preview::default();
        return;
    }
    let reply = app
        .doc
        .selection_preview
        .pending
        .as_ref()
        .and_then(|receiver| receiver.try_recv().ok());
    if let Some(reply) = reply {
        app.doc.selection_preview.pending = None;
        match reply {
            Ok(sketch) if Some(sketch.id) == app.doc.selected_sketch() => {
                app.doc.selection_preview.sketch = Some(sketch)
            }
            Ok(_) => {}
            Err(error) => app.doc.status = format!("Sketch preview unavailable: {error}"),
        }
    }
    if let Some(sketch) = &app.doc.selection_preview.sketch {
        crate::sketch_view::paint_selected(ui.painter(), rect, view, sketch, app.theme.accent);
    }
}

/// A whole-body selection, or the body an editing Extrude targets.
pub(crate) fn whole_body_selected(app: &App, id: BodyId) -> bool {
    app.doc.whole_body_selected(id)
        || app
            .doc
            .interaction
            .tool
            .as_ref()
            .and_then(crate::extrude_ui::params)
            .is_some_and(|params| params.target_body == Some(id))
}

pub(crate) fn refresh(app: &mut App) {
    if let Some(id) = app.doc.selected_sketch()
        && app.doc.selection_preview.expected.is_none()
        && !app.sketch_active()
    {
        request_preview(app, id);
    }
}
