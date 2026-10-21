//! Carrier intersections for Trim/Extend.
//!
//! Every function intersects UNBOUNDED carriers (infinite line, full circle,
//! full ellipse); callers gate the hits to whichever extents matter. A trim
//! gates to both curves' drawn extents, an extend only to the boundary's — the
//! target's continuation is exactly what it is looking along.
//!
//! Line/circle pairs are closed-form and flag a tangential touch, so an extend
//! can refuse to land on one. A circle against an ellipse is a quartic; it is
//! solved by subdividing the ellipse until a derivative bound proves each
//! piece has no root, exactly one (bisected), or is narrower than a fraction
//! of [`COINCIDENT_MM`], where an extremum within it is flagged a touch.
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

/// Seed intervals for the circle/ellipse root search. Each is subdivided as
/// far as needed, so this only sets where the search starts.
const ELLIPSE_SAMPLES: usize = 720;
const BISECTIONS: usize = 64;
/// Subdivision caps: only a coincident or osculating pair, where `f` stays
/// near zero over a stretch, reaches them; the search then stays bounded.
const MAX_DEPTH: u32 = 48;
const NODE_BUDGET: usize = 1 << 14;

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
    let reach = major.max(minor);
    let offset = c - cc;
    let far = offset.len() + reach;
    let mut search = CircleEllipse {
        c,
        major,
        minor,
        rot,
        offset,
        r,
        f2: 2.0 * reach * (reach + far),
        band: COINCIDENT_MM * (2.0 * r + COINCIDENT_MM),
        span: far + r,
        leaf: COINCIDENT_MM / (4.0 * reach),
        nodes: 0,
        roots: Vec::new(),
    };
    let step = TAU / ELLIPSE_SAMPLES as f64;
    let seeds: Vec<Sample> = (0..ELLIPSE_SAMPLES)
        .map(|i| search.sample(i as f64 * step))
        .collect();
    for i in 0..ELLIPSE_SAMPLES {
        // The last interval ends on the first seed's values, so a root or
        // extremum at φ = 0 is seen exactly once.
        let mut hi = seeds[(i + 1) % ELLIPSE_SAMPLES];
        hi.phi = (i + 1) as f64 * step;
        search.visit(seeds[i], hi, 0);
    }
    search.merged_hits()
}

/// `f` and `f'` at one ellipse angle; neighbouring intervals share it.
#[derive(Clone, Copy)]
struct Sample {
    phi: f64,
    f: f64,
    df: f64,
}

/// Root search for `f(φ) = |E(φ) − cc|² − r²`, a degree-2 trigonometric
/// polynomial with up to four roots, any two of which may be arbitrarily close.
/// `f` is evaluated from `E − cc = (c − cc) + R·(a cos φ, b sin φ)`, so its
/// rounding depends on the pair's relative size, not where it sits.
struct CircleEllipse {
    c: Pt,
    major: f64,
    minor: f64,
    rot: f64,
    /// `c − cc`.
    offset: Pt,
    r: f64,
    /// `≥ max|f''|`: `f'' = 2(|E'|² + (E − cc)·E'')`, `|E'|, |E − c| ≤ reach`.
    f2: f64,
    /// `|f|` bound of a point within [`COINCIDENT_MM`] of the circle.
    band: f64,
    /// `|c − cc| + reach + r`, the magnitude every rounding in `f` scales by.
    span: f64,
    /// Interval width below which subdivision stops.
    leaf: f64,
    nodes: usize,
    /// `(φ, tangent)` in increasing φ.
    roots: Vec<(f64, bool)>,
}

impl CircleEllipse {
    fn point(&self, phi: f64) -> Pt {
        ellipse_point(self.c, self.major, self.minor, self.rot, phi)
    }

    /// `E(φ) − cc`.
    fn rel(&self, phi: f64) -> Pt {
        ellipse_point(self.offset, self.major, self.minor, self.rot, phi)
    }

    fn f(&self, phi: f64) -> f64 {
        let d = self.rel(phi);
        d.dot(d) - self.r * self.r
    }

    /// First-order bound on the rounding of `f(φ)` as computed, `u = ε`.
    ///
    /// `c − cc` rounds by `u|c − cc|`; `cos`, `sin` (libm, under 1 ulp), the
    /// axis scaling and the rotation add about `5u·reach` per component, so
    /// `|δd| ≤ 8u(|c − cc| + reach)`. `d·d − r²` then rounds by
    /// `2|d||δd| + 3u(|d|² + r²)`, all within `16u(|d| + r)(span + |d|)`:
    /// about `16u·r·span` near a root, independent of where the pair sits.
    /// Doubled because a merge tests a computed `f` whose true value at a
    /// flickering root's neighbour may itself be off by the bound.
    fn noise(&self, phi: f64) -> f64 {
        let d = self.rel(phi).len();
        32.0 * f64::EPSILON * (d + self.r) * (self.span + d)
    }

    fn df(&self, phi: f64) -> f64 {
        let (sin, cos) = self.rot.sin_cos();
        let (x, y) = (-self.major * phi.sin(), self.minor * phi.cos());
        let tangent = Pt::new(x * cos - y * sin, x * sin + y * cos);
        2.0 * self.rel(phi).dot(tangent)
    }

    fn sample(&self, phi: f64) -> Sample {
        Sample {
            phi,
            f: self.f(phi),
            df: self.df(phi),
        }
    }

    fn visit(&mut self, lo: Sample, hi: Sample, depth: u32) {
        self.nodes += 1;
        let h = 0.5 * (hi.phi - lo.phi);
        let m = self.sample(lo.phi + h);
        if m.df.abs() > self.f2 * h {
            // f' cannot vanish here: f is monotone, so one root at most.
            self.crossing(lo, hi);
            return;
        }
        if m.f.abs() - m.df.abs() * h - 0.5 * self.f2 * h * h > self.band {
            return; // Taylor bound: no root and no touch anywhere in [lo, hi].
        }
        if 2.0 * h <= self.leaf || depth >= MAX_DEPTH || self.nodes >= NODE_BUDGET {
            self.leaf_roots(lo, hi);
            return;
        }
        self.visit(lo, m, depth + 1);
        self.visit(m, hi, depth + 1);
    }

    /// Records the root a sign change brackets. Zero counts as positive, so
    /// an exact zero is one crossing when f passes through it and none (or a
    /// pair, merged later) when f only touches it.
    fn crossing(&mut self, lo: Sample, hi: Sample) -> bool {
        let up = lo.f >= 0.0;
        if up == (hi.f >= 0.0) {
            return false;
        }
        let phi = bisect(|x| self.f(x) >= 0.0, up, lo.phi, hi.phi);
        self.roots.push((phi, false));
        true
    }

    /// A leaf may still hold an extremum of f: two roots either side of it,
    /// or a touch when it approaches zero without crossing. An extremum past
    /// zero between roots outside the leaf is neither.
    fn leaf_roots(&mut self, lo: Sample, hi: Sample) {
        if self.crossing(lo, hi) {
            return;
        }
        let rising = hi.df >= 0.0;
        if (lo.df >= 0.0) == rising {
            return;
        }
        let ext = self.sample(bisect(|x| self.df(x) >= 0.0, !rising, lo.phi, hi.phi));
        if (ext.f >= 0.0) != (lo.f >= 0.0) {
            self.crossing(lo, ext);
            self.crossing(ext, hi);
            return;
        }
        // A minimum at or above zero, or a maximum below it.
        let touches = rising == (ext.f >= 0.0);
        if touches && (self.rel(ext.phi).len() - self.r).abs() <= COINCIDENT_MM {
            self.roots.push((ext.phi, true));
        }
    }

    /// Collapses each run of roots chained around the ring to one hit: a
    /// crossing when the run crosses an odd number of times, else a touch.
    /// Neighbours chain within [`COINCIDENT_MM`], or when f between them is
    /// rounding noise: where the curves agree to rounding over a stretch (an
    /// osculating or coincident pair) the sign of f flickers.
    fn merged_hits(&self) -> Vec<Hit> {
        let n = self.roots.len();
        let chained = |i: usize| {
            let (a, b) = (self.roots[(i + n - 1) % n].0, self.roots[i].0);
            let mid = 0.5 * (a + b + if b < a { TAU } else { 0.0 });
            self.point(a).dist(self.point(b)) <= COINCIDENT_MM
                || self.f(mid).abs() <= self.noise(mid)
        };
        let start = (0..n).find(|&i| !chained(i)).unwrap_or(0);
        let mut runs: Vec<Vec<(f64, bool)>> = Vec::new();
        for k in 0..n {
            let i = (start + k) % n;
            if k == 0 || !chained(i) {
                runs.push(Vec::new());
            }
            runs.last_mut().expect("pushed above").push(self.roots[i]);
        }
        runs.into_iter()
            .map(|run| Hit {
                p: self.point(run[run.len() / 2].0),
                tangent: run.iter().filter(|(_, tangent)| !tangent).count() % 2 == 0,
                overlap: false,
            })
            .collect()
    }
}

/// Where `pred` flips from `up` in `[lo, hi]`, bisected to float resolution.
fn bisect(pred: impl Fn(f64) -> bool, up: bool, mut lo: f64, mut hi: f64) -> f64 {
    for _ in 0..BISECTIONS {
        let mid = 0.5 * (lo + hi);
        if pred(mid) == up {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
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

    fn origin_ellipse(major: f64, minor: f64) -> Curve {
        Curve::Ellipse {
            c: Pt::new(0.0, 0.0),
            major,
            minor,
            rot: 0.0,
        }
    }

    /// Every hit lies on both carriers, and each expected point has a hit.
    fn assert_hits(hits: &[Hit], circle: (Pt, f64), (major, minor): (f64, f64), expect: &[Pt]) {
        let on_ellipse =
            |p: Pt| (to_unit(p, Pt::new(0.0, 0.0), major, minor, 0.0).len() - 1.0).abs();
        for h in hits {
            assert!(!h.tangent && !h.overlap, "{h:?}");
            assert!((h.p.dist(circle.0) - circle.1).abs() < 1e-9, "{h:?}");
            assert!(on_ellipse(h.p) < 1e-9, "{h:?}");
        }
        for e in expect {
            assert!(
                hits.iter().any(|h| h.p.dist(*e) < 1e-6),
                "{e:?} missing from {hits:?}"
            );
        }
    }

    #[test]
    fn a_small_circle_centred_on_an_ellipse_crosses_it_twice() {
        // Centred mid-way through one 0.5 deg seed interval: both crossings
        // fall inside it and the seed samples share a sign.
        for (major, minor, r) in [(100.0, 50.0, 0.1), (1000.0, 500.0, 2.0)] {
            let c = ellipse_point(Pt::new(0.0, 0.0), major, minor, 0.0, 0.25f64.to_radians());
            let hits = carrier_hits(&Curve::Circle { c, r }, &origin_ellipse(major, minor));
            assert_eq!(hits.len(), 2, "{major}x{minor} r={r}: {hits:?}");
            assert_hits(&hits, (c, r), (major, minor), &[]);
        }
    }

    #[test]
    fn a_small_circle_centred_on_a_huge_ellipse_crosses_it_twice() {
        // Rounding of f scales with the circle, not the ellipse: a noise
        // bound growing with the ellipse squared merged these into a touch.
        for (major, minor, r) in [(1e5, 5e4, 0.02), (1e6, 5e5, 0.15)] {
            let c = ellipse_point(Pt::new(0.0, 0.0), major, minor, 0.0, 0.25f64.to_radians());
            let hits = carrier_hits(&Curve::Circle { c, r }, &origin_ellipse(major, minor));
            assert_eq!(hits.len(), 2, "{major}x{minor} r={r}: {hits:?}");
            for h in &hits {
                assert!(!h.tangent && (h.p.dist(c) - r).abs() < 1e-6, "{h:?}");
            }
        }
    }

    #[test]
    fn a_translated_osculating_circle_is_one_tangent_hit() {
        for at in [1e5, 1e6] {
            let ellipse = Curve::Ellipse {
                c: Pt::new(at, at),
                major: 100.0,
                minor: 50.0,
                rot: 0.0,
            };
            let circle = Curve::Circle {
                c: Pt::new(at + 75.0, at),
                r: 25.0,
            };
            let hits = carrier_hits(&circle, &ellipse);
            assert_eq!(hits.len(), 1, "at {at}: {hits:?}");
            assert!(hits[0].tangent && hits[0].p.dist(Pt::new(at + 100.0, at)) < 1e-2);
        }
    }

    #[test]
    fn a_near_tangent_circle_crosses_twice_inside_one_seed_interval() {
        // Crossings 0.3 deg apart, the circle bulging outward from the
        // ellipse between them; 0 and 0.5 deg both lie outside it.
        let at = |deg: f64| ellipse_point(Pt::new(0.0, 0.0), 100.0, 50.0, 0.0, deg.to_radians());
        let (p1, p2, r) = (at(0.1), at(0.4), 30.0);
        let chord = p2 - p1;
        let outward = Pt::new(chord.y, -chord.x) * (1.0 / chord.len());
        let offset = (r * r - chord.dot(chord) / 4.0).sqrt();
        let c = (p1 + p2) * 0.5 + outward * offset;
        let hits = carrier_hits(&Curve::Circle { c, r }, &origin_ellipse(100.0, 50.0));
        assert_eq!(hits.len(), 2, "{hits:?}");
        assert_hits(&hits, (c, r), (100.0, 50.0), &[p1, p2]);
    }

    #[test]
    fn a_circle_touching_an_ellipse_is_one_tangent_hit() {
        let phi = 0.25f64.to_radians();
        let touch = ellipse_point(Pt::new(0.0, 0.0), 100.0, 50.0, 0.0, phi);
        let normal = Pt::new(touch.x / 1e4, touch.y / 2.5e3);
        let off_seed = touch + normal * (10.0 / normal.len());
        let cases = [
            (off_seed, 10.0, touch, 1e-6),
            (Pt::new(110.0, 0.0), 10.0, Pt::new(100.0, 0.0), 1e-6),
            // Osculating at the vertex: the curves agree to rounding along a
            // stretch, and the flickering sign of f still yields one touch.
            (Pt::new(75.0, 0.0), 25.0, Pt::new(100.0, 0.0), 1e-2),
        ];
        for (c, r, at, tol) in cases {
            let hits = carrier_hits(&Curve::Circle { c, r }, &origin_ellipse(100.0, 50.0));
            assert_eq!(hits.len(), 1, "{c:?} r={r}: {hits:?}");
            assert!(hits[0].tangent && hits[0].p.dist(at) < tol, "{hits:?}");
        }
    }
}
