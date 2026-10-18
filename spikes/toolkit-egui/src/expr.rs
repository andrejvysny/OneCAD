//! Tiny length-expression evaluator for the dimension input: `10 mm + 2*3` → 16 mm.
//! Numbers, + - * / parentheses, unary sign, units mm cm m in ft. A bare number is a scalar;
//! a scalar added to a length is taken in the default unit (mm), as CAD fields do.

#[derive(Clone, Copy, Debug, PartialEq)]
struct Q {
    v: f64,
    /// Length exponent: 0 scalar, 1 length, 2 area...
    dim: i32,
}

struct P<'a> {
    s: &'a [u8],
    i: usize,
}

/// Evaluate `text` to millimetres.
pub fn eval_mm(text: &str) -> Result<f64, String> {
    let mut p = P {
        s: text.as_bytes(),
        i: 0,
    };
    let q = p.expr()?;
    p.ws();
    if p.i != p.s.len() {
        return Err(format!("unexpected '{}'", &text[p.i..]));
    }
    if q.dim > 1 || q.dim < 0 {
        return Err("not a length".into());
    }
    if !q.v.is_finite() {
        return Err("not a finite number".into());
    }
    Ok(q.v)
}

fn add(a: Q, b: Q, sign: f64) -> Result<Q, String> {
    let dim = match (a.dim, b.dim) {
        (x, y) if x == y => x,
        (0, 1) | (1, 0) => 1,
        _ => return Err("mismatched units".into()),
    };
    Ok(Q {
        v: a.v + sign * b.v,
        dim,
    })
}

impl P<'_> {
    fn ws(&mut self) {
        while self.s.get(self.i).is_some_and(u8::is_ascii_whitespace) {
            self.i += 1;
        }
    }
    fn peek(&mut self) -> Option<u8> {
        self.ws();
        self.s.get(self.i).copied()
    }
    fn expr(&mut self) -> Result<Q, String> {
        let mut a = self.term()?;
        while let Some(c @ (b'+' | b'-')) = self.peek() {
            self.i += 1;
            let b = self.term()?;
            a = add(a, b, if c == b'+' { 1.0 } else { -1.0 })?;
        }
        Ok(a)
    }
    fn term(&mut self) -> Result<Q, String> {
        let mut a = self.factor()?;
        while let Some(c @ (b'*' | b'/')) = self.peek() {
            self.i += 1;
            let b = self.factor()?;
            a = if c == b'*' {
                Q {
                    v: a.v * b.v,
                    dim: a.dim + b.dim,
                }
            } else {
                if b.v == 0.0 {
                    return Err("division by zero".into());
                }
                Q {
                    v: a.v / b.v,
                    dim: a.dim - b.dim,
                }
            };
        }
        Ok(a)
    }
    fn factor(&mut self) -> Result<Q, String> {
        match self.peek() {
            Some(b'-') => {
                self.i += 1;
                let q = self.factor()?;
                Ok(Q { v: -q.v, ..q })
            }
            Some(b'+') => {
                self.i += 1;
                self.factor()
            }
            _ => {
                let q = self.primary()?;
                self.unit(q)
            }
        }
    }
    fn primary(&mut self) -> Result<Q, String> {
        match self.peek() {
            Some(b'(') => {
                self.i += 1;
                let q = self.expr()?;
                if self.peek() != Some(b')') {
                    return Err("missing ')'".into());
                }
                self.i += 1;
                Ok(q)
            }
            Some(c) if c.is_ascii_digit() || c == b'.' => {
                let start = self.i;
                while self
                    .s
                    .get(self.i)
                    .is_some_and(|c| c.is_ascii_digit() || *c == b'.')
                {
                    self.i += 1;
                }
                let t = std::str::from_utf8(&self.s[start..self.i]).map_err(|e| e.to_string())?;
                let v = t.parse::<f64>().map_err(|_| format!("bad number '{t}'"))?;
                Ok(Q { v, dim: 0 })
            }
            Some(c) => Err(format!("unexpected '{}'", c as char)),
            None => Err("expected a value".into()),
        }
    }
    fn unit(&mut self, q: Q) -> Result<Q, String> {
        self.ws();
        let start = self.i;
        while self.s.get(self.i).is_some_and(u8::is_ascii_alphabetic) {
            self.i += 1;
        }
        let scale = match &self.s[start..self.i] {
            b"" => return Ok(q),
            b"mm" => 1.0,
            b"cm" => 10.0,
            b"m" => 1000.0,
            b"in" => 25.4,
            b"ft" => 304.8,
            u => return Err(format!("unknown unit '{}'", String::from_utf8_lossy(u))),
        };
        if q.dim != 0 {
            return Err("unit on a length".into());
        }
        Ok(Q {
            v: q.v * scale,
            dim: 1,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::eval_mm;

    #[test]
    fn task_example() {
        assert_eq!(eval_mm("10 mm + 2*3"), Ok(16.0));
    }

    #[test]
    fn precedence_parens_units() {
        assert_eq!(eval_mm("(10 + 2) * 3"), Ok(36.0));
        assert_eq!(eval_mm("1 in"), Ok(25.4));
        assert_eq!(eval_mm("-2cm + 5"), Ok(-15.0));
        assert_eq!(eval_mm("42.5"), Ok(42.5));
        assert_eq!(eval_mm("10mm*2/4"), Ok(5.0));
    }

    #[test]
    fn rejects() {
        assert!(eval_mm("").is_err());
        assert!(eval_mm("10 mm * 2 mm").is_err());
        assert!(eval_mm("10 / 0").is_err());
        assert!(eval_mm("10 parsecs").is_err());
        assert!(eval_mm("(1 + 2").is_err());
        assert!(eval_mm("1 2").is_err());
    }
}
