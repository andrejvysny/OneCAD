//! Extension puzzle glyph: the host's mark for tools, tooltips and sections an extension adds.

use egui::{Color32, CornerRadius, Painter, Pos2, vec2};

/// Solid puzzle piece (square body + two knobs) centred on `c`; `halo` paints a 2px ring
/// behind it so the glyph stays legible over icons.
pub fn puzzle(p: &Painter, c: Pos2, size: f32, colour: Color32, halo: Option<Color32>) {
    let pass = |grow: f32, col: Color32| {
        let body = size * 0.64;
        let r = CornerRadius::same(((size * 0.12) + grow).round().clamp(0.0, 255.0) as u8);
        let b = egui::Rect::from_center_size(
            c + vec2(-size * 0.08, size * 0.08),
            vec2(body + 2.0 * grow, body + 2.0 * grow),
        );
        p.rect_filled(b, r, col);
        let knob = size * 0.17 + grow;
        p.circle_filled(c + vec2(size * 0.34 - size * 0.08, size * 0.08), knob, col);
        p.circle_filled(
            c + vec2(-size * 0.08, -size * 0.34 + size * 0.08),
            knob,
            col,
        );
    };
    if let Some(h) = halo {
        pass(2.0, h);
    }
    pass(0.0, colour);
}
