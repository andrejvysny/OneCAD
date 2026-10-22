//! Maps the live draft to dimension graphics; each creation field owns one callout.
use super::*;
use crate::dimension_chip::annotation::{self, Graphic};
use egui::Pos2;
use onecad_interaction::sketch::{DraftPrimitive, arc_geometry::AnalyticArc};

pub(super) struct Callout {
    pub center: Pos2,
    pub graphic: Option<Graphic>,
}

const LABEL_LIFT_PX: f32 = 14.0;

pub(super) fn build(app: &App, ctx: &Context, draft: &Draft, bounds: egui::Rect) -> Vec<Callout> {
    let mut graphics = graphics(app, ctx, draft);
    graphics.resize(draft.fields.len(), None);
    let base = project(app, ctx, draft.placement_anchor).unwrap_or(bounds.center());
    graphics
        .into_iter()
        .enumerate()
        .map(|(index, graphic)| {
            let center = graphic.as_ref().map_or_else(
                || base + egui::vec2(32.0, -14.0 + 28.0 * index as f32),
                Graphic::label_center,
            );
            Callout {
                center: annotation::clamp_center(center, bounds),
                graphic,
            }
        })
        .collect()
}

fn graphics(app: &App, ctx: &Context, draft: &Draft) -> Vec<Option<Graphic>> {
    let Some(session) = app.doc.sketch.session.as_ref() else {
        return Vec::new();
    };
    let Some(live) = session.draft.as_ref() else {
        return Vec::new();
    };
    let proj = |point: Vec2| project(app, ctx, point);
    let preview = session.preview.as_ref();
    let secondary = live.secondary.as_ref().map(|anchor| anchor.point);
    if draft.method == NumericCreationMethod::TangentArc {
        return vec![arc_of(preview, None).and_then(|arc| radial(&proj, &arc, 0.5))];
    }
    match draft.tool {
        SketchTool::Line => {
            let (a, b) = match preview {
                Some(DraftPrimitive::Line { start, end }) => (*start, *end),
                _ => (live.anchor, live.cursor),
            };
            vec![linear(proj(a), proj(b), None)]
        }
        SketchTool::Rectangle | SketchTool::CenterRectangle => {
            let (first, opposite) = match preview {
                Some(DraftPrimitive::Rectangle { first, opposite }) => (*first, *opposite),
                _ if draft.tool == SketchTool::CenterRectangle => (
                    Vec2::new_unchecked(
                        2.0 * live.anchor.x - live.cursor.x,
                        2.0 * live.anchor.y - live.cursor.y,
                    ),
                    live.cursor,
                ),
                _ => (live.anchor, live.cursor),
            };
            rectangle(&proj, first, opposite)
        }
        SketchTool::Circle => {
            let (center, radius) = match preview {
                Some(DraftPrimitive::Circle { center, radius }) => (*center, *radius),
                _ => (
                    live.anchor,
                    (live.cursor.x - live.anchor.x).hypot(live.cursor.y - live.anchor.y),
                ),
            };
            let at = |dx: f64| Vec2::new_unchecked(center.x + dx, center.y);
            vec![across(proj(at(radius)), proj(at(-radius)))]
        }
        SketchTool::Arc => {
            let Some(arc) = arc_of(
                preview,
                secondary.map(|start| (live.anchor, start, live.cursor)),
            ) else {
                return vec![None, None];
            };
            vec![radial(&proj, &arc, 0.0), angular(&proj, &arc)]
        }
        SketchTool::ThreePointArc => {
            let arc = arc_of(preview, None).or_else(|| {
                let second = secondary?;
                onecad_interaction::sketch::arc_geometry::through_points(
                    live.anchor,
                    second,
                    live.cursor,
                )
                .ok()
            });
            vec![arc.and_then(|arc| radial(&proj, &arc, 0.5))]
        }
        SketchTool::Slot => match preview {
            Some(DraftPrimitive::Slot {
                center_start,
                center_end,
                radius,
            }) => {
                let (dx, dy) = (center_end.x - center_start.x, center_end.y - center_start.y);
                let length = dx.hypot(dy);
                if length <= 0.0 {
                    return vec![None];
                }
                let (nx, ny) = (-dy / length * radius, dx / length * radius);
                let at = |sign: f64| {
                    Vec2::new_unchecked(center_start.x + sign * nx, center_start.y + sign * ny)
                };
                vec![across(proj(at(1.0)), proj(at(-1.0)))]
            }
            _ => vec![None],
        },
        SketchTool::Point | SketchTool::Select | SketchTool::PenStroke => Vec::new(),
    }
}

fn arc_of(
    preview: Option<&DraftPrimitive>,
    fallback: Option<(Vec2, Vec2, Vec2)>,
) -> Option<AnalyticArc> {
    if let Some(DraftPrimitive::Arc { arc }) = preview {
        return Some(*arc);
    }
    let (center, start, end) = fallback?;
    onecad_interaction::sketch::arc_geometry::center_start_end(center, start, end).ok()
}

fn on_arc(arc: &AnalyticArc, fraction: f64) -> Vec2 {
    let angle = arc.start_angle + arc.sweep * fraction;
    Vec2::new_unchecked(
        arc.center.x + arc.radius * angle.cos(),
        arc.center.y + arc.radius * angle.sin(),
    )
}

fn linear(a: Option<Pos2>, b: Option<Pos2>, hint: Option<Pos2>) -> Option<Graphic> {
    let (a, b) = (a?, b?);
    Some(Graphic::Linear(annotation::linear(
        a,
        b,
        annotation::offset_away(a, b, hint),
    )))
}

/// Dimension line drawn on the measured segment with the box lifted clear of it.
fn across(a: Option<Pos2>, b: Option<Pos2>) -> Option<Graphic> {
    let (a, b) = (a?, b?);
    let mut dim = annotation::linear(a, b, 0.0);
    let along = (b - a).normalized();
    let lift = if along.is_finite() {
        egui::vec2(-along.y, along.x) * LABEL_LIFT_PX
    } else {
        egui::vec2(0.0, -LABEL_LIFT_PX)
    };
    dim.label_center += lift;
    Some(Graphic::Linear(dim))
}

fn rectangle(
    proj: &impl Fn(Vec2) -> Option<Pos2>,
    first: Vec2,
    opposite: Vec2,
) -> Vec<Option<Graphic>> {
    let (x0, x1) = (first.x.min(opposite.x), first.x.max(opposite.x));
    let (y0, y1) = (first.y.min(opposite.y), first.y.max(opposite.y));
    let at = |x: f64, y: f64| proj(Vec2::new_unchecked(x, y));
    let hint = at((x0 + x1) * 0.5, (y0 + y1) * 0.5);
    vec![
        linear(at(x0, y0), at(x1, y0), hint),
        linear(at(x1, y0), at(x1, y1), hint),
    ]
}

fn radial(
    proj: &impl Fn(Vec2) -> Option<Pos2>,
    arc: &AnalyticArc,
    fraction: f64,
) -> Option<Graphic> {
    Some(Graphic::Radial(annotation::radial(
        proj(arc.center)?,
        proj(on_arc(arc, fraction))?,
    )))
}

fn angular(proj: &impl Fn(Vec2) -> Option<Pos2>, arc: &AnalyticArc) -> Option<Graphic> {
    let center = proj(arc.center)?;
    let (start, mid, end) = (
        proj(on_arc(arc, 0.0))? - center,
        proj(on_arc(arc, 0.5))? - center,
        proj(on_arc(arc, 1.0))? - center,
    );
    let ccw = (end.angle() - start.angle()).rem_euclid(std::f32::consts::TAU);
    let mid_turn = (mid.angle() - start.angle()).rem_euclid(std::f32::consts::TAU);
    let sweep = if mid_turn <= ccw {
        ccw
    } else {
        ccw - std::f32::consts::TAU
    };
    let radius = (start.length() * 0.6).clamp(20.0, 60.0);
    Some(Graphic::Angular(annotation::angular(
        center, start, sweep, radius,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(point: Vec2) -> Option<Pos2> {
        Some(egui::pos2(point.x as f32, point.y as f32))
    }

    #[test]
    fn rectangle_dimensions_sit_outside_on_bottom_and_side_edges() {
        let corner = |x, y| Vec2::new_unchecked(x, y);
        let graphics = rectangle(&flat, corner(0.0, 0.0), corner(100.0, 60.0));
        let [Some(Graphic::Linear(width)), Some(Graphic::Linear(height))] = &graphics[..] else {
            panic!("rectangle needs width and height dimensions");
        };
        let centre = egui::pos2(50.0, 30.0);
        assert!((width.label_center - centre).length() > 30.0);
        assert!((width.label_center.x - 50.0).abs() < 1e-3);
        assert!((height.label_center.y - 30.0).abs() < 1e-3);
        assert!(height.label_center.x > 100.0);
    }

    #[test]
    fn angular_sweep_follows_the_projected_mid_ray() {
        let arc = AnalyticArc {
            center: Vec2::new_unchecked(0.0, 0.0),
            radius: 50.0,
            start_angle: 0.0,
            end_angle: std::f64::consts::FRAC_PI_2,
            sweep: std::f64::consts::FRAC_PI_2,
            clockwise: false,
            travel_start_role: onecad_core::sketch::CurvePosition::Start,
            travel_end_role: onecad_core::sketch::CurvePosition::End,
            exit_tangent: Vec2::new_unchecked(0.0, 1.0),
        };
        let Some(Graphic::Angular(dim)) = angular(&flat, &arc) else {
            panic!("arc needs an angular dimension");
        };
        let mid = std::f32::consts::FRAC_PI_4;
        let direction = (dim.label_center - egui::pos2(0.0, 0.0)).normalized();
        assert!((direction - egui::vec2(mid.cos(), mid.sin())).length() < 1e-3);
    }
}
