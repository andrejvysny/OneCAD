//! Screen-space dimension graphics (Shapr3D-style); pure geometry, painting is separate.
use egui::{Pos2, Rect, Vec2};

pub(crate) const OFFSET_PX: f32 = 24.0;
const EXT_OVERSHOOT_PX: f32 = 4.0;
const ARROW_LEN: f32 = 7.0;
const ARROW_WIDTH: f32 = 5.0;
const ARC_STEPS: usize = 24;

pub(crate) type Arrow = [Pos2; 3];

#[derive(Clone, Debug)]
pub(crate) struct LinearDim {
    pub dim_line: [Pos2; 2],
    pub ext: [[Pos2; 2]; 2],
    pub arrows: [Arrow; 2],
    pub label_center: Pos2,
}

#[derive(Clone, Debug)]
pub(crate) struct RadialDim {
    pub leader: [Pos2; 2],
    pub arrow: Arrow,
    pub label_center: Pos2,
}

#[derive(Clone, Debug)]
pub(crate) struct AngularDim {
    pub arc: Vec<Pos2>,
    pub arrows: [Arrow; 2],
    pub label_center: Pos2,
}

#[derive(Clone, Debug)]
pub(crate) enum Graphic {
    Linear(LinearDim),
    Radial(RadialDim),
    Angular(AngularDim),
}

impl Graphic {
    pub(crate) fn label_center(&self) -> Pos2 {
        match self {
            Self::Linear(dim) => dim.label_center,
            Self::Radial(dim) => dim.label_center,
            Self::Angular(dim) => dim.label_center,
        }
    }
}

fn unit(vector: Vec2, fallback: Vec2) -> Vec2 {
    let length = vector.length();
    if length.is_finite() && length > 1e-6 {
        vector / length
    } else {
        fallback
    }
}

fn arrow(tip: Pos2, outward: Vec2) -> Arrow {
    let base = tip - outward * ARROW_LEN;
    let side = egui::vec2(-outward.y, outward.x) * (ARROW_WIDTH * 0.5);
    [tip, base + side, base - side]
}

/// Perpendicular unit of a-to-b, flipped so a positive offset points away from `hint`.
pub(crate) fn offset_away(a: Pos2, b: Pos2, hint: Option<Pos2>) -> f32 {
    let along = unit(b - a, egui::vec2(1.0, 0.0));
    let normal = egui::vec2(-along.y, along.x);
    match hint {
        Some(hint) if (hint - a.lerp(b, 0.5)).dot(normal) > 0.0 => -OFFSET_PX,
        _ => OFFSET_PX,
    }
}

/// `offset_px` is signed along the left normal of a-to-b; zero draws the line on the segment.
pub(crate) fn linear(a: Pos2, b: Pos2, offset_px: f32) -> LinearDim {
    let along = unit(b - a, egui::vec2(1.0, 0.0));
    let normal = egui::vec2(-along.y, along.x);
    let shift = normal * offset_px;
    let overshoot = normal * offset_px.signum() * EXT_OVERSHOOT_PX;
    let dim_line = [a + shift, b + shift];
    LinearDim {
        ext: [[a, dim_line[0] + overshoot], [b, dim_line[1] + overshoot]],
        arrows: [arrow(dim_line[0], -along), arrow(dim_line[1], along)],
        label_center: dim_line[0].lerp(dim_line[1], 0.5),
        dim_line,
    }
}

pub(crate) fn radial(center: Pos2, on_arc: Pos2) -> RadialDim {
    let outward = unit(on_arc - center, egui::vec2(1.0, 0.0));
    RadialDim {
        leader: [center, on_arc],
        arrow: arrow(on_arc, outward),
        label_center: center.lerp(on_arc, 0.5),
    }
}

/// `start_dir` is a screen-space direction; a positive `sweep` turns toward +y on screen.
pub(crate) fn angular(center: Pos2, start_dir: Vec2, sweep: f32, radius_px: f32) -> AngularDim {
    let start = start_dir.y.atan2(start_dir.x);
    let at = |angle: f32| center + egui::vec2(angle.cos(), angle.sin()) * radius_px;
    let arc: Vec<Pos2> = (0..=ARC_STEPS)
        .map(|step| at(start + sweep * step as f32 / ARC_STEPS as f32))
        .collect();
    let sign = if sweep < 0.0 { -1.0 } else { 1.0 };
    let tangent = |angle: f32| egui::vec2(-angle.sin(), angle.cos()) * sign;
    AngularDim {
        arrows: [
            arrow(at(start), -tangent(start)),
            arrow(at(start + sweep), tangent(start + sweep)),
        ],
        label_center: at(start + sweep * 0.5),
        arc,
    }
}

/// Arrowheads and lines as drawable parts; empty strokes are skipped by the painter.
pub(crate) fn paint(painter: &egui::Painter, graphic: &Graphic, stroke: egui::Stroke) {
    let line = |points: &[Pos2]| {
        if points.len() >= 2 && points.first() != points.last() {
            painter.add(egui::Shape::line(points.to_vec(), stroke));
        }
    };
    let head = |arrow: &Arrow| {
        painter.add(egui::Shape::convex_polygon(
            arrow.to_vec(),
            stroke.color,
            egui::Stroke::NONE,
        ));
    };
    match graphic {
        Graphic::Linear(dim) => {
            dim.ext.iter().for_each(|ext| line(ext));
            line(&dim.dim_line);
            dim.arrows.iter().for_each(head);
        }
        Graphic::Radial(dim) => {
            line(&dim.leader);
            head(&dim.arrow);
        }
        Graphic::Angular(dim) => {
            line(&dim.arc);
            dim.arrows.iter().for_each(head);
        }
    }
}

/// Keeps a chip centre far enough inside `bounds` for the box to stay visible.
pub(crate) fn clamp_center(center: Pos2, bounds: Rect) -> Pos2 {
    let inner = bounds.shrink2(egui::vec2(32.0, 12.0));
    let inner = if inner.is_positive() { inner } else { bounds };
    inner.clamp(center)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_offsets_perpendicular_with_extensions_and_arrows() {
        let dim = linear(egui::pos2(0.0, 0.0), egui::pos2(100.0, 0.0), OFFSET_PX);
        assert_eq!(dim.dim_line[0], egui::pos2(0.0, OFFSET_PX));
        assert_eq!(dim.dim_line[1], egui::pos2(100.0, OFFSET_PX));
        assert_eq!(dim.label_center, egui::pos2(50.0, OFFSET_PX));
        assert_eq!(dim.ext[0][0], egui::pos2(0.0, 0.0));
        assert!(dim.ext[0][1].y > OFFSET_PX);
        assert_eq!(dim.arrows[0][0], dim.dim_line[0]);
        assert!((dim.arrows[0][1].x - ARROW_LEN).abs() < 1e-4);
        assert!((dim.arrows[1][1].x - (100.0 - ARROW_LEN)).abs() < 1e-4);
        assert!(((dim.arrows[0][1].y - dim.arrows[0][2].y).abs() - ARROW_WIDTH).abs() < 1e-4);
    }

    #[test]
    fn offset_flips_away_from_hint_and_degenerate_segments_stay_finite() {
        let (a, b) = (egui::pos2(0.0, 0.0), egui::pos2(100.0, 0.0));
        let below = offset_away(a, b, Some(egui::pos2(50.0, 40.0)));
        let above = offset_away(a, b, Some(egui::pos2(50.0, -40.0)));
        assert_eq!(below, -above);
        assert!(linear(a, b, below).dim_line[0].y < 0.0);
        let dot = linear(a, a, OFFSET_PX);
        assert!(dot.dim_line.iter().all(|point| point.is_finite()));
        assert!(dot.label_center.is_finite());
    }

    #[test]
    fn radial_leader_runs_center_to_arc_with_tip_on_arc() {
        let dim = radial(egui::pos2(10.0, 10.0), egui::pos2(60.0, 10.0));
        assert_eq!(dim.leader[1], dim.arrow[0]);
        assert_eq!(dim.label_center, egui::pos2(35.0, 10.0));
        assert!(dim.arrow[1].x < 60.0);
    }

    #[test]
    fn angular_arc_spans_sweep_with_label_at_midpoint() {
        let center = egui::pos2(0.0, 0.0);
        let dim = angular(
            center,
            egui::vec2(1.0, 0.0),
            std::f32::consts::FRAC_PI_2,
            40.0,
        );
        assert_eq!(dim.arc.len(), ARC_STEPS + 1);
        assert!((dim.arc[0] - egui::pos2(40.0, 0.0)).length() < 1e-3);
        assert!((dim.arc[ARC_STEPS] - egui::pos2(0.0, 40.0)).length() < 1e-3);
        let mid = std::f32::consts::FRAC_PI_4;
        assert!(
            (dim.label_center - egui::pos2(40.0 * mid.cos(), 40.0 * mid.sin())).length() < 1e-3
        );
        assert!(
            dim.arc
                .iter()
                .all(|point| ((*point - center).length() - 40.0).abs() < 1e-3)
        );
    }

    #[test]
    fn centre_clamp_keeps_boxes_inside_bounds() {
        let bounds = Rect::from_min_max(egui::pos2(100.0, 100.0), egui::pos2(400.0, 300.0));
        for point in [
            egui::pos2(0.0, 0.0),
            egui::pos2(900.0, 900.0),
            bounds.center(),
        ] {
            assert!(bounds.contains(clamp_center(point, bounds)));
        }
    }
}
