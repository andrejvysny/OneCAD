//! Pure, dependency-free expression engine (WP-E1).
//!
//! Lexes, parses, and evaluates arithmetic/unit expressions with a small
//! dimension algebra (length / angle / scalar). Variable resolution is the
//! CALLER's job via [`Env`] — nothing here reads the document
//! `VariableTable`. That caller is [`crate::regen::variables`], which resolves
//! the table (chained references, cycles) and evaluates every op parameter's
//! expression at the [`Dimension`] its registry entry declares.
//!
//! Canonical units: millimetres for [`Dimension::Length`], degrees for
//! [`Dimension::Angle`] (the op wire's angle convention is degrees; see
//! SCHEMA), unitless for [`Dimension::Scalar`]. `docs/qa/expressions.fixture.json`
//! is the golden case set this crate's tests replay, and the one a later TS
//! parity test replays independently — see `dimension.rs` for the algebra.

mod dimension;
mod eval;
mod lexer;
mod parser;

pub use dimension::{Dimension, Value};
pub use eval::{check_site, evaluate, evaluate_str, evaluate_value, Env};
pub use parser::{parse, references, BinaryOp, Expr, UnaryOp};

/// Hard cap on source length in bytes, checked before lexing.
pub const MAX_SOURCE_LEN: usize = 512;

/// Hard cap on parenthesized/call-argument nesting depth, checked while
/// parsing.
pub const MAX_DEPTH: usize = 32;

/// A byte-span-carrying evaluation failure. `span` indexes into the source
/// string passed to [`parse`] / [`evaluate_str`] (start inclusive, end
/// exclusive).
#[derive(Debug, Clone, PartialEq)]
pub struct ExprError {
    pub kind: ExprErrorKind,
    pub span: (usize, usize),
    pub message: String,
}

impl ExprError {
    pub(crate) fn new(
        kind: ExprErrorKind,
        span: (usize, usize),
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            span,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ExprError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:?} at {}..{}: {}",
            self.kind, self.span.0, self.span.1, self.message
        )
    }
}

impl std::error::Error for ExprError {}

/// What went wrong. A distinct kind per failure mode (rather than one
/// opaque error) is what lets callers and tests assert on the FAILURE MODE,
/// not on message text — required by the golden fixture's `"error": "..."`
/// cases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExprErrorKind {
    /// Malformed source: unexpected character, unrecognized unit suffix,
    /// unclosed paren, trailing input, etc.
    Parse,
    /// A call named a function outside the fixed builtin set.
    UnknownFunction,
    /// A call to a known function with the wrong number of arguments.
    Arity,
    /// An identifier the [`Env`] could not resolve.
    UndefinedVariable,
    /// An operator or function combined dimensions that cannot combine
    /// (SCHEMA-adjacent: never silently guesses a unit).
    DimensionMismatch,
    /// A `/` operator's right-hand operand evaluated to exactly zero.
    DivideByZero,
    /// A result (or intermediate value) is `NaN` or `±Infinity`.
    NonFinite,
    /// Source exceeds [`MAX_SOURCE_LEN`] bytes.
    TooLong,
    /// Parenthesized/call-argument nesting exceeds [`MAX_DEPTH`].
    TooDeep,
    /// A variable is defined, directly or transitively, in terms of itself.
    ///
    /// **This engine never produces it.** An [`Env`] lookup is a single opaque
    /// callback — nothing here knows that one variable's value is another
    /// variable's expression, so a reference graph (and therefore a cycle in
    /// one) simply does not exist at this layer. The variant lives here so the
    /// ONE error taxonomy callers route on covers every way an expression can
    /// fail to produce a number, including the ways only the caller can detect;
    /// it is raised by
    /// [`crate::regen::variables::resolve_variable_table`], which owns the
    /// document's variable graph.
    Cycle,
}
