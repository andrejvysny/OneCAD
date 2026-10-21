//! Carrier intersections for Trim/Extend.
//!
//! Every function intersects UNBOUNDED carriers (infinite line, full circle,
//! full ellipse); callers gate the hits to whichever extents matter. A trim
//! gates to both curves' drawn extents, an extend only to the boundary's — the
//! target's continuation is exactly what it is looking along.
//!
//! Line/circle pairs are closed-form and flag a tangential touch, so an extend
//! can refuse to land on one. A circle against an ellipse is a quartic; it is
//! solved by sampling the ellipse and bisecting sign changes, which finds every
//! transversal crossing but cannot see a pure tangential touch (no sign change).
//!
//! A boundary on the target's OWN carrier (a collinear line, an arc of the same
//! circle) meets it along a stretch rather than at points. Its drawn end
//! points are reported flagged `overlap`. A collinear line's stand in for
//! crossings (a trim cuts at them, an extend stops at the nearer one or
//! refuses when the end already lies on it); a same-circle arc's are never
//! cut or welded at, only refused at. A full circle on the same circle has no
//! end points and so no hits.

use std::f64::consts::TAU;

use super::geom::{on_circle, Curve, Pt};
use super::COINCIDENT_MM;

/// One carrier intersection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Hit {
    pub p: Pt,
    /// The carriers touch rather than cross here (within [`COINCIDENT_MM`]).
    pub tangent: bool,
    /// An end point of a boundary lying on the target's own carrier.
    pub overlap: bool,
}

/// Ellipse samples for the circle/ellipse root search; fine enough that two
/// crossings of any sketch-scale ellipse never share one sample interval.
const ELLIPSE_SAMPLES: usize = 720;
const BISECTIONS: usize = 64;

/// All intersections of `target`'s carrier with `other`'s carrier. `target` is
/// a line, arc or circle (an arc contributes its full circle).
pub(super) fn carrier_hits(target: &Curve, other: &Curve) -> Vec<Hit> {
    if let Some(ends) = shared_carrier_ends(target, other) {
        return ends
            .into_iter()
            .map(|p| Hit {
                p,
                tangent: false,
                overlap: true,
            })
            .collect();
    }
    match (*target, *other) {
        (Curve::Line { a, b }, Curve::Line { a: p, b: q }) => line_line(a, b, p, q),
        (Curve::Line { a, b }, Curve::Circle { c, r } | Curve::Arc { c, r, .. }) => {
            line_circle(a, b, c, r)
        }
        (Curve::Circle { c, r } | Curve::Arc { c, r, .. }, Curve::Line { a, b }) => {
            line_circle(a, b, c, r)
        }
        (
            Curve::Circle { c, r } | Curve::Arc { c, r, .. },
            Curve::Circle { c: c2, r: r2 } | Curve::Arc { c: c2, r: r2, .. },
        ) => circle_circle(c, r, c2, r2),
        (Curve::Line { a, b }, ell @ Curve::Ellipse { .. }) => line_ellipse(a, b, &ell),
        (Curve::Circle { c, r } | Curve::Arc { c, r, .. }, ell @ Curve::Ellipse { .. }) => {
            circle_ellipse(c, r, &ell)
        }
        (Curve::Ellipse { .. }, _) => Vec::new(),
    }
}

/// The drawn end points of `other` when it lies on `target`'s carrier: a
/// non-degenerate collinear line, or an arc of the same circle.
fn shared_carrier_ends(target: &Curve, other: &Curve) -> Option<Vec<Pt>> {
    match (*target, *other) {
        (Curve::Line { a, b }, Curve::Line { a: p, b: q }) => {
            let off = |x: Pt| ((x - a).cross(b - a) / a.dist(b)).abs();
            let collinear =
                p.dist(q) > COINCIDENT_MM && off(p) <= COINCIDENT_MM && off(q) <= COINCIDENT_MM;
            collinear.then(|| vec![p, q])
        }
        (
            Curve::Circle { c, r } | Curve::Arc { c, r, .. },
            Curve::Arc {
                c: c2,
                r: r2,
                start,
                sweep,
            },
        ) => {
            let same = c.dist(c2) <= COINCIDENT_MM && (r - r2).abs() <= COINCIDENT_MM;
            same.then(|| vec![on_circle(c2, r2, start), on_circle(c2, r2, start + sweep)])
        }
        _ => None,
    }
}

fn line_line(a: Pt, b: Pt, p: Pt, q: Pt) -> Vec<Hit> {
    let (d1, d2) = (b - a, q - p);
    let cross = d1.cross(d2);
    if cross.abs() <= 1e-12 * d1.len() * d2.len() {
        return Vec::new();
    }
    let t = (p - a).cross(d2) / cross;
    vec![Hit {
        p: a + d1 * t,
        tangent: false,
        overlap: false,
    }]
}

/// Line parameters (along `a→b`) where the infinite line meets circle `(c, r)`;
/// a touch within `tol` of the radius is ONE tangent root.
fn line_circle_params(a: Pt, b: Pt, c: Pt, r: f64, tol: f64) -> Vec<(f64, bool)> {
    let d = b - a;
    let dd = d.dot(d);
    if dd <= 0.0 {
        return Vec::new();
    }
    let t0 = (c - a).dot(d) / dd;
    let h = c.dist(a + d * t0);
    if h > r + tol {
        return Vec::new();
    }
    if (h - r).abs() <= tol {
        return vec![(t0, true)];
    }
    let k = (r * r - h * h).sqrt() / dd.sqrt();
    vec![(t0 - k, false), (t0 + k, false)]
}

fn line_circle(a: Pt, b: Pt, c: Pt, r: f64) -> Vec<Hit> {
    line_circle_params(a, b, c, r, COINCIDENT_MM)
        .into_iter()
        .map(|(t, tangent)| Hit {
            p: a + (b - a) * t,
            tangent,
            overlap: false,
        })
        .collect()
}

fn circle_circle(c1: Pt, r1: f64, c2: Pt, r2: f64) -> Vec<Hit> {
    let d = c1.dist(c2);
    // Concentric circles either coincide (no discrete crossing) or never meet.
    if d <= COINCIDENT_MM || d > r1 + r2 + COINCIDENT_MM || d < (r1 - r2).abs() - COINCIDENT_MM {
        return Vec::new();
    }
    let u = (c2 - c1) * (1.0 / d);
    let along = (r1 * r1 - r2 * r2 + d * d) / (2.0 * d);
    let base = c1 + u * along;
    if (d - (r1 + r2)).abs() <= COINCIDENT_MM || (d - (r1 - r2).abs()).abs() <= COINCIDENT_MM {
        return vec![Hit {
            p: base,
            tangent: true,
            overlap: false,
        }];
    }
    let h = (r1 * r1 - along * along).max(0.0).sqrt();
    let perp = Pt::new(-u.y, u.x);
    vec![
        Hit {
            p: base + perp * h,
            tangent: false,
            overlap: false,
        },
        Hit {
            p: base - perp * h,
            tangent: false,
            overlap: false,
        },
    ]
}

/// Maps `p` into the ellipse's unit-circle frame (an affine map, so line
/// parameters are preserved).
fn to_unit(p: Pt, c: Pt, major: f64, minor: f64, rot: f64) -> Pt {
    let q = p - c;
    let (sin, cos) = rot.sin_cos();
    Pt::new(
        (q.x * cos + q.y * sin) / major,
        (q.y * cos - q.x * sin) / minor,
    )
}

fn line_ellipse(a: Pt, b: Pt, ellipse: &Curve) -> Vec<Hit> {
    let Curve::Ellipse {
        c,
        major,
        minor,
        rot,
    } = *ellipse
    else {
        return Vec::new();
    };
    let (ua, ub) = (
        to_unit(a, c, major, minor, rot),
        to_unit(b, c, major, minor, rot),
    );
    let tol = COINCIDENT_MM / major.min(minor);
    line_circle_params(ua, ub, Pt::new(0.0, 0.0), 1.0, tol)
        .into_iter()
        .map(|(t, tangent)| Hit {
            p: a + (b - a) * t,
            tangent,
            overlap: false,
        })
        .collect()
}

fn ellipse_point(c: Pt, major: f64, minor: f64, rot: f64, phi: f64) -> Pt {
    let (sin, cos) = rot.sin_cos();
    let (x, y) = (major * phi.cos(), minor * phi.sin());
    c + Pt::new(x * cos - y * sin, x * sin + y * cos)
}

fn circle_ellipse(cc: Pt, r: f64, ellipse: &Curve) -> Vec<Hit> {
    let Curve::Ellipse {
        c,
        major,
        minor,
        rot,
    } = *ellipse
    else {
        return Vec::new();
    };
    let at = |phi: f64| ellipse_point(c, major, minor, rot, phi);
    let f = |phi: f64| {
        let d = at(phi).dist(cc);
        d * d - r * r
    };
    let step = TAU / ELLIPSE_SAMPLES as f64;
    let mut hits = Vec::new();
    for i in 0..ELLIPSE_SAMPLES {
        let (mut lo, mut hi) = (i as f64 * step, (i + 1) as f64 * step);
        let (flo, fhi) = (f(lo), f(hi));
        if flo == 0.0 {
            hits.push(at(lo));
            continue;
        }
        if flo * fhi >= 0.0 {
            continue;
        }
        for _ in 0..BISECTIONS {
            let mid = 0.5 * (lo + hi);
            if f(mid) * flo > 0.0 {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        hits.push(at(0.5 * (lo + hi)));
    }
    hits.into_iter()
        .map(|p| Hit {
            p,
            tangent: false,
            overlap: false,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(ax: f64, ay: f64, bx: f64, by: f64) -> Curve {
        Curve::Line {
            a: Pt::new(ax, ay),
            b: Pt::new(bx, by),
        }
    }

    #[test]
    fn line_line_crossing_and_parallel() {
        let hits = carrier_hits(&line(0.0, 0.0, 10.0, 0.0), &line(5.0, -5.0, 5.0, 5.0));
        assert_eq!(hits.len(), 1);
        assert!(hits[0].p.dist(Pt::new(5.0, 0.0)) < 1e-12);
        assert!(carrier_hits(&line(0.0, 0.0, 1.0, 0.0), &line(0.0, 1.0, 1.0, 1.0)).is_empty());
    }

    #[test]
    fn a_same_carrier_boundary_meets_at_its_end_points() {
        let collinear = carrier_hits(&line(0.0, 0.0, 20.0, 0.0), &line(12.0, 0.0, 8.0, 0.0));
        assert_eq!(
            collinear.iter().map(|h| h.p).collect::<Vec<_>>(),
            vec![Pt::new(12.0, 0.0), Pt::new(8.0, 0.0)]
        );
        assert!(collinear.iter().all(|h| h.overlap && !h.tangent));
        let circle = Curve::Circle {
            c: Pt::new(0.0, 0.0),
            r: 5.0,
        };
        let arc = Curve::Arc {
            c: Pt::new(0.0, 0.0),
            r: 5.0,
            start: 0.0,
            sweep: std::f64::consts::FRAC_PI_2,
        };
        let same = carrier_hits(&circle, &arc);
        assert_eq!(same.len(), 2);
        assert!(same[0].p.dist(Pt::new(5.0, 0.0)) < 1e-12);
        assert!(same[1].p.dist(Pt::new(0.0, 5.0)) < 1e-12);
        assert!(
            carrier_hits(&arc, &circle).is_empty(),
            "a full circle has no ends"
        );
    }

    #[test]
    fn line_circle_two_roots_and_tangent() {
        let circle = Curve::Circle {
            c: Pt::new(0.0, 0.0),
            r: 5.0,
        };
        let two = carrier_hits(&line(-10.0, 0.0, 10.0, 0.0), &circle);
        assert_eq!(two.len(), 2);
        assert!(two
            .iter()
            .all(|h| !h.tangent && (h.p.len() - 5.0).abs() < 1e-12));
        let touch = carrier_hits(&line(-10.0, 5.0, 10.0, 5.0), &circle);
        assert_eq!(touch.len(), 1);
        assert!(touch[0].tangent);
    }

    #[test]
    fn circle_circle_crossing_tangent_and_concentric() {
        let a = Curve::Circle {
            c: Pt::new(0.0, 0.0),
            r: 5.0,
        };
        let crossing = Curve::Circle {
            c: Pt::new(6.0, 0.0),
            r: 5.0,
        };
        assert_eq!(carrier_hits(&a, &crossing).len(), 2);
        let external = Curve::Circle {
            c: Pt::new(10.0, 0.0),
            r: 5.0,
        };
        let touch = carrier_hits(&a, &external);
        assert_eq!(touch.len(), 1);
        assert!(touch[0].tangent && touch[0].p.dist(Pt::new(5.0, 0.0)) < 1e-9);
        let concentric = Curve::Circle {
            c: Pt::new(0.0, 0.0),
            r: 3.0,
        };
        assert!(carrier_hits(&a, &concentric).is_empty());
    }

    #[test]
    fn line_and_circle_meet_a_rotated_ellipse_on_its_boundary() {
        let ellipse = Curve::Ellipse {
            c: Pt::new(1.0, 2.0),
            major: 6.0,
            minor: 3.0,
            rot: 0.4,
        };
        let on_ellipse = |p: Pt| {
            let u = to_unit(p, Pt::new(1.0, 2.0), 6.0, 3.0, 0.4);
            (u.len() - 1.0).abs() < 1e-9
        };
        let hits = carrier_hits(&line(-20.0, 2.0, 20.0, 2.0), &ellipse);
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().all(|h| on_ellipse(h.p)));
        let circle = Curve::Circle {
            c: Pt::new(1.0, 2.0),
            r: 4.5,
        };
        let hits = carrier_hits(&circle, &ellipse);
        assert_eq!(
            hits.len(),
            4,
            "a circle between the semi-axes crosses 4 times"
        );
        assert!(hits
            .iter()
            .all(|h| on_ellipse(h.p) && (h.p.dist(Pt::new(1.0, 2.0)) - 4.5).abs() < 1e-9));
    }
}
