// RevolveProfileAxis.cpp — see RevolveProfileAxis.h.
#include "ops/RevolveProfileAxis.h"

#include <algorithm>
#include <cmath>
#include <limits>
#include <string>

#include <BRepAdaptor_Curve.hxx>
#include <BRep_Tool.hxx>
#include <GeomAbs_CurveType.hxx>
#include <Precision.hxx>
#include <TopExp_Explorer.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Edge.hxx>
#include <gp_Circ.hxx>
#include <gp_Dir.hxx>
#include <gp_Elips.hxx>
#include <gp_Pln.hxx>
#include <gp_Pnt.hxx>
#include <gp_Vec.hxx>

#include "kernel/validation/GeometryPrecision.h"
#include "ops/OpCommon.h"

namespace onecad::ops {

using nlohmann::json;

namespace {

constexpr double kTwoPi = 2.0 * 3.14159265358979323846;

// Signed in-plane offset range [lo, hi] of the profile boundary from the axis.
// side(P) = ((P − O) × a) · n = (P − O) · w with w = a × n, linear in P. Vertices
// alone are NOT enough — a full circle has one vertex (its seam) and an arc's
// extremum can lie between its endpoints — so every edge contributes its endpoints
// plus the analytic extrema of a Circle/Ellipse, and a free-form curve is sampled.
// Adversarial review 2026-09-02 measured the vertex-only form missing every
// crossing circle profile. Returns false when the profile has no edge.
bool signed_offset_range(const TopoDS_Face& profile, const gp_Ax1& axis, const gp_Dir& profile_n,
                         double& lo, double& hi) {
    const gp_Vec w = gp_Vec(axis.Direction()).Crossed(gp_Vec(profile_n));
    auto side_of = [&](const gp_Pnt& p) { return gp_Vec(axis.Location(), p).Dot(w); };
    bool any = false;
    auto take = [&](double side) {
        lo = any ? std::min(lo, side) : side;
        hi = any ? std::max(hi, side) : side;
        any = true;
    };
    for (TopExp_Explorer exp(profile, TopAbs_EDGE); exp.More(); exp.Next()) {
        const TopoDS_Edge edge = TopoDS::Edge(exp.Current());
        if (BRep_Tool::Degenerated(edge)) continue;
        BRepAdaptor_Curve curve(edge);
        const double first = curve.FirstParameter();
        const double last = curve.LastParameter();
        take(side_of(curve.Value(first)));
        take(side_of(curve.Value(last)));
        // Extremum parameters of a conic: side(t) = s_c + A cos t + B sin t.
        auto take_conic_extrema = [&](const gp_Dir& xdir, const gp_Dir& ydir, double rx, double ry) {
            const double a = gp_Vec(xdir).Dot(w) * rx;
            const double b = gp_Vec(ydir).Dot(w) * ry;
            const double t_star = std::atan2(b, a);
            for (const double t : {t_star, t_star + kTwoPi / 2.0}) {
                // Fold t into [first, first + 2π) and keep it if it lies on the arc.
                double tt = first + std::fmod(std::fmod(t - first, kTwoPi) + kTwoPi, kTwoPi);
                if (tt <= last + 1.0e-9) take(side_of(curve.Value(tt)));
            }
        };
        switch (curve.GetType()) {
            case GeomAbs_Line: break;
            case GeomAbs_Circle: {
                const gp_Circ c = curve.Circle();
                take_conic_extrema(c.XAxis().Direction(), c.YAxis().Direction(), c.Radius(), c.Radius());
                break;
            }
            case GeomAbs_Ellipse: {
                const gp_Elips e = curve.Ellipse();
                take_conic_extrema(e.XAxis().Direction(), e.YAxis().Direction(), e.MajorRadius(),
                                   e.MinorRadius());
                break;
            }
            default: {
                constexpr int kSamples = 128;
                for (int k = 1; k < kSamples; ++k) {
                    take(side_of(curve.Value(first + (last - first) * k / kSamples)));
                }
                break;
            }
        }
    }
    return any;
}

double point_axis_distance(const gp_Pnt& p, const gp_Ax1& axis) {
    return gp_Vec(axis.Location(), p).Crossed(gp_Vec(axis.Direction())).Magnitude();
}

// True closest approach of one edge to the (infinite) axis line, in 3D. A line is
// closed form; any other curve is sampled densely and the best bracket refined by
// golden section, so an arc's INTERIOR minimum counts, not only its endpoints.
double edge_axis_distance(const TopoDS_Edge& edge, const gp_Ax1& axis) {
    BRepAdaptor_Curve curve(edge);
    const double first = curve.FirstParameter();
    const double last = curve.LastParameter();
    const gp_Vec a(axis.Direction());
    if (curve.GetType() == GeomAbs_Line) {
        // |u + t·v| over t ∈ [0, 1] with u = (P0 − O) × a, v = (P1 − P0) × a.
        const gp_Pnt p0 = curve.Value(first);
        const gp_Vec u = gp_Vec(axis.Location(), p0).Crossed(a);
        const gp_Vec v = gp_Vec(p0, curve.Value(last)).Crossed(a);
        const double vv = v.SquareMagnitude();
        const double t = vv > 0.0 ? std::clamp(-u.Dot(v) / vv, 0.0, 1.0) : 0.0;
        return (u + v * t).Magnitude();
    }
    auto dist_at = [&](double t) { return point_axis_distance(curve.Value(t), axis); };
    constexpr int kSamples = 256;
    int best = 0;
    double best_d = dist_at(first);
    for (int k = 1; k <= kSamples; ++k) {
        const double d = dist_at(first + (last - first) * k / kSamples);
        if (d < best_d) { best_d = d; best = k; }
    }
    double lo = first + (last - first) * std::max(best - 1, 0) / kSamples;
    double hi = first + (last - first) * std::min(best + 1, kSamples) / kSamples;
    constexpr double kInvPhi = 0.61803398874989484820;
    for (int i = 0; i < 200 && hi - lo > 1.0e-15 * (1.0 + std::abs(lo) + std::abs(hi)); ++i) {
        const double m1 = hi - (hi - lo) * kInvPhi;
        const double m2 = lo + (hi - lo) * kInvPhi;
        if (dist_at(m1) < dist_at(m2)) hi = m2; else lo = m1;
    }
    return std::min(best_d, dist_at(0.5 * (lo + hi)));
}

OpOutcome refusal(const std::string& code, const std::string& message, json evidence) {
    OpOutcome failure = OpOutcome::fail("OP_FAILED", message);
    failure.diagnostics.push_back({{"severity", "error"},
                                   {"code", code},
                                   {"message", message},
                                   {"stage", "classify"},
                                   {"evidence", {{"revolve", std::move(evidence)}}}});
    return failure;
}

}  // namespace

std::optional<OpOutcome> classify_revolve_profile(const TopoDS_Face& profile, const gp_Ax1& axis) {
    const double resolution = kernel::validation::precision_of(profile).authoring_resolution();
    // OCCT's MakeRevol decides "this edge lies on the axis" and "this edge meets the
    // axis" with two different Confusion tests. At exactly that distance they
    // disagree and its validity check indexes an EMPTY parallel-lines extrema result
    // (SIGSEGV, kernel audit 2026-11-11 finding 1). A rounding guard keeps the
    // pass-through band strictly below that boundary; everything between it and the
    // authoring resolution is refused by name, never snapped.
    const double on_axis = Precision::Confusion() * (1.0 - 1.0e-3);

    // A profile that CROSSES its axis sweeps through itself: OCCT builds the
    // figure-8 solid, BRepCheck accepts it face by face, and the volume is positive.
    // Touching the axis (within on_axis) is legal — it makes the degenerate pole
    // edges of a shaft or a sphere.
    gp_Pln profile_pln;
    gp_Dir profile_n;
    double lo = 0.0, hi = 0.0;
    if (planar_face_plane_normal(profile, profile_pln, profile_n) &&
        signed_offset_range(profile, axis, profile_n, lo, hi) && lo < -on_axis && hi > on_axis) {
        return refusal("REVOLVE_PROFILE_CROSSES_AXIS",
                       "Revolve profile crosses its axis (the sweep would pass through itself)",
                       {{"minSignedOffset", lo}, {"maxSignedOffset", hi},
                        {"authoringResolution", resolution}, {"onAxisTolerance", on_axis}});
    }

    // Per EDGE, not per profile: an edge parallel to the axis at ~Confusion is the
    // crashing input even when another edge of the same profile touches the axis.
    // An edge authored exactly AT the resolution measures a few ulps below it after
    // the sketch solve and plane mapping; it is at-resolution, not below.
    const double below_resolution = resolution * (1.0 - 1.0e-9);
    double nearest = std::numeric_limits<double>::infinity();
    for (TopExp_Explorer exp(profile, TopAbs_EDGE); exp.More(); exp.Next()) {
        const TopoDS_Edge edge = TopoDS::Edge(exp.Current());
        if (BRep_Tool::Degenerated(edge)) continue;
        const double d = edge_axis_distance(edge, axis);
        if (d >= on_axis && d < below_resolution) nearest = std::min(nearest, d);
    }
    if (std::isfinite(nearest)) {
        return refusal("REVOLVE_PROFILE_NEAR_AXIS",
                       "Revolve profile comes closer to its axis than the authoring resolution "
                       "without touching it (the sweep would leave a sub-resolution bore or pinch)",
                       {{"minAxisDistance", nearest}, {"onAxisTolerance", on_axis},
                        {"authoringResolution", resolution}});
    }
    return std::nullopt;
}

}  // namespace onecad::ops
