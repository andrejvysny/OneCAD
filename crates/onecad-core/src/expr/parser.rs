//! Recursive-descent parser — turns the lexer's token stream into an
//! [`Expr`] tree.
//!
//! ```text
//! expression = term ;
//! term       = factor , { ("+"|"-") , factor } ;
//! factor     = unary , { ("*"|"/") , unary } ;
//! unary      = ["+"|"-"] , power ;
//! power      = atom , ["^" , unary] ;   (* right-assoc: -2^2 = -(2^2) *)
//! atom       = literal | ident "(" [args] ")" | ident | "(" expression ")" ;
//! args       = expression , { "," , expression } ;
//! ```

use std::collections::HashSet;

use super::dimension::{literal_value, Value};
use super::lexer::{lex, SpannedToken, Token};
use super::{ExprError, ExprErrorKind, MAX_DEPTH};

/// A unary prefix operator. `Plus` is a structural no-op (kept so `+2` has
/// a clean node/span rather than being silently dropped by the parser).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Plus,
    Neg,
}

/// A binary infix operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
}

/// The parsed expression tree. A literal's number + unit is folded to a
/// canonical [`Value`] at parse time (units are context-free); everything
/// else is resolved at evaluation time against an [`super::Env`] and a call
/// site [`super::Dimension`].
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Number {
        value: Value,
        span: (usize, usize),
    },
    Ident {
        name: String,
        span: (usize, usize),
    },
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
        span: (usize, usize),
    },
    Binary {
        op: BinaryOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
        span: (usize, usize),
    },
    Call {
        name: String,
        args: Vec<Expr>,
        span: (usize, usize),
    },
}

impl Expr {
    pub(crate) fn span(&self) -> (usize, usize) {
        match self {
            Expr::Number { span, .. }
            | Expr::Ident { span, .. }
            | Expr::Unary { span, .. }
            | Expr::Binary { span, .. }
            | Expr::Call { span, .. } => *span,
        }
    }
}

/// Parses `src` into an [`Expr`] tree. Rejects sources over
/// [`super::MAX_SOURCE_LEN`] bytes before lexing at all, and expressions
/// nested past [`MAX_DEPTH`] parens/call-args deep while parsing.
///
/// # Errors
/// [`ExprError`] on malformed source, over-length source, or over-deep
/// nesting — see [`super::ExprErrorKind`].
pub fn parse(src: &str) -> Result<Expr, ExprError> {
    if src.len() > super::MAX_SOURCE_LEN {
        return Err(ExprError::new(
            ExprErrorKind::TooLong,
            (0, src.len()),
            format!(
                "expression exceeds MAX_SOURCE_LEN ({} bytes)",
                super::MAX_SOURCE_LEN
            ),
        ));
    }
    let tokens = lex(src)?;
    let mut parser = Parser {
        tokens: &tokens,
        pos: 0,
        depth: 0,
        src_len: src.len(),
    };
    let expr = parser.parse_expression()?;
    if let Some(t) = parser.tokens.get(parser.pos) {
        return Err(ExprError::new(
            ExprErrorKind::Parse,
            t.span,
            "unexpected trailing input",
        ));
    }
    Ok(expr)
}

struct Parser<'a> {
    tokens: &'a [SpannedToken],
    pos: usize,
    depth: usize,
    src_len: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos).map(|t| &t.tok)
    }

    fn peek_span(&self) -> (usize, usize) {
        self.tokens
            .get(self.pos)
            .map_or((self.src_len, self.src_len), |t| t.span)
    }

    fn bump(&mut self) -> Option<SpannedToken> {
        let t = self.tokens.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn eof_error(&self, what: &str) -> ExprError {
        ExprError::new(
            ExprErrorKind::Parse,
            self.peek_span(),
            format!("expected {what}, found end of input"),
        )
    }

    fn expect_rparen(&mut self) -> Result<usize, ExprError> {
        match self.bump() {
            Some(t) if t.tok == Token::RParen => Ok(t.span.1),
            Some(t) => Err(ExprError::new(ExprErrorKind::Parse, t.span, "expected `)`")),
            None => Err(self.eof_error("`)`")),
        }
    }

    /// Bounds nesting via [`MAX_DEPTH`]: called for the top-level
    /// expression, each parenthesized sub-expression, and each function
    /// call argument — exactly the places a source can nest.
    fn parse_expression(&mut self) -> Result<Expr, ExprError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            let span = self.peek_span();
            self.depth -= 1;
            return Err(ExprError::new(
                ExprErrorKind::TooDeep,
                span,
                format!("expression nested past MAX_DEPTH ({MAX_DEPTH})"),
            ));
        }
        let result = self.parse_term();
        self.depth -= 1;
        result
    }

    fn parse_term(&mut self) -> Result<Expr, ExprError> {
        let mut lhs = self.parse_factor()?;
        loop {
            let op = match self.peek() {
                Some(Token::Plus) => BinaryOp::Add,
                Some(Token::Minus) => BinaryOp::Sub,
                _ => break,
            };
            self.bump();
            let rhs = self.parse_factor()?;
            let span = (lhs.span().0, rhs.span().1);
            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
        }
        Ok(lhs)
    }

    fn parse_factor(&mut self) -> Result<Expr, ExprError> {
        let mut lhs = self.parse_unary()?;
        loop {
            let op = match self.peek() {
                Some(Token::Star) => BinaryOp::Mul,
                Some(Token::Slash) => BinaryOp::Div,
                _ => break,
            };
            self.bump();
            let rhs = self.parse_unary()?;
            let span = (lhs.span().0, rhs.span().1);
            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<Expr, ExprError> {
        let op = match self.peek() {
            Some(Token::Plus) => Some(UnaryOp::Plus),
            Some(Token::Minus) => Some(UnaryOp::Neg),
            _ => None,
        };
        let Some(op) = op else {
            return self.parse_power();
        };
        let start = self.peek_span().0;
        self.bump();
        let operand = self.parse_power()?;
        let span = (start, operand.span().1);
        Ok(Expr::Unary {
            op,
            expr: Box::new(operand),
            span,
        })
    }

    /// Right-associative: recursing back into `unary` for the exponent
    /// makes `2^3^2` parse as `2^(3^2)` and `2^-3` parse the unary minus as
    /// part of the exponent, not a separate top-level negation.
    fn parse_power(&mut self) -> Result<Expr, ExprError> {
        let base = self.parse_atom()?;
        if !matches!(self.peek(), Some(Token::Caret)) {
            return Ok(base);
        }
        self.bump();
        let exponent = self.parse_unary()?;
        let span = (base.span().0, exponent.span().1);
        Ok(Expr::Binary {
            op: BinaryOp::Pow,
            lhs: Box::new(base),
            rhs: Box::new(exponent),
            span,
        })
    }

    fn parse_atom(&mut self) -> Result<Expr, ExprError> {
        let Some(spanned) = self.bump() else {
            return Err(self.eof_error("an expression"));
        };
        match spanned.tok {
            Token::Literal { value, unit } => Ok(Expr::Number {
                value: literal_value(value, unit),
                span: spanned.span,
            }),
            Token::Ident(name) => self.parse_ident_atom(name, spanned.span),
            Token::LParen => {
                let inner = self.parse_expression()?;
                self.expect_rparen()?;
                Ok(inner) // parens group; they are not their own node
            }
            _ => Err(ExprError::new(
                ExprErrorKind::Parse,
                spanned.span,
                "expected a number, identifier, or `(`",
            )),
        }
    }

    fn parse_ident_atom(
        &mut self,
        name: String,
        name_span: (usize, usize),
    ) -> Result<Expr, ExprError> {
        if !matches!(self.peek(), Some(Token::LParen)) {
            return Ok(Expr::Ident {
                name,
                span: name_span,
            });
        }
        self.bump();
        let args = self.parse_args()?;
        let end = self.expect_rparen()?;
        Ok(Expr::Call {
            name,
            args,
            span: (name_span.0, end),
        })
    }

    fn parse_args(&mut self) -> Result<Vec<Expr>, ExprError> {
        if matches!(self.peek(), Some(Token::RParen)) {
            return Ok(Vec::new());
        }
        let mut args = vec![self.parse_expression()?];
        while matches!(self.peek(), Some(Token::Comma)) {
            self.bump();
            args.push(self.parse_expression()?);
        }
        Ok(args)
    }
}

/// Identifiers referenced by `expr` (bare variable names, not call names),
/// in first-appearance order, deduplicated — feeds the WP-E2 dependency
/// graph and variable rename.
#[must_use]
pub fn references(expr: &Expr) -> Vec<&str> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    collect_references(expr, &mut seen, &mut out);
    out
}

fn collect_references<'a>(expr: &'a Expr, seen: &mut HashSet<&'a str>, out: &mut Vec<&'a str>) {
    match expr {
        Expr::Number { .. } => {}
        Expr::Ident { name, .. } => {
            if seen.insert(name.as_str()) {
                out.push(name.as_str());
            }
        }
        Expr::Unary { expr, .. } => collect_references(expr, seen, out),
        Expr::Binary { lhs, rhs, .. } => {
            collect_references(lhs, seen, out);
            collect_references(rhs, seen, out);
        }
        Expr::Call { args, .. } => {
            for arg in args {
                collect_references(arg, seen, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mul_binds_tighter_than_add() {
        // 1+2*3 must parse as Add(1, Mul(2,3)), not Mul(Add(1,2), 3).
        let e = parse("1+2*3").unwrap();
        match e {
            Expr::Binary {
                op: BinaryOp::Add,
                rhs,
                ..
            } => {
                assert!(matches!(
                    *rhs,
                    Expr::Binary {
                        op: BinaryOp::Mul,
                        ..
                    }
                ));
            }
            other => panic!("expected a top-level Add, got {other:?}"),
        }
    }

    #[test]
    fn power_is_right_associative() {
        // 2^3^2 = 2^(3^2): the RHS of the outer Pow is itself a Pow.
        let e = parse("2^3^2").unwrap();
        match e {
            Expr::Binary {
                op: BinaryOp::Pow,
                lhs,
                rhs,
                ..
            } => {
                assert!(matches!(*lhs, Expr::Number { .. }));
                assert!(matches!(
                    *rhs,
                    Expr::Binary {
                        op: BinaryOp::Pow,
                        ..
                    }
                ));
            }
            other => panic!("expected a top-level Pow, got {other:?}"),
        }
    }

    #[test]
    fn unary_minus_binds_looser_than_power() {
        // -2^2 = -(2^2): Unary wraps a Pow, not the other way round.
        let e = parse("-2^2").unwrap();
        match e {
            Expr::Unary {
                op: UnaryOp::Neg,
                expr,
                ..
            } => {
                assert!(matches!(
                    *expr,
                    Expr::Binary {
                        op: BinaryOp::Pow,
                        ..
                    }
                ));
            }
            other => panic!("expected a top-level Unary Neg, got {other:?}"),
        }
    }

    #[test]
    fn depth_at_the_cap_still_parses() {
        // MAX_DEPTH=32; the top-level call already counts as depth 1, so 31
        // nested parens lands exactly at the cap and must still succeed.
        let src = format!("{}1{}", "(".repeat(31), ")".repeat(31));
        assert!(parse(&src).is_ok());
    }

    #[test]
    fn depth_one_past_the_cap_is_too_deep() {
        let src = format!("{}1{}", "(".repeat(32), ")".repeat(32));
        let err = parse(&src).unwrap_err();
        assert_eq!(err.kind, ExprErrorKind::TooDeep);
    }

    #[test]
    fn references_are_in_first_appearance_order_and_deduped() {
        let e = parse("a+b*a+c").unwrap();
        assert_eq!(references(&e), vec!["a", "b", "c"]);
    }

    #[test]
    fn references_ignore_call_names() {
        let e = parse("sin(x)+y").unwrap();
        assert_eq!(references(&e), vec!["x", "y"]);
    }
}
