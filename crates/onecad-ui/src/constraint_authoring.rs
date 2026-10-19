//! Typed native constraint intents; only application admission changes accepted sketch state.
mod dimensions;
mod geometric;
mod operands;
mod relation;
mod targets;
use crate::{app::App, tool_registry::ToolId};
use onecad_app::dto::VariableDto;
use onecad_core::{document::variables::Scalar, ids::ConstraintId, sketch::Constraint};
use onecad_interaction::{
    DocumentContext,
    sketch::{SketchAction, SketchSession, SketchSessionKey, SketchTarget, SketchTool},
};
pub use relation::Relation;
pub use targets::constraint_targets;

pub(crate) fn creation_scalar(text: &str, variables: &[VariableDto]) -> Result<Scalar, String> {
    let scalar = dimensions::scalar(text, Relation::Distance, variables)?;
    if scalar.value <= 0.0 {
        return Err("Use a known positive length".into());
    }
    Ok(scalar)
}

pub(crate) fn coordinate_scalar(text: &str, variables: &[VariableDto]) -> Result<Scalar, String> {
    dimensions::scalar(text, Relation::HorizontalDistance, variables)
}

pub(crate) fn angle_scalar(text: &str, variables: &[VariableDto]) -> Result<Scalar, String> {
    dimensions::scalar(text, Relation::Angle, variables)
}

#[derive(Clone)]
struct Draft {
    key: SketchSessionKey,
    document: DocumentContext,
    targets: Vec<SketchTarget>,
    constraint: Constraint,
    editing: bool,
    text: String,
    focus: bool,
}

fn editor_id() -> egui::Id {
    egui::Id::unique("sketch-dimension-editor")
}

fn ready(app: &App) -> Option<&SketchSession> {
    app.sketch.session.as_ref().filter(|session| {
        !app.busy()
            && crate::sketch_ui::tools_enabled(app)
            && session.tool == SketchTool::Select
            && session.draft.is_none()
    })
}

fn tool_relation(session: &SketchSession, tool: ToolId) -> Option<Relation> {
    if tool != ToolId::Dimension {
        return Relation::for_tool(tool);
    }
    if session.selection.len() == 1
        && operands::circular(&session.authoritative, session.selection[0]).is_some()
    {
        return Relation::for_tool(ToolId::Diameter);
    }
    if operands::build(
        Relation::Distance,
        &session.authoritative,
        &session.selection,
        None,
    )
    .is_ok()
    {
        Some(Relation::Distance)
    } else {
        Some(Relation::Angle)
    }
}

pub fn available(app: &App, tool: ToolId) -> bool {
    ready(app)
        .and_then(|session| tool_relation(session, tool))
        .is_some_and(|relation| applicability(app, relation).is_ok())
}

pub fn applicability(app: &App, relation: Relation) -> Result<(), String> {
    let session = ready(app).ok_or("Finish the current command and select sketch operands")?;
    let candidate = operands::build(relation, &session.authoritative, &session.selection, None)?;
    if operands::already_applied(&session.authoritative, &candidate) {
        return Err(format!(
            "{} is already applied to these operands",
            relation.label()
        ));
    }
    Ok(())
}

pub fn activate(app: &mut App, ctx: &egui::Context, tool: ToolId) {
    let Some(relation) = ready(app).and_then(|session| tool_relation(session, tool)) else {
        return;
    };
    activate_relation(app, ctx, relation);
}

pub fn activate_relation(app: &mut App, ctx: &egui::Context, relation: Relation) {
    if let Err(error) = applicability(app, relation) {
        app.status = error;
        app.diagnostics.record(
            crate::diagnostics::Source::Sketch,
            crate::diagnostics::State::Rejected,
            &app.status,
        );
        return;
    }
    let Some(session) = ready(app) else { return };
    let Some(value) = Scalar::try_new(dimensions::initial(
        relation,
        &session.authoritative,
        &session.selection,
    )) else {
        app.status = "Dimension measurement is not finite".into();
        return;
    };
    let Ok(constraint) = operands::create(
        relation,
        &session.authoritative,
        &session.selection,
        Some(value),
    ) else {
        return;
    };
    if relation.dimensional() {
        open_editor(ctx, session, constraint, false);
    } else {
        app.sketch_dispatch(SketchAction::AddConstraints {
            key: session.key.clone(),
            constraints: vec![constraint],
        });
    }
    app.ui.constraints_open = false;
}

fn open_editor(
    ctx: &egui::Context,
    session: &SketchSession,
    constraint: Constraint,
    editing: bool,
) {
    let Some(text) = dimensions::text(&constraint) else {
        return;
    };
    ctx.data_mut(|data| {
        data.insert_temp(
            editor_id(),
            Draft {
                key: session.key.clone(),
                document: session.document.clone(),
                targets: session.selection.clone(),
                constraint,
                editing,
                text,
                focus: true,
            },
        )
    });
}

pub fn edit_dimension(app: &App, ctx: &egui::Context, id: ConstraintId) {
    let Some(session) = ready(app) else { return };
    let Some(constraint) = session
        .authoritative
        .get_constraint(id)
        .filter(|constraint| constraint.value().is_some())
    else {
        return;
    };
    open_editor(ctx, session, constraint.clone(), true);
}

fn current(draft: &Draft, session: &SketchSession) -> bool {
    draft.key == session.key
        && draft.document == session.document
        && if draft.editing {
            session.authoritative.get_constraint(draft.constraint.id()) == Some(&draft.constraint)
        } else {
            session.selection == draft.targets
        }
}

fn editor(ctx: &egui::Context, draft: &mut Draft, variables: &[VariableDto]) -> (bool, bool) {
    let Some(relation) = Relation::from_constraint(&draft.constraint) else {
        return (false, true);
    };
    let mut apply = false;
    let mut cancel = false;
    let modal = egui::Modal::new(editor_id()).show(ctx, |ui| {
        let action = if draft.editing { "Edit" } else { "Add" };
        ui.heading(format!("{action} {}", relation.label()));
        ui.label(if relation == Relation::Angle {
            "Degrees; expressions may use deg or rad"
        } else {
            "Millimetres; expressions may reference document variables"
        });
        let field = ui.add(egui::TextEdit::singleline(&mut draft.text)
            .desired_width(260.0)
            .char_limit(onecad_core::expr::MAX_SOURCE_LEN));
        if draft.focus {
            field.request_focus();
            draft.focus = false;
        }
        let parsed = dimensions::scalar(&draft.text, relation, variables);
        if let Err(error) = &parsed {
            ui.colored_label(ui.visuals().error_fg_color, error);
        } else {
            ui.small("Expressions use current document variables. A literal removes the expression binding.");
        }
        ui.horizontal(|ui| {
            apply = ui.add_enabled(parsed.is_ok(), egui::Button::new("Apply")).clicked()
                || (parsed.is_ok() && ui.input(|input| input.key_pressed(egui::Key::Enter)));
            cancel = ui.button("Cancel").clicked();
        });
    });
    (apply, cancel || modal.should_close())
}

pub(crate) fn editor_open(ctx: &egui::Context) -> bool {
    ctx.data(|data| data.get_temp::<Draft>(editor_id()).is_some())
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut draft) = ctx.data_mut(|data| {
        let draft = data.get_temp::<Draft>(editor_id());
        data.remove::<Draft>(editor_id());
        draft
    }) else {
        return;
    };
    if ready(app).is_none_or(|session| !current(&draft, session)) {
        return;
    }
    let variables = app
        .display
        .as_ref()
        .map_or(&[][..], |display| display.projection.variables.as_slice());
    let (apply, cancel) = editor(ctx, &mut draft, variables);
    if apply {
        let Some(relation) = Relation::from_constraint(&draft.constraint) else {
            return;
        };
        if let Ok(value) = dimensions::scalar(&draft.text, relation, variables) {
            if draft.editing {
                app.sketch_dispatch(SketchAction::SetDimension {
                    key: draft.key,
                    constraint: draft.constraint.id(),
                    value,
                });
            } else {
                dimensions::set_value(&mut draft.constraint, value);
                app.sketch_dispatch(SketchAction::AddConstraints {
                    key: draft.key,
                    constraints: vec![draft.constraint],
                });
            }
        }
    } else if !cancel {
        ctx.data_mut(|data| data.insert_temp(editor_id(), draft));
    }
}

pub fn dimension_list(app: &mut App, ui: &mut egui::Ui) {
    if !app.sketch_active() {
        return;
    }
    let Some(session) = app.sketch.session.as_ref() else {
        return;
    };
    let constraints: Vec<_> = session
        .authoritative
        .constraints()
        .iter()
        .map(|constraint| {
            (
                constraint.id(),
                constraint_label(constraint),
                dimensions::text(constraint),
            )
        })
        .collect();
    if constraints.is_empty() {
        return;
    }
    ui.separator();
    ui.strong("Sketch constraints");
    if ui.small_button("Show dimension labels").clicked() {
        crate::sketch_dimensions::show_all(app, ui.ctx());
    }
    let mut edit = None;
    let mut remove = None;
    crate::sketch_repair::set_constraint_hover(app, ui.ctx(), None);
    egui::ScrollArea::vertical()
        .id_salt("sketch-constraints")
        .max_height(220.0)
        .show(ui, |ui| {
            for (id, label, text) in &constraints {
                let row = ui.horizontal_wrapped(|ui| {
                    ui.label(format!(
                        "{}{}",
                        label,
                        text.as_ref()
                            .map_or(String::new(), |text| format!(": {text}"))
                    ))
                    .on_hover_text(id.to_string());
                    if text.is_some()
                        && ui
                            .add_enabled(ready(app).is_some(), egui::Button::new("Edit"))
                            .clicked()
                    {
                        edit = Some(*id);
                    }
                    if ui
                        .add_enabled(
                            crate::sketch_repair::ready(app),
                            egui::Button::new("Remove"),
                        )
                        .clicked()
                    {
                        remove = Some(*id);
                    }
                });
                if row.response.hovered() {
                    crate::sketch_repair::set_constraint_hover(app, ui.ctx(), Some(*id));
                }
            }
        });
    if let Some(id) = edit {
        edit_dimension(app, ui.ctx(), id);
    }
    if let Some(id) = remove {
        crate::sketch_repair::remove_constraint(app, id);
    }
}

fn constraint_label(constraint: &Constraint) -> &'static str {
    match constraint {
        Constraint::Coincident { .. } => "Coincident",
        Constraint::Horizontal { .. } => "Horizontal",
        Constraint::Vertical { .. } => "Vertical",
        Constraint::Fixed { .. } => "Fixed",
        Constraint::Midpoint { .. } => "Midpoint",
        Constraint::OnCurve { .. } => "Point on curve",
        Constraint::Parallel { .. } => "Parallel",
        Constraint::Perpendicular { .. } => "Perpendicular",
        Constraint::Tangent { .. } => "Tangent",
        Constraint::TangentAtPoint { .. } => "Tangent at point",
        Constraint::Concentric { .. } => "Concentric",
        Constraint::Equal { .. } => "Equal",
        Constraint::Distance { .. } => "Distance (mm)",
        Constraint::HorizontalDistance { .. } => "Horizontal distance (mm)",
        Constraint::VerticalDistance { .. } => "Vertical distance (mm)",
        Constraint::Angle { .. } => "Angle (°)",
        Constraint::Radius { .. } => "Radius (mm)",
        Constraint::Diameter { .. } => "Diameter (mm)",
        Constraint::Symmetric { .. } => "Symmetric",
        Constraint::HorizontalPoints { .. } => "Horizontal points",
        Constraint::VerticalPoints { .. } => "Vertical points",
    }
}

#[cfg(test)]
mod tests;
