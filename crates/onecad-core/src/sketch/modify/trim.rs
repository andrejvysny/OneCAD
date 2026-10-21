//! Trim: where the target is cut, and which spans survive.

use std::f64::consts::{PI, TAU};

use super::common::{crossings, modifiable, representative, Crossing};
use super::geom::{angle_of, arc_param, curve_of, line_param, Curve, Pt};
use super::trim_emit::build;
use super::{ModifyIds, ModifyPlan, ModifyRefusal, TrimRequest, COINCIDENT_MM, MIN_PIECE_MM};
use crate::ids::EntityId;
use crate::sketch::{Sketch, SketchEntity};

/// Plans a trim of `request.entity` at `request.pick`.
///
/// # Errors
/// A [`ModifyRefusal`] when the target is missing, a point, locked, an
/// ellipse, degenerate, the pick sits on a crossing, or a surviving piece would
/// be shorter than [`MIN_PIECE_MM`]. A refusal draws no id from `ids`.
pub fn plan_trim(
    sketch: &Sketch,
    request: &TrimRequest,
    ids: &mut dyn ModifyIds,
) -> Result<ModifyPlan, ModifyRefusal> {
    let entity = modifiable(sketch, request.entity)?;
    let target = Target::resolve(sketch, entity)?;
    let cut = target.cut(sketch, request.entity, request.pick.into())?;
    Ok(build(sketch, entity, &target, &cut, ids))
}

/// The trim target with its defining point ids.
#[derive(Debug, Clone, Copy)]
pub(super) enum Target {
    Line {
        start: EntityId,
        end: EntityId,
        a: Pt,
        b: Pt,
    },
    Arc {
        center: EntityId,
        c: Pt,
        r: f64,
        start: f64,
        sweep: f64,
    },
    Circle {
        center: EntityId,
        c: Pt,
        r: f64,
    },
}

/// One end of a surviving span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SpanEnd {
    /// The target's original start/end.
    Original,
    /// A cut at `Cut::crossings[i]`.
    Cut(usize),
}

/// A surviving span in the target's param space (a circle's may run past 2π).
#[derive(Debug, Clone, Copy)]
pub(super) struct Span {
    pub from: f64,
    pub to: f64,
    pub start: SpanEnd,
    pub end: SpanEnd,
}

/// Where the target is cut: deduplicated crossings and the surviving spans
/// (empty ⇒ the whole entity goes).
#[derive(Debug)]
pub(super) struct Cut {
    pub crossings: Vec<Crossing>,
    pub spans: Vec<Span>,
}

impl Target {
    fn resolve(sketch: &Sketch, entity: &SketchEntity) -> Result<Self, ModifyRefusal> {
        let id = entity.id();
        let curve = curve_of(sketch, entity).ok_or(ModifyRefusal::DegenerateTarget(id))?;
        let target = match (curve, entity) {
            (Curve::Line { a, b }, SketchEntity::Line { start, end, .. }) => Self::Line {
                start: *start,
                end: *end,
                a,
                b,
            },
            (Curve::Arc { c, r, start, sweep }, SketchEntity::Arc { center, .. }) => Self::Arc {
                center: *center,
                c,
                r,
                start,
                sweep,
            },
            (Curve::Circle { c, r }, SketchEntity::Circle { center, .. }) => Self::Circle {
                center: *center,
                c,
                r,
            },
            _ => {
                return Err(ModifyRefusal::UnsupportedKind {
                    entity: id,
                    kind: "ellipse",
                    action: "trimmed",
                })
            }
        };
        if target.size() * target.extent().unwrap_or(TAU) < MIN_PIECE_MM {
            return Err(ModifyRefusal::DegenerateTarget(id));
        }
        Ok(target)
    }

    pub(super) fn curve(&self) -> Curve {
        match *self {
            Self::Line { a, b, .. } => Curve::Line { a, b },
            Self::Arc {
                c, r, start, sweep, ..
            } => Curve::Arc { c, r, start, sweep },
            Self::Circle { c, r, .. } => Curve::Circle { c, r },
        }
    }

    /// Millimetres per unit of param.
    fn size(&self) -> f64 {
        match *self {
            Self::Line { a, b, .. } => a.dist(b),
            Self::Arc { r, .. } | Self::Circle { r, .. } => r,
        }
    }

    /// Param length of an open curve; `None` for the circle's ring.
    fn extent(&self) -> Option<f64> {
        match *self {
            Self::Line { .. } => Some(1.0),
            Self::Arc { sweep, .. } => Some(sweep),
            Self::Circle { .. } => None,
        }
    }

    /// The param space is angular (a pick past the end may be nearer the start).
    fn wraps(&self) -> bool {
        !matches!(self, Self::Line { .. })
    }

    /// [`COINCIDENT_MM`] in param units.
    pub(super) fn tol(&self) -> f64 {
        COINCIDENT_MM / self.size()
    }

    pub(super) fn param(&self, p: Pt) -> f64 {
        match *self {
            Self::Line { a, b, .. } => line_param(a, b, p),
            Self::Arc { c, r, start, .. } => arc_param(c, r, start, p),
            Self::Circle { c, .. } => angle_of(c, p),
        }
    }

    fn cut(&self, sketch: &Sketch, id: EntityId, pick: Pt) -> Result<Cut, ModifyRefusal> {
        let (tol, min) = (self.tol(), MIN_PIECE_MM / self.size());
        let raw = crossings(sketch, id, &self.curve(), true, &|p| self.param(p));
        let pick = self.param(pick);
        match self.extent() {
            Some(len) => {
                let inner = raw
                    .into_iter()
                    .filter(|c| c.param > tol && c.param < len - tol)
                    .collect();
                let crossings = cluster(inner, tol);
                let spans = open_spans(&crossings, len, tol, clamp_pick(pick, len, self.wraps()))
                    .ok_or(ModifyRefusal::AmbiguousPick(id))?;
                check_spans(&spans, min, id)?;
                Ok(Cut { crossings, spans })
            }
            None => {
                let crossings = wrap_merge(cluster(raw, tol), tol);
                let spans =
                    ring_spans(&crossings, tol, pick).ok_or(ModifyRefusal::AmbiguousPick(id))?;
                check_spans(&spans, min, id)?;
                Ok(Cut { crossings, spans })
            }
        }
    }
}

/// A pick off an open curve's extent selects the nearer end; on an arc
/// (`wrap`) "nearer" is measured around the circle.
fn clamp_pick(pick: f64, len: f64, wrap: bool) -> f64 {
    if pick <= len || !wrap {
        return pick.clamp(0.0, len);
    }
    if pick - len < TAU - pick {
        len
    } else {
        0.0
    }
}

fn check_spans(spans: &[Span], min: f64, id: EntityId) -> Result<(), ModifyRefusal> {
    if spans.iter().any(|s| s.to - s.from < min) {
        return Err(ModifyRefusal::DegeneratePiece(id));
    }
    Ok(())
}

/// Sorts by param and collapses crossings within `tol` to one representative.
fn cluster(mut raw: Vec<Crossing>, tol: f64) -> Vec<Crossing> {
    raw.sort_by(|a, b| a.param.total_cmp(&b.param));
    let mut out = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        let j = (i..raw.len())
            .find(|&j| raw[j].param - raw[i].param > tol)
            .unwrap_or(raw.len());
        out.push(representative(raw[i..j].iter()));
        i = j;
    }
    out
}

/// On a ring, the last crossing may be the first one seen across 2π.
fn wrap_merge(mut ring: Vec<Crossing>, tol: f64) -> Vec<Crossing> {
    if ring.len() >= 2 && ring[0].param + TAU - ring[ring.len() - 1].param <= tol {
        let last = ring.pop().expect("len >= 2");
        let mut rep = representative([last, ring[0]].iter());
        if rep.param > PI {
            rep.param -= TAU;
        }
        ring[0] = rep;
    }
    ring
}

/// Surviving spans of a line/arc of param length `len`; `None` when the pick
/// sits on a crossing.
fn open_spans(crossings: &[Crossing], len: f64, tol: f64, pick: f64) -> Option<Vec<Span>> {
    if crossings.iter().any(|c| (c.param - pick).abs() <= tol) {
        return None;
    }
    let mut spans = Vec::new();
    if let Some(i) = crossings.iter().rposition(|c| c.param < pick) {
        spans.push(Span {
            from: 0.0,
            to: crossings[i].param,
            start: SpanEnd::Original,
            end: SpanEnd::Cut(i),
        });
    }
    if let Some(j) = crossings.iter().position(|c| c.param > pick) {
        spans.push(Span {
            from: crossings[j].param,
            to: len,
            start: SpanEnd::Cut(j),
            end: SpanEnd::Original,
        });
    }
    Some(spans)
}

/// The arc that survives removing the ring interval under `pick`; empty with
/// fewer than two crossings (the circle goes whole).
fn ring_spans(crossings: &[Crossing], tol: f64, pick: f64) -> Option<Vec<Span>> {
    let n = crossings.len();
    if n < 2 {
        return Some(Vec::new());
    }
    let ring_dist = |a: f64| {
        let d = (a - pick).rem_euclid(TAU);
        d.min(TAU - d)
    };
    if crossings.iter().any(|c| ring_dist(c.param) <= tol) {
        return None;
    }
    let first = crossings[0].param;
    let lifted = if pick < first { pick + TAU } else { pick };
    let i = crossings
        .iter()
        .rposition(|c| c.param < lifted)
        .unwrap_or(n - 1);
    let next = (i + 1) % n;
    let hi = crossings[next].param + if next == 0 { TAU } else { 0.0 };
    let mut from = hi;
    let mut to = crossings[i].param + TAU;
    if from >= TAU {
        from -= TAU;
        to -= TAU;
    }
    Some(vec![Span {
        from,
        to,
        start: SpanEnd::Cut(next),
        end: SpanEnd::Cut(i),
    }])
}
