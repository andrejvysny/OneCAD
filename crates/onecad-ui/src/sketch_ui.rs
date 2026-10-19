//! Sketch chrome and input emit typed actions; solved geometry stays application-owned.
use crate::{
    app::App,
    bridge::Command,
    sketch_services::{Request, effect_tag},
};
use onecad_core::{
    ids::{ConstraintId, EntityId, SketchId},
    sketch::{Sketch, SketchAttachment, WorldPlane},
};
use onecad_interaction::sketch::{
    PrimitiveIds, SketchAction, SketchPhase, SketchStatus, SketchTool,
};
use onecad_scene::{DVec2, ViewSnapshot};

impl App {
    pub fn sketch_active(&self) -> bool {
        self.sketch
            .session
            .as_ref()
            .is_some_and(|session| session.phase != SketchPhase::Closed)
    }
    pub fn sketch_dispatch(&mut self, action: SketchAction) {
        for effect in self.sketch.dispatch(action) {
            let tag = effect_tag(&effect).clone();
            if let Err(message) = self
                .bridge
                .submit(Command::Sketch(Box::new(Request::Effect(effect))))
            {
                self.sketch_dispatch(SketchAction::Failed { tag, message });
            }
        }
        if let Some(session) = &self.sketch.session {
            self.status = match &session.status {
                SketchStatus::Starting => "Opening sketch…".into(),
                SketchStatus::Entered(solve) => {
                    format!(
                        "{}: {} DOF; {:?}",
                        if solve.solve_current {
                            "Sketch"
                        } else {
                            "Cached sketch solve"
                        },
                        solve.dof,
                        solve.status
                    )
                }
                SketchStatus::Pending => "Solving sketch…".into(),
                SketchStatus::Accepted => {
                    "Sketch batch accepted; waiting for solved snapshot".into()
                }
                SketchStatus::Solved(solve) => {
                    format!("Sketch: {} DOF; {:?}", solve.dof, solve.status)
                }
                SketchStatus::Refused(message) | SketchStatus::Failed { message, .. } => {
                    message.clone()
                }
                SketchStatus::Finished { kept_reason, .. } => {
                    kept_reason.clone().unwrap_or("Sketch finished".into())
                }
                SketchStatus::Discarded(_) => "Sketch edits discarded".into(),
            };
        }
    }
    pub fn new_sketch(&mut self) {
        if !self.viewport_ready() || crate::viewport_selection::sectioned(self) {
            self.status = "Turn off section display before editing geometry".into();
            return;
        }
        let Some(context) = self.interaction.context.clone() else {
            return;
        };
        self.dispatch(onecad_interaction::Action::Cancel);
        let sketch = Sketch::new(
            SketchId::new(),
            "Sketch",
            SketchAttachment::World {
                plane: WorldPlane::XY,
            },
        );
        self.sketch_dispatch(SketchAction::Begin {
            sketch: Box::new(sketch),
            create: true,
            context,
        });
    }
    pub fn edit_sketch(&mut self, id: SketchId) {
        if !self.viewport_ready() || crate::viewport_selection::sectioned(self) {
            self.status = "Turn off section display before editing geometry".into();
            return;
        }
        let Some(context) = self.interaction.context.clone() else {
            return;
        };
        self.dispatch(onecad_interaction::Action::Cancel);
        self.submit(Command::Sketch(Box::new(Request::Load { id, context })));
    }
}

/// Sketch tools are clickable only on an active, bridge-valid session.
pub fn tools_enabled(app: &App) -> bool {
    app.viewport_ready() && can_finish(app)
}
fn can_finish(app: &App) -> bool {
    app.sketch
        .session
        .as_ref()
        .is_some_and(|session| session.phase == SketchPhase::Active && app.bridge.valid())
}
/// Finish and Discard stay available on a faulted session so it can be left.
pub fn can_exit(app: &App) -> bool {
    can_finish(app) || session_faulted(app)
}
fn session_faulted(app: &App) -> bool {
    app.sketch
        .session
        .as_ref()
        .is_some_and(|session| session.phase == SketchPhase::Faulted)
}

pub fn viewport(
    app: &mut App,
    ui: &egui::Ui,
    rect: egui::Rect,
    response: &egui::Response,
    view: &ViewSnapshot,
) {
    let Some(session) = app.sketch.session.as_ref() else {
        return;
    };
    if session.phase == SketchPhase::Closed {
        return;
    }
    if session.phase != SketchPhase::Active || (!app.viewport_ready() || !app.bridge.valid()) {
        return;
    }
    let Some(position) = response.hover_pos() else {
        return;
    };
    if session.tool == SketchTool::Select {
        if response.clicked() {
            let entities = crate::sketch_view::pick_entity(
                rect,
                view,
                session,
                position,
                crate::sketch_view::DEFAULT_PICK_RADIUS,
            )
            .into_iter()
            .collect();
            app.sketch_dispatch(SketchAction::Select {
                key: session.key.clone(),
                entities,
            });
        }
        return;
    }
    let logical = DVec2::new(
        f64::from(position.x - rect.min.x),
        f64::from(position.y - rect.min.y),
    );
    let Ok(point) = onecad_interaction::sketch::capture_sketch_pointer(session, view, logical)
    else {
        return;
    };
    if response.clicked() {
        app.sketch_dispatch(SketchAction::Click {
            point,
            ids: PrimitiveIds {
                entities: std::array::from_fn(|_| EntityId::new()),
                constraints: std::array::from_fn(|_| ConstraintId::new()),
            },
        });
    } else if ui.input(|input| input.pointer.delta() != egui::Vec2::ZERO) {
        app.sketch_dispatch(SketchAction::Move(point));
    }
}
