//! Evaluator — walks an [`Expr`] tree against an [`Env`] and a call-site
//! [`Dimension`], applying the dimension algebra in `dimension.rs`.
//!
//! Every recursive step returns through [`finite`], so a `NaN`/`±Infinity`
//! is caught at the exact node that produced it (tight error span) rather
//! than discovered later after it has propagated through more arithmetic.

use super::dimension::{self, Dimension, Value};
use super::parser::{parse, BinaryOp, Expr, UnaryOp};
use super::{ExprError, ExprErrorKind};

/// Variable resolution is the CALLER's job (WP-E2 adds the document
/// variable-table pass); this engine only evaluates against whatever
/// `lookup` returns.
pub struct Env<'a> {
    lookup: &'a dyn Fn(&str) -> Option<Value>,
}

impl<'a> Env<'a> {
    #[must_use]
    pub fn new(lookup: &'a dyn Fn(&str) -> Option<Value>) -> Self {
        Self { lookup }
    }

    fn resolve(&self, name: &str) -> Option<Value> {
        (self.lookup)(name)
    }
}

/// Evaluates `expr` against `env`, coercing the result into `site`'s
/// canonical unit. A `Scalar` result coerces into any site verbatim (no
/// rescale — the number is read as already being in the site's canonical
/// unit); a mismatched non-`Scalar` dimension is a loud
/// [`ExprErrorKind::DimensionMismatch`] naming both dimensions.
///
/// # Errors
/// [`ExprError`] on an unresolved identifier, an unknown function, a wrong
/// arity, a dimension mismatch (including at the site boundary), a
/// division by zero, or a non-finite result.
pub fn evaluate(expr: &Expr, env: &Env, site: Dimension) -> Result<f64, ExprError> {
    check_site(expr, evaluate_value(expr, env)?, site)
}

/// The site boundary on its own: coerces an already-evaluated [`Value`] into
/// `site`'s canonical unit, or refuses with a mismatch naming both dimensions.
///
/// [`evaluate`] is [`evaluate_value`] followed by this. Split out for the caller
/// that needs BOTH the dimensioned value and the coerced number
/// ([`crate::regen::variables`], which reports a variable's inferred dimension
/// while writing the parameter's number) — going through `evaluate` there would
/// walk the tree a second time, once per bound scalar per regen. `expr` is
/// carried only for its span, so the refusal points at the right source text.
///
/// # Errors
/// [`ExprErrorKind::DimensionMismatch`] when `value` is neither `site`'s
/// dimension nor [`Dimension::Scalar`].
pub fn check_site(expr: &Expr, value: Value, site: Dimension) -> Result<f64, ExprError> {
    if value.dim == site || value.dim == Dimension::Scalar {
        Ok(value.number)
    } else {
        Err(ExprError::new(
            ExprErrorKind::DimensionMismatch,
            expr.span(),
            format!(
                "expected {} (or a dimensionless number), found {}",
                dimension::label(site),
                dimension::label(value.dim)
            ),
        ))
    }
}

/// Evaluates `expr` against `env` WITHOUT a call-site coercion, returning the
/// dimensioned [`Value`] the algebra produced.
///
/// [`evaluate`] is this plus the site check; this exists for the one caller that
/// has no site to check against — the document variable-table pass
/// ([`crate::regen::variables`]), which INFERS a variable's dimension from its
/// own expression (`45deg` ⇒ angle, `w*2` ⇒ whatever `w` is) and would otherwise
/// have to probe all three sites to recover a number the evaluator already
/// computed.
///
/// # Errors
/// See [`evaluate`], minus the site-boundary mismatch this one cannot raise.
pub fn evaluate_value(expr: &Expr, env: &Env) -> Result<Value, ExprError> {
    eval_node(expr, env)
}

/// Parses then evaluates `src` in one call.
///
/// # Errors
/// See [`evaluate`]; parse failures ([`ExprErrorKind::Parse`],
/// [`ExprErrorKind::TooLong`], [`ExprErrorKind::TooDeep`]) surface the same
/// way.
pub fn evaluate_str(src: &str, env: &Env, site: Dimension) -> Result<f64, ExprError> {
    let expr = parse(src)?;
    evaluate(&expr, env, site)
}

fn finite(number: f64, dim: Dimension, span: (usize, usize)) -> Result<Value, ExprError> {
    if number.is_finite() {
        Ok(Value { number, dim })
    } else {
        Err(ExprError::new(
            ExprErrorKind::NonFinite,
            span,
            "result is not finite",
        ))
    }
}

fn eval_node(expr: &Expr, env: &Env) -> Result<Value, ExprError> {
    match expr {
        Expr::Number { value, span } => finite(value.number, value.dim, *span),
        Expr::Ident { name, span } => {
            let v = env.resolve(name).ok_or_else(|| {
                ExprError::new(
                    ExprErrorKind::UndefinedVariable,
                    *span,
                    format!("undefined variable `{name}`"),
                )
            })?;
            finite(v.number, v.dim, *span)
        }
        Expr::Unary { op, expr, span } => {
            let v = eval_node(expr, env)?;
            let number = match op {
                UnaryOp::Plus => v.number,
                UnaryOp::Neg => -v.number,
            };
            finite(number, v.dim, *span)
        }
        Expr::Binary { op, lhs, rhs, span } => eval_binary(*op, lhs, rhs, env, *span),
        Expr::Call { name, args, span } => eval_call(name, args, env, *span),
    }
}

fn eval_binary(
    op: BinaryOp,
    lhs: &Expr,
    rhs: &Expr,
    env: &Env,
    span: (usize, usize),
) -> Result<Value, ExprError> {
    let a = eval_node(lhs, env)?;
    let b = eval_node(rhs, env)?;
    match op {
        BinaryOp::Add => finite(
            a.number + b.number,
            dimension::add_like(a.dim, b.dim, span, "+")?,
            span,
        ),
        BinaryOp::Sub => finite(
            a.number - b.number,
            dimension::add_like(a.dim, b.dim, span, "-")?,
            span,
        ),
        BinaryOp::Mul => finite(
            a.number * b.number,
            dimension::mul(a.dim, b.dim, span)?,
            span,
        ),
        BinaryOp::Div => {
            if b.number == 0.0 {
                return Err(ExprError::new(
                    ExprErrorKind::DivideByZero,
                    span,
                    "division by zero",
                ));
            }
            finite(
                a.number / b.number,
                dimension::div(a.dim, b.dim, span)?,
                span,
            )
        }
        BinaryOp::Pow => finite(
            a.number.powf(b.number),
            dimension::pow(a.dim, b.dim, span)?,
            span,
        ),
    }
}

/// The fixed builtin function set (SCHEMA-adjacent: `KnownOperation` stays
/// closed; likewise no addon-defined expression function).
fn eval_call(
    name: &str,
    args: &[Expr],
    env: &Env,
    span: (usize, usize),
) -> Result<Value, ExprError> {
    let vals = args
        .iter()
        .map(|a| eval_node(a, env))
        .collect::<Result<Vec<_>, _>>()?;
    match name {
        "sin" | "cos" | "tan" => eval_trig(name, &vals, span),
        "asin" | "acos" | "atan" => eval_inverse_trig(name, &vals, span),
        "sqrt" => eval_sqrt(&vals, span),
        "abs" | "floor" | "ceil" | "round" => eval_dim_preserving(name, &vals, span),
        "min" | "max" => eval_minmax(name, &vals, span),
        _ => Err(ExprError::new(
            ExprErrorKind::UnknownFunction,
            span,
            format!("unknown function `{name}`"),
        )),
    }
}

fn check_arity(
    vals: &[Value],
    want: usize,
    name: &str,
    span: (usize, usize),
) -> Result<(), ExprError> {
    if vals.len() == want {
        Ok(())
    } else {
        Err(ExprError::new(
            ExprErrorKind::Arity,
            span,
            format!("`{name}` takes {want} argument(s), got {}", vals.len()),
        ))
    }
}

/// `sin|cos|tan`: argument must be `Angle`, result is `Scalar`.
fn eval_trig(name: &str, vals: &[Value], span: (usize, usize)) -> Result<Value, ExprError> {
    check_arity(vals, 1, name, span)?;
    dimension::require_angle(vals[0].dim, span)?;
    let rad = vals[0].number.to_radians();
    let n = match name {
        "sin" => rad.sin(),
        "cos" => rad.cos(),
        _ => rad.tan(),
    };
    finite(n, Dimension::Scalar, span)
}

/// `asin|acos|atan`: argument must be `Scalar`, result is `Angle` (degrees).
fn eval_inverse_trig(name: &str, vals: &[Value], span: (usize, usize)) -> Result<Value, ExprError> {
    check_arity(vals, 1, name, span)?;
    dimension::require_scalar(vals[0].dim, span, name)?;
    let rad = match name {
        "asin" => vals[0].number.asin(),
        "acos" => vals[0].number.acos(),
        _ => vals[0].number.atan(),
    };
    finite(rad.to_degrees(), Dimension::Angle, span)
}

/// `sqrt`: argument must be `Scalar`, result is `Scalar`.
fn eval_sqrt(vals: &[Value], span: (usize, usize)) -> Result<Value, ExprError> {
    check_arity(vals, 1, "sqrt", span)?;
    dimension::require_scalar(vals[0].dim, span, "sqrt")?;
    finite(vals[0].number.sqrt(), Dimension::Scalar, span)
}

/// `abs|floor|ceil|round`: any dimension in, same dimension out — these
/// operate on the canonical number only.
fn eval_dim_preserving(
    name: &str,
    vals: &[Value],
    span: (usize, usize),
) -> Result<Value, ExprError> {
    check_arity(vals, 1, name, span)?;
    let n = match name {
        "abs" => vals[0].number.abs(),
        "floor" => vals[0].number.floor(),
        "ceil" => vals[0].number.ceil(),
        _ => vals[0].number.round(),
    };
    finite(n, vals[0].dim, span)
}

/// `min|max`: same dimension algebra as `+`/`-` (Scalar coerces).
fn eval_minmax(name: &str, vals: &[Value], span: (usize, usize)) -> Result<Value, ExprError> {
    check_arity(vals, 2, name, span)?;
    let dim = dimension::add_like(vals[0].dim, vals[1].dim, span, name)?;
    let n = if name == "min" {
        vals[0].number.min(vals[1].number)
    } else {
        vals[0].number.max(vals[1].number)
    };
    finite(n, dim, span)
}
