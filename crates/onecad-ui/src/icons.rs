//! OneCAD's two-tone icons, read from the TS registry (`assets/native/cppIcons.generated.ts`,
//! `assets/native/authored.ts`) and painted as egui strokes. Paths are SVG data on a 24-unit grid;
//! curves and arcs are flattened to polylines once at startup.

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, pos2};
use std::collections::HashMap;
use std::sync::OnceLock;

const CPP: &str = include_str!("../../../assets/native/cppIcons.generated.ts");
const AUTHORED: &str = include_str!("../../../assets/native/authored.ts");

struct Element {
    subpaths: Vec<(Vec<Pos2>, bool)>,
    accent: bool,
    filled: bool,
    sw: f32,
    dash: Option<(f32, f32)>,
}

fn quoted<'a>(s: &'a str, key: &str) -> Option<&'a str> {
    let at = s.find(key)? + key.len();
    let rest = s[at..].trim_start().strip_prefix('"')?;
    Some(&rest[..rest.find('"')?])
}

fn number_after(s: &str, key: &str) -> Option<f32> {
    let at = s.find(key)? + key.len();
    let rest = s[at..].trim_start();
    let end = rest
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

/// Scan a TS registry for `name: [ { d: "...", tone, filled, sw, dash }, ... ]` entries,
/// one-line or multi-line.
fn parse_registry(src: &str, out: &mut HashMap<String, Vec<Element>>) {
    let mut rest = src;
    while let Some(at) = rest.find(": [") {
        let head = &rest[..at];
        let name_start = head
            .rfind(|c: char| !c.is_ascii_alphanumeric())
            .map_or(0, |i| i + 1);
        let name = &head[name_start..];
        let body_start = at + 3;
        let Some(len) = rest[body_start..].find(']') else {
            break;
        };
        let body = &rest[body_start..body_start + len];
        rest = &rest[body_start + len..];
        if name.is_empty() || !body.trim_start().starts_with('{') {
            continue;
        }
        let mut list = Vec::new();
        for obj in body.split('}') {
            let Some(d) = quoted(obj, "d:") else { continue };
            let dash = quoted(obj, "dash:").and_then(|s| {
                let mut it = s.split_whitespace().filter_map(|x| x.parse().ok());
                Some((it.next()?, it.next()?))
            });
            list.push(Element {
                subpaths: flatten(d),
                accent: obj.contains("tone: \"accent\""),
                filled: obj.contains("filled: true"),
                sw: number_after(obj, "sw:").unwrap_or(1.0),
                dash,
            });
        }
        out.insert(name.to_owned(), list);
    }
}

fn registry() -> &'static HashMap<String, Vec<Element>> {
    static R: OnceLock<HashMap<String, Vec<Element>>> = OnceLock::new();
    R.get_or_init(|| {
        let mut m = HashMap::new();
        parse_registry(CPP, &mut m);
        parse_registry(AUTHORED, &mut m);
        m
    })
}

pub fn count() -> usize {
    registry().len()
}

/// Tokenize SVG path data into commands and numbers.
fn tokens(d: &str) -> Vec<Tok> {
    let b = d.as_bytes();
    let (mut i, mut out) = (0, Vec::new());
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_alphabetic() && c != b'e' && c != b'E' {
            out.push(Tok::Cmd(c));
            i += 1;
        } else if c.is_ascii_digit() || c == b'.' || c == b'-' || c == b'+' {
            let start = i;
            i += 1;
            let mut dot = c == b'.';
            while i < b.len() {
                let x = b[i];
                if x.is_ascii_digit() {
                    i += 1;
                } else if x == b'.' && !dot {
                    dot = true;
                    i += 1;
                } else if (x == b'e' || x == b'E') && i + 1 < b.len() {
                    i += 2;
                } else {
                    break;
                }
            }
            out.push(Tok::Num(d[start..i].parse().unwrap_or(0.0)));
        } else {
            i += 1;
        }
    }
    out
}

#[derive(Clone, Copy)]
enum Tok {
    Cmd(u8),
    Num(f32),
}

fn arc_points(p0: Pos2, r: [f32; 2], rot: f32, large: bool, sweep: bool, p1: Pos2) -> Vec<Pos2> {
    // SVG endpoint → center parameterization (SVG 1.1 implementation notes F.6.5).
    let (mut rx, mut ry) = (r[0].abs(), r[1].abs());
    if rx < 1e-6 || ry < 1e-6 {
        return vec![p1];
    }
    let (s, c) = rot.to_radians().sin_cos();
    let (dx, dy) = ((p0.x - p1.x) * 0.5, (p0.y - p1.y) * 0.5);
    let (x1, y1) = (c * dx + s * dy, -s * dx + c * dy);
    let lambda = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    if lambda > 1.0 {
        rx *= lambda.sqrt();
        ry *= lambda.sqrt();
    }
    let num = (rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1).max(0.0);
    let den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let mut k = (num / den.max(1e-12)).sqrt();
    if large == sweep {
        k = -k;
    }
    let (cx1, cy1) = (k * rx * y1 / ry, -k * ry * x1 / rx);
    let (cx, cy) = (
        c * cx1 - s * cy1 + (p0.x + p1.x) * 0.5,
        s * cx1 + c * cy1 + (p0.y + p1.y) * 0.5,
    );
    let ang = |ux: f32, uy: f32| uy.atan2(ux);
    let t0 = ang((x1 - cx1) / rx, (y1 - cy1) / ry);
    let mut dt = ang((-x1 - cx1) / rx, (-y1 - cy1) / ry) - t0;
    let tau = std::f32::consts::TAU;
    if sweep && dt < 0.0 {
        dt += tau;
    } else if !sweep && dt > 0.0 {
        dt -= tau;
    }
    let n = ((dt.abs() / tau) * 48.0).ceil().max(2.0) as usize;
    (1..=n)
        .map(|i| {
            let t = t0 + dt * i as f32 / n as f32;
            let (ex, ey) = (rx * t.cos(), ry * t.sin());
            pos2(c * ex - s * ey + cx, s * ex + c * ey + cy)
        })
        .collect()
}

/// SVG path → polylines (closed flag per subpath). Supports M L H V C Q A Z and lower case.
fn flatten(d: &str) -> Vec<(Vec<Pos2>, bool)> {
    let toks = tokens(d);
    let (mut out, mut cur, mut start) = (Vec::new(), Vec::<Pos2>::new(), pos2(0.0, 0.0));
    let (mut p, mut i, mut cmd) = (pos2(0.0, 0.0), 0, b'M');
    let num = |i: &mut usize| -> f32 {
        let v = match toks.get(*i) {
            Some(Tok::Num(v)) => *v,
            _ => 0.0,
        };
        *i += 1;
        v
    };
    while i < toks.len() {
        if let Tok::Cmd(c) = toks[i] {
            cmd = c;
            i += 1;
            if c == b'Z' || c == b'z' {
                if cur.len() > 1 {
                    out.push((std::mem::take(&mut cur), true));
                }
                p = start;
                continue;
            }
        }
        let rel = cmd.is_ascii_lowercase();
        let base = if rel { p } else { pos2(0.0, 0.0) };
        match cmd.to_ascii_uppercase() {
            b'M' => {
                if cur.len() > 1 {
                    out.push((std::mem::take(&mut cur), false));
                }
                cur.clear();
                p = pos2(base.x + num(&mut i), base.y + num(&mut i));
                start = p;
                cur.push(p);
                cmd = if rel { b'l' } else { b'L' };
            }
            b'L' => {
                p = pos2(base.x + num(&mut i), base.y + num(&mut i));
                cur.push(p);
            }
            b'H' => {
                p = pos2(if rel { p.x } else { 0.0 } + num(&mut i), p.y);
                cur.push(p);
            }
            b'V' => {
                p = pos2(p.x, if rel { p.y } else { 0.0 } + num(&mut i));
                cur.push(p);
            }
            b'C' | b'Q' => {
                let cubic = cmd.eq_ignore_ascii_case(&b'C');
                let n = if cubic { 3 } else { 2 };
                let pts: Vec<Pos2> = (0..n)
                    .map(|_| pos2(base.x + num(&mut i), base.y + num(&mut i)))
                    .collect();
                for k in 1..=12 {
                    let t = k as f32 / 12.0;
                    let u = 1.0 - t;
                    let q = if cubic {
                        p.to_vec2() * u * u * u
                            + pts[0].to_vec2() * 3.0 * u * u * t
                            + pts[1].to_vec2() * 3.0 * u * t * t
                            + pts[2].to_vec2() * t * t * t
                    } else {
                        p.to_vec2() * u * u
                            + pts[0].to_vec2() * 2.0 * u * t
                            + pts[1].to_vec2() * t * t
                    };
                    cur.push(q.to_pos2());
                }
                p = pts[n - 1];
            }
            b'A' => {
                let r = [num(&mut i), num(&mut i)];
                let rot = num(&mut i);
                let (large, sweep) = (num(&mut i) != 0.0, num(&mut i) != 0.0);
                let end = pos2(base.x + num(&mut i), base.y + num(&mut i));
                cur.extend(arc_points(p, r, rot, large, sweep, end));
                p = end;
            }
            _ => i += 1,
        }
    }
    if cur.len() > 1 {
        out.push((cur, false));
    }
    out
}

fn dashed(points: &[Pos2], on: f32, off: f32) -> Vec<Vec<Pos2>> {
    let (mut out, mut cur, mut left, mut drawing) = (Vec::new(), vec![points[0]], on, true);
    for w in points.windows(2) {
        let (mut a, b) = (w[0], w[1]);
        let mut seg = (b - a).length();
        while seg > left {
            let m = a + (b - a).normalized() * left;
            if drawing {
                cur.push(m);
                out.push(std::mem::take(&mut cur));
            } else {
                cur = vec![m];
            }
            seg -= left;
            a = m;
            drawing = !drawing;
            left = if drawing { on } else { off };
        }
        left -= seg;
        if drawing {
            cur.push(b);
        }
    }
    if drawing && cur.len() > 1 {
        out.push(cur);
    }
    out
}

/// egui's feathering computes a normal per edge, so a zero-length edge (an arc ending on its
/// start point, a repeated vertex) becomes a spike. Drop consecutive duplicates.
fn dedup(mut pts: Vec<Pos2>, closed: bool) -> Vec<Pos2> {
    pts.dedup_by(|a, b| (*a - *b).length_sq() < 1e-4);
    if closed && pts.len() > 2 && (pts[0] - pts[pts.len() - 1]).length_sq() < 1e-4 {
        pts.pop();
    }
    pts
}

/// Paint icon `name` into `rect` (square), primary tone `fg`, accent `accent`, stroke width
/// `weight` in 24-grid units (the TS call sites' `strokeWidth`). Returns false if unknown.
pub fn paint(
    painter: &Painter,
    rect: Rect,
    name: &str,
    fg: Color32,
    accent: Color32,
    weight: f32,
) -> bool {
    let Some(elements) = registry().get(name) else {
        return false;
    };
    let s = rect.width() / 24.0;
    let map = |q: &Pos2| pos2(rect.min.x + q.x * s, rect.min.y + q.y * s);
    for e in elements {
        let color = if e.accent { accent } else { fg };
        for (pts, closed) in &e.subpaths {
            let pts = dedup(pts.iter().map(map).collect(), *closed || e.filled);
            if pts.len() < 2 {
                continue;
            }
            if e.filled {
                painter.add(Shape::convex_polygon(pts, color, Stroke::NONE));
                continue;
            }
            let stroke = Stroke::new(weight * e.sw * s, color);
            match (e.dash, *closed) {
                (Some((on, off)), _) => {
                    let mut ring = pts.clone();
                    if *closed {
                        ring.push(pts[0]);
                    }
                    for part in dashed(&ring, on * s, off * s) {
                        painter.add(Shape::line(part, stroke));
                    }
                }
                (None, true) => {
                    painter.add(Shape::closed_line(pts, stroke));
                }
                (None, false) => {
                    painter.add(Shape::line(pts, stroke));
                }
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    #[test]
    fn registry_has_toolbar_icons() {
        for name in [
            "select",
            "sketch",
            "plane",
            "extrude",
            "revolve",
            "fillet",
            "booleanUnion",
            "shell",
            "pushpull",
            "hole",
            "patternLinear",
            "patternCircular",
            "mirrorBody",
            "move",
            "measure",
            "plus",
            "chevronDown",
            "eyeOn",
            "body",
            "home",
            "fit",
            "stack",
        ] {
            assert!(super::registry().contains_key(name), "{name}");
        }
        assert!(super::count() > 90);
    }

    #[test]
    fn compact_path_numbers() {
        let p = super::flatten("M12.22 2h-.44a2 2 0 0 0-2 2v.18z");
        assert_eq!(p.len(), 1);
        assert!((p[0].0[1].x - 11.78).abs() < 1e-4);
    }
}
