//! Transient topology highlights never become persistent authoring references.
use crate::{app::App, bridge::Display};
use onecad_interaction::selection_set::SelectionItem;
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
    /// A selected face or edge drawn against `scene`; other kinds have no highlight.
    fn from_item(item: &SelectionItem, scene: &SceneSnapshot) -> Option<Self> {
        let (kind, topology) = match item {
            SelectionItem::Face(topology) => (HighlightKind::Face, topology),
            SelectionItem::Edge(topology) => (HighlightKind::Edge, topology),
            _ => return None,
        };
        let body = scene
            .bodies()
            .iter()
            .find(|body| body.body_id() == topology.body)?;
        (topology.scene == scene.identity()).then(|| Self {
            scene: topology.scene,
            body_id: topology.body,
            mesh_revision: body.mesh_revision(),
            kind,
            topology_id: crate::model_pick::label(&topology.id).to_owned(),
        })
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

/// Hover plus the highlight pipeline for the shared model selection.
#[derive(Default)]
pub struct Selection {
    hover: Option<Target>,
    dirty: bool,
    sequence: u64,
    pending: Option<mpsc::Receiver<Delivery>>,
    /// Draw count of the last uploaded highlights (selection plus hover).
    #[cfg(test)]
    pub(crate) uploaded: usize,
    #[cfg(test)]
    pub(crate) hover_picks: u64,
}
struct Delivery {
    sequence: u64,
    scene: SceneIdentity,
    result: Result<PreparedHighlights, String>,
}
impl Selection {
    pub fn invalidate(&mut self, scene: SceneIdentity) {
        self.hover = self.hover.take().filter(|target| target.scene == scene);
        self.changed();
    }
    pub(crate) fn changed(&mut self) {
        self.sequence = self.sequence.wrapping_add(1);
        self.dirty = true;
    }
    pub fn invalidate_pending(&mut self) {
        self.hover = None;
        self.changed();
        self.pending = None;
    }
    #[cfg(test)]
    pub(crate) fn settled(&self) -> bool {
        !self.dirty && self.pending.is_none()
    }
    fn accepts(&self, sequence: u64, scene: SceneIdentity, current: SceneIdentity) -> bool {
        self.sequence == sequence && scene == current
    }

    fn clear_hover(&mut self) -> bool {
        if self.hover.take().is_none() {
            return false;
        }
        self.changed();
        true
    }
}

pub(crate) fn clear_hover(app: &mut App) {
    app.draw_dirty |= app.doc.topology.clear_hover();
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
        && !app.doc.navigation.active(ui.input(|input| input.time))
        && app.doc.bridge.valid()
        && !sectioned(app)
        && !app.sketch_active()
        && !app.busy()
        && crate::model_pick::allowed(app)
        && !crate::viewport_widgets::owns_pointer(app, ui, rect);
    let target = pointer
        .filter(|point| allowed && crate::viewport_widgets::allows_pointer(ui, rect, *point))
        .and_then(|point| {
            let _timing = crate::performance::span(
                crate::performance::Stage::HoverPick,
                ui.ctx().cumulative_frame_nr(),
                [
                    view.identity().view,
                    app.doc.navigation.generation(),
                    display.scene.identity().scene,
                ],
            );
            #[cfg(test)]
            {
                app.doc.topology.hover_picks += 1;
            }
            let logical = point - rect.min;
            let hit = onecad_scene::pick_with_tolerance(
                &display.scene,
                view,
                view.identity(),
                DVec2::new(f64::from(logical.x), f64::from(logical.y)),
                crate::model_pick::policy(app),
                crate::viewport_widgets::edge_tolerance(ui.ctx()),
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
                    let prepared = delivery.result?;
                    #[cfg(test)]
                    {
                        app.doc.topology.uploaded = prepared.len();
                    }
                    app.renderer
                        .replace_highlights(prepared)
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
    let selected = highlighted(app, &scene);
    let hover = app.doc.topology.hover.clone();
    if selected.is_empty() && hover.is_none() {
        app.doc.topology.dirty = false;
        #[cfg(test)]
        {
            app.doc.topology.uploaded = 0;
        }
        return Ok(());
    }
    let lease = app.doc.bridge.shared.host_budget.reserve_kind(
        crate::budget::MAX_PREPARED_BYTES as usize,
        crate::budget::Kind::Highlight,
    )?;
    let source_lease = app
        .doc
        .display
        .as_ref()
        .filter(|display| display.scene.identity() == scene.identity())
        .map(|display| display._lease.clone())
        .ok_or("highlight scene has no retained source owner")?;
    let sequence = app.doc.topology.sequence;
    let budget = app
        .renderer
        .preparation_budget()
        .limit_cpu_prepared_bytes(crate::budget::MAX_PREPARED_BYTES);
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("viewport-highlights".into())
        .spawn(move || {
            let source = (scene, source_lease);
            let targets: Vec<_> = selected.iter().map(Target::borrowed).collect();
            let result = PreparedHighlights::new_retained(
                &source.0,
                &targets,
                hover.as_ref().map(Target::borrowed),
                budget,
                Some(Arc::new(lease)),
            )
            .map_err(|error| error.to_string());
            let _ = sender.send(Delivery {
                sequence,
                scene: source.0.identity(),
                result,
            });
            ctx.request_repaint();
        })
        .map_err(|error| error.to_string())?;
    app.doc.topology.pending = Some(receiver);
    app.doc.topology.dirty = false;
    Ok(())
}

/// Staged operands while a tool feeds them, else the shared selection.
fn highlighted(app: &App, scene: &SceneSnapshot) -> Vec<Target> {
    let items: Vec<&SelectionItem> = match &app.doc.operand_staging {
        Some(staging) if crate::model_pick::staging(app) => staging
            .operands
            .stages()
            .iter()
            .flat_map(|stage| stage.items.iter())
            .collect(),
        _ => app.doc.selection.items().iter().collect(),
    };
    items
        .into_iter()
        .filter_map(|item| Target::from_item(item, scene))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use onecad_interaction::selection_set::{TopologyId, TopologyRef};
    use onecad_scene::{MeshLimits, SceneBody, SceneLimits};

    #[test]
    fn replacement_scene_discards_hover_topology() {
        let scene = SceneIdentity {
            session: 1,
            scene: 1,
        };
        let mut selection = Selection {
            hover: Some(Target {
                scene,
                body_id: BodyId::new(),
                mesh_revision: 7,
                kind: HighlightKind::Face,
                topology_id: "f:1".into(),
            }),
            ..Default::default()
        };
        selection.invalidate(SceneIdentity { scene: 2, ..scene });
        assert!(selection.hover.is_none());
        assert!(selection.dirty);
    }

    #[test]
    fn clearing_transient_hover_invalidates_it_once_without_repeated_highlight_work() {
        let mut selection = Selection {
            hover: Some(Target {
                scene: SceneIdentity {
                    session: 1,
                    scene: 1,
                },
                body_id: BodyId::new(),
                mesh_revision: 7,
                kind: HighlightKind::Face,
                topology_id: "f:1".into(),
            }),
            ..Default::default()
        };
        assert!(selection.clear_hover());
        assert!(selection.hover.is_none());
        assert_eq!(selection.sequence, 1);
        assert!(!selection.clear_hover());
        assert_eq!(
            selection.sequence, 1,
            "continuous navigation must not churn highlight work"
        );
    }

    #[test]
    fn every_selected_face_and_edge_of_the_current_scene_is_highlighted() {
        let body = SceneBody::new(
            BodyId::new(),
            4,
            Arc::from(&include_bytes!("../../../fixtures/render/edge-policy/box.mesh1")[..]),
            MeshLimits::default(),
        )
        .unwrap();
        let id = body.body_id();
        let identity = SceneIdentity {
            session: 1,
            scene: 3,
        };
        let scene = SceneSnapshot::new(identity, vec![body], SceneLimits::default()).unwrap();
        let topology = |scene, raw: &str| TopologyRef {
            scene,
            body: id,
            id: TopologyId::Snapshot(onecad_core::ids::TopoKey::new(raw)),
        };
        let stale = SceneIdentity {
            scene: 2,
            ..identity
        };
        let items = [
            SelectionItem::Face(topology(identity, "f:1")),
            SelectionItem::Face(topology(identity, "f:2")),
            SelectionItem::Edge(topology(identity, "e:3")),
            SelectionItem::Body(id),
            SelectionItem::Face(topology(stale, "f:4")),
        ];
        let targets: Vec<_> = items
            .iter()
            .filter_map(|item| Target::from_item(item, &scene))
            .collect();
        let labels: Vec<_> = targets
            .iter()
            .map(|t| (t.kind, t.topology_id.as_str(), t.mesh_revision))
            .collect();
        assert_eq!(
            labels,
            [
                (HighlightKind::Face, "f:1", 4),
                (HighlightKind::Face, "f:2", 4),
                (HighlightKind::Edge, "e:3", 4)
            ]
        );
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
        selection.changed();
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
