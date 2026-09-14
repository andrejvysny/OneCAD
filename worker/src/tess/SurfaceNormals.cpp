// SurfaceNormals.cpp — see SurfaceNormals.h.
#include "tess/SurfaceNormals.h"

#include <algorithm>
#include <cmath>
#include <cstddef>
#include <cstdio>
#include <exception>
#include <limits>

#include <BRepAdaptor_Surface.hxx>
#include <BRepTools.hxx>
#include <BRep_Tool.hxx>
#include <GeomAbs_SurfaceType.hxx>
#include <Geom_BSplineSurface.hxx>
#include <Geom_BezierSurface.hxx>
#include <NCollection_Array2.hxx>
#include <Poly_Triangle.hxx>
#include <Standard_Failure.hxx>
#include <TopAbs_ShapeEnum.hxx>
#include <TopExp_Explorer.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Edge.hxx>
#include <gp_Cone.hxx>
#include <gp_Cylinder.hxx>
#include <gp_Mat.hxx>
#include <gp_Pnt2d.hxx>
#include <gp_Sphere.hxx>
#include <gp_Torus.hxx>
#include <gp_Trsf.hxx>
#include <gp_Vec.hxx>

#include "tess/CurveSampler.h"
#include "util/Log.h"

namespace onecad::tess {

namespace {

constexpr double kInfinity = std::numeric_limits<double>::infinity();
// sqrt(3): one coordinate triple's Euclidean error from a per-component bound.
constexpr double kSqrt3 = 1.7320508075688772;

std::string num_text(double v) {
    char buf[32];
    std::snprintf(buf, sizeof(buf), "%.6g", v);
    return std::string(buf);
}

// --- Astra F3/F4: the representation-derived derivative budget --------------

// Largest |coordinate| over a patch's poles, and the weight window. Poles come
// back from `BRepAdaptor_Surface::Bezier()/BSpline()` already in WORLD placement
// (OCCT 8.0.1 `GeomAdaptor_TransformedSurface` overrides both, verified against
// a translated face), so no second location factor belongs here.
struct PoleExtent {
    double maxAbs = 0.0;
    double minWeight = 1.0;
    double maxWeight = 1.0;
    bool valid = false;
};

template <typename Patch>
PoleExtent pole_extent(const Patch& patch) {
    PoleExtent out;
    const int nu = patch->NbUPoles();
    const int nv = patch->NbVPoles();
    if (nu < 1 || nv < 1) return out;
    const bool rational = patch->IsURational() || patch->IsVRational();
    double mx = 0.0, my = 0.0, mz = 0.0;
    double lo = kInfinity, hi = 0.0;
    for (int i = 1; i <= nu; ++i) {
        for (int j = 1; j <= nv; ++j) {
            const gp_Pnt p = patch->Pole(i, j);
            if (!std::isfinite(p.X()) || !std::isfinite(p.Y()) || !std::isfinite(p.Z())) return out;
            mx = std::max(mx, std::abs(p.X()));
            my = std::max(my, std::abs(p.Y()));
            mz = std::max(mz, std::abs(p.Z()));
            if (rational) {
                // The convex-hull/de Boor argument needs a STRICTLY positive
                // finite denominator; anything else leaves the patch unbounded.
                const double w = patch->Weight(i, j);
                if (!std::isfinite(w) || !(w > 0.0)) return out;
                lo = std::min(lo, w);
                hi = std::max(hi, w);
            }
        }
    }
    if (!rational) {
        lo = 1.0;
        hi = 1.0;
    }
    out.maxAbs = std::sqrt(mx * mx + my * my + mz * mz);
    out.minWeight = lo;
    out.maxWeight = hi;
    out.valid = std::isfinite(out.maxAbs) && lo > 0.0 && std::isfinite(hi);
    return out;
}

// The WP08 F6 enclosure family, applied to a tensor-product de Boor ladder.
// `pole_roundoff_multiplier(depth, degree)` is `(2g + u(1+g))/(1-g)` with
// `g = gamma_K`, `K = depth*(degree+3)+2` (CurveSampler.h). Evaluating S and its
// first derivatives at one (u,v) runs the u ladder once per surviving v row and
// then the v ladder once, plus one differencing step for the derivative, so
// `depth = uDegree + vDegree + 1` counts the recurrence levels the rounding
// actually accumulates over and `degree = max(uDegree, vDegree)` the width of
// each level. Conservative op-count accounting in the accepted WP08 family, not
// a new derivation: an under-count would UNDER-state the budget, so the count is
// deliberately the larger of the two ladders' lengths added together.
//
// That product is a POSITION enclosure, in mm. The DERIVATIVE control net is
// `Q_i = degree * (P_{i+1} - P_i) / (u_{i+degree+1} - u_{i+1})`, so a position
// error of `e` on each pole becomes at most `2 * degree * e / span` on each
// derivative pole, and by the convex-hull property that bounds the evaluated
// derivative too. `span` is the SMALLEST knot interval rather than the knot
// window it divides by — the window is never smaller, so this over-states the
// error and fails closed. Dividing is also what makes the bound
// reparameterization invariant: compress the knot vector by 1/lambda and both
// `|S_u|` and this bound grow by lambda (R1(c) MAJOR 5).
struct PatchBudget {
    double u = kInfinity;
    double v = kInfinity;
};

PatchBudget patch_derivative_budget(int uDegree, int vDegree, double uSpanMin, double vSpanMin,
                                    const PoleExtent& extent, double scale) {
    PatchBudget out;
    if (!extent.valid || !(uSpanMin > 0.0) || !(vSpanMin > 0.0) || !(scale > 0.0) ||
        !std::isfinite(scale)) {
        return out;
    }
    const int depth = std::max(uDegree, 0) + std::max(vDegree, 0) + 1;
    const int degree = std::max(std::max(uDegree, vDegree), 0);
    const double mult = pole_roundoff_multiplier(depth, degree);
    if (!std::isfinite(mult)) return out;
    // Dehomogenisation divides by w(u,v) >= minWeight while the recurrence
    // handled magnitudes up to maxWeight * |pole|; the ratio is 1 for every
    // polynomial patch.
    const double ratio = extent.maxWeight / extent.minWeight;
    // `GeomAdaptor_TransformedSurface` evaluates on the ORIGINAL surface and
    // applies the location afterwards ("D1 evaluation. Applies transformation
    // after evaluation."), so the magnitudes the recurrence actually rounded are
    // the LOCAL pole magnitudes; `|s|` then carries the result into the world
    // units the caller's derivatives are in. Reading `BRepAdaptor_Surface's`
    // transformed `BSpline()` instead made the bound grow with the face's
    // distance from the world origin, which nothing in the evaluation does.
    const double positionMm = mult * extent.maxAbs * ratio * scale;
    if (!std::isfinite(positionMm)) return out;
    out.u = positionMm * 2.0 * static_cast<double>(std::max(uDegree, 1)) / uSpanMin;
    out.v = positionMm * 2.0 * static_cast<double>(std::max(vDegree, 1)) / vSpanMin;
    if (!std::isfinite(out.u) || !std::isfinite(out.v)) return PatchBudget{};
    return out;
}

// Smallest positive interval of a B-spline's knot vector. OCCT stores knots
// strictly increasing with separate multiplicities, so consecutive entries
// already differ.
double min_knot_span(const Handle(Geom_BSplineSurface)& patch, bool inU) {
    const int n = inU ? patch->NbUKnots() : patch->NbVKnots();
    if (n < 2) return 0.0;
    double best = kInfinity;
    for (int i = 1; i < n; ++i) {
        const double lo = inU ? patch->UKnot(i) : patch->VKnot(i);
        const double hi = inU ? patch->UKnot(i + 1) : patch->VKnot(i + 1);
        const double span = hi - lo;
        if (std::isfinite(span) && span > 0.0) best = std::min(best, span);
    }
    return std::isfinite(best) ? best : 0.0;
}

}  // namespace

DerivativeBudget derivative_error_budget(const BRepAdaptor_Surface& surf) {
    DerivativeBudget out;
    try {
        switch (surf.GetType()) {
            case GeomAbs_Plane: {
                // S_u and S_v are the transformed axis directions: no sum, no
                // cancellation, magnitude |s|. The plane's LOCATION never enters
                // its derivative, which is why a location-scaled bound would be
                // wrong rather than merely conservative.
                const double s = std::abs(surf.Trsf().ScaleFactor());
                const double m = std::max(std::isfinite(s) && s > 0.0 ? s : 1.0, 1.0);
                out.uAbsoluteMm = kAnalyticDerivativeUlps * kUnitRoundoff * kSqrt3 * m;
                out.vAbsoluteMm = out.uAbsoluteMm;
                out.known = true;
                out.reason = "plane: closed-form unit axis derivatives";
                break;
            }
            case GeomAbs_Cylinder: {
                const double r = std::abs(surf.Cylinder().Radius());
                if (!std::isfinite(r)) break;
                out.uAbsoluteMm =
                    kAnalyticDerivativeUlps * kUnitRoundoff * kSqrt3 * std::max(r, 1.0);
                out.vAbsoluteMm = out.uAbsoluteMm;
                out.known = true;
                out.reason = "cylinder: closed-form, radius-scaled";
                break;
            }
            case GeomAbs_Cone: {
                const gp_Cone cone = surf.Cone();
                const double r = std::abs(cone.RefRadius());
                const double apex = cone.Apex().XYZ().Modulus();
                if (!std::isfinite(r) || !std::isfinite(apex)) break;
                // `R + v*sin(a)` CANCELS to zero at the apex, so the magnitude
                // the closed form handles is the reference radius and the apex
                // offset, never the (vanishing) result.
                out.uAbsoluteMm = kAnalyticDerivativeUlps * kUnitRoundoff * kSqrt3 *
                                  std::max(std::max(r, apex), 1.0);
                out.vAbsoluteMm = out.uAbsoluteMm;
                out.known = true;
                out.reason = "cone: closed-form, reference-radius scaled";
                break;
            }
            case GeomAbs_Sphere: {
                const double r = std::abs(surf.Sphere().Radius());
                if (!std::isfinite(r)) break;
                out.uAbsoluteMm =
                    kAnalyticDerivativeUlps * kUnitRoundoff * kSqrt3 * std::max(r, 1.0);
                out.vAbsoluteMm = out.uAbsoluteMm;
                out.known = true;
                out.reason = "sphere: closed-form, radius-scaled";
                break;
            }
            case GeomAbs_Torus: {
                const gp_Torus torus = surf.Torus();
                const double m = std::abs(torus.MajorRadius()) + std::abs(torus.MinorRadius());
                if (!std::isfinite(m)) break;
                out.uAbsoluteMm =
                    kAnalyticDerivativeUlps * kUnitRoundoff * kSqrt3 * std::max(m, 1.0);
                out.vAbsoluteMm = out.uAbsoluteMm;
                out.known = true;
                out.reason = "torus: closed-form, (R+r)-scaled";
                break;
            }
            case GeomAbs_BezierSurface: {
                // The ORIGINAL (untransformed) patch: see patch_derivative_budget.
                const Handle(Geom_BezierSurface) patch = surf.AdaptorSurfaceOriginal().Bezier();
                if (patch.IsNull()) break;
                // A Bezier patch's domain is [0,1] in both directions.
                const PatchBudget budget =
                    patch_derivative_budget(patch->UDegree(), patch->VDegree(), 1.0, 1.0,
                                            pole_extent(patch), std::abs(surf.Trsf().ScaleFactor()));
                if (!std::isfinite(budget.u) || !std::isfinite(budget.v)) break;
                out.uAbsoluteMm = budget.u;
                out.vAbsoluteMm = budget.v;
                out.known = true;
                out.reason = "Bezier patch: WP08 F6 roundoff enclosure of the control net";
                break;
            }
            case GeomAbs_BSplineSurface: {
                const Handle(Geom_BSplineSurface) patch = surf.AdaptorSurfaceOriginal().BSpline();
                if (patch.IsNull()) break;
                const PatchBudget budget = patch_derivative_budget(
                    patch->UDegree(), patch->VDegree(), min_knot_span(patch, /*inU=*/true),
                    min_knot_span(patch, /*inU=*/false), pole_extent(patch),
                    std::abs(surf.Trsf().ScaleFactor()));
                if (!std::isfinite(budget.u) || !std::isfinite(budget.v)) break;
                out.uAbsoluteMm = budget.u;
                out.vAbsoluteMm = budget.v;
                out.known = true;
                out.reason = "B-spline patch: WP08 F6 roundoff enclosure of the control net";
                break;
            }
            default:
                break;
        }
    } catch (const Standard_Failure&) {
        out.known = false;
    }
    if (!out.known) {
        out.uAbsoluteMm = kInfinity;
        out.vAbsoluteMm = kInfinity;
        // Astra F3: an asserted bound is not a bound. A surface of revolution,
        // a surface of extrusion, an offset surface and an unrecognised type all
        // evaluate through a basis this file has no enclosure for, so every node
        // on them is Unresolved rather than silently Surface.
        out.reason = "no derivative-error enclosure exists for this surface representation";
    }
    return out;
}

double normal_angular_error_rad(const DerivativeBudget& budget, const gp_Vec& du,
                                const gp_Vec& dv) {
    if (!budget.known || !std::isfinite(budget.uAbsoluteMm) ||
        !std::isfinite(budget.vAbsoluteMm)) {
        return kInfinity;
    }
    const double lu = du.Magnitude();
    const double lv = dv.Magnitude();
    const double g = du.Crossed(dv).Magnitude();
    if (!std::isfinite(lu) || !std::isfinite(lv) || !std::isfinite(g)) return kInfinity;
    if (!(g > 0.0)) return kInfinity;
    const double eu = budget.uAbsoluteMm;
    const double ev = budget.vAbsoluteMm;
    const double perturbation = eu * lv + ev * lu + eu * ev;
    if (!std::isfinite(perturbation)) return kInfinity;
    const double ratio = perturbation / g;
    if (!(ratio < 1.0)) return kInfinity;  // the perturbation reaches the cross product
    return std::asin(ratio);
}

namespace {

// --- Astra §3: the face-degeneracy certificates -----------------------------

bool patch_is_rank_one(const NCollection_Array2<gp_Pnt>& poles, std::string& why) {
    const int nu = poles.UpperRow() - poles.LowerRow() + 1;
    const int nv = poles.UpperCol() - poles.LowerCol() + 1;
    gp_Vec basis(0.0, 0.0, 0.0);
    bool haveBasis = false;
    const auto consider = [&](const gp_Vec& d) {
        if (d.X() == 0.0 && d.Y() == 0.0 && d.Z() == 0.0) return true;
        if (!haveBasis) {
            basis = d;
            haveBasis = true;
            return true;
        }
        const gp_Vec c = d.Crossed(basis);
        return c.X() == 0.0 && c.Y() == 0.0 && c.Z() == 0.0;
    };
    for (int i = poles.LowerRow(); i <= poles.UpperRow(); ++i) {
        for (int j = poles.LowerCol(); j <= poles.UpperCol(); ++j) {
            if (i < poles.UpperRow() &&
                !consider(gp_Vec(poles(i, j), poles(i + 1, j)))) {
                return false;
            }
            if (j < poles.UpperCol() &&
                !consider(gp_Vec(poles(i, j), poles(i, j + 1)))) {
                return false;
            }
        }
    }
    why = haveBasis ? "every u- and v-difference pole of the " + std::to_string(nu) + "x" +
                          std::to_string(nv) +
                          " control net is parallel to one direction, so S_u x S_v is "
                          "identically zero (rank-one image)"
                    : "every pole of the " + std::to_string(nu) + "x" + std::to_string(nv) +
                          " control net is the same point (point image)";
    return true;
}

// Exactly-equal weights make a rational patch polynomial in effect, so the plain
// pole differences ARE the derivative control net. Varying weights are not
// handled: the derivative numerator is `A_u w - A w_u`, which the differences do
// not describe, so such a patch stays Unproved (Astra §3 "certify rational
// denominator validity").
template <typename Patch>
bool patch_weights_are_uniform(const Patch& patch) {
    if (!patch->IsURational() && !patch->IsVRational()) return true;
    const double w0 = patch->Weight(1, 1);
    if (!std::isfinite(w0) || !(w0 > 0.0)) return false;
    for (int i = 1; i <= patch->NbUPoles(); ++i) {
        for (int j = 1; j <= patch->NbVPoles(); ++j) {
            const double w = patch->Weight(i, j);
            if (!std::isfinite(w) || !(w > 0.0) || w != w0) return false;
        }
    }
    return true;
}

}  // namespace

FaceDegeneracy classify_face_degeneracy(const TopoDS_Face& face) {
    FaceDegeneracy out;
    std::string evidence;
    try {
        // All-degenerate boundary edges: EVIDENCE only. Astra F2 refutes it as a
        // certificate with `f = 16u(1-u)v(1-v)`, `S = ((2u-1)f, (2v-1)f, f)`,
        // whose whole boundary maps to one point while the centre Jacobian is
        // 4 mm^2 of genuine interior area.
        int wires = 0;
        for (TopExp_Explorer it(face, TopAbs_WIRE); it.More(); it.Next()) ++wires;
        bool sawEdge = false;
        bool allDegenerate = true;
        for (TopExp_Explorer it(face, TopAbs_EDGE); it.More(); it.Next()) {
            sawEdge = true;
            if (!BRep_Tool::Degenerated(TopoDS::Edge(it.Current()))) allDegenerate = false;
        }
        if (sawEdge && allDegenerate) {
            evidence = "; evidence only: every boundary edge is degenerate (not a certificate)";
        }

        if (wires == 0) {
            // Astra §3: "Missing/malformed trimming is uncertainty, not an
            // empty-domain certificate." A face with no wire retains its
            // surface's whole natural domain (BRepTools::UVBounds reports it),
            // so it is the canonical MISSING nondegenerate face, not a zero one.
            out.cls = DegeneracyClass::Unproved;
            out.reason = "the face carries no trim description at all" + evidence;
            return out;
        }

        // (a) zero-measure retained 2D domain, EXACT comparison. The retained
        // parameter set lies inside a line, so its continuous image has
        // dimension <= 1 and therefore zero area.
        double u0 = 0.0, u1 = 0.0, v0 = 0.0, v1 = 0.0;
        BRepTools::UVBounds(face, u0, u1, v0, v1);
        // `u1 >= u0 && v1 >= v0` rejects an INVERTED box, which is how a
        // trim description that produced no pcurve at all would present itself —
        // uncertainty, not an empty-domain certificate (Astra §3). A genuinely
        // zero-measure trim gives an exact equality in one direction.
        const bool boxIsValid =
            std::isfinite(u0) && std::isfinite(u1) && std::isfinite(v0) && std::isfinite(v1) &&
            u1 >= u0 && v1 >= v0;
        if (boxIsValid && (!(u1 > u0) || !(v1 > v0))) {
            out.cls = DegeneracyClass::CertifiedZero;
            out.reason = "the retained UV domain has zero measure: u [" + num_text(u0) + ", " +
                         num_text(u1) + "] v [" + num_text(v0) + ", " + num_text(v1) + "]" +
                         evidence;
            return out;
        }

        BRepAdaptor_Surface surf(face, /*Restriction=*/false);
        const GeomAbs_SurfaceType type = surf.GetType();

        // (b) an elementary surface whose OWN parametric domain has an exactly
        // zero extent: the surface image is a curve before any trimming.
        const bool elementary = type == GeomAbs_Plane || type == GeomAbs_Cylinder ||
                                type == GeomAbs_Cone || type == GeomAbs_Sphere ||
                                type == GeomAbs_Torus;
        if (elementary) {
            const double su0 = surf.FirstUParameter();
            const double su1 = surf.LastUParameter();
            const double sv0 = surf.FirstVParameter();
            const double sv1 = surf.LastVParameter();
            if (!(su1 > su0) || !(sv1 > sv0)) {
                out.cls = DegeneracyClass::CertifiedZero;
                out.reason =
                    "the elementary surface's own parametric domain has a zero extent: u [" +
                    num_text(su0) + ", " + num_text(su1) + "] v [" + num_text(sv0) + ", " +
                    num_text(sv1) + "]" + evidence;
                return out;
            }
        }

        // (c) rank-one derivative control net on a polynomial (or uniform-weight
        // rational) patch: S_u x S_v is identically zero over every retained span.
        std::string why;
        if (type == GeomAbs_BezierSurface) {
            const Handle(Geom_BezierSurface) patch = surf.Bezier();
            if (!patch.IsNull() && patch_weights_are_uniform(patch)) {
                if (patch_is_rank_one(patch->Poles(), why)) {
                    out.cls = DegeneracyClass::CertifiedZero;
                    out.reason = why + evidence;
                    return out;
                }
            }
        } else if (type == GeomAbs_BSplineSurface) {
            const Handle(Geom_BSplineSurface) patch = surf.BSpline();
            if (!patch.IsNull() && patch_weights_are_uniform(patch)) {
                if (patch_is_rank_one(patch->Poles(), why)) {
                    out.cls = DegeneracyClass::CertifiedZero;
                    out.reason = why + evidence;
                    return out;
                }
            }
        }

        // Not certified. A regular interior sample with a certified nonzero
        // Jacobian is the positive DISPROOF of degeneracy (Astra §4) and is worth
        // naming in the diagnostic; its absence proves nothing either way.
        out.cls = DegeneracyClass::Unproved;
        const DerivativeBudget budget = derivative_error_budget(surf);
        gp_Pnt at;
        gp_Vec du;
        gp_Vec dv;
        bool witness = false;
        double witnessArea = 0.0;
        try {
            surf.D1(0.5 * (u0 + u1), 0.5 * (v0 + v1), at, du, dv);
            witnessArea = du.Crossed(dv).Magnitude();
            const double eu = budget.uAbsoluteMm;
            const double ev = budget.vAbsoluteMm;
            witness = budget.known && std::isfinite(witnessArea) &&
                      witnessArea > eu * dv.Magnitude() + ev * du.Magnitude() + eu * ev;
        } catch (const Standard_Failure&) {
            witness = false;
        }
        out.reason = witness ? "interior Jacobian witness: |S_u x S_v| = " +
                                   num_text(witnessArea) +
                                   " mm^2 at the retained UV centroid proves positive area" +
                                   evidence
                             : std::string("no certificate applies to this face representation") +
                                   evidence;
        return out;
    } catch (const Standard_Failure& failure) {
        out.cls = DegeneracyClass::Unproved;
        out.reason = std::string("the face's representation could not be inspected: ") +
                     failure.GetMessageString();
        return out;
    } catch (const std::exception& error) {
        out.cls = DegeneracyClass::Unproved;
        out.reason =
            std::string("the face's representation could not be inspected: ") + error.what();
        return out;
    }
}

namespace {

// --- Astra F5: the symmetric triangle predicate -----------------------------

enum class TriangleClass : std::uint8_t {
    ExactZero,          // the cross product is exactly zero in binary64
    Usable,             // direction uncertainty under kTriangleDirectionAcceptanceRad
    UnresolvedNonzero   // nonzero, but no bound certifies its direction
};

// One triangle in EMITTED winding order, so `areaNormal` already points the way
// the rasterizer will see it (outward for a valid solid).
struct TriangleRecord {
    std::uint32_t node[3] = {0, 0, 0};
    gp_Vec areaNormal{0.0, 0.0, 0.0};
    gp_Vec unitNormal{0.0, 0.0, 1.0};
    TriangleClass cls = TriangleClass::ExactZero;
    bool hasDirection = false;
};

bool exactly_zero(const gp_Vec& v) { return v.X() == 0.0 && v.Y() == 0.0 && v.Z() == 0.0; }

bool lexicographically_greater(const gp_Vec& a, const gp_Vec& b) {
    if (a.X() != b.X()) return a.X() > b.X();
    if (a.Y() != b.Y()) return a.Y() > b.Y();
    return a.Z() > b.Z();
}

// Astra F5: the retired predicate took BOTH edges from the FIRST vertex, so the
// same triangle (0,0,0),(1,0,0),(2,1.5e-12,0) was rejected/accepted/rejected
// depending on which cyclic rotation the mesher happened to emit. The base
// vertex is now the one OPPOSITE the longest edge — a property of the geometry,
// not of the vertex order — which also maximises the included angle and so gives
// the best-conditioned cross product available. Exact length ties fall back to
// the larger, then lexicographically greater, cross product; both are functions
// of the point set alone and the winding is preserved by every rotation, so the
// classification is invariant under all three cyclic orders.
void classify_triangle(const gp_Pnt (&p)[3], TriangleRecord& rec) {
    double len[3];  // len[i] = |p[(i+1)%3] - p[i]|
    for (int i = 0; i < 3; ++i) len[i] = p[i].Distance(p[(i + 1) % 3]);

    // The base vertex is the one opposite the longest edge. Exact length ties
    // (an equilateral or isosceles facet) are broken by the larger, then the
    // lexicographically greater, cross product; every rotation of the same
    // triangle preserves both the length multiset and the winding, so all three
    // cyclic orders pick the same candidate.
    int base = 0;
    double bestOpposite = len[1];  // the edge not touching vertex 0
    int ties = 0;
    for (int k = 1; k < 3; ++k) {
        const double opposite = len[(k + 1) % 3];
        if (opposite > bestOpposite) {
            bestOpposite = opposite;
            base = k;
        }
    }
    for (int k = 0; k < 3; ++k) {
        if (len[(k + 1) % 3] == bestOpposite) ++ties;
    }
    gp_Vec bestCross = gp_Vec(p[base], p[(base + 1) % 3]).Crossed(gp_Vec(p[base], p[(base + 2) % 3]));
    double bestSpan = gp_Vec(p[base], p[(base + 1) % 3]).Magnitude() +
                      gp_Vec(p[base], p[(base + 2) % 3]).Magnitude();
    if (ties > 1) {
        // The full key is (|cross| desc, cross lexicographically desc, |e1|+|e2|
        // asc). All three components are functions of the point SET and of the
        // winding, both preserved by a cyclic rotation, and a candidate that ties
        // on all three yields an identical classification — so no comparison ever
        // falls through to iteration order.
        for (int k = 0; k < 3; ++k) {
            if (len[(k + 1) % 3] != bestOpposite || k == base) continue;
            const gp_Vec e1(p[k], p[(k + 1) % 3]);
            const gp_Vec e2(p[k], p[(k + 2) % 3]);
            const gp_Vec cross = e1.Crossed(e2);
            const double a = cross.Magnitude();
            const double b = bestCross.Magnitude();
            const double span = e1.Magnitude() + e2.Magnitude();
            const bool better =
                a > b ||
                (a == b && (lexicographically_greater(cross, bestCross) ||
                            (!lexicographically_greater(bestCross, cross) && span < bestSpan)));
            if (better) {
                base = k;
                bestCross = cross;
                bestSpan = span;
            }
        }
    }

    const gp_Vec e1(p[base], p[(base + 1) % 3]);
    const gp_Vec e2(p[base], p[(base + 2) % 3]);
    rec.areaNormal = bestCross;
    const double area2 = bestCross.Magnitude();
    rec.hasDirection = std::isfinite(area2) && area2 > 0.0;
    if (rec.hasDirection) rec.unitNormal = bestCross.Divided(area2);

    if (exactly_zero(bestCross)) {
        rec.cls = TriangleClass::ExactZero;
        return;
    }
    if (!rec.hasDirection) {
        rec.cls = TriangleClass::UnresolvedNonzero;  // non-finite coordinates
        return;
    }
    // Position roundoff: each stored coordinate carries at most half an ulp of
    // the triangle's OWN largest magnitude, so an edge (a difference of two
    // stored points) carries at most twice that per coordinate triple. The
    // first-order direction uncertainty of e1 x e2 is then
    // `eEdge * (|e1| + |e2|) / |e1 x e2|` — Astra F5's `eD / (L sin(theta))`
    // with both edges accounted rather than the shorter one alone.
    double maxAbs = 0.0;
    for (int i = 0; i < 3; ++i) {
        maxAbs = std::max(maxAbs, std::abs(p[i].X()));
        maxAbs = std::max(maxAbs, std::abs(p[i].Y()));
        maxAbs = std::max(maxAbs, std::abs(p[i].Z()));
    }
    const double eEdge = 2.0 * kSqrt3 * kUnitRoundoff * maxAbs;
    const double angular = eEdge * (e1.Magnitude() + e2.Magnitude()) / area2;
    rec.cls = std::isfinite(angular) && angular <= kTriangleDirectionAcceptanceRad
                  ? TriangleClass::Usable
                  : TriangleClass::UnresolvedNonzero;
}

std::vector<TriangleRecord> build_triangles(const Handle(Poly_Triangulation)& tri,
                                            const std::vector<gp_Pnt>& nodes,
                                            bool reverseWinding) {
    std::vector<TriangleRecord> out;
    out.reserve(static_cast<std::size_t>(tri->NbTriangles()));
    for (int t = 1; t <= tri->NbTriangles(); ++t) {
        Standard_Integer n1 = 0, n2 = 0, n3 = 0;
        tri->Triangle(t).Get(n1, n2, n3);
        if (reverseWinding) std::swap(n2, n3);
        TriangleRecord rec;
        rec.node[0] = static_cast<std::uint32_t>(n1 - 1);
        rec.node[1] = static_cast<std::uint32_t>(n2 - 1);
        rec.node[2] = static_cast<std::uint32_t>(n3 - 1);
        const gp_Pnt p[3] = {nodes[rec.node[0]], nodes[rec.node[1]], nodes[rec.node[2]]};
        classify_triangle(p, rec);
        out.push_back(rec);
    }
    return out;
}

// Astra §4's last class. The EMITTED positions are float32; a triangle with
// positive binary64 area whose float32 image collapses to exactly zero is a
// REPRESENTATION failure that must be recorded, never a degeneracy exemption.
// `ulp32(8192 mm) = 2^-10 mm`, so a 0.0001 x 0.01 mm rectangle at X = 8192 mm
// collapses while its BRep is perfectly valid. The rounding is done once per
// NODE (the caller emits one float32 triple per node) and reused per triangle.
gp_Pnt round_to_float32(const gp_Pnt& p) {
    return gp_Pnt(static_cast<double>(static_cast<float>(p.X())),
                  static_cast<double>(static_cast<float>(p.Y())),
                  static_cast<double>(static_cast<float>(p.Z())));
}

struct SurfaceSample {
    gp_Vec direction{0.0, 0.0, 1.0};
    double angularErrorRad = kInfinity;
    bool hasDirection = false;   // finite, nonzero cross product
    bool certified = false;      // hasDirection AND angularErrorRad <= kNormalAcceptanceRad
    bool sphereSingular = false; // the sphere's pole locus: answer it analytically
    bool coneApex = false;       // the cone's apex locus: split, never average (F8)
    double crossMagnitude = 0.0;
};

// NUM §7.1 for a regular node. The `orientedSign` argument is
// `orientationSign * sign(det A)`; see the header for why the determinant belongs
// on the cross product exactly once.
SurfaceSample surface_normal_at(const BRepAdaptor_Surface& surf, GeomAbs_SurfaceType type,
                                const DerivativeBudget& budget, double u, double v) {
    SurfaceSample out;
    gp_Pnt at;
    gp_Vec du;
    gp_Vec dv;
    surf.D1(u, v, at, du, dv);
    const double lu = du.Magnitude();
    const double lv = dv.Magnitude();
    const gp_Vec cross = du.Crossed(dv);
    const double g = cross.Magnitude();
    if (!std::isfinite(lu) || !std::isfinite(lv) || !std::isfinite(g)) return out;
    out.crossMagnitude = g;
    if (g > 0.0) {
        out.direction = cross.Divided(g);
        out.hasDirection = true;
    }
    out.angularErrorRad = normal_angular_error_rad(budget, du, dv);
    out.certified = out.hasDirection && out.angularErrorRad <= kNormalAcceptanceRad;
    if (out.certified) return out;

    // Astra F8: the cone apex is a KNOWN nonsmooth singularity, and |S_u| there
    // is exactly `R + v*sin(a)` = 0. NUM §7.3 requires a singular SPLIT at it, so
    // it must be recognised BEFORE the general 5 degree averaging rule can invent
    // an axial normal (a 10/0/20 mm cone's -2/0/+2 degree apex fan spreads
    // 1.7889 degrees; a shallow 72-sector cone spreads 4.0038 degrees, and both
    // used to be averaged).
    if (type == GeomAbs_Cone && budget.known && !(lu > budget.uAbsoluteMm)) {
        out.coneApex = true;
        return out;
    }
    // NUM §7.3: "Sphere pole: use the normalized vector from analytic sphere
    // center to point". The SIGN is settled in a second pass from a regular
    // witness of the same face, never from the incident facets (Astra F7).
    if (type == GeomAbs_Sphere) out.sphereSingular = true;
    return out;
}

// Greatest pairwise angle among unit vectors, in radians (NUM §7.3's "maximum
// angular spread"). Only ever called with at most kMaxSpreadValence vectors.
double max_spread_rad(const std::vector<gp_Vec>& units) {
    double worst = 0.0;
    for (std::size_t i = 0; i < units.size(); ++i) {
        for (std::size_t j = i + 1; j < units.size(); ++j) {
            const double d = std::clamp(units[i].Dot(units[j]), -1.0, 1.0);
            worst = std::max(worst, std::acos(d));
        }
    }
    return worst;
}

// Node -> incident triangles that carry a DIRECTION, in compressed row form. A
// vector-per-node map is the same information, but 21k heap allocations on a
// torus face is most of the cost of the whole pass; this is two allocations.
struct Incidence {
    std::vector<std::uint32_t> offsets;  // nbNodes + 1
    std::vector<std::uint32_t> items;    // triangle indices, ascending within a node
};

Incidence build_incidence(const std::vector<TriangleRecord>& triangles, int nbNodes) {
    Incidence inc;
    inc.offsets.assign(static_cast<std::size_t>(nbNodes) + 1U, 0U);
    for (const TriangleRecord& rec : triangles) {
        if (!rec.hasDirection) continue;
        for (const std::uint32_t n : rec.node) ++inc.offsets[static_cast<std::size_t>(n) + 1U];
    }
    for (std::size_t i = 1; i < inc.offsets.size(); ++i) inc.offsets[i] += inc.offsets[i - 1];
    inc.items.resize(inc.offsets.back());
    std::vector<std::uint32_t> cursor(inc.offsets.begin(), inc.offsets.end() - 1);
    for (std::size_t t = 0; t < triangles.size(); ++t) {
        if (!triangles[t].hasDirection) continue;
        for (const std::uint32_t n : triangles[t].node) {
            inc.items[cursor[n]++] = static_cast<std::uint32_t>(t);
        }
    }
    return inc;
}

struct NodeDecision {
    gp_Vec normal{0.0, 0.0, 1.0};
    NormalProvenance provenance = NormalProvenance::Missing;
    bool split = false;
};

// NUM §7.3's general singular rule, and the only path available when the
// triangulation carries no UV nodes at all. Astra F5/F9: only triangles whose
// direction is CERTIFIED feed the fallback; an unresolved one can still keep a
// node off `Missing`, but it is labelled `Unresolved` rather than passed off as a
// bounded answer.
NodeDecision decide_from_triangles(const std::vector<TriangleRecord>& triangles,
                                   const std::uint32_t* incident, std::size_t count) {
    NodeDecision out;
    std::vector<gp_Vec> units;
    units.reserve(std::min(count, kMaxSpreadValence + 1U));
    gp_Vec weighted(0.0, 0.0, 0.0);
    const TriangleRecord* firstUnresolved = nullptr;
    std::size_t usable = 0;
    for (std::size_t k = 0; k < count; ++k) {
        const TriangleRecord& rec = triangles[incident[k]];
        if (!rec.hasDirection) continue;
        if (rec.cls == TriangleClass::Usable) {
            ++usable;
            if (units.size() <= kMaxSpreadValence) units.push_back(rec.unitNormal);
            weighted += rec.areaNormal;
        } else if (firstUnresolved == nullptr) {
            firstUnresolved = &rec;
        }
    }
    if (usable == 0) {
        if (firstUnresolved != nullptr) {
            out.normal = firstUnresolved->unitNormal;
            out.provenance = NormalProvenance::Unresolved;
        }
        return out;  // Missing when nothing incident carries a direction at all
    }
    // Astra F11: the pairwise spread is k(k-1)/2, so a valence-10 000 collapsed
    // spline row would cost 49 995 000 comparisons. Above kMaxSpreadValence the
    // node is split deterministically without measuring — the answer a fan that
    // wide reaches in practice anyway, at O(k).
    if (usable == 1 ||
        (usable <= kMaxSpreadValence && max_spread_rad(units) <= kSingularSpreadRad)) {
        const double len = weighted.Magnitude();
        if (len > 0.0) {
            out.normal = weighted.Divided(len);
            out.provenance = NormalProvenance::TriangulationFallback;
            return out;
        }
    }
    out.split = true;
    out.provenance = NormalProvenance::SingularSplit;
    return out;
}

}  // namespace

FaceNormalResult compute_face_normals(const TopoDS_Face& face,
                                      const Handle(Poly_Triangulation)& tri,
                                      const TopLoc_Location& loc, bool reversed) {
    FaceNormalResult out;
    if (tri.IsNull() || tri->NbNodes() < 3 || tri->NbTriangles() < 1) {
        out.diagnostic = "no triangulation";
        return out;
    }

    const gp_Trsf trsf = loc.Transformation();
    // NUM §7.2, applied to the location that is ACTUALLY still on the shape.
    // `gp_Trsf::IsNegative()` is NOT this predicate: it reports `scale < 0`, which
    // is neither necessary nor sufficient for a negative determinant.
    const bool detNegative = trsf.VectorialPart().Determinant() < 0.0;
    out.reverseWinding = (reversed != detNegative);
    const double orientationSign = reversed ? -1.0 : 1.0;
    const double orientedSign = orientationSign * (detNegative ? -1.0 : 1.0);

    const int nbNodes = tri->NbNodes();
    out.worldNodes.resize(static_cast<std::size_t>(nbNodes));
    for (int i = 1; i <= nbNodes; ++i) {
        out.worldNodes[static_cast<std::size_t>(i - 1)] = tri->Node(i).Transformed(trsf);
    }
    const std::vector<TriangleRecord> triangles =
        build_triangles(tri, out.worldNodes, out.reverseWinding);

    for (const TriangleRecord& rec : triangles) {
        if (rec.cls == TriangleClass::ExactZero) ++out.droppedTriangles;
    }
    const Incidence incident = build_incidence(triangles, nbNodes);
    const auto incident_begin = [&](std::size_t n) {
        return incident.items.data() + incident.offsets[n];
    };
    const auto incident_count = [&](std::size_t n) {
        return static_cast<std::size_t>(incident.offsets[n + 1] - incident.offsets[n]);
    };

    std::vector<NodeDecision> decisions(static_cast<std::size_t>(nbNodes));
    const bool haveUv = tri->HasUVNodes();
    bool surfaceReady = false;
    BRepAdaptor_Surface surf;
    GeomAbs_SurfaceType type = GeomAbs_OtherSurface;
    try {
        surf.Initialize(face, /*Restriction=*/false);
        type = surf.GetType();
        surfaceReady = true;
    } catch (const Standard_Failure& failure) {
        out.diagnostic = std::string("supporting surface unavailable: ") + failure.GetMessageString();
    }
    if (!haveUv && out.diagnostic.empty()) {
        out.diagnostic = "the triangulation carries no UV nodes";
    }
    const DerivativeBudget budget =
        surfaceReady ? derivative_error_budget(surf) : DerivativeBudget{};
    if (surfaceReady && !budget.known && out.diagnostic.empty()) {
        out.diagnostic = std::string("derivative budget unavailable: ") + budget.reason;
    }

    // A plane's derivatives are constant by definition, so one D1 answers every
    // node exactly (NUM §7.3's "Plane: use oriented analytic plane normal"). This
    // is the prismatic-model fast path, not an approximation.
    SurfaceSample planar;
    if (surfaceReady && haveUv && type == GeomAbs_Plane) {
        try {
            const gp_Pnt2d uv = tri->UVNode(1);
            planar = surface_normal_at(surf, type, budget, uv.X(), uv.Y());
        } catch (const Standard_Failure&) {
            planar = SurfaceSample{};
        }
    }

    // Astra F7: the sphere-pole sign is decided ONCE per face, from the most
    // regular certified node of that same face, and cached. Reversing or removing
    // the pole's incident facets must not move it by 180 degrees.
    std::vector<char> pendingSphere(static_cast<std::size_t>(nbNodes), 0);
    bool anyPendingSphere = false;
    gp_Vec witnessNormal(0.0, 0.0, 1.0);
    gp_Pnt witnessPoint;
    double witnessCross = 0.0;
    bool haveWitness = false;

    for (int i = 0; i < nbNodes; ++i) {
        const std::size_t n = static_cast<std::size_t>(i);
        SurfaceSample sample = planar;
        if (surfaceReady && haveUv && !planar.certified) {
            try {
                const gp_Pnt2d uv = tri->UVNode(i + 1);
                sample = surface_normal_at(surf, type, budget, uv.X(), uv.Y());
            } catch (const Standard_Failure&) {
                sample = SurfaceSample{};
            }
        }
        if (sample.certified) {
            decisions[n].normal = sample.direction.Multiplied(orientedSign);
            decisions[n].provenance = NormalProvenance::Surface;
            if (!haveWitness || sample.crossMagnitude > witnessCross) {
                haveWitness = true;
                witnessCross = sample.crossMagnitude;
                witnessNormal = decisions[n].normal;
                witnessPoint = out.worldNodes[n];
            }
            continue;
        }
        if (sample.coneApex) {
            decisions[n].split = true;
            decisions[n].provenance = NormalProvenance::SingularSplit;
            continue;
        }
        if (sample.sphereSingular) {
            pendingSphere[n] = 1;
            anyPendingSphere = true;
            continue;
        }
        if (!budget.known && sample.hasDirection) {
            // R1(c) BLOCKER 2. No enclosure exists for this representation — a
            // surface of extrusion or revolution, an offset surface — so the
            // node cannot be CERTIFIED. That is not evidence against its own
            // S_u x S_v, and the facet ladder is a strictly worse estimator of
            // it: routing these nodes there split 1 835 of a spline prism's
            // 1 582 nodes and 10 917 of a revolve's 2 702, inflating the emitted
            // vertex count 2.0x and 4.4x for geometry with no singularity in it
            // at all. Keep the surface's own answer and label it `Unresolved`.
            // A budget that EXISTS and refuses the node is a different case: the
            // direction is then known to be untrustworthy, so it still falls to
            // the ladder below.
            decisions[n].normal = sample.direction.Multiplied(orientedSign);
            decisions[n].provenance = NormalProvenance::Unresolved;
            continue;
        }
        decisions[n] = decide_from_triangles(triangles, incident_begin(n), incident_count(n));
        if (decisions[n].provenance == NormalProvenance::Missing && sample.hasDirection) {
            // The surface DID answer; only its error bound failed (Astra F3/F4).
            // That is `Unresolved`, never `Missing`.
            decisions[n].normal = sample.direction.Multiplied(orientedSign);
            decisions[n].provenance = NormalProvenance::Unresolved;
        }
    }

    if (anyPendingSphere) {
        // sign(dot(radial, n)) at ONE certified regular point of this face is the
        // face's radial convention, including an indirect gp_Ax3 and any reflected
        // placement. The facets may only diagnose a disagreement.
        double radialSign = 0.0;
        if (haveWitness) {
            const gp_Vec radial(surf.Sphere().Location(), witnessPoint);
            const double dot = radial.Dot(witnessNormal);
            if (std::isfinite(dot) && dot != 0.0) radialSign = dot > 0.0 ? 1.0 : -1.0;
        }
        for (int i = 0; i < nbNodes; ++i) {
            const std::size_t n = static_cast<std::size_t>(i);
            if (pendingSphere[n] == 0) continue;
            bool answered = false;
            if (radialSign != 0.0) {
                const gp_Vec radial(surf.Sphere().Location(), out.worldNodes[n]);
                const double r = radial.Magnitude();
                if (std::isfinite(r) && r > 0.0) {
                    decisions[n].normal = radial.Multiplied(radialSign / r);
                    decisions[n].provenance = NormalProvenance::AnalyticSingular;
                    answered = true;
                    gp_Vec facets(0.0, 0.0, 0.0);
                    for (std::size_t k = 0; k < incident_count(n); ++k) {
                        facets += triangles[incident_begin(n)[k]].areaNormal;
                    }
                    if (facets.Magnitude() > 0.0 && facets.Dot(decisions[n].normal) < 0.0) {
                        WLOG_DEBUG("normals: sphere pole node %d disagrees with its incident "
                                   "facets; keeping the analytic radial sign",
                                   i);
                    }
                }
            }
            if (!answered) {
                decisions[n] =
                    decide_from_triangles(triangles, incident_begin(n), incident_count(n));
            }
        }
    }

    // --- emission ----------------------------------------------------------
    // Pass 1 gives every node that HAS a normal its own vertex, in node order, so
    // a caller's face-local vertex block stays contiguous. Astra F10: a `Missing`
    // node gets NO slot — the retired code serialised world +Z for it.
    out.nodeRemap.assign(static_cast<std::size_t>(nbNodes), FaceNormalResult::kNoVertex);
    out.vertexNode.reserve(static_cast<std::size_t>(nbNodes));
    out.provenance.reserve(static_cast<std::size_t>(nbNodes));
    out.normals.reserve(static_cast<std::size_t>(nbNodes) * 3U);
    out.triangleVertexIndices.reserve(triangles.size() * 3U);
    // Per triangle CORNER, the split copy that corner must use; kNoCopy keeps the
    // node's base vertex. A triangle can have more than one split corner, so this
    // is written per corner and never rewritten wholesale.
    constexpr std::uint32_t kNoCopy = 0xFFFFFFFFU;
    std::vector<std::uint32_t> copyOfTriangle(triangles.size() * 3U, kNoCopy);
    const auto first_directed = [&](std::size_t n) -> const TriangleRecord* {
        for (std::size_t k = 0; k < incident_count(n); ++k) {
            const TriangleRecord& rec = triangles[incident_begin(n)[k]];
            if (rec.hasDirection) return &rec;
        }
        return nullptr;
    };
    for (int i = 0; i < nbNodes; ++i) {
        const std::size_t n = static_cast<std::size_t>(i);
        gp_Vec normal = decisions[n].normal;
        NormalProvenance prov = decisions[n].provenance;
        if (prov == NormalProvenance::Missing) {
            // NUM §7.3: "do not emit an arbitrary normal." The node is not
            // emitted at all, so no +Z reaches the wire; a triangle that still
            // references it fails the face below.
            ++out.missingCount;
            continue;
        }
        if (decisions[n].split) {
            const TriangleRecord* first = first_directed(n);
            if (first == nullptr) {
                ++out.missingCount;
                continue;
            }
            normal = first->unitNormal;
            ++out.splitCount;
        } else if (prov == NormalProvenance::TriangulationFallback) {
            ++out.fallbackCount;
        } else if (prov == NormalProvenance::Unresolved) {
            ++out.unresolvedCount;
        }
        out.nodeRemap[n] = static_cast<std::uint32_t>(out.vertexNode.size());
        out.vertexNode.push_back(static_cast<std::uint32_t>(i));
        out.provenance.push_back(prov);
        out.normals.push_back(static_cast<float>(normal.X()));
        out.normals.push_back(static_cast<float>(normal.Y()));
        out.normals.push_back(static_cast<float>(normal.Z()));
    }
    // Pass 2 appends one extra vertex per additional incident triangle of a split
    // node, in node order then triangle order. Deterministic, and it touches only
    // vertex indices — never the triangle order or the face's triangle count.
    for (int i = 0; i < nbNodes; ++i) {
        const std::size_t n = static_cast<std::size_t>(i);
        if (!decisions[n].split || out.nodeRemap[n] == FaceNormalResult::kNoVertex) continue;
        bool first = true;
        for (std::size_t k = 0; k < incident_count(n); ++k) {
            const std::uint32_t t = incident_begin(n)[k];
            if (!triangles[t].hasDirection) continue;
            if (first) {
                first = false;  // the node's own vertex already carries this normal
                continue;
            }
            const gp_Vec& unit = triangles[t].unitNormal;
            const std::uint32_t vertex = static_cast<std::uint32_t>(out.vertexNode.size());
            out.vertexNode.push_back(static_cast<std::uint32_t>(i));
            out.provenance.push_back(NormalProvenance::SingularSplit);
            out.normals.push_back(static_cast<float>(unit.X()));
            out.normals.push_back(static_cast<float>(unit.Y()));
            out.normals.push_back(static_cast<float>(unit.Z()));
            for (int corner = 0; corner < 3; ++corner) {
                if (triangles[t].node[corner] == static_cast<std::uint32_t>(i)) {
                    copyOfTriangle[static_cast<std::size_t>(t) * 3U +
                                   static_cast<std::size_t>(corner)] = vertex;
                }
            }
        }
    }
    // Astra F5/F9: ONLY a certified-zero triangle is dropped. A triangle whose
    // direction could not be resolved keeps its coverage — deleting it is exactly
    // the hole the retired floors punched while `complete` stayed true.
    std::uint32_t referencedMissing = 0;
    std::vector<gp_Pnt> emittedNodes(out.worldNodes.size());
    for (std::size_t i = 0; i < out.worldNodes.size(); ++i) {
        emittedNodes[i] = round_to_float32(out.worldNodes[i]);
    }
    for (std::size_t t = 0; t < triangles.size(); ++t) {
        if (triangles[t].cls == TriangleClass::ExactZero) continue;
        bool emittable = true;
        for (int corner = 0; corner < 3; ++corner) {
            if (out.nodeRemap[triangles[t].node[corner]] == FaceNormalResult::kNoVertex) {
                emittable = false;
            }
        }
        if (!emittable) {
            ++referencedMissing;
            continue;
        }
        const gp_Pnt& q0 = emittedNodes[triangles[t].node[0]];
        const gp_Pnt& q1 = emittedNodes[triangles[t].node[1]];
        const gp_Pnt& q2 = emittedNodes[triangles[t].node[2]];
        if (exactly_zero(gp_Vec(q0, q1).Crossed(gp_Vec(q0, q2)))) ++out.float32CollapsedTriangles;
        for (int corner = 0; corner < 3; ++corner) {
            const std::uint32_t copy = copyOfTriangle[t * 3U + static_cast<std::size_t>(corner)];
            out.triangleVertexIndices.push_back(
                copy != kNoCopy ? copy : out.nodeRemap[triangles[t].node[corner]]);
        }
    }

    out.complete = !out.triangleVertexIndices.empty() && referencedMissing == 0;
    const auto note = [&out](const std::string& text) {
        out.diagnostic += (out.diagnostic.empty() ? "" : "; ") + text;
    };
    if (out.droppedTriangles > 0) {
        note("dropped " + std::to_string(out.droppedTriangles) +
             " triangles with an exactly zero cross product");
    }
    if (out.missingCount > 0) {
        note(std::to_string(out.missingCount) + " nodes have no normal source");
    }
    if (referencedMissing > 0) {
        note(std::to_string(referencedMissing) +
             " surviving triangles reference a node with no normal source");
    }
    if (out.unresolvedCount > 0) {
        note(std::to_string(out.unresolvedCount) +
             " nodes carry a normal no error bound certifies");
    }
    if (out.float32CollapsedTriangles > 0) {
        note(std::to_string(out.float32CollapsedTriangles) +
             " triangles with positive area collapse to zero in the emitted float32 positions");
    }
    return out;
}

}  // namespace onecad::tess
