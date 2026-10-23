//! A host's temporary ink belongs to the editor presentation that received it.
use crate::app::App;
use onecad_interaction::{
    RequestTag,
    sketch::{SketchSessionKey, SketchTool},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OwnerIdentity {
    pub tab: u64,
    pub editor_visible: bool,
}

#[derive(Clone, Debug, PartialEq)]
enum Authoring {
    Sketch(SketchSessionKey, SketchTool),
    Model(RequestTag),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PresentationContext {
    pub owner: OwnerIdentity,
    pub runtime: Option<String>,
    pub capture: Option<onecad_scene::CaptureIdentity>,
    pub camera: Option<onecad_scene::Camera>,
    pub viewport: Option<egui::Rect>,
    authoring: Option<Authoring>,
}

impl PresentationContext {
    pub fn allows_prediction(&self) -> bool {
        self.owner.editor_visible
            && self.runtime.is_some()
            && self.capture.is_some()
            && self.viewport.is_some()
            && self.authoring.is_some()
    }
}

pub(super) fn capture(app: &App, ctx: &egui::Context) -> PresentationContext {
    let authoring =
        if app.ui.start_open || app.busy() || crate::input::overlay_owns_keyboard(app, ctx) {
            None
        } else if app.sketch_active() {
            app.doc
                .sketch
                .session
                .as_ref()
                .map(|session| Authoring::Sketch(session.key.clone(), session.tool))
        } else {
            app.doc
                .interaction
                .tool
                .as_ref()
                .filter(|_| super::drag_authoring(app))
                .map(|tool| {
                    let mut tag = tool.tag.clone();
                    // Preview replies advance the request sequence inside the same authoring gesture.
                    tag.sequence = 0;
                    Authoring::Model(tag)
                })
        };
    PresentationContext {
        owner: OwnerIdentity {
            tab: app.doc.tab,
            editor_visible: !app.ui.start_open,
        },
        runtime: app
            .doc
            .document_state
            .as_ref()
            .map(|state| state.runtime_session.clone()),
        capture: app.doc.view.map(|view| view.identity()),
        camera: Some(app.doc.camera),
        viewport: crate::viewport_widgets::canvas_rect(ctx),
        authoring,
    }
}

pub(super) fn clips(ctx: &egui::Context) -> Vec<egui::Rect> {
    crate::viewport_widgets::canvas_clips(ctx)
}

pub(super) fn text_scope(app: &App) -> String {
    format!(
        "{:?}",
        (
            app.doc.tab,
            !app.ui.start_open,
            app.doc
                .document_state
                .as_ref()
                .map(|state| &state.runtime_session),
            app.doc.sketch.session.as_ref().map(|session| &session.key),
            app.doc
                .interaction
                .tool
                .as_ref()
                .map(|tool| (tool.tag.record_id, tool.tag.gesture)),
        )
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> PresentationContext {
        let view = onecad_scene::ViewSnapshot::new(
            onecad_scene::SceneIdentity {
                session: 1,
                scene: 2,
            },
            3,
            Default::default(),
            onecad_scene::Viewport {
                width: 800.0,
                height: 600.0,
                scale_factor: 2.0,
            },
        )
        .unwrap();
        PresentationContext {
            owner: OwnerIdentity {
                tab: 4,
                editor_visible: true,
            },
            runtime: Some("runtime".into()),
            capture: Some(view.identity()),
            camera: Some(view.camera()),
            viewport: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            authoring: Some(Authoring::Sketch(
                SketchSessionKey {
                    document_id: "doc".into(),
                    runtime_session: "runtime".into(),
                    sketch_id: onecad_core::ids::SketchId::new(),
                    generation: 5,
                },
                SketchTool::PenStroke,
            )),
        }
    }

    #[test]
    fn prediction_permission_requires_visible_active_author_and_real_presentation() {
        let original = context();
        assert!(original.allows_prediction());
        for loss in 0..5 {
            let mut context = original.clone();
            match loss {
                0 => context.owner.editor_visible = false,
                1 => context.authoring = None,
                2 => context.runtime = None,
                3 => context.capture = None,
                _ => context.viewport = None,
            }
            assert!(!context.allows_prediction());
            assert_ne!(original, context);
        }
        let mut camera = original.clone();
        camera.camera.as_mut().unwrap().target.x += 1.0;
        assert_ne!(
            original, camera,
            "pending camera motion also retires temporary ink"
        );
    }
}
