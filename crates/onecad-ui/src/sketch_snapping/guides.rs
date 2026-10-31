use super::loci::{Class, Locus, LocusId, Relation, extension_loci, resolve_loci};
use super::*;
use crate::sketch_snapping::curves::{Shape, point};
use onecad_scene::DVec2;

const POLAR_MIN_RADIUS: f32 = 24.0;
/// Matches the renderer, which fades minor grid lines once cells shrink below ~8 px.
const MIN_VISIBLE_GRID_CELL: f32 = 8.0;

fn visible_step(request: &SnapRequest<'_>, raw: Vec2, minor: f64) -> f64 {
    let cell = Vec2::new(raw.x + minor, raw.y)
        .and_then(|next| Some(project(request, raw)?.distance(project(request, next)?)));
    if cell.is_some_and(|cell| cell < MIN_VISIBLE_GRID_CELL) {
        minor * 10.0
    } else {
        minor
    }
}

pub(super) fn guides_and_grid(collector: &mut Collector<'_, '_>, nearby: &[Curve]) -> Option<()> {
    let request = collector.request;
    let logical = onecad_scene::DVec2::new(
        f64::from(request.pointer.x - request.rect.min.x),
        f64::from(request.pointer.y - request.rect.min.y),
    );
    let world = onecad_interaction::sketch::intersect_sketch_plane(
        request.plane,
        request.view.ray(logical).ok()?,
    )?;
    let raw = onecad_interaction::sketch::sketch_plane_coordinates(request.plane, world)?;
    let mut loci = Vec::new();
    if let Some(step) = request
        .settings
        .grid_step
        .filter(|step| step.is_finite() && *step > 0.0)
    {
        let step = visible_step(request, raw, step);
        let x = (raw.x / step).round() * step;
        let y = (raw.y / step).round() * step;
        collector.push(Vec2::new(x, y)?, SnapKind::Grid, SnapIntent::GeometryOnly)?;
        loci.extend(grid_loci(x, y));
    }
    if let Some((anchor, reference)) = request.anchor {
        alignment_loci(&mut loci, anchor, reference);
        loci.extend(polar_locus(request, raw, anchor));
    }
    for entity in request.sketch.entities() {
        let roles: &[CurvePosition] = match entity {
            SketchEntity::Point { .. } => &[CurvePosition::Arbitrary],
            SketchEntity::Arc { .. } => &[CurvePosition::Start, CurvePosition::End],
            _ => &[],
        };
        for role in roles {
            let reference = target(entity.id(), *role);
            let at = reference.point_coordinates(request.sketch)?;
            if project(request, at).is_none_or(|screen| !request.rect.contains(screen)) {
                continue;
            }
            alignment_loci(&mut loci, at, Some(reference));
        }
    }
    extension_loci(request, &mut loci);
    loci.extend(nearby.iter().map(|curve| Locus {
        id: LocusId::Curve(curve.id),
        class: Class::Curve,
        shape: curve.shape,
        relation: Relation::OnCurve(curve.id),
        guide: None,
    }));
    resolve_loci(collector, raw, &loci)
}

fn infinite(origin: Vec2, dir: DVec2) -> Shape {
    Shape::Line {
        origin: vector(origin),
        dir,
        range: (f64::NEG_INFINITY, f64::INFINITY),
    }
}

fn grid_loci(x: f64, y: f64) -> Vec<Locus> {
    let lines = [
        (LocusId::GridX(x.to_bits()), Vec2::new(x, 0.0), DVec2::Y),
        (LocusId::GridY(y.to_bits()), Vec2::new(0.0, y), DVec2::X),
    ];
    lines
        .into_iter()
        .filter_map(|(id, origin, dir)| {
            Some(Locus {
                id,
                class: Class::Grid,
                shape: infinite(origin?, dir),
                relation: Relation::None,
                guide: Some(SnapGuide {
                    kind: GuideKind::Grid,
                    shape: GuideShape::Along(point(dir)?),
                    persisted: false,
                }),
            })
        })
        .collect()
}

fn alignment_loci(loci: &mut Vec<Locus>, at: Vec2, reference: Option<SketchTarget>) {
    for (alignment, kind, dir) in [
        (PointAlignment::Horizontal, GuideKind::Horizontal, DVec2::X),
        (PointAlignment::Vertical, GuideKind::Vertical, DVec2::Y),
    ] {
        loci.push(Locus {
            id: LocusId::Guide {
                reference,
                alignment,
            },
            class: Class::Guide(alignment),
            shape: infinite(at, dir),
            relation: reference.map_or(Relation::None, |reference| Relation::Align {
                reference,
                alignment,
            }),
            guide: Some(SnapGuide {
                kind,
                shape: GuideShape::From(at),
                persisted: reference.is_some(),
            }),
        });
    }
}

/// Polar steps on the axes are left to the anchor's horizontal and vertical guides.
fn polar_locus(request: &SnapRequest<'_>, raw: Vec2, anchor: Vec2) -> Option<Locus> {
    let step = request
        .settings
        .polar_step_degrees
        .filter(|step| step.is_finite() && (1.0..=180.0).contains(step))?;
    // Near the anchor every direction lies within the aperture of some polar step.
    if let (Some(from), Some(to)) = (project(request, anchor), project(request, raw))
        && from.distance(to) < POLAR_MIN_RADIUS
    {
        return None;
    }
    let delta = vector(raw) - vector(anchor);
    let index = (delta.y.atan2(delta.x).to_degrees() / step).round();
    let degrees = (index * step).rem_euclid(360.0);
    let off_axis = degrees.rem_euclid(90.0);
    if off_axis < 1e-6 || 90.0 - off_axis < 1e-6 {
        return None;
    }
    let radians = (index * step).to_radians();
    Some(Locus {
        id: LocusId::Polar(index as i64),
        class: Class::Polar,
        shape: Shape::Line {
            origin: vector(anchor),
            dir: DVec2::new(radians.cos(), radians.sin()),
            range: (0.0, f64::INFINITY),
        },
        relation: Relation::None,
        guide: Some(SnapGuide {
            kind: GuideKind::Polar { degrees },
            shape: GuideShape::From(anchor),
            persisted: false,
        }),
    })
}
