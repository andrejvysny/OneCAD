use super::*;
use crate::app::App;
use onecad_interaction::{
    DocumentContext,
    sketch::{SelectionMode, SketchAction, SketchSessionKey, SketchTarget},
};
use onecad_scene::CaptureIdentity;

#[derive(Clone)]
struct Choice {
    key: SketchSessionKey,
    document: DocumentContext,
    capture: CaptureIdentity,
    targets: Vec<SketchTarget>,
    mode: SelectionMode,
    position: Pos2,
}

fn id() -> egui::Id {
    egui::Id::unique("sketch-target-choice")
}

pub(super) fn open(
    ctx: &egui::Context,
    session: &SketchSession,
    view: &ViewSnapshot,
    targets: Vec<SketchTarget>,
    mode: SelectionMode,
    position: Pos2,
) {
    ctx.data_mut(|data| {
        data.insert_temp(
            id(),
            Choice {
                key: session.key.clone(),
                document: session.document.clone(),
                capture: view.identity(),
                targets,
                mode,
                position,
            },
        )
    });
    egui::Popup::open_id(ctx, id());
}

fn valid(choice: &Choice, session: &SketchSession, view: Option<ViewSnapshot>) -> bool {
    session.phase == SketchPhase::Active
        && session.key == choice.key
        && session.document == choice.document
        && session.accepted_capture == Some(choice.capture)
        && view.is_some_and(|view| view.identity() == choice.capture)
        && choice
            .targets
            .iter()
            .all(|target| target.is_valid(&session.authoritative))
}

pub fn show_target_chooser(app: &mut App, ctx: &egui::Context) {
    let Some(choice) = ctx.data(|data| data.get_temp::<Choice>(id())) else {
        return;
    };
    let Some(session) = app
        .sketch
        .session
        .as_ref()
        .filter(|session| valid(&choice, session, app.view))
    else {
        close(ctx);
        return;
    };
    let selected = show_choices(ctx, &choice, session);
    if let Some(target) = selected {
        app.sketch_dispatch(SketchAction::Select {
            key: choice.key,
            document: choice.document,
            capture: choice.capture,
            targets: vec![target],
            mode: choice.mode,
        });
        close(ctx);
    } else if !egui::Popup::is_id_open(ctx, id()) {
        close(ctx);
    }
}

fn show_choices(
    ctx: &egui::Context,
    choice: &Choice,
    session: &SketchSession,
) -> Option<SketchTarget> {
    let mut selected = None;
    egui::Popup::new(
        id(),
        ctx.clone(),
        choice.position,
        egui::LayerId::background(),
    )
    .kind(egui::PopupKind::Menu)
    .open_memory(None)
    .frame(onecad_widgets::components::lists::menu_frame(ctx))
    .show(|ui| {
        ui.label("Choose sketch target");
        egui::ScrollArea::vertical()
            .max_height(260.0)
            .show(ui, |ui| {
                for target in &choice.targets {
                    if ui.button(label(*target, &session.authoritative)).clicked() {
                        selected = Some(*target);
                    }
                }
            });
    });
    selected
}

fn close(ctx: &egui::Context) {
    egui::Popup::close_id(ctx, id());
    ctx.data_mut(|data| data.remove::<Choice>(id()));
}

fn label(target: SketchTarget, sketch: &Sketch) -> String {
    format!("{} · {}", target_name(target, sketch), target.owner_id())
}

pub fn target_name(target: SketchTarget, sketch: &Sketch) -> String {
    let kind = match sketch.get_entity(target.owner_id()) {
        Some(SketchEntity::Point { .. }) => "Point",
        Some(SketchEntity::Line { .. }) => "Line",
        Some(SketchEntity::Circle { .. }) => "Circle",
        Some(SketchEntity::Arc { .. }) => "Arc",
        Some(SketchEntity::Ellipse { .. }) => "Ellipse",
        None => "Unavailable",
    };
    let role = match target {
        SketchTarget::Entity(_) => "",
        SketchTarget::Point { role, .. } => match role {
            onecad_core::sketch::CurvePosition::Start => " start",
            onecad_core::sketch::CurvePosition::End => " end",
            onecad_core::sketch::CurvePosition::Center => " center",
            onecad_core::sketch::CurvePosition::Arbitrary => "",
        },
    };
    format!("{kind}{role}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch_view::selection::tests::{add_point, session};

    #[test]
    fn chooser_scopes_candidates_to_view_document_and_visit() {
        let (mut session, _, view) = session();
        let point = add_point(&mut session, 0.0, 0.0);
        let ctx = egui::Context::default();
        open(
            &ctx,
            &session,
            &view,
            vec![SketchTarget::Point {
                entity: point,
                role: onecad_core::sketch::CurvePosition::Arbitrary,
            }],
            SelectionMode::Toggle,
            Pos2::ZERO,
        );
        let choice = ctx.data(|data| data.get_temp::<Choice>(id())).unwrap();
        assert!(egui::Popup::is_id_open(&ctx, id()));
        assert!(valid(&choice, &session, Some(view)));
        session.key.generation += 1;
        assert!(!valid(&choice, &session, Some(view)));
        session.key.generation -= 1;
        session.document.revision += 1;
        assert!(!valid(&choice, &session, Some(view)));
        close(&ctx);
        assert!(!egui::Popup::is_id_open(&ctx, id()));
        assert!(ctx.data(|data| data.get_temp::<Choice>(id())).is_none());
    }
    #[test]
    fn escape_closes_chooser_without_changing_selection() {
        let (mut session, _, view) = session();
        let point = add_point(&mut session, 0.0, 0.0);
        let target = SketchTarget::Point {
            entity: point,
            role: onecad_core::sketch::CurvePosition::Arbitrary,
        };
        session.selection = vec![target];
        let ctx = egui::Context::default();
        open(
            &ctx,
            &session,
            &view,
            vec![target],
            SelectionMode::Toggle,
            egui::pos2(100.0, 100.0),
        );
        let choice = ctx.data(|data| data.get_temp::<Choice>(id())).unwrap();
        for events in [
            vec![],
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        ] {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    assert!(show_choices(ui.ctx(), &choice, &session).is_none());
                },
            );
            output.textures_delta.clear();
        }
        assert!(!egui::Popup::is_id_open(&ctx, id()));
        assert_eq!(session.selection, vec![target]);
    }
}
