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
#include <TopExp.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Vertex.hxx>
#include <gp_Trsf.hxx>

namespace onecad::tess {

namespace {

// NUM §2.2. Poor weight conditioning cannot corrupt the CHORD certificate — the
// convex-hull property holds for any positive weights and this sampler never
// evaluates the rational curve, only its poles. It can corrupt the derivative
// numerator, so below this ratio the ANGULAR claim is withdrawn. Individual
// weights are NEVER clamped, and conditioning never vetoes a certified chord.
constexpr double kWeightConditionFloor = 1e-12;

// The unit roundoff of IEEE binary64, u = 2^-53 (followup §2 F6).
constexpr double kUnitRoundoff = 0x1p-53;

// The POSITIONAL RESOLUTION FLOOR at which an unresolved region stops being
// worth refining: a span shorter than a thousandth of the display budget is a
// vertex, not a segment. followup §2 C1 is emphatic about what this is NOT — it
// is not an angular guarantee and it never accepts a REGULAR join. It only
// bounds a region where a singularity has ALREADY been established
// independently (the derivative-numerator coefficients cannot be separated from
// the origin, so Q may vanish inside), which is the one case where no amount of
// subdivision can make the cone hold. Used alone it admitted a 0.05 mm
// quarter-circle as one chord with 45-degree turns.
constexpr double kAngularExhaustionFraction = 1.0 / 1024.0;

// followup §2 C1: how many times a single leaf may be split because the EMITTED
// turn at its incoming join is too wide. Splitting shortens the chord, which
// eventually makes the encoded direction worse rather than better, so the
// attempt is bounded; past it the turn is reported, not hidden.
constexpr int kTurnSplitAttempts = 12;

constexpr double kPi = 3.14159265358979323846;
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
//
// `magnitudes` (optional) receives the SAME sum run on absolute values, using
// per-term magnitude bounds `aMag`/`bMag` rather than the signed values. That is
// the quantity the standard gamma_K bound multiplies, so it carries the
// cancellation the signed sum hides — translating a quadratic far from the
// origin cancels most of Q while leaving the terms large.
std::vector<gp_XYZ> bern_product(const std::vector<gp_XYZ>& a, const std::vector<double>& b,
                                 const std::vector<double>* aMag = nullptr,
                                 const std::vector<double>* bMag = nullptr,
                                 std::vector<double>* magnitudes = nullptr) {
    if (a.empty() || b.empty()) return {};
    const int m = static_cast<int>(a.size()) - 1;
    const int n = static_cast<int>(b.size()) - 1;
    std::vector<gp_XYZ> out(static_cast<std::size_t>(m + n + 1));
    if (magnitudes) magnitudes->assign(static_cast<std::size_t>(m + n + 1), 0.0);
    for (int k = 0; k <= m + n; ++k) {
        gp_XYZ acc(0.0, 0.0, 0.0);
        double mag = 0.0;
        const int lo = std::max(0, k - n);
        const int hi = std::min(m, k);
        for (int i = lo; i <= hi; ++i) {
            const std::size_t ui = static_cast<std::size_t>(i);
            const std::size_t uj = static_cast<std::size_t>(k - i);
            const double c = binom(m, i) * binom(n, k - i) / binom(m + n, k);
            acc.Add(a[ui].Multiplied(c * b[uj]));
            if (magnitudes) {
                const double am = aMag ? (*aMag)[ui] : a[ui].Modulus();
                const double bm = bMag ? (*bMag)[uj] : std::abs(b[uj]);
                mag += c * am * bm;
            }
        }
        out[static_cast<std::size_t>(k)] = acc;
        if (magnitudes) (*magnitudes)[static_cast<std::size_t>(k)] = mag;
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
// followup §2 F6: the DIVISION itself can underflow a legal weight to zero —
// (1e-300, 1e300, 1e300) normalizes to (0, 1, 1) — which then dehomogenizes to a
// NaN pole two subdivisions later. The scaling is abandoned rather than allowed
// to destroy the net; `control_net_is_valid` still guards what follows.
void normalize_weights(RationalBezierSpan& span) {
    double hi = 0.0;
    for (double w : span.weights) hi = std::max(hi, w);
    if (!(hi > 0.0) || hi == 1.0) return;
    std::vector<double> scaled(span.weights.size());
    for (std::size_t i = 0; i < span.weights.size(); ++i) {
        scaled[i] = span.weights[i] / hi;
        if (!std::isfinite(scaled[i]) || !(scaled[i] > 0.0)) return;
    }
    span.weights.swap(scaled);
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

// float32 spacing at |v|. PER AXIS, because a coordinate that is exactly
// representable (1e9 = 1953125 * 2^9) costs nothing however large it is, while
// a blanket bound taken from the largest axis would starve the other two.
double float32_ulp(double v) {
    const float f = std::abs(static_cast<float>(v));
    if (!(f > 0.0F) || !std::isfinite(f)) return 0.0;
    return static_cast<double>(std::nextafterf(f, std::numeric_limits<float>::infinity()) - f);
}

// The per-axis greatest |coordinate| of a net, used both for the float32
// allowance and for the F6 roundoff enclosure of everything descended from it.
gp_XYZ pole_extents(const RationalBezierSpan& span) {
    double mx = 0.0, my = 0.0, mz = 0.0;
    for (const gp_XYZ& p : span.poles) {
        mx = std::max(mx, std::abs(p.X()));
        my = std::max(my, std::abs(p.Y()));
        mz = std::max(mz, std::abs(p.Z()));
    }
    return gp_XYZ(mx, my, mz);
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

// The point as MESH1 actually writes it: three float32 world coordinates.
gp_XYZ encode32(const gp_XYZ& p) {
    return gp_XYZ(static_cast<double>(static_cast<float>(p.X())),
                  static_cast<double>(static_cast<float>(p.Y())),
                  static_cast<double>(static_cast<float>(p.Z())));
}

gp_XYZ encode32(const gp_Pnt& p) { return encode32(p.XYZ()); }

// The MEASURED float32 rounding of one point (NUM §1.2: measure, do not guess).
double measured_quantization(const gp_Pnt& p) {
    return encode32(p).Subtracted(p.XYZ()).Modulus();
}

// The angle between two vectors, or -1 when either has no direction. Via
// atan2(|a x b|, a.b) rather than acos: near-parallel vectors are exactly where
// most of these angles live, and acos(1 - eps) loses half its significant digits
// there.
double vector_angle(const gp_XYZ& a, const gp_XYZ& b) {
    const double la = a.Modulus();
    const double lb = b.Modulus();
    if (!(la > 0.0) || !(lb > 0.0) || !std::isfinite(la) || !std::isfinite(lb)) return -1.0;
    return std::atan2(a.Crossed(b).Modulus(), a.Dot(b));
}

// gamma_k = k*u / (1 - k*u), the standard accumulated-rounding factor for k
// dependent floating-point operations. Infinite once k*u reaches 1, where the
// bound stops meaning anything.
double gamma_k(int k) {
    if (k <= 0) return 0.0;
    const double ku = static_cast<double>(k) * kUnitRoundoff;
    if (!(ku < 1.0)) return std::numeric_limits<double>::infinity();
    return ku / (1.0 - ku);
}

}  // namespace

bool control_net_is_valid(const RationalBezierSpan& span) {
    if (span.poles.empty() || span.poles.size() != span.weights.size()) return false;
    for (std::size_t i = 0; i < span.weights.size(); ++i) {
        // The homogeneous denominator must be STRICTLY positive and finite: the
        // convex-hull argument of NUM §2.3 needs w(t) > 0 across the span, and a
        // zero denominator dehomogenizes to a non-finite pole.
        if (!std::isfinite(span.weights[i]) || !(span.weights[i] > 0.0)) return false;
        if (!std::isfinite(span.poles[i].X()) || !std::isfinite(span.poles[i].Y()) ||
            !std::isfinite(span.poles[i].Z())) {
            return false;
        }
    }
    return true;
}

double pole_roundoff_multiplier(int depth, int degree) {
    if (depth < 0 || degree < 0) return std::numeric_limits<double>::infinity();
    const long long k = static_cast<long long>(depth) * (static_cast<long long>(degree) + 3) + 2;
    if (k > 1000000000LL) return std::numeric_limits<double>::infinity();
    const double g = gamma_k(static_cast<int>(k));
    if (!std::isfinite(g) || !(g < 1.0)) return std::numeric_limits<double>::infinity();
    return (2.0 * g + kUnitRoundoff * (1.0 + g)) / (1.0 - g);
}

double pole_roundoff_enclosure(const RationalBezierSpan& span) {
    if (span.poles.empty()) return 0.0;
    // The recurrence handled the ANCESTOR's magnitudes; a hand-built span has
    // none recorded, so fall back to this span's own extents. Never smaller than
    // the extent of the net actually in hand.
    double mx = std::abs(span.sourceMaxAbs.X());
    double my = std::abs(span.sourceMaxAbs.Y());
    double mz = std::abs(span.sourceMaxAbs.Z());
    for (const gp_XYZ& p : span.poles) {
        mx = std::max(mx, std::abs(p.X()));
        my = std::max(my, std::abs(p.Y()));
        mz = std::max(mz, std::abs(p.Z()));
    }
    const double mult =
        pole_roundoff_multiplier(span.depth, static_cast<int>(span.poles.size()) - 1);
    if (!std::isfinite(mult)) return std::numeric_limits<double>::infinity();
    const double magnitude = std::sqrt(mx * mx + my * my + mz * mz);
    if (!std::isfinite(magnitude)) return std::numeric_limits<double>::infinity();
    return mult * magnitude;
}

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
    // F6: the roundoff enclosure of a child is set by the magnitudes its
    // ancestors' recurrence handled, so the source extents travel down.
    left.sourceMaxAbs = span.sourceMaxAbs;
    right.sourceMaxAbs = span.sourceMaxAbs;
    normalize_weights(left);
    normalize_weights(right);
}

double hull_chord_bound(const RationalBezierSpan& span) {
    if (span.poles.size() < 2) return 0.0;
    // followup §2 F6: `std::max(worst, NaN)` returns `worst`, so a single
    // non-finite pole used to hand back a ZERO bound for a net that bounds
    // nothing. An invalid net has no hull certificate at all.
    if (!control_net_is_valid(span)) return std::numeric_limits<double>::infinity();
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
        if (!std::isfinite(d)) return std::numeric_limits<double>::infinity();
        worst = std::max(worst, d);
    }
    return worst;
}

namespace {

// Bernstein coefficients of the derivative numerator together with a per-
// coefficient ABSOLUTE arithmetic uncertainty (followup §2 C2's `E`).
//
// The uncertainty is `gamma_K` times the SAME Bernstein sums run on absolute
// values. That magnitude is what the standard accumulated-rounding bound
// multiplies, and running it on absolute values is what makes it see the
// cancellation the signed sum hides: Q = X'W - XW' for a curve with a constant
// coordinate M cancels that coordinate exactly, so the computed coefficient is
// ~0 while the terms behind it are ~M. K counts the roundings on the chain from
// the input poles and weights to one coefficient — one for w_i*P_i, two for the
// scaled difference, three for the binomial ratio, two per product term, at most
// 2n accumulations and one final subtraction, i.e. at most 2n + 9. K = 3n + 16
// keeps headroom over that count.
struct DerivativeNumerator {
    std::vector<gp_XYZ> q;
    std::vector<double> uncertainty;
};

DerivativeNumerator derivative_numerator_with_uncertainty(const RationalBezierSpan& span) {
    DerivativeNumerator out;
    if (span.poles.size() < 2 || span.poles.size() != span.weights.size()) return out;
    const int n = static_cast<int>(span.poles.size()) - 1;
    const std::size_t count = span.poles.size();

    std::vector<gp_XYZ> x(count);
    std::vector<double> w(count);
    std::vector<double> xMag(count);
    std::vector<double> wMag(count);
    for (std::size_t i = 0; i < count; ++i) {
        x[i] = span.poles[i].Multiplied(span.weights[i]);
        w[i] = span.weights[i];
        xMag[i] = span.poles[i].Modulus() * std::abs(span.weights[i]);
        wMag[i] = std::abs(w[i]);
    }
    std::vector<gp_XYZ> dx(count - 1);
    std::vector<double> dw(count - 1);
    std::vector<double> dxMag(count - 1);
    std::vector<double> dwMag(count - 1);
    for (std::size_t i = 0; i + 1 < count; ++i) {
        dx[i] = x[i + 1].Subtracted(x[i]).Multiplied(static_cast<double>(n));
        dw[i] = (w[i + 1] - w[i]) * static_cast<double>(n);
        // A DIFFERENCE carries the magnitudes of both operands, not of its own
        // (possibly cancelled) value.
        dxMag[i] = static_cast<double>(n) * (xMag[i + 1] + xMag[i]);
        dwMag[i] = static_cast<double>(n) * (wMag[i + 1] + wMag[i]);
    }

    std::vector<double> lhsMag;
    std::vector<double> rhsMag;
    const std::vector<gp_XYZ> lhs = bern_product(dx, w, &dxMag, &wMag, &lhsMag);  // X'W
    const std::vector<gp_XYZ> rhs = bern_product(x, dw, &xMag, &dwMag, &rhsMag);  // XW'
    const double gamma = gamma_k(3 * n + 16);
    out.q.resize(lhs.size());
    out.uncertainty.resize(lhs.size());
    for (std::size_t k = 0; k < lhs.size(); ++k) {
        out.q[k] = lhs[k].Subtracted(rhs[k]);
        out.uncertainty[k] = gamma * (lhsMag[k] + rhsMag[k]);
    }
    return out;
}

// max_t B_k^m(t) = B_k^m(k/m), the greatest share of the Bernstein partition of
// unity one basis function can take. Summing it over the EXCLUDED indices gives a
// sound lower bound on the retained mass s_min = min_t sum_{retained} B_k(t).
double bernstein_peak(int m, int k) {
    if (m <= 0) return 1.0;
    if (k <= 0 || k >= m) return 1.0;
    const double t = static_cast<double>(k) / static_cast<double>(m);
    return binom(m, k) * std::pow(t, k) * std::pow(1.0 - t, m - k);
}

}  // namespace

std::vector<gp_XYZ> derivative_numerator_bernstein(const RationalBezierSpan& span) {
    return derivative_numerator_with_uncertainty(span).q;
}

ConeVerdict tangent_cone_verdict(const RationalBezierSpan& span, double angularTol,
                                 const gp_XYZ& axis) {
    ConeVerdict verdict;
    verdict.uncertaintyRad = kPi;  // "nothing established" until the bound closes
    const DerivativeNumerator dn = derivative_numerator_with_uncertainty(span);
    const std::vector<gp_XYZ>& q = dn.q;
    if (q.empty()) {
        verdict.singularAtStart = verdict.singularAtEnd = true;
        verdict.status = AngularStatus::Undefined;
        verdict.incompleteReason = "the span carries no derivative numerator";
        return verdict;
    }
    const int m = static_cast<int>(q.size()) - 1;

    double qmax = 0.0;
    for (std::size_t k = 0; k < q.size(); ++k) {
        if (!is_finite(q[k]) || !std::isfinite(dn.uncertainty[k])) {
            verdict.status = AngularStatus::Undefined;
            verdict.incompleteReason = "the derivative numerator is not finite";
            return verdict;
        }
        qmax = std::max(qmax, q[k].Modulus());
    }
    if (!(qmax > 0.0)) {
        // Q vanishes identically: the span is a single point, not a direction.
        verdict.singularAtStart = verdict.singularAtEnd = true;
        verdict.status = AngularStatus::Undefined;
        verdict.incompleteReason = "the derivative numerator vanishes identically";
        return verdict;
    }
    // Q(0) and Q(1) are the first and last Bernstein coefficients, and they ARE
    // the curve's one-sided tangents there up to the positive factor W^2. Read
    // them out before anything can return early: the display-turn rule needs
    // them at every join, including joins whose cone did not close.
    auto unit_or_zero = [](const gp_XYZ& v) {
        const double len = v.Modulus();
        if (!(len > 0.0) || !std::isfinite(len)) return gp_XYZ(0.0, 0.0, 0.0);
        return v.Multiplied(1.0 / len);
    };
    verdict.startDirection = unit_or_zero(q.front());
    verdict.endDirection = unit_or_zero(q.back());

    const double axisLength = axis.Modulus();
    if (!(axisLength > 0.0) || !std::isfinite(axisLength)) {
        // No direction to test against — a closed span, or two coincident
        // emitted endpoints. The caller decides whether that is worth splitting.
        verdict.status = AngularStatus::Uncertified;
        verdict.incompleteReason = "the span has no chord direction";
        return verdict;
    }
    const gp_XYZ d = axis.Multiplied(1.0 / axisLength);

    double beta = 0.0;
    double pMin = std::numeric_limits<double>::infinity();
    double excludedPeak = 0.0;
    double excludedNorm = 0.0;
    double worstUncertainty = 0.0;
    bool anyRetained = false;
    for (std::size_t k = 0; k < q.size(); ++k) {
        worstUncertainty = std::max(worstUncertainty, dn.uncertainty[k] / qmax);
        if (q[k].Modulus() == 0.0) {
            // followup §2 C2: ONLY an exactly zero coefficient may leave the
            // retained set, and even then its uncertainty is still charged
            // through E — a computed floating-point zero is not a proved zero.
            // At an END it IS Q(0) or Q(1), so no tangent exists there.
            if (k == 0) verdict.singularAtStart = true;
            if (static_cast<int>(k) == m) verdict.singularAtEnd = true;
            excludedPeak += bernstein_peak(m, static_cast<int>(k));
            continue;
        }
        anyRetained = true;
        const gp_XYZ scaled = q[k].Multiplied(1.0 / qmax);
        const double parallel = scaled.Dot(d);
        const double perpendicular = scaled.Subtracted(d.Multiplied(parallel)).Modulus();
        beta = std::max(beta, std::atan2(perpendicular, parallel));
        if (parallel > 0.0) {
            pMin = std::min(pMin, parallel);
        } else {
            // No half-space about `d` holds every coefficient, so the coefficient
            // hull — which CONTAINS Q over the whole span — may contain the
            // origin. Necessary for a singularity, never sufficient.
            verdict.reversalPresent = true;
        }
    }
    verdict.coneHalfAngleRad = beta;
    verdict.excludedMass = excludedNorm + worstUncertainty;
    if (!anyRetained) {
        verdict.status = AngularStatus::Undefined;
        verdict.incompleteReason = "every derivative-numerator coefficient is exactly zero";
        return verdict;
    }
    if (verdict.reversalPresent) {
        verdict.status = AngularStatus::Uncertified;
        return verdict;
    }
    if (beta > 0.5 * angularTol) {
        // DISPROVED about this axis: the cone genuinely does not hold.
        verdict.status = AngularStatus::Uncertified;
        return verdict;
    }

    const double sMin = 1.0 - excludedPeak;
    verdict.retainedMassMin = sMin;
    const double gap = pMin * sMin;
    verdict.retainedGap = gap;
    if (!(sMin > 0.0)) {
        verdict.status = AngularStatus::Undefined;
        verdict.incompleteReason = "the retained Bernstein mass vanishes on this span";
        return verdict;
    }
    if (!(verdict.excludedMass < gap)) {
        verdict.status = AngularStatus::Undefined;
        verdict.incompleteReason = "coefficient uncertainty reaches the retained cone gap";
        return verdict;
    }
    if (weight_condition(span) < kWeightConditionFloor) {
        verdict.status = AngularStatus::Undefined;
        verdict.incompleteReason = "weight conditioning below the numerator's usable range";
        return verdict;
    }
    verdict.uncertaintyRad = std::asin(std::clamp(verdict.excludedMass / gap, 0.0, 1.0));
    verdict.marginRad = 0.5 * angularTol - (beta + verdict.uncertaintyRad);
    if (!(verdict.marginRad >= 0.0)) {
        verdict.status = AngularStatus::Undefined;
        verdict.incompleteReason = "the retained cone already spends the angular allowance";
        return verdict;
    }
    verdict.status = AngularStatus::Satisfied;
    return verdict;
}

AngularStatus tangent_cone_status(const RationalBezierSpan& span, double angularTol,
                                  bool& undefinedAtStart, bool& undefinedAtEnd) {
    gp_XYZ axis(0.0, 0.0, 0.0);
    if (span.poles.size() >= 2) axis = span.poles.back().Subtracted(span.poles.front());
    const ConeVerdict verdict = tangent_cone_verdict(span, angularTol, axis);
    undefinedAtStart = verdict.singularAtStart;
    undefinedAtEnd = verdict.singularAtEnd;
    return verdict.status;
}

AngularStatus tangent_cone_status(const RationalBezierSpan& span, double angularTol) {
    bool start = false;
    bool end = false;
    return tangent_cone_status(span, angularTol, start, end);
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
        if (!control_net_is_valid(span)) {
            diagnostic = "non-positive or non-finite Bezier weight — the convex-hull "
                         "bound does not hold for this span";
            return false;
        }
        normalize_weights(span);
        // followup §2 F6: re-validate AFTER scaling. The division itself can
        // underflow a legal weight — (1e-300, 1e300, 1e300) normalises to
        // (0, 1, 1) — and everything downstream, the hull bound included, is
        // meaningless once a homogeneous denominator reaches zero.
        if (!control_net_is_valid(span)) {
            diagnostic = "weight normalisation underflowed a homogeneous denominator to zero "
                         "for this span";
            return false;
        }
        span.sourceMaxAbs = pole_extents(span);
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
            if (tangent_cone_status(span, kHalfPi) == AngularStatus::Uncertified) {
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
        if (!control_net_is_valid(span)) {
            set.diagnostic = "line endpoints are not finite";
            return set;
        }
        span.sourceMaxAbs = pole_extents(span);
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
    double encodedChordRotation = 0.0;
    double angularMargin = -1.0;
    AngularStatus angular = AngularStatus::Uncertified;
    // The GEOMETRIC verdict, about the double-precision chord. When this is
    // Satisfied and the EMITTED verdict is not, the shortfall is float32 encoding
    // noise and subdividing makes it worse, not better (followup §2 F5).
    AngularStatus geometricAngular = AngularStatus::Uncertified;
    bool singularAtStart = false;
    bool singularAtEnd = false;
    bool singularBracket = false;
    gp_XYZ startTangent = gp_XYZ(0.0, 0.0, 0.0);
    gp_XYZ endTangent = gp_XYZ(0.0, 0.0, 0.0);
    bool representationLimited = false;
    bool invalidNet = false;
    std::string reason;
};

// The emitted chord of a span: the two endpoints exactly as MESH1 writes them.
gp_XYZ encoded_chord(const RationalBezierSpan& span) {
    if (span.poles.size() < 2) return gp_XYZ(0.0, 0.0, 0.0);
    return encode32(span.poles.back()).Subtracted(encode32(span.poles.front()));
}

Acceptance evaluate_span(const RationalBezierSpan& span, double chordTolerance,
                         double callerAllowance, double angularTol) {
    Acceptance verdict;
    if (!control_net_is_valid(span)) {
        // followup §2 F6: an invalid net certifies nothing. It must never reach
        // the hull bound, where a NaN pole silently produced a bound of zero.
        verdict.invalidNet = true;
        verdict.reason = "the generated control net is not finite with positive weights";
        return verdict;
    }
    verdict.chordBound = hull_chord_bound(span);
    verdict.chordLength = span.poles.back().Subtracted(span.poles.front()).Modulus();

    // followup §2 F6: the poles this bound is computed from are themselves
    // computed, so the enclosure of their roundoff is charged to the certificate.
    // Twice, because the chord the distance is measured to is built from two of
    // those same uncertain poles.
    const double enclosure = pole_roundoff_enclosure(span);
    if (!std::isfinite(enclosure)) {
        verdict.invalidNet = true;
        verdict.reason = "the control net's roundoff enclosure is unbounded at this depth";
        return verdict;
    }
    verdict.chordBound += 2.0 * enclosure;

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

    // followup §2 F5: TWO cones. The geometric one, about the double chord,
    // decides whether the CURVE needs refining. The emitted one, about the
    // float32 chord a viewer actually sees, decides whether the angular claim
    // may be published. A straight leaf whose encoded chord rotates 7 degrees
    // fails the second and passes the first: splitting it would shorten the
    // chord and rotate it further, so it is published with the claim withdrawn,
    // never refused and never silently kept.
    const gp_XYZ doubleChord = span.poles.back().Subtracted(span.poles.front());
    const ConeVerdict geometric = tangent_cone_verdict(span, angularTol, doubleChord);
    verdict.geometricAngular = geometric.status;
    verdict.singularAtStart = geometric.singularAtStart;
    verdict.singularAtEnd = geometric.singularAtEnd;
    verdict.startTangent = geometric.startDirection;
    verdict.endTangent = geometric.endDirection;

    const gp_XYZ emittedChord = encoded_chord(span);
    const double rotation = vector_angle(doubleChord, emittedChord);
    verdict.encodedChordRotation = rotation >= 0.0 ? rotation : kPi;

    if (geometric.status == AngularStatus::Uncertified) {
        // The cone genuinely fails about the curve's own chord. Splitting is the
        // answer while it can help — and for an ordinary curve it always can.
        // It cannot at a SINGULARITY, where the tangent reverses at a point and
        // every containing span fails at every tolerance. followup §2 C1 is
        // explicit that neither the cone failure nor the span's small size may
        // authorise stopping on its own: the exemption needs BOTH the
        // independently established bracket (no tested direction separates the
        // coefficient hull from the origin, so Q may vanish inside) AND a span
        // already below the positional resolution floor, where it is a vertex
        // rather than a segment. A regular corner — a 0.05 mm quarter circle
        // between two straights — has a separating direction and is refined.
        const double floor = budget * kAngularExhaustionFraction;
        if (geometric.reversalPresent && verdict.chordLength + verdict.chordBound <= floor) {
            verdict.angular = AngularStatus::Undefined;
            verdict.singularBracket = true;
            verdict.accepted = true;
            verdict.reason = "unresolved singular region below the positional resolution floor";
        }
        return verdict;
    }

    if (!(emittedChord.Modulus() > 0.0)) {
        // The two endpoints encode to the same float32 point: the emitted
        // segment has no direction at all.
        verdict.angular = AngularStatus::Undefined;
        verdict.angularMargin = -1.0;
        verdict.accepted = true;
        verdict.reason = "the emitted float32 endpoints coincide";
        return verdict;
    }

    const ConeVerdict emitted = tangent_cone_verdict(span, angularTol, emittedChord);
    verdict.angular = emitted.status == AngularStatus::Uncertified ? AngularStatus::Undefined
                                                                  : emitted.status;
    verdict.angularMargin = emitted.marginRad;
    if (verdict.angular == AngularStatus::Undefined && emitted.incompleteReason != nullptr) {
        verdict.reason = emitted.incompleteReason;
    }
    verdict.accepted = true;
    return verdict;
}

struct SubdivideOutcome {
    bool limited = false;
    bool cancelled = false;
    bool invalidNet = false;
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
    leaf.angularMarginRad = verdict.angularMargin;
    leaf.encodedChordRotationRad = verdict.encodedChordRotation;
    leaf.singularVertexAtStart = verdict.singularAtStart;
    leaf.singularVertexAtEnd = verdict.singularAtEnd;
    leaf.singularBracket = verdict.singularBracket;
    leaf.startTangent = verdict.startTangent;
    leaf.endTangent = verdict.endTangent;
    out.leaves.push_back(leaf);
}

// followup §2 C1: which joins the display-turn rule does NOT bind. A join is
// exempt only where a singularity is established INDEPENDENTLY of the turn
// itself — an exactly vanishing derivative numerator at that very parameter, or
// an unresolved singular bracket on one side. A failed cone, a short leaf, or
// incomplete arithmetic evidence authorises nothing.
bool join_is_singular(const CurveLeaf& before, const CurveLeaf& after) {
    return before.singularVertexAtEnd || after.singularVertexAtStart || before.singularBracket ||
           after.singularBracket;
}

// followup §2 C1: the curve's OWN tangent jump J at a join, measured from the two
// one-sided analytic tangents that meet there. Zero at a smooth join (a
// subdivision boundary shares its split point, so both sides read the same Q),
// the real corner angle at a C0 knot, and undefined where either side has no
// tangent — in which case the join is already exempt.
double genuine_tangent_jump(const CurveLeaf& before, const CurveLeaf& after) {
    const double jump = vector_angle(before.endTangent, after.startTangent);
    return jump >= 0.0 ? jump : 0.0;
}

// The turn between two consecutive EMITTED chords, or -1 when either encodes to
// zero length.
double encoded_turn(const gp_Pnt& a, const gp_Pnt& b, const gp_Pnt& c) {
    const gp_XYZ ea = encode32(b).Subtracted(encode32(a));
    const gp_XYZ eb = encode32(c).Subtracted(encode32(b));
    return vector_angle(ea, eb);
}

SubdivideOutcome subdivide_spans(const std::vector<RationalBezierSpan>& sources,
                                 double chordTolerance, double callerAllowance, double angularTol,
                                 const CurveSampleLimits& limits,
                                 const std::function<bool()>& cancelled, CurveSampleResult& out,
                                 RationalBezierSpan& firstSpan, RationalBezierSpan& lastSpan) {
    SubdivideOutcome outcome;
    const std::size_t cap = std::min(limits.maxSegmentsPerEdge, limits.remainingBodySegments);

    std::vector<RationalBezierSpan> stack;
    stack.reserve(sources.size() * 2 + 16);
    for (auto it = sources.rbegin(); it != sources.rend(); ++it) stack.push_back(*it);
    out.points.emplace_back(sources.front().poles.front());

    // How many times the CURRENT span has been split because its incoming join
    // turned too far. Bounded: shortening a chord eventually rotates its encoded
    // direction more, not less.
    int turnSplits = 0;
    while (!stack.empty()) {
        if (cancelled && cancelled()) {
            outcome.cancelled = true;
            return outcome;
        }
        const RationalBezierSpan span = std::move(stack.back());
        stack.pop_back();

        Acceptance verdict = evaluate_span(span, chordTolerance, callerAllowance, angularTol);
        if (verdict.invalidNet) {
            outcome.invalidNet = true;
            outcome.diagnostic = verdict.reason;
            return outcome;
        }
        const double outerBudget = std::max(chordTolerance - callerAllowance, 0.0);
        const double floor = outerBudget * kAngularExhaustionFraction;

        // followup §2 C1: the display turn between the EMITTED chords is the
        // acceptance rule, not a by-product of the cone. Where the cone closed on
        // both sides the turn is within tolerance by construction, so this only
        // ever bites at a join whose evidence was incomplete — check it directly
        // and refine while refining can still help.
        bool refine_for_turn = false;
        if (verdict.accepted && !out.leaves.empty() && turnSplits < kTurnSplitAttempts) {
            CurveLeaf provisional;
            provisional.singularVertexAtStart = verdict.singularAtStart;
            provisional.singularBracket = verdict.singularBracket;
            provisional.startTangent = verdict.startTangent;
            const CurveLeaf& before = out.leaves.back();
            if (!join_is_singular(before, provisional)) {
                const double turn = encoded_turn(out.points[out.points.size() - 2],
                                                 out.points.back(), gp_Pnt(span.poles.back()));
                const double allowed = angularTol + genuine_tangent_jump(before, provisional);
                const bool worth_splitting =
                    verdict.chordLength + verdict.chordBound > floor &&
                    !(before.angular == AngularStatus::Satisfied &&
                      verdict.angular == AngularStatus::Satisfied);
                if (turn > allowed && worth_splitting) refine_for_turn = true;
            }
        }

        const bool want_split = !verdict.accepted || refine_for_turn;
        const bool room_to_split = span.depth < limits.maxDepthPerSpan &&
                                   out.leaves.size() + stack.size() + 2 <= cap;
        if (want_split && !room_to_split && !verdict.accepted && span.depth >= limits.maxDepthPerSpan &&
            verdict.chordBound <= outerBudget && verdict.chordLength + verdict.chordBound <= floor) {
            // Subdivision is exhausted on a span that is nonetheless POSITIONALLY
            // negligible — inside the chord budget and below the resolution
            // floor, so on screen it is a vertex. Publishing it keeps a bounded
            // error where refusing would delete the edge outright. It gets NO
            // singular label and NO exemption from the display-turn rule, so the
            // angular quality it failed to reach is measured and reported rather
            // than quietly claimed (followup §2 C1: the floor bounds an
            // unresolved region, it never certifies one).
            verdict.accepted = true;
            verdict.angular = AngularStatus::Undefined;
            verdict.angularMargin = -1.0;
            verdict.reason = "unresolved below the positional resolution floor at the depth cap";
        } else if (want_split && !room_to_split && verdict.accepted) {
            // The span itself is acceptable; only the turn refinement wanted more
            // room. Publish it and let the turn pass report what the display does.
            refine_for_turn = false;
        }

        if (verdict.accepted && !refine_for_turn) {
            if (out.leaves.size() >= cap) {
                outcome.limited = true;
                outcome.diagnostic =
                    "segment budget reached (" + num_text(static_cast<double>(cap)) + ")";
                return outcome;
            }
            if (out.leaves.empty()) firstSpan = span;
            lastSpan = span;
            out.points.emplace_back(span.poles.back());
            record_leaf(span, verdict, out);
            turnSplits = 0;
            continue;
        }
        if (refine_for_turn) ++turnSplits;
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
        if (!control_net_is_valid(left) || !control_net_is_valid(right)) {
            outcome.invalidNet = true;
            outcome.diagnostic =
                "subdivision produced a non-finite control net at depth " + num_text(left.depth);
            return outcome;
        }
        stack.push_back(std::move(right));
        stack.push_back(std::move(left));
    }
    return outcome;
}

// Closure is a SEMANTIC property of the edge, never a distance and never a
// parameter epsilon (followup §2 F7). A 1e-5 mm line a billion millimetres from
// the origin is short, not closed; a radius-5000 circle trimmed to
// [0, 2pi - 5e-10] has a real 2.5e-6 mm gap and two distinct vertices, and the
// retired `span >= period - 1e-9` shortcut closed it anyway. Exactly three
// things establish closure here:
//   * the edge's two vertices are the SAME TopoDS_Vertex;
//   * BRep_Tool::IsClosed on the edge — a seam edge of a closed surface;
//   * the basis curve is closed AND the trim covers its full closed parameter
//     range EXACTLY as the BRep records it, compared bitwise with no tolerance.
bool edge_is_semantically_closed(const BRepAdaptor_Curve& curve, const Handle(Geom_Curve) & base,
                                 double u1, double u2) {
    try {
        const TopoDS_Edge& edge = curve.Edge();
        const TopoDS_Vertex first = TopExp::FirstVertex(edge);
        const TopoDS_Vertex last = TopExp::LastVertex(edge);
        if (!first.IsNull() && !last.IsNull() && first.IsSame(last)) return true;
        if (BRep_Tool::IsClosed(edge)) return true;
    } catch (const Standard_Failure&) {
        // fall through to the parameter-coverage predicate
    }
    if (base.IsNull()) return false;
    if (base->IsPeriodic() && (u2 - u1) == base->Period()) return true;
    return base->IsClosed() && u1 == base->FirstParameter() && u2 == base->LastParameter();
}

// followup §2 F5/F7: an endpoint that moves AFTER its leaf was certified takes
// the leaf's angular evidence with it — the cone was proved about a chord that
// no longer exists. Re-run it about the chord actually emitted.
void recheck_leaf_after_endpoint_move(const RationalBezierSpan& span, double angularTol,
                                      const gp_Pnt& a, const gp_Pnt& b, CurveLeaf& leaf) {
    if (leaf.singularBracket) return;  // its evidence is positional, not a cone
    const gp_XYZ chord = b.XYZ().Subtracted(a.XYZ());
    const gp_XYZ emitted = encode32(b).Subtracted(encode32(a));
    const double rotation = vector_angle(chord, emitted);
    leaf.encodedChordRotationRad = rotation >= 0.0 ? rotation : kPi;
    if (!(emitted.Modulus() > 0.0)) {
        leaf.angular = AngularStatus::Undefined;
        leaf.angularMarginRad = -1.0;
        return;
    }
    const ConeVerdict verdict = tangent_cone_verdict(span, angularTol, emitted);
    leaf.angular = verdict.status == AngularStatus::Uncertified ? AngularStatus::Undefined
                                                                : verdict.status;
    leaf.angularMarginRad = verdict.marginRad;
}

// NUM §2.3: keep the analytic endpoints, so closure and seam identity survive
// the conversion. Any replacement moves a point AFTER its leaf was certified, so
// the displacement is charged to that leaf's bound and re-checked; a snap that
// would break the budget is not taken.
bool snap_analytic_endpoints(const BRepAdaptor_Curve& curve, const Handle(Geom_Curve) & base,
                             double u1, double u2, double budget, double angularTol,
                             const RationalBezierSpan& firstSpan,
                             const RationalBezierSpan& lastSpan, CurveSampleResult& out,
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
    bool head_moved = false;
    bool tail_moved = false;
    if (out.leaves.front().chordBoundMm + head <= budget) {
        out.points.front() = first;
        out.leaves.front().chordBoundMm += head;
        head_moved = head > 0.0;
    }
    if (out.leaves.back().chordBoundMm + tail <= budget) {
        out.points.back() = last;
        out.leaves.back().chordBoundMm += tail;
        tail_moved = tail > 0.0;
    }

    if (edge_is_semantically_closed(curve, base, u1, u2)) {
        const double gap = out.points.front().Distance(out.points.back());
        if (out.leaves.back().chordBoundMm + gap <= budget) {
            out.points.back() = out.points.front();
            out.leaves.back().chordBoundMm += gap;
            tail_moved = tail_moved || gap > 0.0;
        }
    }

    if (head_moved && out.points.size() >= 2) {
        recheck_leaf_after_endpoint_move(firstSpan, angularTol, out.points[0], out.points[1],
                                         out.leaves.front());
    }
    if (tail_moved && out.points.size() >= 2) {
        recheck_leaf_after_endpoint_move(lastSpan, angularTol,
                                         out.points[out.points.size() - 2], out.points.back(),
                                         out.leaves.back());
    }
    return true;
}

// What the EMITTED float32 polyline turned out to be: the measured quantization
// (NUM §1.2 — measure, never guess) and the followup §2 C1 display-turn check
// over the very points MESH1 will carry.
struct EncodedEvidence {
    bool turnViolation = false;
    bool zeroChord = false;
    double worstTurn = -1.0;
    std::size_t violationJoin = 0;
    std::size_t unmeasurableJoins = 0;
};

// followup §2 F5 read as a MEASUREMENT bound rather than an acceptance rule: an
// emitted chord of length L whose two endpoints were displaced by at most e- and
// e+ by float32 rounding has a direction uncertain by asin((e- + e+)/L). This is
// the derived quantity, from the MEASURED per-point displacement — not a
// constant multiple of anything. When the two chords meeting at a join are each
// so short that this uncertainty alone covers the whole allowance, the turn
// there is rounding jitter between points a viewer cannot tell apart, and no
// measurement of it can distinguish a real excess from noise.
double encoded_direction_uncertainty(const gp_Pnt& a, const gp_Pnt& b) {
    const double length = encode32(b).Subtracted(encode32(a)).Modulus();
    if (!(length > 0.0)) return kPi;
    const double displacement = measured_quantization(a) + measured_quantization(b);
    return std::asin(std::clamp(displacement / length, 0.0, 1.0));
}

EncodedEvidence apply_encoded_evidence(CurveSampleResult& out, double angularTol) {
    EncodedEvidence evidence;
    if (out.points.empty()) return evidence;
    double worst = 0.0;
    for (const gp_Pnt& p : out.points) worst = std::max(worst, measured_quantization(p));
    out.quantizationErrorMm = worst;

    for (std::size_t k = 0; k + 1 < out.points.size() && k < out.leaves.size(); ++k) {
        const gp_XYZ chord = encode32(out.points[k + 1]).Subtracted(encode32(out.points[k]));
        if (!(chord.Modulus() > 0.0)) {
            // A segment whose two emitted endpoints coincide has no direction at
            // all; the join it takes part in is skipped explicitly rather than
            // measured from a zero vector.
            evidence.zeroChord = true;
            out.leaves[k].angular = AngularStatus::Undefined;
            out.leaves[k].angularMarginRad = -1.0;
        }
    }

    for (std::size_t i = 1; i + 1 < out.points.size() && i < out.leaves.size(); ++i) {
        if (join_is_singular(out.leaves[i - 1], out.leaves[i])) continue;
        const double turn = encoded_turn(out.points[i - 1], out.points[i], out.points[i + 1]);
        if (turn < 0.0) continue;  // zero-length encoded chord, already recorded
        // followup §2 C1: what the tolerance bounds is the polyline's EXCESS over
        // the curve's own tangent jump at this join. A C0 knot really does turn
        // 166 degrees and no refinement removes it; what must not happen is the
        // POLYLINE adding more than the tolerance on top of it.
        const double allowed = angularTol + genuine_tangent_jump(out.leaves[i - 1], out.leaves[i]);
        const double noise = encoded_direction_uncertainty(out.points[i - 1], out.points[i]) +
                             encoded_direction_uncertainty(out.points[i], out.points[i + 1]);
        if (noise >= allowed) {
            // Unmeasurable: the two chords are shorter than the rounding of their
            // own endpoints, so this join is three points inside one float32
            // cell. Both leaves keep `Undefined` — the claim is withdrawn — but a
            // turn that carries no information is not reported as a defect.
            ++evidence.unmeasurableJoins;
            out.leaves[i - 1].angular = AngularStatus::Undefined;
            out.leaves[i - 1].angularMarginRad = -1.0;
            out.leaves[i].angular = AngularStatus::Undefined;
            out.leaves[i].angularMarginRad = -1.0;
            continue;
        }
        evidence.worstTurn = std::max(evidence.worstTurn, turn);
        if (turn > allowed) {
            if (!evidence.turnViolation) evidence.violationJoin = i;
            evidence.turnViolation = true;
            out.leaves[i - 1].angular = AngularStatus::Undefined;
            out.leaves[i - 1].angularMarginRad = -1.0;
            out.leaves[i].angular = AngularStatus::Undefined;
            out.leaves[i].angularMarginRad = -1.0;
        }
    }
    out.encodedTurnMaxRad = evidence.worstTurn;
    return evidence;
}

// Singular parameters and the undefined-somewhere flag are derived from the
// FINAL leaves, after every endpoint move and every encoded downgrade, so a leaf
// the turn pass rewrote cannot leave stale bookkeeping behind.
void collect_angular_summary(CurveSampleResult& out) {
    out.singularParameters.clear();
    out.angularUndefinedSomewhere = false;
    auto note = [&out](double t) {
        for (double known : out.singularParameters) {
            if (std::abs(known - t) <= 1e-12) return;
        }
        out.singularParameters.push_back(t);
    };
    for (const CurveLeaf& leaf : out.leaves) {
        if (leaf.angular == AngularStatus::Undefined) out.angularUndefinedSomewhere = true;
        if (leaf.singularVertexAtStart) note(leaf.t0);
        if (leaf.singularVertexAtEnd) note(leaf.t1);
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

std::string encoded_shortfall_text(const CurveSampleResult& out, const EncodedEvidence& encoded,
                                   double angularTol) {
    return "float32 representation: measured quantization " + num_text(out.quantizationErrorMm) +
           " mm; " +
           (encoded.turnViolation
                ? "the emitted turn at join " +
                      num_text(static_cast<double>(encoded.violationJoin)) + " is " +
                      num_text(encoded.worstTurn) + " rad against the requested " +
                      num_text(angularTol) + " rad"
                : std::string("a segment's two emitted endpoints coincide"));
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
    // The sampler itself never relaxes anything: what was requested is what was
    // achieved on every path it returns. Only the caller's edge policy
    // (`Tessellate.cpp`'s doubling ladder) can make the two differ, and it
    // rewrites these fields itself (PR-07 / D9).
    auto stamp = [&](CurveSampleResult result) {
        result.requestedChordToleranceMm = tolerance;
        result.achievedChordToleranceMm = tolerance;
        result.requestedAngularToleranceRad = angular;
        result.achievedAngularToleranceRad = angular;
        return result;
    };
    if (limits.remainingBodySegments < 1) {
        CurveSampleResult exhausted;
        exhausted.certification = CurveCertification::QualityLimited;
        exhausted.diagnostic = "the per-body segment budget is exhausted";
        return stamp(std::move(exhausted));
    }

    CurveSampleResult out;
    try {
        const double u1 = curve.FirstParameter();
        const double u2 = curve.LastParameter();
        if (!std::isfinite(u1) || !std::isfinite(u2) || !(u2 > u1) ||
            Precision::IsInfinite(std::abs(u1)) || Precision::IsInfinite(std::abs(u2))) {
            return stamp(failure("edge has a non-finite, unbounded or empty parameter range"));
        }

        Handle(Geom_Curve) base;
        if (curve.Is3DCurve()) base = curve.GeomCurve();
        while (!base.IsNull() && base->IsKind(STANDARD_TYPE(Geom_TrimmedCurve))) {
            base = Handle(Geom_TrimmedCurve)::DownCast(base)->BasisCurve();
        }

        const SpanSet source = build_source_spans(curve, base, u1, u2, tolerance);
        if (!source.ok || source.spans.empty()) {
            return stamp(failure(source.diagnostic.empty()
                                     ? std::string("no exact or approximate span set for this curve")
                                     : source.diagnostic));
        }

        // §2.7 step 4: the fallback already spent a quarter of the budget on the
        // approximation, so the hull algorithm gets the remaining three quarters.
        const double chordBudget = source.approximate ? 0.75 * tolerance : tolerance;
        RationalBezierSpan firstSpan;
        RationalBezierSpan lastSpan;
        const SubdivideOutcome outcome = subdivide_spans(source.spans, chordBudget, allowance,
                                                         angular, limits, cancelled, out, firstSpan,
                                                         lastSpan);
        if (outcome.cancelled) return stamp(failure("cancelled"));
        if (outcome.invalidNet) {
            // followup §2 F6: an arithmetically invalid control net has no hull
            // certificate, so there is nothing to publish — and emitting the
            // endpoints would hand back a polyline with no bound at all.
            CurveSampleResult invalid;
            invalid.certification = CurveCertification::QualityLimited;
            invalid.diagnostic = outcome.diagnostic;
            return stamp(std::move(invalid));
        }
        if (outcome.limited) {
            // A WORK cap publishes nothing: there is no complete polyline, and a
            // straight chord in its place would be a lie (header contract).
            CurveSampleResult limited;
            limited.certification = CurveCertification::QualityLimited;
            limited.diagnostic = outcome.diagnostic;
            return stamp(std::move(limited));
        }
        if (out.points.size() < 2 || out.leaves.empty()) return stamp(failure("no segment emitted"));

        std::string reason;
        if (!snap_analytic_endpoints(curve, base, u1, u2, chordBudget - allowance, angular,
                                     firstSpan, lastSpan, out, reason)) {
            return stamp(failure(reason));
        }
        for (const gp_Pnt& p : out.points) {
            if (!is_finite(p)) {
                return stamp(failure("the sampled polyline contains a non-finite point"));
            }
        }

        double worst = 0.0;
        for (const CurveLeaf& leaf : out.leaves) worst = std::max(worst, leaf.chordBoundMm);
        const EncodedEvidence encoded = apply_encoded_evidence(out, angular);
        collect_angular_summary(out);

        if (source.approximate) {
            const IndependentCheck check = independent_max_error(curve, out, u1, u2, tolerance);
            out.sampledMaxErrorMm = check.maxError;
            if (!(check.maxError + out.quantizationErrorMm <= tolerance)) {
                return stamp(
                    failure("approximate fallback rejected: independent sampled error " +
                            num_text(check.maxError) + " mm plus measured quantization " +
                            num_text(out.quantizationErrorMm) + " mm exceeds the requested " +
                            num_text(tolerance) + " mm"));
            }
            out.certification = CurveCertification::KernelEstimated;
            out.diagnostic = check.complete
                                 ? "GeomConvert_ApproxCurve fallback: sampled evidence only"
                                 : "GeomConvert_ApproxCurve fallback: sampled evidence only, and "
                                   "the independent check stopped at its evaluation cap";
            if (encoded.turnViolation || encoded.zeroChord) {
                // The provenance is still the fallback, but the REQUESTED display
                // quality was not reached, and that is the claim a consumer acts
                // on. QualityLimited wins and the diagnostic keeps both facts.
                out.certification = CurveCertification::QualityLimited;
                out.diagnostic += "; " + encoded_shortfall_text(out, encoded, angular);
            }
            return stamp(std::move(out));
        }

        if (encoded.turnViolation || encoded.zeroChord) {
            // followup §2 C1: the emitted polyline does not meet the requested
            // angular quality and no further refinement of it can — the excess is
            // in the float32 representation, not in the curve. Publish the
            // double-certified geometry and say exactly what is wrong with it.
            out.certification = CurveCertification::QualityLimited;
            out.certifiedChordBoundMm = -1;
            out.diagnostic = encoded_shortfall_text(out, encoded, angular);
            return stamp(std::move(out));
        }

        if (worst + allowance + out.quantizationErrorMm <= tolerance) {
            out.certifiedChordBoundMm = worst;
            out.certification = CurveCertification::Certified;
            return stamp(std::move(out));
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
        return stamp(std::move(out));
    } catch (const Standard_Failure& failed) {
        return stamp(failure(std::string("OCCT failure: ") + failed.GetMessageString()));
    }
}

}  // namespace onecad::tess
