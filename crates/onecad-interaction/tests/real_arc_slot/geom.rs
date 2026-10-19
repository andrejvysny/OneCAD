//! Independent geometric residuals read from the worker-solved sketch.
//! Nothing here consults solver diagnostics; it recomputes from entity data.
use onecad_core::{
    ids::EntityId,
    sketch::{Constraint, CurvePosition, Sketch, SketchEntity},
};
use std::collections::BTreeSet;

pub type P = [f64; 2];

pub fn dist(a: P, b: P) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

pub fn pt(sketch: &Sketch, id: EntityId) -> P {
    match sketch.get_entity(id) {
        Some(SketchEntity::Point { at, .. }) => [at.x, at.y],
        other => panic!("point expected, got {other:?}"),
    }
}

/// Center, radius, start angle, end angle.
pub fn arc(sketch: &Sketch, id: EntityId) -> (P, f64, f64, f64) {
    match sketch.get_entity(id) {
        Some(SketchEntity::Arc {
            center,
            radius,
            start_angle,
            end_angle,
            ..
        }) => (pt(sketch, *center), *radius, *start_angle, *end_angle),
        other => panic!("arc expected, got {other:?}"),
    }
}

pub fn line(sketch: &Sketch, id: EntityId) -> (P, P) {
    match sketch.get_entity(id) {
        Some(SketchEntity::Line { start, end, .. }) => (pt(sketch, *start), pt(sketch, *end)),
        other => panic!("line expected, got {other:?}"),
    }
}

fn angle_at(a: (P, f64, f64, f64), role: CurvePosition) -> f64 {
    if role == CurvePosition::Start {
        a.2
    } else {
        a.3
    }
}

/// World position of `role` on an entity, recomputed from raw parameters.
pub fn endpoint(sketch: &Sketch, id: EntityId, role: CurvePosition) -> P {
    match sketch.get_entity(id) {
        Some(SketchEntity::Point { .. }) => pt(sketch, id),
        Some(SketchEntity::Arc { .. }) => {
            let a = arc(sketch, id);
            let t = angle_at(a, role);
            [a.0[0] + a.1 * t.cos(), a.0[1] + a.1 * t.sin()]
        }
        Some(SketchEntity::Line { .. }) => {
            let (s, e) = line(sketch, id);
            if role == CurvePosition::Start {
                s
            } else {
                e
            }
        }
        other => panic!("unsupported {other:?}"),
    }
}

/// Unit direction of travel: line start->end, or CCW arc tangent at `role`.
fn direction(sketch: &Sketch, id: EntityId, role: CurvePosition) -> P {
    if let Some(SketchEntity::Arc { .. }) = sketch.get_entity(id) {
        let t = angle_at(arc(sketch, id), role);
        return [-t.sin(), t.cos()];
    }
    let (s, e) = line(sketch, id);
    let len = dist(s, e);
    [(e[0] - s[0]) / len, (e[1] - s[1]) / len]
}

/// Every Coincident residual <= 1e-6; returns how many were checked.
pub fn welds(sketch: &Sketch) -> usize {
    let mut count = 0;
    for c in sketch.constraints() {
        if let Constraint::Coincident {
            point1,
            point1_position,
            point2,
            point2_position,
            ..
        } = c
        {
            let gap = dist(
                endpoint(sketch, *point1, *point1_position),
                endpoint(sketch, *point2, *point2_position),
            );
            assert!(gap <= 1e-6, "weld {c:?} gap {gap}");
            count += 1;
        }
    }
    count
}

/// Every TangentAtPoint: shared endpoint and |sin| <= 1e-6.
/// Returns (dot, cross) of the two travel directions per constraint.
pub fn tangents(sketch: &Sketch) -> Vec<(f64, f64)> {
    let mut found = Vec::new();
    for c in sketch.constraints() {
        if let Constraint::TangentAtPoint {
            entity1,
            entity1_position: p1,
            entity2,
            entity2_position: p2,
            ..
        } = c
        {
            let gap = dist(
                endpoint(sketch, *entity1, *p1),
                endpoint(sketch, *entity2, *p2),
            );
            assert!(gap <= 1e-6, "tangent point gap {gap}");
            let (a, b) = (
                direction(sketch, *entity1, *p1),
                direction(sketch, *entity2, *p2),
            );
            let cross = a[0] * b[1] - a[1] * b[0];
            assert!(cross.abs() <= 1e-6, "tangent {c:?} sin {cross}");
            found.push((a[0] * b[0] + a[1] * b[1], cross));
        }
    }
    found
}

/// Every point is a line end or an arc center.
pub fn assert_no_orphans(sketch: &Sketch) {
    let mut used = BTreeSet::new();
    for e in sketch.entities() {
        match e {
            SketchEntity::Line { start, end, .. } => used.extend([*start, *end]),
            SketchEntity::Arc { center, .. } => {
                used.insert(*center);
            }
            _ => {}
        }
    }
    for e in sketch.entities() {
        if let SketchEntity::Point { id, .. } = e {
            assert!(used.contains(id), "orphan point {id:?}");
        }
    }
}

/// Distance from `p` to the infinite line through `a`,`b`.
pub fn line_gap(a: P, b: P, p: P) -> f64 {
    let len = dist(a, b);
    ((b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])).abs() / len
}

/// Structural JSON equality with numeric tolerance.
pub fn json_close(a: &serde_json::Value, b: &serde_json::Value, eps: f64) -> bool {
    use serde_json::Value::*;
    match (a, b) {
        (Number(x), Number(y)) => (x.as_f64().unwrap() - y.as_f64().unwrap()).abs() <= eps,
        (Array(x), Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| json_close(p, q, eps))
        }
        (Object(x), Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, v)| y.get(k).is_some_and(|w| json_close(v, w, eps)))
        }
        _ => a == b,
    }
}
