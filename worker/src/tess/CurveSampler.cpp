// CurveSampler.cpp — see CurveSampler.h. NUM §2 is the binding derivation.
#include "tess/CurveSampler.h"

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <limits>
#include <string>
#include <utility>
#include <vector>

#include <Adaptor3d_Curve.hxx>
#include <BRepAdaptor_Curve.hxx>
#include <BRep_Tool.hxx>
#include <GeomAbs_CurveType.hxx>
#include <GeomAbs_Shape.hxx>
#include <GeomConvert.hxx>
#include <GeomConvert_ApproxCurve.hxx>
#include <GeomConvert_BSplineCurveToBezierCurve.hxx>
#include <Geom_BSplineCurve.hxx>
#include <Geom_BezierCurve.hxx>
#include <Geom_Curve.hxx>
#include <Geom_TrimmedCurve.hxx>
#include <NCollection_Array1.hxx>
#include <Precision.hxx>
#include <Standard_Failure.hxx>
#include <TopoDS_Edge.hxx>
#include <gp_Trsf.hxx>

namespace onecad::tess {

namespace {

// NUM §1.2 scale-dependent roundoff estimate, applied to quantities that have
// already been scaled to a greatest magnitude of one.
constexpr double kRoundoff = 64.0 * std::numeric_limits<double>::epsilon();

// NUM §2.4: the relative magnitude below which a derivative-numerator
// coefficient is TOO SMALL TO TEST — the cone arithmetic on it would be noise.
// It is NOT a licence to ignore the coefficient: a nonzero coefficient below
// this floor WITHDRAWS the span's angular claim (status Undefined) instead of
// being dropped from a `Satisfied` verdict, because a coefficient this small
// sits exactly where the tangent swings fastest. Only an EXACTLY zero
// coefficient may be dropped, and only because Q is then provably a
// non-negative Bernstein combination of the rest.
constexpr double kAngularTestFloor = 1e-9;

// NUM §2.2. Poor weight conditioning cannot corrupt the CHORD certificate — the
// convex-hull property holds for any positive weights and this sampler never
// evaluates the rational curve, only its poles. It can corrupt the derivative
// numerator, so below this ratio the ANGULAR claim is withdrawn. Individual
// weights are NEVER clamped, and conditioning never vetoes a certified chord.
constexpr double kWeightConditionFloor = 1e-12;

// Once a segment is written as float32 its DIRECTION is noise unless the chord
// is comfortably longer than the rounding of its own endpoints.
constexpr double kShortChordQuantizationFactor = 16.0;

// The RESOLUTION FLOOR at which an angular question stops being answerable.
// A true cusp makes the cone fail for every span that contains it, at any
// tolerance, so subdivision must be allowed to stop — but only once the
// offending span contributes less than a thousandth of the display budget, far
// below anything a refinement of it could change on screen. This is a property
// of the geometry and the output format, NOT a work cap: it does not depend on
// how much subdivision has happened. Using the whole budget here instead was
// measured to let 67-degree creases through the 5-degree fine tier.
constexpr double kAngularExhaustionFraction = 1.0 / 1024.0;

constexpr double kHalfPi = 1.5707963267948966;
constexpr int kConicPreSplitRounds = 8;
constexpr int kMaxUnrolledPeriods = 64;
constexpr std::size_t kIndependentSampleCap = 200000;
constexpr int kMaxSpanPoles = 33;  // degree 32 -> derivative numerator degree 63
constexpr int kApproxMaxSegments = 64;
constexpr int kApproxMaxDegree = 14;

std::string num_text(double v) {
    char buf[32];
    std::snprintf(buf, sizeof(buf), "%.6g", v);
    return std::string(buf);
}

// Pascal's triangle. The widest Bernstein product this sampler builds is the
// derivative numerator at degree 2n-1, so with kMaxSpanPoles = 33 (degree 32)
// it needs C(63, k); the table therefore runs to 64.
const std::vector<std::vector<double>>& pascal() {
    static const std::vector<std::vector<double>> table = [] {
        std::vector<std::vector<double>> t(65, std::vector<double>(65, 0.0));
        for (std::size_t n = 0; n < t.size(); ++n) {
            t[n][0] = 1.0;
            for (std::size_t k = 1; k <= n; ++k) {
                t[n][k] = t[n - 1][k - 1] + (k < n ? t[n - 1][k] : 0.0);
            }
        }
        return t;
    }();
    return table;
}

double binom(int n, int k) {
    if (n < 0 || k < 0 || k > n || n > 64) return 0.0;
    return pascal()[static_cast<std::size_t>(n)][static_cast<std::size_t>(k)];
}

// NUM §2.4 Bernstein product: c_k = sum_i C(m,i)C(n,k-i)/C(m+n,k) * a_i * b_(k-i)
// with vector coefficients `a` (degree m) and scalar coefficients `b` (degree n).
std::vector<gp_XYZ> bern_product(const std::vector<gp_XYZ>& a, const std::vector<double>& b) {
    if (a.empty() || b.empty()) return {};
    const int m = static_cast<int>(a.size()) - 1;
    const int n = static_cast<int>(b.size()) - 1;
    std::vector<gp_XYZ> out(static_cast<std::size_t>(m + n + 1));
    for (int k = 0; k <= m + n; ++k) {
        gp_XYZ acc(0.0, 0.0, 0.0);
        const int lo = std::max(0, k - n);
        const int hi = std::min(m, k);
        for (int i = lo; i <= hi; ++i) {
            const double c = binom(m, i) * binom(n, k - i) / binom(m + n, k);
            acc.Add(
                a[static_cast<std::size_t>(i)].Multiplied(c * b[static_cast<std::size_t>(k - i)]));
        }
        out[static_cast<std::size_t>(k)] = acc;
    }
    return out;
}

struct Hom {
    double x = 0.0, y = 0.0, z = 0.0, w = 0.0;
};

Hom hom_midpoint(const Hom& p, const Hom& q) {
    return Hom{0.5 * (p.x + q.x), 0.5 * (p.y + q.y), 0.5 * (p.z + q.z), 0.5 * (p.w + q.w)};
}

// Normalizing by the greatest weight leaves the curve unchanged (NUM §2.2) and
// keeps the homogeneous coordinates in a comparable range across subdivisions.
void normalize_weights(RationalBezierSpan& span) {
    double hi = 0.0;
    for (double w : span.weights) hi = std::max(hi, w);
    if (!(hi > 0.0) || hi == 1.0) return;
    for (double& w : span.weights) w /= hi;
}

double weight_condition(const RationalBezierSpan& span) {
    double lo = std::numeric_limits<double>::infinity();
    double hi = 0.0;
    for (double w : span.weights) {
        lo = std::min(lo, w);
        hi = std::max(hi, w);
    }
    if (!(hi > 0.0)) return 0.0;
    return lo / hi;
}

bool is_finite(const gp_XYZ& p) {
    return std::isfinite(p.X()) && std::isfinite(p.Y()) && std::isfinite(p.Z());
}

bool is_finite(const gp_Pnt& p) { return is_finite(p.XYZ()); }

bool weights_are_positive_and_finite(const RationalBezierSpan& span) {
    if (span.poles.size() != span.weights.size() || span.poles.empty()) return false;
    for (std::size_t i = 0; i < span.weights.size(); ++i) {
        if (!std::isfinite(span.weights[i]) || span.weights[i] <= 0.0) return false;
        if (!is_finite(span.poles[i])) return false;
    }
    return true;
}

// float32 spacing at |v|. PER AXIS, because a coordinate that is exactly
// representable (1e9 = 1953125 * 2^9) costs nothing however large it is, while
// a blanket bound taken from the largest axis would starve the other two.
double float32_ulp(double v) {
    const float f = std::abs(static_cast<float>(v));
    if (!(f > 0.0F) || !std::isfinite(f)) return 0.0;
    return static_cast<double>(std::nextafterf(f, std::numeric_limits<float>::infinity()) - f);
}

// The a-priori float32 allowance for one span, from its own pole extents (the
// poles bound the curve piece, so this is sound and tight).
double span_representation_allowance(const RationalBezierSpan& span) {
    double mx = 0.0, my = 0.0, mz = 0.0;
    for (const gp_XYZ& p : span.poles) {
        mx = std::max(mx, std::abs(p.X()));
        my = std::max(my, std::abs(p.Y()));
        mz = std::max(mz, std::abs(p.Z()));
    }
    const double ux = float32_ulp(mx);
    const double uy = float32_ulp(my);
    const double uz = float32_ulp(mz);
    return 0.5 * std::sqrt(ux * ux + uy * uy + uz * uz);
}

// The MEASURED float32 rounding of one point (NUM §1.2: measure, do not guess).
double measured_quantization(const gp_Pnt& p) {
    const gp_XYZ rounded(static_cast<double>(static_cast<float>(p.X())),
                         static_cast<double>(static_cast<float>(p.Y())),
                         static_cast<double>(static_cast<float>(p.Z())));
    return rounded.Subtracted(p.XYZ()).Modulus();
}

}  // namespace

// ---------------------------------------------------------------------------
// Pure helpers (NUM §2.2-§2.4)
// ---------------------------------------------------------------------------

void subdivide_homogeneous(const RationalBezierSpan& span, RationalBezierSpan& left,
                           RationalBezierSpan& right) {
    left = RationalBezierSpan{};
    right = RationalBezierSpan{};
    if (span.poles.empty() || span.poles.size() != span.weights.size()) return;

    const int n = static_cast<int>(span.poles.size()) - 1;
    const std::size_t count = span.poles.size();
    std::vector<Hom> work(count);
    for (std::size_t i = 0; i < count; ++i) {
        const double w = span.weights[i];
        work[i] = Hom{span.poles[i].X() * w, span.poles[i].Y() * w, span.poles[i].Z() * w, w};
    }

    std::vector<Hom> lh(count), rh(count);
    lh[0] = work[0];
    rh[static_cast<std::size_t>(n)] = work[static_cast<std::size_t>(n)];
    for (int k = 1; k <= n; ++k) {
        for (int i = 0; i + k <= n; ++i) {
            work[static_cast<std::size_t>(i)] = hom_midpoint(
                work[static_cast<std::size_t>(i)], work[static_cast<std::size_t>(i + 1)]);
        }
        // At k == n both of these read work[0]: the shared split point is
        // computed once and handed to both children, so the halves meet exactly.
        lh[static_cast<std::size_t>(k)] = work[0];
        rh[static_cast<std::size_t>(n - k)] = work[static_cast<std::size_t>(n - k)];
    }

    const double mid = 0.5 * (span.t0 + span.t1);
    left.poles.resize(count);
    left.weights.resize(count);
    right.poles.resize(count);
    right.weights.resize(count);
    for (std::size_t i = 0; i < count; ++i) {
        left.poles[i] = gp_XYZ(lh[i].x / lh[i].w, lh[i].y / lh[i].w, lh[i].z / lh[i].w);
        left.weights[i] = lh[i].w;
        right.poles[i] = gp_XYZ(rh[i].x / rh[i].w, rh[i].y / rh[i].w, rh[i].z / rh[i].w);
        right.weights[i] = rh[i].w;
    }
    left.t0 = span.t0;
    left.t1 = mid;
    right.t0 = mid;
    right.t1 = span.t1;
    left.depth = right.depth = span.depth + 1;
    left.at_source_start = span.at_source_start;
    left.at_source_end = false;
    right.at_source_start = false;
    right.at_source_end = span.at_source_end;
    normalize_weights(left);
    normalize_weights(right);
}

double hull_chord_bound(const RationalBezierSpan& span) {
    if (span.poles.size() < 2) return 0.0;
    const gp_XYZ& a = span.poles.front();
    const gp_XYZ v = span.poles.back().Subtracted(a);
    const double len2 = v.SquareModulus();
    double worst = 0.0;
    for (const gp_XYZ& p : span.poles) {
        const gp_XYZ w = p.Subtracted(a);
        double d = 0.0;
        if (len2 > 0.0) {
            const double u = std::clamp(w.Dot(v) / len2, 0.0, 1.0);
            d = w.Subtracted(v.Multiplied(u)).Modulus();
        } else {
            d = w.Modulus();  // degenerate chord: distance to A (NUM §2.3)
        }
        worst = std::max(worst, d);
    }
    return worst;
}

std::vector<gp_XYZ> derivative_numerator_bernstein(const RationalBezierSpan& span) {
    if (span.poles.size() < 2 || span.poles.size() != span.weights.size()) return {};
    const int n = static_cast<int>(span.poles.size()) - 1;
    const std::size_t count = span.poles.size();

    std::vector<gp_XYZ> x(count);
    std::vector<double> w(count);
    for (std::size_t i = 0; i < count; ++i) {
        x[i] = span.poles[i].Multiplied(span.weights[i]);
        w[i] = span.weights[i];
    }
    std::vector<gp_XYZ> dx(count - 1);
    std::vector<double> dw(count - 1);
    for (std::size_t i = 0; i + 1 < count; ++i) {
        dx[i] = x[i + 1].Subtracted(x[i]).Multiplied(static_cast<double>(n));
        dw[i] = (w[i + 1] - w[i]) * static_cast<double>(n);
    }

    const std::vector<gp_XYZ> lhs = bern_product(dx, w);  // X'W, degree 2n-1
    const std::vector<gp_XYZ> rhs = bern_product(x, dw);  // XW',  degree 2n-1
    std::vector<gp_XYZ> q(lhs.size());
    for (std::size_t k = 0; k < lhs.size(); ++k) q[k] = lhs[k].Subtracted(rhs[k]);
    return q;
}

namespace {

// What the cone test actually established about a span.
struct ConeEvidence {
    AngularStatus status = AngularStatus::Uncertified;
    // The derivative vanishes EXACTLY at this end: a genuine cusp or stationary
    // point, i.e. a place where no tangent exists.
    bool singularAtStart = false;
    bool singularAtEnd = false;
    // A coefficient was nonzero but too small to test, or the weights were too
    // ill-conditioned to trust the numerator. Position is unaffected; the
    // angular claim is simply not established.
    bool evidenceIncomplete = false;
};

ConeEvidence tangent_cone_evidence(const RationalBezierSpan& span, double angularTol,
                                   double testFloor) {
    ConeEvidence evidence;
    const std::vector<gp_XYZ> q = derivative_numerator_bernstein(span);
    if (q.empty()) {
        evidence.singularAtStart = evidence.singularAtEnd = true;
        evidence.status = AngularStatus::Undefined;
        return evidence;
    }
    double qmax = 0.0;
    for (const gp_XYZ& c : q) {
        if (!is_finite(c)) {
            evidence.evidenceIncomplete = true;
            evidence.status = AngularStatus::Undefined;
            return evidence;
        }
        qmax = std::max(qmax, c.Modulus());
    }
    if (!(qmax > 0.0)) {
        // Q vanishes identically: the span is a single point, not a direction.
        evidence.singularAtStart = evidence.singularAtEnd = true;
        evidence.status = AngularStatus::Undefined;
        return evidence;
    }

    gp_XYZ d = span.poles.back().Subtracted(span.poles.front());
    const double dlen = d.Modulus();
    if (!(dlen > 0.0)) {
        evidence.status = AngularStatus::Uncertified;  // closed span: no chord direction
        return evidence;
    }
    d.Multiply(1.0 / dlen);  // reciprocal: gp_XYZ::Divide raises below gp::Resolution()

    if (weight_condition(span) < kWeightConditionFloor) evidence.evidenceIncomplete = true;

    const double tan_half = std::tan(0.5 * angularTol);
    for (std::size_t i = 0; i < q.size(); ++i) {
        if (q[i].Modulus() == 0.0) {
            // An EXACTLY zero coefficient cannot push Q(t) out of the cone: Q is
            // a non-negative Bernstein combination of the rest, and a convex cone
            // is closed under those. At an end it IS Q, so no tangent exists
            // there — a singular vertex, which the caller publishes.
            if (i == 0) evidence.singularAtStart = true;
            if (i + 1 == q.size()) evidence.singularAtEnd = true;
            continue;
        }
        const gp_XYZ s = q[i].Multiplied(1.0 / qmax);
        if (s.Modulus() <= testFloor) {
            // Nonzero but unmeasurable. Withdraw the angular claim rather than
            // certify a cone this coefficient may well leave.
            evidence.evidenceIncomplete = true;
            continue;
        }
        const double parallel = s.Dot(d);
        if (parallel <= testFloor) {
            evidence.status = AngularStatus::Uncertified;  // turnaround
            return evidence;
        }
        const double perpendicular = s.Subtracted(d.Multiplied(parallel)).Modulus();
        if (perpendicular > tan_half * parallel) {
            evidence.status = AngularStatus::Uncertified;
            return evidence;
        }
    }
    const bool undefined =
        evidence.singularAtStart || evidence.singularAtEnd || evidence.evidenceIncomplete;
    evidence.status = undefined ? AngularStatus::Undefined : AngularStatus::Satisfied;
    return evidence;
}

}  // namespace

AngularStatus tangent_cone_status(const RationalBezierSpan& span, double angularTol,
                                  double roundoffBound, bool& undefinedAtStart,
                                  bool& undefinedAtEnd) {
    const ConeEvidence evidence = tangent_cone_evidence(span, angularTol, roundoffBound);
    undefinedAtStart = evidence.singularAtStart;
    undefinedAtEnd = evidence.singularAtEnd;
    return evidence.status;
}

AngularStatus tangent_cone_status(const RationalBezierSpan& span, double angularTol,
                                  double roundoffBound) {
    bool start = false;
    bool end = false;
    return tangent_cone_status(span, angularTol, roundoffBound, start, end);
}

// ---------------------------------------------------------------------------
// Source-span construction (NUM §2.1, §2.7)
// ---------------------------------------------------------------------------

namespace {

// One exactly-convertible piece of the edge. `a`/`b` are parameters in `bs`'s
// own domain; `edgeStart`/`edgeEnd` are the EDGE parameters that piece covers.
// Leaf parameters are the affine image of [a,b] on [edgeStart, edgeEnd], so an
// edge's leaves stay contiguous end to end even when the source was
// re-parameterised (conics) or unrolled across a period (periodic splines).
struct BSplinePiece {
    Handle(Geom_BSplineCurve) bs;
    double a = 0.0;
    double b = 0.0;
    double edgeStart = 0.0;
    double edgeEnd = 0.0;
};

struct SpanSet {
    std::vector<RationalBezierSpan> spans;
    bool approximate = false;
    bool ok = false;
    std::string diagnostic;
};

// The arcs of a BSpline→Bézier conversion are delimited by knot values. Fall
// back to a uniform split of [a,b] only if OCCT hands back something that is not
// a strictly increasing (NbArcs + 1) sequence.
std::vector<double> arc_knots(GeomConvert_BSplineCurveToBezierCurve& conv, double a, double b) {
    const int nb = conv.NbArcs();
    std::vector<double> knots;
    try {
        NCollection_Array1<double> raw(1, nb + 1);
        conv.Knots(raw);
        for (int i = 1; i <= nb + 1; ++i) knots.push_back(raw(i));
    } catch (const Standard_Failure&) {
        knots.clear();
    }
    bool usable = knots.size() == static_cast<std::size_t>(nb) + 1;
    for (std::size_t i = 0; usable && i + 1 < knots.size(); ++i) {
        usable = std::isfinite(knots[i]) && knots[i + 1] > knots[i];
    }
    if (!usable) {
        knots.clear();
        for (int i = 0; i <= nb; ++i) {
            knots.push_back(a + (b - a) * static_cast<double>(i) / static_cast<double>(nb));
        }
    }
    return knots;
}

bool spans_from_piece(const BSplinePiece& piece, const gp_Trsf& trsf, bool identity,
                      std::vector<RationalBezierSpan>& out, std::string& diagnostic) {
    GeomConvert_BSplineCurveToBezierCurve conv(piece.bs, piece.a, piece.b, Precision::PConfusion());
    const int arcs = conv.NbArcs();
    if (arcs < 1) {
        diagnostic = "BSpline to Bezier conversion produced no arcs";
        return false;
    }
    const std::vector<double> knots = arc_knots(conv, piece.a, piece.b);
    // Identity map when the piece already lives in edge parameters — keep the
    // knot values verbatim so a leaf boundary lands EXACTLY on the knot.
    const bool identity_domain = piece.a == piece.edgeStart && piece.b == piece.edgeEnd;
    const double scale =
        identity_domain ? 1.0 : (piece.edgeEnd - piece.edgeStart) / (piece.b - piece.a);
    auto to_edge = [&](double t) {
        return identity_domain ? t : piece.edgeStart + (t - piece.a) * scale;
    };

    for (int i = 1; i <= arcs; ++i) {
        Handle(Geom_BezierCurve) arc = conv.Arc(i);
        if (arc.IsNull()) {
            diagnostic = "null Bezier arc from the exact conversion";
            return false;
        }
        const int poles = arc->NbPoles();
        if (poles > kMaxSpanPoles) {
            // The derivative numerator is degree 2n-1 and the Bernstein product
            // needs C(2n-1, k); beyond this the binomial table runs out.
            diagnostic = "Bezier arc of degree " + num_text(poles - 1) +
                         " exceeds the supported span degree";
            return false;
        }
        RationalBezierSpan span;
        span.poles.reserve(static_cast<std::size_t>(poles));
        span.weights.reserve(static_cast<std::size_t>(poles));
        for (int j = 1; j <= poles; ++j) {
            gp_Pnt p = arc->Pole(j);
            if (!identity) p.Transform(trsf);
            span.poles.push_back(p.XYZ());
            span.weights.push_back(arc->IsRational() ? arc->Weight(j) : 1.0);
        }
        span.t0 = to_edge(knots[static_cast<std::size_t>(i - 1)]);
        span.t1 = to_edge(knots[static_cast<std::size_t>(i)]);
        if (!weights_are_positive_and_finite(span)) {
            diagnostic = "non-positive or non-finite Bezier weight — the convex-hull "
                         "bound does not hold for this span";
            return false;
        }
        normalize_weights(span);
        out.push_back(std::move(span));
    }
    return true;
}

// NUM §2.1 asks conic spans to be at most 90 degrees. A rational quadratic arc
// wider than that conditions badly and at 180 degrees has no positive-weight
// form at all, so split until the forward cone holds at 90 degrees.
void pre_split_wide_spans(std::vector<RationalBezierSpan>& spans) {
    for (int round = 0; round < kConicPreSplitRounds; ++round) {
        bool split_any = false;
        std::vector<RationalBezierSpan> next;
        next.reserve(spans.size() * 2);
        for (const RationalBezierSpan& span : spans) {
            if (tangent_cone_status(span, kHalfPi, kAngularTestFloor) ==
                AngularStatus::Uncertified) {
                RationalBezierSpan left, right;
                subdivide_homogeneous(span, left, right);
                next.push_back(std::move(left));
                next.push_back(std::move(right));
                split_any = true;
            } else {
                next.push_back(span);
            }
        }
        spans.swap(next);
        if (!split_any) return;
    }
}

bool is_conic(GeomAbs_CurveType type) {
    return type == GeomAbs_Circle || type == GeomAbs_Ellipse || type == GeomAbs_Hyperbola ||
           type == GeomAbs_Parabola;
}

Handle(Geom_BSplineCurve) clamped_copy(const Handle(Geom_BSplineCurve) & bs) {
    if (bs.IsNull() || !bs->IsPeriodic()) return bs;
    Handle(Geom_BSplineCurve) copy = Handle(Geom_BSplineCurve)::DownCast(bs->Copy());
    copy->SetNotPeriodic();
    return copy;
}

// A conic covering a whole period must still START where the EDGE starts.
// Converting the untrimmed basis throws that phase away, and the endpoints then
// disagree by a chord's worth — which used to delete the edge outright. Two
// trimmed halves keep the phase and the closure identity.
bool conic_pieces(const Handle(Geom_Curve) & base, double u1, double u2,
                  std::vector<BSplinePiece>& out, std::string& diagnostic) {
    std::vector<std::pair<double, double>> intervals;
    if (base->IsPeriodic() && (u2 - u1) >= base->Period() - 1e-9) {
        const double mid = 0.5 * (u1 + u2);
        intervals.emplace_back(u1, mid);
        intervals.emplace_back(mid, u2);
    } else {
        intervals.emplace_back(u1, u2);
    }
    for (const auto& interval : intervals) {
        Handle(Geom_TrimmedCurve) trimmed =
            new Geom_TrimmedCurve(base, interval.first, interval.second);
        Handle(Geom_BSplineCurve) bs = clamped_copy(GeomConvert::CurveToBSplineCurve(trimmed));
        if (bs.IsNull()) {
            diagnostic = "conic to B-spline conversion returned nothing";
            return false;
        }
        // The conic conversion re-parameterises, so the piece's own domain IS
        // the trimmed portion and the leaf map carries it back to edge space.
        out.push_back(BSplinePiece{bs, bs->FirstParameter(), bs->LastParameter(), interval.first,
                                   interval.second});
    }
    return true;
}

// A periodic B-spline whose edge range crosses (or laps) the seam. Unroll into
// period-mapped sub-intervals and convert each; NEVER clamp, which would
// silently drop the wrapped part and — when the dropped overhang is small —
// hand back a truncated polyline labelled Certified.
bool periodic_bspline_pieces(const Handle(Geom_BSplineCurve) & periodic, double u1, double u2,
                             std::vector<BSplinePiece>& out, std::string& diagnostic) {
    Handle(Geom_BSplineCurve) clamped = clamped_copy(periodic);
    const double first = clamped->FirstParameter();
    const double last = clamped->LastParameter();
    const double period = last - first;
    if (!(period > Precision::PConfusion())) {
        diagnostic = "periodic B-spline has a degenerate period";
        return false;
    }

    double offset = std::fmod(u1 - first, period);
    if (offset < 0.0) offset += period;
    double cursor = first + offset;
    double consumed = 0.0;
    const double total = u2 - u1;
    for (int piece = 0; piece < kMaxUnrolledPeriods && consumed < total; ++piece) {
        const double step = std::min(total - consumed, last - cursor);
        if (step > Precision::PConfusion()) {
            out.push_back(
                BSplinePiece{clamped, cursor, cursor + step, u1 + consumed, u1 + consumed + step});
        }
        consumed += std::max(step, 0.0);
        cursor += step;
        if (cursor >= last - Precision::PConfusion()) cursor = first;
    }
    if (out.empty() || consumed < total - Precision::PConfusion()) {
        diagnostic = "periodic B-spline range " + num_text(u1) + ".." + num_text(u2) +
                     " could not be unrolled within " + num_text(kMaxUnrolledPeriods) + " periods";
        return false;
    }
    return true;
}

// Exact families only (NUM §2.1 rows 1-4). Returns false when the family has no
// exact rational Bézier form, which routes the edge to §2.7.
bool exact_pieces(const Handle(Geom_Curve) & base, GeomAbs_CurveType type, double u1, double u2,
                  std::vector<BSplinePiece>& out, std::string& diagnostic) {
    if (base.IsNull()) return false;

    if (base->IsKind(STANDARD_TYPE(Geom_BSplineCurve))) {
        Handle(Geom_BSplineCurve) bs = Handle(Geom_BSplineCurve)::DownCast(base);
        if (bs->IsPeriodic()) return periodic_bspline_pieces(bs, u1, u2, out, diagnostic);
        // A non-periodic domain must CONTAIN the edge range. Clamping instead
        // would drop geometry and then certify the remainder.
        const double slack = Precision::PConfusion() * (1.0 + std::abs(u1) + std::abs(u2));
        if (u1 < bs->FirstParameter() - slack || u2 > bs->LastParameter() + slack) {
            diagnostic = "edge range " + num_text(u1) + ".." + num_text(u2) +
                         " falls outside the B-spline domain " + num_text(bs->FirstParameter()) +
                         ".." + num_text(bs->LastParameter());
            return false;
        }
        const double a = std::clamp(u1, bs->FirstParameter(), bs->LastParameter());
        const double b = std::clamp(u2, bs->FirstParameter(), bs->LastParameter());
        out.push_back(BSplinePiece{bs, a, b, a, b});
        return true;
    }
    if (base->IsKind(STANDARD_TYPE(Geom_BezierCurve))) {
        Handle(Geom_BSplineCurve) bs = GeomConvert::CurveToBSplineCurve(base);
        if (bs.IsNull()) return false;
        const double a = std::clamp(u1, bs->FirstParameter(), bs->LastParameter());
        const double b = std::clamp(u2, bs->FirstParameter(), bs->LastParameter());
        out.push_back(BSplinePiece{bs, a, b, a, b});
        return true;
    }
    if (is_conic(type)) return conic_pieces(base, u1, u2, out, diagnostic);
    return false;
}

SpanSet approximate_span_set(const BRepAdaptor_Curve& curve, double u1, double u2,
                             double chordTolerance) {
    SpanSet set;
    const double allowance = 0.25 * chordTolerance;
    Handle(Adaptor3d_Curve) adaptor = curve.ShallowCopy();
    if (adaptor.IsNull()) {
        set.diagnostic = "no adaptor available for the approximate fallback";
        return set;
    }
    GeomConvert_ApproxCurve approx(adaptor, allowance, GeomAbs_C1, kApproxMaxSegments,
                                   kApproxMaxDegree);
    if (!approx.IsDone() || !approx.HasResult()) {
        set.diagnostic = "GeomConvert_ApproxCurve produced no usable result";
        return set;
    }
    if (!(approx.MaxError() <= allowance)) {
        set.diagnostic = "approximation error " + num_text(approx.MaxError()) +
                         " mm exceeds the quarter-tolerance allowance " + num_text(allowance) +
                         " mm";
        return set;
    }
    Handle(Geom_BSplineCurve) bs = clamped_copy(approx.Curve());
    if (bs.IsNull()) {
        set.diagnostic = "GeomConvert_ApproxCurve returned a null curve";
        return set;
    }
    // §2.7 step 5 measures the approximation against the ORIGINAL curve at the
    // leaf parameters, which is only meaningful while the approximation keeps
    // the edge's domain. Check that rather than assume it.
    const double domain = std::max(1.0, std::abs(u1)) + std::max(1.0, std::abs(u2));
    if (std::abs(bs->FirstParameter() - u1) > 1e-9 * domain ||
        std::abs(bs->LastParameter() - u2) > 1e-9 * domain) {
        set.diagnostic = "GeomConvert_ApproxCurve re-parameterised the edge domain";
        return set;
    }
    // The adaptor already applied the edge location, so these poles are world.
    const gp_Trsf identity_trsf;
    const BSplinePiece piece{bs, bs->FirstParameter(), bs->LastParameter(), u1, u2};
    if (!spans_from_piece(piece, identity_trsf, true, set.spans, set.diagnostic)) {
        set.spans.clear();
        return set;
    }
    set.approximate = true;
    set.ok = !set.spans.empty();
    return set;
}

SpanSet build_source_spans(const BRepAdaptor_Curve& curve, const Handle(Geom_Curve) & base,
                           double u1, double u2, double chordTolerance) {
    SpanSet set;
    const GeomAbs_CurveType type = curve.GetType();
    const gp_Trsf trsf = curve.Trsf();
    const bool identity = trsf.Form() == gp_Identity;

    if (type == GeomAbs_Line) {
        RationalBezierSpan span;
        span.poles = {curve.Value(u1).XYZ(), curve.Value(u2).XYZ()};
        span.weights = {1.0, 1.0};
        span.t0 = u1;
        span.t1 = u2;
        if (!weights_are_positive_and_finite(span)) {
            set.diagnostic = "line endpoints are not finite";
            return set;
        }
        set.spans.push_back(std::move(span));
        set.ok = true;
        return set;
    }

    std::vector<BSplinePiece> pieces;
    try {
        if (exact_pieces(base, type, u1, u2, pieces, set.diagnostic) && !pieces.empty()) {
            bool built = true;
            for (const BSplinePiece& piece : pieces) {
                if (piece.b - piece.a <= Precision::PConfusion()) continue;
                if (!spans_from_piece(piece, trsf, identity, set.spans, set.diagnostic)) {
                    built = false;
                    break;
                }
            }
            if (built && !set.spans.empty()) {
                if (is_conic(type)) pre_split_wide_spans(set.spans);
                set.ok = true;
                return set;
            }
        }
    } catch (const Standard_Failure& failure) {
        set.diagnostic = std::string("exact conversion failed: ") + failure.GetMessageString();
    }

    set.spans.clear();
    const std::string exact_reason = set.diagnostic;
    try {
        set = approximate_span_set(curve, u1, u2, chordTolerance);
    } catch (const Standard_Failure& failure) {
        set = SpanSet{};
        set.diagnostic = std::string("approximate fallback failed: ") + failure.GetMessageString();
    }
    if (!set.ok && !exact_reason.empty()) set.diagnostic = exact_reason + "; " + set.diagnostic;
    return set;
}

// ---------------------------------------------------------------------------
// Subdivision (NUM §2.5)
// ---------------------------------------------------------------------------

struct Acceptance {
    bool accepted = false;
    double chordBound = 0.0;
    double chordLength = 0.0;
    AngularStatus angular = AngularStatus::Uncertified;
    bool singularAtStart = false;
    bool singularAtEnd = false;
    bool representationLimited = false;
};

Acceptance evaluate_span(const RationalBezierSpan& span, double chordTolerance,
                         double callerAllowance, double angularTol) {
    Acceptance verdict;
    verdict.chordBound = hull_chord_bound(span);
    verdict.chordLength = span.poles.back().Subtracted(span.poles.front()).Modulus();

    // The a-priori float32 allowance is per span and per axis. When it alone
    // would exhaust the budget the span is certified in DOUBLE instead — the
    // display is then as good as the output format allows, and the MEASURED
    // quantization decides the final label. Refusing the edge would swap a
    // bounded error for an unbounded one.
    const double rho = span_representation_allowance(span);
    double budget = chordTolerance - callerAllowance - rho;
    if (!(budget > 0.0)) {
        verdict.representationLimited = true;
        budget = chordTolerance - callerAllowance;
    }
    if (!(budget > 0.0)) return verdict;
    // The cone test builds a degree-2n-1 Bernstein product; skip it for a span
    // the chord certificate has already rejected.
    if (!(verdict.chordBound <= budget)) return verdict;

    const ConeEvidence evidence = tangent_cone_evidence(span, angularTol, kAngularTestFloor);
    verdict.angular = evidence.status;
    verdict.singularAtStart = evidence.singularAtStart;
    verdict.singularAtEnd = evidence.singularAtEnd;

    if (evidence.status != AngularStatus::Uncertified) {
        // Satisfied, or Undefined with a certified chord. NUM §2.4 accepts a
        // stationary or unestablished tangent AT A SPAN BOUNDARY as long as the
        // position bound holds — where the curve, not the subdivision, put the
        // singularity, and the caller publishes it as an explicit singular
        // vertex. Gating this on "the boundary of the original source span"
        // would refuse every cusp that subdivision moves onto a child boundary,
        // including a perfectly straight locus with a stationary parameter.
        verdict.accepted = true;
        return verdict;
    }

    // The cone genuinely fails: a turnaround strictly inside the span. Splitting
    // is the answer while it can still help, and for an ordinary curve it always
    // does. A CUSP is different — the tangent reverses at a point, so every span
    // containing it fails at every tolerance and subdivision alone would run to
    // the depth cap (a stationary parameter that is not a dyadic fraction never
    // lands on a boundary). Stop only once the whole span sits below the
    // resolution floor, where it is a vertex rather than a segment, and record
    // it as Undefined — never as Satisfied.
    const double exhausted = std::max(kShortChordQuantizationFactor * rho,
                                      budget * kAngularExhaustionFraction);
    if (verdict.chordLength + verdict.chordBound <= exhausted) {
        verdict.angular = AngularStatus::Undefined;
        verdict.accepted = true;
    }
    return verdict;
}

struct SubdivideOutcome {
    bool limited = false;
    bool cancelled = false;
    std::string diagnostic;
};

void record_leaf(const RationalBezierSpan& span, const Acceptance& verdict,
                 CurveSampleResult& out) {
    CurveLeaf leaf;
    leaf.t0 = span.t0;
    leaf.t1 = span.t1;
    leaf.chordBoundMm = verdict.chordBound;
    leaf.chordLengthMm = verdict.chordLength;
    leaf.angular = verdict.angular;
    leaf.singularVertexAtStart = verdict.singularAtStart;
    leaf.singularVertexAtEnd = verdict.singularAtEnd;
    out.leaves.push_back(leaf);
    if (verdict.angular == AngularStatus::Undefined) out.angularUndefinedSomewhere = true;

    auto note_singular = [&out](double t) {
        for (double known : out.singularParameters) {
            if (std::abs(known - t) <= 1e-12) return;
        }
        out.singularParameters.push_back(t);
    };
    if (verdict.singularAtStart) note_singular(span.t0);
    if (verdict.singularAtEnd) note_singular(span.t1);
}

SubdivideOutcome subdivide_spans(const std::vector<RationalBezierSpan>& sources,
                                 double chordTolerance, double callerAllowance, double angularTol,
                                 const CurveSampleLimits& limits,
                                 const std::function<bool()>& cancelled, CurveSampleResult& out) {
    SubdivideOutcome outcome;
    const std::size_t cap = std::min(limits.maxSegmentsPerEdge, limits.remainingBodySegments);

    std::vector<RationalBezierSpan> stack;
    stack.reserve(sources.size() * 2 + 16);
    for (auto it = sources.rbegin(); it != sources.rend(); ++it) stack.push_back(*it);
    out.points.emplace_back(sources.front().poles.front());

    while (!stack.empty()) {
        if (cancelled && cancelled()) {
            outcome.cancelled = true;
            return outcome;
        }
        const RationalBezierSpan span = std::move(stack.back());
        stack.pop_back();

        const Acceptance verdict = evaluate_span(span, chordTolerance, callerAllowance, angularTol);
        if (verdict.accepted) {
            if (out.leaves.size() >= cap) {
                outcome.limited = true;
                outcome.diagnostic =
                    "segment budget reached (" + num_text(static_cast<double>(cap)) + ")";
                return outcome;
            }
            out.points.emplace_back(span.poles.back());
            record_leaf(span, verdict, out);
            continue;
        }
        if (span.depth >= limits.maxDepthPerSpan) {
            outcome.limited = true;
            outcome.diagnostic = "depth cap " + num_text(limits.maxDepthPerSpan) +
                                 " reached with a chord bound of " + num_text(verdict.chordBound) +
                                 " mm against a tolerance of " + num_text(chordTolerance) + " mm";
            return outcome;
        }
        // Caps are checked BEFORE the split allocates (NUM §2.5): the stack plus
        // the leaves already emitted is a lower bound on the final segment count.
        if (out.leaves.size() + stack.size() + 2 > cap) {
            outcome.limited = true;
            outcome.diagnostic =
                "segment budget reached (" + num_text(static_cast<double>(cap)) + ")";
            return outcome;
        }
        RationalBezierSpan left;
        RationalBezierSpan right;
        subdivide_homogeneous(span, left, right);
        stack.push_back(std::move(right));
        stack.push_back(std::move(left));
    }
    return outcome;
}

// Closure is a SEMANTIC property of the edge, never a distance. A 1e-5 mm line a
// billion millimetres from the origin is short, not closed.
bool edge_is_semantically_closed(const BRepAdaptor_Curve& curve, const Handle(Geom_Curve) & base,
                                 double u1, double u2) {
    try {
        if (BRep_Tool::IsClosed(curve.Edge())) return true;
    } catch (const Standard_Failure&) {
        // fall through to the geometric predicates
    }
    if (base.IsNull()) return false;
    if (base->IsPeriodic() && (u2 - u1) >= base->Period() - 1e-9) return true;
    return base->IsClosed() && (u2 - u1) >= (base->LastParameter() - base->FirstParameter()) - 1e-9;
}

// NUM §2.3: keep the analytic endpoints, so closure and seam identity survive
// the conversion. Any replacement moves a point AFTER its leaf was certified, so
// the displacement is charged to that leaf's bound and re-checked; a snap that
// would break the budget is not taken.
bool snap_analytic_endpoints(const BRepAdaptor_Curve& curve, const Handle(Geom_Curve) & base,
                             double u1, double u2, double budget, CurveSampleResult& out,
                             std::string& diagnostic) {
    const gp_Pnt first = curve.Value(u1);
    const gp_Pnt last = curve.Value(u2);
    if (!is_finite(first) || !is_finite(last)) {
        diagnostic = "the edge's analytic endpoints are not finite";
        return false;
    }
    const double head = first.Distance(out.points.front());
    const double tail = last.Distance(out.points.back());
    if (head > budget || tail > budget) {
        diagnostic = "converted span endpoints disagree with the curve by " +
                     num_text(std::max(head, tail)) + " mm";
        return false;
    }
    if (out.leaves.front().chordBoundMm + head <= budget) {
        out.points.front() = first;
        out.leaves.front().chordBoundMm += head;
    }
    if (out.leaves.back().chordBoundMm + tail <= budget) {
        out.points.back() = last;
        out.leaves.back().chordBoundMm += tail;
    }

    if (edge_is_semantically_closed(curve, base, u1, u2)) {
        const double gap = out.points.front().Distance(out.points.back());
        if (out.leaves.back().chordBoundMm + gap <= budget) {
            out.points.back() = out.points.front();
            out.leaves.back().chordBoundMm += gap;
        }
    }
    return true;
}

// After encoding, a segment whose chord is not comfortably longer than the
// float32 rounding of its own endpoints has a direction made of noise. Withdraw
// the angular claim on it; the position bound is unaffected.
void apply_measured_quantization(CurveSampleResult& out) {
    if (out.points.empty()) return;
    std::vector<double> displacement(out.points.size(), 0.0);
    double worst = 0.0;
    for (std::size_t i = 0; i < out.points.size(); ++i) {
        displacement[i] = measured_quantization(out.points[i]);
        worst = std::max(worst, displacement[i]);
    }
    out.quantizationErrorMm = worst;
    for (std::size_t k = 0; k < out.leaves.size() && k + 1 < out.points.size(); ++k) {
        if (out.leaves[k].angular == AngularStatus::Undefined) continue;
        const double noise = std::max(displacement[k], displacement[k + 1]);
        if (out.leaves[k].chordLengthMm < kShortChordQuantizationFactor * noise) {
            out.leaves[k].angular = AngularStatus::Undefined;
            out.angularUndefinedSomewhere = true;
        }
    }
}

double segment_distance(const gp_Pnt& p, const gp_Pnt& a, const gp_Pnt& b) {
    const gp_XYZ v = b.XYZ().Subtracted(a.XYZ());
    const gp_XYZ w = p.XYZ().Subtracted(a.XYZ());
    const double len2 = v.SquareModulus();
    if (!(len2 > 0.0)) return w.Modulus();
    const double u = std::clamp(w.Dot(v) / len2, 0.0, 1.0);
    return w.Subtracted(v.Multiplied(u)).Modulus();
}

struct IndependentCheck {
    double maxError = 0.0;
    bool complete = true;  // false when the evaluation cap cut the check short
};

// NUM §2.7 step 5. Evidence, not proof: it can falsify a claimed bound but never
// establish one, which is why the caller only ever labels the result
// KernelEstimated. Each sample is measured against the leaf's OWN segment, the
// most conservative reading of "distance to the emitted polyline".
IndependentCheck independent_max_error(const BRepAdaptor_Curve& curve,
                                       const CurveSampleResult& result, double u1, double u2,
                                       double trigger) {
    IndependentCheck check;
    std::size_t used = 0;
    for (std::size_t k = 0; k + 1 < result.points.size() && k < result.leaves.size(); ++k) {
        const CurveLeaf& leaf = result.leaves[k];
        const gp_Pnt& a = result.points[k];
        const gp_Pnt& b = result.points[k + 1];
        const double mid = 0.5 * (leaf.t0 + leaf.t1);
        const double half = 0.5 * (leaf.t1 - leaf.t0);
        int count = 17;
        double leaf_worst = 0.0;
        for (int round = 0; round < 3; ++round) {
            leaf_worst = 0.0;
            for (int i = 0; i < count; ++i) {
                const double frac = static_cast<double>(i) / static_cast<double>(count - 1);
                const double uniform = std::clamp(leaf.t0 + (leaf.t1 - leaf.t0) * frac, u1, u2);
                leaf_worst = std::max(leaf_worst, segment_distance(curve.Value(uniform), a, b));
                if (i + 1 < count) {
                    const double angle = (2.0 * static_cast<double>(i) + 1.0) * kHalfPi /
                                         static_cast<double>(count - 1);
                    const double cheb = std::clamp(mid + half * std::cos(angle), u1, u2);
                    leaf_worst = std::max(leaf_worst, segment_distance(curve.Value(cheb), a, b));
                }
            }
            used += 2U * static_cast<std::size_t>(count);
            if (used > kIndependentSampleCap) {
                check.complete = false;
                break;
            }
            // Refine only where the deviation is already a sizeable part of the
            // budget — that is where a sharp change would matter.
            if (leaf_worst <= 0.25 * trigger) break;
            count = 4 * (count - 1) + 1;
        }
        check.maxError = std::max(check.maxError, leaf_worst);
        if (!check.complete) break;
    }
    return check;
}

CurveSampleResult failure(const std::string& reason) {
    CurveSampleResult out;
    out.certification = CurveCertification::Failed;
    out.diagnostic = reason;
    return out;
}

}  // namespace

// ---------------------------------------------------------------------------

CurveSampleResult sample_edge_curve(const BRepAdaptor_Curve& curve,
                                    const CurveSampleRequest& request,
                                    const CurveSampleLimits& limits,
                                    const std::function<bool()>& cancelled) {
    const double tolerance = request.chordToleranceMm;
    const double angular = request.angularToleranceRad;
    const double allowance = request.representationAllowanceMm;
    if (!(std::isfinite(tolerance) && tolerance > 0.0) ||
        !(std::isfinite(angular) && angular > 0.0) ||
        !(std::isfinite(allowance) && allowance >= 0.0) || allowance >= tolerance ||
        limits.maxDepthPerSpan < 1 || limits.maxSegmentsPerEdge < 1) {
        return failure("invalid sampling request or limits");
    }
    if (limits.remainingBodySegments < 1) {
        CurveSampleResult exhausted;
        exhausted.certification = CurveCertification::QualityLimited;
        exhausted.diagnostic = "the per-body segment budget is exhausted";
        return exhausted;
    }

    CurveSampleResult out;
    try {
        const double u1 = curve.FirstParameter();
        const double u2 = curve.LastParameter();
        if (!std::isfinite(u1) || !std::isfinite(u2) || !(u2 > u1) ||
            Precision::IsInfinite(std::abs(u1)) || Precision::IsInfinite(std::abs(u2))) {
            return failure("edge has a non-finite, unbounded or empty parameter range");
        }

        Handle(Geom_Curve) base;
        if (curve.Is3DCurve()) base = curve.GeomCurve();
        while (!base.IsNull() && base->IsKind(STANDARD_TYPE(Geom_TrimmedCurve))) {
            base = Handle(Geom_TrimmedCurve)::DownCast(base)->BasisCurve();
        }

        const SpanSet source = build_source_spans(curve, base, u1, u2, tolerance);
        if (!source.ok || source.spans.empty()) {
            return failure(source.diagnostic.empty()
                               ? std::string("no exact or approximate span set for this curve")
                               : source.diagnostic);
        }

        // §2.7 step 4: the fallback already spent a quarter of the budget on the
        // approximation, so the hull algorithm gets the remaining three quarters.
        const double chordBudget = source.approximate ? 0.75 * tolerance : tolerance;
        const SubdivideOutcome outcome =
            subdivide_spans(source.spans, chordBudget, allowance, angular, limits, cancelled, out);
        if (outcome.cancelled) return failure("cancelled");
        if (outcome.limited) {
            // A WORK cap publishes nothing: there is no complete polyline, and a
            // straight chord in its place would be a lie (header contract).
            CurveSampleResult limited;
            limited.certification = CurveCertification::QualityLimited;
            limited.diagnostic = outcome.diagnostic;
            return limited;
        }
        if (out.points.size() < 2 || out.leaves.empty()) return failure("no segment emitted");

        std::string reason;
        if (!snap_analytic_endpoints(curve, base, u1, u2, chordBudget - allowance, out, reason)) {
            return failure(reason);
        }
        for (const gp_Pnt& p : out.points) {
            if (!is_finite(p)) return failure("the sampled polyline contains a non-finite point");
        }

        double worst = 0.0;
        for (const CurveLeaf& leaf : out.leaves) worst = std::max(worst, leaf.chordBoundMm);
        apply_measured_quantization(out);

        if (source.approximate) {
            const IndependentCheck check = independent_max_error(curve, out, u1, u2, tolerance);
            out.sampledMaxErrorMm = check.maxError;
            if (!(check.maxError + out.quantizationErrorMm <= tolerance)) {
                return failure("approximate fallback rejected: independent sampled error " +
                               num_text(check.maxError) + " mm plus measured quantization " +
                               num_text(out.quantizationErrorMm) + " mm exceeds the requested " +
                               num_text(tolerance) + " mm");
            }
            out.certification = CurveCertification::KernelEstimated;
            out.diagnostic = check.complete
                                 ? "GeomConvert_ApproxCurve fallback: sampled evidence only"
                                 : "GeomConvert_ApproxCurve fallback: sampled evidence only, and "
                                   "the independent check stopped at its evaluation cap";
            return out;
        }

        if (worst + allowance + out.quantizationErrorMm <= tolerance) {
            out.certifiedChordBoundMm = worst;
            out.certification = CurveCertification::Certified;
            return out;
        }
        // The geometry is certified in double and the polyline is the best this
        // output format can carry. Publish it and say so (header contract:
        // QualityLimited WITH points means representation-limited, not capped).
        out.certification = CurveCertification::QualityLimited;
        out.certifiedChordBoundMm = -1;
        out.diagnostic = "float32 representation: measured quantization " +
                         num_text(out.quantizationErrorMm) + " mm plus the chord bound " +
                         num_text(worst) + " mm exceeds the requested " + num_text(tolerance) +
                         " mm";
        return out;
    } catch (const Standard_Failure& failed) {
        return failure(std::string("OCCT failure: ") + failed.GetMessageString());
    }
}

}  // namespace onecad::tess
