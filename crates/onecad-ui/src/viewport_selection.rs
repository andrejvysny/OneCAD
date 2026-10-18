//! Transient topology highlights never become persistent authoring references.
use crate::{app::App, bridge::Display};
use onecad_render::{HighlightKind, HighlightTarget, PreparedHighlights};
use onecad_scene::{BodyId, DVec2, PickHit, SceneIdentity, SceneSnapshot, ViewSnapshot, Viewport};

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
}
impl Selection {
    pub fn select(&mut self, hit: Option<&PickHit<'_>>) {
        self.selected = hit.map(Target::from_hit);
        self.dirty = true;
    }
    pub fn invalidate(&mut self, scene: SceneIdentity) {
        self.selected = self.selected.take().filter(|target| target.scene == scene);
        self.hover = self.hover.take().filter(|target| target.scene == scene);
        self.dirty = true;
    }
}

pub fn hover(
    app: &mut App,
    ui: &egui::Ui,
    rect: egui::Rect,
    viewport: Viewport,
    display: &Display,
) {
    let pointer = ui.input(|input| input.pointer.hover_pos());
    let allowed = app.bridge.valid()
        && !sectioned(app)
        && !app.sketch_active()
        && !app.busy()
        && app.interaction.tool.is_none()
        && !crate::viewport_widgets::owns_pointer(app, ui, rect);
    let target = pointer
        .filter(|point| allowed && rect.contains(*point))
        .and_then(|point| {
            let view = ViewSnapshot::new(
                display.scene.identity(),
                app.view_number + 1,
                app.camera,
                viewport,
            )
            .ok()?;
            let logical = point - rect.min;
            let hit = onecad_scene::pick(
                &display.scene,
                &view,
                view.identity(),
                DVec2::new(f64::from(logical.x), f64::from(logical.y)),
            )
            .ok()??;
            Some(Target::from_hit(&hit))
        });
    if target != app.topology.hover {
        app.topology.hover = target;
        app.topology.dirty = true;
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

pub fn upload(app: &mut App, scene: &SceneSnapshot) -> Result<(), String> {
    if !app.topology.dirty {
        return Ok(());
    }
    let selected: Vec<_> = app
        .topology
        .selected
        .as_ref()
        .map(Target::borrowed)
        .into_iter()
        .collect();
    let hover = app.topology.hover.as_ref().map(Target::borrowed);
    let prepared = PreparedHighlights::new(
        scene,
        &selected,
        hover,
        app.renderer.highlight_preparation_budget(),
    )
    .map_err(|error| error.to_string())?;
    app.renderer
        .replace_highlights(prepared)
        .map_err(|error| error.to_string())?;
    app.topology.dirty = false;
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
        };
        selection.invalidate(SceneIdentity { scene: 2, ..scene });
        assert!(selection.selected.is_none() && selection.hover.is_none());
        assert!(selection.dirty);
    }
}
