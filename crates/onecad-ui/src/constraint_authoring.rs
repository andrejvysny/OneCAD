//! Selection-derived constraints use authoritative sketch IDs and strict application admission.
use crate::{app::App, tool_registry::ToolId};
use onecad_app::dto::VariableDto;
use onecad_core::{
    document::variables::Scalar,
    expr::{Dimension, Env, Value},
    ids::{ConstraintId, EntityId},
    math::Vec2,
    sketch::{Constraint, CurvePosition, Sketch, SketchEntity},
};
use onecad_interaction::{
    DocumentContext,
    sketch::{SketchAction, SketchSession, SketchSessionKey, SketchTool},
};

#[derive(Clone, Debug, PartialEq)]
enum Target {
    Point {
        id: EntityId,
        at: Vec2,
    },
    Line {
        id: EntityId,
        start: EntityId,
        end: EntityId,
        length: f64,
    },
    Circle {
        id: EntityId,
        diameter: f64,
    },
}

fn point(sketch: &Sketch, id: EntityId) -> Option<Vec2> {
    match sketch.get_entity(id)? {
        SketchEntity::Point { at, .. } if at.x.is_finite() && at.y.is_finite() => Some(*at),
        _ => None,
    }
}

fn selected(session: &SketchSession) -> Option<Target> {
    if session.selection.len() != 1 || session.tool != SketchTool::Select || session.draft.is_some()
    {
        return None;
    }
    let id = *session.selection.first()?;
    let sketch = &session.authoritative;
    let entity = sketch.get_entity(id)?;
    if entity.is_reference_locked() {
        return None;
    }
    match entity {
        SketchEntity::Point { at, .. } => Some(Target::Point { id, at: *at }),
        SketchEntity::Line { start, end, .. } => {
            let a = point(sketch, *start)?;
            let b = point(sketch, *end)?;
            let length = (b.x - a.x).hypot(b.y - a.y);
            (length.is_finite() && length > 0.0).then_some(Target::Line {
                id,
                start: *start,
                end: *end,
                length,
            })
        }
        SketchEntity::Circle { radius, .. } if radius.is_finite() && *radius > 0.0 => {
            Some(Target::Circle {
                id,
                diameter: 2.0 * radius,
            })
        }
        _ => None,
    }
}

fn supports(target: &Target, tool: ToolId) -> bool {
    matches!(
        (target, tool),
        (Target::Point { .. }, ToolId::Fix)
            | (
                Target::Line { .. },
                ToolId::Horizontal | ToolId::Vertical | ToolId::Distance | ToolId::Dimension
            )
            | (Target::Circle { .. }, ToolId::Diameter | ToolId::Dimension)
    )
}

pub fn available(app: &App, tool: ToolId) -> bool {
    !app.busy()
        && crate::sketch_ui::tools_enabled(app)
        && app
            .sketch
            .session
            .as_ref()
            .and_then(selected)
            .is_some_and(|target| supports(&target, tool))
}

fn constraint(target: &Target, tool: ToolId, value: Option<Scalar>) -> Option<Constraint> {
    let id = ConstraintId::new();
    match (target, tool) {
        (Target::Point { id: point, at }, ToolId::Fix) => Some(Constraint::Fixed {
            id,
            point: *point,
            point_position: CurvePosition::Arbitrary,
            at: *at,
        }),
        (Target::Line { id: line, .. }, ToolId::Horizontal) => {
            Some(Constraint::Horizontal { id, line: *line })
        }
        (Target::Line { id: line, .. }, ToolId::Vertical) => {
            Some(Constraint::Vertical { id, line: *line })
        }
        (Target::Line { start, end, .. }, ToolId::Distance | ToolId::Dimension) => {
            Some(Constraint::Distance {
                id,
                entity1: *start,
                entity1_position: CurvePosition::Arbitrary,
                entity2: *end,
                entity2_position: CurvePosition::Arbitrary,
                value: value?,
            })
        }
        (Target::Circle { id: entity, .. }, ToolId::Diameter | ToolId::Dimension) => {
            Some(Constraint::Diameter {
                id,
                entity: *entity,
                value: value?,
            })
        }
        _ => None,
    }
}

#[derive(Clone)]
struct Draft {
    key: SketchSessionKey,
    document: DocumentContext,
    target: Target,
    text: String,
    focus: bool,
}

fn editor_id() -> egui::Id {
    egui::Id::unique("sketch-dimension-editor")
}

pub fn activate(app: &mut App, ctx: &egui::Context, tool: ToolId) {
    if !available(app, tool) {
        return;
    }
    let Some(session) = app.sketch.session.as_ref() else {
        return;
    };
    let Some(target) = selected(session) else {
        return;
    };
    if matches!(
        tool,
        ToolId::Distance | ToolId::Diameter | ToolId::Dimension
    ) {
        let value = match &target {
            Target::Line { length, .. } => *length,
            Target::Circle { diameter, .. } => *diameter,
            _ => return,
        };
        ctx.data_mut(|data| {
            data.insert_temp(
                editor_id(),
                Draft {
                    key: session.key.clone(),
                    document: session.document.clone(),
                    target,
                    text: value.to_string(),
                    focus: true,
                },
            )
        });
    } else if let Some(constraint) = constraint(&target, tool, None) {
        app.sketch_dispatch(SketchAction::AddConstraints {
            key: session.key.clone(),
            constraints: vec![constraint],
        });
    }
    app.ui.constraints_open = false;
}

fn scalar(text: &str, variables: &[VariableDto]) -> Result<Scalar, String> {
    let expression = text.strip_prefix('=').unwrap_or(text);
    let lookup = |name: &str| {
        variables
            .iter()
            .find(|variable| variable.name == name && variable.error.is_none())
            .and_then(|variable| {
                let dim = match variable.dimension.as_str() {
                    "length" => Dimension::Length,
                    "angle" => Dimension::Angle,
                    "scalar" => Dimension::Scalar,
                    _ => return None,
                };
                Some(Value {
                    number: variable.resolved_value,
                    dim,
                })
            })
    };
    let value = onecad_core::expr::evaluate_str(expression, &Env::new(&lookup), Dimension::Length)
        .map_err(|error| error.to_string())?;
    if !value.is_finite() || value <= 0.0 {
        return Err("Dimension must be finite and greater than zero".into());
    }
    Ok(if text.trim().parse::<f64>().is_ok() {
        Scalar::new(value)
    } else {
        Scalar::with_expr(value, expression)
    })
}

fn current(draft: &Draft, session: &SketchSession) -> bool {
    draft.key == session.key
        && draft.document == session.document
        && selected(session).as_ref() == Some(&draft.target)
}

fn editor(ctx: &egui::Context, draft: &mut Draft, variables: &[VariableDto]) -> (bool, bool) {
    let mut apply = false;
    let mut cancel = false;
    let modal = egui::Modal::new(editor_id()).show(ctx, |ui| {
        ui.heading(if matches!(draft.target, Target::Circle { .. }) {
            "Circle diameter"
        } else {
            "Line length"
        });
        let field = ui.add(egui::TextEdit::singleline(&mut draft.text).desired_width(260.0));
        if draft.focus {
            field.request_focus();
            draft.focus = false;
        }
        let parsed = scalar(&draft.text, variables);
        match &parsed {
            Ok(value) => {
                ui.label(format!("{} mm", value.value));
            }
            Err(message) => {
                ui.label(message);
            }
        }
        ui.horizontal(|ui| {
            apply = ui
                .add_enabled(parsed.is_ok(), egui::Button::new("Apply"))
                .clicked()
                || (parsed.is_ok() && ui.input(|input| input.key_pressed(egui::Key::Enter)));
            cancel = ui.button("Cancel").clicked();
        });
    });
    (apply, cancel || modal.should_close())
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut draft) = ctx.data_mut(|data| {
        let draft = data.get_temp::<Draft>(editor_id());
        data.remove::<Draft>(editor_id());
        draft
    }) else {
        return;
    };
    if app.busy()
        || !crate::sketch_ui::tools_enabled(app)
        || !app
            .sketch
            .session
            .as_ref()
            .is_some_and(|session| current(&draft, session))
    {
        return;
    }
    let variables = app
        .display
        .as_ref()
        .map_or(&[][..], |display| display.projection.variables.as_slice());
    let (apply, cancel) = editor(ctx, &mut draft, variables);
    if apply {
        if let Ok(value) = scalar(&draft.text, variables)
            && let Some(constraint) = constraint(&draft.target, ToolId::Dimension, Some(value))
        {
            app.sketch_dispatch(SketchAction::AddConstraints {
                key: draft.key,
                constraints: vec![constraint],
            });
        }
    } else if !cancel {
        ctx.data_mut(|data| data.insert_temp(editor_id(), draft));
    }
}

#[cfg(test)]
mod tests;
