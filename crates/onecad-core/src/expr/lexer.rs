//! Tokenizer — turns source text into a flat token stream with byte spans.
//!
//! Unit adjacency (`2m` vs `2 m` vs `2*m`) is resolved HERE, once: a unit
//! suffix is recognised only when its alpha run immediately follows the
//! number's digits with zero intervening whitespace, and only when that run
//! is an EXACT match for one of the six keywords — no prefix matching, so
//! `1radius` is a lex error rather than a silent `1rad` + stray `ius`
//! identifier. Any other adjacency (`2 m`, with whitespace) leaves the
//! letters as a separate `Ident` token, which the parser then rejects as
//! two atoms with no operator between them.

use super::dimension::Unit;
use super::{ExprError, ExprErrorKind};

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Token {
    Literal { value: f64, unit: Option<Unit> },
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    Caret,
    LParen,
    RParen,
    Comma,
}

#[derive(Debug, Clone)]
pub(crate) struct SpannedToken {
    pub tok: Token,
    pub span: (usize, usize),
}

/// Tokenizes `src`. The caller has already checked `src.len()` against
/// [`super::MAX_SOURCE_LEN`].
pub(crate) fn lex(src: &str) -> Result<Vec<SpannedToken>, ExprError> {
    let chars: Vec<(usize, char)> = src.char_indices().collect();
    let n = chars.len();
    let byte_len = src.len();
    let mut i = 0usize;
    let mut out = Vec::new();

    while i < n {
        let (start, c) = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c.is_ascii_digit() || c == '.' {
            let (tok, next_i) = lex_number(&chars, i, src, byte_len)?;
            out.push(spanned(tok, start, &chars, next_i, byte_len));
            i = next_i;
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let (name, next_i) = lex_ident(&chars, i, src, byte_len);
            out.push(spanned(Token::Ident(name), start, &chars, next_i, byte_len));
            i = next_i;
            continue;
        }
        let tok = match c {
            '+' => Token::Plus,
            '-' => Token::Minus,
            '*' => Token::Star,
            '/' => Token::Slash,
            '^' => Token::Caret,
            '(' => Token::LParen,
            ')' => Token::RParen,
            ',' => Token::Comma,
            other => {
                let end = start + other.len_utf8();
                return Err(ExprError::new(
                    ExprErrorKind::Parse,
                    (start, end),
                    format!("unexpected character `{other}`"),
                ));
            }
        };
        out.push(spanned(tok, start, &chars, i + 1, byte_len));
        i += 1;
    }
    Ok(out)
}

/// Byte offset just past `chars[at]` (or end-of-source if `at` is past the
/// end) — every single-char/run-based token needs this for its span end.
fn byte_end_at(chars: &[(usize, char)], at: usize, byte_len: usize) -> usize {
    chars.get(at).map_or(byte_len, |(b, _)| *b)
}

fn spanned(
    tok: Token,
    start: usize,
    chars: &[(usize, char)],
    next_i: usize,
    byte_len: usize,
) -> SpannedToken {
    SpannedToken {
        tok,
        span: (start, byte_end_at(chars, next_i, byte_len)),
    }
}

/// `number = digits ["." [digits]] [exponent] | "." digits [exponent]`,
/// `exponent = ("e"|"E") ["+"|"-"] digits`, plus an optional adjacent unit
/// suffix folded into the same token.
fn lex_number(
    chars: &[(usize, char)],
    i: usize,
    src: &str,
    byte_len: usize,
) -> Result<(Token, usize), ExprError> {
    let start_byte = chars[i].0;
    let after_digits = scan_number_digits(chars, i);
    let num_end_byte = byte_end_at(chars, after_digits, byte_len);
    let text = &src[start_byte..num_end_byte];
    let value: f64 = text.parse().map_err(|_| {
        ExprError::new(
            ExprErrorKind::Parse,
            (start_byte, num_end_byte),
            format!("invalid number literal `{text}`"),
        )
    })?;
    let (unit, next_i) = scan_unit_suffix(chars, after_digits, src, byte_len)?;
    Ok((Token::Literal { value, unit }, next_i))
}

/// Consumes `digits ["." [digits]] [exponent]` starting at `i`, returning
/// the index just past the number text. Never fails — the caller parses the
/// sliced text with `f64::from_str`, which does the actual validation.
fn scan_number_digits(chars: &[(usize, char)], mut i: usize) -> usize {
    let n = chars.len();
    while i < n && chars[i].1.is_ascii_digit() {
        i += 1;
    }
    if i < n && chars[i].1 == '.' {
        i += 1;
        while i < n && chars[i].1.is_ascii_digit() {
            i += 1;
        }
    }
    if i < n && (chars[i].1 == 'e' || chars[i].1 == 'E') {
        let mut j = i + 1;
        if j < n && (chars[j].1 == '+' || chars[j].1 == '-') {
            j += 1;
        }
        if j < n && chars[j].1.is_ascii_digit() {
            while j < n && chars[j].1.is_ascii_digit() {
                j += 1;
            }
            i = j;
        }
        // else: the 'e'/'E' is not an exponent marker (no digit follows) —
        // leave it for the adjacent-unit check, where it fails as an
        // unrecognized suffix.
    }
    i
}

/// Recognizes an adjacent (zero-whitespace) unit suffix right after a
/// number's digits — a WHOLE-keyword match only, so `1radius` is a lex
/// error rather than a silent `1rad` + stray `ius` identifier.
fn scan_unit_suffix(
    chars: &[(usize, char)],
    mut i: usize,
    src: &str,
    byte_len: usize,
) -> Result<(Option<Unit>, usize), ExprError> {
    let n = chars.len();
    if i >= n || !chars[i].1.is_ascii_alphabetic() {
        return Ok((None, i));
    }
    let suffix_start_byte = chars[i].0;
    while i < n && chars[i].1.is_ascii_alphabetic() {
        i += 1;
    }
    let suffix_end_byte = byte_end_at(chars, i, byte_len);
    let suffix = &src[suffix_start_byte..suffix_end_byte];
    let unit = Unit::from_suffix(suffix).ok_or_else(|| {
        ExprError::new(
            ExprErrorKind::Parse,
            (suffix_start_byte, suffix_end_byte),
            format!("`{suffix}` is not a recognized unit (mm, cm, m, in, deg, rad)"),
        )
    })?;
    Ok((Some(unit), i))
}

/// `ident = (ALPHA|"_") {ALPHA|DIGIT|"_"}`.
fn lex_ident(chars: &[(usize, char)], mut i: usize, src: &str, byte_len: usize) -> (String, usize) {
    let n = chars.len();
    let start_byte = chars[i].0;
    i += 1; // first char already validated alpha|_ by the caller
    while i < n && (chars[i].1.is_ascii_alphanumeric() || chars[i].1 == '_') {
        i += 1;
    }
    let end_byte = byte_end_at(chars, i, byte_len);
    (src[start_byte..end_byte].to_string(), i)
}

#[cfg(test)]
mod tests {
    use super::super::dimension::Unit;
    use super::*;

    fn kinds(src: &str) -> Vec<Token> {
        lex(src).unwrap().into_iter().map(|t| t.tok).collect()
    }

    #[test]
    fn adjacent_suffix_is_folded_into_the_literal() {
        assert_eq!(
            kinds("2mm"),
            vec![Token::Literal {
                value: 2.0,
                unit: Some(Unit::Mm)
            }]
        );
    }

    #[test]
    fn whitespace_before_a_would_be_suffix_splits_into_two_tokens() {
        assert_eq!(
            kinds("2 m"),
            vec![
                Token::Literal {
                    value: 2.0,
                    unit: None
                },
                Token::Ident("m".to_string())
            ]
        );
    }

    #[test]
    fn an_operator_between_number_and_letters_is_not_adjacency() {
        assert_eq!(
            kinds("2*m"),
            vec![
                Token::Literal {
                    value: 2.0,
                    unit: None
                },
                Token::Star,
                Token::Ident("m".to_string())
            ]
        );
    }

    #[test]
    fn a_non_unit_adjacent_run_is_a_lex_error_not_a_prefix_match() {
        // "radius" starts with "rad" but is not itself a unit keyword — this
        // must fail loudly rather than silently read `1rad` + stray `ius`.
        let err = lex("1radius").unwrap_err();
        assert_eq!(err.kind, ExprErrorKind::Parse);
    }

    #[test]
    fn leading_dot_number_lexes_as_a_literal() {
        assert_eq!(
            kinds(".5"),
            vec![Token::Literal {
                value: 0.5,
                unit: None
            }]
        );
    }

    #[test]
    fn exponent_and_unit_suffix_compose_in_one_token() {
        // The lexer stores the RAW parsed value; canonical unit conversion
        // happens later, in `dimension::literal_value`.
        assert_eq!(
            kinds("1e3mm"),
            vec![Token::Literal {
                value: 1000.0,
                unit: Some(Unit::Mm)
            }]
        );
    }
}
