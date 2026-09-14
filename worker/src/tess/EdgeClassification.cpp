// EdgeClassification.cpp — see EdgeClassification.h.
#include "tess/EdgeClassification.h"

#include <algorithm>
#include <cmath>
#include <cstddef>

#include <BRepAdaptor_Surface.hxx>
#include <BRep_Tool.hxx>
#include <Geom2d_Curve.hxx>
#include <GeomAbs_Shape.hxx>
#include <Geom_Surface.hxx>
#include <Standard_Failure.hxx>
#include <TopAbs_Orientation.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopLoc_Location.hxx>
#include <TopTools_IndexedDataMapOfShapeListOfShape.hxx>
#include <TopTools_ListOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Face.hxx>
#include <gp_Mat.hxx>
#include <gp_Pnt2d.hxx>
#include <gp_Trsf.hxx>
#include <gp_Vec.hxx>

#include "tess/SurfaceNormals.h"

namespace onecad::tess {

namespace {

struct OrientedNormal {
    gp_Vec direction{0.0, 0.0, 1.0};
    bool valid = false;
};

// The face's OUTWARD normal at the edge parameter `t`, under the same contract
// the shading normals use (SurfaceNormals.h): the adaptor applies the face
// location once, the face orientation and the location's determinant sign are
// applied once each.
//
// Astra F6 adds the acceptance half of NUM §7.1 to it: a normal whose DIRECTION
// carries no error bound (`normal_angular_error_rad` above kNormalAcceptanceRad)
// is not evidence of anything, so the sample is reported UNEVALUABLE rather than
// folded into a dihedral the rest of the ladder then trusts.
OrientedNormal face_normal_at(const TopoDS_Face& face, const BRepAdaptor_Surface& surf,
                              const DerivativeBudget& budget, const TopoDS_Edge& edge, double t) {
    OrientedNormal out;
    try {
        Standard_Real first = 0.0, last = 0.0;
        const Handle(Geom2d_Curve) pcurve = BRep_Tool::CurveOnSurface(edge, face, first, last);
        if (pcurve.IsNull() || !(last > first)) return out;
        const double at = std::clamp(t, first, last);
        const gp_Pnt2d uv = pcurve->Value(at);
        gp_Pnt p;
        gp_Vec du;
        gp_Vec dv;
        surf.D1(uv.X(), uv.Y(), p, du, dv);
        const gp_Vec cross = du.Crossed(dv);
        const double g = cross.Magnitude();
        if (!std::isfinite(g) || g <= 0.0) return out;
        if (!(normal_angular_error_rad(budget, du, dv) <= kNormalAcceptanceRad)) return out;
        double sign = face.Orientation() == TopAbs_REVERSED ? -1.0 : 1.0;
        if (face.Location().Transformation().VectorialPart().Determinant() < 0.0) sign = -sign;
        out.direction = cross.Multiplied(sign / g);
        out.valid = true;
    } catch (const Standard_Failure&) {
        out.valid = false;
    }
    return out;
}

// NUM §8.2's "analytic supporting-surface equivalence", narrowed by Astra F6 to
// the only form that establishes continuity over the COMPLETE edge interval: the
// two faces are carried by the SAME `Geom_Surface` object at the SAME location.
//
// The retired version also accepted tolerance-close analytic PARAMETERS — equal
// radii and coincident centres within `max(tol(f1), tol(f2))`. Two spheres of
// radius 0.1 mm whose centres are 0.0001 mm apart, both with tolerance
// 0.0001 mm, pass that test while genuinely intersecting at a dihedral of
// `2 asin(0.0001 / 0.2) = 0.0573 degrees` — under the 0.2 degree screen, so every
// sample agreed and two distinct surfaces were declared tangent. Nearly-equal
// parameters describe nearly-equal surfaces, which is not the same statement.
bool same_supporting_surface(const TopoDS_Face& fa, const TopoDS_Face& fb) {
    TopLoc_Location la;
    TopLoc_Location lb;
    const Handle(Geom_Surface) sa = BRep_Tool::Surface(fa, la);
    const Handle(Geom_Surface) sb = BRep_Tool::Surface(fb, lb);
    if (sa.IsNull() || sb.IsNull()) return false;
    return sa == sb && la.IsEqual(lb);
}

// The continuity/measurement ladder for an edge with exactly two incident faces.
std::uint32_t classify_two_face_edge(const TopoDS_Edge& edge, const TopoDS_Face& f1,
                                     const TopoDS_Face& f2) {
    GeomAbs_Shape continuity = GeomAbs_C0;
    try {
        continuity = BRep_Tool::Continuity(edge, f1, f2);
    } catch (const Standard_Failure&) {
        continuity = GeomAbs_C0;
    }
    // GeomAbs_C0 is what OCCT returns when NO continuity is registered, so it is
    // the absence of metadata, never proof of a discontinuity.
    const bool claimsG1 = continuity >= GeomAbs_G1;
    return decide_edge_continuity(claimsG1, same_supporting_surface(f1, f2),
                                  sample_edge_agreement(f1, f2, edge));
}

bool is_seam_of(const TopoDS_Edge& edge, const TopoDS_Face& face) {
    try {
        return BRep_Tool::IsClosed(edge, face);
    } catch (const Standard_Failure&) {
        return false;
    }
}

}  // namespace

EdgeSampleAgreement sample_edge_agreement(const TopoDS_Face& f1, const TopoDS_Face& f2,
                                          const TopoDS_Edge& edge) {
    EdgeSampleAgreement out;
    Standard_Real first = 0.0, last = 0.0;
    BRep_Tool::Range(edge, first, last);
    if (!(last > first) || !std::isfinite(first) || !std::isfinite(last)) return out;
    BRepAdaptor_Surface sa;
    BRepAdaptor_Surface sb;
    try {
        sa.Initialize(f1, /*Restriction=*/false);
        sb.Initialize(f2, /*Restriction=*/false);
    } catch (const Standard_Failure&) {
        return out;
    }
    const DerivativeBudget ba = derivative_error_budget(sa);
    const DerivativeBudget bb = derivative_error_budget(sb);
    for (int k = 1; k <= kEdgeContinuitySamples; ++k) {
        const double u = static_cast<double>(k) / static_cast<double>(kEdgeContinuitySamples + 1);
        const double t = first + u * (last - first);
        const OrientedNormal a = face_normal_at(f1, sa, ba, edge, t);
        const OrientedNormal b = face_normal_at(f2, sb, bb, edge, t);
        if (!a.valid || !b.valid) continue;
        ++out.evaluable;
        const double d = std::clamp(a.direction.Dot(b.direction), -1.0, 1.0);
        out.maxDihedralRad = std::max(out.maxDihedralRad, std::acos(d));
    }
    return out;
}

std::uint32_t decide_edge_continuity(bool trustedG1, bool sameSupportingSurface,
                                     const EdgeSampleAgreement& agreement) {
    // A measured crease is definitive evidence of a discontinuity at that
    // parameter, whether or not the other samples could be evaluated.
    if (agreement.evaluable > 0 && agreement.maxDihedralRad > kEdgeCreaseRad) return kEdgeHard;
    // NUM §8.2: sampling is a DIAGNOSTIC SCREEN, never permission to hide an
    // edge, so a partially screened join cannot be upgraded even with trusted
    // metadata — "partial sampling cannot certify consistency" (Astra F6).
    if (!agreement.complete()) return kEdgeUnknown;
    if (trustedG1 || sameSupportingSurface) return kEdgeTangent;
    return kEdgeUnknown;
}

std::vector<EdgeClass> classify_edges(const TopoDS_Shape& shape,
                                      const TopTools_IndexedMapOfShape& faces,
                                      const TopTools_IndexedMapOfShape& edges) {
    std::vector<EdgeClass> out(static_cast<std::size_t>(edges.Extent()));
    if (shape.IsNull()) return out;

    TopTools_IndexedDataMapOfShapeListOfShape edgeFaces;
    TopExp::MapShapesAndAncestors(shape, TopAbs_EDGE, TopAbs_FACE, edgeFaces);

    for (int ei = 1; ei <= edges.Extent(); ++ei) {
        EdgeClass& cls = out[static_cast<std::size_t>(ei - 1)];
        const TopoDS_Edge edge = TopoDS::Edge(edges(ei));

        // Ancestors come back once per USE, so a seam names its face twice. The
        // published list is the sorted unique set (NUM §9.2).
        std::vector<TopoDS_Face> distinct;
        if (edgeFaces.Contains(edge)) {
            for (const TopoDS_Shape& s : edgeFaces.FindFromKey(edge)) {
                const int ordinal = faces.FindIndex(s);
                if (ordinal <= 0) continue;
                const std::uint32_t local = static_cast<std::uint32_t>(ordinal - 1);
                if (std::find(cls.faceOrdinals.begin(), cls.faceOrdinals.end(), local) !=
                    cls.faceOrdinals.end()) {
                    continue;
                }
                cls.faceOrdinals.push_back(local);
                distinct.push_back(TopoDS::Face(s));
            }
        }
        std::vector<std::uint32_t> sorted = cls.faceOrdinals;
        std::sort(sorted.begin(), sorted.end());
        cls.faceOrdinals = sorted;

        if (BRep_Tool::Degenerated(edge)) {
            // Semantically present, no reliable drawable segment. Definitive, so
            // no continuity bit and no `unknown` is added on top of it.
            cls.flags = kEdgeDegenerate;
            continue;
        }
        bool seam = false;
        for (const TopoDS_Face& f : distinct) seam = seam || is_seam_of(edge, f);
        if (seam) {
            cls.flags = kEdgeSeam;
            if (distinct.size() > 2) cls.flags |= kEdgeNonmanifold;
            continue;
        }
        if (distinct.size() > 2) {
            cls.flags = kEdgeNonmanifold;
            continue;
        }
        if (distinct.size() < 2) {
            // One incident face is an open boundary; a free wire edge bounds no
            // surface at all, which is the same display answer — always visible.
            cls.flags = kEdgeOpen;
            continue;
        }
        cls.flags = classify_two_face_edge(edge, distinct[0], distinct[1]);
    }
    return out;
}

std::vector<std::uint32_t> face_solid_ordinals(const TopoDS_Shape& shape,
                                               const TopTools_IndexedMapOfShape& faces) {
    std::vector<std::uint32_t> out(static_cast<std::size_t>(faces.Extent()), kNoSolidOrdinal);
    if (shape.IsNull()) return out;

    TopTools_IndexedMapOfShape solids;
    TopExp::MapShapes(shape, TopAbs_SOLID, solids);
    if (solids.IsEmpty()) return out;

    // `BRep_Tool::IsClosed` only answers the free-boundary question for a SHELL;
    // for a SOLID it merely returns the `Closed()` FLAG, which `BRepPrimAPI` does
    // not set on a perfectly closed box. Verify the shells instead, which is the
    // measurement rather than a stored claim.
    std::vector<bool> verified(static_cast<std::size_t>(solids.Extent()), false);
    for (int si = 1; si <= solids.Extent(); ++si) {
        bool sawShell = false;
        bool closed = true;
        try {
            for (TopExp_Explorer it(solids(si), TopAbs_SHELL); it.More(); it.Next()) {
                sawShell = true;
                if (!BRep_Tool::IsClosed(it.Current())) closed = false;
            }
        } catch (const Standard_Failure&) {
            closed = false;
        }
        verified[static_cast<std::size_t>(si - 1)] = sawShell && closed;
    }

    TopTools_IndexedDataMapOfShapeListOfShape faceSolids;
    TopExp::MapShapesAndAncestors(shape, TopAbs_FACE, TopAbs_SOLID, faceSolids);
    for (int fi = 1; fi <= faces.Extent(); ++fi) {
        const TopoDS_Shape& face = faces(fi);
        if (!faceSolids.Contains(face)) continue;
        std::vector<int> owners;
        for (const TopoDS_Shape& s : faceSolids.FindFromKey(face)) {
            const int ordinal = solids.FindIndex(s);
            if (ordinal > 0 && std::find(owners.begin(), owners.end(), ordinal) == owners.end()) {
                owners.push_back(ordinal);
            }
        }
        if (owners.size() != 1) continue;  // shared or unknown: not cap-eligible
        const int ordinal = owners.front();
        if (!verified[static_cast<std::size_t>(ordinal - 1)]) continue;
        out[static_cast<std::size_t>(fi - 1)] = static_cast<std::uint32_t>(ordinal - 1);
    }
    return out;
}

}  // namespace onecad::tess
