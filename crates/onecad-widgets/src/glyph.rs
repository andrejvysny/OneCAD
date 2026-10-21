//! Crisp vector UI glyphs (close, chevrons, search, eye, check, plus, more) drawn with
//! the painter as 1.5px strokes. The baked SVG icons render these too small and faint at
//! 10–14px, so small chrome glyphs bypass them. `size` is the glyph's layout box; the
//! drawn extent is a fraction of it, like the design's 24-unit icon grid.

use egui::{Color32, Painter, Pos2, Shape, Stroke, vec2};

const WEIGHT: f32 = 1.5;

fn stroke(col: Color32) -> Stroke {
    Stroke::new(WEIGHT, col)
}

fn poly(p: &Painter, c: Pos2, size: f32, pts: &[(f32, f32)], col: Color32) {
    let pts: Vec<Pos2> = pts.iter().map(|&(x, y)| c + vec2(x, y) * size).collect();
    p.add(Shape::line(pts, stroke(col)));
}

pub(crate) fn close(p: &Painter, c: Pos2, size: f32, col: Color32) {
    let h = 0.32;
    poly(p, c, size, &[(-h, -h), (h, h)], col);
    poly(p, c, size, &[(h, -h), (-h, h)], col);
}

pub(crate) fn plus(p: &Painter, c: Pos2, size: f32, col: Color32) {
    let h = 0.34;
    poly(p, c, size, &[(0.0, -h), (0.0, h)], col);
    poly(p, c, size, &[(-h, 0.0), (h, 0.0)], col);
}

pub(crate) fn chevron_down(p: &Painter, c: Pos2, size: f32, col: Color32) {
    poly(p, c, size, &[(-0.3, -0.13), (0.0, 0.17), (0.3, -0.13)], col);
}

pub(crate) fn chevron_up(p: &Painter, c: Pos2, size: f32, col: Color32) {
    poly(p, c, size, &[(-0.3, 0.13), (0.0, -0.17), (0.3, 0.13)], col);
}

pub(crate) fn chevron_right(p: &Painter, c: Pos2, size: f32, col: Color32) {
    poly(p, c, size, &[(-0.13, -0.3), (0.17, 0.0), (-0.13, 0.3)], col);
}

pub(crate) fn check(p: &Painter, c: Pos2, size: f32, col: Color32) {
    poly(p, c, size, &[(-0.3, 0.02), (-0.1, 0.22), (0.3, -0.22)], col);
}

pub(crate) fn search(p: &Painter, c: Pos2, size: f32, col: Color32) {
    let ring = c + vec2(-0.08, -0.08) * size;
    let r = 0.27 * size;
    p.circle_stroke(ring, r, stroke(col));
    let d = r * std::f32::consts::FRAC_1_SQRT_2;
    let from = ring + vec2(d, d);
    let to = c + vec2(0.34, 0.34) * size;
    p.add(Shape::line_segment([from, to], stroke(col)));
}

fn eye_outline(p: &Painter, c: Pos2, size: f32, col: Color32) {
    let (w, h, n) = (0.44, 0.27, 12);
    for sign in [-1.0_f32, 1.0] {
        let pts: Vec<(f32, f32)> = (0..=n)
            .map(|i| {
                let x = -w + 2.0 * w * i as f32 / n as f32;
                let k = 1.0 - (x / w).powi(2);
                (x, sign * h * k.max(0.0).sqrt())
            })
            .collect();
        poly(p, c, size, &pts, col);
    }
}

pub(crate) fn eye(p: &Painter, c: Pos2, size: f32, col: Color32) {
    eye_outline(p, c, size, col);
    p.circle_stroke(c, 0.1 * size, stroke(col));
}

pub(crate) fn eye_off(p: &Painter, c: Pos2, size: f32, col: Color32) {
    eye_outline(p, c, size, col);
    poly(p, c, size, &[(-0.34, 0.34), (0.34, -0.34)], col);
}

/// Degree sign: a small ring raised above the centre.
pub(crate) fn degree(p: &Painter, c: Pos2, size: f32, col: Color32) {
    p.circle_stroke(c, 0.18 * size, stroke(col));
}

/// Three horizontal dots ("more").
pub(crate) fn more(p: &Painter, c: Pos2, size: f32, col: Color32) {
    for dx in [-0.3, 0.0, 0.3] {
        p.circle_filled(c + vec2(dx, 0.0) * size, 1.2, col);
    }
}

/// Paints the glyph for a baked-icon name when one has a vector twin; `false` otherwise.
pub(crate) fn by_name(p: &Painter, name: &str, c: Pos2, size: f32, col: Color32) -> bool {
    let f: fn(&Painter, Pos2, f32, Color32) = match name {
        "close" => close,
        "plus" => plus,
        "chevronDown" => chevron_down,
        "check" => check,
        "search" => search,
        "eyeOn" => eye,
        "eyeOff" => eye_off,
        "overflow" => more,
        _ => return false,
    };
    f(p, c, size, col);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_glyph_paints_shapes() {
        let ctx = egui::Context::default();
        let layer = egui::LayerId::background();
        let p = Painter::new(ctx, layer, egui::Rect::EVERYTHING);
        for name in [
            "close",
            "plus",
            "chevronDown",
            "check",
            "search",
            "eyeOn",
            "eyeOff",
            "overflow",
        ] {
            assert!(by_name(&p, name, Pos2::new(5.0, 5.0), 14.0, Color32::BLACK));
        }
        assert!(!by_name(
            &p,
            "extrude",
            Pos2::new(5.0, 5.0),
            14.0,
            Color32::BLACK
        ));
        chevron_right(&p, Pos2::new(5.0, 5.0), 14.0, Color32::BLACK);
    }
}
