//! UI values use degrees; the sketch wire stores radians. Expressions remain application-resolved.
use super::{operands, relation::Relation};
use onecad_app::dto::VariableDto;
use onecad_core::{
    document::variables::Scalar,
    expr::{Dimension, Env, ExprErrorKind, Value},
    sketch::{Constraint, Sketch},
};
use onecad_interaction::sketch::SketchTarget;

pub fn scalar(text: &str, relation: Relation, variables: &[VariableDto]) -> Result<Scalar, String> {
    let text = text.trim();
    if text.parse::<f64>().is_ok_and(|value| !value.is_finite()) {
        return Err("Dimension must be finite".into());
    }
    let expression = text.strip_prefix('=').unwrap_or(text).trim();
    let parsed = onecad_core::expr::parse(expression).map_err(|error| error.to_string())?;
    let site = if relation == Relation::Angle {
        Dimension::Angle
    } else {
        Dimension::Length
    };
    let lookup = |name: &str| {
        variables
            .iter()
            .find(|variable| variable.name == name && variable.error.is_none())
            .and_then(|variable| {
                Some(Value {
                    number: variable.resolved_value,
                    dim: match variable.dimension.as_str() {
                        "length" => Dimension::Length,
                        "angle" => Dimension::Angle,
                        "scalar" => Dimension::Scalar,
                        _ => return None,
                    },
                })
            })
    };
    let evaluated = onecad_core::expr::evaluate(&parsed, &Env::new(&lookup), site);
    let mut value = match evaluated {
        Ok(value) => value,
        Err(error) if error.kind == ExprErrorKind::UndefinedVariable => {
            return Ok(Scalar::with_expr(0.0, expression));
        }
        Err(error) => return Err(error.to_string()),
    };
    if !value.is_finite() || (relation.positive() && value <= 0.0) {
        return Err(if relation.positive() {
            "Dimension must be finite and greater than zero"
        } else {
            "Dimension must be finite"
        }
        .into());
    }
    if relation == Relation::Angle {
        value = value.to_radians();
    }
    Ok(if text.parse::<f64>().is_ok() {
        Scalar::new(value)
    } else {
        Scalar::with_expr(value, expression)
    })
}

pub fn text(constraint: &Constraint) -> Option<String> {
    let scalar = constraint.value()?;
    Some(scalar.expr.clone().unwrap_or_else(|| {
        if matches!(constraint, Constraint::Angle { .. }) {
            scalar.value.to_degrees().to_string()
        } else {
            scalar.value.to_string()
        }
    }))
}

pub fn set_value(constraint: &mut Constraint, scalar: Scalar) {
    match constraint {
        Constraint::Distance { value, .. }
        | Constraint::HorizontalDistance { value, .. }
        | Constraint::VerticalDistance { value, .. }
        | Constraint::Angle { value, .. }
        | Constraint::Radius { value, .. }
        | Constraint::Diameter { value, .. } => *value = scalar,
        _ => {}
    }
}

pub fn initial(relation: Relation, sketch: &Sketch, targets: &[SketchTarget]) -> f64 {
    match (relation, targets) {
        (Relation::Radius | Relation::Diameter, [target]) => operands::circular(sketch, *target)
            .map_or(1.0, |(_, radius)| {
                if relation == Relation::Diameter {
                    2.0 * radius
                } else {
                    radius
                }
            }),
        (Relation::Distance, [target]) => {
            operands::line(sketch, *target).map_or(1.0, |(_, a, b)| (b.x - a.x).hypot(b.y - a.y))
        }
        (
            Relation::HorizontalDistance | Relation::VerticalDistance | Relation::Distance,
            [a, b],
        ) if a.point_coordinates(sketch).is_some() && b.point_coordinates(sketch).is_some() => {
            let (Some(a), Some(b)) = (a.point_coordinates(sketch), b.point_coordinates(sketch))
            else {
                return 1.0;
            };
            match relation {
                Relation::HorizontalDistance => b.x - a.x,
                Relation::VerticalDistance => b.y - a.y,
                _ => (b.x - a.x).hypot(b.y - a.y),
            }
        }
        (Relation::Angle, [a, b]) => {
            let (Some((_, a, b)), Some((_, c, d))) =
                (operands::line(sketch, *a), operands::line(sketch, *b))
            else {
                return 0.0;
            };
            ((b.x - a.x) * (d.y - c.y) - (b.y - a.y) * (d.x - c.x))
                .atan2((b.x - a.x) * (d.x - c.x) + (b.y - a.y) * (d.y - c.y))
        }
        (Relation::Distance, [a, b]) => line_distance(sketch, *a, *b).unwrap_or(1.0),
        _ => 1.0,
    }
}

fn line_distance(sketch: &Sketch, a: SketchTarget, b: SketchTarget) -> Option<f64> {
    let (point, (_, start, end)) = if let Some(point) = a.point_coordinates(sketch) {
        (point, operands::line(sketch, b)?)
    } else if let Some(point) = b.point_coordinates(sketch) {
        (point, operands::line(sketch, a)?)
    } else {
        let (_, point, _) = operands::line(sketch, a)?;
        (point, operands::line(sketch, b)?)
    };
    let (dx, dy) = (end.x - start.x, end.y - start.y);
    Some(((point.x - start.x) * dy - (point.y - start.y) * dx).abs() / dx.hypot(dy))
}
