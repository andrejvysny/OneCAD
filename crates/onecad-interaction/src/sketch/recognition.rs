//! Pencil stroke recognition (IP11, ONECAD-458): confirmed samples (actual and
//! coalesced, never predicted) projected to the sketch plane become a Line, a
//! three-point Arc, or nothing. Fit residuals are judged relative to the stroke
//! length; a movement threshold ignores tiny strokes; closure and sweep checks
//! reject loops; hysteresis keeps the live Line/Arc decision from flickering.
use super::arc_geometry;
use onecad_core::math::Vec2;

/// Bend (largest chord deviation / chord) at or below which a stroke is a line.
pub const LINE_BEND: f64 = 0.035;
/// Bend at or above which a stroke is an arc candidate; between the two the
/// previous decision holds.
pub const ARC_BEND: f64 = 0.07;
/// A line's chord must cover this share of its travel (no back-and-forth).
const LINE_CHORD_RATIO: f64 = 0.85;
/// RMS / largest fit residual relative to stroke length.
const RMS_RESIDUAL: f64 = 0.025;
const MAX_RESIDUAL: f64 = 0.07;
/// Allowed travel against the sweep direction, relative to the sweep.
const REVERSAL: f64 = 0.08;
const MIN_SWEEP: f64 = 15.0 * std::f64::consts::PI / 180.0;
const MAX_SWEEP: f64 = 330.0 * std::f64::consts::PI / 180.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrokeKind {
    Line,
    Arc,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StrokeShape {
    Line {
        start: Vec2,
        end: Vec2,
    },
    /// Three-point arc from `start` through `through` to `end`.
    Arc {
        start: Vec2,
        through: Vec2,
        end: Vec2,
    },
}

impl StrokeShape {
    pub fn kind(&self) -> StrokeKind {
        match self {
            Self::Line { .. } => StrokeKind::Line,
            Self::Arc { .. } => StrokeKind::Arc,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Recognition {
    /// Shorter than the movement threshold: a stray touch, silently ignored.
    TooShort,
    Unrecognized(&'static str),
    Shape(StrokeShape),
}

/// Accumulates one stroke. `min_travel` is the movement threshold in sketch
/// units (hosts derive it from points at the current zoom).
#[derive(Clone, Debug)]
pub struct StrokeRecognizer {
    points: Vec<Vec2>,
    travel: f64,
    min_travel: f64,
    decision: Option<StrokeKind>,
}

impl StrokeRecognizer {
    pub fn new(min_travel: f64) -> Self {
        Self {
            points: Vec::new(),
            travel: 0.0,
            min_travel: if min_travel.is_finite() {
                min_travel.max(0.0)
            } else {
                0.0
            },
            decision: None,
        }
    }

    pub fn points(&self) -> &[Vec2] {
        &self.points
    }

    /// Adds a confirmed sample; returns the live decision once the stroke has
    /// moved far enough. Non-finite and repeated points are skipped.
    pub fn push(&mut self, point: Vec2) -> Option<StrokeKind> {
        if !point.is_finite() {
            return self.decision;
        }
        if let Some(last) = self.points.last() {
            let step = distance(*last, point);
            if step == 0.0 {
                return self.decision;
            }
            self.travel += step;
        }
        self.points.push(point);
        if self.travel >= self.min_travel && self.points.len() >= 2 {
            self.decision = Some(decide(bend(&self.points), self.decision));
        }
        self.decision
    }

    pub fn finish(&self) -> Recognition {
        if self.points.len() < 2 || self.travel < self.min_travel || self.travel == 0.0 {
            return Recognition::TooShort;
        }
        let kind = decide(bend(&self.points), self.decision);
        match self.shape_as(kind) {
            Ok(shape) => Recognition::Shape(shape),
            Err(reason) => Recognition::Unrecognized(reason),
        }
    }

    /// The stroke fitted as `kind`: the recognized result, or an explicit
    /// override of it. Fails when the samples do not support that shape.
    pub fn shape_as(&self, kind: StrokeKind) -> Result<StrokeShape, &'static str> {
        let (Some(&start), Some(&end)) = (self.points.first(), self.points.last()) else {
            return Err("The stroke has no samples");
        };
        if distance(start, end) < self.travel * 0.1 {
            return Err("Closed strokes are neither lines nor arcs");
        }
        match kind {
            StrokeKind::Line => self.line(start, end),
            StrokeKind::Arc => self.arc(start, end),
        }
    }

    /// The bend thresholds already bound how far a recognized line strays; an
    /// explicit override only needs one direction of travel.
    fn line(&self, start: Vec2, end: Vec2) -> Result<StrokeShape, &'static str> {
        if distance(start, end) < LINE_CHORD_RATIO * self.travel {
            return Err("The stroke doubles back; draw one straight line");
        }
        Ok(StrokeShape::Line { start, end })
    }

    fn arc(&self, start: Vec2, end: Vec2) -> Result<StrokeShape, &'static str> {
        let (center, radius) =
            fit_circle(&self.points).ok_or("The stroke is too straight for an arc")?;
        let residuals: Vec<f64> = self
            .points
            .iter()
            .map(|p| (distance(*p, center) - radius).abs())
            .collect();
        self.residuals_fit(&residuals)?;
        let (sweep, against) = sweep(&self.points, center);
        if !(MIN_SWEEP..=MAX_SWEEP).contains(&sweep.abs()) {
            return Err("The arc sweep is too small or closes on itself");
        }
        if against > REVERSAL * sweep.abs() {
            return Err("The stroke does not follow one arc");
        }
        let start_angle = angle(start, center);
        let mid = start_angle + sweep / 2.0;
        let through =
            Vec2::new_unchecked(center.x + radius * mid.cos(), center.y + radius * mid.sin());
        arc_geometry::through_points(start, end, through).map_err(|_| "The arc is degenerate")?;
        Ok(StrokeShape::Arc {
            start,
            through,
            end,
        })
    }

    fn residuals_fit(&self, residuals: &[f64]) -> Result<(), &'static str> {
        let rms = (residuals.iter().map(|r| r * r).sum::<f64>() / residuals.len() as f64).sqrt();
        let max = residuals.iter().copied().fold(0.0, f64::max);
        if rms > RMS_RESIDUAL * self.travel || max > MAX_RESIDUAL * self.travel {
            return Err("The stroke is too irregular to recognize");
        }
        Ok(())
    }
}

/// Hysteresis: a clear bend decides; the band between keeps the last decision.
fn decide(bend: f64, previous: Option<StrokeKind>) -> StrokeKind {
    if bend <= LINE_BEND {
        StrokeKind::Line
    } else if bend >= ARC_BEND {
        StrokeKind::Arc
    } else {
        previous.unwrap_or(StrokeKind::Line)
    }
}

/// Largest distance from the chord, relative to the chord length.
fn bend(points: &[Vec2]) -> f64 {
    let (Some(&start), Some(&end)) = (points.first(), points.last()) else {
        return 0.0;
    };
    let chord = distance(start, end);
    if chord == 0.0 {
        return f64::INFINITY;
    }
    points
        .iter()
        .map(|p| deviation(*p, start, end))
        .fold(0.0, f64::max)
        / chord
}

fn distance(a: Vec2, b: Vec2) -> f64 {
    (a.x - b.x).hypot(a.y - b.y)
}

fn deviation(p: Vec2, a: Vec2, b: Vec2) -> f64 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let length = dx.hypot(dy);
    if length == 0.0 {
        return distance(p, a);
    }
    ((p.x - a.x) * dy - (p.y - a.y) * dx).abs() / length
}

fn angle(p: Vec2, center: Vec2) -> f64 {
    (p.y - center.y).atan2(p.x - center.x)
}

/// Signed total sweep and the travel against its direction (radians).
fn sweep(points: &[Vec2], center: Vec2) -> (f64, f64) {
    let steps: Vec<f64> = points
        .windows(2)
        .map(|pair| {
            let step = angle(pair[1], center) - angle(pair[0], center);
            (step + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI
        })
        .collect();
    let total: f64 = steps.iter().sum();
    let against = steps
        .iter()
        .filter(|step| step.signum() != total.signum())
        .map(|step| step.abs())
        .sum();
    (total, against)
}

/// Algebraic (Kåsa) least-squares circle in centroid coordinates.
fn fit_circle(points: &[Vec2]) -> Option<(Vec2, f64)> {
    let n = points.len() as f64;
    if points.len() < 3 {
        return None;
    }
    let cx = points.iter().map(|p| p.x).sum::<f64>() / n;
    let cy = points.iter().map(|p| p.y).sum::<f64>() / n;
    let (mut suu, mut svv, mut suv, mut suuu, mut svvv, mut suvv, mut svuu) =
        (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    for p in points {
        let (u, v) = (p.x - cx, p.y - cy);
        suu += u * u;
        svv += v * v;
        suv += u * v;
        suuu += u * u * u;
        svvv += v * v * v;
        suvv += u * v * v;
        svuu += v * u * u;
    }
    let det = suu * svv - suv * suv;
    let scale = (suu + svv).powi(2);
    if scale == 0.0 || det.abs() <= 1e-12 * scale {
        return None;
    }
    let (b1, b2) = (0.5 * (suuu + suvv), 0.5 * (svvv + svuu));
    let uc = (b1 * svv - b2 * suv) / det;
    let vc = (suu * b2 - suv * b1) / det;
    let radius = (uc * uc + vc * vc + (suu + svv) / n).sqrt();
    let center = Vec2::new(uc + cx, vc + cy)?;
    radius.is_finite().then_some((center, radius))
}

#[cfg(test)]
#[path = "recognition_tests.rs"]
mod tests;
