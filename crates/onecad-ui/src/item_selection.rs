//! Tree and viewport selections share identities; hidden-sketch previews stay transient.
use crate::{app::App, bridge::Display};
use onecad_app::api::PreviewContext;
use onecad_core::{
    ids::{BodyId, SketchId},
    sketch::Sketch,
};
use onecad_interaction::Action;
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

pub(crate) fn body(app: &mut App, id: BodyId) {
    crate::pill::cancel_tool(app);
    app.dispatch(Action::ClearSelection);
    app.doc.selected_body = Some(id);
    app.doc.selected_feature = None;
    app.doc.topology.select(None);
    app.doc.selection_preview = Preview::default();
    app.draw_dirty = true;
}

pub(crate) fn sketch(app: &mut App, id: SketchId) {
    app.dispatch(Action::ClearSelection);
    app.doc.selected_body = None;
    app.doc.selected_feature = None;
    app.doc.topology.select(None);
    app.doc.selected_sketch = Some(id);
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

pub(crate) fn reconcile(app: &mut App, display: &Display) {
    app.doc.selected_body = app
        .doc
        .selected_body
        .filter(|id| display.projection.bodies.contains_key(&id.to_string()));
    app.doc.selected_sketch = app
        .doc
        .selected_sketch
        .filter(|id| display.projection.sketches.contains_key(&id.to_string()));
    app.doc.selected_feature = app
        .doc
        .selected_feature
        .filter(|id| display.records.iter().any(|record| record.record_id == *id));
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
    if !current || app.doc.selected_sketch.is_none() {
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
            Ok(sketch) if Some(sketch.id) == app.doc.selected_sketch => {
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

pub(crate) fn whole_body_selected(app: &App, id: BodyId) -> bool {
    app.doc.selected_body == Some(id) && app.doc.topology.selected_kind().is_none()
}

pub(crate) fn refresh(app: &mut App) {
    if let Some(id) = app.doc.selected_sketch
        && app.doc.selection_preview.expected.is_none()
        && !app.sketch_active()
    {
        request_preview(app, id);
    }
}
