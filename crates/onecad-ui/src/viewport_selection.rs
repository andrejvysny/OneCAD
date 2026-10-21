//! Transient topology highlights never become persistent authoring references.
use crate::{app::App, bridge::Display};
use onecad_render::{HighlightKind, HighlightTarget, PreparedHighlights};
use onecad_scene::{BodyId, DVec2, PickHit, SceneIdentity, SceneSnapshot, ViewSnapshot};
use std::sync::{Arc, mpsc};

#[derive(Clone, Debug, PartialEq)]
struct Target {
    scene: SceneIdentity,
    body_id: BodyId,
    mesh_revision: u64,
    kind: HighlightKind,
    topology_id: String,
}
impl Target {
    fn from_hit(hit: &PickHit<'_>) -> Self {
        Self {
            scene: hit.capture.scene,
            body_id: hit.body_id,
            mesh_revision: hit.mesh_revision,
            kind: hit.kind,
            topology_id: hit.topology_id.to_owned(),
        }
    }
    fn borrowed(&self) -> HighlightTarget<'_> {
        HighlightTarget {
            scene: self.scene,
            body_id: self.body_id,
            mesh_revision: self.mesh_revision,
            kind: self.kind,
            topology_id: &self.topology_id,
        }
    }
}

#[derive(Default)]
pub struct Selection {
    selected: Option<Target>,
    hover: Option<Target>,
    dirty: bool,
    sequence: u64,
    pending: Option<mpsc::Receiver<Delivery>>,
}
struct Delivery {
    sequence: u64,
    scene: SceneIdentity,
    result: Result<PreparedHighlights, String>,
}
impl Selection {
    pub fn selected_face(&self, scene: SceneIdentity) -> Option<(BodyId, String)> {
        self.selected
            .as_ref()
            .filter(|target| target.scene == scene && target.kind == HighlightKind::Face)
            .map(|target| (target.body_id, target.topology_id.clone()))
    }
    #[cfg(test)]
    pub(crate) fn select_face_for_test(
        &mut self,
        scene: SceneIdentity,
        body_id: BodyId,
        face: &str,
    ) {
        self.selected = Some(Target {
            scene,
            body_id,
            mesh_revision: 0,
            kind: HighlightKind::Face,
            topology_id: face.into(),
        });
    }
    pub fn selected_kind(&self) -> Option<(HighlightKind, BodyId)> {
        self.selected
            .as_ref()
            .map(|target| (target.kind, target.body_id))
    }
    pub fn select(&mut self, hit: Option<&PickHit<'_>>) {
        self.selected = hit.map(Target::from_hit);
        self.changed();
    }
    pub fn invalidate(&mut self, scene: SceneIdentity) {
        self.selected = self.selected.take().filter(|target| target.scene == scene);
        self.hover = self.hover.take().filter(|target| target.scene == scene);
        self.changed();
    }
    fn changed(&mut self) {
        self.sequence = self.sequence.wrapping_add(1);
        self.dirty = true;
    }
    pub fn invalidate_pending(&mut self) {
        self.selected = None;
        self.hover = None;
        self.changed();
    }
    fn accepts(&self, sequence: u64, scene: SceneIdentity, current: SceneIdentity) -> bool {
        self.sequence == sequence && scene == current
    }
}

pub fn hover(
    app: &mut App,
    ui: &egui::Ui,
    rect: egui::Rect,
    view: &ViewSnapshot,
    display: &Display,
) {
    let pointer = ui.input(|input| input.pointer.hover_pos());
    let allowed = app.viewport_ready()
        && app.doc.bridge.valid()
        && !sectioned(app)
        && !app.sketch_active()
        && !app.busy()
        && app.doc.interaction.tool.is_none()
        && !crate::viewport_widgets::owns_pointer(app, ui, rect);
    let target = pointer
        .filter(|point| allowed && crate::viewport_widgets::allows_pointer(ui, rect, *point))
        .and_then(|point| {
            let logical = point - rect.min;
            let hit = onecad_scene::pick_with_policy(
                &display.scene,
                view,
                view.identity(),
                DVec2::new(f64::from(logical.x), f64::from(logical.y)),
                crate::viewport::pick_policy(app.renderer.options().display_mode),
            )
            .ok()??;
            Some(Target::from_hit(&hit))
        });
    if target != app.doc.topology.hover {
        app.doc.topology.hover = target;
        app.doc.topology.changed();
        app.draw_dirty = true;
    }
}

pub fn sectioned(app: &App) -> bool {
    let options = app
        .pending_display_options
        .as_ref()
        .unwrap_or_else(|| app.renderer.options());
    options.section[..3].iter().any(|value| *value != 0.0)
}

pub fn upload(
    app: &mut App,
    scene: &Arc<SceneSnapshot>,
    ctx: &egui::Context,
) -> Result<(), String> {
    if app.doc.topology.dirty {
        app.renderer
            .clear_highlights()
            .map_err(|error| error.to_string())?;
    }
    if let Some(receiver) = &app.doc.topology.pending {
        match receiver.try_recv() {
            Ok(delivery) => {
                app.doc.topology.pending = None;
                if app
                    .doc
                    .topology
                    .accepts(delivery.sequence, delivery.scene, scene.identity())
                {
                    app.renderer
                        .replace_highlights(delivery.result?)
                        .map_err(|error| error.to_string())?;
                    app.draw_dirty = true;
                    ctx.request_repaint();
                }
            }
            Err(mpsc::TryRecvError::Empty) => return Ok(()),
            Err(mpsc::TryRecvError::Disconnected) => {
                app.doc.topology.pending = None;
                return Err("highlight preparation worker stopped".into());
            }
        }
    }
    if app.doc.topology.dirty {
        start_preparation(app, scene.clone(), ctx.clone())?;
    }
    Ok(())
}

fn start_preparation(
    app: &mut App,
    scene: Arc<SceneSnapshot>,
    ctx: egui::Context,
) -> Result<(), String> {
    let lease = app
        .doc
        .bridge
        .shared
        .host_budget
        .reserve(scene.resident_bytes())?;
    let sequence = app.doc.topology.sequence;
    let selected = app.doc.topology.selected.clone();
    let hover = app.doc.topology.hover.clone();
    let budget = app.renderer.preparation_budget();
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("viewport-highlights".into())
        .spawn(move || {
            let _lease = lease;
            let targets: Vec<_> = selected
                .as_ref()
                .map(Target::borrowed)
                .into_iter()
                .collect();
            let result = PreparedHighlights::new(
                &scene,
                &targets,
                hover.as_ref().map(Target::borrowed),
                budget,
            )
            .map_err(|error| error.to_string());
            let _ = sender.send(Delivery {
                sequence,
                scene: scene.identity(),
                result,
            });
            ctx.request_repaint();
        })
        .map_err(|error| error.to_string())?;
    app.doc.topology.pending = Some(receiver);
    app.doc.topology.dirty = false;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacement_scene_discards_selected_and_hover_topology() {
        let scene = SceneIdentity {
            session: 1,
            scene: 1,
        };
        let target = Target {
            scene,
            body_id: BodyId::new(),
            mesh_revision: 7,
            kind: HighlightKind::Face,
            topology_id: "face:1".into(),
        };
        let mut selection = Selection {
            selected: Some(target.clone()),
            hover: Some(target),
            dirty: false,
            ..Default::default()
        };
        assert!(selection.selected_face(scene).is_some());
        assert!(
            selection
                .selected_face(SceneIdentity { scene: 2, ..scene })
                .is_none()
        );
        selection.invalidate(SceneIdentity { scene: 2, ..scene });
        assert!(selection.selected.is_none() && selection.hover.is_none());
        assert!(selection.dirty);
    }
    #[test]
    fn old_highlights_rejected_after_selection_scene_or_session_change() {
        let scene = SceneIdentity {
            session: 1,
            scene: 1,
        };
        let mut selection = Selection::default();
        let sequence = selection.sequence;
        assert!(selection.accepts(sequence, scene, scene));
        selection.select(None);
        assert!(!selection.accepts(sequence, scene, scene));
        assert!(!selection.accepts(
            selection.sequence,
            scene,
            SceneIdentity { scene: 2, ..scene }
        ));
        assert!(!selection.accepts(
            selection.sequence,
            scene,
            SceneIdentity {
                session: 2,
                ..scene
            }
        ));
    }
}
