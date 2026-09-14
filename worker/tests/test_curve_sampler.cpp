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
#include <utility>
#include <vector>

#include <BRepAdaptor_Curve.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRep_Builder.hxx>
#include <Geom_BSplineCurve.hxx>
#include <Geom_BezierCurve.hxx>
#include <Geom_Circle.hxx>
#include <Geom_Ellipse.hxx>
#include <Geom_OffsetCurve.hxx>
#include <NCollection_Array1.hxx>
#include <Standard_Failure.hxx>
#include <TopExp.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Compound.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Vertex.hxx>
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

std::string num_string(double v) {
    char buf[32];
    std::snprintf(buf, sizeof(buf), "%.6g", v);
    return std::string(buf);
}

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

double degrees(double radians) { return radians * 180.0 / kPi; }

gp_XYZ enc32(const gp_Pnt& p) {
    return gp_XYZ(static_cast<double>(static_cast<float>(p.X())),
                  static_cast<double>(static_cast<float>(p.Y())),
                  static_cast<double>(static_cast<float>(p.Z())));
}

double angle_between(const gp_XYZ& a, const gp_XYZ& b) {
    if (!(a.Modulus() > 0.0) || !(b.Modulus() > 0.0)) return -1.0;
    return std::atan2(a.Crossed(b).Modulus(), a.Dot(b));
}

// The turn at join i of the EMITTED float32 polyline, measured here, not read
// from the sampler.
double encoded_join_turn(const std::vector<gp_Pnt>& points, std::size_t i) {
    return angle_between(enc32(points[i]).Subtracted(enc32(points[i - 1])),
                         enc32(points[i + 1]).Subtracted(enc32(points[i])));
}

// Every leaf's published `encodedChordRotationRad` must describe the segment
// that was actually written — recomputed here from `points` alone. This is what
// catches an endpoint that moved after its leaf was certified (followup §2 F7).
void check_encoded_rotation_matches_points(const CurveSampleResult& r, const std::string& label) {
    for (std::size_t k = 0; k + 1 < r.points.size() && k < r.leaves.size(); ++k) {
        const gp_XYZ chord = r.points[k + 1].XYZ().Subtracted(r.points[k].XYZ());
        const gp_XYZ emitted = enc32(r.points[k + 1]).Subtracted(enc32(r.points[k]));
        const double expected = angle_between(chord, emitted);
        if (expected < 0.0) continue;  // a degenerate or coincident pair
        // The sampler and this check compute the angle by different formulas, so
        // the comparison is to within their shared rounding — orders of magnitude
        // tighter than the discrepancy a stale pre-snap value would leave.
        if (std::abs(r.leaves[k].encodedChordRotationRad - expected) > 1e-12 + 1e-9 * expected) {
            check(false, label + ": leaf " + std::to_string(k) +
                             " publishes an encoded rotation of " +
                             std::to_string(r.leaves[k].encodedChordRotationRad) +
                             " rad for a segment that actually rotates " +
                             std::to_string(expected) + " rad");
            return;
        }
    }
}

// The greatest turn of the EMITTED float32 polyline, over every join whose two
// chords have a direction at all. This is the quantity `encodedTurnMaxRad`
// claims to be, measured here from the published points and nothing else.
double independent_encoded_turn_max(const std::vector<gp_Pnt>& points) {
    double worst = -1.0;
    for (std::size_t i = 1; i + 1 < points.size(); ++i) {
        const double turn = encoded_join_turn(points, i);
        if (turn < 0.0) continue;
        worst = std::max(worst, turn);
    }
    return worst;
}

// verify §5 / R1(c) Blocker 1: the emitted turn is EXACT on the emitted points.
// `encodedTurnMaxRad` must equal an independent measurement of it — no join may
// be dropped from the maximum on the grounds that its chords are short.
void check_published_turn_matches_points(const CurveSampleResult& r, const std::string& label) {
    const double measured = independent_encoded_turn_max(r.points);
    if (measured < 0.0 && r.encodedTurnMaxRad < 0.0) return;
    check(std::abs(r.encodedTurnMaxRad - measured) <= 1e-12 + 1e-9 * std::abs(measured),
          label + ": encodedTurnMaxRad " + std::to_string(r.encodedTurnMaxRad) +
              " must equal the independently measured emitted maximum " +
              std::to_string(measured));
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
    check(onecad::tess::tangent_cone_status(span, kFiveDegrees) == AngularStatus::Uncertified,
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
    const AngularStatus status =
        onecad::tess::tangent_cone_status(span, 3.0, undefined_at_start, undefined_at_end);
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

    // ROUND 3 (followup §2 C1/F5): `KernelEstimated` is a PROVENANCE label, not a
    // quality claim. When the same fallback is placed where float32 cannot carry
    // its segments, the requested display quality is not met and the result must
    // say so — the provenance stays in the diagnostic, the certification becomes
    // the claim a consumer acts on.
    TopoDS_Edge far_edge = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(offset), 0.0, 3.0).Edge();
    gp_Trsf shift;
    shift.SetTranslation(gp_Vec(1e6, 0.0, 0.0));
    far_edge.Move(TopLoc_Location(shift));
    const CurveSampleResult far_result = sample(far_edge, tolerance);
    std::fprintf(stderr,
                 "TEST-CURVE-07: offset curve at x=1e6 -> certification %d, %zu points, "
                 "quantization %.3e mm (%s)\n",
                 static_cast<int>(far_result.certification), far_result.points.size(),
                 far_result.quantizationErrorMm, far_result.diagnostic.c_str());
    check(far_result.certification == CurveCertification::KernelEstimated &&
              far_result.qualityLimited,
          "TEST-CURVE-07: an approximation whose emitted segments cannot carry the requested "
          "angular quality keeps its PROVENANCE and reports the shortfall alongside it "
          "(R1(c) MINOR 7), rather than having KernelEstimated overwritten");
    check(!far_result.points.empty(),
          "TEST-CURVE-07: and it still publishes its polyline — QualityLimited WITH points");
    check(far_result.diagnostic.find("ApproxCurve") != std::string::npos &&
              far_result.diagnostic.find("float32 representation") != std::string::npos,
          "TEST-CURVE-07: the diagnostic keeps BOTH the provenance and the shortfall (" +
              far_result.diagnostic + ")");
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
    // ROUND 3 FIX ROUND (verify §5). A cusp at t* = 0.4 never lands on a leaf
    // boundary, so the leaf whose chord crosses it emits a 178 degree spike at a
    // join where the curve's own jump J = 0 is PROVED. That is a real excess over
    // the emitted-turn contract and it is now reported instead of suppressed —
    // the POSITION certificate is untouched, which is why the polyline is still
    // published and still inside its chord budget.
    check(r.certification == CurveCertification::QualityLimited && !r.points.empty(),
          "TEST-CURVE-05: a non-dyadic stationary point converges on its chord and reports the "
          "emitted turn it cannot avoid");
    check(r.diagnostic.find("emitted turn") != std::string::npos,
          "TEST-CURVE-05: and the diagnostic names that turn (" + r.diagnostic + ")");
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
    const AngularStatus status = onecad::tess::tangent_cone_status(span, 0.3);
    check(status == AngularStatus::Satisfied,
          "TEST-CURVE-05: a collinear span is Satisfied on the derived bound — the retired 1e-9 "
          "floor called the same span Undefined for no reason it could state");

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
    const double tangent_degrees = degrees(angle_between(tangent, gp_XYZ(1, 0, 0)));

    const TopoDS_Edge edge = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(bezier)).Edge();
    const CurveSampleResult r = sample(edge, 0.05);
    // ROUND 3 re-derivation (followup §2 C2). Round 2 SKIPPED q0 because its
    // relative magnitude 9e-10 sat under a 1e-9 floor, and downgraded the whole
    // unsplit span to Undefined. The followup deletes that floor: only an exactly
    // zero coefficient leaves the retained set, so q0 now takes part in the cone,
    // the cone is DISPROVED for the unsplit span, and the near-cusp at t=0 is
    // subdivided until it is genuinely resolved. The property to pin is
    // therefore stronger than "never Satisfied on that span": the 89.87 degree
    // excursion must not survive into the emitted polyline at all.
    double worst_turn = 0.0;
    for (std::size_t i = 1; i + 1 < r.points.size(); ++i) {
        worst_turn = std::max(worst_turn, degrees(encoded_join_turn(r.points, i)));
    }
    std::fprintf(stderr,
                 "TEST-CURVE-05: Astra quadratic -> certification %d, %zu leaves, first leaf "
                 "angular %d, greatest emitted turn %.4f deg, independent tangent at t=1e-6 is "
                 "%.2f deg off the chord\n",
                 static_cast<int>(r.certification), r.leaves.size(),
                 r.leaves.empty() ? -1 : static_cast<int>(r.leaves.front().angular), worst_turn,
                 tangent_degrees);
    check(tangent_degrees > 80.0,
          "TEST-CURVE-05: the fixture really does swing the tangent near t=0 (measured " +
              std::to_string(tangent_degrees) + " deg)");
    check(r.points.size() > 2,
          "TEST-CURVE-05: the sub-floor coefficient is NOT dropped — the span that hides an "
          "89.87 degree excursion is refused and subdivided, not accepted whole");
    check(worst_turn <= 5.0 * 1.0001,
          "TEST-CURVE-05: and the excursion never reaches the emitted polyline (greatest turn " +
              std::to_string(worst_turn) + " deg)");
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
    // A = (100000,0,0) -> B = (100000.003, 0.001, 0). ROUND 3 re-derivation
    // (followup §2 F5): the expectation no longer comes from "chord shorter than
    // 16 x its own rounding" — that rule is deleted — but from the cone being
    // tested about the chord that is actually EMITTED. float32 at 1e5 rounds both
    // X coordinates to 100000, so the encoded chord is (0, 0.001, 0) while the
    // real one is (0.003, 0.001, 0): a rotation of 71.57 degrees, far outside the
    // 2.5 degree half-allowance, and the claim goes with it.
    const gp_Pnt a(100000.0, 0.0, 0.0);
    const gp_Pnt b(100000.003, 0.001, 0.0);
    const TopoDS_Edge edge = BRepBuilderAPI_MakeEdge(a, b).Edge();
    const double expected_rotation =
        std::atan2(gp_XYZ(0.003, 0.001, 0.0).Crossed(gp_XYZ(0.0, 0.001, 0.0)).Modulus(),
                   gp_XYZ(0.003, 0.001, 0.0).Dot(gp_XYZ(0.0, 0.001, 0.0)));
    const CurveSampleResult r = sample(edge, 0.05);
    std::fprintf(stderr,
                 "TEST-CURVE-06: 1e5 short line -> certification %d, %zu leaves, angular %d, "
                 "chordLength %.3e, quantization %.3e, encoded rotation %.4f deg (analytic "
                 "%.4f deg)\n",
                 static_cast<int>(r.certification), r.leaves.size(),
                 r.leaves.empty() ? -1 : static_cast<int>(r.leaves.front().angular),
                 r.leaves.empty() ? -1.0 : r.leaves.front().chordLengthMm, r.quantizationErrorMm,
                 r.leaves.empty() ? -1.0 : r.leaves.front().encodedChordRotationRad * 180.0 / kPi,
                 expected_rotation * 180.0 / kPi);
    check(std::abs(expected_rotation * 180.0 / kPi - 71.565) < 0.01,
          "TEST-CURVE-06: the analytic encoded rotation of this fixture is 71.57 deg");
    check(!r.leaves.empty() && r.leaves.front().angular == AngularStatus::Undefined,
          "TEST-CURVE-06: a chord whose EMITTED direction leaves the cone reports no angular "
          "evidence");
    // The ideal vector above is what the fixture asks for; 100000.003 - 100000.0
    // is 0.0030000000260770321 in double, so the published rotation is compared
    // to the analytic one only to the precision the endpoints carry.
    check(!r.leaves.empty() &&
              std::abs(r.leaves.front().encodedChordRotationRad - expected_rotation) < 1e-7,
          "TEST-CURVE-06: and it publishes the measured rotation that cost it the claim");
    check_encoded_rotation_matches_points(r, "TEST-CURVE-06: 1e5 short line");
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
    // ROUND 3 FIX ROUND (verify §5, REFUTED suppression). This fixture's tail
    // leaves are 1e-11 mm long at 0.1 mm coordinates — far under one float32
    // ULP — so the EMITTED polyline really does turn 88 degrees there. The
    // retired `noise >= allowed -> skip` branch hid exactly that, on the grounds
    // that a bound on chord-vs-curve error was large. The emitted vertices
    // determine the turn exactly, so it is measured, published and reported.
    const double arc_error = dense_deviation_of_edge(arc_edge, ra.points, 8000);
    std::fprintf(stderr,
                 "TEST-CURVE-04: near-semicircle published turn %.4f deg, dense max error "
                 "%.3e mm (%s)\n",
                 ra.encodedTurnMaxRad * 180.0 / kPi, arc_error, ra.diagnostic.c_str());
    check(ra.certification == CurveCertification::QualityLimited && !ra.points.empty(),
          "TEST-CURVE-04: the near-semicircle weight pattern converges on its chord and reports "
          "the emitted turn of its sub-ULP tail");
    check(arc_error <= 0.01,
          "TEST-CURVE-04: its POSITION is still inside the requested budget (measured " +
              std::to_string(arc_error) + " mm)");
    check(ra.diagnostic.find("quantization-conditioned") != std::string::npos,
          "TEST-CURVE-04: and the conditioning of those joins is reported separately, not used "
          "to erase them");
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


// ===========================================================================
// ROUND 3 — the Astra followup (docs/design/astra/wp08-curve-sampler-followup.md
// §2 findings C1, C2, F5, F6, F7, F9; required tests §4) and review findings
// PR-07 / PR-09. Every oracle below is independent of the sampler: analytic
// tangents by central difference on a separately written rational evaluator, a
// double-double de Casteljau reference, float32 encoding done in the test, and
// turns measured from the published points alone.
// ===========================================================================


// --- ROUND 3 / C1: the display turn between EMITTED chords is the acceptance rule

// followup §4 "C1 regular corner": a degree-2 rational B-spline that runs a
// 100 mm straight in, turns a quarter circle of radius r, and runs 100 mm out,
// on the plane X = M. It is G1 everywhere (the straights are tangent to the
// arc), so no join carries a genuine tangent jump and every emitted turn must be
// inside the 5 degree tier. Before round 3 both (M, r) pairs came out as a
// single 0.0707 mm chord making 45 degree turns with each neighbour: at
// M = 1e9 the retired `16 * quantization` floor was 512 mm wide, and at r = 2e-5
// the `budget/1024` floor swallowed it.
Handle(Geom_BSplineCurve) regular_corner_spline(double m, double r) {
    NCollection_Array1<gp_Pnt> poles(1, 7);
    poles.SetValue(1, gp_Pnt(m, r, -100.0));
    poles.SetValue(2, gp_Pnt(m, r, -50.0));
    poles.SetValue(3, gp_Pnt(m, r, 0.0));
    poles.SetValue(4, gp_Pnt(m, r, r));
    poles.SetValue(5, gp_Pnt(m, 0.0, r));
    poles.SetValue(6, gp_Pnt(m, -50.0, r));
    poles.SetValue(7, gp_Pnt(m, -100.0, r));
    NCollection_Array1<double> weights(1, 7);
    for (int i = 1; i <= 7; ++i) weights.SetValue(i, 1.0);
    weights.SetValue(4, std::sqrt(0.5));
    NCollection_Array1<double> knots(1, 4);
    knots.SetValue(1, 0.0);
    knots.SetValue(2, 1.0);
    knots.SetValue(3, 2.0);
    knots.SetValue(4, 3.0);
    NCollection_Array1<int> mults(1, 4);
    mults.SetValue(1, 3);
    mults.SetValue(2, 2);
    mults.SetValue(3, 2);
    mults.SetValue(4, 3);
    return new Geom_BSplineCurve(poles, weights, knots, mults, 2);
}

void test_curve_08_c1_regular_corner_turns() {
    for (const std::pair<double, double>& mr :
         {std::make_pair(1e9, 0.05), std::make_pair(0.0, 2e-5)}) {
        const double m = mr.first;
        const double r = mr.second;
        const std::string label = "TEST-CURVE-08: regular corner M=" + num_string(m) +
                                  " r=" + num_string(r);
        Handle(Geom_BSplineCurve) spline = regular_corner_spline(m, r);
        const TopoDS_Edge edge = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(spline)).Edge();
        const CurveSampleResult result = sample(edge, 0.05);

        double worst_turn = 0.0;
        std::size_t worst_join = 0;
        for (std::size_t i = 1; i + 1 < result.points.size(); ++i) {
            const double turn = encoded_join_turn(result.points, i);
            if (turn > worst_turn) {
                worst_turn = turn;
                worst_join = i;
            }
        }
        bool any_singular_label = false;
        for (const CurveLeaf& leaf : result.leaves) {
            any_singular_label = any_singular_label || leaf.singularVertexAtStart ||
                                 leaf.singularVertexAtEnd || leaf.unresolvedRegion;
        }
        std::fprintf(stderr,
                     "TEST-CURVE-08: regular corner M=%.3g r=%.3g -> certification %d, %zu "
                     "points, greatest emitted turn %.4f deg at join %zu, singular labels %d, "
                     "singularParameters %zu\n",
                     m, r, static_cast<int>(result.certification), result.points.size(), 
                     degrees(worst_turn), worst_join, any_singular_label ? 1 : 0,
                     result.singularParameters.size());
        check(result.points.size() > 4,
              label + ": the corner is refined, not emitted as one chord");
        check(worst_turn <= kFiveDegrees * 1.0001,
              label + ": every EMITTED turn is inside the 5 degree tier (measured " +
                  std::to_string(degrees(worst_turn)) + " deg)");
        check(!any_singular_label,
              label + ": a G1 corner gets NO singular label — a failed cone and a short leaf "
                      "authorise nothing on their own");
        check(result.singularParameters.empty(),
              label + ": no singular parameter is published for a curve that has none");
        check_encoded_rotation_matches_points(result, label);

        const double positional = dense_deviation_of_edge(edge, result.points, 8000);
        check(positional <= 0.05,
              label + ": the refined corner is still inside its chord budget (measured " +
                  std::to_string(positional) + " mm)");
    }
}

// followup §4 "C1 genuine cusp": exact cubic poles (1,-1), (-1,2), (0,-4), (4,8).
// C'(1/3) = 3[(4/9)(-2,3) + (4/9)(1,-6) + (1/9)(4,12)] = 0, so the tangent
// genuinely reverses at t = 1/3 — a non-dyadic parameter subdivision can never
// land on. Its reversal is permitted INSIDE the independently bracketed region
// and the turn rule is enforced everywhere else.
void test_curve_08_c1_genuine_cusp_bracket() {
    Handle(Geom_BezierCurve) bezier = polynomial_cubic(gp_Pnt(1, -1, 0), gp_Pnt(-1, 2, 0),
                                                       gp_Pnt(0, -4, 0), gp_Pnt(4, 8, 0));
    const TopoDS_Edge edge = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(bezier)).Edge();
    std::vector<gp_XYZ> poles;
    std::vector<double> weights;
    for (int i = 1; i <= 4; ++i) {
        poles.push_back(bezier->Pole(i).XYZ());
        weights.push_back(1.0);
    }
    // INDEPENDENT root evidence: the analytic derivative, by hand, vanishes at
    // t = 1/3 and reverses across it.
    auto derivative = [&](double t) {
        const gp_XYZ a = poles[1].Subtracted(poles[0]);
        const gp_XYZ b = poles[2].Subtracted(poles[1]);
        const gp_XYZ c = poles[3].Subtracted(poles[2]);
        return a.Multiplied(3.0 * (1 - t) * (1 - t))
            .Added(b.Multiplied(6.0 * t * (1 - t)))
            .Added(c.Multiplied(3.0 * t * t));
    };
    const double cusp = 1.0 / 3.0;
    check(derivative(cusp).Modulus() < 1e-12,
          "TEST-CURVE-08: the cusp fixture really has C'(1/3) = 0 (measured " +
              std::to_string(derivative(cusp).Modulus()) + ")");
    check(derivative(cusp - 1e-3).Dot(derivative(cusp + 1e-3)) < 0.0,
          "TEST-CURVE-08: the tangent genuinely REVERSES across t = 1/3");

    const double tolerance = 0.002;
    const CurveSampleResult result = sample(edge, tolerance);

    double bracket_t0 = -1.0;
    double bracket_t1 = -1.0;
    std::size_t brackets = 0;
    double widest_bracket = 0.0;
    for (const CurveLeaf& leaf : result.leaves) {
        if (!leaf.unresolvedRegion) continue;
        ++brackets;
        widest_bracket = std::max(widest_bracket, leaf.chordLengthMm + leaf.chordBoundMm);
        if (bracket_t0 < 0.0 || leaf.t0 < bracket_t0) bracket_t0 = leaf.t0;
        if (leaf.t1 > bracket_t1) bracket_t1 = leaf.t1;
    }
    std::fprintf(stderr,
                 "TEST-CURVE-08: genuine cusp -> certification %d, %zu points, %zu singular "
                 "brackets over [%.8f, %.8f], widest %.3e mm (floor %.3e mm)\n",
                 static_cast<int>(result.certification), result.points.size(), brackets,
                 bracket_t0, bracket_t1, widest_bracket, tolerance / 1024.0);
    check(result.points.size() > 4, "TEST-CURVE-08: the cusped cubic is sampled, not collapsed");
    check(!result.points.empty(),
          "TEST-CURVE-08: and it is published — the reported turn is an angular shortfall, not a "
          "positional one");
    check(brackets >= 1,
          "TEST-CURVE-08: the reversal at t = 1/3 is bracketed as an unresolved singular region");
    check(bracket_t0 <= cusp && cusp <= bracket_t1,
          "TEST-CURVE-08: the bracket actually contains t = 1/3");
    check(widest_bracket <= tolerance / 1024.0,
          "TEST-CURVE-08: the bracket is bounded by the positional resolution floor (measured " +
              std::to_string(widest_bracket) + " mm)");

    // FIX ROUND (verify §3/§4): the label exempts NOTHING. What excuses the two
    // joins around a cusp is that the one-sided tangent DIRECTIONS there are not
    // resolved — the derivative-numerator enclosure swallows the coefficient — so
    // J's interval is the whole [0, pi] and no excess can be established either
    // way. Every join whose two tangents ARE resolved is held to the tier.
    double worst_outside = 0.0;
    bool bracket_join_reported = false;
    for (std::size_t i = 1; i + 1 < result.points.size() && i < result.leaves.size(); ++i) {
        if (result.leaves[i - 1].unresolvedRegion || result.leaves[i].unresolvedRegion) {
            // A join touching the unresolved region. Its turn is NOT excused —
            // the label carries no exemption — it is simply the one place this
            // fixture cannot meet the tier.
            bracket_join_reported =
                bracket_join_reported || encoded_join_turn(result.points, i) > kFiveDegrees;
            continue;
        }
        worst_outside = std::max(worst_outside, encoded_join_turn(result.points, i));
    }
    std::fprintf(stderr, "TEST-CURVE-08: cusp fixture greatest turn outside the bracket %.4f deg\n",
                 degrees(worst_outside));
    check(worst_outside <= kFiveDegrees * 1.0001,
          "TEST-CURVE-08: away from the unresolved region the 5 degree tier still binds "
          "(measured " +
              std::to_string(degrees(worst_outside)) + " deg)");
    check(bracket_join_reported && result.certification != CurveCertification::Certified &&
              result.diagnostic.find("emitted turn") != std::string::npos,
          "TEST-CURVE-08: and the turn at the unresolved region's own join is REPORTED, not "
          "excused by the label (" +
              result.diagnostic + ")");
    check_published_turn_matches_points(result, "TEST-CURVE-08: cusp fixture");
    const double positional = dense_deviation(
        [&](double t) { return rational_bezier_point(poles, weights, t); }, 0.0, 1.0,
        result.points, 8000);
    check(positional <= tolerance,
          "TEST-CURVE-08: the cusped cubic stays inside its chord budget (measured " +
              std::to_string(positional) + " mm)");
}

// followup §4 "C1 oracle integrity": the exemption needs singularity provenance
// that does not come from the turn oracle itself. A span whose cone merely FAILS
// (a 90 degree arc) and a span that is merely SHORT (the same arc at r = 2e-5,
// whose length plus bound is under budget/1024) must each be refined, never
// labelled singular.
void test_curve_08_c1_oracle_integrity() {
    // A bare 90 degree arc: the cone fails outright on the unsplit span.
    Handle(Geom_Circle) circle = new Geom_Circle(gp_Ax2(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1)), 2e-5);
    const TopoDS_Edge tiny = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(circle), 0.0, kPi / 2).Edge();
    const CurveSampleResult r = sample(tiny, 0.05);
    const double chord = 2e-5 * std::sqrt(2.0);
    std::fprintf(stderr,
                 "TEST-CURVE-08: oracle integrity — r=2e-5 quarter arc, chord %.3e mm vs floor "
                 "%.3e mm -> %zu points, %zu leaves\n",
                 chord, 0.05 / 1024.0, r.points.size(), r.leaves.size());
    check(chord < 0.05 / 1024.0,
          "TEST-CURVE-08: the fixture really does sit under the positional resolution floor");
    bool any_label = false;
    for (const CurveLeaf& leaf : r.leaves) {
        any_label = any_label || leaf.unresolvedRegion || leaf.singularVertexAtStart ||
                    leaf.singularVertexAtEnd;
    }
    check(!any_label,
          "TEST-CURVE-08: a failed cone on a sub-floor span is NOT evidence of a singularity");
    double worst = 0.0;
    for (std::size_t i = 1; i + 1 < r.points.size(); ++i) {
        worst = std::max(worst, encoded_join_turn(r.points, i));
    }
    check(r.points.size() > 3 && worst <= kFiveDegrees * 1.0001,
          "TEST-CURVE-08: it is REFINED until the emitted turns meet the tier (measured " +
              std::to_string(degrees(worst)) + " deg over " + std::to_string(r.points.size()) +
              " points)");
}

// --- ROUND 3 / C2: `Satisfied` iff beta + asin(E/g) <= tol/2 ----------------

// An INDEPENDENT tangent oracle: central differences on the separately written
// rational Bernstein evaluator, never the sampler's own derivative numerator.
double independent_tangent_excess(const std::vector<gp_XYZ>& poles,
                                  const std::vector<double>& weights, const CurveLeaf& leaf,
                                  const gp_XYZ& emittedChord, int samples) {
    double worst = 0.0;
    for (int i = 0; i <= samples; ++i) {
        const double t = leaf.t0 + (leaf.t1 - leaf.t0) * static_cast<double>(i) /
                                       static_cast<double>(samples);
        const double h = std::max(1e-12, 1e-7 * (leaf.t1 - leaf.t0));
        const double a = std::clamp(t - h, 0.0, 1.0);
        const double b = std::clamp(t + h, 0.0, 1.0);
        if (!(b > a)) continue;
        const gp_XYZ tangent = rational_bezier_point(poles, weights, b).XYZ().Subtracted(
            rational_bezier_point(poles, weights, a).XYZ());
        const double angle = angle_between(tangent, emittedChord);
        if (angle >= 0.0) worst = std::max(worst, angle);
    }
    return worst;
}

void test_curve_08_c2_satisfied_is_a_complete_bound() {
    // The Astra F1 quadratic, swept through delta. Its start tangent swings
    // toward +Y as delta grows, so a span that claims Satisfied at ANY delta is
    // claiming something an independent tangent evaluation can refute.
    for (const double delta : {0.001, 0.005, 0.009, 0.018, 0.022, 0.03, 0.04}) {
        NCollection_Array1<gp_Pnt> occtPoles(1, 3);
        occtPoles.SetValue(1, gp_Pnt(0.0, 0.0, 0.0));
        occtPoles.SetValue(2, gp_Pnt(0.0, delta, 0.0));
        occtPoles.SetValue(3, gp_Pnt(20.0, 0.0, 0.0));
        NCollection_Array1<double> occtWeights(1, 3);
        occtWeights.SetValue(1, 1e-6);
        occtWeights.SetValue(2, 1.0);
        occtWeights.SetValue(3, 1.0);
        Handle(Geom_BezierCurve) bezier = new Geom_BezierCurve(occtPoles, occtWeights);
        const TopoDS_Edge edge = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(bezier)).Edge();
        std::vector<gp_XYZ> poles;
        std::vector<double> weights;
        for (int i = 1; i <= 3; ++i) {
            poles.push_back(occtPoles.Value(i).XYZ());
            weights.push_back(occtWeights.Value(i));
        }
        const double tolerance = 0.05;
        const CurveSampleResult r = sample(edge, tolerance);
        const std::string at = " at delta=" + std::to_string(delta);

        check(r.points.size() >= 2 && r.certification != CurveCertification::Failed,
              "TEST-CURVE-08: the swept quadratic is sampled" + at);
        const double positional = dense_deviation(
            [&](double t) { return rational_bezier_point(poles, weights, t); }, 0.0, 1.0, r.points,
            8000);
        check(positional <= tolerance,
              "TEST-CURVE-08: the swept quadratic is inside its chord budget" + at + " (measured " +
                  std::to_string(positional) + " mm)");

        std::size_t satisfied = 0;
        double worst_excess = 0.0;
        for (std::size_t k = 0; k + 1 < r.points.size() && k < r.leaves.size(); ++k) {
            if (r.leaves[k].angular != AngularStatus::Satisfied) continue;
            ++satisfied;
            check(r.leaves[k].angularMarginRad >= 0.0,
                  "TEST-CURVE-08: a Satisfied leaf publishes a non-negative margin" + at);
            const gp_XYZ emitted = enc32(r.points[k + 1]).Subtracted(enc32(r.points[k]));
            const double excess = independent_tangent_excess(poles, weights, r.leaves[k], emitted,
                                                             64);
            worst_excess = std::max(worst_excess, excess);
        }
        std::fprintf(stderr,
                     "TEST-CURVE-08: C2 sweep delta=%.4f -> %zu leaves, %zu Satisfied, greatest "
                     "INDEPENDENT tangent-to-emitted-chord angle %.4f deg (half allowance "
                     "%.4f deg)\n",
                     delta, r.leaves.size(), satisfied, degrees(worst_excess),
                     degrees(0.5 * kFiveDegrees));
        check(worst_excess <= 0.5 * kFiveDegrees * 1.02,
              "TEST-CURVE-08: every leaf that claims Satisfied really does hold every tangent "
              "inside half the tolerance of its EMITTED chord" +
                  at + " (measured " + std::to_string(degrees(worst_excess)) + " deg)");
        check_encoded_rotation_matches_points(r, "TEST-CURVE-08: C2 sweep" + at);
    }
}

void test_curve_08_c2_four_regimes() {
    // 1. PASSING — a straight span: beta = 0, full retained mass, negligible E.
    {
        RationalBezierSpan span;
        span.poles = {gp_XYZ(0, 0, 0), gp_XYZ(1, 0, 0), gp_XYZ(2, 0, 0)};
        span.weights = {1.0, 1.0, 1.0};
        const onecad::tess::ConeVerdict v =
            onecad::tess::tangent_cone_verdict(span, kFiveDegrees, gp_XYZ(1, 0, 0),
                                              onecad::tess::pole_roundoff_enclosure(span));
        std::fprintf(stderr,
                     "TEST-CURVE-08: C2 passing regime -> status %d, beta %.3e, E %.3e, g %.3e, "
                     "margin %.6f rad\n",
                     static_cast<int>(v.status), v.coneHalfAngleRad, v.excludedMass,
                     v.retainedGap, v.marginRad);
        check(v.status == AngularStatus::Satisfied && v.marginRad > 0.0 &&
                  v.retainedMassMin == 1.0 && v.excludedMass < v.retainedGap,
              "TEST-CURVE-08: C2 passing regime — full retained mass, E < g, positive margin");
    }
    // 2. VANISHING RETAINED MASS — P0 == P1 makes q_0 exactly zero, so the
    //    retained Bernstein mass drops to zero at t = 0 and nothing is proved.
    {
        RationalBezierSpan span;
        span.poles = {gp_XYZ(0, 0, 0), gp_XYZ(0, 0, 0), gp_XYZ(1, 2, 0), gp_XYZ(2, 0, 0)};
        span.weights = {1.0, 1.0, 1.0, 1.0};
        const onecad::tess::ConeVerdict v =
            onecad::tess::tangent_cone_verdict(span, 3.0, gp_XYZ(2, 0, 0),
                                              onecad::tess::pole_roundoff_enclosure(span));
        std::fprintf(stderr,
                     "TEST-CURVE-08: C2 vanishing-mass regime -> status %d, s_min %.3e, "
                     "singularAtStart %d (%s)\n",
                     static_cast<int>(v.status), v.retainedMassMin, v.singularAtStart ? 1 : 0,
                     v.incompleteReason ? v.incompleteReason : "");
        check(v.status == AngularStatus::Undefined && v.retainedMassMin == 0.0 &&
                  v.singularAtStart && !v.singularAtEnd,
              "TEST-CURVE-08: C2 vanishing-mass regime — Undefined means evidence incomplete, "
              "reported at the end that actually vanishes");
    }
    // 3. E >= g and 4. ZERO MARGIN — a straight span in Y on the plane X = 1e9.
    //    Q cancels the 1e9 exactly, so the retained coefficients shrink with the
    //    span while the terms behind them stay at 1e9: sweeping the span length
    //    walks the SAME geometry through a passing margin, through the margin
    //    the uncertainty exhausts, and into E >= g.
    {
        bool saw_pass = false;
        bool saw_spent = false;
        bool saw_gap = false;
        double previous_margin = 1e300;
        bool monotone = true;
        for (int exponent = -2; exponent >= -16; --exponent) {
            const double d = std::pow(10.0, static_cast<double>(exponent));
            RationalBezierSpan span;
            span.poles = {gp_XYZ(1e9, 0, 0), gp_XYZ(1e9, d, 0), gp_XYZ(1e9, 2 * d, 0)};
            span.weights = {1.0, 0.8, 1.0};
            const onecad::tess::ConeVerdict v =
                onecad::tess::tangent_cone_verdict(span, kFiveDegrees, gp_XYZ(0, 1, 0),
                                              onecad::tess::pole_roundoff_enclosure(span));
            const std::string reason = v.incompleteReason ? v.incompleteReason : "";
            if (v.status == AngularStatus::Satisfied) {
                saw_pass = true;
                if (v.marginRad > previous_margin + 1e-12) monotone = false;
                previous_margin = v.marginRad;
            } else if (reason.find("spends the angular allowance") != std::string::npos) {
                saw_spent = true;
            } else if (reason.find("reaches the retained cone gap") != std::string::npos) {
                saw_gap = true;
            }
            std::fprintf(stderr,
                         "TEST-CURVE-08: C2 regime sweep d=1e%d -> status %d, E/g %.3e, margin "
                         "%.3e rad (%s)\n",
                         exponent, static_cast<int>(v.status),
                         v.retainedGap > 0.0 ? v.excludedMass / v.retainedGap : -1.0, v.marginRad,
                         reason.c_str());
        }
        check(saw_pass, "TEST-CURVE-08: C2 sweep reaches a PASSING regime");
        check(saw_spent,
              "TEST-CURVE-08: C2 sweep reaches the regime where the uncertainty spends the "
              "angular allowance — Undefined, not a false Satisfied");
        check(saw_gap, "TEST-CURVE-08: C2 sweep reaches the regime where E >= g");
        check(monotone,
              "TEST-CURVE-08: the published margin shrinks monotonically as the retained "
              "coefficients shrink — a continuous quantity behind a discrete status");
    }
}

// followup §4 C2: "independently evaluate Q at endpoint-adjacent parameters".
void test_curve_08_c2_numerator_at_the_endpoints() {
    Handle(Geom_BezierCurve) bezier = rational_cubic();
    std::vector<gp_XYZ> poles;
    std::vector<double> weights;
    RationalBezierSpan span;
    for (int i = 1; i <= 4; ++i) {
        poles.push_back(bezier->Pole(i).XYZ());
        weights.push_back(bezier->Weight(i));
        span.poles.push_back(poles.back());
        span.weights.push_back(weights.back());
    }
    const std::vector<gp_XYZ> q = onecad::tess::derivative_numerator_bernstein(span);
    double worst = 0.0;
    for (const double t : {1e-9, 1e-7, 1e-5, 1.0 - 1e-5, 1.0 - 1e-7, 1.0 - 1e-9}) {
        const double h = 1e-11;
        const gp_XYZ fd = rational_bezier_point(poles, weights, std::min(t + h, 1.0))
                              .XYZ()
                              .Subtracted(
                                  rational_bezier_point(poles, weights, std::max(t - h, 0.0)).XYZ());
        const gp_XYZ bern = bernstein_value(q, t);
        const double angle = angle_between(fd, bern);
        worst = std::max(worst, angle);
    }
    std::fprintf(stderr,
                 "TEST-CURVE-08: Q against an independent quotient derivative at endpoint-"
                 "adjacent parameters -> greatest direction disagreement %.3e deg\n",
                 degrees(worst));
    check(worst < 1e-4,
          "TEST-CURVE-08: the derivative numerator points the same way as an independently "
          "evaluated derivative right up against both endpoints");
}

void test_curve_08_c2_determinism() {
    Handle(Geom_BSplineCurve) spline = smooth_spline();
    const TopoDS_Edge edge = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(spline)).Edge();
    const CurveSampleResult a = sample(edge, 0.01);
    const CurveSampleResult b = sample(edge, 0.01);
    bool identical = a.points.size() == b.points.size() && a.leaves.size() == b.leaves.size() &&
                     a.certification == b.certification &&
                     a.quantizationErrorMm == b.quantizationErrorMm &&
                     a.encodedTurnMaxRad == b.encodedTurnMaxRad &&
                     a.quantizationConditionedJoins == b.quantizationConditionedJoins &&
                     a.qualityLimited == b.qualityLimited &&
                     a.singularParameters == b.singularParameters;
    for (std::size_t i = 0; identical && i < a.points.size(); ++i) {
        identical = a.points[i].X() == b.points[i].X() && a.points[i].Y() == b.points[i].Y() &&
                    a.points[i].Z() == b.points[i].Z();
    }
    for (std::size_t i = 0; identical && i < a.leaves.size(); ++i) {
        identical = a.leaves[i].t0 == b.leaves[i].t0 && a.leaves[i].t1 == b.leaves[i].t1 &&
                    a.leaves[i].chordBoundMm == b.leaves[i].chordBoundMm &&
                    a.leaves[i].angular == b.leaves[i].angular &&
                    a.leaves[i].angularMarginRad == b.leaves[i].angularMarginRad &&
                    a.leaves[i].encodedChordRotationRad == b.leaves[i].encodedChordRotationRad &&
                    a.leaves[i].startTangentUncertaintyRad ==
                        b.leaves[i].startTangentUncertaintyRad &&
                    a.leaves[i].endTangentUncertaintyRad == b.leaves[i].endTangentUncertaintyRad &&
                    a.leaves[i].startsAtSourceStart == b.leaves[i].startsAtSourceStart &&
                    a.leaves[i].endsAtSourceEnd == b.leaves[i].endsAtSourceEnd &&
                    a.leaves[i].unresolvedRegion == b.leaves[i].unresolvedRegion;
    }
    check(identical,
          "TEST-CURVE-08: identical requests reproduce the points AND the evidence exactly");
}

// --- ROUND 3 / F5: the cone is tested about the ENCODED chord ---------------

void test_curve_08_f5_encoded_chord_withdraws_the_angular_claim() {
    // followup §2 F5, orchestrator-recomputed: both X coordinates round to
    // 100000 in float32, so the emitted chord loses the whole X slope and points
    // 7.125 degrees away from the real one. Its length is 0.0484 mm, comfortably
    // past 16 x the 0.003 mm endpoint rounding, so the RETIRED
    // `chord >= 16 * quantization` rule passed it and left the leaf Satisfied.
    const gp_Pnt a(99999.997, 0.0, 0.0);
    const gp_Pnt b(100000.003, 0.048, 0.0);
    check(static_cast<float>(a.X()) == static_cast<float>(b.X()),
          "TEST-CURVE-08: the F5 fixture really does collapse both X coordinates to one float32");

    const gp_XYZ chord = b.XYZ().Subtracted(a.XYZ());
    const gp_XYZ emitted = enc32(b).Subtracted(enc32(a));
    const double rotation = angle_between(chord, emitted);
    check(std::abs(degrees(rotation) - 7.125016) < 0.001,
          "TEST-CURVE-08: the independently computed encoded rotation is 7.125 deg (measured " +
              std::to_string(degrees(rotation)) + ")");

    const TopoDS_Edge edge = BRepBuilderAPI_MakeEdge(a, b).Edge();
    const CurveSampleResult r = sample(edge, 0.05);
    std::fprintf(stderr,
                 "TEST-CURVE-08: F5 line -> certification %d, %zu leaves, angular %d, chord "
                 "%.6g mm, quantization %.6g mm, 16x rule %s, published rotation %.6f deg\n",
                 static_cast<int>(r.certification), r.leaves.size(),
                 r.leaves.empty() ? -1 : static_cast<int>(r.leaves.front().angular),
                 r.leaves.empty() ? -1.0 : r.leaves.front().chordLengthMm, r.quantizationErrorMm,
                 (!r.leaves.empty() && r.leaves.front().chordLengthMm >= 16.0 * r.quantizationErrorMm)
                     ? "PASSES"
                     : "fails",
                 r.leaves.empty() ? -1.0 : degrees(r.leaves.front().encodedChordRotationRad));

    check(r.points.size() == 2 && r.leaves.size() == 1,
          "TEST-CURVE-08: the F5 line is one emitted segment");
    check(r.certification == CurveCertification::Certified,
          "TEST-CURVE-08: the F5 line keeps its POSITION certificate — the chord bound is exact");
    check(!r.leaves.empty() && r.leaves.front().angular == AngularStatus::Undefined,
          "TEST-CURVE-08: it LOSES the angular certification, because the cone is tested about "
          "the chord that is actually emitted");
    check(r.angularUndefinedSomewhere,
          "TEST-CURVE-08: the withdrawn angular claim is visible on the result");
    check(!r.leaves.empty() && r.leaves.front().angularMarginRad < 0.0,
          "TEST-CURVE-08: an unestablished leaf publishes no angular margin");
    check(!r.leaves.empty() &&
              std::abs(r.leaves.front().encodedChordRotationRad - rotation) < 1e-12,
          "TEST-CURVE-08: the published encoded rotation is the measured one");
    check(!r.leaves.empty() && r.leaves.front().chordLengthMm >= 16.0 * r.quantizationErrorMm,
          "TEST-CURVE-08: the fixture really does pass the RETIRED 16x length rule, which is why "
          "that rule had to be replaced rather than supplemented");
    check(!r.leaves.empty() && !r.leaves.front().singularVertexAtStart &&
              !r.leaves.front().singularVertexAtEnd && !r.leaves.front().unresolvedRegion,
          "TEST-CURVE-08: an encoding limit is not a singularity");
}

// --- ROUND 3 / F6: control-net validation and the charged roundoff enclosure --

// A double-double reference (two doubles, ~106 significand bits) for the
// de Casteljau recurrence, written here so the comparison does not run on the
// same arithmetic it is checking. `long double` is 64-bit on arm64 macOS and
// would prove nothing.
struct DD {
    double hi = 0.0;
    double lo = 0.0;
};

DD quick_two_sum(double a, double b) {
    const double s = a + b;
    return DD{s, b - (s - a)};
}

DD two_sum(double a, double b) {
    const double s = a + b;
    const double bb = s - a;
    return DD{s, (a - (s - bb)) + (b - bb)};
}

DD dd_add(DD a, DD b) {
    const DD s = two_sum(a.hi, b.hi);
    return quick_two_sum(s.hi, s.lo + a.lo + b.lo);
}

DD dd_neg(DD a) { return DD{-a.hi, -a.lo}; }

DD dd_mul_double(DD a, double b) {
    const double p = a.hi * b;
    return quick_two_sum(p, std::fma(a.hi, b, -p) + a.lo * b);
}

DD dd_div(DD a, DD b) {
    const double q1 = a.hi / b.hi;
    const DD r = dd_add(a, dd_neg(dd_mul_double(b, q1)));
    return quick_two_sum(q1, (r.hi + r.lo) / b.hi);
}

struct DDHom {
    DD x, y, z, w;
};

DDHom dd_half_sum(const DDHom& p, const DDHom& q) {
    return DDHom{dd_mul_double(dd_add(p.x, q.x), 0.5), dd_mul_double(dd_add(p.y, q.y), 0.5),
                 dd_mul_double(dd_add(p.z, q.z), 0.5), dd_mul_double(dd_add(p.w, q.w), 0.5)};
}

// Left/right de Casteljau at t = 0.5 in homogeneous double-double coordinates.
void dd_subdivide(const std::vector<DDHom>& span, std::vector<DDHom>& left,
                  std::vector<DDHom>& right) {
    const std::size_t count = span.size();
    std::vector<DDHom> work = span;
    left.assign(count, DDHom{});
    right.assign(count, DDHom{});
    left[0] = work[0];
    right[count - 1] = work[count - 1];
    for (std::size_t k = 1; k < count; ++k) {
        for (std::size_t i = 0; i + k < count; ++i) work[i] = dd_half_sum(work[i], work[i + 1]);
        left[k] = work[0];
        right[count - 1 - k] = work[count - 1 - k];
    }
}

std::vector<DDHom> dd_from(const RationalBezierSpan& span) {
    std::vector<DDHom> out(span.poles.size());
    for (std::size_t i = 0; i < span.poles.size(); ++i) {
        const double w = span.weights[i];
        out[i] = DDHom{DD{span.poles[i].X() * w, std::fma(span.poles[i].X(), w, -(span.poles[i].X() * w))},
                       DD{span.poles[i].Y() * w, std::fma(span.poles[i].Y(), w, -(span.poles[i].Y() * w))},
                       DD{span.poles[i].Z() * w, std::fma(span.poles[i].Z(), w, -(span.poles[i].Z() * w))},
                       DD{w, 0.0}};
    }
    return out;
}

void test_curve_08_f6_invalid_control_nets_never_certify() {
    // The exact `std::max(worst, NaN)` regression: a net with one NaN pole used
    // to hand back a chord bound of ZERO, which certifies everything.
    {
        RationalBezierSpan broken;
        broken.poles = {gp_XYZ(std::nan(""), std::nan(""), 0.0), gp_XYZ(0, 1, 0), gp_XYZ(1.0 / 3.0, 2.0 / 3.0, 0)};
        broken.weights = {1.0, 1.0, 1.0};
        const double bound = onecad::tess::hull_chord_bound(broken);
        std::fprintf(stderr, "TEST-CURVE-08: F6 NaN-pole net -> valid %d, hull bound %g\n",
                     onecad::tess::control_net_is_valid(broken) ? 1 : 0, bound);
        check(!onecad::tess::control_net_is_valid(broken),
              "TEST-CURVE-08: a net with a non-finite pole is rejected outright");
        check(!(bound <= 1.0) && !std::isnan(bound),
              "TEST-CURVE-08: its hull bound is INFINITE, never the zero that std::max(worst, "
              "NaN) used to return");
    }
    // A homogeneous denominator that underflowed to zero — what weight
    // normalisation did to (1e-300, 1e300, 1e300) before round 3.
    {
        RationalBezierSpan underflowed;
        underflowed.poles = {gp_XYZ(0, 0, 0), gp_XYZ(0, 1, 0), gp_XYZ(1, 0, 0)};
        underflowed.weights = {0.0, 1.0, 1.0};
        check(!onecad::tess::control_net_is_valid(underflowed),
              "TEST-CURVE-08: a zero homogeneous denominator is rejected — the convex-hull "
              "argument needs w(t) > 0");
        check(std::isinf(onecad::tess::hull_chord_bound(underflowed)),
              "TEST-CURVE-08: and it has no hull certificate at all");
    }
    // The same weights through the real conversion. Round 3 stopped the scaling
    // from underflowing in the first place, so this net is now SUPPORTED and is
    // sampled honestly; what it demonstrates is that the guard works, and the
    // separate subnormal fixture in TEST-CURVE-09 covers a net that genuinely
    // cannot be supported.
    {
        NCollection_Array1<gp_Pnt> poles(1, 3);
        poles.SetValue(1, gp_Pnt(0.0, 0.0, 0.0));
        poles.SetValue(2, gp_Pnt(0.0, 1.0, 0.0));
        poles.SetValue(3, gp_Pnt(1.0, 0.0, 0.0));
        NCollection_Array1<double> weights(1, 3);
        weights.SetValue(1, 1e-300);
        weights.SetValue(2, 1e300);
        weights.SetValue(3, 1e300);
        Handle(Geom_BezierCurve) bezier = new Geom_BezierCurve(poles, weights);
        const TopoDS_Edge edge = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(bezier)).Edge();
        BRepAdaptor_Curve adaptor(edge);
        const gp_Pnt first = adaptor.Value(adaptor.FirstParameter());
        const gp_Pnt last = adaptor.Value(adaptor.LastParameter());
        const CurveSampleResult r = sample(edge, 0.05);
        std::fprintf(stderr,
                     "TEST-CURVE-08: F6 normalisation underflow -> certification %d, %zu points, "
                     "endpoints finite %d (%s)\n",
                     static_cast<int>(r.certification), r.points.size(),
                     (std::isfinite(first.X()) && std::isfinite(last.X())) ? 1 : 0,
                     r.diagnostic.c_str());
        check(std::isfinite(first.X()) && std::isfinite(first.Y()) && std::isfinite(last.X()) &&
                  std::isfinite(last.Y()),
              "TEST-CURVE-08: the fixture's own analytic endpoints ARE finite, which is what made "
              "the underflow invisible");
        RationalBezierSpan raw;
        for (int i = 1; i <= 3; ++i) {
            raw.poles.push_back(poles.Value(i).XYZ());
            raw.weights.push_back(weights.Value(i));
        }
        check(onecad::tess::homogeneous_arithmetic_is_supported(raw),
              "TEST-CURVE-08: with the normalisation guard in place this net never underflows");
        std::vector<gp_XYZ> p;
        std::vector<double> w;
        for (int i = 1; i <= 3; ++i) {
            p.push_back(poles.Value(i).XYZ());
            w.push_back(weights.Value(i));
        }
        const double worst = dense_deviation(
            [&](double t) { return rational_bezier_point(p, w, t); }, 0.0, 1.0, r.points, 8000);
        check(all_finite(r.points),
              "TEST-CURVE-08: and it never emits a non-finite point either");
        check(worst <= 0.05,
              "TEST-CURVE-08: the polyline it does emit is inside the requested budget, measured "
              "against an independent rational evaluation (" +
                  std::to_string(worst) + " mm)");
    }
}

void test_curve_08_f6_roundoff_enclosure_covers_the_discrepancy() {
    // followup §2 F6: u = 2^-53, K = depth*(degree+3)+2, g = gamma_K, multiplier
    // (2g + u(1+g))/(1-g). At degree 32 / depth 32 that is 2.492e-13, i.e.
    // 4.317e-4 mm Euclidean when every coordinate is at most 1e9 mm.
    const double multiplier = onecad::tess::pole_roundoff_multiplier(32, 32);
    std::fprintf(stderr,
                 "TEST-CURVE-08: F6 enclosure multiplier at n=32 d=32 -> %.6e (expected "
                 "2.492e-13), Euclidean at 1e9 mm %.6e mm (expected 4.317e-4)\n",
                 multiplier, multiplier * std::sqrt(3.0) * 1e9);
    check(std::abs(multiplier - 2.492e-13) < 1e-16,
          "TEST-CURVE-08: the derived per-axis multiplier is 2.492e-13 at n=32, depth=32");
    check(std::abs(multiplier * std::sqrt(3.0) * 1e9 - 4.317e-4) < 1e-7,
          "TEST-CURVE-08: which is 4.317e-4 mm Euclidean at |coordinate| <= 1e9 mm");
    check(onecad::tess::pole_roundoff_multiplier(0, 2) > 0.0 &&
              onecad::tess::pole_roundoff_multiplier(0, 2) < 1e-15,
          "TEST-CURVE-08: an unsubdivided net carries only the final rounding");

    // The charged enclosure must cover the REAL discrepancy between the shipped
    // double subdivision and a double-double reference, at a 1e-18 weight ratio,
    // across translations and depths.
    double worst_ratio = 0.0;
    for (const double shift : {0.0, 1e5, 1e9}) {
        for (int depth = 1; depth <= 8; ++depth) {
            RationalBezierSpan span;
            span.poles = {gp_XYZ(shift + 0.0, 0.0, 0.0), gp_XYZ(shift + 3.0, 4.0, 1.0),
                          gp_XYZ(shift + 7.0, -2.0, 5.0), gp_XYZ(shift + 11.0, 1.0, 0.0)};
            span.weights = {1e-18, 1.0, 1.0, 1e-18};
            span.sourceMaxAbs = gp_XYZ(0, 0, 0);
            for (const gp_XYZ& p : span.poles) {
                span.sourceMaxAbs.SetX(std::max(span.sourceMaxAbs.X(), std::abs(p.X())));
                span.sourceMaxAbs.SetY(std::max(span.sourceMaxAbs.Y(), std::abs(p.Y())));
                span.sourceMaxAbs.SetZ(std::max(span.sourceMaxAbs.Z(), std::abs(p.Z())));
            }
            std::vector<DDHom> reference = dd_from(span);
            RationalBezierSpan current = span;
            for (int level = 0; level < depth; ++level) {
                RationalBezierSpan left, right;
                onecad::tess::subdivide_homogeneous(current, left, right);
                std::vector<DDHom> dleft, dright;
                dd_subdivide(reference, dleft, dright);
                // Walk the LEFT branch at even levels and the RIGHT branch at odd
                // ones so the comparison visits both halves of the recurrence.
                if (level % 2 == 0) {
                    current = left;
                    reference = dleft;
                } else {
                    current = right;
                    reference = dright;
                }
            }
            check(onecad::tess::control_net_is_valid(current),
                  "TEST-CURVE-08: the 1e-18 weight-ratio net stays valid through subdivision");
            const double enclosure = onecad::tess::pole_roundoff_enclosure(current);
            double worst_axis = 0.0;
            for (std::size_t i = 0; i < current.poles.size(); ++i) {
                const DD rx = dd_div(reference[i].x, reference[i].w);
                const DD ry = dd_div(reference[i].y, reference[i].w);
                const DD rz = dd_div(reference[i].z, reference[i].w);
                worst_axis = std::max(worst_axis, std::abs(current.poles[i].X() - (rx.hi + rx.lo)));
                worst_axis = std::max(worst_axis, std::abs(current.poles[i].Y() - (ry.hi + ry.lo)));
                worst_axis = std::max(worst_axis, std::abs(current.poles[i].Z() - (rz.hi + rz.lo)));
            }
            const double per_axis_budget =
                onecad::tess::pole_roundoff_multiplier(current.depth,
                                                       static_cast<int>(current.poles.size()) - 1) *
                std::max({span.sourceMaxAbs.X(), span.sourceMaxAbs.Y(), span.sourceMaxAbs.Z()});
            if (per_axis_budget > 0.0) {
                worst_ratio = std::max(worst_ratio, worst_axis / per_axis_budget);
            }
            check(worst_axis <= per_axis_budget,
                  "TEST-CURVE-08: the charged enclosure covers the measured subdivision "
                  "discrepancy at shift " +
                      std::to_string(shift) + " depth " + std::to_string(depth) + " (measured " +
                      std::to_string(worst_axis) + " mm against " +
                      std::to_string(per_axis_budget) + " mm)");
            check(enclosure >= worst_axis,
                  "TEST-CURVE-08: and so does the Euclidean enclosure the certificate is charged");
        }
    }
    std::fprintf(stderr,
                 "TEST-CURVE-08: F6 greatest measured discrepancy is %.4f of the charged per-axis "
                 "enclosure\n",
                 worst_ratio);
}

// --- ROUND 3 / F7: closure is semantic, with no parameter epsilon -----------

void test_curve_08_f7_near_period_trim_stays_open() {
    // followup §2 F7: a radius-5000 circle trimmed to [0, 2pi - 5e-10] has a real
    // gap of 5000 * 5e-10 = 2.5e-6 mm and two DISTINCT end vertices. The retired
    // `span >= period - 1e-9` shortcut closed it, moving a real endpoint onto
    // another one and calling an open edge closed.
    const double radius = 5000.0;
    const double shortfall = 5e-10;
    Handle(Geom_Circle) circle =
        new Geom_Circle(gp_Ax2(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1)), radius);
    const TopoDS_Edge edge =
        BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(circle), 0.0, 2.0 * kPi - shortfall).Edge();
    const TopoDS_Vertex first = TopExp::FirstVertex(edge);
    const TopoDS_Vertex last = TopExp::LastVertex(edge);
    check(!first.IsNull() && !last.IsNull() && !first.IsSame(last),
          "TEST-CURVE-08: the near-period trim really does carry two DISTINCT vertices");
    check(2.0 * kPi - shortfall >= circle->Period() - 1e-9,
          "TEST-CURVE-08: and it really does satisfy the retired `period - 1e-9` shortcut");

    const CurveSampleResult r = sample(edge, 0.05);
    const double gap = r.points.size() >= 2 ? r.points.front().Distance(r.points.back()) : -1.0;
    const double expected = radius * shortfall;
    std::fprintf(stderr,
                 "TEST-CURVE-08: F7 r=5000 trimmed to 2pi-5e-10 -> %zu points, endpoint gap "
                 "%.6e mm (analytic %.6e mm)\n",
                 r.points.size(), gap, expected);
    check(r.points.size() > 2, "TEST-CURVE-08: the near-closed circle is still sampled");
    check(gap > 0.0 && std::abs(gap - expected) <= 1e-6 * expected,
          "TEST-CURVE-08: the 2.5e-6 mm gap is PRESERVED, not snapped shut (measured " +
              std::to_string(gap) + " mm)");
    check_encoded_rotation_matches_points(r, "TEST-CURVE-08: F7 open near-closure");
}

void test_curve_08_f7_genuine_closure_still_snaps() {
    // The same radius, closed for real: one shared vertex, exact closure kept,
    // and the angular evidence of the two leaves the snap touches re-derived from
    // the endpoints that were actually emitted.
    const double radius = 5000.0;
    Handle(Geom_Circle) circle =
        new Geom_Circle(gp_Ax2(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1)), radius);
    const TopoDS_Edge edge = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(circle)).Edge();
    const TopoDS_Vertex first = TopExp::FirstVertex(edge);
    const TopoDS_Vertex last = TopExp::LastVertex(edge);
    check(!first.IsNull() && !last.IsNull() && first.IsSame(last),
          "TEST-CURVE-08: a genuinely closed circle carries ONE vertex, which is the evidence "
          "the closure rule now uses");

    const CurveSampleResult r = sample(edge, 0.5);
    std::fprintf(stderr,
                 "TEST-CURVE-08: F7 closed r=5000 circle -> certification %d, %zu points, "
                 "closure gap %.3e mm\n",
                 static_cast<int>(r.certification), r.points.size(),
                 r.points.size() >= 2 ? r.points.front().Distance(r.points.back()) : -1.0);
    check(r.points.size() > 8 && r.points.front().Distance(r.points.back()) == 0.0,
          "TEST-CURVE-08: semantic closure still snaps exactly");
    // The snap MOVED the last emitted point after its leaf was certified, so the
    // leaf's angular evidence must describe the segment that ended up on the wire.
    check_encoded_rotation_matches_points(r, "TEST-CURVE-08: F7 closed circle");
    double worst = 0.0;
    for (std::size_t i = 1; i + 1 < r.points.size(); ++i) {
        worst = std::max(worst, encoded_join_turn(r.points, i));
    }
    check(worst <= kFiveDegrees * 1.0001,
          "TEST-CURVE-08: the re-checked polyline still meets the 5 degree tier after the snap "
          "(measured " +
              std::to_string(degrees(worst)) + " deg)");
}

// --- ROUND 3 / F9 + PR-07: the per-body segment cap is HARD -----------------

std::vector<std::pair<std::uint32_t, std::uint32_t>> edge_ranges_of(
    const std::vector<std::uint8_t>& bytes) {
    const Section ranges = find_section(bytes, 7);
    std::vector<std::pair<std::uint32_t, std::uint32_t>> out;
    for (std::size_t i = 0; (i + 1) * 8U <= ranges.length; ++i) {
        const std::size_t entry = ranges.offset + i * 8U;
        out.emplace_back(u32(bytes, entry), u32(bytes, entry + 4));
    }
    return out;
}

std::size_t emitted_segments(const std::vector<std::uint8_t>& bytes) {
    std::size_t total = 0;
    for (const auto& range : edge_ranges_of(bytes)) {
        if (range.second >= 2) total += range.second - 1;
    }
    return total;
}

TopoDS_Shape six_circle_compound() {
    TopoDS_Compound compound;
    BRep_Builder builder;
    builder.MakeCompound(compound);
    for (int i = 0; i < 6; ++i) {
        Handle(Geom_Circle) circle = new Geom_Circle(
            gp_Ax2(gp_Pnt(20.0 * static_cast<double>(i), 0, 0), gp_Dir(0, 0, 1)), 5.0);
        builder.Add(compound, BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(circle)).Edge());
    }
    return compound;
}

void test_curve_08_f9_body_segment_cap_is_hard() {
    const TopoDS_Shape compound = six_circle_compound();
    onecad::tess::detail::clear_body_edge_segment_budget_for_test();
    const onecad::tess::BodyMesh full =
        onecad::tess::tessellate_body(compound, "body_budget_full", "fine", true, nullptr);
    const auto full_ranges = edge_ranges_of(full.blob);
    check(full_ranges.size() == 6, "TEST-CURVE-08: the budget fixture has six edges");
    if (full_ranges.size() != 6) return;
    const std::size_t first_edge_segments = full_ranges[0].second - 1;
    std::fprintf(stderr,
                 "TEST-CURVE-08: F9 unconstrained -> %zu segments total, first edge %zu\n",
                 emitted_segments(full.blob), first_edge_segments);
    check(full.completeness.budgetLimitedEdges.empty(),
          "TEST-CURVE-08: nothing is budget-limited when the budget is the shipped one");

    // (a) The LAST edge that fits exactly fills the budget: edge 1 takes all of
    //     it and edges 2..6 get nothing at all.
    onecad::tess::detail::set_body_edge_segment_budget_for_test(first_edge_segments);
    const onecad::tess::BodyMesh exact =
        onecad::tess::tessellate_body(compound, "body_budget_exact", "fine", true, nullptr);
    const auto exact_ranges = edge_ranges_of(exact.blob);
    std::fprintf(stderr,
                 "TEST-CURVE-08: F9 cap %zu -> %zu segments emitted, budgetLimitedEdges %zu\n",
                 first_edge_segments, emitted_segments(exact.blob),
                 exact.completeness.budgetLimitedEdges.size());
    check(emitted_segments(exact.blob) == first_edge_segments,
          "TEST-CURVE-08: total emitted segments equal the cap EXACTLY — five post-budget edges "
          "add none (they used to add one apiece, uncounted)");
    check(exact_ranges.size() == 6,
          "TEST-CURVE-08: every edge keeps its range slot, so the edge tables stay aligned");
    bool zero_after_first = exact_ranges.size() == 6;
    for (std::size_t i = 1; i < exact_ranges.size(); ++i) {
        zero_after_first = zero_after_first && exact_ranges[i].second == 0;
    }
    check(zero_after_first,
          "TEST-CURVE-08: a post-budget edge carries a ZERO-POINT range, never two endpoints");
    check(exact.completeness.budgetLimitedEdges ==
              std::vector<std::string>{"e:2", "e:3", "e:4", "e:5", "e:6"},
          "TEST-CURVE-08: and every one of them is named in completeness.budgetLimitedEdges");
    check(exact.edge_classes.size() == 6,
          "TEST-CURVE-08: per-edge display semantics stay index-aligned with the ranges");

    // (b) A cap so small that even the four doublings cannot fit an edge: each
    //     surviving edge falls back to its two analytic endpoints, those segments
    //     are COUNTED, and the total still stops at the cap.
    onecad::tess::detail::set_body_edge_segment_budget_for_test(3);
    const onecad::tess::BodyMesh tiny =
        onecad::tess::tessellate_body(compound, "body_budget_tiny", "fine", true, nullptr);
    const auto tiny_ranges = edge_ranges_of(tiny.blob);
    std::fprintf(stderr,
                 "TEST-CURVE-08: F9 cap 3 -> %zu segments emitted, budgetLimitedEdges %zu\n",
                 emitted_segments(tiny.blob), tiny.completeness.budgetLimitedEdges.size());
    check(emitted_segments(tiny.blob) <= 3,
          "TEST-CURVE-08: a three-segment cap emits at most three segments across SIX edges");
    check(tiny_ranges.size() == 6 && tiny.edge_classes.size() == 6,
          "TEST-CURVE-08: identity survives an exhausted budget");
    check(!tiny.completeness.budgetLimitedEdges.empty(),
          "TEST-CURVE-08: the body is explicitly reported incomplete rather than silently short");

    onecad::tess::detail::clear_body_edge_segment_budget_for_test();
    check(onecad::tess::detail::body_edge_segment_budget() == 2000000,
          "TEST-CURVE-08: the production budget is restored — the seam is test-only");
}

// --- ROUND 3 / PR-07: requested and achieved tolerance stay separate --------

void test_curve_08_doubling_ladder_separates_requested_from_achieved() {
    Handle(Geom_BSplineCurve) spline = oscillating_spline();
    const TopoDS_Edge edge = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(spline)).Edge();
    BRepAdaptor_Curve adaptor(edge);

    CurveSampleLimits roomy;
    CurveSampleRequest tight;
    tight.chordToleranceMm = 0.05;
    tight.angularToleranceRad = kFiveDegrees;
    CurveSampleRequest doubled = tight;
    doubled.chordToleranceMm = 0.10;
    doubled.angularToleranceRad = 2.0 * kFiveDegrees;
    const std::size_t at_tight = sample_edge_curve(adaptor, tight, roomy, {}).leaves.size();
    const std::size_t at_doubled = sample_edge_curve(adaptor, doubled, roomy, {}).leaves.size();
    std::fprintf(stderr,
                 "TEST-CURVE-08: ladder fixture needs %zu segments at 0.05 mm and %zu at "
                 "0.10 mm\n",
                 at_tight, at_doubled);
    check(at_doubled + 8 < at_tight,
          "TEST-CURVE-08: the ladder fixture really is cheaper at twice the tolerance");
    if (!(at_doubled + 8 < at_tight)) return;

    CurveSampleLimits capped;
    capped.maxSegmentsPerEdge = at_doubled + 8;
    const CurveSampleResult refused = sample_edge_curve(adaptor, tight, capped, {});
    check(refused.certification == CurveCertification::QualityLimited && refused.points.empty(),
          "TEST-CURVE-08: at that cap the requested tolerance is a WORK cap — nothing published");
    check(refused.requestedChordToleranceMm == 0.05 && refused.achievedChordToleranceMm == 0.05,
          "TEST-CURVE-08: the sampler itself never relaxes anything, so requested == achieved on "
          "every path it returns");

    const CurveSampleResult r =
        onecad::tess::detail::sample_edge_polyline(adaptor, "e:1", tight, capped);
    std::fprintf(stderr,
                 "TEST-CURVE-08: ladder -> certification %d, %zu points, requested %.6g / "
                 "achieved %.6g mm, requested %.6g / achieved %.6g rad, certificate %.3g (%s)\n",
                 static_cast<int>(r.certification), r.points.size(), r.requestedChordToleranceMm,
                 r.achievedChordToleranceMm, r.requestedAngularToleranceRad,
                 r.achievedAngularToleranceRad, r.certifiedChordBoundMm, r.diagnostic.c_str());
    check(!r.points.empty(),
          "TEST-CURVE-08: the relaxed retry publishes a real polyline, not two endpoints");
    check(r.certification == CurveCertification::QualityLimited,
          "TEST-CURVE-08: a relaxed retry is QualityLimited — the retry's own Certified must "
          "never leak out as if the request had been met");
    check(r.requestedChordToleranceMm == 0.05 && r.achievedChordToleranceMm == 0.10,
          "TEST-CURVE-08: requested 0.05 mm and achieved 0.10 mm are carried SEPARATELY");
    check(r.requestedAngularToleranceRad == kFiveDegrees &&
              r.achievedAngularToleranceRad == 2.0 * kFiveDegrees,
          "TEST-CURVE-08: and so are the requested and achieved angular tolerances");
    check(r.certifiedChordBoundMm < 0.0,
          "TEST-CURVE-08: a relaxed retry carries no certificate against the REQUESTED budget");
    check(r.diagnostic.find("requested 0.05") != std::string::npos &&
              r.diagnostic.find("achieved 0.1") != std::string::npos &&
              r.diagnostic.find("1 tolerance doubling") != std::string::npos,
          "TEST-CURVE-08: the diagnostic names the doubling count and both tolerances (" +
              r.diagnostic + ")");

    const double positional = dense_deviation_of_edge(edge, r.points, 8000);
    check(positional <= r.achievedChordToleranceMm,
          "TEST-CURVE-08: the published polyline really is inside the ACHIEVED tolerance "
          "(measured " +
              std::to_string(positional) + " mm)");
}


// ===========================================================================
// ROUND 3 FIX ROUND — the local adversarial review R1(c) and the Astra `verify`
// (docs/design/astra/wp08-curve-sampler-verify.md). Every construction that was
// REFUTED there gets its counterexample as a fixture here.
// ===========================================================================

// --- FIX / verify §5 + R1(c) Blocker 1 ---------------------------------------

// A 0.1 mm circle in the XY plane at X = 8192 mm: float32 spacing there is
// 2^-10 = 0.0009765625 mm, so a chord of a few thousandths of a millimetre is a
// handful of ULPs and the EMITTED polyline zig-zags by up to 90 degrees. The
// positional certificate is untouched by that — every emitted point is within
// half an ULP of the curve — but the emitted TURN contract is violated, and the
// retired `noise >= allowed -> skip` branch suppressed exactly the joins that
// prove it: the sampler published `Certified` with encodedTurnMaxRad = 0.0837 rad
// while the polyline turned 1.54 rad.
TopoDS_Edge quantized_small_circle() {
    Handle(Geom_Circle) circle =
        new Geom_Circle(gp_Ax2(gp_Pnt(8192.0, 0.0, 0.0), gp_Dir(0, 0, 1)), 0.1);
    return BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(circle)).Edge();
}

void test_curve_09_emitted_turn_is_never_suppressed() {
    const TopoDS_Edge edge = quantized_small_circle();
    // The three display tiers' (linear, angular) pairs for a 50 mm body.
    const double tiers[3][2] = {{0.5, 0.5}, {0.15, 0.21}, {0.025, kFiveDegrees}};
    const char* names[3] = {"coarse", "medium", "fine"};
    int tiers_seen[2] = {0, 0};
    for (int i = 0; i < 3; ++i) {
        const CurveSampleResult r = sample(edge, tiers[i][0], tiers[i][1]);
        const double measured = independent_encoded_turn_max(r.points);
        std::size_t over = 0;
        for (std::size_t k = 1; k + 1 < r.points.size(); ++k) {
            const double turn = encoded_join_turn(r.points, k);
            if (turn >= 0.0 && turn > tiers[i][1]) ++over;
        }
        std::fprintf(stderr,
                     "TEST-CURVE-09: r=0.1 circle at X=8192, %s tier -> certification %d, %zu "
                     "points, %zu joins past the tier, published turn %.6f rad, INDEPENDENT "
                     "maximum %.6f rad, quantization-conditioned joins %zu (%s)\n",
                     names[i], static_cast<int>(r.certification), r.points.size(), over,
                     r.encodedTurnMaxRad, measured, r.quantizationConditionedJoins,
                     r.diagnostic.c_str());
        const std::string label = std::string("TEST-CURVE-09: X=8192 circle, ") + names[i];
        // The published maximum must be the measured one at EVERY tier, whether
        // or not the tier was met. Suppressing a join because its chords are
        // short is what let this fixture publish `Certified` with a 0.084 rad
        // maximum while the polyline turned 1.54 rad.
        check_published_turn_matches_points(r, label);
        if (over > 0) {
            check(r.certification != CurveCertification::Certified,
                  label + ": an emitted turn past the tier can never be published as Certified — "
                          "the turn is EXACT on the emitted points, so a large quantization bound "
                          "proves nothing about it");
            check(!r.diagnostic.empty(), label + ": and the result says what is wrong with it");
            check(r.quantizationConditionedJoins > 0 &&
                      r.diagnostic.find("quantization-conditioned") != std::string::npos,
                  label + ": the conditioning count is REPORTED as its own diagnostic rather than "
                          "used to suppress the turn");
        } else {
            check(measured <= tiers[i][1] * 1.0001,
                  label + ": where no join passes the tier, the polyline really does meet it "
                          "(measured " +
                      std::to_string(degrees(measured)) + " deg against " +
                      std::to_string(degrees(tiers[i][1])) + " deg)");
        }
        ++tiers_seen[over > 0 ? 1 : 0];
    }
    check(tiers_seen[1] > 0,
          "TEST-CURVE-09: at its finest tier the fixture really is beyond repair — the emitted "
          "chords are a few ULPs long and no refinement makes their directions better");
    check(tiers_seen[0] > 0,
          "TEST-CURVE-09: and at its coarser tiers the turn rule REPAIRS the polyline instead of "
          "reporting it, which is what deleting the suppression made possible");
}

// --- FIX / verify §3 (REFUTED): coefficient reversal is not a singularity -----

// Astra's counterexample. The scalar cubic with unit weights and poles
// (0, 1/3, 1/30, 11/30) has Q(t) = 1 - 3.8t + 3.8t^2 >= 0.05 everywhere — a
// regular, monotonically traversed STRAIGHT LINE — yet its degree-5 Bernstein
// coefficients (1, 0.24, -0.14, -0.14, 0.24, 1) change sign. Scaled to 1e-4 mm
// its chord is 3.667e-5 mm, under the fine floor of 4.883e-5 mm, so the retired
// floor-plus-reversal rule labelled it a singular bracket and exempted its joins
// from the emitted-turn rule.
void test_curve_09_reversal_is_not_singularity_evidence() {
    for (const double scale : {1.0, 1e-4}) {
        std::vector<gp_XYZ> poles = {gp_XYZ(0.0, 0.0, 0.0), gp_XYZ(scale / 3.0, 0.0, 0.0),
                                     gp_XYZ(scale / 30.0, 0.0, 0.0),
                                     gp_XYZ(scale * 11.0 / 30.0, 0.0, 0.0)};
        std::vector<double> weights = {1.0, 1.0, 1.0, 1.0};
        RationalBezierSpan span;
        span.poles = poles;
        span.weights = weights;

        // INDEPENDENT: Q(t) = 3 * scale * (1 - 3.8t + 3.8t^2), whose minimum over
        // [0,1] is at t = 0.5 and equals 0.05 * 3 * scale > 0. The curve never
        // stops and never reverses.
        double q_min = 1e300;
        for (int i = 0; i <= 1000; ++i) {
            const double t = static_cast<double>(i) / 1000.0;
            q_min = std::min(q_min, 1.0 - 3.8 * t + 3.8 * t * t);
        }
        check(q_min > 0.049,
              "TEST-CURVE-09: the reversal counterexample's derivative never vanishes (minimum " +
                  std::to_string(q_min) + ")");

        // INDEPENDENT: its degree-5 Bernstein coefficients nevertheless change sign.
        const std::vector<gp_XYZ> q = onecad::tess::derivative_numerator_bernstein(span);
        bool positive = false;
        bool negative = false;
        for (const gp_XYZ& c : q) {
            if (c.X() > 0.0) positive = true;
            if (c.X() < 0.0) negative = true;
        }
        check(q.size() == 6 && positive && negative,
              "TEST-CURVE-09: and its degree-5 coefficients really do change sign");

        NCollection_Array1<gp_Pnt> occt(1, 4);
        for (int i = 0; i < 4; ++i) occt.SetValue(i + 1, gp_Pnt(poles[static_cast<std::size_t>(i)]));
        Handle(Geom_BezierCurve) bezier = new Geom_BezierCurve(occt);
        const TopoDS_Edge real_edge = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(bezier)).Edge();
        const double tolerance = 0.05;
        const CurveSampleResult r = sample(real_edge, tolerance);
        const double chord = scale * 11.0 / 30.0;
        std::size_t exempt_labels = 0;
        for (const CurveLeaf& leaf : r.leaves) {
            if (leaf.singularVertexAtStart || leaf.singularVertexAtEnd) ++exempt_labels;
        }
        std::fprintf(stderr,
                     "TEST-CURVE-09: reversal counterexample at scale %.0e -> certification %d, "
                     "%zu points, chord %.4e mm vs fine floor %.4e mm, singular vertex labels "
                     "%zu, published turn %.6f rad\n",
                     scale, static_cast<int>(r.certification), r.points.size(), chord,
                     tolerance / 1024.0, exempt_labels, r.encodedTurnMaxRad);
        const std::string label =
            "TEST-CURVE-09: reversal counterexample at scale " + num_string(scale);
        check(exempt_labels == 0,
              label + ": a REGULAR straight line gets no singular vertex label, whatever its "
                      "coefficients do");
        check_published_turn_matches_points(r, label);
        const double measured = independent_encoded_turn_max(r.points);
        check(measured <= kFiveDegrees * 1.0001 || r.certification != CurveCertification::Certified,
              label + ": the emitted-turn rule is APPLIED to it — either it meets the tier or the "
                      "result is not Certified (measured " +
                  std::to_string(degrees(measured)) + " deg)");
    }
}

// --- FIX / verify §4 (D-b): the genuine tangent jump J needs an interval ------

void test_curve_09_j_is_an_interval_not_a_number() {
    // A real C0 corner: the F07 spline turns about 166 degrees at knot 1.0, and
    // its one-sided tangents there are both WELL RESOLVED, so J's interval is
    // tight and the join still passes.
    Handle(Geom_BSplineCurve) spline = f07_spline();
    const TopoDS_Edge edge = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(spline)).Edge();
    const CurveSampleResult r = sample(edge, 0.01);
    std::size_t corner = 0;
    double corner_turn = 0.0;
    for (std::size_t i = 1; i + 1 < r.points.size() && i < r.leaves.size(); ++i) {
        const double turn = encoded_join_turn(r.points, i);
        if (turn > corner_turn) {
            corner_turn = turn;
            corner = i;
        }
    }
    double jump = -1.0;
    double jump_uncertainty = -1.0;
    if (corner > 0 && corner < r.leaves.size()) {
        jump = angle_between(r.leaves[corner - 1].endTangent, r.leaves[corner].startTangent);
        jump_uncertainty = r.leaves[corner - 1].endTangentUncertaintyRad +
                           r.leaves[corner].startTangentUncertaintyRad;
    }
    std::fprintf(stderr,
                 "TEST-CURVE-09: F07 C0 join %zu -> emitted turn %.4f deg, computed J %.4f deg, "
                 "J uncertainty %.3e rad, certification %d\n",
                 corner, degrees(corner_turn), degrees(jump), jump_uncertainty,
                 static_cast<int>(r.certification));
    check(degrees(corner_turn) > 150.0,
          "TEST-CURVE-09: the F07 fixture really does carry a ~166 degree C0 corner");
    check(jump_uncertainty >= 0.0 && jump_uncertainty < 1e-6,
          "TEST-CURVE-09: both one-sided tangents at that corner are well resolved, so J's "
          "interval is tight (measured " +
              std::to_string(jump_uncertainty) + " rad)");
    check(r.certification == CurveCertification::Certified,
          "TEST-CURVE-09: a genuine C0 corner with a tight J interval still certifies");
    check_published_turn_matches_points(r, "TEST-CURVE-09: F07 C0 corner");

    // A cusp AT a leaf boundary: Q vanishes at that very parameter, so BOTH
    // one-sided coefficients compute to ~0, their directions are unresolved, J's
    // interval is the whole [0, pi] — and the 180 degree turn there is neither
    // certified nor reported as a violation. This is the J interval doing the
    // work the retired `singularBracket` exemption used to do, on evidence about
    // the CURVE rather than on a sign pattern in the coefficients.
    Handle(Geom_BezierCurve) dyadic = polynomial_cubic(gp_Pnt(0, 0, 0), gp_Pnt(1, 1, 0),
                                                       gp_Pnt(0, 1, 0), gp_Pnt(1, 0, 0));
    const TopoDS_Edge dyadic_edge = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(dyadic)).Edge();
    const CurveSampleResult rd = sample(dyadic_edge, 0.001);
    double widest_j_interval = 0.0;
    double turn_at_widest = 0.0;
    for (std::size_t i = 1; i + 1 < rd.points.size() && i < rd.leaves.size(); ++i) {
        const double eta = rd.leaves[i - 1].endTangentUncertaintyRad +
                           rd.leaves[i].startTangentUncertaintyRad;
        if (eta > widest_j_interval) {
            widest_j_interval = eta;
            turn_at_widest = encoded_join_turn(rd.points, i);
        }
    }
    std::fprintf(stderr,
                 "TEST-CURVE-09: cusp at a dyadic parameter -> certification %d, widest J "
                 "half-interval %.4f rad at a join turning %.2f deg, published turn %.6f rad\n",
                 static_cast<int>(rd.certification), widest_j_interval, degrees(turn_at_widest),
                 rd.encodedTurnMaxRad);
    check(widest_j_interval >= 0.5 * kPi,
          "TEST-CURVE-09: where Q vanishes ON a join, the one-sided tangent DIRECTIONS are "
          "unresolved and J's interval opens to [0, pi] (measured " +
              std::to_string(widest_j_interval) + " rad)");
    check(rd.certification == CurveCertification::Certified,
          "TEST-CURVE-09: so the cusp's own 180 degree turn is not reported as a defect");
    check_published_turn_matches_points(rd, "TEST-CURVE-09: dyadic cusp");

    // The SAME cusp at a non-dyadic parameter never lands on a join: it sits
    // inside a leaf whose chord crosses it, so the neighbouring join has a proved
    // J = 0 and a 180 degree emitted turn. That IS an excess over the curve's own
    // jump at that vertex, and it is reported rather than excused.
    Handle(Geom_BezierCurve) cusp = polynomial_cubic(gp_Pnt(1, -1, 0), gp_Pnt(-1, 2, 0),
                                                     gp_Pnt(0, -4, 0), gp_Pnt(4, 8, 0));
    const TopoDS_Edge cusp_edge = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(cusp)).Edge();
    const CurveSampleResult rc = sample(cusp_edge, 0.002);
    std::fprintf(stderr,
                 "TEST-CURVE-09: cusp at t=1/3 (non-dyadic) -> certification %d, %zu leaves, "
                 "published turn %.6f rad (%s)\n",
                 static_cast<int>(rc.certification), rc.leaves.size(), rc.encodedTurnMaxRad,
                 rc.diagnostic.c_str());
    check(rc.certification != CurveCertification::Certified &&
              rc.diagnostic.find("emitted turn") != std::string::npos,
          "TEST-CURVE-09: a 180 degree emitted turn at a join where J = 0 is PROVED is reported, "
          "whatever the leaf next to it could not resolve");
    check(!rc.points.empty(),
          "TEST-CURVE-09: and the polyline is still published — the position bound is untouched");
    check_published_turn_matches_points(rc, "TEST-CURVE-09: non-dyadic cusp");
}

// --- FIX / verify §6: subnormal homogeneous coordinates ----------------------

void test_curve_09_f6_subnormal_homogeneous_coordinates() {
    // Astra's counterexample: w = (2^-1074, 1), P = (1e-5, 2e-5) mm. The first
    // homogeneous coordinate fl(2^-1074 * 1e-5) is ZERO, so the retained endpoint
    // dehomogenizes to 0/w0 = 0 while the exact endpoint is 1e-5 mm — a 1e-5 mm
    // error the depth-1 roundoff enclosure (2.9e-20 mm) misses by fifteen orders
    // of magnitude. Every weight stays positive and every pole stays finite, so
    // `control_net_is_valid` alone cannot see it.
    const double tiny = 0x1p-1074;
    check(tiny > 0.0 && tiny * 1e-5 == 0.0,
          "TEST-CURVE-09: fl(2^-1074 * 1e-5) really is zero while 2^-1074 itself is positive");
    {
        RationalBezierSpan span;
        span.poles = {gp_XYZ(1e-5, 0.0, 0.0), gp_XYZ(2e-5, 0.0, 0.0)};
        span.weights = {tiny, 1.0};
        check(onecad::tess::control_net_is_valid(span),
              "TEST-CURVE-09: Astra's exact net passes the finite-and-positive test, which is "
              "precisely why it needed its own detection");
        check(!onecad::tess::homogeneous_arithmetic_is_supported(span),
              "TEST-CURVE-09: but its homogeneous coordinates underflow, and that IS detected");
    }

    // `Geom_BezierCurve` refuses a weight of 2^-1074 outright, so the end-to-end
    // fixture uses the smallest weight OCCT does accept together with poles small
    // enough that the PRODUCT leaves the normal range — the same failure, reached
    // through the real conversion.
    NCollection_Array1<gp_Pnt> poles(1, 2);
    poles.SetValue(1, gp_Pnt(1e-10, 0.0, 0.0));
    poles.SetValue(2, gp_Pnt(2e-10, 0.0, 0.0));
    NCollection_Array1<double> weights(1, 2);
    weights.SetValue(1, 1e-300);
    weights.SetValue(2, 1.0);
    check(!std::isnormal(1e-300 * 1e-10) && 1e-300 * 1e-10 > 0.0,
          "TEST-CURVE-09: fl(1e-300 * 1e-10) is subnormal — present, but with a fraction of its "
          "significand left");
    Handle(Geom_BezierCurve) bezier;
    try {
        bezier = new Geom_BezierCurve(poles, weights);
    } catch (const Standard_Failure&) {
        bezier.Nullify();
    }
    check(!bezier.IsNull(), "TEST-CURVE-09: OCCT accepts the fixture's weights");
    if (bezier.IsNull()) return;
    const TopoDS_Edge edge = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(bezier)).Edge();
    const CurveSampleResult r = sample(edge, 0.05);
    std::fprintf(stderr,
                 "TEST-CURVE-09: subnormal homogeneous net -> certification %d, %zu points (%s)\n",
                 static_cast<int>(r.certification), r.points.size(), r.diagnostic.c_str());
    check(r.certification != CurveCertification::Certified,
          "TEST-CURVE-09: a span whose homogeneous arithmetic leaves the normal range is never "
          "certified");
    check(!r.diagnostic.empty(), "TEST-CURVE-09: and it names the reason");
}

void test_curve_09_f6_enclosure_is_charged_into_the_cone() {
    // verify §6 / R1(c) MAJOR 4: a pole perturbation delta propagates into the
    // derivative-numerator coefficients by at most 4 n wmax^2 delta, so the
    // positional roundoff enclosure of a DEEP net has to be charged into the C2
    // uncertainty E — otherwise the cone is certified about coefficients that
    // were computed from poles the enclosure admits are wrong.
    RationalBezierSpan span;
    span.poles = {gp_XYZ(1e9, 0.0, 0.0), gp_XYZ(1e9, 1.0, 0.0), gp_XYZ(1e9, 2.0, 1.0),
                  gp_XYZ(1e9, 3.0, 0.0)};
    span.weights = {1.0, 0.9, 0.8, 1.0};
    span.sourceMaxAbs = gp_XYZ(1e9, 3.0, 1.0);
    span.depth = 32;

    const double enclosure = onecad::tess::pole_roundoff_enclosure(span);
    double qmax = 0.0;
    for (const gp_XYZ& c : onecad::tess::derivative_numerator_bernstein(span)) {
        qmax = std::max(qmax, c.Modulus());
    }
    const int n = static_cast<int>(span.poles.size()) - 1;
    double wmax = 0.0;
    for (const double w : span.weights) wmax = std::max(wmax, w);
    const double pole_term = 4.0 * static_cast<double>(n) * wmax * wmax * enclosure / qmax;

    const onecad::tess::ConeVerdict deep =
        onecad::tess::tangent_cone_verdict(span, kFiveDegrees, gp_XYZ(0, 1, 0), enclosure);
    const onecad::tess::ConeVerdict bare =
        onecad::tess::tangent_cone_verdict(span, kFiveDegrees, gp_XYZ(0, 1, 0), 0.0);
    std::fprintf(stderr,
                 "TEST-CURVE-09: depth-32 cubic at 1e9 mm -> enclosure %.6e mm, 4n w^2 "
                 "delta / qmax = %.6e, E with enclosure %.6e, E without %.6e\n",
                 enclosure, pole_term, deep.excludedMass, bare.excludedMass);
    check(enclosure > 0.0 && pole_term > 0.0,
          "TEST-CURVE-09: the deep fixture really does carry a nonzero pole enclosure");
    check(deep.excludedMass >= pole_term,
          "TEST-CURVE-09: E includes the 4 n wmax^2 delta pole term (measured " +
              std::to_string(deep.excludedMass) + " against " + std::to_string(pole_term) + ")");
    check(deep.excludedMass > bare.excludedMass,
          "TEST-CURVE-09: charging the enclosure strictly increases E");
}

// --- FIX / R1(c) MINOR 7: `achieved` is what the retry actually CERTIFIED -----

void test_curve_09_ladder_achieved_is_what_was_certified() {
    // A relaxed retry that is itself only QualityLimited did NOT achieve the
    // relaxed tolerance either, and must not report that it did. A retry that
    // came through the approximate fallback keeps its PROVENANCE — quality
    // limitation is a separate flag, not a replacement label.
    Handle(Geom_BSplineCurve) spline = oscillating_spline();
    const TopoDS_Edge edge = BRepBuilderAPI_MakeEdge(Handle(Geom_Curve)(spline)).Edge();
    BRepAdaptor_Curve adaptor(edge);
    CurveSampleRequest tight;
    tight.chordToleranceMm = 0.05;
    tight.angularToleranceRad = kFiveDegrees;
    CurveSampleRequest doubled = tight;
    doubled.chordToleranceMm = 0.10;
    doubled.angularToleranceRad = 2.0 * kFiveDegrees;
    CurveSampleLimits roomy;
    const std::size_t at_doubled = sample_edge_curve(adaptor, doubled, roomy, {}).leaves.size();
    CurveSampleLimits capped;
    capped.maxSegmentsPerEdge = at_doubled + 8;
    const CurveSampleResult r =
        onecad::tess::detail::sample_edge_polyline(adaptor, "e:1", tight, capped);
    const CurveSampleResult retry = sample_edge_curve(adaptor, doubled, capped, {});
    std::fprintf(stderr,
                 "TEST-CURVE-09: ladder -> certification %d, qualityLimited %d, requested %.6g / "
                 "achieved %.6g mm; the retry alone is certification %d\n",
                 static_cast<int>(r.certification), r.qualityLimited ? 1 : 0,
                 r.requestedChordToleranceMm, r.achievedChordToleranceMm,
                 static_cast<int>(retry.certification));
    check(r.qualityLimited,
          "TEST-CURVE-09: a relaxed retry always carries the quality-limited flag");
    check(r.requestedChordToleranceMm == 0.05,
          "TEST-CURVE-09: and the tolerance that was REQUESTED");
    if (retry.certification == CurveCertification::Certified) {
        check(r.achievedChordToleranceMm == 0.10,
              "TEST-CURVE-09: a retry that CERTIFIED at 0.10 mm reports 0.10 mm as achieved");
        check(r.certification == CurveCertification::QualityLimited,
              "TEST-CURVE-09: and the retry's own Certified never leaks");
    } else {
        check(r.achievedChordToleranceMm < 0.0,
              "TEST-CURVE-09: a retry that certified NOTHING reports no achieved tolerance");
        check(r.certification == retry.certification,
              "TEST-CURVE-09: and the retry's own provenance is preserved, not overwritten");
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
    // ROUND 3 — Astra followup C1/C2/F5/F6/F7/F9 and review findings PR-07/PR-09
    test_curve_08_c1_regular_corner_turns();
    test_curve_08_c1_genuine_cusp_bracket();
    test_curve_08_c1_oracle_integrity();
    test_curve_08_c2_satisfied_is_a_complete_bound();
    test_curve_08_c2_four_regimes();
    test_curve_08_c2_numerator_at_the_endpoints();
    test_curve_08_c2_determinism();
    test_curve_08_f5_encoded_chord_withdraws_the_angular_claim();
    test_curve_08_f6_invalid_control_nets_never_certify();
    test_curve_08_f6_roundoff_enclosure_covers_the_discrepancy();
    test_curve_08_f7_near_period_trim_stays_open();
    test_curve_08_f7_genuine_closure_still_snaps();
    test_curve_08_f9_body_segment_cap_is_hard();
    test_curve_08_doubling_ladder_separates_requested_from_achieved();
    // ROUND 3 FIX ROUND — R1(c) and the Astra `verify` refutations
    test_curve_09_emitted_turn_is_never_suppressed();
    test_curve_09_reversal_is_not_singularity_evidence();
    test_curve_09_j_is_an_interval_not_a_number();
    test_curve_09_f6_subnormal_homogeneous_coordinates();
    test_curve_09_f6_enclosure_is_charged_into_the_cone();
    test_curve_09_ladder_achieved_is_what_was_certified();
    if (g_failures == 0) std::fprintf(stderr, "curve_sampler: OK\n");
    return g_failures;
}
