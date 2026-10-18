//! Canonical units and the dimension algebra.
//!
//! Every [`Value`] carries its number in the CANONICAL unit for its
//! dimension: millimetres for [`Dimension::Length`], degrees for
//! [`Dimension::Angle`], unitless for [`Dimension::Scalar`]. A literal unit
//! suffix (`mm cm m in deg rad`) converts to canonical the moment the
//! parser folds the literal (`literal_value`, context-free — units need no
//! environment); every operator below only ever sees canonical numbers, so
//! this algebra applies no conversion factor of its own — only dimension
//! bookkeeping. That is also why Scalar-into-site coercion (`eval.rs`) is a
//! pass-through: the number is already in whatever unit the call site means.

use super::{ExprError, ExprErrorKind};

/// The dimension a numeric [`Value`] is measured in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dimension {
    Length,
    Angle,
    Scalar,
}

/// Human-facing label for error messages.
pub(crate) fn label(d: Dimension) -> &'static str {
    match d {
        Dimension::Length => "length",
        Dimension::Angle => "angle",
        Dimension::Scalar => "scalar",
    }
}

/// An evaluated number tagged with its dimension, always in canonical units
/// (mm / deg / unitless).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Value {
    pub number: f64,
    pub dim: Dimension,
}

/// A recognized literal unit suffix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Unit {
    Mm,
    Cm,
    M,
    In,
    Deg,
    Rad,
}

impl Unit {
    /// Matches an adjacent alpha run against a WHOLE unit keyword — no
    /// prefix matching, so `1radius` is a lex error rather than a silent
    /// `1rad` + stray `ius` identifier.
    pub(crate) fn from_suffix(s: &str) -> Option<Self> {
        match s {
            "mm" => Some(Unit::Mm),
            "cm" => Some(Unit::Cm),
            "m" => Some(Unit::M),
            "in" => Some(Unit::In),
            "deg" => Some(Unit::Deg),
            "rad" => Some(Unit::Rad),
            _ => None,
        }
    }

    /// `(canonical-unit factor, dimension)`.
    fn canonical(self) -> (f64, Dimension) {
        match self {
            Unit::Mm => (1.0, Dimension::Length),
            Unit::Cm => (10.0, Dimension::Length),
            Unit::M => (1000.0, Dimension::Length),
            Unit::In => (25.4, Dimension::Length),
            Unit::Deg => (1.0, Dimension::Angle),
            Unit::Rad => (180.0 / std::f64::consts::PI, Dimension::Angle),
        }
    }
}

/// Folds a literal's raw text value + optional unit suffix into its
/// canonical [`Value`] — context-free, so this runs at parse time.
pub(crate) fn literal_value(value: f64, unit: Option<Unit>) -> Value {
    match unit {
        None => Value {
            number: value,
            dim: Dimension::Scalar,
        },
        Some(u) => {
            let (factor, dim) = u.canonical();
            Value {
                number: value * factor,
                dim,
            }
        }
    }
}

fn mismatch(a: Dimension, b: Dimension, span: (usize, usize), op: &str) -> ExprError {
    ExprError::new(
        ExprErrorKind::DimensionMismatch,
        span,
        format!("`{op}` cannot combine {} and {}", label(a), label(b)),
    )
}

/// `+`, `-`, `min`, `max`: equal dims OK; either side `Scalar` coerces to
/// the other; otherwise a loud mismatch. Result is the non-`Scalar` dim (or
/// `Scalar` if both are).
pub(crate) fn add_like(
    a: Dimension,
    b: Dimension,
    span: (usize, usize),
    op: &str,
) -> Result<Dimension, ExprError> {
    match (a, b) {
        (x, y) if x == y => Ok(x),
        (Dimension::Scalar, y) => Ok(y),
        (x, Dimension::Scalar) => Ok(x),
        (x, y) => Err(mismatch(x, y, span, op)),
    }
}

/// `*`: at most one operand may be non-`Scalar` — no `mm^2`, no `deg*mm`
/// (and no `mm*mm` either: two equal non-Scalar dims still mismatch).
pub(crate) fn mul(
    a: Dimension,
    b: Dimension,
    span: (usize, usize),
) -> Result<Dimension, ExprError> {
    match (a, b) {
        (Dimension::Scalar, Dimension::Scalar) => Ok(Dimension::Scalar),
        (Dimension::Scalar, y) => Ok(y),
        (x, Dimension::Scalar) => Ok(x),
        (x, y) => Err(mismatch(x, y, span, "*")),
    }
}

/// `/`: dividing by a `Scalar` keeps the numerator's dim; dividing two
/// equal dims cancels to `Scalar`; anything else (including a `Scalar`
/// numerator over a dimensioned denominator) is a mismatch.
pub(crate) fn div(
    a: Dimension,
    b: Dimension,
    span: (usize, usize),
) -> Result<Dimension, ExprError> {
    if b == Dimension::Scalar {
        Ok(a)
    } else if a == b {
        Ok(Dimension::Scalar)
    } else {
        Err(mismatch(a, b, span, "/"))
    }
}

/// `^`: both operands must be `Scalar`.
pub(crate) fn pow(
    a: Dimension,
    b: Dimension,
    span: (usize, usize),
) -> Result<Dimension, ExprError> {
    if a == Dimension::Scalar && b == Dimension::Scalar {
        Ok(Dimension::Scalar)
    } else {
        Err(mismatch(a, b, span, "^"))
    }
}

/// `sin|cos|tan` argument check: must be `Angle`. A bare `Scalar` gets a
/// specific nudge toward the fix; any other mismatch is generic.
pub(crate) fn require_angle(d: Dimension, span: (usize, usize)) -> Result<(), ExprError> {
    if d == Dimension::Angle {
        return Ok(());
    }
    if d == Dimension::Scalar {
        return Err(ExprError::new(
            ExprErrorKind::DimensionMismatch,
            span,
            "trig needs an angle — write 30deg or 0.5rad",
        ));
    }
    Err(mismatch(d, Dimension::Angle, span, "trig"))
}

/// `asin|acos|atan|sqrt` argument check: must be `Scalar`.
pub(crate) fn require_scalar(
    d: Dimension,
    span: (usize, usize),
    fn_name: &str,
) -> Result<(), ExprError> {
    if d == Dimension::Scalar {
        Ok(())
    } else {
        Err(mismatch(d, Dimension::Scalar, span, fn_name))
    }
}
