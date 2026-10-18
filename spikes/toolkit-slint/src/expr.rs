//! Tiny length-expression evaluator for the dimension input: `+ - * /`, parentheses, unary sign,
//! and length units (mm default, cm, m, in, ") on numbers. The result is in millimetres.
//! The real app keeps its evaluator in onecad-core; this is spike-only.

#[derive(Debug, PartialEq)]
pub struct EvalError(pub String);

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

pub fn eval_mm(src: &str) -> Result<f64, EvalError> {
    let mut p = Parser {
        s: src.as_bytes(),
        i: 0,
    };
    let v = p.expr()?;
    p.ws();
    if p.i != p.s.len() {
        return Err(EvalError(format!("unexpected '{}'", &src[p.i..])));
    }
    if !v.is_finite() {
        return Err(EvalError("not a finite number".into()));
    }
    Ok(v)
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.ws();
        self.s.get(self.i).copied()
    }

    fn expr(&mut self) -> Result<f64, EvalError> {
        let mut v = self.term()?;
        while let Some(op @ (b'+' | b'-')) = self.peek() {
            self.i += 1;
            let r = self.term()?;
            v = if op == b'+' { v + r } else { v - r };
        }
        Ok(v)
    }

    fn term(&mut self) -> Result<f64, EvalError> {
        let mut v = self.factor()?;
        while let Some(op @ (b'*' | b'/')) = self.peek() {
            self.i += 1;
            let r = self.factor()?;
            if op == b'/' && r == 0.0 {
                return Err(EvalError("division by zero".into()));
            }
            v = if op == b'*' { v * r } else { v / r };
        }
        Ok(v)
    }

    fn factor(&mut self) -> Result<f64, EvalError> {
        match self.peek() {
            Some(b'-') => {
                self.i += 1;
                Ok(-self.factor()?)
            }
            Some(b'+') => {
                self.i += 1;
                self.factor()
            }
            Some(b'(') => {
                self.i += 1;
                let v = self.expr()?;
                if self.peek() != Some(b')') {
                    return Err(EvalError("missing ')'".into()));
                }
                self.i += 1;
                Ok(v * self.unit())
            }
            Some(c) if c.is_ascii_digit() || c == b'.' => {
                let start = self.i;
                while self.i < self.s.len()
                    && (self.s[self.i].is_ascii_digit() || self.s[self.i] == b'.')
                {
                    self.i += 1;
                }
                let text = std::str::from_utf8(&self.s[start..self.i]).unwrap_or("");
                let n: f64 = text
                    .parse()
                    .map_err(|_| EvalError(format!("bad number '{text}'")))?;
                Ok(n * self.unit())
            }
            Some(_) => Err(EvalError("expected a number".into())),
            None => Err(EvalError("empty expression".into())),
        }
    }

    /// Optional unit suffix; returns the factor to millimetres (1 when absent).
    fn unit(&mut self) -> f64 {
        self.ws();
        let rest = &self.s[self.i..];
        for (name, k) in [
            ("mm", 1.0),
            ("cm", 10.0),
            ("in", 25.4),
            ("\"", 25.4),
            ("m", 1000.0),
        ] {
            let n = name.len();
            let boundary = rest.get(n).is_none_or(|c| !c.is_ascii_alphabetic());
            if rest.starts_with(name.as_bytes()) && boundary {
                self.i += n;
                return k;
            }
        }
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_example() {
        assert_eq!(eval_mm("10 mm + 2*3"), Ok(16.0));
    }

    #[test]
    fn units_and_precedence() {
        assert_eq!(eval_mm("1 cm"), Ok(10.0));
        assert_eq!(eval_mm("1in"), Ok(25.4));
        assert_eq!(eval_mm("0.5 m - 100"), Ok(400.0));
        assert_eq!(eval_mm("(2 + 3) * 4"), Ok(20.0));
        assert_eq!(eval_mm("-(1+1)cm"), Ok(-20.0));
        assert_eq!(eval_mm("12 / 4 + 1"), Ok(4.0));
    }

    #[test]
    fn errors() {
        assert!(eval_mm("").is_err());
        assert!(eval_mm("1 +").is_err());
        assert!(eval_mm("1 / 0").is_err());
        assert!(eval_mm("2 mmx").is_err());
        assert!(eval_mm("(1").is_err());
    }
}
