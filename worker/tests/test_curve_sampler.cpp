// test_curve_sampler.cpp — VP-HARDENING WP08 / VP11 / finding R03.
//
// Acceptance TEST-CURVE-01..07 (docs/viewport-hardening/04-ACCEPTANCE-AND-TESTS.md
// §7, oracle rules §3.2). Every oracle here is INDEPENDENT of the sampler:
// analytic polynomials, analytic conic equations, an independently written
// rational Bernstein evaluator, central differences, and dense Geom_Curve
// evaluation through the adaptor. Nothing asserts against the sampler's own leaf
// evaluator, and no dense sample check is ever treated as a proof of a bound.
#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <functional>
#include <random>
#include <string>
#include <vector>

#include <BRepAdaptor_Curve.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <Geom_BSplineCurve.hxx>
#include <Geom_BezierCurve.hxx>
#include <Geom_Circle.hxx>
#include <Geom_Ellipse.hxx>
#include <Geom_OffsetCurve.hxx>
#include <NCollection_Array1.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Edge.hxx>
#include <TopLoc_Location.hxx>
#include <gp_Ax1.hxx>
#include <gp_Ax2.hxx>
#include <gp_Dir.hxx>
#include <gp_Lin.hxx>
#include <gp_Pnt.hxx>
#include <gp_Trsf.hxx>
#include <gp_Vec.hxx>

#include "tess/CurveSampler.h"
#include "tess/Tessellate.h"

namespace {

using onecad::tess::AngularStatus;
using onecad::tess::CurveCertification;
using onecad::tess::CurveLeaf;
using onecad::tess::CurveSampleLimits;
using onecad::tess::CurveSampleRequest;
using onecad::tess::CurveSampleResult;
using onecad::tess::RationalBezierSpan;

int g_failures = 0;

void check(bool condition, const std::string& message) {
    if (!condition) {
        std::fprintf(stderr, "FAIL: %s\n", message.c_str());
        ++g_failures;
    }
}

constexpr double kPi = 3.14159265358979323846;
constexpr double kFiveDegrees = 0.08726646259971647;

// ---------------------------------------------------------------------------
// MESH1 section readers (same shape as test_tessellation_quality.cpp)
// ---------------------------------------------------------------------------

std::uint16_t u16(const std::vector<std::uint8_t>& bytes, std::size_t offset) {
    return static_cast<std::uint16_t>(bytes[offset] | (bytes[offset + 1] << 8));
}

std::uint32_t u32(const std::vector<std::uint8_t>& bytes, std::size_t offset) {
    std::uint32_t value = 0;
    for (int i = 0; i < 4; ++i) {
        value |= static_cast<std::uint32_t>(bytes[offset + i]) << (i * 8);
    }
    return value;
}

float f32(const std::vector<std::uint8_t>& bytes, std::size_t offset) {
    const std::uint32_t bits = u32(bytes, offset);
    float out = 0.0F;
    std::memcpy(&out, &bits, 4);
    return out;
}

struct Section {
    std::uint32_t type = 0;
    std::uint32_t offset = 0;
    std::uint32_t length = 0;
};

Section find_section(const std::vector<std::uint8_t>& bytes, std::uint32_t type) {
    if (bytes.size() < 64) return {};
    const std::uint16_t count = u16(bytes, 0x1E);
    if (bytes.size() < 64U + static_cast<std::size_t>(count) * 16U) return {};
    for (std::uint16_t i = 0; i < count; ++i) {
        const std::size_t entry = 64U + static_cast<std::size_t>(i) * 16U;
        if (u32(bytes, entry) == type) {
            return {type, u32(bytes, entry + 8), u32(bytes, entry + 12)};
        }
    }
    return {};
}

// mesh_format.md §4: EDGE_RANGES is section type 7, EDGE_POSITIONS type 8.
std::vector<gp_Pnt> edge_polyline(const std::vector<std::uint8_t>& bytes, std::uint32_t index) {
    const Section ranges = find_section(bytes, 7);
    const Section positions = find_section(bytes, 8);
    std::vector<gp_Pnt> out;
    if (ranges.type == 0 || positions.type == 0) return out;
    if (static_cast<std::size_t>(index + 1) * 8U > ranges.length) return out;
    const std::size_t entry = ranges.offset + static_cast<std::size_t>(index) * 8U;
    const std::uint32_t first = u32(bytes, entry);
    const std::uint32_t count = u32(bytes, entry + 4);
    for (std::uint32_t i = 0; i < count; ++i) {
        const std::size_t at = positions.offset + static_cast<std::size_t>(first + i) * 12U;
        if (at + 12U > bytes.size()) return {};
        out.emplace_back(f32(bytes, at), f32(bytes, at + 4), f32(bytes, at + 8));
    }
    return out;
}

// ---------------------------------------------------------------------------
// Independent geometry helpers
// ---------------------------------------------------------------------------

double point_to_segment(const gp_Pnt& p, const gp_Pnt& a, const gp_Pnt& b) {
    const double vx = b.X() - a.X(), vy = b.Y() - a.Y(), vz = b.Z() - a.Z();
    const double len2 = vx * vx + vy * vy + vz * vz;
    const double wx = p.X() - a.X(), wy = p.Y() - a.Y(), wz = p.Z() - a.Z();
    double u = 0.0;
    if (len2 > 0.0) u = std::clamp((wx * vx + wy * vy + wz * vz) / len2, 0.0, 1.0);
    const double dx = wx - u * vx, dy = wy - u * vy, dz = wz - u * vz;
    return std::sqrt(dx * dx + dy * dy + dz * dz);
}

double distance_to_polyline(const gp_Pnt& p, const std::vector<gp_Pnt>& poly) {
    double best = 1e300;
    for (std::size_t i = 0; i + 1 < poly.size(); ++i) {
        best = std::min(best, point_to_segment(p, poly[i], poly[i + 1]));
    }
    return best;
}

bool all_finite(const std::vector<gp_Pnt>& poly) {
    for (const gp_Pnt& p : poly) {
        if (!std::isfinite(p.X()) || !std::isfinite(p.Y()) || !std::isfinite(p.Z())) return false;
    }
    return true;
}

// Dense max deviation of an analytically supplied curve from an emitted polyline.
double dense_deviation(const std::function<gp_Pnt(double)>& curve, double t0, double t1,
                       const std::vector<gp_Pnt>& poly, int samples) {
    if (poly.size() < 2) return 1e300;
    double worst = 0.0;
    for (int i = 0; i <= samples; ++i) {
        const double t = t0 + (t1 - t0) * static_cast<double>(i) / static_cast<double>(samples);
        worst = std::max(worst, distance_to_polyline(curve(t), poly));
    }
    return worst;
}

// Dense deviation measured through OCCT's own evaluator for the ORIGINAL edge —
// used where no closed form is available (offset curves, B-splines).
double dense_deviation_of_edge(const TopoDS_Edge& edge, const std::vector<gp_Pnt>& poly,
                               int samples) {
    BRepAdaptor_Curve curve(edge);
    const double u0 = curve.FirstParameter();
    const double u1 = curve.LastParameter();
    return dense_deviation([&](double t) { return curve.Value(t); }, u0, u1, poly, samples);
}

double binomial(int n, int k) {
    double result = 1.0;
    for (int i = 0; i < k; ++i) result = result * (n - i) / (i + 1);
    return result;
}

// Independent rational Bernstein evaluator (never the sampler's own).
gp_Pnt rational_bezier_point(const std::vector<gp_XYZ>& poles, const std::vector<double>& weights,
                             double t) {
    const int n = static_cast<int>(poles.size()) - 1;
    gp_XYZ numerator(0.0, 0.0, 0.0);
    double denominator = 0.0;
    for (int i = 0; i <= n; ++i) {
        const double basis = binomial(n, i) * std::pow(1.0 - t, n - i) * std::pow(t, i) *
                             weights[static_cast<std::size_t>(i)];
        numerator.Add(poles[static_cast<std::size_t>(i)].Multiplied(basis));
        denominator += basis;
    }
    return gp_Pnt(numerator.Divided(denominator));
}

gp_XYZ bernstein_value(const std::vector<gp_XYZ>& coefficients, double t) {
    const int n = static_cast<int>(coefficients.size()) - 1;
    gp_XYZ sum(0.0, 0.0, 0.0);
    for (int i = 0; i <= n; ++i) {
        sum.Add(coefficients[static_cast<std::size_t>(i)].Multiplied(
            binomial(n, i) * std::pow(1.0 - t, n - i) * std::pow(t, i)));
    }
    return sum;
}

double bernstein_value(const std::vector<double>& coefficients, double t) {
    const int n = static_cast<int>(coefficients.size()) - 1;
    double sum = 0.0;
    for (int i = 0; i <= n; ++i) {
        sum += coefficients[static_cast<std::size_t>(i)] * binomial(n, i) *
               std::pow(1.0 - t, n - i) * std::pow(t, i);
    }
    return sum;
}

bool leaves_are_contiguous_and_increasing(const std::vector<CurveLeaf>& leaves) {
    if (leaves.empty()) return false;
    for (std::size_t i = 0; i < leaves.size(); ++i) {
        if (!(leaves[i].t1 > leaves[i].t0)) return false;
        if (i > 0 &&
            std::abs(leaves[i].t0 - leaves[i - 1].t1) > 1e-12 * (1.0 + std::abs(leaves[i].t0))) {
            return false;
        }
    }
    return true;
}

// INDEPENDENT angular oracle: the greatest turn between consecutive emitted
// segments, measured from the published points alone. This is what a faceted
// silhouette actually looks like, and it is the guarantee the fine tier's
// 5 degree cap exists to give — so it is pinned separately from the sampler's
// own angular bookkeeping. A fixture with a genuine cusp is excluded, since a
// cusp's turn is real geometry and no refinement removes it.
double max_segment_turn_degrees(const std::vector<gp_Pnt>& poly) {
    double worst = 0.0;
    for (std::size_t i = 0; i + 2 < poly.size(); ++i) {
        const gp_XYZ a = poly[i + 1].XYZ().Subtracted(poly[i].XYZ());
        const gp_XYZ b = poly[i + 2].XYZ().Subtracted(poly[i + 1].XYZ());
        if (!(a.Modulus() > 0.0) || !(b.Modulus() > 0.0)) continue;
        const double dot = std::clamp(a.Normalized().Dot(b.Normalized()), -1.0, 1.0);
        worst = std::max(worst, std::acos(dot) * 180.0 / kPi);
    }
    return worst;
}

CurveSampleResult sample(const TopoDS_Edge& edge, double tolerance, double angular = kFiveDegrees,
                         CurveSampleLimits limits = CurveSampleLimits{},
                         const std::function<bool()>& cancelled = {}) {
    BRepAdaptor_Curve curve(edge);
    CurveSampleRequest request;
    request.chordToleranceMm = tolerance;
    request.angularToleranceRad = angular;
    request.representationAllowanceMm = 0.0;
    return sample_edge_curve(curve, request, limits, cancelled);
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

// F05: the NUM §2.6 degree-five counterexample.
Handle(Geom_BezierCurve) s_curve_bezier(double dx = 0.0) {
    NCollection_Array1<gp_Pnt> poles(1, 6);
    poles.SetValue(1, gp_Pnt(dx + 0.0, 0.0, 0.0));
    poles.SetValue(2, gp_Pnt(dx + 0.2, 0.0, 0.0));
    poles.SetValue(3, gp_Pnt(dx + 0.4, -6.4, 0.0));
    poles.SetValue(4, gp_Pnt(dx + 0.6, 6.4, 0.0));
    poles.SetValue(5, gp_Pnt(dx + 0.8, 0.0, 0.0));
    poles.SetValue(6, gp_Pnt(dx + 1.0, 0.0, 0.0));
    return new Geom_BezierCurve(poles);
}

gp_Pnt s_curve_point(double t, double dx = 0.0) {
    return gp_Pnt(dx + t, 128.0 * t * t * (t - 0.5) * (t - 1.0) * (t - 1.0), 0.0);
}

Handle(Geom_BezierCurve) rational_cubic() {
    NCollection_Array1<gp_Pnt> poles(1, 4);
    poles.SetValue(1, gp_Pnt(0.0, 0.0, 0.0));
    poles.SetValue(2, gp_Pnt(3.0, 8.0, 1.0));
    poles.SetValue(3, gp_Pnt(9.0, -6.0, -2.0));
    poles.SetValue(4, gp_Pnt(12.0, 2.0, 0.0));
    NCollection_Array1<double> weights(1, 4);
    weights.SetValue(1, 1.0);
    weights.SetValue(2, 0.4);
    weights.SetValue(3, 2.5);
    weights.SetValue(4, 1.0);
    return new Geom_BezierCurve(poles, weights);
}

// F07: nonuniform knots with a C2 join at 0.5 and a C0 join at 1.0 (degree 3,
// interior multiplicity 3 == degree) with a genuine tangent break there.
Handle(Geom_BSplineCurve) f07_spline() {
    NCollection_Array1<gp_Pnt> poles(1, 8);
    poles.SetValue(1, gp_Pnt(0.0, 0.0, 0.0));
    poles.SetValue(2, gp_Pnt(1.0, 3.0, 0.0));
    poles.SetValue(3, gp_Pnt(2.0, -3.0, 0.0));
    poles.SetValue(4, gp_Pnt(3.0, 4.0, 0.0));
    poles.SetValue(5, gp_Pnt(4.0, 0.0, 0.0));  // the C0 join sits on this pole
    poles.SetValue(6, gp_Pnt(4.0, 5.0, 0.0));  // and turns sharply upward
    poles.SetValue(7, gp_Pnt(6.0, 5.0, 0.0));
    poles.SetValue(8, gp_Pnt(7.0, 1.0, 0.0));
    NCollection_Array1<double> knots(1, 4);
    knots.SetValue(1, 0.0);
    knots.SetValue(2, 0.5);
    knots.SetValue(3, 1.0);
    knots.SetValue(4, 2.0);
    NCollection_Array1<int> mults(1, 4);
    mults.SetValue(1, 4);
    mults.SetValue(2, 1);
    mults.SetValue(3, 3);
    mults.SetValue(4, 4);
    return new Geom_BSplineCurve(poles, knots, mults, 3);
}

// A smooth C2 cubic B-spline (uniform interior knots, gentle curvature) — a
// legal basis for a Geom_OffsetCurve, unlike the C0 F07 fixture.
Handle(Geom_BSplineCurve) smooth_spline() {
    NCollection_Array1<gp_Pnt> poles(1, 6);
    poles.SetValue(1, gp_Pnt(0.0, 0.0, 0.0));
    poles.SetValue(2, gp_Pnt(2.0, 3.0, 0.0));
    poles.SetValue(3, gp_Pnt(5.0, -1.0, 0.0));
    poles.SetValue(4, gp_Pnt(8.0, 4.0, 0.0));
    poles.SetValue(5, gp_Pnt(11.0, 1.0, 0.0));
    poles.SetValue(6, gp_Pnt(14.0, 2.0, 0.0));
    NCollection_Array1<double> knots(1, 4);
    knots.SetValue(1, 0.0);
    knots.SetValue(2, 1.0);
    knots.SetValue(3, 2.0);
    knots.SetValue(4, 3.0);
    NCollection_Array1<int> mults(1, 4);
    mults.SetValue(1, 4);
    mults.SetValue(2, 1);
    mults.SetValue(3, 1);
    mults.SetValue(4, 4);
    return new Geom_BSplineCurve(poles, knots, mults, 3);
}

// A high-oscillation spline: 41 poles alternating in y, uniform cubic knots.
Handle(Geom_BSplineCurve) oscillating_spline() {
    const int count = 41;
    NCollection_Array1<gp_Pnt> poles(1, count);
    for (int i = 1; i <= count; ++i) {
        poles.SetValue(i,
                       gp_Pnt(static_cast<double>(i - 1) * 0.5, (i % 2 == 0) ? 5.0 : -5.0, 0.0));
    }
    const int spans = count - 3;  // degree 3, clamped
    NCollection_Array1<double> knots(1, spans + 1);
    NCollection_Array1<int> mults(1, spans + 1);
    for (int i = 1; i <= spans + 1; ++i) {
        knots.SetValue(i, static_cast<double>(i - 1));
        mults.SetValue(i, (i == 1 || i == spans + 1) ? 4 : 1);
    }
    return new Geom_BSplineCurve(poles, knots, mults, 3);
}

// ---------------------------------------------------------------------------
// TEST-CURVE-01 — the NUM §2.6 S-curve
// ---------------------------------------------------------------------------

void test_curve_01_s_curve_through_mesh1() {
    const TopoDS_Edge edge = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(s_curve_bezier())).Edge();
    const onecad::tess::BodyMesh mesh =
        onecad::tess::tessellate_body(edge, "body_scurve", "fine", true, nullptr);
    check(mesh.ok, "TEST-CURVE-01: S-curve edge tessellates");

    const std::vector<gp_Pnt> poly = edge_polyline(mesh.blob, 0);
    check(poly.size() >= 2, "TEST-CURVE-01: S-curve edge exports a polyline");
    if (poly.size() < 2) return;

    const double worst =
        dense_deviation([](double t) { return s_curve_point(t); }, 0.0, 1.0, poly, 2000);
    double magnitude = 0.0;
    for (const gp_Pnt& p : poly) {
        magnitude = std::max({magnitude, std::abs(p.X()), std::abs(p.Y()), std::abs(p.Z())});
    }
    // float32 world storage: 24-bit significand, so a written component carries
    // at most |v|*2^-24; 2^-23*4 keeps head-room over the three-component bound.
    const double allowance = magnitude * 4.0 * 0x1p-23;
    const double at_quarter = distance_to_polyline(s_curve_point(0.25), poly);

    std::fprintf(stderr,
                 "TEST-CURVE-01: %zu polyline points, independent max deviation %.6f mm, "
                 "deviation at NUM §2.6 t=0.25 %.6f mm (budget 0.05 + %.3g mm allowance)\n",
                 poly.size(), worst, at_quarter, allowance);
    check(poly.size() > 2,
          "TEST-CURVE-01: the S-curve must not be exported as a single straight segment");
    check(worst <= 0.05 + allowance,
          "TEST-CURVE-01: independent dense max deviation is within the 0.05 mm fine "
          "chord budget plus the float32 representation allowance (measured " +
              std::to_string(worst) + " mm)");
}

void test_curve_01_budget_ladder() {
    const TopoDS_Edge edge = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(s_curve_bezier())).Edge();
    for (const double tolerance : {0.05, 0.01, 0.001}) {
        const CurveSampleResult result = sample(edge, tolerance);
        const std::string at = " at " + std::to_string(tolerance) + " mm";
        check(result.certification == CurveCertification::Certified,
              "TEST-CURVE-01: exact Bezier span set is Certified" + at);
        check(result.points.size() > 2, "TEST-CURVE-01: more than one segment emitted" + at);
        check(all_finite(result.points), "TEST-CURVE-01: all emitted points finite" + at);
        check(leaves_are_contiguous_and_increasing(result.leaves),
              "TEST-CURVE-01: leaf intervals are contiguous and increasing" + at);
        check(result.certifiedChordBoundMm >= 0.0 && result.certifiedChordBoundMm <= tolerance,
              "TEST-CURVE-01: the reported certificate is inside the requested budget" + at);

        const double worst = dense_deviation([](double t) { return s_curve_point(t); }, 0.0, 1.0,
                                             result.points, 4000);
        const double turn = max_segment_turn_degrees(result.points);
        std::fprintf(stderr,
                     "TEST-CURVE-01: tolerance %.4f mm -> %zu points, certificate %.3e mm, "
                     "independent dense max error %.3e mm, greatest segment turn %.3f deg\n",
                     tolerance, result.points.size(), result.certifiedChordBoundMm, worst, turn);
        check(turn <= 5.0 * 1.05,
              "TEST-CURVE-01: the 5 degree angular criterion still binds — no leaf may be "
              "waived into a visible crease" +
                  at + " (measured " + std::to_string(turn) + " deg)");
        check(worst <= tolerance,
              "TEST-CURVE-01: independent dense max error within the requested tolerance" + at +
                  " (measured " + std::to_string(worst) + " mm)");
    }
}

// ---------------------------------------------------------------------------
// TEST-CURVE-02 — exact conics and positive weighted Beziers
// ---------------------------------------------------------------------------

void test_curve_02_circular_arc() {
    const double radius = 20.0;
    Handle(Geom_Circle) circle = new Geom_Circle(gp_Ax2(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1)), radius);
    const double last = 2.0 * kPi / 3.0;  // 120 degrees
    const TopoDS_Edge edge = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(circle), 0.0, last).Edge();
    const double tolerance = 0.01;
    const CurveSampleResult result = sample(edge, tolerance);

    check(result.certification == CurveCertification::Certified,
          "TEST-CURVE-02: 120 degree circular arc is Certified");
    check(leaves_are_contiguous_and_increasing(result.leaves),
          "TEST-CURVE-02: circular arc leaf intervals are contiguous and increasing");
    check(result.points.size() >= 2 &&
              result.points.front().Distance(gp_Pnt(radius, 0, 0)) <= 1e-9 &&
              result.points.back().Distance(
                  gp_Pnt(radius * std::cos(last), radius * std::sin(last), 0)) <= 1e-9,
          "TEST-CURVE-02: circular arc endpoints are exact to 1e-9");

    double radial = 0.0;
    for (const gp_Pnt& p : result.points) {
        radial = std::max(radial, std::abs(p.Distance(gp_Pnt(0, 0, 0)) - radius));
    }
    const double worst = dense_deviation(
        [&](double t) { return gp_Pnt(radius * std::cos(t), radius * std::sin(t), 0.0); }, 0.0,
        last, result.points, 4000);
    std::fprintf(stderr,
                 "TEST-CURVE-02: r=20 arc 0..120deg -> %zu points, radial error %.3e mm, "
                 "analytic dense max error %.3e mm (budget %.3f)\n",
                 result.points.size(), radial, worst, tolerance);
    check(radial <= 1e-9, "TEST-CURVE-02: every emitted arc point lies on the analytic circle");
    check(worst <= tolerance, "TEST-CURVE-02: analytic circle points are within the chord bound");
}

void test_curve_02_ellipse_trim_and_reversal() {
    const double major = 30.0;
    const double minor = 12.0;
    Handle(Geom_Ellipse) ellipse =
        new Geom_Ellipse(gp_Ax2(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1)), major, minor);
    const double u0 = 30.0 * kPi / 180.0;
    const double u1 = 250.0 * kPi / 180.0;
    const TopoDS_Edge edge = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(ellipse), u0, u1).Edge();
    const TopoDS_Edge reversed = TopoDS::Edge(edge.Reversed());
    const double tolerance = 0.01;

    const CurveSampleResult forward = sample(edge, tolerance);
    const CurveSampleResult backward = sample(reversed, tolerance);

    auto analytic = [&](double t) {
        return gp_Pnt(major * std::cos(t), minor * std::sin(t), 0.0);
    };
    for (const CurveSampleResult* r : {&forward, &backward}) {
        const std::string which = (r == &forward) ? " (forward)" : " (reversed)";
        check(r->certification == CurveCertification::Certified,
              "TEST-CURVE-02: trimmed ellipse is Certified" + which);
        check(leaves_are_contiguous_and_increasing(r->leaves),
              "TEST-CURVE-02: ellipse leaf intervals are contiguous and increasing" + which);
        check(r->points.size() >= 2 && r->points.front().Distance(analytic(u0)) <= 1e-9 &&
                  r->points.back().Distance(analytic(u1)) <= 1e-9,
              "TEST-CURVE-02: trimmed ellipse endpoints are exact to 1e-9" + which);
        const double worst = dense_deviation(analytic, u0, u1, r->points, 4000);
        check(worst <= tolerance,
              "TEST-CURVE-02: analytic ellipse dense max error within the chord bound" + which +
                  " (measured " + std::to_string(worst) + " mm)");
    }
    // ORIENTATION CONTRACT (CurveSampler.h): BRepAdaptor_Curve is
    // orientation-blind, so a reversed edge samples to the IDENTICAL sequence in
    // the IDENTICAL order. MESH1's EDGE_POSITIONS is an undirected polyline and
    // no consumer reads a direction from it, so this is the behaviour to pin —
    // reversing on orientation would change published bytes for no reader.
    bool identical_order = forward.points.size() == backward.points.size();
    for (std::size_t i = 0; identical_order && i < forward.points.size(); ++i) {
        identical_order = forward.points[i].Distance(backward.points[i]) == 0.0;
    }
    check(identical_order,
          "TEST-CURVE-02: a reversed edge yields the same points in the same order (the "
          "documented undirected EDGE_POSITIONS contract)");
    std::fprintf(stderr,
                 "TEST-CURVE-02: ellipse 30..250deg -> %zu points forward, %zu reversed, "
                 "identical order %d\n",
                 forward.points.size(), backward.points.size(), identical_order ? 1 : 0);
}

void test_curve_02_full_periodic_circle() {
    const double radius = 15.0;
    Handle(Geom_Circle) circle = new Geom_Circle(gp_Ax2(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1)), radius);
    const TopoDS_Edge edge = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(circle)).Edge();
    const double tolerance = 0.01;
    const CurveSampleResult result = sample(edge, tolerance);

    check(result.certification == CurveCertification::Certified,
          "TEST-CURVE-02: full periodic circle is Certified");
    check(result.points.size() > 8, "TEST-CURVE-02: full circle emits a real polyline");
    check(leaves_are_contiguous_and_increasing(result.leaves),
          "TEST-CURVE-02: full circle leaf intervals are contiguous and increasing");
    check(result.points.size() >= 2 &&
              result.points.front().Distance(result.points.back()) == 0.0,
          "TEST-CURVE-02: a closed curve keeps exact closure (first point == last point)");
    double radial = 0.0;
    for (const gp_Pnt& p : result.points) {
        radial = std::max(radial, std::abs(p.Distance(gp_Pnt(0, 0, 0)) - radius));
    }
    const double worst = dense_deviation(
        [&](double t) { return gp_Pnt(radius * std::cos(t), radius * std::sin(t), 0.0); }, 0.0,
        2.0 * kPi, result.points, 6000);
    const double turn = max_segment_turn_degrees(result.points);
    std::fprintf(stderr,
                 "TEST-CURVE-02: full circle r=15 -> %zu points, radial error %.3e mm, "
                 "analytic dense max error %.3e mm, greatest segment turn %.3f deg\n",
                 result.points.size(), radial, worst, turn);
    check(turn <= 5.0 * 1.05,
          "TEST-CURVE-02: a full circle's silhouette stays inside the 5 degree angular "
          "criterion (measured " +
              std::to_string(turn) + " deg)");
    check(radial <= 1e-9, "TEST-CURVE-02: full circle points lie on the analytic circle");
    check(worst <= tolerance, "TEST-CURVE-02: full circle dense max error within the chord bound");
}

void test_curve_02_rational_cubic_and_subdivision() {
    Handle(Geom_BezierCurve) bezier = rational_cubic();
    const std::vector<gp_XYZ> poles = {bezier->Pole(1).XYZ(), bezier->Pole(2).XYZ(),
                                       bezier->Pole(3).XYZ(), bezier->Pole(4).XYZ()};
    const std::vector<double> weights = {1.0, 0.4, 2.5, 1.0};

    RationalBezierSpan span;
    span.poles = poles;
    span.weights = weights;
    span.t0 = 0.0;
    span.t1 = 1.0;
    RationalBezierSpan left, right;
    onecad::tess::subdivide_homogeneous(span, left, right);

    double split_error = 0.0;
    for (int i = 0; i <= 32; ++i) {
        const double t = static_cast<double>(i) / 32.0;
        const gp_Pnt reference = rational_bezier_point(poles, weights, t);
        const gp_Pnt from_child =
            (t <= 0.5) ? rational_bezier_point(left.poles, left.weights, 2.0 * t)
                       : rational_bezier_point(right.poles, right.weights, 2.0 * t - 1.0);
        split_error = std::max(split_error, reference.Distance(from_child));
    }
    std::fprintf(stderr,
                 "TEST-CURVE-02: homogeneous subdivision reproduces the rational cubic to %.3e mm "
                 "over 33 parameters\n",
                 split_error);
    check(split_error <= 1e-12,
          "TEST-CURVE-02: subdivide_homogeneous conserves the original rational curve to 1e-12");
    check(gp_Pnt(left.poles.back()).Distance(gp_Pnt(right.poles.front())) == 0.0,
          "TEST-CURVE-02: the shared split point is computed once and is bit-identical");

    // Whole edge, a trimmed edge, and the reversal — endpoints and monotonicity.
    const double tolerance = 0.005;
    const TopoDS_Edge whole = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(bezier)).Edge();
    const TopoDS_Edge trimmed = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(bezier), 0.2, 0.8).Edge();
    const TopoDS_Edge flipped = TopoDS::Edge(whole.Reversed());
    for (const TopoDS_Edge* e : {&whole, &trimmed, &flipped}) {
        BRepAdaptor_Curve adaptor(*e);
        const double a = adaptor.FirstParameter();
        const double b = adaptor.LastParameter();
        const CurveSampleResult result = sample(*e, tolerance);
        check(result.certification == CurveCertification::Certified,
              "TEST-CURVE-02: rational cubic edge is Certified");
        check(leaves_are_contiguous_and_increasing(result.leaves),
              "TEST-CURVE-02: rational cubic leaf intervals are contiguous and increasing");
        check(result.points.size() >= 2 &&
                  result.points.front().Distance(rational_bezier_point(poles, weights, a)) <= 1e-9 &&
                  result.points.back().Distance(rational_bezier_point(poles, weights, b)) <= 1e-9,
              "TEST-CURVE-02: rational cubic endpoints are exact to 1e-9 under trim and reversal");
        const double worst =
            dense_deviation([&](double t) { return rational_bezier_point(poles, weights, t); }, a,
                            b, result.points, 4000);
        check(worst <= tolerance,
              "TEST-CURVE-02: independently evaluated rational cubic is within the chord bound "
              "(measured " +
                  std::to_string(worst) + " mm)");
    }
}

// ---------------------------------------------------------------------------
// TEST-CURVE-03 — repeated knots and C0 joins
// ---------------------------------------------------------------------------

void test_curve_03_knot_and_continuity_splits() {
    Handle(Geom_BSplineCurve) spline = f07_spline();
    const TopoDS_Edge edge = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(spline)).Edge();
    const CurveSampleResult result = sample(edge, 0.01);
    check(result.certification == CurveCertification::Certified,
          "TEST-CURVE-03: F07 spline is Certified");
    check(leaves_are_contiguous_and_increasing(result.leaves),
          "TEST-CURVE-03: F07 leaf intervals are contiguous and increasing");

    auto has_boundary_at = [&](double knot) {
        for (const CurveLeaf& leaf : result.leaves) {
            if (std::abs(leaf.t1 - knot) <= 1e-12) return true;
        }
        return false;
    };
    auto crosses = [&](double knot) {
        for (const CurveLeaf& leaf : result.leaves) {
            if (leaf.t0 < knot - 1e-12 && leaf.t1 > knot + 1e-12) return true;
        }
        return false;
    };

    std::fprintf(stderr, "TEST-CURVE-03: F07 spline -> %zu leaves; boundary at 0.5 %d, at 1.0 %d\n",
                 result.leaves.size(), has_boundary_at(0.5) ? 1 : 0, has_boundary_at(1.0) ? 1 : 0);
    check(has_boundary_at(0.5), "TEST-CURVE-03: a leaf boundary sits on the C2 knot at 0.5");
    check(has_boundary_at(1.0), "TEST-CURVE-03: a leaf boundary sits on the C0 join at 1.0");
    check(!crosses(0.5) && !crosses(1.0),
          "TEST-CURVE-03: no accepted span crosses a knot or the C0 join");

    const double worst = dense_deviation_of_edge(edge, result.points, 8000);
    check(worst <= 0.01,
          "TEST-CURVE-03: dense evaluation of the original spline is within the chord bound "
          "(measured " +
              std::to_string(worst) + " mm)");
}

// ---------------------------------------------------------------------------
// TEST-CURVE-04 — midpoint/chord coincidence and a degenerate chord
// ---------------------------------------------------------------------------

void test_curve_04_bulge_with_midpoint_on_the_chord() {
    // Symmetric quartic: y(t) = 12 t (1-t) (1-2t)^2, so y(0.5) = 0 exactly while
    // the curve reaches 0.75 mm off the chord. The retired midpoint heuristic is
    // blind to this; the control hull is not.
    NCollection_Array1<gp_Pnt> poles(1, 5);
    poles.SetValue(1, gp_Pnt(0.00, 0.0, 0.0));
    poles.SetValue(2, gp_Pnt(0.25, 3.0, 0.0));
    poles.SetValue(3, gp_Pnt(0.50, -4.0, 0.0));
    poles.SetValue(4, gp_Pnt(0.75, 3.0, 0.0));
    poles.SetValue(5, gp_Pnt(1.00, 0.0, 0.0));
    Handle(Geom_BezierCurve) bezier = new Geom_BezierCurve(poles);
    auto analytic = [](double t) {
        return gp_Pnt(t, 12.0 * t * (1.0 - t) * (1.0 - 2.0 * t) * (1.0 - 2.0 * t), 0.0);
    };

    RationalBezierSpan span;
    for (int i = 1; i <= 5; ++i) {
        span.poles.push_back(poles.Value(i).XYZ());
        span.weights.push_back(1.0);
    }
    const double midpoint_distance = point_to_segment(analytic(0.5), analytic(0.0), analytic(1.0));
    const double bound = onecad::tess::hull_chord_bound(span);
    double true_max = 0.0;
    for (int i = 0; i <= 4000; ++i) {
        const double t = static_cast<double>(i) / 4000.0;
        true_max = std::max(true_max, point_to_segment(analytic(t), analytic(0.0), analytic(1.0)));
    }
    std::fprintf(stderr,
                 "TEST-CURVE-04: midpoint-to-chord %.3e mm, true max excursion %.6f mm, "
                 "hull certificate %.6f mm\n",
                 midpoint_distance, true_max, bound);
    check(midpoint_distance <= 1e-12,
          "TEST-CURVE-04: the fixture's midpoint really does sit on the chord");
    check(bound >= true_max,
          "TEST-CURVE-04: the hull certificate is an upper bound on the true excursion");

    const TopoDS_Edge edge = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(bezier)).Edge();
    const CurveSampleResult result = sample(edge, 0.005);
    check(result.certification == CurveCertification::Certified,
          "TEST-CURVE-04: the bulge samples to a Certified polyline");
    const double worst = dense_deviation(analytic, 0.0, 1.0, result.points, 8000);
    check(worst <= 0.005,
          "TEST-CURVE-04: the finite-chord hull criterion catches the interior deviation "
          "(measured " +
              std::to_string(worst) + " mm)");
}

void test_curve_04_loop_with_coincident_endpoints() {
    NCollection_Array1<gp_Pnt> poles(1, 4);
    poles.SetValue(1, gp_Pnt(0.0, 0.0, 0.0));
    poles.SetValue(2, gp_Pnt(4.0, 3.0, 0.0));
    poles.SetValue(3, gp_Pnt(-4.0, 3.0, 0.0));
    poles.SetValue(4, gp_Pnt(0.0, 0.0, 0.0));
    Handle(Geom_BezierCurve) bezier = new Geom_BezierCurve(poles);

    RationalBezierSpan span;
    for (int i = 1; i <= 4; ++i) {
        span.poles.push_back(poles.Value(i).XYZ());
        span.weights.push_back(1.0);
    }
    const double bound = onecad::tess::hull_chord_bound(span);
    check(bound >= 4.0,
          "TEST-CURVE-04: a degenerate chord falls back to the distance from P0, so the loop "
          "cannot be mistaken for a zero-error span");
    check(onecad::tess::tangent_cone_status(span, kFiveDegrees, 1e-9) ==
              AngularStatus::Uncertified,
          "TEST-CURVE-04: a degenerate chord yields no tangent cone, never a false pass");

    const TopoDS_Edge edge = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(bezier)).Edge();
    const CurveSampleResult result = sample(edge, 0.005);
    const std::vector<gp_XYZ> loop_poles = span.poles;
    const std::vector<double> loop_weights = span.weights;
    const double worst = dense_deviation(
        [&](double t) { return rational_bezier_point(loop_poles, loop_weights, t); }, 0.0, 1.0,
        result.points, 8000);
    std::fprintf(stderr,
                 "TEST-CURVE-04: loop hull certificate %.3f mm, %zu points, dense max error "
                 "%.3e mm\n",
                 bound, result.points.size(), worst);
    check(result.certification == CurveCertification::Certified,
          "TEST-CURVE-04: the loop is resolved by subdivision, not refused");
    check(result.points.size() >= 2 && result.points.front().Distance(result.points.back()) == 0.0,
          "TEST-CURVE-04: the loop's coincident endpoints stay coincident");
    check(worst <= 0.005, "TEST-CURVE-04: the loop polyline is within the chord bound (measured " +
                              std::to_string(worst) + " mm)");
}

// ---------------------------------------------------------------------------
// TEST-CURVE-05 — stationary and nearly vanishing derivatives
// ---------------------------------------------------------------------------

void test_curve_05_stationary_endpoint() {
    NCollection_Array1<gp_Pnt> poles(1, 4);
    poles.SetValue(1, gp_Pnt(0.0, 0.0, 0.0));
    poles.SetValue(2, gp_Pnt(0.0, 0.0, 0.0));  // P0 == P1: C'(0) == 0
    poles.SetValue(3, gp_Pnt(1.0, 2.0, 0.0));
    poles.SetValue(4, gp_Pnt(2.0, 0.0, 0.0));
    Handle(Geom_BezierCurve) bezier = new Geom_BezierCurve(poles);

    RationalBezierSpan span;
    for (int i = 1; i <= 4; ++i) {
        span.poles.push_back(poles.Value(i).XYZ());
        span.weights.push_back(1.0);
    }
    bool undefined_at_start = false;
    bool undefined_at_end = false;
    // A wide cone (3 rad) so the whole unsplit span passes the DIRECTIONAL test
    // and the only thing left to report is the vanishing coefficient at t=0.
    const AngularStatus status = onecad::tess::tangent_cone_status(
        span, 3.0, 1e-9, undefined_at_start, undefined_at_end);
    check(status == AngularStatus::Undefined && undefined_at_start && !undefined_at_end,
          "TEST-CURVE-05: a stationary start is reported angular-undefined at the start only");

    const TopoDS_Edge edge = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(bezier)).Edge();
    const CurveSampleResult result = sample(edge, 0.002);
    const std::vector<gp_XYZ> cusp_poles = span.poles;
    const std::vector<double> cusp_weights = span.weights;
    const double worst = dense_deviation(
        [&](double t) { return rational_bezier_point(cusp_poles, cusp_weights, t); }, 0.0, 1.0,
        result.points, 8000);
    std::fprintf(stderr,
                 "TEST-CURVE-05: cusp-like Bezier -> %zu points, angularUndefinedSomewhere %d, "
                 "dense max error %.3e mm\n",
                 result.points.size(), result.angularUndefinedSomewhere ? 1 : 0, worst);
    check(all_finite(result.points), "TEST-CURVE-05: no NaN or infinity in the emitted points");
    check(result.angularUndefinedSomewhere,
          "TEST-CURVE-05: the undefined tangent is reported, not silently absorbed");
    check(result.certification == CurveCertification::Certified,
          "TEST-CURVE-05: the chord bound is still certified across the stationary endpoint");
    check(worst <= 0.002, "TEST-CURVE-05: chord accuracy is still enforced (measured " +
                              std::to_string(worst) + " mm)");
}

void test_curve_05_nearly_vanishing_derivative() {
    NCollection_Array1<gp_Pnt> poles(1, 4);
    poles.SetValue(1, gp_Pnt(0.0, 0.0, 0.0));
    poles.SetValue(2, gp_Pnt(1e-12, 0.0, 0.0));  // derivative pole ~ 3e-12
    poles.SetValue(3, gp_Pnt(1.0, 2.0, 0.0));
    poles.SetValue(4, gp_Pnt(2.0, 0.0, 0.0));
    Handle(Geom_BezierCurve) bezier = new Geom_BezierCurve(poles);
    const TopoDS_Edge edge = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(bezier)).Edge();
    const CurveSampleResult result = sample(edge, 0.002);

    std::vector<gp_XYZ> p;
    std::vector<double> w;
    for (int i = 1; i <= 4; ++i) {
        p.push_back(poles.Value(i).XYZ());
        w.push_back(1.0);
    }
    const double worst = dense_deviation(
        [&](double t) { return rational_bezier_point(p, w, t); }, 0.0, 1.0, result.points, 8000);
    std::fprintf(stderr,
                 "TEST-CURVE-05: near-vanishing derivative -> certification %d, %zu points, "
                 "dense max error %.3e mm\n",
                 static_cast<int>(result.certification), result.points.size(), worst);
    check(all_finite(result.points),
          "TEST-CURVE-05: a nearly vanishing derivative produces no NaN");
    check(result.certification == CurveCertification::Certified,
          "TEST-CURVE-05: a nearly vanishing derivative still resolves to a certified chord");
    check(worst <= 0.002,
          "TEST-CURVE-05: nearly vanishing derivative case stays within the chord bound");
}

void test_curve_05_derivative_numerator_crosscheck() {
    // Port of the design's independent arithmetic check
    // (docs/viewport-hardening/reference/design-math-checks.md): Q = X'W - XW'
    // in Bernstein form must equal the same product evaluated pointwise, for
    // degrees 2..6 with random positive weights and a fixed seed.
    std::mt19937_64 rng(90213);
    std::uniform_real_distribution<double> coordinate(-4.0, 4.0);
    std::uniform_real_distribution<double> weight(0.3, 3.0);

    double worst_identity = 0.0;
    double worst_finite_difference = 0.0;
    for (int degree = 2; degree <= 6; ++degree) {
        for (int trial = 0; trial < 20; ++trial) {
            RationalBezierSpan span;
            for (int i = 0; i <= degree; ++i) {
                span.poles.emplace_back(coordinate(rng), coordinate(rng), coordinate(rng));
                span.weights.push_back(weight(rng));
            }
            const std::vector<gp_XYZ> q = onecad::tess::derivative_numerator_bernstein(span);
            check(q.size() == static_cast<std::size_t>(2 * degree),
                  "TEST-CURVE-05: the derivative numerator has degree 2n-1");

            std::vector<gp_XYZ> x;
            std::vector<double> w;
            for (int i = 0; i <= degree; ++i) {
                x.push_back(span.poles[static_cast<std::size_t>(i)].Multiplied(
                    span.weights[static_cast<std::size_t>(i)]));
                w.push_back(span.weights[static_cast<std::size_t>(i)]);
            }
            std::vector<gp_XYZ> dx;
            std::vector<double> dw;
            for (int i = 0; i < degree; ++i) {
                dx.push_back(x[static_cast<std::size_t>(i + 1)]
                                 .Subtracted(x[static_cast<std::size_t>(i)])
                                 .Multiplied(static_cast<double>(degree)));
                dw.push_back((w[static_cast<std::size_t>(i + 1)] - w[static_cast<std::size_t>(i)]) *
                             static_cast<double>(degree));
            }
            for (int s = 0; s < 31; ++s) {
                const double t = static_cast<double>(s) / 30.0;
                const gp_XYZ reference =
                    bernstein_value(dx, t)
                        .Multiplied(bernstein_value(w, t))
                        .Subtracted(bernstein_value(x, t).Multiplied(bernstein_value(dw, t)));
                worst_identity =
                    std::max(worst_identity, reference.Subtracted(bernstein_value(q, t)).Modulus());
            }
            // Independent central difference: C'(t) == Q(t)/W(t)^2.
            const double h = 1e-6;
            for (int s = 1; s < 30; ++s) {
                const double t = static_cast<double>(s) / 30.0;
                const gp_Pnt ahead = rational_bezier_point(span.poles, span.weights, t + h);
                const gp_Pnt behind = rational_bezier_point(span.poles, span.weights, t - h);
                const gp_XYZ numeric = ahead.XYZ().Subtracted(behind.XYZ()).Divided(2.0 * h);
                const double denominator = bernstein_value(w, t) * bernstein_value(w, t);
                const gp_XYZ analytic = bernstein_value(q, t).Divided(denominator);
                const double scale = std::max(1.0, analytic.Modulus());
                worst_finite_difference = std::max(
                    worst_finite_difference, numeric.Subtracted(analytic).Modulus() / scale);
            }
        }
    }
    std::fprintf(stderr,
                 "TEST-CURVE-05: derivative-numerator identity max abs error %.3e, central "
                 "difference max relative error %.3e\n",
                 worst_identity, worst_finite_difference);
    check(worst_identity < 1e-10,
          "TEST-CURVE-05: derivative_numerator_bernstein matches an independent Bernstein "
          "evaluation to 1e-10");
    check(worst_finite_difference < 1e-6,
          "TEST-CURVE-05: derivative_numerator_bernstein matches a central-difference rational "
          "derivative");
}

// ---------------------------------------------------------------------------
// TEST-CURVE-06 — budgets, cancellation, and the quality-limited path
// ---------------------------------------------------------------------------

void test_curve_06_segment_budget() {
    Handle(Geom_BSplineCurve) spline = oscillating_spline();
    const TopoDS_Edge edge = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(spline)).Edge();
    CurveSampleLimits limits;
    limits.maxSegmentsPerEdge = 64;
    const CurveSampleResult result = sample(edge, 1e-6, kFiveDegrees, limits);
    std::fprintf(stderr, "TEST-CURVE-06: segment cap 64 -> certification %d (%s)\n",
                 static_cast<int>(result.certification), result.diagnostic.c_str());
    check(result.certification == CurveCertification::QualityLimited,
          "TEST-CURVE-06: hitting the per-edge segment cap reports QualityLimited");
    check(result.certifiedChordBoundMm < 0.0,
          "TEST-CURVE-06: a quality-limited result carries no certified bound");
}

void test_curve_06_depth_cap() {
    const TopoDS_Edge edge = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(s_curve_bezier())).Edge();
    CurveSampleLimits limits;
    limits.maxDepthPerSpan = 1;
    const CurveSampleResult result = sample(edge, 1e-9, kFiveDegrees, limits);
    std::fprintf(stderr, "TEST-CURVE-06: depth cap 1 -> certification %d (%s)\n",
                 static_cast<int>(result.certification), result.diagnostic.c_str());
    check(result.certification == CurveCertification::QualityLimited,
          "TEST-CURVE-06: hitting the per-span depth cap reports QualityLimited");
}

void test_curve_06_cancellation() {
    const TopoDS_Edge edge = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(s_curve_bezier())).Edge();
    int polls = 0;
    const CurveSampleResult result =
        sample(edge, 0.001, kFiveDegrees, CurveSampleLimits{}, [&polls]() {
            ++polls;
            return true;
        });
    std::fprintf(stderr,
                 "TEST-CURVE-06: cancellation honoured after %d poll(s), certification %d\n", polls,
                 static_cast<int>(result.certification));
    check(polls == 1, "TEST-CURVE-06: cancellation is honoured within the first span");
    check(result.certification == CurveCertification::Failed && result.diagnostic == "cancelled",
          "TEST-CURVE-06: a cancelled sample is typed, not silently truncated");
    check(result.points.empty(), "TEST-CURVE-06: a cancelled sample publishes no partial polyline");
}

void test_curve_06_tessellate_quality_limited_path() {
    // Round 2 (F4): the per-span per-AXIS allowance at x = 5000 is
    // 0.5*ulp(5001) = 0.000244 mm, comfortably inside the 0.0016117 mm fine
    // deflection for this edge's 3.2234 mm bbox diagonal, so this edge is now
    // Certified with no doubling at all. The blanket maxAbs*4*2^-23 bound the
    // first round used made it 0.0023847 mm and starved the tier.
    const double shift = 5000.0;
    const TopoDS_Edge edge =
        BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(s_curve_bezier(shift))).Edge();
    const onecad::tess::BodyMesh mesh =
        onecad::tess::tessellate_body(edge, "body_far_scurve", "fine", true, nullptr);
    const std::vector<gp_Pnt> poly = edge_polyline(mesh.blob, 0);
    const double achieved = 0.0016117;  // the fine deflection itself, undoubled
    const double worst =
        dense_deviation([&](double t) { return s_curve_point(t, shift); }, 0.0, 1.0, poly, 4000);
    std::fprintf(stderr,
                 "TEST-CURVE-06: per-axis allowance at x=5000 -> %zu points, dense max error "
                 "%.6f mm against the undoubled fine tolerance %.6f mm\n",
                 poly.size(), worst, achieved);
    check(poly.size() > 2,
          "TEST-CURVE-06: the far edge still exports a real polyline, not endpoints");
    check(worst <= achieved,
          "TEST-CURVE-06: the per-axis allowance keeps x=5000 inside the UNDOUBLED fine "
          "tolerance (measured " +
              std::to_string(worst) + " mm)");
}

// ---------------------------------------------------------------------------
// TEST-CURVE-07 — the fixed approximate fallback
// ---------------------------------------------------------------------------

void test_curve_07_offset_curve_fallback() {
    // Geom_OffsetCurve refuses a C0 basis, so the offset rides the smooth C2
    // spline. An offset curve has no exact rational Bezier form, which is what
    // routes it to the NUM §2.7 fallback.
    Handle(Geom_BSplineCurve) basis = smooth_spline();
    Handle(Geom_OffsetCurve) offset = new Geom_OffsetCurve(basis, 0.5, gp_Dir(0, 0, 1));
    const TopoDS_Edge edge = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(offset), 0.0, 3.0).Edge();
    const double tolerance = 0.05;
    const CurveSampleResult result = sample(edge, tolerance);

    std::fprintf(stderr,
                 "TEST-CURVE-07: offset curve -> certification %d, %zu points, sampledMaxError "
                 "%.3e mm, certifiedChordBound %.3e (%s)\n",
                 static_cast<int>(result.certification), result.points.size(),
                 result.sampledMaxErrorMm, result.certifiedChordBoundMm, result.diagnostic.c_str());
    check(result.certification == CurveCertification::KernelEstimated,
          "TEST-CURVE-07: an offset curve is routed through the approximate fallback and stays "
          "KernelEstimated");
    check(result.certifiedChordBoundMm < 0.0,
          "TEST-CURVE-07: sampling never upgrades an approximation to a certified bound");
    check(result.sampledMaxErrorMm >= 0.0 && result.sampledMaxErrorMm <= tolerance,
          "TEST-CURVE-07: the fallback's own independent check is inside the requested tolerance");

    const double worst = dense_deviation_of_edge(edge, result.points, 8000);
    check(worst <= tolerance,
          "TEST-CURVE-07: an independent dense evaluation of the ORIGINAL offset curve is inside "
          "the requested tolerance (measured " +
              std::to_string(worst) + " mm)");

    // The refusal branch: an unreachable tolerance must be typed, never
    // relabelled as an approximation that happened to pass its own samples.
    for (const double tight : {1e-9, 1e-12, 1e-15}) {
        const CurveSampleResult r = sample(edge, tight);
        std::fprintf(stderr, "TEST-CURVE-07: at %.0e mm -> certification %d, sampled %.3e (%s)\n",
                     tight, static_cast<int>(r.certification), r.sampledMaxErrorMm,
                     r.diagnostic.c_str());
        check(r.certification != CurveCertification::Certified,
              "TEST-CURVE-07: an approximation is never relabelled Certified, at any tolerance");
        check(r.certification != CurveCertification::KernelEstimated ||
                  (r.sampledMaxErrorMm >= 0.0 && r.sampledMaxErrorMm <= tight),
              "TEST-CURVE-07: a KernelEstimated result always carries evidence inside its own "
              "tolerance");
        check(!r.diagnostic.empty(),
              "TEST-CURVE-07: every outcome names what happened instead of failing silently");
    }
}

// A REPRESENTATION-limited edge still draws. At x = 200000 the float32 rounding
// is 0.0078 mm against a 0.0016 mm fine budget, so the requested accuracy is
// unreachable in the output format — but the polyline is certified in double and
// is the best float32 can carry. Reducing it to two endpoints would swap a
// bounded error for an unbounded one, so the doubling ladder and the
// endpoints-only rung are reserved for genuine WORK caps.
void test_curve_06_representation_limited_edge_still_draws() {
    const double shift = 200000.0;
    const TopoDS_Edge edge =
        BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(s_curve_bezier(shift))).Edge();
    const onecad::tess::BodyMesh mesh =
        onecad::tess::tessellate_body(edge, "body_very_far_scurve", "fine", true, nullptr);
    const std::vector<gp_Pnt> poly = edge_polyline(mesh.blob, 0);
    const double worst =
        dense_deviation([&](double t) { return s_curve_point(t, shift); }, 0.0, 1.0, poly, 4000);
    std::fprintf(stderr,
                 "TEST-CURVE-06: representation-limited edge at x=2e5 -> %zu points, dense max "
                 "error %.6f mm\n",
                 poly.size(), worst);
    check(poly.size() > 2,
          "TEST-CURVE-06: a representation-limited edge keeps its polyline instead of collapsing "
          "to two endpoints");
    check(worst <= 0.02,
          "TEST-CURVE-06: that polyline is still bounded by the float32 rounding at 2e5 mm "
          "(measured " +
              std::to_string(worst) + " mm)");

    BRepAdaptor_Curve adaptor(edge);
    CurveSampleRequest request;
    request.chordToleranceMm = 0.0016117;
    request.angularToleranceRad = kFiveDegrees;
    const CurveSampleResult r = sample_edge_curve(adaptor, request, CurveSampleLimits{}, {});
    std::fprintf(stderr,
                 "TEST-CURVE-06: representation-limited direct -> certification %d, %zu points, "
                 "quantization %.3e mm (%s)\n",
                 static_cast<int>(r.certification), r.points.size(), r.quantizationErrorMm,
                 r.diagnostic.c_str());
    check(r.certification == CurveCertification::QualityLimited && !r.points.empty(),
          "TEST-CURVE-06: a representation limit is QualityLimited WITH points, the contract that "
          "separates it from a work cap");
    check(r.certifiedChordBoundMm < 0.0 && r.quantizationErrorMm > 0.0,
          "TEST-CURVE-06: it carries no chord certificate but does carry the measured "
          "quantization");
    check(r.diagnostic.find("quantization") != std::string::npos,
          "TEST-CURVE-06: the diagnostic names the measured quantization");
}


// ===========================================================================
// ROUND 2 — fixes demanded by the local adversarial review and the Astra break
// ===========================================================================

bool has_leaf_boundary_at(const CurveSampleResult& r, double t) {
    for (const CurveLeaf& leaf : r.leaves) {
        if (std::abs(leaf.t1 - t) <= 1e-12) return true;
    }
    return false;
}

bool leaf_crosses(const CurveSampleResult& r, double t) {
    for (const CurveLeaf& leaf : r.leaves) {
        if (leaf.t0 < t - 1e-12 && leaf.t1 > t + 1e-12) return true;
    }
    return false;
}

bool lists_singular_parameter(const CurveSampleResult& r, double t) {
    for (double s : r.singularParameters) {
        if (std::abs(s - t) <= 1e-12) return true;
    }
    return false;
}

Handle(Geom_BezierCurve) polynomial_cubic(const gp_Pnt& p0, const gp_Pnt& p1, const gp_Pnt& p2,
                                          const gp_Pnt& p3) {
    NCollection_Array1<gp_Pnt> poles(1, 4);
    poles.SetValue(1, p0);
    poles.SetValue(2, p1);
    poles.SetValue(3, p2);
    poles.SetValue(4, p3);
    return new Geom_BezierCurve(poles);
}

// --- ITEM 1: an interior stationary point must not sink the whole edge -------

// Cubic with an EXACT cusp at t = 0.5: C'(0.5) = 3[0.25(P1-P0) + 0.5(P2-P1) +
// 0.25(P3-P2)] = 0. Subdivision puts the singularity on a child boundary, where
// `at_source_*` is false — the acceptance gate that must go.
void test_curve_05_interior_cusp_at_a_dyadic_parameter() {
    Handle(Geom_BezierCurve) bezier = polynomial_cubic(
        gp_Pnt(0, 0, 0), gp_Pnt(1, 1, 0), gp_Pnt(0, 1, 0), gp_Pnt(1, 0, 0));
    const TopoDS_Edge edge = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(bezier)).Edge();

    std::vector<gp_XYZ> poles;
    std::vector<double> weights;
    for (int i = 1; i <= 4; ++i) {
        poles.push_back(bezier->Pole(i).XYZ());
        weights.push_back(1.0);
    }
    auto analytic = [&](double s) { return rational_bezier_point(poles, weights, s); };

    const onecad::tess::BodyMesh mesh =
        onecad::tess::tessellate_body(edge, "body_cusp", "fine", true, nullptr);
    const std::vector<gp_Pnt> poly = edge_polyline(mesh.blob, 0);
    const double worst = dense_deviation(analytic, 0.0, 1.0, poly, 8000);
    std::fprintf(stderr,
                 "TEST-CURVE-05: interior cusp at t=0.5 -> %zu MESH1 points, dense max error "
                 "%.6f mm (fine budget ~0.000707 mm)\n",
                 poly.size(), worst);
    check(poly.size() > 2,
          "TEST-CURVE-05: an interior cusp must not collapse the edge to its two endpoints");
    check(worst <= 0.000708,
          "TEST-CURVE-05: the cusped cubic is sampled inside the fine chord budget (measured " +
              std::to_string(worst) + " mm)");

    const CurveSampleResult r = sample(edge, 0.001);
    check(r.certification == CurveCertification::Certified,
          "TEST-CURVE-05: an interior cusp still yields a certified chord bound");
    check(has_leaf_boundary_at(r, 0.5) && !leaf_crosses(r, 0.5),
          "TEST-CURVE-05: the cusp sits exactly on a leaf boundary and no leaf spans it");
    check(r.angularUndefinedSomewhere,
          "TEST-CURVE-05: the undefined tangent at the cusp is reported");
    check(lists_singular_parameter(r, 0.5),
          "TEST-CURVE-05: the cusp parameter is published as an explicit singular vertex");
}

// Straight LOCUS with a stationary point: C(t) = ((2t-1)^3, 0). Q has a root at
// t = 0.5, so the old gate refuses a perfectly straight edge.
void test_curve_05_straight_locus_with_interior_root() {
    Handle(Geom_BezierCurve) bezier = polynomial_cubic(
        gp_Pnt(-1, 0, 0), gp_Pnt(1, 0, 0), gp_Pnt(-1, 0, 0), gp_Pnt(1, 0, 0));
    const TopoDS_Edge edge = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(bezier)).Edge();
    const CurveSampleResult r = sample(edge, 0.001);
    std::fprintf(stderr,
                 "TEST-CURVE-05: straight locus ((2t-1)^3,0) -> certification %d, %zu points, "
                 "%zu leaves (%s)\n",
                 static_cast<int>(r.certification), r.points.size(), r.leaves.size(),
                 r.diagnostic.c_str());
    check(r.certification == CurveCertification::Certified,
          "TEST-CURVE-05: a straight locus with an interior derivative root is still Certified");
    check(has_leaf_boundary_at(r, 0.5),
          "TEST-CURVE-05: the straight locus splits at its stationary parameter");
    auto analytic = [](double s) {
        const double x = (2.0 * s - 1.0);
        return gp_Pnt(x * x * x, 0.0, 0.0);
    };
    const double worst = dense_deviation(analytic, 0.0, 1.0, r.points, 8000);
    check(worst <= 0.001, "TEST-CURVE-05: the straight locus is reproduced exactly (measured " +
                              std::to_string(worst) + " mm)");
}

// A stationary point at t* = 0.4 never lands on a dyadic boundary, so the span
// bracketing it can never satisfy the cone. It must still converge on the chord
// certificate rather than run to the depth cap.
void test_curve_05_non_dyadic_stationary_point() {
    // C'(0.4) = 0 with A = P1-P0 = (1,1), C = P3-P2 = (1,-1):
    //   0.36 A + 0.48 B + 0.16 C = 0  =>  B = -(0.52, 0.20)/0.48.
    const gp_Pnt p0(0.0, 0.0, 0.0);
    const gp_Pnt p1(1.0, 1.0, 0.0);
    const gp_Pnt p2(1.0 - 0.52 / 0.48, 1.0 - 0.20 / 0.48, 0.0);
    const gp_Pnt p3(p2.X() + 1.0, p2.Y() - 1.0, 0.0);
    Handle(Geom_BezierCurve) bezier = polynomial_cubic(p0, p1, p2, p3);
    const TopoDS_Edge edge = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(bezier)).Edge();

    std::vector<gp_XYZ> poles;
    std::vector<double> weights;
    for (int i = 1; i <= 4; ++i) {
        poles.push_back(bezier->Pole(i).XYZ());
        weights.push_back(1.0);
    }
    const CurveSampleResult r = sample(edge, 0.002);
    const double worst = dense_deviation(
        [&](double s) { return rational_bezier_point(poles, weights, s); }, 0.0, 1.0, r.points,
        8000);
    std::fprintf(stderr,
                 "TEST-CURVE-05: non-dyadic stationary point t*=0.4 -> certification %d, %zu "
                 "points, dense max error %.3e mm (%s)\n",
                 static_cast<int>(r.certification), r.points.size(), worst, r.diagnostic.c_str());
    check(r.certification == CurveCertification::Certified,
          "TEST-CURVE-05: a non-dyadic stationary point converges instead of hitting the cap");
    check(worst <= 0.002,
          "TEST-CURVE-05: the non-dyadic stationary fixture is inside its chord budget (measured " +
              std::to_string(worst) + " mm)");
}

// --- ITEM 2: a sub-floor coefficient is not evidence of anything -------------

void test_curve_05_subfloor_coefficient_never_certifies() {
    // Cubic (0,0,0),(3,0,0),(3.001,0,0),(4,0,0): a straight locus that nearly
    // stalls in the middle. Its six derivative-numerator coefficients have
    // relative magnitudes 1, 0.60, 0.33, 0.20, 0.20, 0.33, so a test floor of
    // 0.25 puts exactly the two INTERIOR ones below it while every remaining
    // coefficient still clears the parallel threshold. Dropping those two and
    // reporting Satisfied is a false angular certificate.
    RationalBezierSpan span;
    span.poles = {gp_XYZ(0, 0, 0), gp_XYZ(3, 0, 0), gp_XYZ(3.001, 0, 0), gp_XYZ(4, 0, 0)};
    span.weights = {1.0, 1.0, 1.0, 1.0};
    const std::vector<gp_XYZ> q = onecad::tess::derivative_numerator_bernstein(span);
    double qmax = 0.0;
    for (const gp_XYZ& c : q) qmax = std::max(qmax, c.Modulus());
    std::fprintf(stderr, "TEST-CURVE-05: sub-floor probe relative magnitudes");
    for (const gp_XYZ& c : q) std::fprintf(stderr, " %.6f", c.Modulus() / qmax);
    std::fprintf(stderr, "\n");
    const AngularStatus status = onecad::tess::tangent_cone_status(span, 0.3, 0.25);
    check(status == AngularStatus::Undefined,
          "TEST-CURVE-05: a NONZERO INTERIOR coefficient below the test floor downgrades the "
          "span to Undefined, never Satisfied");

    // Astra's rational quadratic: position is fine, the tangent at t=1e-6 is not.
    NCollection_Array1<gp_Pnt> poles(1, 3);
    poles.SetValue(1, gp_Pnt(0.0, 0.0, 0.0));
    poles.SetValue(2, gp_Pnt(0.0, 0.018, 0.0));
    poles.SetValue(3, gp_Pnt(20.0, 0.0, 0.0));
    NCollection_Array1<double> weights(1, 3);
    weights.SetValue(1, 1e-6);
    weights.SetValue(2, 1.0);
    weights.SetValue(3, 1.0);
    Handle(Geom_BezierCurve) bezier = new Geom_BezierCurve(poles, weights);
    std::vector<gp_XYZ> p;
    std::vector<double> w;
    for (int i = 1; i <= 3; ++i) {
        p.push_back(poles.Value(i).XYZ());
        w.push_back(weights.Value(i));
    }
    const gp_Pnt ahead = rational_bezier_point(p, w, 1e-6 + 1e-9);
    const gp_Pnt behind = rational_bezier_point(p, w, 1e-6 - 1e-9);
    const gp_XYZ tangent = ahead.XYZ().Subtracted(behind.XYZ());
    const double degrees =
        std::acos(std::clamp(tangent.Normalized().Dot(gp_XYZ(1, 0, 0)), -1.0, 1.0)) * 180.0 / kPi;

    const TopoDS_Edge edge = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(bezier)).Edge();
    const CurveSampleResult r = sample(edge, 0.05);
    std::fprintf(stderr,
                 "TEST-CURVE-05: Astra quadratic -> certification %d, %zu leaves, first leaf "
                 "angular %d, independent tangent at t=1e-6 is %.2f deg off the chord\n",
                 static_cast<int>(r.certification), r.leaves.size(),
                 r.leaves.empty() ? -1 : static_cast<int>(r.leaves.front().angular), degrees);
    check(degrees > 80.0,
          "TEST-CURVE-05: the fixture really does swing the tangent near t=0 (measured " +
              std::to_string(degrees) + " deg)");
    check(r.points.size() >= 2, "TEST-CURVE-05: the Astra quadratic is accepted on its chord");
    check(r.angularUndefinedSomewhere,
          "TEST-CURVE-05: the Astra quadratic reports that angular evidence was not established");
    check(!r.leaves.empty() && r.leaves.front().angular != AngularStatus::Satisfied,
          "TEST-CURVE-05: the leaf holding the sub-floor coefficient is never Satisfied");
    check(!r.leaves.empty() && !r.leaves.front().singularVertexAtStart,
          "TEST-CURVE-05: a small-but-nonzero derivative is NOT a singular vertex");

    // delta = 0.022 pushes q0's relative magnitude just ABOVE the 1e-9 test
    // floor, so it becomes testable, fails the cone (it is perpendicular to the
    // chord) and the span is subdivided until the near-cusp region is genuinely
    // resolved. Both variants are honest; what must hold either way is that the
    // result is complete, inside its chord budget, and never claims Satisfied on
    // evidence it does not have.
    poles.SetValue(2, gp_Pnt(0.0, 0.022, 0.0));
    Handle(Geom_BezierCurve) wider = new Geom_BezierCurve(poles, weights);
    const TopoDS_Edge wide_edge = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(wider)).Edge();
    const CurveSampleResult rw = sample(wide_edge, 0.05);
    std::vector<gp_XYZ> pw;
    for (int i = 1; i <= 3; ++i) pw.push_back(poles.Value(i).XYZ());
    const double wide_error = dense_deviation(
        [&](double s) { return rational_bezier_point(pw, w, s); }, 0.0, 1.0, rw.points, 8000);
    std::fprintf(stderr,
                 "TEST-CURVE-05: delta=0.022 variant -> certification %d, %zu points, %zu "
                 "leaves, first leaf angular %d, undefined-somewhere %d, dense max error "
                 "%.3e mm\n",
                 static_cast<int>(rw.certification), rw.points.size(), rw.leaves.size(),
                 rw.leaves.empty() ? -1 : static_cast<int>(rw.leaves.front().angular),
                 rw.angularUndefinedSomewhere ? 1 : 0, wide_error);
    check(rw.certification == CurveCertification::Certified && rw.points.size() >= 2,
          "TEST-CURVE-05: the delta=0.022 variant is complete and certified");
    check(wide_error <= 0.05,
          "TEST-CURVE-05: the delta=0.022 variant stays inside its chord budget (measured " +
              std::to_string(wide_error) + " mm)");
}

// --- ITEM 3: a full conic whose range does not start at the basis origin -----

void test_curve_02_full_circle_with_a_shifted_range() {
    const double radius = 20.0;
    Handle(Geom_Circle) circle = new Geom_Circle(gp_Ax2(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1)), radius);
    const TopoDS_Edge edge =
        BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(circle), 1.0, 1.0 + 2.0 * kPi).Edge();
    const CurveSampleResult r = sample(edge, 0.01);
    auto analytic = [&](double t) {
        return gp_Pnt(radius * std::cos(t), radius * std::sin(t), 0.0);
    };
    double radial = 0.0;
    for (const gp_Pnt& q : r.points) {
        radial = std::max(radial, std::abs(q.Distance(gp_Pnt(0, 0, 0)) - radius));
    }
    std::fprintf(stderr,
                 "TEST-CURVE-02: full circle over [1, 1+2pi] -> certification %d, %zu points, "
                 "radial error %.3e mm (%s)\n",
                 static_cast<int>(r.certification), r.points.size(), radial, r.diagnostic.c_str());
    check(r.certification == CurveCertification::Certified,
          "TEST-CURVE-02: a full circle whose range starts away from u=0 must not vanish");
    check(r.points.size() >= 65, "TEST-CURVE-02: the shifted full circle emits a real polyline");
    check(r.points.size() >= 2 && r.points.front().Distance(analytic(1.0)) <= 1e-9,
          "TEST-CURVE-02: the shifted full circle starts at C(1.0), preserving its phase");
    check(r.points.size() >= 2 && r.points.front().Distance(r.points.back()) == 0.0,
          "TEST-CURVE-02: the shifted full circle keeps exact closure");
    check(radial <= 1e-9, "TEST-CURVE-02: shifted full circle points lie on the analytic circle");
}

// --- ITEM 4: a periodic B-spline sampled across its seam ---------------------

// Degree-3 uniform periodic spline, 8 poles, period 8.
Handle(Geom_BSplineCurve) periodic_spline() {
    NCollection_Array1<gp_Pnt> poles(1, 8);
    const double r = 10.0;
    for (int i = 1; i <= 8; ++i) {
        const double a = 2.0 * kPi * static_cast<double>(i - 1) / 8.0;
        poles.SetValue(i, gp_Pnt(r * std::cos(a), r * std::sin(a), 0.3 * static_cast<double>(i)));
    }
    NCollection_Array1<double> knots(1, 9);
    NCollection_Array1<int> mults(1, 9);
    for (int i = 1; i <= 9; ++i) {
        knots.SetValue(i, static_cast<double>(i - 1));
        mults.SetValue(i, 1);
    }
    return new Geom_BSplineCurve(poles, knots, mults, 3, Standard_True);
}

void test_curve_03_periodic_bspline_across_the_seam() {
    Handle(Geom_BSplineCurve) spline = periodic_spline();
    const TopoDS_Edge edge =
        BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(spline), 6.5, 9.5).Edge();
    BRepAdaptor_Curve adaptor(edge);
    std::fprintf(stderr, "TEST-CURVE-03: periodic seam adaptor domain [%.3f, %.3f]\n",
                 adaptor.FirstParameter(), adaptor.LastParameter());
    check(std::abs(adaptor.FirstParameter() - 6.5) <= 1e-12 &&
              std::abs(adaptor.LastParameter() - 9.5) <= 1e-12,
          "TEST-CURVE-03: the adaptor reports the requested wrapped range");

    const CurveSampleResult r = sample(edge, 0.01);
    const double worst = dense_deviation_of_edge(edge, r.points, 8000);
    std::fprintf(stderr,
                 "TEST-CURVE-03: periodic seam [6.5, 9.5] -> certification %d, %zu points, dense "
                 "max error %.3e mm (%s)\n",
                 static_cast<int>(r.certification), r.points.size(), worst, r.diagnostic.c_str());
    check(r.certification == CurveCertification::Certified,
          "TEST-CURVE-03: a periodic spline sampled across its seam is Certified");
    check(worst <= 0.01,
          "TEST-CURVE-03: the wrapped range is covered end to end, not silently truncated "
          "(measured " +
              std::to_string(worst) + " mm)");
    check(r.points.size() >= 2 &&
              r.points.front().Distance(adaptor.Value(6.5)) <= 1e-9 &&
              r.points.back().Distance(adaptor.Value(9.5)) <= 1e-9,
          "TEST-CURVE-03: the wrapped range keeps both analytic endpoints");
    check(leaves_are_contiguous_and_increasing(r.leaves),
          "TEST-CURVE-03: leaf parameters stay contiguous and increasing across the seam");
}

// Astra's three-arc C1 periodic spline, period [0,3]; the adaptor interval [2,4]
// covers A2 then wraps into A0, whose point at local s = 1/3 is (40/9, 20/9).
Handle(Geom_BSplineCurve) three_arc_periodic_spline() {
    NCollection_Array1<gp_Pnt> poles(1, 9);
    poles.SetValue(1, gp_Pnt(0, 0, 0));
    poles.SetValue(2, gp_Pnt(10, 0, 0));
    poles.SetValue(3, gp_Pnt(0, 10, 0));
    poles.SetValue(4, gp_Pnt(0, 0, 0));
    poles.SetValue(5, gp_Pnt(0, -10, 0));
    poles.SetValue(6, gp_Pnt(-10, -10, 0));
    poles.SetValue(7, gp_Pnt(-10, 0, 0));
    poles.SetValue(8, gp_Pnt(-10, 10, 0));
    poles.SetValue(9, gp_Pnt(-10, 0, 0));
    NCollection_Array1<double> knots(1, 4);
    NCollection_Array1<int> mults(1, 4);
    for (int i = 1; i <= 4; ++i) {
        knots.SetValue(i, static_cast<double>(i - 1));
        mults.SetValue(i, 3);
    }
    return new Geom_BSplineCurve(poles, knots, mults, 3, Standard_True);
}

void test_curve_03_periodic_three_arc_wrap() {
    Handle(Geom_BSplineCurve) spline = three_arc_periodic_spline();
    const TopoDS_Edge edge = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(spline), 2.0, 4.0).Edge();
    BRepAdaptor_Curve adaptor(edge);
    std::fprintf(stderr, "TEST-CURVE-03: three-arc adaptor domain [%.3f, %.3f], basis [%.3f, %.3f]\n",
                 adaptor.FirstParameter(), adaptor.LastParameter(), spline->FirstParameter(),
                 spline->LastParameter());
    check(std::abs(adaptor.FirstParameter() - 2.0) <= 1e-12 &&
              std::abs(adaptor.LastParameter() - 4.0) <= 1e-12,
          "TEST-CURVE-03: the three-arc adaptor reports [2, 4]");

    const double tolerance = 0.05;
    const CurveSampleResult r = sample(edge, tolerance);
    const gp_Pnt target(40.0 / 9.0, 20.0 / 9.0, 0.0);
    const double reach = distance_to_polyline(target, r.points);
    const double worst = dense_deviation_of_edge(edge, r.points, 8000);
    std::fprintf(stderr,
                 "TEST-CURVE-03: three-arc wrap [2,4] -> certification %d, %zu points, distance to "
                 "A0(1/3) = %.3e mm, dense max error %.3e mm (%s)\n",
                 static_cast<int>(r.certification), r.points.size(), reach, worst,
                 r.diagnostic.c_str());
    check(r.certification == CurveCertification::Certified,
          "TEST-CURVE-03: the wrapped three-arc range is Certified");
    check(reach <= tolerance,
          "TEST-CURVE-03: the wrapped part of the range really is sampled — A0's point "
          "(40/9, 20/9) is on the polyline (measured " +
              std::to_string(reach) + " mm)");
    check(worst <= tolerance, "TEST-CURVE-03: the whole wrapped range is inside the chord bound");
}

// --- ITEM 5: representation allowance is per span, per axis, and measured ----

void test_curve_06_per_axis_representation_allowance() {
    // X = 1e9 is EXACTLY representable in float32 (1e9 = 1953125 * 2^9, and
    // 1953125 < 2^21), so the circle's real quantization error comes only from
    // the 15 mm Y/Z components. A blanket ulp(maxAbs) allowance would starve it.
    Handle(Geom_Circle) circle =
        new Geom_Circle(gp_Ax2(gp_Pnt(1e9, 0, 0), gp_Dir(1, 0, 0)), 15.0);
    const TopoDS_Edge edge = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(circle)).Edge();
    const onecad::tess::BodyMesh mesh =
        onecad::tess::tessellate_body(edge, "body_far_circle", "fine", true, nullptr);
    const std::vector<gp_Pnt> poly = edge_polyline(mesh.blob, 0);

    double radial = 0.0;
    for (const gp_Pnt& q : poly) {
        const double dy = q.Y();
        const double dz = q.Z();
        radial = std::max(radial, std::abs(std::sqrt(dy * dy + dz * dz) - 15.0));
    }
    std::fprintf(stderr,
                 "TEST-CURVE-06: circle at X=1e9 -> %zu MESH1 points, radial error %.3e mm\n",
                 poly.size(), radial);
    check(poly.size() >= 65,
          "TEST-CURVE-06: a far-from-origin circle whose own axis is exactly representable is "
          "still sampled, not reduced to endpoints");
    check(radial <= 1e-5,
          "TEST-CURVE-06: the measured float32 error of that circle is tiny (measured " +
              std::to_string(radial) + " mm)");

    const CurveSampleResult r = sample(edge, 0.02);
    std::fprintf(stderr, "TEST-CURVE-06: X=1e9 circle direct -> quantizationErrorMm %.3e\n",
                 r.quantizationErrorMm);
    check(r.quantizationErrorMm >= 0.0 && r.quantizationErrorMm < 1e-5,
          "TEST-CURVE-06: the MEASURED float32 quantization is reported and is per-axis honest");
}

void test_curve_06_short_chord_angular_downgrade() {
    // A = (100000,0,0) -> B = (100000.003, 0.001, 0). float32 rounding at 1e5 is
    // about 0.0039 mm, larger than the whole 0.00316 mm chord, so the encoded
    // segment's DIRECTION is noise. The angular claim must be withdrawn.
    const gp_Pnt a(100000.0, 0.0, 0.0);
    const gp_Pnt b(100000.003, 0.001, 0.0);
    const TopoDS_Edge edge = BRepBuilderAPI_MakeEdge(a, b).Edge();
    const CurveSampleResult r = sample(edge, 0.05);
    std::fprintf(stderr,
                 "TEST-CURVE-06: 1e5 short line -> certification %d, %zu leaves, angular %d, "
                 "chordLength %.3e, quantization %.3e\n",
                 static_cast<int>(r.certification), r.leaves.size(),
                 r.leaves.empty() ? -1 : static_cast<int>(r.leaves.front().angular),
                 r.leaves.empty() ? -1.0 : r.leaves.front().chordLengthMm, r.quantizationErrorMm);
    check(!r.leaves.empty() && r.leaves.front().angular == AngularStatus::Undefined,
          "TEST-CURVE-06: a chord shorter than its own float32 rounding reports no angular "
          "evidence");
    check(r.angularUndefinedSomewhere,
          "TEST-CURVE-06: the withdrawn angular claim is visible on the result");
}

// --- ITEM 6: degenerate weights must not veto a provable chord ---------------

void test_curve_04_degenerate_weights_on_a_collinear_span() {
    NCollection_Array1<gp_Pnt> poles(1, 3);
    poles.SetValue(1, gp_Pnt(0.0, 0.0, 0.0));
    poles.SetValue(2, gp_Pnt(0.05, 0.0, 0.0));
    poles.SetValue(3, gp_Pnt(0.1, 0.0, 0.0));
    NCollection_Array1<double> weights(1, 3);
    weights.SetValue(1, 1.0);
    weights.SetValue(2, 1e18);
    weights.SetValue(3, 1e18);
    Handle(Geom_BezierCurve) bezier = new Geom_BezierCurve(poles, weights);
    const TopoDS_Edge edge = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(bezier)).Edge();
    const CurveSampleResult r = sample(edge, 0.001);
    std::fprintf(stderr,
                 "TEST-CURVE-04: collinear span with weights (1, 1e18, 1e18) -> certification %d, "
                 "%zu points, %zu leaves (%s)\n",
                 static_cast<int>(r.certification), r.points.size(), r.leaves.size(),
                 r.diagnostic.c_str());
    check(r.certification == CurveCertification::Certified,
          "TEST-CURVE-04: a collinear locus is certified whatever its weight conditioning");
    check(r.leaves.size() <= 4,
          "TEST-CURVE-04: a collinear locus does not subdivide toward the depth cap");

    // The near-semicircle conditioning pattern still converges normally.
    weights.SetValue(1, 1.0);
    weights.SetValue(2, 1e-7);
    weights.SetValue(3, 1.0);
    poles.SetValue(2, gp_Pnt(0.05, 8.0, 0.0));
    Handle(Geom_BezierCurve) arc = new Geom_BezierCurve(poles, weights);
    const TopoDS_Edge arc_edge = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(arc)).Edge();
    const CurveSampleResult ra = sample(arc_edge, 0.01);
    std::fprintf(stderr, "TEST-CURVE-04: near-semicircle weights (1,1e-7,1) -> certification %d, "
                         "%zu points\n",
                 static_cast<int>(ra.certification), ra.points.size());
    check(ra.certification == CurveCertification::Certified,
          "TEST-CURVE-04: the near-semicircle weight pattern still converges");
}

// --- ITEM 7: closure snapping is semantic, never distance-based --------------

void test_curve_02_open_near_closure_is_not_snapped() {
    // A 1e-5 mm line at 1e9: the two endpoints are far closer than the old
    // 64*eps*1e9 = 1.42e-5 mm snap threshold, yet the edge is OPEN.
    const gp_Pnt a(1e9, 0.0, 0.0);
    const gp_Pnt b(1e9 + 1e-5, 0.0, 0.0);
    const TopoDS_Edge edge = BRepBuilderAPI_MakeEdge(a, b).Edge();
    const CurveSampleResult r = sample(edge, 0.05);
    std::fprintf(stderr,
                 "TEST-CURVE-02: open 1e-5 mm line at 1e9 -> %zu points, endpoint separation "
                 "%.3e mm\n",
                 r.points.size(),
                 r.points.size() >= 2 ? r.points.front().Distance(r.points.back()) : -1.0);
    check(r.points.size() >= 2 && r.points.front().Distance(r.points.back()) > 0.0,
          "TEST-CURVE-02: an OPEN edge is never collapsed into a closed one by distance");
}

// --- ITEM 9: a work cap publishes nothing ------------------------------------

void test_curve_06_work_caps_publish_nothing() {
    const TopoDS_Edge edge = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(s_curve_bezier())).Edge();
    CurveSampleLimits depth;
    depth.maxDepthPerSpan = 1;
    const CurveSampleResult rd = sample(edge, 1e-9, kFiveDegrees, depth);
    check(rd.certification == CurveCertification::QualityLimited && rd.points.empty() &&
              rd.leaves.empty() && rd.certifiedChordBoundMm < 0.0,
          "TEST-CURVE-06: a depth-cap result publishes no points, no leaves and no certificate");

    Handle(Geom_BSplineCurve) spline = oscillating_spline();
    const TopoDS_Edge dense = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(spline)).Edge();
    CurveSampleLimits segments;
    segments.maxSegmentsPerEdge = 64;
    const CurveSampleResult rs = sample(dense, 1e-6, kFiveDegrees, segments);
    check(rs.certification == CurveCertification::QualityLimited && rs.points.empty() &&
              rs.leaves.empty() && rs.certifiedChordBoundMm < 0.0,
          "TEST-CURVE-06: a segment-cap result publishes no points, no leaves and no certificate");

    CurveSampleLimits body;
    body.remainingBodySegments = 1;
    const CurveSampleResult rb = sample(edge, 0.001, kFiveDegrees, body);
    check(rb.certification == CurveCertification::QualityLimited && rb.points.empty(),
          "TEST-CURVE-06: an exhausted body budget publishes no points");
}

// --- ITEM 10: a non-finite curve is refused, never cast to float -------------

void test_curve_06_non_finite_range_is_refused() {
    const TopoDS_Edge edge = BRepBuilderAPI_MakeEdge(gp_Lin(gp_Pnt(0, 0, 0), gp_Dir(1, 0, 0))).Edge();
    const CurveSampleResult r = sample(edge, 0.05);
    std::fprintf(stderr, "TEST-CURVE-06: unbounded line -> certification %d (%s)\n",
                 static_cast<int>(r.certification), r.diagnostic.c_str());
    check(r.certification == CurveCertification::Failed && r.points.empty() &&
              !r.diagnostic.empty(),
          "TEST-CURVE-06: an unbounded (non-finite) edge is a typed refusal, never a float cast");
}

// --- ITEM 14: the located-edge branch ----------------------------------------

gp_Trsf located_test_transform() {
    gp_Trsf mirror;
    mirror.SetMirror(gp_Ax2(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1)));
    gp_Trsf rotate;
    rotate.SetRotation(gp_Ax1(gp_Pnt(0, 0, 0), gp_Dir(0, 1, 0)), 0.7);
    gp_Trsf translate;
    translate.SetTranslation(gp_Vec(13.0, -7.0, 4.0));
    return translate * rotate * mirror;
}

void test_curve_02_located_edges_use_the_transform_branch() {
    const gp_Trsf trsf = located_test_transform();
    const double radius = 20.0;
    Handle(Geom_Circle) circle = new Geom_Circle(gp_Ax2(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1)), radius);
    TopoDS_Edge conic = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(circle), 0.0, 2.0).Edge();
    conic.Move(TopLoc_Location(trsf));
    {
        BRepAdaptor_Curve adaptor(conic);
        check(adaptor.Trsf().Form() != gp_Identity,
              "TEST-CURVE-02: the located conic edge really carries a non-identity transform");
        const CurveSampleResult r = sample(conic, 0.01);
        double worst = 0.0;
        for (int i = 0; i <= 4000; ++i) {
            const double t = 2.0 * static_cast<double>(i) / 4000.0;
            gp_Pnt analytic(radius * std::cos(t), radius * std::sin(t), 0.0);
            analytic.Transform(trsf);
            worst = std::max(worst, distance_to_polyline(analytic, r.points));
        }
        std::fprintf(stderr,
                     "TEST-CURVE-02: located conic -> certification %d, %zu points, transformed "
                     "analytic max error %.3e mm\n",
                     static_cast<int>(r.certification), r.points.size(), worst);
        check(r.certification == CurveCertification::Certified,
              "TEST-CURVE-02: a located conic edge is Certified");
        check(worst <= 0.01,
              "TEST-CURVE-02: the located conic matches the TRANSFORMED analytic circle "
              "(measured " +
                  std::to_string(worst) + " mm)");
    }
    {
        Handle(Geom_BSplineCurve) spline = smooth_spline();
        TopoDS_Edge located = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(spline)).Edge();
        located.Move(TopLoc_Location(trsf));
        BRepAdaptor_Curve adaptor(located);
        check(adaptor.Trsf().Form() != gp_Identity,
              "TEST-CURVE-02: the located spline edge really carries a non-identity transform");
        const CurveSampleResult r = sample(located, 0.01);
        double worst = 0.0;
        for (int i = 0; i <= 4000; ++i) {
            const double t = 3.0 * static_cast<double>(i) / 4000.0;
            gp_Pnt analytic = spline->Value(t);
            analytic.Transform(trsf);
            worst = std::max(worst, distance_to_polyline(analytic, r.points));
        }
        std::fprintf(stderr,
                     "TEST-CURVE-02: located spline -> certification %d, %zu points, transformed "
                     "analytic max error %.3e mm\n",
                     static_cast<int>(r.certification), r.points.size(), worst);
        check(r.certification == CurveCertification::Certified,
              "TEST-CURVE-02: a located spline edge is Certified");
        check(worst <= 1e-9 + 0.01,
              "TEST-CURVE-02: the located spline matches the TRANSFORMED analytic curve "
              "(measured " +
                  std::to_string(worst) + " mm)");
    }
}

}  // namespace

int main() {
    test_curve_01_s_curve_through_mesh1();
    test_curve_01_budget_ladder();
    test_curve_02_circular_arc();
    test_curve_02_ellipse_trim_and_reversal();
    test_curve_02_full_periodic_circle();
    test_curve_02_rational_cubic_and_subdivision();
    test_curve_03_knot_and_continuity_splits();
    test_curve_04_bulge_with_midpoint_on_the_chord();
    test_curve_04_loop_with_coincident_endpoints();
    test_curve_05_stationary_endpoint();
    test_curve_05_nearly_vanishing_derivative();
    test_curve_05_derivative_numerator_crosscheck();
    test_curve_06_segment_budget();
    test_curve_06_depth_cap();
    test_curve_06_cancellation();
    test_curve_06_tessellate_quality_limited_path();
    test_curve_06_representation_limited_edge_still_draws();
    test_curve_07_offset_curve_fallback();
    test_curve_05_interior_cusp_at_a_dyadic_parameter();
    test_curve_05_straight_locus_with_interior_root();
    test_curve_05_non_dyadic_stationary_point();
    test_curve_05_subfloor_coefficient_never_certifies();
    test_curve_02_full_circle_with_a_shifted_range();
    test_curve_03_periodic_bspline_across_the_seam();
    test_curve_03_periodic_three_arc_wrap();
    test_curve_06_per_axis_representation_allowance();
    test_curve_06_short_chord_angular_downgrade();
    test_curve_04_degenerate_weights_on_a_collinear_span();
    test_curve_02_open_near_closure_is_not_snapped();
    test_curve_06_work_caps_publish_nothing();
    test_curve_06_non_finite_range_is_refused();
    test_curve_02_located_edges_use_the_transform_branch();
    if (g_failures == 0) std::fprintf(stderr, "curve_sampler: OK\n");
    return g_failures;
}
