// SurfaceNormals.cpp — see SurfaceNormals.h.
#include "tess/SurfaceNormals.h"

#include <algorithm>
#include <cmath>
#include <cstddef>
#include <exception>

#include <BRepAdaptor_Surface.hxx>
#include <BRepGProp.hxx>
#include <BRep_Tool.hxx>
#include <GProp_GProps.hxx>
#include <GeomAbs_SurfaceType.hxx>
#include <Poly_Triangle.hxx>
#include <Standard_Failure.hxx>
#include <TopAbs_ShapeEnum.hxx>
#include <TopExp_Explorer.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Edge.hxx>
#include <gp_Mat.hxx>
#include <gp_Pnt2d.hxx>
#include <gp_Sphere.hxx>
#include <gp_Trsf.hxx>
#include <gp_Vec.hxx>

namespace onecad::tess {

namespace {

// World size of the face plus the parameter span its UV nodes cover. Both floors
// in SurfaceNormals.h are expressed against these, so nothing here depends on the
// absolute magnitude of the model.
struct FaceScale {
    double diagonal = 1.0;
    double uSpan = 1.0;
    double vSpan = 1.0;
};

FaceScale face_scale(const std::vector<gp_Pnt>& nodes, const Handle(Poly_Triangulation)& tri) {
    FaceScale scale;
    double lo[3] = {1e300, 1e300, 1e300};
    double hi[3] = {-1e300, -1e300, -1e300};
    for (const gp_Pnt& p : nodes) {
        for (int a = 0; a < 3; ++a) {
            lo[a] = std::min(lo[a], p.Coord(a + 1));
            hi[a] = std::max(hi[a], p.Coord(a + 1));
        }
    }
    const double dx = hi[0] - lo[0], dy = hi[1] - lo[1], dz = hi[2] - lo[2];
    scale.diagonal = std::sqrt(dx * dx + dy * dy + dz * dz);
    if (!(scale.diagonal > 0.0) || !std::isfinite(scale.diagonal)) scale.diagonal = 1.0;
    if (tri->HasUVNodes()) {
        double ulo = 1e300, uhi = -1e300, vlo = 1e300, vhi = -1e300;
        for (int i = 1; i <= tri->NbNodes(); ++i) {
            const gp_Pnt2d uv = tri->UVNode(i);
            ulo = std::min(ulo, uv.X());
            uhi = std::max(uhi, uv.X());
            vlo = std::min(vlo, uv.Y());
            vhi = std::max(vhi, uv.Y());
        }
        if (uhi > ulo) scale.uSpan = uhi - ulo;
        if (vhi > vlo) scale.vSpan = vhi - vlo;
    }
    return scale;
}

// One triangle in EMITTED winding order, so `areaNormal` already points the way
// the rasterizer will see it (outward for a valid solid).
struct TriangleRecord {
    std::uint32_t node[3] = {0, 0, 0};
    gp_Vec areaNormal{0.0, 0.0, 0.0};
    gp_Vec unitNormal{0.0, 0.0, 1.0};
    bool usable = false;
};

std::vector<TriangleRecord> build_triangles(const Handle(Poly_Triangulation)& tri,
                                            const std::vector<gp_Pnt>& nodes,
                                            bool reverseWinding, const FaceScale& scale) {
    std::vector<TriangleRecord> out;
    out.reserve(static_cast<std::size_t>(tri->NbTriangles()));
    const double edgeFloor = kMinTriangleEdgeRatio * scale.diagonal;
    for (int t = 1; t <= tri->NbTriangles(); ++t) {
        Standard_Integer n1 = 0, n2 = 0, n3 = 0;
        tri->Triangle(t).Get(n1, n2, n3);
        if (reverseWinding) std::swap(n2, n3);
        TriangleRecord rec;
        rec.node[0] = static_cast<std::uint32_t>(n1 - 1);
        rec.node[1] = static_cast<std::uint32_t>(n2 - 1);
        rec.node[2] = static_cast<std::uint32_t>(n3 - 1);
        const gp_Pnt& a = nodes[rec.node[0]];
        const gp_Pnt& b = nodes[rec.node[1]];
        const gp_Pnt& c = nodes[rec.node[2]];
        const gp_Vec ab(a, b);
        const gp_Vec ac(a, c);
        rec.areaNormal = ab.Crossed(ac);
        const double lab = ab.Magnitude();
        const double lac = ac.Magnitude();
        const double area2 = rec.areaNormal.Magnitude();
        rec.usable = std::isfinite(area2) && lab > edgeFloor && lac > edgeFloor &&
                     area2 >= kMinTriangleSine * lab * lac;
        if (rec.usable) rec.unitNormal = rec.areaNormal.Divided(area2);
        out.push_back(rec);
    }
    return out;
}

struct SurfaceSample {
    gp_Vec direction{0.0, 0.0, 1.0};
    NormalProvenance provenance = NormalProvenance::Surface;
    bool valid = false;
};

// NUM §7.1 for a regular node, NUM §7.3's analytic case at a sphere pole. The
// `orientedSign` argument is `orientationSign * sign(det A)`; see the header for
// why the determinant belongs on the cross product exactly once.
SurfaceSample surface_normal_at(const BRepAdaptor_Surface& surf, GeomAbs_SurfaceType type,
                                double u, double v, const gp_Pnt& world, double orientationSign,
                                double orientedSign, const FaceScale& scale) {
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
    const double floorMm = kMinDerivativeSpanRatio * scale.diagonal;
    if (lu * scale.uSpan >= floorMm && lv * scale.vSpan >= floorMm &&
        g >= kMinDerivativeSine * lu * lv) {
        out.direction = cross.Multiplied(orientedSign / g);
        out.provenance = NormalProvenance::Surface;
        out.valid = true;
        return out;
    }
    // NUM §7.3: "Sphere pole: use the normalized vector from analytic sphere
    // center to point, with orientation/transform." The centre comes back in the
    // same world placement as the derivatives, so no location is applied twice.
    if (type == GeomAbs_Sphere) {
        const gp_Vec radial(surf.Sphere().Location(), world);
        const double r = radial.Magnitude();
        if (std::isfinite(r) && r > floorMm) {
            out.direction = radial.Multiplied(orientationSign / r);
            out.provenance = NormalProvenance::AnalyticSingular;
            out.valid = true;
        }
    }
    return out;
}

// Greatest pairwise angle among unit vectors, in radians (NUM §7.3's "maximum
// angular spread"). Fans are small, so the quadratic pass is not a cost.
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

// Node -> usable incident triangles, in compressed row form. A vector-per-node
// map is the same information, but 21k heap allocations on a torus face is most
// of the cost of the whole pass; this is two allocations.
struct Incidence {
    std::vector<std::uint32_t> offsets;  // nbNodes + 1
    std::vector<std::uint32_t> items;    // triangle indices, ascending within a node
};

Incidence build_incidence(const std::vector<TriangleRecord>& triangles, int nbNodes) {
    Incidence inc;
    inc.offsets.assign(static_cast<std::size_t>(nbNodes) + 1U, 0U);
    for (const TriangleRecord& rec : triangles) {
        if (!rec.usable) continue;
        for (const std::uint32_t n : rec.node) ++inc.offsets[static_cast<std::size_t>(n) + 1U];
    }
    for (std::size_t i = 1; i < inc.offsets.size(); ++i) inc.offsets[i] += inc.offsets[i - 1];
    inc.items.resize(inc.offsets.back());
    std::vector<std::uint32_t> cursor(inc.offsets.begin(), inc.offsets.end() - 1);
    for (std::size_t t = 0; t < triangles.size(); ++t) {
        if (!triangles[t].usable) continue;
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
// triangulation carries no UV nodes at all.
NodeDecision decide_from_triangles(const std::vector<TriangleRecord>& triangles,
                                   const std::uint32_t* incident, std::size_t count) {
    NodeDecision out;
    std::vector<gp_Vec> units;
    units.reserve(count);
    gp_Vec weighted(0.0, 0.0, 0.0);
    for (std::size_t k = 0; k < count; ++k) {
        const TriangleRecord& rec = triangles[incident[k]];
        units.push_back(rec.unitNormal);
        weighted += rec.areaNormal;
    }
    if (units.empty()) return out;  // Missing — every incident triangle is degenerate
    if (units.size() == 1 || max_spread_rad(units) <= kSingularSpreadRad) {
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
    const FaceScale scale = face_scale(out.worldNodes, tri);
    const std::vector<TriangleRecord> triangles =
        build_triangles(tri, out.worldNodes, out.reverseWinding, scale);

    for (const TriangleRecord& rec : triangles) {
        if (!rec.usable) ++out.droppedTriangles;
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

    // A plane's derivatives are constant by definition, so one D1 answers every
    // node exactly (NUM §7.3's "Plane: use oriented analytic plane normal"). This
    // is the prismatic-model fast path, not an approximation.
    SurfaceSample planar;
    if (surfaceReady && haveUv && type == GeomAbs_Plane) {
        try {
            const gp_Pnt2d uv = tri->UVNode(1);
            planar = surface_normal_at(surf, type, uv.X(), uv.Y(), out.worldNodes[0],
                                       orientationSign, orientedSign, scale);
        } catch (const Standard_Failure&) {
            planar.valid = false;
        }
    }

    for (int i = 0; i < nbNodes; ++i) {
        const std::size_t n = static_cast<std::size_t>(i);
        SurfaceSample sample = planar;
        if (surfaceReady && haveUv && !planar.valid) {
            try {
                const gp_Pnt2d uv = tri->UVNode(i + 1);
                sample = surface_normal_at(surf, type, uv.X(), uv.Y(), out.worldNodes[n],
                                           orientationSign, orientedSign, scale);
            } catch (const Standard_Failure&) {
                sample.valid = false;
            }
        }
        if (sample.valid) {
            decisions[n].normal = sample.direction;
            decisions[n].provenance = sample.provenance;
            // An analytic limit fixes the LINE the normal lies on; the sign
            // convention of an indirect gp_Ax3 does not. Where the node has usable
            // facets, let them settle the sign — a discrete choice, never a
            // direction estimate.
            if (sample.provenance == NormalProvenance::AnalyticSingular) {
                gp_Vec witness(0.0, 0.0, 0.0);
                for (std::size_t k = 0; k < incident_count(n); ++k) {
                    witness += triangles[incident_begin(n)[k]].areaNormal;
                }
                if (witness.Magnitude() > 0.0 && witness.Dot(decisions[n].normal) < 0.0) {
                    decisions[n].normal.Reverse();
                }
            }
            continue;
        }
        decisions[n] = decide_from_triangles(triangles, incident_begin(n), incident_count(n));
    }

    // --- emission ----------------------------------------------------------
    // Pass 1 gives every node its own vertex at its own index, so `nodeRemap` is
    // the identity and a caller's face-local vertex block stays contiguous.
    out.nodeRemap.resize(static_cast<std::size_t>(nbNodes));
    out.vertexNode.reserve(static_cast<std::size_t>(nbNodes));
    out.provenance.reserve(static_cast<std::size_t>(nbNodes));
    out.normals.reserve(static_cast<std::size_t>(nbNodes) * 3U);
    out.triangleVertexIndices.reserve(triangles.size() * 3U);
    // Per triangle CORNER, the split copy that corner must use; kNoCopy keeps the
    // node's base vertex. A triangle can have more than one split corner, so this
    // is written per corner and never rewritten wholesale.
    constexpr std::uint32_t kNoCopy = 0xFFFFFFFFU;
    std::vector<std::uint32_t> copyOfTriangle(triangles.size() * 3U, kNoCopy);
    for (int i = 0; i < nbNodes; ++i) {
        const std::size_t n = static_cast<std::size_t>(i);
        out.nodeRemap[n] = static_cast<std::uint32_t>(i);
        out.vertexNode.push_back(static_cast<std::uint32_t>(i));
        gp_Vec normal = decisions[n].normal;
        NormalProvenance prov = decisions[n].provenance;
        if (decisions[n].split && incident_count(n) > 0) {
            normal = triangles[incident_begin(n)[0]].unitNormal;
            ++out.splitCount;
        } else if (prov == NormalProvenance::TriangulationFallback) {
            ++out.fallbackCount;
        } else if (prov == NormalProvenance::Missing) {
            // NUM §7.3: "do not emit an arbitrary normal". Missing means the node
            // has no surface answer AND no usable incident triangle, which by
            // construction means every triangle touching it was degenerate and has
            // been dropped — so this vertex is referenced by no emitted triangle
            // and is never shaded. The slot exists only to keep `nodeRemap` the
            // identity; the value is inert and the count is reported.
            ++out.missingCount;
        }
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
        if (!decisions[n].split) continue;
        for (std::size_t k = 1; k < incident_count(n); ++k) {
            const std::uint32_t t = incident_begin(n)[k];
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
    for (std::size_t t = 0; t < triangles.size(); ++t) {
        if (!triangles[t].usable) continue;
        for (int corner = 0; corner < 3; ++corner) {
            const std::uint32_t copy = copyOfTriangle[t * 3U + static_cast<std::size_t>(corner)];
            out.triangleVertexIndices.push_back(copy != kNoCopy ? copy : triangles[t].node[corner]);
        }
    }

    out.complete = !out.triangleVertexIndices.empty();
    if (out.droppedTriangles > 0 && out.diagnostic.empty()) {
        out.diagnostic = "dropped " + std::to_string(out.droppedTriangles) + " degenerate triangles";
    }
    if (out.missingCount > 0) {
        out.diagnostic += (out.diagnostic.empty() ? "" : "; ") + std::to_string(out.missingCount) +
                          " nodes have no normal source";
    }
    return out;
}

bool face_is_degenerate(const TopoDS_Face& face, double bodyDiagonalMm) {
    bool sawEdge = false;
    bool allDegenerate = true;
    for (TopExp_Explorer it(face, TopAbs_EDGE); it.More(); it.Next()) {
        sawEdge = true;
        if (!BRep_Tool::Degenerated(TopoDS::Edge(it.Current()))) allDegenerate = false;
    }
    if (sawEdge && allDegenerate) return true;
    try {
        GProp_GProps props;
        BRepGProp::SurfaceProperties(face, props);
        const double area = props.Mass();
        if (!std::isfinite(area)) return false;
        const double reference = bodyDiagonalMm > 0.0 ? bodyDiagonalMm : 1.0;
        return area <= kDegenerateFaceAreaRatio * reference * reference;
    } catch (const Standard_Failure&) {
        return false;
    } catch (const std::exception&) {
        return false;
    }
}

}  // namespace onecad::tess
