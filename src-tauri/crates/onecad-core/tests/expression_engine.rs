//! Golden-fixture replay for the WP-E1 expression engine
//! (`docs/qa/expressions.fixture.json`), plus a no-panic proptest over
//! random well-formed scalar arithmetic.

use std::collections::HashMap;

use proptest::prelude::*;
use serde::Deserialize;

use onecad_core::expr::{evaluate_str, Dimension, Env, ExprErrorKind, Value};

#[derive(Debug, Deserialize)]
struct Fixture {
    version: u32,
    cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
struct Case {
    expr: String,
    site: String,
    #[serde(default)]
    vars: HashMap<String, VarSpec>,
    #[serde(default)]
    expect: Option<ExpectValue>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct VarSpec {
    value: f64,
    dim: String,
}

#[derive(Debug, Deserialize)]
struct ExpectValue {
    value: f64,
    dim: String,
}

fn dim_from_str(s: &str) -> Dimension {
    match s {
        "length" => Dimension::Length,
        "angle" => Dimension::Angle,
        "scalar" => Dimension::Scalar,
        other => panic!("fixture: unrecognized dimension `{other}`"),
    }
}

fn kind_from_str(s: &str) -> ExprErrorKind {
    match s {
        "Parse" => ExprErrorKind::Parse,
        "UnknownFunction" => ExprErrorKind::UnknownFunction,
        "Arity" => ExprErrorKind::Arity,
        "UndefinedVariable" => ExprErrorKind::UndefinedVariable,
        "DimensionMismatch" => ExprErrorKind::DimensionMismatch,
        "DivideByZero" => ExprErrorKind::DivideByZero,
        "NonFinite" => ExprErrorKind::NonFinite,
        "TooLong" => ExprErrorKind::TooLong,
        "TooDeep" => ExprErrorKind::TooDeep,
        other => panic!("fixture: unrecognized error kind `{other}`"),
    }
}

fn load_fixture() -> Fixture {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../docs/qa/expressions.fixture.json"
    );
    let text =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read fixture at {path}: {e}"));
    serde_json::from_str(&text).expect("parse expressions.fixture.json")
}

#[test]
fn fixture_cases_match_the_evaluator() {
    let fixture = load_fixture();
    assert_eq!(fixture.version, 1);
    assert!(
        fixture.cases.len() >= 40,
        "golden fixture must carry at least 40 cases, found {}",
        fixture.cases.len()
    );

    for (i, case) in fixture.cases.iter().enumerate() {
        let vars = &case.vars;
        let lookup = |name: &str| -> Option<Value> {
            vars.get(name).map(|v| Value {
                number: v.value,
                dim: dim_from_str(&v.dim),
            })
        };
        let env = Env::new(&lookup);
        let site = dim_from_str(&case.site);
        let result = evaluate_str(&case.expr, &env, site);

        match (&case.expect, &case.error) {
            (Some(expect), None) => {
                assert_eq!(
                    expect.dim, case.site,
                    "case {i} (`{}`): fixture's expect.dim must match site (self-consistency)",
                    case.expr
                );
                let got = result.unwrap_or_else(|e| {
                    panic!(
                        "case {i} (`{}`): expected value {}, got error {e:?}",
                        case.expr, expect.value
                    )
                });
                let tol = 1e-12 * expect.value.abs().max(1.0);
                let diff = (got - expect.value).abs();
                assert!(
                    diff <= tol,
                    "case {i} (`{}`): got {got}, expected {}, |diff|={diff} > tol={tol}",
                    case.expr,
                    expect.value
                );
            }
            (None, Some(error_kind)) => {
                let err = result.err().unwrap_or_else(|| {
                    panic!(
                        "case {i} (`{}`): expected error {error_kind}, got a value",
                        case.expr
                    )
                });
                assert_eq!(
                    err.kind,
                    kind_from_str(error_kind),
                    "case {i} (`{}`): expected error kind {error_kind}, got {:?} ({})",
                    case.expr,
                    err.kind,
                    err.message
                );
            }
            _ => panic!(
                "case {i} (`{}`): fixture must set exactly one of expect/error",
                case.expr
            ),
        }
    }
}

/// A small well-formed scalar-arithmetic expression: nested `+ - * /` (and a
/// bounded `^`) over integer literals, always fully parenthesized so the
/// generated string is syntactically valid by construction.
fn arb_scalar_expr() -> impl Strategy<Value = String> {
    let leaf = (-1000i32..1000).prop_map(|n| n.to_string());
    leaf.prop_recursive(4, 64, 4, |inner| {
        prop_oneof![
            (inner.clone(), inner.clone()).prop_map(|(a, b)| format!("({a}+{b})")),
            (inner.clone(), inner.clone()).prop_map(|(a, b)| format!("({a}-{b})")),
            (inner.clone(), inner.clone()).prop_map(|(a, b)| format!("({a}*{b})")),
            (inner.clone(), inner.clone()).prop_map(|(a, b)| format!("({a}/{b})")),
            (inner, 1u32..4).prop_map(|(a, e)| format!("({a}^{e})")),
        ]
    })
}

proptest! {
    /// Well-formed scalar arithmetic must never panic — a `Result::Err`
    /// (divide-by-zero, overflow-to-NonFinite, or a random tree that
    /// happens to trip TooLong/TooDeep) is a fine outcome; a panic is not.
    #[test]
    fn scalar_arithmetic_never_panics(src in arb_scalar_expr()) {
        let lookup = |_: &str| None;
        let env = Env::new(&lookup);
        let _ = evaluate_str(&src, &env, Dimension::Scalar);
    }
}
