// test_surface_normals.cpp — VP-HARDENING WP09 / VP10 / findings R04, R11.
//
// Acceptance TEST-NORMAL-01..06 and TEST-MESH-06
// (docs/viewport-hardening/04-ACCEPTANCE-AND-TESTS.md §6/§5, oracle rules §3.2).
//
// Every oracle here is INDEPENDENT of the tessellator: the exact analytic normal
// of the primitive's own equation (plane axis, radial direction, torus tube
// centre, cone gradient), the triangle normal recomputed from the EMITTED
// positions, and `acos(clamp(dot,-1,1))` in degrees. Sign is handled separately
// by an outwardness test against the body centre, never by taking an absolute
// value. Nothing compares the tessellator to itself.
#include <algorithm>
#include <array>
#include <chrono>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <functional>
#include <string>
#include <utility>
#include <vector>

#include <BRepAdaptor_Surface.hxx>
#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakeVertex.hxx>
#include <BRepBuilderAPI_MakeWire.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <BRepTools.hxx>
#include <BRepFilletAPI_MakeFillet.hxx>
#include <BRepMesh_IncrementalMesh.hxx>
#include <BRep_Builder.hxx>
#include <BRep_Tool.hxx>
#include <Geom2d_Line.hxx>
#include <Geom_Plane.hxx>
#include <Geom_Surface.hxx>
#include <Geom_TrimmedCurve.hxx>
#include <Geom_BSplineCurve.hxx>
#include <Geom_BSplineSurface.hxx>
#include <Geom_BezierSurface.hxx>
#include <NCollection_Array1.hxx>
#include <NCollection_Array2.hxx>
#include <Poly_Triangulation.hxx>
#include <Precision.hxx>
#include <TopoDS_Compound.hxx>
#include <TopoDS_Wire.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Vertex.hxx>
#include <gp_Dir2d.hxx>
#include <gp_Pln.hxx>
#include <gp_Pnt2d.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakeCone.hxx>
#include <BRepPrimAPI_MakePrism.hxx>
#include <BRepPrimAPI_MakeRevol.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <BRepPrimAPI_MakeSphere.hxx>
#include <BRepPrimAPI_MakeTorus.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopLoc_Location.hxx>
#include <TopTools_IndexedDataMapOfShapeListOfShape.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Ax1.hxx>
#include <gp_Ax2.hxx>
#include <gp_Dir.hxx>
#include <gp_Mat.hxx>
#include <gp_Pnt.hxx>
#include <gp_Trsf.hxx>
#include <gp_Vec.hxx>

#include "tess/EdgeClassification.h"
#include "tess/SurfaceNormals.h"
#include "tess/Tessellate.h"

namespace {

int g_failures = 0;

void check(bool condition, const std::string& message) {
    if (!condition) {
        std::fprintf(stderr, "FAIL: %s\n", message.c_str());
        ++g_failures;
    }
}

constexpr double kPi = 3.14159265358979323846;

// ---------------------------------------------------------------------------
// MESH1 readers (mesh_format.md §4: 1 POSITIONS, 2 NORMALS, 3 INDICES,
// 4 FACE_RANGES, 5 FACE_ID_OFFS, 6 FACE_ID_CHARS).
// ---------------------------------------------------------------------------

std::uint16_t u16(const std::vector<std::uint8_t>& b, std::size_t at) {
    return static_cast<std::uint16_t>(b[at] | (b[at + 1] << 8));
}

std::uint32_t u32(const std::vector<std::uint8_t>& b, std::size_t at) {
    std::uint32_t v = 0;
    for (int i = 0; i < 4; ++i) v |= static_cast<std::uint32_t>(b[at + i]) << (i * 8);
    return v;
}

float f32(const std::vector<std::uint8_t>& b, std::size_t at) {
    const std::uint32_t bits = u32(b, at);
    float out = 0.0F;
    std::memcpy(&out, &bits, 4);
    return out;
}

struct Section {
    std::uint32_t type = 0;
    std::uint32_t offset = 0;
    std::uint32_t length = 0;
};

Section find_section(const std::vector<std::uint8_t>& b, std::uint32_t type) {
    if (b.size() < 64) return {};
    const std::uint16_t count = u16(b, 0x1E);
    if (b.size() < 64U + static_cast<std::size_t>(count) * 16U) return {};
    for (std::uint16_t i = 0; i < count; ++i) {
        const std::size_t entry = 64U + static_cast<std::size_t>(i) * 16U;
        if (u32(b, entry) == type) return {type, u32(b, entry + 8), u32(b, entry + 12)};
    }
    return {};
}

struct MeshView {
    std::vector<gp_Pnt> positions;
    std::vector<gp_Vec> normals;
    std::vector<std::uint32_t> indices;
    std::vector<std::pair<std::uint32_t, std::uint32_t>> faceRanges;  // firstTri, triCount
    // Emitted vertex -> owning face ordinal (0-based), 0xFFFFFFFF when unreferenced.
    std::vector<std::uint32_t> vertexFace;
};

MeshView read_mesh(const std::vector<std::uint8_t>& blob) {
    MeshView m;
    const std::uint32_t vcount = u32(blob, 0x08);
    const std::uint32_t tcount = u32(blob, 0x0C);
    const std::uint32_t fcount = u32(blob, 0x10);
    const Section pos = find_section(blob, 1);
    const Section nor = find_section(blob, 2);
    const Section idx = find_section(blob, 3);
    const Section rng = find_section(blob, 4);
    for (std::uint32_t i = 0; i < vcount; ++i) {
        const std::size_t at = pos.offset + static_cast<std::size_t>(i) * 12U;
        m.positions.emplace_back(f32(blob, at), f32(blob, at + 4), f32(blob, at + 8));
        const std::size_t nat = nor.offset + static_cast<std::size_t>(i) * 12U;
        m.normals.emplace_back(f32(blob, nat), f32(blob, nat + 4), f32(blob, nat + 8));
    }
    for (std::uint32_t i = 0; i < tcount * 3U; ++i) {
        m.indices.push_back(u32(blob, idx.offset + static_cast<std::size_t>(i) * 4U));
    }
    for (std::uint32_t f = 0; f < fcount; ++f) {
        const std::size_t at = rng.offset + static_cast<std::size_t>(f) * 8U;
        m.faceRanges.emplace_back(u32(blob, at), u32(blob, at + 4));
    }
    m.vertexFace.assign(vcount, 0xFFFFFFFFU);
    for (std::uint32_t f = 0; f < fcount; ++f) {
        const auto [first, count] = m.faceRanges[f];
        for (std::uint32_t t = 0; t < count; ++t) {
            for (int k = 0; k < 3; ++k) {
                m.vertexFace[m.indices[(static_cast<std::size_t>(first) + t) * 3U + k]] = f;
            }
        }
    }
    return m;
}

double angle_deg(const gp_Vec& a, const gp_Vec& b) {
    const double la = a.Magnitude();
    const double lb = b.Magnitude();
    if (la <= 0.0 || lb <= 0.0) return 180.0;
    double d = a.Dot(b) / (la * lb);
    d = std::clamp(d, -1.0, 1.0);
    return std::acos(d) * 180.0 / kPi;
}

// Triangle normal recomputed from the EMITTED winding — the independent check
// that the emitted winding and the emitted shading normals agree.
gp_Vec emitted_triangle_normal(const MeshView& m, std::size_t tri) {
    const gp_Pnt& a = m.positions[m.indices[tri * 3U]];
    const gp_Pnt& b = m.positions[m.indices[tri * 3U + 1]];
    const gp_Pnt& c = m.positions[m.indices[tri * 3U + 2]];
    return gp_Vec(a, b).Crossed(gp_Vec(a, c));
}

// Greatest angular error over every vertex of the mesh whose position satisfies
// `select`, measured against the oracle `exact`.
struct AngularReport {
    double maxDeg = 0.0;
    std::size_t samples = 0;
    std::size_t plusZ = 0;  // vertices whose normal is exactly (0,0,1)
};

AngularReport measure(const MeshView& m, const std::function<bool(const gp_Pnt&)>& select,
                      const std::function<gp_Vec(const gp_Pnt&)>& exact) {
    AngularReport r;
    for (std::size_t v = 0; v < m.positions.size(); ++v) {
        if (m.vertexFace[v] == 0xFFFFFFFFU) continue;
        if (!select(m.positions[v])) continue;
        ++r.samples;
        r.maxDeg = std::max(r.maxDeg, angle_deg(m.normals[v], exact(m.positions[v])));
        const gp_Vec& n = m.normals[v];
        if (n.X() == 0.0 && n.Y() == 0.0 && n.Z() == 1.0) ++r.plusZ;
    }
    return r;
}

// The emitted vertices a face's own triangles reference, in ascending order.
std::vector<std::uint32_t> face_vertices(const MeshView& m, std::uint32_t face) {
    std::vector<std::uint32_t> out;
    for (std::size_t v = 0; v < m.positions.size(); ++v) {
        if (m.vertexFace[v] == face) out.push_back(static_cast<std::uint32_t>(v));
    }
    return out;
}

AngularReport measure_face(const MeshView& m, std::uint32_t face,
                           const std::function<gp_Vec(const gp_Pnt&)>& exact) {
    AngularReport r;
    for (const std::uint32_t v : face_vertices(m, face)) {
        ++r.samples;
        r.maxDeg = std::max(r.maxDeg, angle_deg(m.normals[v], exact(m.positions[v])));
        const gp_Vec& n = m.normals[v];
        if (n.X() == 0.0 && n.Y() == 0.0 && n.Z() == 1.0) ++r.plusZ;
    }
    return r;
}

// The world-Z extent of one face's emitted vertices — used to tell a cylinder's
// or cone's lateral face from its flat caps WITHOUT consulting any normal.
double face_z_extent(const MeshView& m, std::uint32_t face) {
    double lo = 1e300, hi = -1e300;
    for (const std::uint32_t v : face_vertices(m, face)) {
        lo = std::min(lo, m.positions[v].Z());
        hi = std::max(hi, m.positions[v].Z());
    }
    return hi > lo ? hi - lo : 0.0;
}

// Pairs of DISTINCT emitted vertices of the SAME face at the SAME 3D point — the
// periodic-seam duplicates. Returns the greatest normal disagreement in degrees.
double max_seam_disagreement_deg(const MeshView& m, double coincidence, std::size_t& pairs) {
    double worst = 0.0;
    pairs = 0;
    for (std::size_t i = 0; i < m.positions.size(); ++i) {
        if (m.vertexFace[i] == 0xFFFFFFFFU) continue;
        for (std::size_t j = i + 1; j < m.positions.size(); ++j) {
            if (m.vertexFace[j] != m.vertexFace[i]) continue;
            if (m.positions[i].Distance(m.positions[j]) > coincidence) continue;
            ++pairs;
            worst = std::max(worst, angle_deg(m.normals[i], m.normals[j]));
        }
    }
    return worst;
}

onecad::tess::BodyMesh mesh_of(const TopoDS_Shape& shape, const char* lod) {
    return onecad::tess::tessellate_body(shape, "body_probe", lod, /*include_edges=*/true, nullptr);
}

// ---------------------------------------------------------------------------
// TEST-NORMAL-01 — planar box faces, both orientations.
// ---------------------------------------------------------------------------

// The exact outward normal of ONE planar box face, determined from the emitted
// POSITIONS alone: exactly one coordinate is constant across the face's vertices,
// and the outward side is the one pointing away from the box centre. No normal is
// consulted, so this is an independent oracle.
gp_Vec box_face_outward(const MeshView& m, std::uint32_t face, const gp_Pnt& centre, bool& ok) {
    const std::vector<std::uint32_t> vs = face_vertices(m, face);
    ok = false;
    if (vs.empty()) return gp_Vec(0, 0, 0);
    for (int axis = 0; axis < 3; ++axis) {
        const double c0 = m.positions[vs.front()].Coord(axis + 1);
        bool constant = true;
        for (const std::uint32_t v : vs) {
            if (std::abs(m.positions[v].Coord(axis + 1) - c0) > 1e-6) constant = false;
        }
        if (!constant) continue;
        ok = true;
        gp_Vec n(0, 0, 0);
        n.SetCoord(axis + 1, c0 > centre.Coord(axis + 1) ? 1.0 : -1.0);
        return n;
    }
    return gp_Vec(0, 0, 0);
}

void test_normal_01_box_both_orientations() {
    const double sx = 40.0, sy = 30.0, sz = 20.0;
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(sx, sy, sz).Shape();
    const onecad::tess::BodyMesh bm = mesh_of(box, "fine");
    check(bm.ok, "TEST-NORMAL-01: box tessellates");
    const MeshView m = read_mesh(bm.blob);

    const gp_Pnt centre(sx / 2.0, sy / 2.0, sz / 2.0);
    double worstForward = 0.0;
    std::size_t planarFaces = 0, shaded = 0;
    for (std::uint32_t f = 0; f < m.faceRanges.size(); ++f) {
        bool ok = false;
        const gp_Vec exact = box_face_outward(m, f, centre, ok);
        if (!ok) continue;
        ++planarFaces;
        const AngularReport rep = measure_face(m, f, [&](const gp_Pnt&) { return exact; });
        shaded += rep.samples;
        worstForward = std::max(worstForward, rep.maxDeg);
    }
    check(planarFaces == 6, "TEST-NORMAL-01: all six box faces are axis-constant planes");
    check(shaded > 0, "TEST-NORMAL-01: box has shaded vertices");
    check(worstForward <= 0.1,
          "TEST-NORMAL-01: every box vertex normal is within 0.1 deg of the exact outward face "
          "normal (measured " + std::to_string(worstForward) + " deg)");

    // Sharp edges stay sharp: a box corner is three distinct emitted vertices
    // carrying three mutually perpendicular normals, never one averaged normal.
    std::size_t seamPairs = 0;
    const double corner = max_seam_disagreement_deg(m, 1e-6, seamPairs);
    check(seamPairs == 0,
          "TEST-NORMAL-01: no two vertices of the SAME planar face coincide (found " +
              std::to_string(seamPairs) + ")");
    (void)corner;
    std::size_t coincident = 0;
    for (std::size_t i = 0; i < m.positions.size(); ++i) {
        for (std::size_t j = i + 1; j < m.positions.size(); ++j) {
            if (m.positions[i].Distance(m.positions[j]) < 1e-9) ++coincident;
        }
    }
    check(coincident > 0, "TEST-NORMAL-01: box corners are duplicated per face (crease split)");

    // The complemented solid: the material side is inverted, so every normal must
    // flip exactly once. A normal that did not flip means the orientation sign was
    // dropped; one that flipped twice means it was applied twice.
    const TopoDS_Shape reversed = box.Reversed();
    const onecad::tess::BodyMesh rm = mesh_of(reversed, "fine");
    const MeshView r = read_mesh(rm.blob);
    double worstInward = 0.0;
    std::size_t inwardFaces = 0;
    for (std::uint32_t f = 0; f < r.faceRanges.size(); ++f) {
        bool ok = false;
        const gp_Vec exact = box_face_outward(r, f, centre, ok);
        if (!ok) continue;
        ++inwardFaces;
        const AngularReport rep =
            measure_face(r, f, [&](const gp_Pnt&) { return exact.Reversed(); });
        worstInward = std::max(worstInward, rep.maxDeg);
    }
    check(inwardFaces == 6, "TEST-NORMAL-01: the complemented box still has six planar faces");
    check(worstInward <= 0.1,
          "TEST-NORMAL-01: every vertex normal of the COMPLEMENTED box points inward within "
          "0.1 deg (measured " + std::to_string(worstInward) + " deg)");
}

// ---------------------------------------------------------------------------
// TEST-NORMAL-02 — cylinder periodic seam and tangent fillet chain.
// ---------------------------------------------------------------------------

void test_normal_02_cylinder_seam_and_fillet_chain() {
    const double radius = 20.0, height = 40.0;
    const TopoDS_Shape cyl = BRepPrimAPI_MakeCylinder(radius, height).Shape();
    const onecad::tess::BodyMesh bm = mesh_of(cyl, "fine");
    check(bm.ok, "TEST-NORMAL-02: cylinder tessellates");
    const MeshView m = read_mesh(bm.blob);

    // The lateral face is the only one whose vertices span the full height; the two
    // caps are flat in z. Picking it by geometry keeps the oracle independent.
    double sideWorst = 0.0;
    std::size_t sideSamples = 0, lateralFaces = 0;
    for (std::uint32_t f = 0; f < m.faceRanges.size(); ++f) {
        if (face_z_extent(m, f) < height * 0.5) continue;
        ++lateralFaces;
        const AngularReport rep = measure_face(
            m, f, [](const gp_Pnt& p) { return gp_Vec(p.X(), p.Y(), 0.0); });
        sideSamples += rep.samples;
        sideWorst = std::max(sideWorst, rep.maxDeg);
    }
    check(lateralFaces == 1, "TEST-NORMAL-02: exactly one cylinder face spans the height");
    check(sideSamples > 0, "TEST-NORMAL-02: cylinder has lateral vertices");
    check(sideWorst <= 0.1,
          "TEST-NORMAL-02: every cylinder side normal is within 0.1 deg of the exact radial "
          "direction (measured " + std::to_string(sideWorst) + " deg)");

    std::size_t pairs = 0;
    const double seam = max_seam_disagreement_deg(m, 1e-6, pairs);
    check(pairs > 0, "TEST-NORMAL-02: the cylinder's periodic seam duplicates nodes");
    check(seam <= 0.2,
          "TEST-NORMAL-02: the two nodes at the cylinder's periodic seam agree within 0.2 deg "
          "(measured " + std::to_string(seam) + " deg)");

    // Tangent fillet chain: every edge of a 40x30x20 box blended at r=4.
    const TopoDS_Shape blendBox = BRepPrimAPI_MakeBox(40.0, 30.0, 20.0).Shape();
    BRepFilletAPI_MakeFillet blend(blendBox);
    TopTools_IndexedMapOfShape blendEdges;
    TopExp::MapShapes(blendBox, TopAbs_EDGE, blendEdges);
    for (int e = 1; e <= blendEdges.Extent(); ++e) {
        blend.Add(4.0, TopoDS::Edge(blendEdges(e)));
    }
    blend.Build();
    check(blend.IsDone(), "TEST-NORMAL-02: the all-edge fillet builds");
    if (blend.IsDone()) {
        const TopoDS_Shape filleted = blend.Shape();
        TopTools_IndexedMapOfShape faces;
        TopExp::MapShapes(filleted, TopAbs_FACE, faces);
        const onecad::tess::BodyMesh fm = mesh_of(filleted, "fine");
        const MeshView f = read_mesh(fm.blob);
        check(f.faceRanges.size() == static_cast<std::size_t>(faces.Extent()),
              "TEST-NORMAL-02: the filleted box keeps every BRep face split (no welding)");

        // Nodes at the SAME 3D point on DIFFERENT faces are the tangent boundary
        // between a blend and its planar neighbour; they must agree within 0.2 deg.
        double worst = 0.0;
        std::size_t crossings = 0;
        for (std::size_t i = 0; i < f.positions.size(); ++i) {
            if (f.vertexFace[i] == 0xFFFFFFFFU) continue;
            for (std::size_t j = i + 1; j < f.positions.size(); ++j) {
                if (f.vertexFace[j] == 0xFFFFFFFFU || f.vertexFace[j] == f.vertexFace[i]) continue;
                if (f.positions[i].Distance(f.positions[j]) > 1e-6) continue;
                const double a = angle_deg(f.normals[i], f.normals[j]);
                if (a > 45.0) continue;  // a genuine crease corner, not a tangent join
                ++crossings;
                worst = std::max(worst, a);
            }
        }
        check(crossings > 0, "TEST-NORMAL-02: the fillet chain has tangent face boundaries");
        check(worst <= 0.2,
              "TEST-NORMAL-02: tangent fillet/planar boundary normals agree within 0.2 deg across "
              "the two faces (measured " + std::to_string(worst) + " deg)");
    }
}

// ---------------------------------------------------------------------------
// TEST-NORMAL-03 — sphere poles, torus seam, cone apex.
// ---------------------------------------------------------------------------

void test_normal_03_sphere_torus_cone() {
    // Sphere r=20 on an OBLIQUE axis, so a (0,0,1) pole answer is detectable as a
    // bug rather than accidentally correct.
    const gp_Dir axis(1.0, 2.0, 3.0);
    const gp_Pnt centre(0.0, 0.0, 0.0);
    const TopoDS_Shape sphere = BRepPrimAPI_MakeSphere(gp_Ax2(centre, axis), 20.0).Shape();
    const onecad::tess::BodyMesh sm = mesh_of(sphere, "fine");
    check(sm.ok, "TEST-NORMAL-03: sphere tessellates");
    const MeshView s = read_mesh(sm.blob);
    const AngularReport radial = measure(
        s, [](const gp_Pnt&) { return true; },
        [&](const gp_Pnt& p) { return gp_Vec(centre, p); });
    check(radial.samples > 0, "TEST-NORMAL-03: sphere has shaded vertices");
    check(radial.maxDeg <= 0.1,
          "TEST-NORMAL-03: every sphere normal is within 0.1 deg of the exact radial direction, "
          "poles included (measured " + std::to_string(radial.maxDeg) + " deg)");
    check(radial.plusZ == 0,
          "TEST-NORMAL-03: no sphere vertex falls back to world +Z (found " +
              std::to_string(radial.plusZ) + ")");

    // Torus R=30 r=8: the exact normal is the tube radius direction.
    const double bigR = 30.0, tubeR = 8.0;
    const TopoDS_Shape torus = BRepPrimAPI_MakeTorus(bigR, tubeR).Shape();
    const onecad::tess::BodyMesh tm = mesh_of(torus, "fine");
    const MeshView t = read_mesh(tm.blob);
    const AngularReport tube = measure(
        t, [](const gp_Pnt&) { return true; },
        [&](const gp_Pnt& p) {
            const double rho = std::hypot(p.X(), p.Y());
            const gp_Pnt onCircle(bigR * p.X() / rho, bigR * p.Y() / rho, 0.0);
            return gp_Vec(onCircle, p);
        });
    check(tube.samples > 0, "TEST-NORMAL-03: torus has shaded vertices");
    check(tube.maxDeg <= 0.1,
          "TEST-NORMAL-03: every torus normal is within 0.1 deg of the exact tube normal "
          "(measured " + std::to_string(tube.maxDeg) + " deg)");
    std::size_t torusPairs = 0;
    const double torusSeam = max_seam_disagreement_deg(t, 1e-6, torusPairs);
    check(torusPairs > 0, "TEST-NORMAL-03: the torus duplicates its periodic seam nodes");
    check(torusSeam <= 0.2,
          "TEST-NORMAL-03: torus seam normals agree within 0.2 deg (measured " +
              std::to_string(torusSeam) + " deg)");

    // Cone r1=10 r2=0 h=20: the apex has NO unique smooth normal. What it must not
    // have is world +Z, which is what an area-weighted fan average produces.
    const double coneR = 10.0, coneH = 20.0;
    const TopoDS_Shape cone = BRepPrimAPI_MakeCone(coneR, 0.0, coneH).Shape();
    const onecad::tess::BodyMesh cm = mesh_of(cone, "fine");
    const MeshView c = read_mesh(cm.blob);
    std::size_t apexVerts = 0, apexPlusZ = 0;
    double apexWorst = 0.0;
    for (std::size_t v = 0; v < c.positions.size(); ++v) {
        if (c.vertexFace[v] == 0xFFFFFFFFU) continue;
        if (c.positions[v].Distance(gp_Pnt(0.0, 0.0, coneH)) > 1e-4) continue;
        ++apexVerts;
        const gp_Vec& n = c.normals[v];
        if (n.X() == 0.0 && n.Y() == 0.0 && n.Z() == 1.0) ++apexPlusZ;
        // The split normal must match a real incident triangle of the emitted mesh.
        double best = 180.0;
        for (std::size_t tri = 0; tri * 3U < c.indices.size(); ++tri) {
            if (c.indices[tri * 3U] != v && c.indices[tri * 3U + 1] != v &&
                c.indices[tri * 3U + 2] != v) {
                continue;
            }
            const gp_Vec face = emitted_triangle_normal(c, tri);
            if (face.Magnitude() <= 0.0) continue;
            best = std::min(best, angle_deg(n, face));
        }
        apexWorst = std::max(apexWorst, best);
    }
    check(apexVerts > 0, "TEST-NORMAL-03: the cone apex is present in the mesh");
    check(apexPlusZ == 0,
          "TEST-NORMAL-03: no cone apex vertex falls back to world +Z (found " +
              std::to_string(apexPlusZ) + ")");
    check(apexWorst <= 0.1,
          "TEST-NORMAL-03: each cone apex normal is within 0.1 deg of one of its own incident "
          "triangles' face-side normals (measured " + std::to_string(apexWorst) + " deg)");
    // The lateral surface away from the apex is exact: grad(x^2+y^2-R^2(1-z/H)^2).
    const AngularReport lateral = measure(
        c,
        [&](const gp_Pnt& p) {
            const double rho = std::hypot(p.X(), p.Y());
            return rho > 1.0 && p.Z() > 0.05 * coneH;
        },
        [&](const gp_Pnt& p) {
            const double rho = std::hypot(p.X(), p.Y());
            return gp_Vec(p.X() / rho, p.Y() / rho, coneR / coneH);
        });
    check(lateral.samples > 0, "TEST-NORMAL-03: the cone has lateral vertices");
    check(lateral.maxDeg <= 0.1,
          "TEST-NORMAL-03: every cone lateral normal is within 0.1 deg of the exact cone normal "
          "(measured " + std::to_string(lateral.maxDeg) + " deg)");
}

// ---------------------------------------------------------------------------
// TEST-NORMAL-04 (F09) — transforms, winding parity, and the double-reflection trap.
// ---------------------------------------------------------------------------

struct WindingReport {
    double agreeFraction = 0.0;
    double outwardFraction = 0.0;
    double maxNormalErrDeg = 0.0;
};

WindingReport check_transformed_sphere(const TopoDS_Shape& shape, const gp_Pnt& centre) {
    const onecad::tess::BodyMesh bm = mesh_of(shape, "fine");
    const MeshView m = read_mesh(bm.blob);
    WindingReport r;
    std::size_t tris = 0, agree = 0, outward = 0;
    for (std::size_t tri = 0; tri * 3U < m.indices.size(); ++tri) {
        const gp_Vec face = emitted_triangle_normal(m, tri);
        if (face.Magnitude() <= 0.0) continue;
        ++tris;
        gp_Vec vertexAvg(0, 0, 0);
        gp_Pnt mid(0, 0, 0);
        for (int k = 0; k < 3; ++k) {
            const std::uint32_t v = m.indices[tri * 3U + k];
            vertexAvg += m.normals[v];
            mid.ChangeCoord() += m.positions[v].XYZ() / 3.0;
        }
        if (face.Dot(vertexAvg) > 0.0) ++agree;
        if (face.Dot(gp_Vec(centre, mid)) > 0.0) ++outward;
    }
    for (std::size_t v = 0; v < m.positions.size(); ++v) {
        if (m.vertexFace[v] == 0xFFFFFFFFU) continue;
        r.maxNormalErrDeg =
            std::max(r.maxNormalErrDeg, angle_deg(m.normals[v], gp_Vec(centre, m.positions[v])));
    }
    if (tris > 0) {
        r.agreeFraction = static_cast<double>(agree) / static_cast<double>(tris);
        r.outwardFraction = static_cast<double>(outward) / static_cast<double>(tris);
    }
    return r;
}

void test_normal_04_transforms_and_mirror() {
    const gp_Pnt origin(0.0, 0.0, 0.0);
    const TopoDS_Shape sphere = BRepPrimAPI_MakeSphere(gp_Ax2(origin, gp_Dir(1, 2, 3)), 20.0).Shape();

    gp_Trsf move;
    move.SetTranslation(gp_Vec(100.0, 200.0, 300.0));
    const WindingReport translated =
        check_transformed_sphere(sphere.Moved(TopLoc_Location(move)), gp_Pnt(100.0, 200.0, 300.0));
    check(translated.maxNormalErrDeg <= 0.1,
          "TEST-NORMAL-04: translated sphere normals stay within 0.1 deg of the transformed exact "
          "normal (measured " + std::to_string(translated.maxNormalErrDeg) + " deg)");
    check(translated.outwardFraction >= 0.999,
          "TEST-NORMAL-04: translated winding stays outward");

    gp_Trsf spin;
    spin.SetRotation(gp_Ax1(origin, gp_Dir(1, 2, 3)), 37.0 * kPi / 180.0);
    const WindingReport rotated = check_transformed_sphere(sphere.Moved(TopLoc_Location(spin)), origin);
    check(rotated.maxNormalErrDeg <= 0.1,
          "TEST-NORMAL-04: rotated sphere normals stay within 0.1 deg (measured " +
              std::to_string(rotated.maxNormalErrDeg) + " deg)");
    check(rotated.agreeFraction >= 0.999, "TEST-NORMAL-04: rotated winding agrees with the normals");

    // A TRUE reflection retained as the shape's location: det(A) < 0.
    gp_Trsf mirror;
    mirror.SetMirror(gp_Ax2(gp_Pnt(5.0, 0.0, 0.0), gp_Dir(1.0, 0.0, 0.0)));
    check(mirror.VectorialPart().Determinant() < 0.0,
          "TEST-NORMAL-04: the plane mirror really has a negative determinant");
    const TopoDS_Shape located = sphere.Moved(TopLoc_Location(mirror));
    const WindingReport reflected = check_transformed_sphere(located, gp_Pnt(10.0, 0.0, 0.0));
    check(reflected.outwardFraction >= 0.999,
          "TEST-NORMAL-04: a mirrored solid held as a NEGATIVE-determinant location keeps its "
          "emitted winding outward (measured " + std::to_string(reflected.outwardFraction) + ")");
    check(reflected.agreeFraction >= 0.999,
          "TEST-NORMAL-04: mirrored triangle winding agrees with the emitted vertex normals "
          "(measured " + std::to_string(reflected.agreeFraction) + ")");
    check(reflected.maxNormalErrDeg <= 0.1,
          "TEST-NORMAL-04: mirrored sphere normals stay within 0.1 deg of the reflected exact "
          "normal (measured " + std::to_string(reflected.maxNormalErrDeg) + " deg)");

    // The XOR corner: a REVERSED solid inside a NEGATIVE-determinant location.
    // Both flips must be applied, and applied once each, so the emitted winding
    // and the normals land on the complemented (inward) side together.
    const TopoDS_Shape mirroredComplement = located.Reversed();
    const WindingReport both = check_transformed_sphere(mirroredComplement, gp_Pnt(10.0, 0.0, 0.0));
    check(both.outwardFraction <= 0.001,
          "TEST-NORMAL-04: reversed XOR reflected flips exactly twice, so the winding is inward "
          "(outward fraction " + std::to_string(both.outwardFraction) + ")");
    check(both.agreeFraction >= 0.999,
          "TEST-NORMAL-04: reversed AND reflected keeps winding and normals on the same side "
          "(measured " + std::to_string(both.agreeFraction) + ")");
    check(both.maxNormalErrDeg >= 179.9,
          "TEST-NORMAL-04: the complemented mirrored sphere's normals are the exact reflected "
          "normal, negated (measured " + std::to_string(both.maxNormalErrDeg) + " deg)");

    // The double-reflection trap: OCCT's own transform builder may already have
    // flipped the face orientations. Assert outwardness, never a flip count.
    BRepBuilderAPI_Transform baked(sphere, mirror, /*copyGeom=*/false);
    check(baked.IsDone(), "TEST-NORMAL-04: BRepBuilderAPI_Transform applies the mirror");
    if (baked.IsDone()) {
        const WindingReport rebuilt = check_transformed_sphere(baked.Shape(), gp_Pnt(10.0, 0.0, 0.0));
        check(rebuilt.outwardFraction >= 0.999,
              "TEST-NORMAL-04: a mirrored solid rebuilt by BRepBuilderAPI_Transform is NOT flipped "
              "twice (outward fraction " + std::to_string(rebuilt.outwardFraction) + ")");
        check(rebuilt.maxNormalErrDeg <= 0.1,
              "TEST-NORMAL-04: rebuilt mirrored sphere normals stay within 0.1 deg (measured " +
                  std::to_string(rebuilt.maxNormalErrDeg) + " deg)");
    }
}

// ---------------------------------------------------------------------------
// TEST-NORMAL-06 — density independence across tiers.
// ---------------------------------------------------------------------------

void test_normal_06_density_independence() {
    const double radius = 20.0;
    for (const char* lod : {"coarse", "fine"}) {
        const TopoDS_Shape sphere = BRepPrimAPI_MakeSphere(radius).Shape();
        const onecad::tess::BodyMesh bm = mesh_of(sphere, lod);
        const MeshView m = read_mesh(bm.blob);
        const AngularReport radialErr = measure(
            m, [](const gp_Pnt&) { return true; },
            [](const gp_Pnt& p) { return gp_Vec(gp_Pnt(0, 0, 0), p); });
        check(radialErr.maxDeg <= 0.1,
              std::string("TEST-NORMAL-06: sphere normals are density independent at ") + lod +
                  " (measured " + std::to_string(radialErr.maxDeg) + " deg)");
        std::size_t pairs = 0;
        const double seam = max_seam_disagreement_deg(m, 1e-6, pairs);
        check(seam <= 0.2,
              std::string("TEST-NORMAL-06: sphere seam agreement holds at ") + lod + " (measured " +
                  std::to_string(seam) + " deg)");

        const TopoDS_Shape cyl = BRepPrimAPI_MakeCylinder(radius, 40.0).Shape();
        const onecad::tess::BodyMesh cm = mesh_of(cyl, lod);
        const MeshView c = read_mesh(cm.blob);
        double sideWorst = 0.0;
        for (std::uint32_t f = 0; f < c.faceRanges.size(); ++f) {
            if (face_z_extent(c, f) < 20.0) continue;  // the lateral face, not a cap
            const AngularReport rep = measure_face(
                c, f, [](const gp_Pnt& p) { return gp_Vec(p.X(), p.Y(), 0.0); });
            sideWorst = std::max(sideWorst, rep.maxDeg);
        }
        check(sideWorst <= 0.1,
              std::string("TEST-NORMAL-06: cylinder normals are density independent at ") + lod +
                  " (measured " + std::to_string(sideWorst) + " deg)");
    }
}


// ---------------------------------------------------------------------------
// Unit-length window (NUM §1.2) — asserted over every fixture in this file.
// ---------------------------------------------------------------------------

// Astra F10: `std::max(worst, NaN)` returns `worst`, so the retired reduction
// let a NaN normal through untouched. Every COMPONENT is checked finite first,
// and the predicate is a bool so a positive control can prove it fires.
bool unit_normals_ok(const MeshView& m, std::string& why) {
    double worst = 0.0;
    for (std::size_t v = 0; v < m.normals.size(); ++v) {
        const gp_Vec& n = m.normals[v];
        if (!std::isfinite(n.X()) || !std::isfinite(n.Y()) || !std::isfinite(n.Z())) {
            why = "vertex " + std::to_string(v) + " has a non-finite normal component";
            return false;
        }
        worst = std::max(worst, std::abs(n.Magnitude() - 1.0));
    }
    if (!(worst <= 1e-3)) {
        why = "worst unit-length deviation " + std::to_string(worst);
        return false;
    }
    why = "worst unit-length deviation " + std::to_string(worst);
    return true;
}

void check_unit_normals(const MeshView& m, const char* fixture) {
    std::string why;
    check(unit_normals_ok(m, why), std::string("NUM 1.2: every emitted normal of ") + fixture +
                                       " is finite and has unit length within [0.999, 1.001] (" +
                                       why + ")");
}

// ---------------------------------------------------------------------------
// TEST-NORMAL-05 — trimmed spline patch with a collapsed pole row.
// ---------------------------------------------------------------------------

// A 5x5 bicubic patch whose LAST v pole row collapses to one point, so S_u
// vanishes identically along v = vmax: a genuine parametric singularity that is
// not one of the analytic families.
Handle(Geom_BSplineSurface) collapsed_row_patch() {
    NCollection_Array2<gp_Pnt> poles(1, 5, 1, 5);
    const gp_Pnt tip(20.0, 30.0, 14.0);
    for (int i = 1; i <= 5; ++i) {
        for (int j = 1; j <= 5; ++j) {
            if (j == 5) {
                poles.SetValue(i, j, tip);
                continue;
            }
            const double x = 10.0 * (i - 1);
            const double y = 7.5 * (j - 1);
            poles.SetValue(i, j, gp_Pnt(x, y, 3.0 * std::sin(0.4 * x) + 2.0 * std::cos(0.3 * y)));
        }
    }
    NCollection_Array1<double> knots(1, 3);
    knots.SetValue(1, 0.0);
    knots.SetValue(2, 0.5);
    knots.SetValue(3, 1.0);
    NCollection_Array1<int> mults(1, 3);
    mults.SetValue(1, 4);
    mults.SetValue(2, 1);
    mults.SetValue(3, 4);
    return new Geom_BSplineSurface(poles, knots, knots, mults, mults, 3, 3);
}

void test_normal_05_singular_spline_patch() {
    const Handle(Geom_BSplineSurface) surface = collapsed_row_patch();
    const TopoDS_Face face =
        BRepBuilderAPI_MakeFace(surface, Precision::Confusion()).Face();
    const onecad::tess::BodyMesh bm = mesh_of(face, "fine");
    check(bm.ok, "TEST-NORMAL-05: the singular spline patch tessellates");
    check(bm.completeness.allNondegenerateFacesCovered,
          "TEST-NORMAL-05: the nondegenerate spline face is covered");
    check_unit_normals(read_mesh(bm.blob), "the singular spline patch");

    TopLoc_Location loc;
    const Handle(Poly_Triangulation) tri = BRep_Tool::Triangulation(face, loc);
    check(!tri.IsNull() && tri->HasUVNodes(),
          "TEST-NORMAL-05: the meshed patch carries UV nodes");
    if (tri.IsNull() || !tri->HasUVNodes()) return;

    const onecad::tess::FaceNormalResult fn = onecad::tess::compute_face_normals(
        face, tri, loc, face.Orientation() == TopAbs_REVERSED);
    const double vMax = surface->VKnot(surface->NbVKnots());
    const double vTol = 1e-9 * std::abs(vMax > 0.0 ? vMax : 1.0) + 1e-12;
    double regularWorst = 0.0;
    std::size_t regular = 0, singular = 0, missing = 0;
    for (std::size_t v = 0; v < fn.vertexNode.size(); ++v) {
        const int node = static_cast<int>(fn.vertexNode[v]) + 1;
        const gp_Pnt2d uv = tri->UVNode(node);
        const gp_Vec emitted(fn.normals[v * 3], fn.normals[v * 3 + 1], fn.normals[v * 3 + 2]);
        if (fn.provenance[v] == onecad::tess::NormalProvenance::Missing) ++missing;
        if (std::abs(uv.Y() - vMax) <= vTol) {
            ++singular;
            check(fn.provenance[v] == onecad::tess::NormalProvenance::TriangulationFallback ||
                      fn.provenance[v] == onecad::tess::NormalProvenance::SingularSplit,
                  "TEST-NORMAL-05: the collapsed-row node carries a bounded fallback provenance");
            check(std::abs(emitted.Magnitude() - 1.0) <= 1e-3,
                  "TEST-NORMAL-05: the singular node still carries a finite unit normal");
            continue;
        }
        // Independent oracle: S_u x S_v evaluated by the test on the SAME
        // Geom_BSplineSurface, with the orientation sign applied here.
        gp_Pnt at;
        gp_Vec du;
        gp_Vec dv;
        surface->D1(uv.X(), uv.Y(), at, du, dv);
        const gp_Vec exact = du.Crossed(dv);
        if (exact.Magnitude() <= 0.0) continue;
        ++regular;
        check(fn.provenance[v] == onecad::tess::NormalProvenance::Surface,
              "TEST-NORMAL-05: a regular spline node uses the supporting surface");
        regularWorst = std::max(regularWorst, angle_deg(emitted, exact));
    }
    check(regular > 0, "TEST-NORMAL-05: the patch has regular nodes");
    check(singular > 0, "TEST-NORMAL-05: the patch has nodes on the collapsed row");
    check(missing == 0,
          "TEST-NORMAL-05: no node of a NONDEGENERATE face is left without a normal source");
    check(regularWorst <= 0.1,
          "TEST-NORMAL-05: every regular spline node is within 0.1 deg of S_u x S_v (measured " +
              std::to_string(regularWorst) + " deg)");
}

// ---------------------------------------------------------------------------
// Provenance and counters at the analytic singularities (TEST-NORMAL-03).
// ---------------------------------------------------------------------------

onecad::tess::FaceNormalResult normals_of_first_face(const TopoDS_Shape& shape) {
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(shape, TopAbs_FACE, faces);
    const TopoDS_Face face = TopoDS::Face(faces(1));
    TopLoc_Location loc;
    const Handle(Poly_Triangulation) tri = BRep_Tool::Triangulation(face, loc);
    if (tri.IsNull()) return {};
    return onecad::tess::compute_face_normals(face, tri, loc,
                                              face.Orientation() == TopAbs_REVERSED);
}

void test_normal_03_provenance() {
    const gp_Pnt centre(0.0, 0.0, 0.0);
    const TopoDS_Shape sphere =
        BRepPrimAPI_MakeSphere(gp_Ax2(centre, gp_Dir(1, 2, 3)), 20.0).Shape();
    const onecad::tess::BodyMesh sm = mesh_of(sphere, "fine");
    const onecad::tess::FaceNormalResult sn = normals_of_first_face(sphere);
    std::size_t analytic = 0;
    for (std::size_t v = 0; v < sn.provenance.size(); ++v) {
        if (sn.provenance[v] != onecad::tess::NormalProvenance::AnalyticSingular) continue;
        ++analytic;
        const gp_Vec emitted(sn.normals[v * 3], sn.normals[v * 3 + 1], sn.normals[v * 3 + 2]);
        const gp_Vec exact(centre, sn.worldNodes[sn.vertexNode[v]]);
        check(angle_deg(emitted, exact) <= 0.1,
              "TEST-NORMAL-03: an AnalyticSingular sphere node is the exact radial direction");
    }
    check(analytic > 0,
          "TEST-NORMAL-03: the sphere poles are answered analytically, not by a facet average");
    check(sn.missingCount == 0, "TEST-NORMAL-03: no sphere node lacks a normal source");
    check(sm.completeness.allNondegenerateFacesCovered,
          "TEST-NORMAL-03: the sphere's nondegenerate faces are all covered");
    check_unit_normals(read_mesh(sm.blob), "the oblique sphere");

    const TopoDS_Shape cone = BRepPrimAPI_MakeCone(10.0, 0.0, 20.0).Shape();
    const onecad::tess::BodyMesh cm = mesh_of(cone, "fine");
    const onecad::tess::FaceNormalResult cn = normals_of_first_face(cone);
    check(cn.splitCount > 0,
          "TEST-NORMAL-03: the cone apex is SPLIT per incident fan, not averaged (splitCount " +
              std::to_string(cn.splitCount) + ")");
    std::size_t splitVertices = 0;
    for (const onecad::tess::NormalProvenance p : cn.provenance) {
        if (p == onecad::tess::NormalProvenance::SingularSplit) ++splitVertices;
    }
    check(splitVertices >= cn.splitCount,
          "TEST-NORMAL-03: every split node emits at least one SingularSplit vertex");
    check(cm.completeness.singularSplitNodes == cn.splitCount,
          "TEST-NORMAL-03: the body reports the cone's singular splits");
    check_unit_normals(read_mesh(cm.blob), "the cone");

    // The split duplicates vertices; it must not renumber triangles or ranges.
    const MeshView view = read_mesh(cm.blob);
    std::uint32_t rangeTris = 0;
    for (const auto& [first, count] : view.faceRanges) {
        check(first == rangeTris, "TEST-NORMAL-03: face ranges stay contiguous through a split");
        rangeTris += count;
    }
    check(rangeTris * 3U == view.indices.size(),
          "TEST-NORMAL-03: the face ranges still cover every emitted triangle");
    // Split copies are appended INSIDE the face's own vertex block, so each face
    // still owns one contiguous vertex range — the property WP10's local-origin
    // and per-face bbox work depends on.
    for (std::uint32_t f = 0; f < view.faceRanges.size(); ++f) {
        const std::vector<std::uint32_t> vs = face_vertices(view, f);
        if (vs.empty()) continue;
        check(vs.back() - vs.front() + 1U == vs.size(),
              "TEST-NORMAL-03: a split face still owns ONE contiguous vertex range");
    }
}

// ---------------------------------------------------------------------------
// TEST-MESH-06 — incomplete display tessellation is a FAILED tessellation
// (D11 / Astra F1), and degeneracy is decided by a face-local CERTIFICATE
// (Astra F2 / §3), never by an area threshold relative to the body.
// ---------------------------------------------------------------------------

// A bounded polynomial patch `size x size` mm carried as a face with NO bounding
// wire: BRepMesh leaves it untriangulated, and it is a perfectly valid face, so
// it is the canonical "nondegenerate face the mesher covered with nothing".
TopoDS_Face naked_patch(double size) {
    NCollection_Array2<gp_Pnt> poles(1, 2, 1, 2);
    poles.SetValue(1, 1, gp_Pnt(0.0, 0.0, 0.0));
    poles.SetValue(2, 1, gp_Pnt(size, 0.0, 0.0));
    poles.SetValue(1, 2, gp_Pnt(0.0, size, 0.0));
    poles.SetValue(2, 2, gp_Pnt(size, size, 0.0));
    TopoDS_Face face;
    BRep_Builder builder;
    builder.MakeFace(face, new Geom_BezierSurface(poles), Precision::Confusion());
    return face;
}

void test_mesh_06_completeness() {
    // A nondegenerate face the mesher cannot cover: a bounded spline surface
    // carried as a face with no bounding wire at all.
    TopoDS_Face naked;
    BRep_Builder builder;
    builder.MakeFace(naked, collapsed_row_patch(), Precision::Confusion());
    TopoDS_Compound compound;
    builder.MakeCompound(compound);
    builder.Add(compound, BRepPrimAPI_MakeBox(40.0, 30.0, 20.0).Shape());
    builder.Add(compound, naked);

    const onecad::tess::BodyMesh bm = mesh_of(compound, "fine");
    // D11 / Astra F1: the retired code published this body with `ok = true` and
    // recorded the contradiction in a field nothing read.
    check(!bm.ok,
          "TEST-MESH-06 (F1): a body with a missing NONDEGENERATE face FAILS to tessellate");
    check(bm.diagnostic.find("f:7") != std::string::npos,
          "TEST-MESH-06 (F1): the failure diagnostic NAMES the missing face (got \"" +
              bm.diagnostic + "\")");
    check(!bm.completeness.allNondegenerateFacesCovered,
          "TEST-MESH-06: a NONDEGENERATE face with no triangles cannot be called complete");
    check(bm.completeness.missingFaces.size() == 1,
          "TEST-MESH-06: the missing face is named once (named " +
              std::to_string(bm.completeness.missingFaces.size()) + ")");
    if (!bm.completeness.missingFaces.empty()) {
        check(bm.completeness.missingFaces.front() == "f:7",
              "TEST-MESH-06: the missing face is the naked spline face, by TopoKey (got " +
                  bm.completeness.missingFaces.front() + ")");
    }
    // The ids and the ordinal tables are untouched by the failure: the blob is
    // still built and every OTHER face keeps the range it earned.
    const MeshView view = read_mesh(bm.blob);
    check(view.faceRanges.size() == 7,
          "TEST-MESH-06: the missing face keeps its ordinal and a zero range");
    if (view.faceRanges.size() == 7) {
        check(view.faceRanges.back().second == 0,
              "TEST-MESH-06: the missing face's range is empty, never fabricated");
        std::uint32_t cursor = 0;
        bool contiguous = true;
        std::uint32_t covered = 0;
        for (std::size_t f = 0; f + 1 < view.faceRanges.size(); ++f) {
            if (view.faceRanges[f].first != cursor) contiguous = false;
            if (view.faceRanges[f].second == 0) contiguous = false;
            cursor += view.faceRanges[f].second;
            covered += view.faceRanges[f].second;
        }
        check(contiguous && covered * 3U == view.indices.size(),
              "TEST-MESH-06 (F1): the SURVIVING faces keep contiguous, non-empty ranges that "
              "cover every emitted triangle");
    }

    // A whole box is complete, with no missing and no degenerate faces.
    const onecad::tess::BodyMesh whole = mesh_of(BRepPrimAPI_MakeBox(40.0, 30.0, 20.0).Shape(), "fine");
    check(whole.ok && whole.completeness.allNondegenerateFacesCovered &&
              whole.completeness.missingFaces.empty(),
          "TEST-MESH-06: an intact box reports complete coverage and publishes");
    // The contract both `!ok` consumers read (the §7.6 Tessellate verb and
    // ExecutePlan's §7.2 artifact): a failure ALWAYS carries a diagnostic naming
    // the faces, and a success NEVER carries one, so neither caller can emit a
    // typed failure with nothing in it or swallow one that has something to say.
    check(whole.diagnostic.empty(),
          "TEST-MESH-06 (R1(c) MAJOR 3): a body that tessellates carries no failure diagnostic");
    check(!bm.ok && !bm.diagnostic.empty() &&
              bm.diagnostic.find(bm.completeness.missingFaces.front()) != std::string::npos,
          "TEST-MESH-06 (R1(c) MAJOR 3): a body that FAILS always carries a diagnostic naming "
          "its missing faces, which is what the Tessellate verb returns as OP_FAILED");

    // --- Astra F2 / §5: the predicate is FACE-LOCAL ------------------------
    // The retired rule was `area <= 1e-12 * bodyDiagonal^2`, so a valid
    // 0.005 x 0.005 mm face (2.5e-5 mm^2) on a 10 000 mm body was excused by a
    // 1e-4 mm^2 threshold. `classify_face_degeneracy` takes no body argument at
    // all, so the same face gets the same answer inside any body.
    const TopoDS_Face tiny =
        BRepBuilderAPI_MakeFace(gp_Pln(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1)), -0.0025, 0.0025, -0.0025,
                                0.0025)
            .Face();
    for (const double bodyMm : {1.0, 1000.0, 10000.0}) {
        BRep_Builder cb;
        TopoDS_Compound host;
        cb.MakeCompound(host);
        cb.Add(host, BRepPrimAPI_MakeBox(bodyMm, bodyMm, bodyMm).Shape());
        cb.Add(host, tiny);
        TopTools_IndexedMapOfShape hostFaces;
        TopExp::MapShapes(host, TopAbs_FACE, hostFaces);
        const onecad::tess::FaceDegeneracy d =
            onecad::tess::classify_face_degeneracy(TopoDS::Face(hostFaces(hostFaces.Extent())));
        check(!d.certified(),
              "TEST-MESH-06 (F2): a valid 0.005 x 0.005 mm face is NOT certified degenerate "
              "inside a " +
                  std::to_string(static_cast<long long>(bodyMm)) + " mm body (reason: " + d.reason +
                  ")");
    }
    // The same face, with its triangulation missing, is therefore a MISSING face
    // at every body scale, and the body fails.
    for (const double bodyMm : {1.0, 1000.0, 10000.0}) {
        BRep_Builder cb;
        TopoDS_Compound host;
        cb.MakeCompound(host);
        cb.Add(host, BRepPrimAPI_MakeBox(bodyMm, bodyMm, bodyMm).Shape());
        cb.Add(host, naked_patch(0.005));
        const onecad::tess::BodyMesh hm = mesh_of(host, "fine");
        check(!hm.ok && hm.completeness.missingFaces.size() == 1 &&
                  hm.completeness.degenerateFaces.empty(),
              "TEST-MESH-06 (F1/F2): a 0.005 x 0.005 mm uncovered face fails a " +
                  std::to_string(static_cast<long long>(bodyMm)) +
                  " mm body instead of being excused (ok=" + std::to_string(hm.ok) + ", missing=" +
                  std::to_string(hm.completeness.missingFaces.size()) + ", degenerate=" +
                  std::to_string(hm.completeness.degenerateFaces.size()) + ")");
        const MeshView hv = read_mesh(hm.blob);
        check(hv.faceRanges.size() == 7 && hv.faceRanges.back().second == 0,
              "TEST-MESH-06 (F1): the failing body still carries all seven face ordinals with a "
              "zero range on the uncovered one");
    }

    // Astra F2: all-degenerate boundary edges are EVIDENCE, never a certificate.
    // `f = 16u(1-u)v(1-v)`, `S = ((2u-1)f, (2v-1)f, f)` maps its WHOLE boundary
    // to the origin while |S_u x S_v| = 4 mm^2 at the centre.
    {
        const double g[4] = {0.0, 4.0 / 3.0, 4.0 / 3.0, 0.0};
        const double q[4] = {0.0, -4.0 / 3.0, 4.0 / 3.0, 0.0};
        NCollection_Array2<gp_Pnt> poles(1, 4, 1, 4);
        for (int i = 1; i <= 4; ++i) {
            for (int j = 1; j <= 4; ++j) {
                poles.SetValue(i, j, gp_Pnt(q[i - 1] * g[j - 1], g[i - 1] * q[j - 1],
                                            g[i - 1] * g[j - 1]));
            }
        }
        const Handle(Geom_BezierSurface) bulge = new Geom_BezierSurface(poles);
        gp_Pnt at;
        gp_Vec du;
        gp_Vec dv;
        bulge->D1(0.5, 0.5, at, du, dv);
        check(std::abs(du.Crossed(dv).Magnitude() - 4.0) <= 1e-9,
              "TEST-MESH-06 (F2): the collapsed-boundary bulge has |S_u x S_v| = 4 mm^2 at its "
              "centre (measured " + std::to_string(du.Crossed(dv).Magnitude()) + ")");
        const TopoDS_Face bulgeFace =
            BRepBuilderAPI_MakeFace(bulge, Precision::Confusion()).Face();
        std::size_t degenerateEdgeCount = 0, edgeCount = 0;
        for (TopExp_Explorer it(bulgeFace, TopAbs_EDGE); it.More(); it.Next()) {
            ++edgeCount;
            if (BRep_Tool::Degenerated(TopoDS::Edge(it.Current()))) ++degenerateEdgeCount;
        }
        const onecad::tess::FaceDegeneracy d = onecad::tess::classify_face_degeneracy(bulgeFace);
        check(!d.certified(),
              "TEST-MESH-06 (F2): the collapsed-boundary bulge is NOT certified degenerate "
              "despite " + std::to_string(degenerateEdgeCount) + " of " +
                  std::to_string(edgeCount) + " boundary edges being degenerate (reason: " +
                  d.reason + ")");
    }

    // Astra §3's counterexample to a FACE-LOCAL area rule: a 1 x 1 mm square
    // joined to a 2000 x 1e-5 mm strip has area 1.02 mm^2 and a face diagonal of
    // about 2001 mm, so `area <= tolerance * diagonal` would excuse it too.
    {
        const gp_Pnt ring[6] = {gp_Pnt(0, 0, 0),      gp_Pnt(2001, 0, 0), gp_Pnt(2001, 1e-5, 0),
                                gp_Pnt(1, 1e-5, 0),   gp_Pnt(1, 1, 0),    gp_Pnt(0, 1, 0)};
        BRepBuilderAPI_MakeWire wire;
        for (int i = 0; i < 6; ++i) {
            wire.Add(BRepBuilderAPI_MakeEdge(ring[i], ring[(i + 1) % 6]).Edge());
        }
        const TopoDS_Face strip = BRepBuilderAPI_MakeFace(wire.Wire(), /*OnlyPlane=*/true).Face();
        const onecad::tess::FaceDegeneracy d = onecad::tess::classify_face_degeneracy(strip);
        check(!d.certified(),
              "TEST-MESH-06 (§3): the 1 x 1 mm square plus 2000 x 1e-5 mm strip (1.02 mm^2) is "
              "NOT certified degenerate (reason: " + d.reason + ")");
    }

    // A retained UV domain of zero measure IS a certificate: the retained
    // parameter set lies in a line, so its image has dimension <= 1.
    {
        BRep_Builder zb;
        TopoDS_Face flat;
        const Handle(Geom_Surface) plane =
            new Geom_Plane(gp_Pln(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1)));
        zb.MakeFace(flat, plane, Precision::Confusion());
        TopoDS_Edge rail = BRepBuilderAPI_MakeEdge(gp_Pnt(0, 0, 0), gp_Pnt(1, 0, 0)).Edge();
        zb.UpdateEdge(rail, new Geom2d_Line(gp_Pnt2d(0.0, 0.0), gp_Dir2d(1.0, 0.0)), flat,
                      Precision::Confusion());
        zb.Range(rail, flat, 0.0, 1.0);
        TopoDS_Wire seam;
        zb.MakeWire(seam);
        zb.Add(seam, rail);
        zb.Add(seam, rail.Reversed());
        zb.Add(flat, seam);
        double u0 = 0.0, u1 = 0.0, v0 = 0.0, v1 = 0.0;
        BRepTools::UVBounds(flat, u0, u1, v0, v1);
        const onecad::tess::FaceDegeneracy d = onecad::tess::classify_face_degeneracy(flat);
        check(d.cls == onecad::tess::DegeneracyClass::CertifiedZero,
              "TEST-MESH-06 (§3): a face whose retained UV domain has zero v-measure is "
              "CertifiedZero (v [" + std::to_string(v0) + ", " + std::to_string(v1) +
                  "], reason: " + d.reason + ")");
    }

    // Astra §4's representation-collapse class: `ulp32(8192 mm) = 2^-10 mm`, so
    // both 8192 and 8192.0001 cast to the same float32 and a 0.0001 x 0.01 mm
    // rectangle there has NO float32 area. That is a representation failure to
    // record, never permission to call the face degenerate.
    {
        check(static_cast<float>(8192.0001) == static_cast<float>(8192.0),
              "TEST-MESH-06 (§4): float32(8192.0001 mm) == float32(8192 mm)");
        const TopoDS_Face thin =
            BRepBuilderAPI_MakeFace(gp_Pln(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1)), 8192.0, 8192.0001,
                                    0.0, 0.01)
                .Face();
        const onecad::tess::FaceDegeneracy d = onecad::tess::classify_face_degeneracy(thin);
        check(!d.certified(),
              "TEST-MESH-06 (§4): a float32-collapsing 0.0001 x 0.01 mm face at X = 8192 mm is "
              "NOT certified degenerate (reason: " + d.reason + ")");
        const onecad::tess::BodyMesh tm = mesh_of(thin, "fine");
        check(tm.completeness.degenerateFaces.empty(),
              "TEST-MESH-06 (§4): float32 collapse never puts a face in degenerateFaces");
        std::fprintf(stderr,
                     "INFO float32-collapse fixture: ok=%d triangles=%u collapsed=%u missing=%zu\n",
                     static_cast<int>(tm.ok), tm.triangle_count,
                     tm.completeness.float32CollapsedTriangles,
                     tm.completeness.missingFaces.size());
        check(tm.triangle_count == 0 || tm.completeness.float32CollapsedTriangles > 0,
              "TEST-MESH-06 (§4): the collapse is RECORDED when triangles are emitted");
    }

    // Known degenerate topology keeps a zero range with an explicit reason and
    // does NOT invalidate completeness: a sphere's two pole edges.
    const onecad::tess::BodyMesh sphere = mesh_of(BRepPrimAPI_MakeSphere(20.0).Shape(), "fine");
    check(sphere.ok && sphere.completeness.allNondegenerateFacesCovered,
          "TEST-MESH-06: a sphere's degenerate pole EDGES do not make the mesh incomplete");
    std::size_t degenerateEdges = 0;
    for (std::size_t e = 0; e < sphere.edge_classes.size(); ++e) {
        if ((sphere.edge_classes[e].flags & onecad::tess::kEdgeDegenerate) == 0) continue;
        ++degenerateEdges;
        const Section ranges = find_section(sphere.blob, 7);
        const std::uint32_t count = u32(sphere.blob, ranges.offset + e * 8U + 4U);
        check(count == 0, "TEST-MESH-06: a degenerate edge holds a zero-point range");
    }
    check(degenerateEdges == 2,
          "TEST-MESH-06: both sphere pole edges are classified degenerate (found " +
              std::to_string(degenerateEdges) + ")");
    std::fprintf(stderr, "INFO sphere display triangles at fine: %u\n", sphere.triangle_count);
}

// ---------------------------------------------------------------------------
// Edge classification (NUM §8.1/§8.2) and solid partitions (NUM §9.2).
// ---------------------------------------------------------------------------

struct FlagHistogram {
    std::size_t open = 0, hard = 0, tangent = 0, seam = 0, degenerate = 0, nonmanifold = 0,
                unknown = 0, total = 0;
};

FlagHistogram histogram(const std::vector<onecad::tess::EdgeClass>& classes, const char* fixture) {
    FlagHistogram h;
    for (const onecad::tess::EdgeClass& c : classes) {
        ++h.total;
        if (c.flags & onecad::tess::kEdgeOpen) ++h.open;
        if (c.flags & onecad::tess::kEdgeHard) ++h.hard;
        if (c.flags & onecad::tess::kEdgeTangent) ++h.tangent;
        if (c.flags & onecad::tess::kEdgeSeam) ++h.seam;
        if (c.flags & onecad::tess::kEdgeDegenerate) ++h.degenerate;
        if (c.flags & onecad::tess::kEdgeNonmanifold) ++h.nonmanifold;
        if (c.flags & onecad::tess::kEdgeUnknown) ++h.unknown;
        check((c.flags & 0xFFFFFF80U) == 0,
              std::string("NUM 8.1: ") + fixture + " uses no reserved edge flag bit");
        check(!((c.flags & onecad::tess::kEdgeTangent) && (c.flags & onecad::tess::kEdgeHard)),
              std::string("NUM 8.1: ") + fixture + " never marks an edge tangent AND hard");
        check(!((c.flags & onecad::tess::kEdgeUnknown) &&
                (c.flags & (onecad::tess::kEdgeTangent | onecad::tess::kEdgeHard |
                            onecad::tess::kEdgeDegenerate))),
              std::string("NUM 8.1: ") + fixture + " never marks an edge unknown AND definitive");
        check(c.flags != 0, std::string("NUM 8.1: ") + fixture + " classifies every edge");
        std::vector<std::uint32_t> sorted = c.faceOrdinals;
        std::sort(sorted.begin(), sorted.end());
        sorted.erase(std::unique(sorted.begin(), sorted.end()), sorted.end());
        check(sorted == c.faceOrdinals,
              std::string("NUM 9.2: ") + fixture + " lists face ordinals sorted and unique");
    }
    return h;
}

// A planar face through `wire`, flipped if needed so that its ORIENTED normal
// (plane axis composed with the orientation flag) points along `want`.
TopoDS_Face planar_face_facing(const TopoDS_Wire& wire, const gp_Dir& want) {
    TopoDS_Face face = BRepBuilderAPI_MakeFace(wire, /*OnlyPlane=*/true).Face();
    const BRepAdaptor_Surface surf(face, /*Restriction=*/false);
    gp_Vec axis(surf.Plane().Axis().Direction());
    if (face.Orientation() == TopAbs_REVERSED) axis.Reverse();
    if (axis.Dot(gp_Vec(want)) < 0.0) face = TopoDS::Face(face.Reversed());
    return face;
}

TopoDS_Wire quad_wire(const TopoDS_Edge& shared, const gp_Pnt& a, const gp_Pnt& b) {
    const TopoDS_Vertex v0 = TopExp::FirstVertex(shared, /*CumOri=*/true);
    const TopoDS_Vertex v1 = TopExp::LastVertex(shared, /*CumOri=*/true);
    BRepBuilderAPI_MakeWire wire(shared);
    wire.Add(BRepBuilderAPI_MakeEdge(v1, BRepBuilderAPI_MakeVertex(a)).Edge());
    wire.Add(BRepBuilderAPI_MakeEdge(BRepBuilderAPI_MakeVertex(a),
                                     BRepBuilderAPI_MakeVertex(b)).Edge());
    wire.Add(BRepBuilderAPI_MakeEdge(BRepBuilderAPI_MakeVertex(b), v0).Edge());
    return wire.Wire();
}

void test_edge_classification() {
    // A plain box: twelve plane/plane creases, every one hard.
    const onecad::tess::BodyMesh box = mesh_of(BRepPrimAPI_MakeBox(40.0, 30.0, 20.0).Shape(), "fine");
    const FlagHistogram boxFlags = histogram(box.edge_classes, "the box");
    check(boxFlags.total == 12 && boxFlags.hard == 12,
          "NUM 8.2: every box edge is a hard crease (" + std::to_string(boxFlags.hard) + " of " +
              std::to_string(boxFlags.total) + ")");
    for (const std::uint32_t ordinal : box.face_solid_ordinals) {
        check(ordinal == 0, "NUM 9.2: every box face belongs to the one verified closed solid");
    }

    // A cylinder: the periodic seam plus two 90 degree cap joins.
    const onecad::tess::BodyMesh cyl =
        mesh_of(BRepPrimAPI_MakeCylinder(20.0, 40.0).Shape(), "fine");
    const FlagHistogram cylFlags = histogram(cyl.edge_classes, "the cylinder");
    check(cylFlags.seam == 1, "NUM 8.2: the cylinder's periodic seam is classified seam (" +
                                  std::to_string(cylFlags.seam) + ")");
    check(cylFlags.hard == 2, "NUM 8.2: both cylinder cap joins are hard (" +
                                  std::to_string(cylFlags.hard) + ")");
    for (const onecad::tess::EdgeClass& c : cyl.edge_classes) {
        if (c.flags & onecad::tess::kEdgeSeam) {
            check(c.faceOrdinals.size() == 1,
                  "NUM 9.2: a seam names its single supporting face exactly once");
        }
        if (c.flags & onecad::tess::kEdgeHard) {
            check(c.faceOrdinals.size() == 2, "NUM 9.2: a manifold join names two face ordinals");
        }
    }

    // A tangent blend chain: the fillet/plane joins carry OCCT G1 metadata.
    const TopoDS_Shape blendBox = BRepPrimAPI_MakeBox(40.0, 30.0, 20.0).Shape();
    BRepFilletAPI_MakeFillet blend(blendBox);
    TopTools_IndexedMapOfShape blendEdges;
    TopExp::MapShapes(blendBox, TopAbs_EDGE, blendEdges);
    for (int e = 1; e <= blendEdges.Extent(); ++e) blend.Add(4.0, TopoDS::Edge(blendEdges(e)));
    blend.Build();
    if (blend.IsDone()) {
        const onecad::tess::BodyMesh filleted = mesh_of(blend.Shape(), "fine");
        const FlagHistogram f = histogram(filleted.edge_classes, "the filleted box");
        check(f.tangent > 0, "NUM 8.2: the fillet/plane boundaries are classified tangent (" +
                                 std::to_string(f.tangent) + " of " + std::to_string(f.total) + ")");
        std::fprintf(stderr,
                     "INFO filleted box edges: total=%zu tangent=%zu hard=%zu unknown=%zu "
                     "seam=%zu open=%zu degenerate=%zu nonmanifold=%zu\n",
                     f.total, f.tangent, f.hard, f.unknown, f.seam, f.open, f.degenerate,
                     f.nonmanifold);
    }

    // An open sheet: one incident face per edge.
    const TopoDS_Shape sheet =
        BRepBuilderAPI_MakeFace(gp_Pln(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1)), 0.0, 10.0, 0.0, 10.0)
            .Face();
    const onecad::tess::BodyMesh sheetMesh = mesh_of(sheet, "fine");
    const FlagHistogram sheetFlags = histogram(sheetMesh.edge_classes, "the open sheet");
    check(sheetFlags.open == 4 && sheetFlags.total == 4,
          "NUM 8.2: every edge of an open sheet is an open boundary (" +
              std::to_string(sheetFlags.open) + " of " + std::to_string(sheetFlags.total) + ")");
    for (const std::uint32_t ordinal : sheetMesh.face_solid_ordinals) {
        check(ordinal == onecad::tess::kNoSolidOrdinal,
              "NUM 9.2: a face outside every solid gets no solid ordinal");
    }

    // Three faces sharing ONE edge: nonmanifold adjacency, still visible.
    const TopoDS_Vertex v0 = BRepBuilderAPI_MakeVertex(gp_Pnt(0, 0, 0));
    const TopoDS_Vertex v1 = BRepBuilderAPI_MakeVertex(gp_Pnt(0, 0, 10));
    const TopoDS_Edge spine = BRepBuilderAPI_MakeEdge(v0, v1).Edge();
    BRep_Builder tBuilder;
    TopoDS_Compound tee;
    tBuilder.MakeCompound(tee);
    for (int k = 0; k < 3; ++k) {
        const double a = 2.0 * kPi * k / 3.0;
        const gp_Pnt far1(10.0 * std::cos(a), 10.0 * std::sin(a), 10.0);
        const gp_Pnt far0(10.0 * std::cos(a), 10.0 * std::sin(a), 0.0);
        BRepBuilderAPI_MakeWire wire(spine);
        wire.Add(BRepBuilderAPI_MakeEdge(v1, BRepBuilderAPI_MakeVertex(far1)).Edge());
        wire.Add(BRepBuilderAPI_MakeEdge(BRepBuilderAPI_MakeVertex(far1),
                                         BRepBuilderAPI_MakeVertex(far0)).Edge());
        wire.Add(BRepBuilderAPI_MakeEdge(BRepBuilderAPI_MakeVertex(far0), v0).Edge());
        tBuilder.Add(tee, BRepBuilderAPI_MakeFace(wire.Wire(), true).Face());
    }
    const onecad::tess::BodyMesh teeMesh = mesh_of(tee, "fine");
    const FlagHistogram teeFlags = histogram(teeMesh.edge_classes, "the nonmanifold fan");
    check(teeFlags.nonmanifold == 1,
          "NUM 8.2: the edge shared by three faces is nonmanifold (" +
              std::to_string(teeFlags.nonmanifold) + ")");
    for (const onecad::tess::EdgeClass& c : teeMesh.edge_classes) {
        if (c.flags & onecad::tess::kEdgeNonmanifold) {
            check(c.faceOrdinals.size() == 3,
                  "NUM 9.2: the nonmanifold edge names all three incident faces");
        }
    }

    // Two planar faces sharing one edge at a controlled dihedral. Neither carries
    // continuity metadata, so 1 deg must be MEASURED hard and 0.05 deg must stay
    // unknown-and-visible rather than be smoothed away by a sample.
    for (const double degrees : {1.0, 0.05}) {
        const double a = degrees * kPi / 180.0;
        const TopoDS_Vertex e0 = BRepBuilderAPI_MakeVertex(gp_Pnt(0, 0, 0));
        const TopoDS_Vertex e1 = BRepBuilderAPI_MakeVertex(gp_Pnt(10, 0, 0));
        const TopoDS_Edge shared = BRepBuilderAPI_MakeEdge(e0, e1).Edge();
        const TopoDS_Face flat = planar_face_facing(
            quad_wire(shared, gp_Pnt(10, 10, 0), gp_Pnt(0, 10, 0)), gp_Dir(0, 0, 1));
        const TopoDS_Face tilted = planar_face_facing(
            quad_wire(shared, gp_Pnt(10, -10 * std::cos(a), 10 * std::sin(a)),
                      gp_Pnt(0, -10 * std::cos(a), 10 * std::sin(a))),
            gp_Dir(0, std::sin(a), std::cos(a)));
        BRep_Builder db;
        TopoDS_Compound pair;
        db.MakeCompound(pair);
        db.Add(pair, flat);
        db.Add(pair, tilted);
        const onecad::tess::BodyMesh dm = mesh_of(pair, "fine");
        std::uint32_t sharedFlags = 0;
        for (const onecad::tess::EdgeClass& c : dm.edge_classes) {
            if (c.faceOrdinals.size() == 2) sharedFlags = c.flags;
        }
        if (degrees > 0.5) {
            check(sharedFlags == onecad::tess::kEdgeHard,
                  "NUM 8.2: a real 1 deg crease is hard, not tangent (flags " +
                      std::to_string(sharedFlags) + ")");
        } else {
            check(sharedFlags == onecad::tess::kEdgeUnknown,
                  "NUM 8.2: a 0.05 deg join with no continuity metadata stays UNKNOWN and visible "
                  "rather than being upgraded to tangent by a sample (flags " +
                      std::to_string(sharedFlags) + ")");
        }
    }

    // Two solids in one compound get distinct verified partitions.
    BRep_Builder cb;
    TopoDS_Compound two;
    cb.MakeCompound(two);
    cb.Add(two, BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape());
    cb.Add(two, BRepPrimAPI_MakeBox(gp_Pnt(50, 0, 0), 10.0, 10.0, 10.0).Shape());
    const onecad::tess::BodyMesh twoMesh = mesh_of(two, "fine");
    check(twoMesh.face_solid_ordinals.size() == 12,
          "NUM 9.2: both solids' faces receive an ordinal");
    std::size_t first = 0, second = 0;
    for (const std::uint32_t ordinal : twoMesh.face_solid_ordinals) {
        if (ordinal == 0) ++first;
        if (ordinal == 1) ++second;
    }
    check(first == 6 && second == 6,
          "NUM 9.2: the two closed solids get distinct local partitions (" +
              std::to_string(first) + "/" + std::to_string(second) + ")");
}

// ---------------------------------------------------------------------------
// Astra F3/F4/F5/F7/F8/F10/F11 — the derivative budget, the symmetric triangle
// predicate, the analytic pole sign, the cone apex, missing nodes and the
// valence bound. Fixtures and their numbers: docs/design/astra/
// wp09-surface-normals-break.md §5.
// ---------------------------------------------------------------------------

Handle(Poly_Triangulation) synthetic_triangulation(const std::vector<gp_Pnt>& nodes,
                                                   const std::vector<gp_Pnt2d>& uv,
                                                   const std::vector<std::array<int, 3>>& tris) {
    const bool hasUv = !uv.empty();
    Handle(Poly_Triangulation) tri = new Poly_Triangulation(
        static_cast<int>(nodes.size()), static_cast<int>(tris.size()), hasUv, false);
    for (std::size_t i = 0; i < nodes.size(); ++i) {
        tri->SetNode(static_cast<int>(i) + 1, nodes[i]);
        if (hasUv) tri->SetUVNode(static_cast<int>(i) + 1, uv[i]);
    }
    for (std::size_t t = 0; t < tris.size(); ++t) {
        tri->SetTriangle(static_cast<int>(t) + 1,
                         Poly_Triangle(tris[t][0], tris[t][1], tris[t][2]));
    }
    return tri;
}

// The triangle normal recomputed from the EMITTED winding of `fn`'s own node
// positions — an oracle independent of the classifier under test.
gp_Vec emitted_face_triangle_normal(const onecad::tess::FaceNormalResult& fn, std::size_t t) {
    const gp_Pnt& a = fn.worldNodes[fn.vertexNode[fn.triangleVertexIndices[t * 3U]]];
    const gp_Pnt& b = fn.worldNodes[fn.vertexNode[fn.triangleVertexIndices[t * 3U + 1]]];
    const gp_Pnt& c = fn.worldNodes[fn.vertexNode[fn.triangleVertexIndices[t * 3U + 2]]];
    return gp_Vec(a, b).Crossed(gp_Vec(a, c));
}

gp_Vec emitted_normal(const onecad::tess::FaceNormalResult& fn, std::size_t v) {
    return gp_Vec(fn.normals[v * 3], fn.normals[v * 3 + 1], fn.normals[v * 3 + 2]);
}

// --- F3: an absolute, representation-derived derivative budget --------------

// Astra §5's dyadic fixture, in mm: `M = 2^42`, `a = 2^-11`, `L = 2^-6`,
// `S(u,v) = (M(u-1/2)^2 + a(u-1/2), Lv, a(u-1/2))`, trimmed to
// `u in [1/2, 1/2 + 2^-24]`. The quadratic X poles `M/4 - a/2`, `-M/4`,
// `M/4 + a/2` are all exactly representable, and the standard derivative-control
// evaluation `Q0 = 2(P1-P0) = -M+a`, `Q1 = 2(P2-P1)` rounds `M+a` to `M`, so
// `Dx(1/2) = a/2` against an exact `a` — a 50 % loss on one component and an
// `atan(2) - 45 deg = 18.4349 deg` normal error that BOTH retired floors passed.
void test_f3_derivative_budget() {
    const double M = std::ldexp(1.0, 42);
    const double a = std::ldexp(1.0, -11);
    const double L = std::ldexp(1.0, -6);
    NCollection_Array2<gp_Pnt> poles(1, 3, 1, 2);
    const double xs[3] = {M / 4.0 - a / 2.0, -M / 4.0, M / 4.0 + a / 2.0};
    const double zs[3] = {-a / 2.0, 0.0, a / 2.0};
    for (int i = 1; i <= 3; ++i) {
        for (int j = 1; j <= 2; ++j) {
            poles.SetValue(i, j, gp_Pnt(xs[i - 1], (j == 1 ? 0.0 : L), zs[i - 1]));
        }
    }
    const Handle(Geom_BezierSurface) surface = new Geom_BezierSurface(poles);
    const double span = std::ldexp(1.0, -24);
    const TopoDS_Face face =
        BRepBuilderAPI_MakeFace(surface, 0.5, 0.5 + span, 0.0, 1.0, Precision::Confusion()).Face();

    // The oracle: EXACT arithmetic on the STORED control values, never a second
    // D1 call. For a quadratic Bezier, `S_u(1/2) = P2 - P0` exactly, and both
    // poles and their difference are exactly representable in binary64.
    const gp_Vec exactSu(xs[2] - xs[0], 0.0, zs[2] - zs[0]);
    const gp_Vec exactSv(0.0, L, 0.0);
    check(exactSu.X() == a && exactSu.Z() == a,
          "F3: the exact derivative from the stored poles is (a, 0, a) with a = 2^-11");
    const gp_Vec exactCross = exactSu.Crossed(exactSv);

    const BRepAdaptor_Surface surf(face, /*Restriction=*/false);
    const onecad::tess::DerivativeBudget budget = onecad::tess::derivative_error_budget(surf);
    gp_Pnt at;
    gp_Vec du;
    gp_Vec dv;
    surf.D1(0.5, 0.5, at, du, dv);
    const double err = onecad::tess::normal_angular_error_rad(budget, du, dv);
    const double deviation = angle_deg(du.Crossed(dv), exactCross);
    std::fprintf(stderr,
                 "INFO F3 dyadic fixture: budget u=%.6g mm/u v=%.6g mm/v |Su|=%.6g |Sv|=%.6g "
                 "|SuxSv|=%.6g angularBound=%.6g rad measuredDeviation=%.6f deg\n",
                 budget.uAbsoluteMm, budget.vAbsoluteMm, du.Magnitude(), dv.Magnitude(),
                 du.Crossed(dv).Magnitude(), err, deviation);
    check(budget.known, "F3: a Bezier patch carries a control-net derivative budget");
    // The binding statement: a node is either within NUM §7.4's 0.1 deg of the
    // EXACT normal, or it is reported unresolved. Nothing in between.
    check(!(err <= onecad::tess::kNormalAcceptanceRad) || deviation <= 0.1,
          "F3: a node the budget CERTIFIES is within 0.1 deg of the exact normal (bound " +
              std::to_string(err) + " rad, deviation " + std::to_string(deviation) + " deg)");
    check(!(err <= onecad::tess::kNormalAcceptanceRad),
          "F3: the dyadic fixture's normal is UNRESOLVED — the control net's roundoff enclosure "
          "cannot certify 0.1 deg (bound " + std::to_string(err) + " rad)");
}

// --- F4: the budget is a property of the representation, not of a world AABB -

// Astra §5: `S(u,v) = ((a u + (1-a) u^2) mm, v mm, 0)` on [0,1]^2 with
// `a = 1.5e-9`. The retired floor compared `|S_u| * uSpan` against
// `1e-9 * worldDiagonal`, and a unit square's AABB diagonal grows from sqrt(2) mm
// to 2 mm under a 45 degree Z rotation, so the SAME node passed before the
// rotation and failed after it.
void test_f4_rotation_invariance() {
    const double a = 1.5e-9;
    NCollection_Array2<gp_Pnt> poles(1, 3, 1, 2);
    const double xs[3] = {0.0, a / 2.0, 1.0};
    for (int i = 1; i <= 3; ++i) {
        for (int j = 1; j <= 2; ++j) {
            poles.SetValue(i, j, gp_Pnt(xs[i - 1], (j == 1 ? 0.0 : 1.0), 0.0));
        }
    }
    const Handle(Geom_BezierSurface) surface = new Geom_BezierSurface(poles);
    const TopoDS_Face flat = BRepBuilderAPI_MakeFace(surface, Precision::Confusion()).Face();
    gp_Trsf spin;
    spin.SetRotation(gp_Ax1(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1)), kPi / 4.0);
    const TopoDS_Face spun = TopoDS::Face(flat.Moved(TopLoc_Location(spin)));

    bool decision[2] = {false, false};
    double bound[2] = {0.0, 0.0};
    const TopoDS_Face faces[2] = {flat, spun};
    for (int k = 0; k < 2; ++k) {
        const BRepAdaptor_Surface surf(faces[k], /*Restriction=*/false);
        const onecad::tess::DerivativeBudget budget = onecad::tess::derivative_error_budget(surf);
        gp_Pnt at;
        gp_Vec du;
        gp_Vec dv;
        surf.D1(0.0, 0.5, at, du, dv);  // the u = 0 boundary, where |S_u| = a
        bound[k] = onecad::tess::normal_angular_error_rad(budget, du, dv);
        decision[k] = bound[k] <= onecad::tess::kNormalAcceptanceRad;
    }
    std::fprintf(stderr, "INFO F4 a=1.5e-9 patch: bound before=%.6g rad after 45 deg=%.6g rad\n",
                 bound[0], bound[1]);
    check(decision[0] == decision[1],
          "F4: the u = 0 node's decision class is identical before and after a 45 degree Z "
          "rotation (before " + std::to_string(decision[0]) + ", after " +
              std::to_string(decision[1]) + ")");
    check(decision[0],
          "F4: the a = 1.5e-9 node IS certified — its derivative is small, but the control net "
          "that produced it is not");
}

// --- F5/F9: a symmetric triangle predicate, and no silent coverage loss -----

void test_f5_triangle_symmetry() {
    const TopoDS_Face plane =
        BRepBuilderAPI_MakeFace(gp_Pln(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1)), -100.0, 100.0, -100.0,
                                100.0)
            .Face();
    // Astra F5's triangle, plus one ordinary surviving triangle. Under the
    // retired predicate the sines by cyclic permutation were 7.5e-13 / 1.5e-12 /
    // 7.5e-13 — reject / accept / reject for the SAME triangle.
    const std::vector<gp_Pnt> nodes = {gp_Pnt(0.0, 0.0, 0.0),   gp_Pnt(1.0, 0.0, 0.0),
                                       gp_Pnt(2.0, 1.5e-12, 0.0), gp_Pnt(0.0, 5.0, 0.0),
                                       gp_Pnt(5.0, 5.0, 0.0),   gp_Pnt(0.0, 10.0, 0.0)};
    std::vector<gp_Pnt2d> uv;
    for (const gp_Pnt& p : nodes) uv.emplace_back(p.X(), p.Y());

    std::uint32_t dropped[3] = {0, 0, 0};
    std::size_t kept[3] = {0, 0, 0};
    bool complete[3] = {false, false, false};
    for (int rot = 0; rot < 3; ++rot) {
        const std::array<int, 3> sliver = {1 + rot % 3, 1 + (rot + 1) % 3, 1 + (rot + 2) % 3};
        const Handle(Poly_Triangulation) tri =
            synthetic_triangulation(nodes, uv, {sliver, {4, 5, 6}});
        const onecad::tess::FaceNormalResult fn = onecad::tess::compute_face_normals(
            plane, tri, TopLoc_Location(), plane.Orientation() == TopAbs_REVERSED);
        dropped[rot] = fn.droppedTriangles;
        kept[rot] = fn.triangleVertexIndices.size() / 3U;
        complete[rot] = fn.complete;
    }
    std::fprintf(stderr, "INFO F5 cyclic orders: dropped=%u/%u/%u kept=%zu/%zu/%zu\n", dropped[0],
                 dropped[1], dropped[2], kept[0], kept[1], kept[2]);
    check(dropped[0] == dropped[1] && dropped[1] == dropped[2] && kept[0] == kept[1] &&
              kept[1] == kept[2],
          "F5: the sliver triangle is classified identically in all three cyclic vertex orders");
    check(dropped[0] == 0 && kept[0] == 2,
          "F5/F9: a positive-area triangle whose direction cannot be resolved is KEPT, not "
          "deleted (dropped " + std::to_string(dropped[0]) + ", kept " + std::to_string(kept[0]) +
              ")");
    check(complete[0] && complete[1] && complete[2],
          "F9: completeness holds only because no positive-area coverage was lost");

    // An exactly-degenerate triangle IS dropped, and dropping it cannot leave a
    // hole: it had no area to lose.
    const std::vector<gp_Pnt> flatNodes = {gp_Pnt(0, 0, 0), gp_Pnt(1, 0, 0), gp_Pnt(2, 0, 0),
                                           gp_Pnt(0, 5, 0), gp_Pnt(5, 5, 0), gp_Pnt(0, 10, 0)};
    std::vector<gp_Pnt2d> flatUv;
    for (const gp_Pnt& p : flatNodes) flatUv.emplace_back(p.X(), p.Y());
    const onecad::tess::FaceNormalResult fz = onecad::tess::compute_face_normals(
        plane, synthetic_triangulation(flatNodes, flatUv, {{1, 2, 3}, {4, 5, 6}}),
        TopLoc_Location(), plane.Orientation() == TopAbs_REVERSED);
    check(fz.droppedTriangles == 1 && fz.triangleVertexIndices.size() == 3U && fz.complete,
          "F5: a collinear triangle has an exactly zero cross product and is the only kind that "
          "is dropped (dropped " + std::to_string(fz.droppedTriangles) + ")");

    // The tie path: an isosceles triangle whose TWO longest edges are exactly
    // equal (|BC| and |CA| are both sqrt(0.5^2 + 10^2), computed from identical
    // squared operands), so the base vertex cannot be picked by length alone.
    // The remaining key is geometry-only, so every rotation must emit the same
    // normal for the same NODE, bit for bit.
    const std::vector<gp_Pnt> tied = {gp_Pnt(0.0, 0.0, 0.0), gp_Pnt(1.0, 0.0, 0.0),
                                      gp_Pnt(0.5, 10.0, 0.0)};
    std::vector<gp_Pnt2d> tiedUv;
    for (const gp_Pnt& p : tied) tiedUv.emplace_back(p.X(), p.Y());
    check(tied[1].Distance(tied[2]) == tied[2].Distance(tied[0]),
          "F5: the isosceles fixture's two longest edges are EXACTLY equal, so the base vertex "
          "is decided by the tie-break");
    std::vector<std::array<float, 3>> perNode[3];
    for (int rot = 0; rot < 3; ++rot) {
        const std::array<int, 3> order = {1 + rot % 3, 1 + (rot + 1) % 3, 1 + (rot + 2) % 3};
        const onecad::tess::FaceNormalResult fn = onecad::tess::compute_face_normals(
            plane, synthetic_triangulation(tied, tiedUv, {order}), TopLoc_Location(),
            plane.Orientation() == TopAbs_REVERSED);
        perNode[rot].assign(3, {0.0F, 0.0F, 0.0F});
        for (std::size_t v = 0; v < fn.vertexNode.size(); ++v) {
            perNode[rot][fn.vertexNode[v]] = {fn.normals[v * 3], fn.normals[v * 3 + 1],
                                              fn.normals[v * 3 + 2]};
        }
    }
    check(perNode[0] == perNode[1] && perNode[1] == perNode[2],
          "F5: an exact longest-edge tie resolves to the same base vertex in all three cyclic "
          "orders, so each node's emitted normal is bit-identical");
}

// --- F6: the tangency decision and its evidence ----------------------------

void test_f6_tangency_evidence() {
    using onecad::tess::EdgeSampleAgreement;
    EdgeSampleAgreement full;
    full.evaluable = onecad::tess::kEdgeContinuitySamples;
    full.maxDihedralRad = 0.0;
    EdgeSampleAgreement partial = full;
    partial.evaluable = onecad::tess::kEdgeContinuitySamples - 1;
    EdgeSampleAgreement crease = full;
    crease.maxDihedralRad = 1.0 * kPi / 180.0;
    EdgeSampleAgreement shallow = full;
    shallow.maxDihedralRad = 2.0 * std::asin(0.0001 / (2.0 * 0.1));  // the F6 sphere pair

    check(onecad::tess::decide_edge_continuity(true, false, full) == onecad::tess::kEdgeTangent,
          "F6: trusted G1 metadata plus THREE consistent evaluable samples is tangent");
    check(onecad::tess::decide_edge_continuity(false, true, full) == onecad::tess::kEdgeTangent,
          "F6: one shared supporting-surface handle plus three consistent samples is tangent");
    check(onecad::tess::decide_edge_continuity(true, false, partial) == onecad::tess::kEdgeUnknown,
          "F6: two of three samples evaluable stays UNKNOWN even with trusted G1 metadata — "
          "partial sampling cannot certify consistency");
    check(onecad::tess::decide_edge_continuity(false, false, full) == onecad::tess::kEdgeUnknown,
          "F6: three consistent samples alone never upgrade a join (NUM §8.2)");
    check(onecad::tess::decide_edge_continuity(true, true, crease) == onecad::tess::kEdgeHard,
          "F6: a measured 1 deg crease is hard even when the metadata claims G1");
    check(shallow.maxDihedralRad < onecad::tess::kEdgeCreaseRad,
          "F6: the r = 0.1 mm sphere pair's real dihedral (" +
              std::to_string(shallow.maxDihedralRad * 180.0 / kPi) +
              " deg) sits UNDER the 0.2 deg screen, so sampling cannot see it");
    check(onecad::tess::decide_edge_continuity(false, false, shallow) ==
              onecad::tess::kEdgeUnknown,
          "F6: two distinct spheres with tolerance-close parameters are never tangent");

    // Astra §5's polynomial join: dihedral exactly 0 at the three sample
    // parameters and 1 deg at t = 1/8. A three-sample screen therefore measures
    // a perfectly smooth join across a real 1 degree crease.
    const double k = (512.0 / 15.0) * std::tan(1.0 * kPi / 180.0);
    const auto f = [k](double t) { return k * (t - 0.25) * (t - 0.5) * (t - 0.75); };
    double worstSample = 0.0;
    for (int i = 1; i <= 3; ++i) worstSample = std::max(worstSample, std::abs(std::atan(f(0.25 * i))));
    const double excursion = std::abs(std::atan(f(0.125))) * 180.0 / kPi;
    check(worstSample <= 1e-12,
          "F6: the polynomial join is EXACTLY smooth at all three sample parameters (measured " +
              std::to_string(worstSample) + " rad)");
    check(std::abs(excursion - 1.0) <= 1e-9,
          "F6: the same join has a 1 deg crease at t = 1/8 (measured " +
              std::to_string(excursion) + " deg)");

    // The real OCCT fixture for the deleted analytic-parameter equivalence: two
    // r = 0.1 mm spheres, centres 0.0001 mm apart, fused so their intersection
    // circle is a genuine shared edge with two spherical faces.
    const TopoDS_Shape s1 =
        BRepPrimAPI_MakeSphere(gp_Ax2(gp_Pnt(-0.00005, 0, 0), gp_Dir(0, 0, 1)), 0.1).Shape();
    const TopoDS_Shape s2 =
        BRepPrimAPI_MakeSphere(gp_Ax2(gp_Pnt(0.00005, 0, 0), gp_Dir(0, 0, 1)), 0.1).Shape();
    BRepAlgoAPI_Fuse fuse(s1, s2);
    fuse.Build();
    check(fuse.IsDone(), "F6: the two near-coincident spheres fuse into a shared-edge fixture");
    if (fuse.IsDone()) {
        TopoDS_Shape fused = fuse.Shape();
        BRep_Builder tb;
        for (TopExp_Explorer it(fused, TopAbs_FACE); it.More(); it.Next()) {
            tb.UpdateFace(TopoDS::Face(it.Current()), 0.0001);  // the explicit §5 tolerance
        }
        const onecad::tess::BodyMesh fm = mesh_of(fused, "fine");
        TopTools_IndexedMapOfShape fusedFaces;
        TopExp::MapShapes(fused, TopAbs_FACE, fusedFaces);
        std::size_t tangent = 0, crossSurface = 0, sameSurface = 0;
        for (const onecad::tess::EdgeClass& c : fm.edge_classes) {
            if (c.faceOrdinals.size() != 2) continue;
            TopLoc_Location la, lb;
            const Handle(Geom_Surface) sa = BRep_Tool::Surface(
                TopoDS::Face(fusedFaces(static_cast<int>(c.faceOrdinals[0]) + 1)), la);
            const Handle(Geom_Surface) sb = BRep_Tool::Surface(
                TopoDS::Face(fusedFaces(static_cast<int>(c.faceOrdinals[1]) + 1)), lb);
            // Two faces carved out of the SAME sphere legitimately ARE tangent
            // across their shared edge; the attack is about two DISTINCT spheres
            // whose parameters merely agree to within their tolerance.
            if (sa == sb && la.IsEqual(lb)) {
                ++sameSurface;
                continue;
            }
            ++crossSurface;
            if (c.flags & onecad::tess::kEdgeTangent) ++tangent;
        }
        std::fprintf(stderr,
                     "INFO F6 sphere pair: %zu cross-surface edges (%zu same-surface), %zu "
                     "tangent\n",
                     crossSurface, sameSurface, tangent);
        check(crossSurface > 0,
              "F6: the fused spheres really do share an edge between two DISTINCT spherical "
              "surfaces");
        check(tangent == 0,
              "F6: two DISTINCT spheres whose radii and centres agree within their own tolerance "
              "are never classified tangent (" + std::to_string(tangent) + " tangent)");
    }
}

// --- F7: the sphere-pole sign is analytic, not facet-chosen ----------------

std::vector<gp_Vec> sphere_pole_normals(const TopoDS_Face& face,
                                        const Handle(Poly_Triangulation)& tri,
                                        const TopLoc_Location& loc, bool reversed,
                                        std::vector<std::uint32_t>& poleNodes) {
    const onecad::tess::FaceNormalResult fn =
        onecad::tess::compute_face_normals(face, tri, loc, reversed);
    std::vector<gp_Vec> out;
    poleNodes.clear();
    for (std::size_t v = 0; v < fn.vertexNode.size(); ++v) {
        if (fn.provenance[v] != onecad::tess::NormalProvenance::AnalyticSingular) continue;
        poleNodes.push_back(fn.vertexNode[v]);
        out.push_back(emitted_normal(fn, v));
    }
    return out;
}

void test_f7_sphere_pole_sign() {
    const double radius = 20.0;
    const gp_Ax2 axis(gp_Pnt(0, 0, 0), gp_Dir(1, 2, 3));
    const TopoDS_Shape base = BRepPrimAPI_MakeSphere(axis, radius).Shape();
    gp_Trsf mirror;
    mirror.SetMirror(gp_Ax2(gp_Pnt(0, 0, 0), gp_Dir(1, 0, 0)));

    struct Case {
        const char* name;
        TopoDS_Shape shape;
        // The material side, NOT the face orientation flag: a reflection baked by
        // BRepBuilderAPI_Transform flips the face orientations itself, so only the
        // complemented solid is genuinely inward (TEST-NORMAL-04's contract).
        bool outward;
    };
    const std::vector<Case> cases = {
        {"forward", base, true},
        {"reversed", base.Reversed(), false},
        {"baked mirror", BRepBuilderAPI_Transform(base, mirror, /*copyGeom=*/false).Shape(), true},
        {"located mirror", base.Moved(TopLoc_Location(mirror)), true},
    };
    for (const Case& c : cases) {
        BRepMesh_IncrementalMesh mesher(c.shape, 0.01, Standard_False, 0.08726646259971647,
                                        Standard_False);
        TopTools_IndexedMapOfShape faces;
        TopExp::MapShapes(c.shape, TopAbs_FACE, faces);
        const TopoDS_Face face = TopoDS::Face(faces(1));
        TopLoc_Location loc;
        const Handle(Poly_Triangulation) tri = BRep_Tool::Triangulation(face, loc);
        check(!tri.IsNull(), std::string("F7: the ") + c.name + " sphere meshes");
        if (tri.IsNull()) continue;
        const bool reversed = face.Orientation() == TopAbs_REVERSED;

        std::vector<std::uint32_t> poleNodes;
        const std::vector<gp_Vec> baseline =
            sphere_pole_normals(face, tri, loc, reversed, poleNodes);
        check(!baseline.empty(),
              std::string("F7: the ") + c.name + " sphere has analytically answered pole nodes");
        if (baseline.empty()) continue;

        // A copy whose POLE-INCIDENT triangles are wound the other way. Under the
        // retired code the facet witness sum flipped and took the supposedly
        // analytic normal 180 degrees with it, provenance unchanged.
        std::vector<gp_Pnt> nodes;
        std::vector<gp_Pnt2d> uv;
        for (int i = 1; i <= tri->NbNodes(); ++i) {
            nodes.push_back(tri->Node(i));
            uv.push_back(tri->UVNode(i));
        }
        std::vector<std::array<int, 3>> flipped;
        std::vector<std::array<int, 3>> without;
        for (int t = 1; t <= tri->NbTriangles(); ++t) {
            Standard_Integer n1 = 0, n2 = 0, n3 = 0;
            tri->Triangle(t).Get(n1, n2, n3);
            bool touchesPole = false;
            for (const std::uint32_t p : poleNodes) {
                const int node = static_cast<int>(p) + 1;
                if (n1 == node || n2 == node || n3 == node) touchesPole = true;
            }
            flipped.push_back(touchesPole ? std::array<int, 3>{n1, n3, n2}
                                          : std::array<int, 3>{n1, n2, n3});
            if (!touchesPole) without.push_back({n1, n2, n3});
        }
        std::vector<std::uint32_t> flippedNodes;
        const std::vector<gp_Vec> reversedWitness = sphere_pole_normals(
            face, synthetic_triangulation(nodes, uv, flipped), loc, reversed, flippedNodes);
        check(reversedWitness.size() == baseline.size() && flippedNodes == poleNodes,
              std::string("F7: reversing the pole facets does not change WHICH nodes are "
                          "answered analytically on the ") +
                  c.name + " sphere");
        double worst = 0.0;
        for (std::size_t i = 0; i < std::min(baseline.size(), reversedWitness.size()); ++i) {
            worst = std::max(worst, angle_deg(baseline[i], reversedWitness[i]));
        }
        check(worst <= 1e-6,
              std::string("F7: the analytic pole normal is UNCHANGED when every incident facet "
                          "is reversed on the ") +
                  c.name + " sphere (moved " + std::to_string(worst) + " deg)");

        // And with the pole facets REMOVED entirely there is no witness at all,
        // so the sign must still come from the face's own regular geometry.
        std::vector<std::uint32_t> strippedNodes;
        const std::vector<gp_Vec> stripped = sphere_pole_normals(
            face, synthetic_triangulation(nodes, uv, without), loc, reversed, strippedNodes);
        double worstStripped = 0.0;
        std::size_t matched = 0;
        for (std::size_t i = 0; i < strippedNodes.size(); ++i) {
            for (std::size_t j = 0; j < poleNodes.size(); ++j) {
                if (strippedNodes[i] != poleNodes[j]) continue;
                ++matched;
                worstStripped = std::max(worstStripped, angle_deg(stripped[i], baseline[j]));
            }
        }
        check(matched == 0 || worstStripped <= 1e-6,
              std::string("F7: removing the pole facets leaves the analytic radial sign "
                          "unchanged on the ") +
                  c.name + " sphere (moved " + std::to_string(worstStripped) + " deg)");

        // Outwardness, measured against the sphere's own centre (the origin is a
        // fixed point of every transform in this table): the sign must be the
        // physically correct one, not merely stable.
        const gp_Pnt centre(0, 0, 0);
        const bool wantOutward = c.outward;
        const onecad::tess::FaceNormalResult fn =
            onecad::tess::compute_face_normals(face, tri, loc, reversed);
        std::size_t wrong = 0;
        for (std::size_t v = 0; v < fn.vertexNode.size(); ++v) {
            if (fn.provenance[v] != onecad::tess::NormalProvenance::AnalyticSingular) continue;
            const gp_Vec radial(centre, fn.worldNodes[fn.vertexNode[v]]);
            const bool outward = emitted_normal(fn, v).Dot(radial) > 0.0;
            if (outward != wantOutward) ++wrong;
        }
        check(wrong == 0, std::string("F7: every analytic pole normal of the ") + c.name +
                              " sphere points the way the face orientation demands (" +
                              std::to_string(wrong) + " wrong)");
    }
}

// --- F8: a known cone apex is SPLIT before the 5 degree averaging rule ------

struct ConeFan {
    TopoDS_Face face;
    Handle(Poly_Triangulation) tri;
    std::size_t apexNode = 0;
    double spreadDeg = 0.0;
};

ConeFan build_cone_fan(double height, const std::vector<double>& azimuthsRad) {
    ConeFan out;
    const TopoDS_Shape cone = BRepPrimAPI_MakeCone(10.0, 0.0, height).Shape();
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(cone, TopAbs_FACE, faces);
    out.face = TopoDS::Face(faces(1));
    const BRepAdaptor_Surface surf(out.face, /*Restriction=*/false);
    const gp_Cone geom = surf.Cone();
    const double vApex = -geom.RefRadius() / std::sin(geom.SemiAngle());
    std::vector<gp_Pnt> nodes;
    std::vector<gp_Pnt2d> uv;
    nodes.push_back(surf.Value(0.0, vApex));
    uv.emplace_back(0.0, vApex);
    for (const double u : azimuthsRad) {
        nodes.push_back(surf.Value(u, 0.0));
        uv.emplace_back(u, 0.0);
    }
    std::vector<std::array<int, 3>> tris;
    for (std::size_t k = 1; k + 1 < nodes.size(); ++k) {
        tris.push_back({1, static_cast<int>(k) + 1, static_cast<int>(k) + 2});
    }
    out.tri = synthetic_triangulation(nodes, uv, tris);
    out.apexNode = 0;
    // The facet spread, measured from the node positions by the test.
    std::vector<gp_Vec> units;
    for (const std::array<int, 3>& t : tris) {
        const gp_Vec n = gp_Vec(nodes[t[0] - 1], nodes[t[1] - 1])
                             .Crossed(gp_Vec(nodes[t[0] - 1], nodes[t[2] - 1]));
        if (n.Magnitude() > 0.0) units.push_back(n.Divided(n.Magnitude()));
    }
    for (std::size_t i = 0; i < units.size(); ++i) {
        for (std::size_t j = i + 1; j < units.size(); ++j) {
            out.spreadDeg = std::max(out.spreadDeg, angle_deg(units[i], units[j]));
        }
    }
    return out;
}

void test_f8_cone_apex_split() {
    // Astra §5: cone 10 / 0 / 20 mm, apex fan at -2, 0, +2 degrees. Facet spread
    // 1.788891 degrees, so the general 5 degree rule AVERAGED it and the mean was
    // 0.894445 degrees from each facet.
    const double d2r = kPi / 180.0;
    const ConeFan narrow = build_cone_fan(20.0, {-2.0 * d2r, 0.0, 2.0 * d2r});
    check(std::abs(narrow.spreadDeg - 1.788891) <= 1e-4,
          "F8: the -2/0/+2 degree apex fan spreads 1.788891 deg, well inside the 5 deg averaging "
          "window (measured " + std::to_string(narrow.spreadDeg) + ")");
    const onecad::tess::FaceNormalResult nf = onecad::tess::compute_face_normals(
        narrow.face, narrow.tri, TopLoc_Location(),
        narrow.face.Orientation() == TopAbs_REVERSED);
    std::size_t apexVertices = 0;
    for (std::size_t v = 0; v < nf.vertexNode.size(); ++v) {
        if (nf.vertexNode[v] != narrow.apexNode) continue;
        ++apexVertices;
        check(nf.provenance[v] == onecad::tess::NormalProvenance::SingularSplit,
              "F8: every cone-apex vertex is SingularSplit, never an averaged fallback");
    }
    check(nf.splitCount == 1 && apexVertices == 2,
          "F8: the apex node is split once per incident facet (splitCount " +
              std::to_string(nf.splitCount) + ", apex vertices " + std::to_string(apexVertices) +
              ")");
    // Each apex copy carries its OWN facet's normal, recomputed by the test from
    // the emitted winding.
    double worst = 0.0;
    for (std::size_t t = 0; t * 3U < nf.triangleVertexIndices.size(); ++t) {
        const gp_Vec facet = emitted_face_triangle_normal(nf, t);
        for (int corner = 0; corner < 3; ++corner) {
            const std::uint32_t v = nf.triangleVertexIndices[t * 3U + corner];
            if (nf.vertexNode[v] != narrow.apexNode) continue;
            worst = std::max(worst, angle_deg(emitted_normal(nf, v), facet));
        }
    }
    check(worst <= 1e-6,
          "F8: each apex vertex's normal IS its own incident facet normal (worst " +
              std::to_string(worst) + " deg)");

    // The shallow cone: R = 10 mm, H = 10 tan(2 deg) = 0.349207695 mm, 72 equal
    // sectors. Maximum facet spread about 4.003808 degrees — under 5, so the
    // retired rule averaged the whole fan into an axial normal that no facet has.
    std::vector<double> sectors;
    for (int k = 0; k <= 72; ++k) sectors.push_back(2.0 * kPi * k / 72.0);
    const ConeFan shallow = build_cone_fan(10.0 * std::tan(2.0 * d2r), sectors);
    check(std::abs(shallow.spreadDeg - 4.003808) <= 1e-3,
          "F8: the 72-sector shallow cone fan spreads 4.003808 deg (measured " +
              std::to_string(shallow.spreadDeg) + ")");
    const onecad::tess::FaceNormalResult sf = onecad::tess::compute_face_normals(
        shallow.face, shallow.tri, TopLoc_Location(),
        shallow.face.Orientation() == TopAbs_REVERSED);
    const gp_Vec coneAxis(BRepAdaptor_Surface(shallow.face, false).Cone().Axis().Direction());
    std::size_t averaged = 0, split = 0;
    for (std::size_t v = 0; v < sf.vertexNode.size(); ++v) {
        if (sf.vertexNode[v] != shallow.apexNode) continue;
        if (sf.provenance[v] == onecad::tess::NormalProvenance::SingularSplit) ++split;
        if (angle_deg(emitted_normal(sf, v), coneAxis) <= 0.5 ||
            angle_deg(emitted_normal(sf, v), coneAxis.Reversed()) <= 0.5) {
            ++averaged;
        }
    }
    check(split == 72 && averaged == 0,
          "F8: the shallow cone's apex is split per facet with no invented axial normal (split " +
              std::to_string(split) + ", axial " + std::to_string(averaged) + ")");
}

// --- F10: no arbitrary +Z, and the finite check fires ----------------------

void test_f10_missing_nodes_and_nan() {
    // A triangulation with NO UV nodes, so the supporting surface is not
    // consulted and a node's only evidence is its incident triangles. Node 6
    // touches nothing but an exactly-degenerate triangle, so it has no normal
    // source at all — the retired code still emitted world +Z for it.
    const TopoDS_Face plane =
        BRepBuilderAPI_MakeFace(gp_Pln(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1)), -100.0, 100.0, -100.0,
                                100.0)
            .Face();
    const std::vector<gp_Pnt> nodes = {gp_Pnt(0, 0, 0),  gp_Pnt(10, 0, 5), gp_Pnt(0, 10, 5),
                                       gp_Pnt(20, 0, 0), gp_Pnt(21, 0, 0), gp_Pnt(22, 0, 0)};
    const onecad::tess::FaceNormalResult fn = onecad::tess::compute_face_normals(
        plane, synthetic_triangulation(nodes, {}, {{1, 2, 3}, {4, 5, 6}}), TopLoc_Location(),
        plane.Orientation() == TopAbs_REVERSED);
    check(fn.missingCount == 3,
          "F10: the three nodes of the collinear triangle have no normal source (missing " +
              std::to_string(fn.missingCount) + ")");
    check(fn.vertexNode.size() == 3 && fn.nodeRemap[5] == onecad::tess::FaceNormalResult::kNoVertex,
          "F10: a node with no normal source is NOT emitted, so no arbitrary +Z reaches the wire "
          "(emitted " + std::to_string(fn.vertexNode.size()) + " of 6 nodes)");
    std::size_t plusZ = 0;
    for (std::size_t v = 0; v < fn.vertexNode.size(); ++v) {
        const gp_Vec n = emitted_normal(fn, v);
        if (n.X() == 0.0F && n.Y() == 0.0F && n.Z() == 1.0F) ++plusZ;
    }
    check(plusZ == 0, "F10: no emitted normal is the world +Z default");
    check(fn.complete,
          "F10: the face is still complete — the dropped triangle was certified zero-area");

    // The positive control for the finite check: `std::max(worst, NaN)` returns
    // `worst`, so the retired reduction could not see this.
    MeshView poisoned;
    poisoned.normals.emplace_back(0.0, 0.0, 1.0);
    poisoned.normals.emplace_back(std::nan(""), 0.0, 0.0);
    std::string why;
    check(!unit_normals_ok(poisoned, why),
          "F10: a NaN component makes the unit-normal check FAIL (" + why + ")");
    MeshView clean;
    clean.normals.emplace_back(0.0, 0.0, 1.0);
    check(unit_normals_ok(clean, why), "F10: a clean normal array still passes");
}

// --- F11: the pairwise spread scan is bounded ------------------------------

void test_f11_valence_bound() {
    const Handle(Geom_BSplineSurface) surface = collapsed_row_patch();
    const TopoDS_Face face = BRepBuilderAPI_MakeFace(surface, Precision::Confusion()).Face();
    const double vMax = surface->VKnot(surface->NbVKnots());
    constexpr int kValence = 10000;
    std::vector<gp_Pnt> nodes;
    std::vector<gp_Pnt2d> uv;
    nodes.push_back(surface->Value(0.5, vMax));
    uv.emplace_back(0.5, vMax);
    for (int k = 0; k <= kValence; ++k) {
        const double u = static_cast<double>(k) / static_cast<double>(kValence);
        const double v = vMax * 0.7;
        nodes.push_back(surface->Value(u, v));
        uv.emplace_back(u, v);
    }
    std::vector<std::array<int, 3>> tris;
    for (int k = 1; k <= kValence; ++k) tris.push_back({1, k + 1, k + 2});
    const Handle(Poly_Triangulation) tri = synthetic_triangulation(nodes, uv, tris);

    const auto started = std::chrono::steady_clock::now();
    const onecad::tess::FaceNormalResult a = onecad::tess::compute_face_normals(
        face, tri, TopLoc_Location(), face.Orientation() == TopAbs_REVERSED);
    const double elapsedMs =
        std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - started)
            .count();
    const onecad::tess::FaceNormalResult b = onecad::tess::compute_face_normals(
        face, tri, TopLoc_Location(), face.Orientation() == TopAbs_REVERSED);
    std::fprintf(stderr, "INFO F11 valence %d fan: %.3f ms, %zu emitted vertices\n", kValence,
                 elapsedMs, a.vertexNode.size());
    check(a.triangleVertexIndices == b.triangleVertexIndices && a.provenance == b.provenance,
          "F11: a valence-10000 fan produces repeatable indices and provenance");
    std::size_t apexVertices = 0;
    bool allSplit = true;
    for (std::size_t v = 0; v < a.vertexNode.size(); ++v) {
        if (a.vertexNode[v] != 0U) continue;
        ++apexVertices;
        if (a.provenance[v] != onecad::tess::NormalProvenance::SingularSplit) allSplit = false;
    }
    check(a.splitCount == 1 && apexVertices == static_cast<std::size_t>(kValence) && allSplit,
          "F11: the node is split deterministically above kMaxSpreadValence without the "
          "pairwise scan (apex vertices " + std::to_string(apexVertices) + ")");
}

// ---------------------------------------------------------------------------
// R1(c) fix round — BLOCKER 2 (a surface with no enclosure must keep its own
// normal), MAJOR 5 (the derivative budget is per PARAMETER unit, so it is
// reparameterization invariant), MINOR 6 (an edge between two such faces can
// still be classified from trusted metadata).
// ---------------------------------------------------------------------------

// A degree-3 B-spline profile in the XZ plane: nothing about it is elementary,
// so a prism over it is a `GeomAbs_SurfaceOfExtrusion` and a revolve is a
// `GeomAbs_SurfaceOfRevolution` — the two families this file has no derivative
// enclosure for (verified against OCCT 8.0.1).
Handle(Geom_BSplineCurve) spline_profile() {
    NCollection_Array1<gp_Pnt> poles(1, 5);
    poles.SetValue(1, gp_Pnt(10.0, 0.0, 0.0));
    poles.SetValue(2, gp_Pnt(14.0, 0.0, 5.0));
    poles.SetValue(3, gp_Pnt(9.0, 0.0, 12.0));
    poles.SetValue(4, gp_Pnt(15.0, 0.0, 18.0));
    poles.SetValue(5, gp_Pnt(11.0, 0.0, 25.0));
    NCollection_Array1<double> knots(1, 3);
    knots.SetValue(1, 0.0);
    knots.SetValue(2, 0.5);
    knots.SetValue(3, 1.0);
    NCollection_Array1<int> mults(1, 3);
    mults.SetValue(1, 4);
    mults.SetValue(2, 1);
    mults.SetValue(3, 4);
    return new Geom_BSplineCurve(poles, knots, mults, 3);
}

struct ProvenanceCounts {
    std::size_t surface = 0, analytic = 0, fallback = 0, split = 0, unresolved = 0, missing = 0;
    std::size_t emitted = 0;
    int nodes = 0;
    std::string text() const {
        return "surface=" + std::to_string(surface) + " analytic=" + std::to_string(analytic) +
               " fallback=" + std::to_string(fallback) + " split=" + std::to_string(split) +
               " unresolved=" + std::to_string(unresolved) + " missing=" +
               std::to_string(missing) + " emitted=" + std::to_string(emitted) + " nodes=" +
               std::to_string(nodes);
    }
};

ProvenanceCounts provenance_of_face(const TopoDS_Shape& shape, const char* lod) {
    ProvenanceCounts c;
    const onecad::tess::BodyMesh bm = mesh_of(shape, lod);
    (void)bm;
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(shape, TopAbs_FACE, faces);
    if (faces.Extent() < 1) return c;
    const TopoDS_Face face = TopoDS::Face(faces(1));
    TopLoc_Location loc;
    const Handle(Poly_Triangulation) tri = BRep_Tool::Triangulation(face, loc);
    if (tri.IsNull()) return c;
    c.nodes = tri->NbNodes();
    const onecad::tess::FaceNormalResult fn = onecad::tess::compute_face_normals(
        face, tri, loc, face.Orientation() == TopAbs_REVERSED);
    c.emitted = fn.vertexNode.size();
    for (const onecad::tess::NormalProvenance p : fn.provenance) {
        switch (p) {
            case onecad::tess::NormalProvenance::Surface: ++c.surface; break;
            case onecad::tess::NormalProvenance::AnalyticSingular: ++c.analytic; break;
            case onecad::tess::NormalProvenance::TriangulationFallback: ++c.fallback; break;
            case onecad::tess::NormalProvenance::SingularSplit: ++c.split; break;
            case onecad::tess::NormalProvenance::Unresolved: ++c.unresolved; break;
            case onecad::tess::NormalProvenance::Missing: ++c.missing; break;
        }
    }
    return c;
}

void test_r1c_unbounded_surface_keeps_its_own_normal() {
    const TopoDS_Edge profile = BRepBuilderAPI_MakeEdge(spline_profile()).Edge();
    const TopoDS_Shape prism = BRepPrimAPI_MakePrism(profile, gp_Vec(0.0, 30.0, 0.0)).Shape();
    const TopoDS_Shape revolve =
        BRepPrimAPI_MakeRevol(profile, gp_Ax1(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1))).Shape();

    const std::pair<const char*, TopoDS_Shape> swept[2] = {{"prism", prism}, {"revolve", revolve}};
    for (const auto& [name, shape] : swept) {
        for (const char* lod : {"coarse", "fine"}) {
            const ProvenanceCounts c = provenance_of_face(shape, lod);
            std::fprintf(stderr, "INFO R1C %s %s: %s\n", name, lod, c.text().c_str());
            check(c.nodes > 0, std::string("BLOCKER 2: the ") + name + " at " + lod + " meshes");
            // Every node of a swept B-spline is REGULAR: the profile has no cusp
            // and the sweep is nondegenerate, so nothing here is a singularity.
            // The facet ladder had no business seeing any of them.
            check(c.split == 0,
                  std::string("BLOCKER 2: no regular node of the ") + name + " at " + lod +
                      " is singular-split (" + c.text() + ")");
            check(c.fallback == 0 && c.missing == 0,
                  std::string("BLOCKER 2: no regular node of the ") + name + " at " + lod +
                      " falls back to its facets (" + c.text() + ")");
            check(c.surface + c.unresolved == static_cast<std::size_t>(c.nodes),
                  std::string("BLOCKER 2: every node of the ") + name + " at " + lod +
                      " keeps its own supporting-surface normal, certified or Unresolved (" +
                      c.text() + ")");
            check(c.emitted == static_cast<std::size_t>(c.nodes),
                  std::string("BLOCKER 2: the ") + name + " at " + lod +
                      " emits exactly one vertex per node — no split inflation (" + c.text() + ")");
        }
    }
    // Provenance-count coverage for `Unresolved` itself: it must actually occur,
    // or the whole rung is untested.
    const ProvenanceCounts c = provenance_of_face(prism, "fine");
    check(c.unresolved > 0,
          "BLOCKER 2: `Unresolved` is a REACHED rung — a surface of extrusion has no derivative "
          "enclosure, so its nodes carry an uncertified surface normal (" + c.text() + ")");
}

// MAJOR 5: the budget is an error on the DERIVATIVE, i.e. mm per unit of the
// parameter, so scaling the knot vector by 1/lambda must leave every node's
// Surface/Unresolved decision alone. The retired form charged a POSITION
// enclosure (mm) and so grew strictly more permissive as the parameterization
// was compressed.
void test_r1c_budget_is_reparameterization_invariant() {
    constexpr double kLambda = 100.0;
    NCollection_Array2<gp_Pnt> poles(1, 4, 1, 4);
    for (int i = 1; i <= 4; ++i) {
        for (int j = 1; j <= 4; ++j) {
            const double x = 1000.0 + 3.0 * (i - 1);
            const double y = 1000.0 + 3.0 * (j - 1);
            poles.SetValue(i, j, gp_Pnt(x, y, 0.4 * (i - 1) * (j - 1)));
        }
    }
    NCollection_Array1<double> knots(1, 2);
    NCollection_Array1<int> mults(1, 2);
    mults.SetValue(1, 4);
    mults.SetValue(2, 4);

    // Two surfaces that are the SAME map up to `u -> u / lambda`: identical
    // poles, knot vectors [0, 0.01] and [0, 0.01/lambda].
    double bound[2] = {0.0, 0.0};
    bool decision[2] = {false, false};
    const double extents[2] = {0.01, 0.01 / kLambda};
    for (int k = 0; k < 2; ++k) {
        knots.SetValue(1, 0.0);
        knots.SetValue(2, extents[k]);
        const Handle(Geom_BSplineSurface) surface =
            new Geom_BSplineSurface(poles, knots, knots, mults, mults, 3, 3);
        const TopoDS_Face face =
            BRepBuilderAPI_MakeFace(surface, Precision::Confusion()).Face();
        const BRepAdaptor_Surface surf(face, /*Restriction=*/false);
        const onecad::tess::DerivativeBudget budget = onecad::tess::derivative_error_budget(surf);
        gp_Pnt at;
        gp_Vec du;
        gp_Vec dv;
        surf.D1(0.5 * extents[k], 0.5 * extents[k], at, du, dv);
        bound[k] = onecad::tess::normal_angular_error_rad(budget, du, dv);
        decision[k] = bound[k] <= onecad::tess::kNormalAcceptanceRad;
        if (k == 0) {
            std::fprintf(stderr,
                         "INFO R1C budget units: uAbs=%.6g mm/u vAbs=%.6g mm/v |Su|=%.6g "
                         "|Sv|=%.6g\n",
                         budget.uAbsoluteMm, budget.vAbsoluteMm, du.Magnitude(), dv.Magnitude());
        }
    }
    std::fprintf(stderr, "INFO R1C reparameterization: bound at u-extent 0.01 = %.6g rad, at "
                         "0.0001 = %.6g rad\n",
                 bound[0], bound[1]);
    check(decision[0] == decision[1],
          "MAJOR 5: compressing the knot vector by 1/100 does not change the Surface/Unresolved "
          "decision (before " + std::to_string(decision[0]) + ", after " +
              std::to_string(decision[1]) + ")");
    const double ratio = bound[1] > 0.0 ? bound[0] / bound[1] : 0.0;
    check(bound[0] > 0.0 && std::isfinite(bound[0]) && std::abs(ratio - 1.0) <= 1e-6,
          "MAJOR 5: the angular bound itself is reparameterization INVARIANT (ratio " +
              std::to_string(ratio) + ")");
}

// MINOR 6 (follows from BLOCKER 2): two surfaces of extrusion sharing one edge.
// While every sample on them was rejected as unevaluable the join could only be
// `unknown`; with the samples restored, trusted G1 metadata classifies it
// tangent and its absence still leaves it visible.
void test_r1c_edge_between_unbounded_faces() {
    const Handle(Geom_BSplineCurve) curve = spline_profile();
    const double split = 0.5;
    const TopoDS_Edge lower =
        BRepBuilderAPI_MakeEdge(new Geom_TrimmedCurve(curve, curve->FirstParameter(), split))
            .Edge();
    const TopoDS_Edge upper =
        BRepBuilderAPI_MakeEdge(new Geom_TrimmedCurve(curve, split, curve->LastParameter()))
            .Edge();
    BRepBuilderAPI_MakeWire wire(lower);
    wire.Add(upper);
    check(wire.IsDone(), "MINOR 6: the split B-spline profile builds one wire");
    if (!wire.IsDone()) return;
    const TopoDS_Shape sheet = BRepPrimAPI_MakePrism(wire.Wire(), gp_Vec(0.0, 30.0, 0.0)).Shape();

    TopTools_IndexedMapOfShape faces;
    TopTools_IndexedMapOfShape edges;
    TopExp::MapShapes(sheet, TopAbs_FACE, faces);
    TopExp::MapShapes(sheet, TopAbs_EDGE, edges);
    check(faces.Extent() == 2,
          "MINOR 6: the prism over a two-edge wire is two faces (" +
              std::to_string(faces.Extent()) + ")");
    if (faces.Extent() != 2) return;
    for (int f = 1; f <= 2; ++f) {
        const BRepAdaptor_Surface s(TopoDS::Face(faces(f)), /*Restriction=*/false);
        check(s.GetType() == GeomAbs_SurfaceOfExtrusion,
              "MINOR 6: both faces are surfaces of extrusion, the family with no enclosure");
    }

    const auto shared_flags = [&](const TopoDS_Shape& shape) {
        TopTools_IndexedMapOfShape ff;
        TopTools_IndexedMapOfShape ee;
        TopExp::MapShapes(shape, TopAbs_FACE, ff);
        TopExp::MapShapes(shape, TopAbs_EDGE, ee);
        const std::vector<onecad::tess::EdgeClass> classes =
            onecad::tess::classify_edges(shape, ff, ee);
        for (const onecad::tess::EdgeClass& c : classes) {
            if (c.faceOrdinals.size() == 2) return c.flags;
        }
        return std::uint32_t{0};
    };

    // The shared edge, its registered continuity and the two faces' supporting
    // surfaces — read here so the assertions below name real evidence.
    TopoDS_Edge shared;
    TopoDS_Face fa;
    TopoDS_Face fb;
    TopTools_IndexedDataMapOfShapeListOfShape edgeFaces;
    TopExp::MapShapesAndAncestors(sheet, TopAbs_EDGE, TopAbs_FACE, edgeFaces);
    for (int e = 1; e <= edgeFaces.Extent(); ++e) {
        std::vector<TopoDS_Face> incident;
        for (const TopoDS_Shape& sh : edgeFaces.FindFromIndex(e)) {
            const TopoDS_Face face = TopoDS::Face(sh);
            bool seen = false;
            for (const TopoDS_Face& have : incident) seen = seen || have.IsSame(face);
            if (!seen) incident.push_back(face);
        }
        if (incident.size() != 2) continue;
        shared = TopoDS::Edge(edgeFaces.FindKey(e));
        fa = incident[0];
        fb = incident[1];
    }
    check(!shared.IsNull(), "MINOR 6: the two extrusion faces share exactly one edge");
    if (shared.IsNull()) return;
    TopLoc_Location la;
    TopLoc_Location lb;
    const Handle(Geom_Surface) sa = BRep_Tool::Surface(fa, la);
    const Handle(Geom_Surface) sb = BRep_Tool::Surface(fb, lb);
    check(!(sa == sb),
          "MINOR 6: the two faces are carried by DISTINCT surfaces, so the shared-handle "
          "shortcut cannot answer this join");
    const GeomAbs_Shape registered = BRep_Tool::Continuity(shared, fa, fb);
    check(registered >= GeomAbs_G1,
          "MINOR 6: OCCT's prism builder registers G1-or-better continuity across the smooth "
          "profile join (got " + std::to_string(static_cast<int>(registered)) + ")");

    // With trusted metadata and three evaluable samples the join is tangent.
    // While every sample on a surface of extrusion was rejected as unevaluable it
    // could only ever be `unknown`, whatever the metadata said.
    check(shared_flags(sheet) == onecad::tess::kEdgeTangent,
          "MINOR 6: a join between two surfaces of extrusion with trusted continuity metadata "
          "is TANGENT (flags " + std::to_string(shared_flags(sheet)) + ")");

    // Strip the metadata and the same geometry falls back to UNKNOWN: NUM §8.2's
    // "sample-only results remain unknown, visible" is unaffected by the fix.
    BRep_Builder cb;
    cb.Continuity(shared, fa, fb, GeomAbs_C0);
    check(BRep_Tool::Continuity(shared, fa, fb) == GeomAbs_C0,
          "MINOR 6: the continuity metadata really was stripped");
    check(shared_flags(sheet) == onecad::tess::kEdgeUnknown,
          "MINOR 6: without trusted metadata the SAME join stays unknown and visible — three "
          "consistent samples never upgrade it (flags " + std::to_string(shared_flags(sheet)) +
              ")");
}

void report_normal_cost() {
    const TopoDS_Shape torus = BRepPrimAPI_MakeTorus(30.0, 10.0).Shape();
    BRepMesh_IncrementalMesh mesher(torus, 0.03, Standard_False, 0.08726646259971647,
                                    Standard_False);
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(torus, TopAbs_FACE, faces);
    const TopoDS_Face face = TopoDS::Face(faces(1));
    TopLoc_Location loc;
    const Handle(Poly_Triangulation) tri = BRep_Tool::Triangulation(face, loc);
    if (tri.IsNull()) return;
    const auto started = std::chrono::steady_clock::now();
    const onecad::tess::FaceNormalResult fn = onecad::tess::compute_face_normals(
        face, tri, loc, face.Orientation() == TopAbs_REVERSED);
    const double ms =
        std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - started)
            .count();
    std::fprintf(stderr, "INFO torus compute_face_normals: %.3f ms over %d nodes (%zu emitted)\n",
                 ms, tri->NbNodes(), fn.vertexNode.size());
}

// ---------------------------------------------------------------------------
// Determinism: identical input must give identical bytes (Invariant 5).
// ---------------------------------------------------------------------------

void test_determinism() {
    for (const char* lod : {"coarse", "fine"}) {
        const onecad::tess::BodyMesh a =
            mesh_of(BRepPrimAPI_MakeCone(10.0, 0.0, 20.0).Shape(), lod);
        const onecad::tess::BodyMesh b =
            mesh_of(BRepPrimAPI_MakeCone(10.0, 0.0, 20.0).Shape(), lod);
        check(a.blob == b.blob,
              std::string("Invariant 5: two meshes of the same cone at ") + lod +
                  " are byte-identical");
    }
}

}  // namespace

int main() {
    test_normal_01_box_both_orientations();
    test_normal_02_cylinder_seam_and_fillet_chain();
    test_normal_03_sphere_torus_cone();
    test_normal_04_transforms_and_mirror();
    test_normal_03_provenance();
    test_normal_05_singular_spline_patch();
    test_normal_06_density_independence();
    test_mesh_06_completeness();
    test_f3_derivative_budget();
    test_f4_rotation_invariance();
    test_f5_triangle_symmetry();
    test_f6_tangency_evidence();
    test_f7_sphere_pole_sign();
    test_f8_cone_apex_split();
    test_f10_missing_nodes_and_nan();
    test_f11_valence_bound();
    test_r1c_unbounded_surface_keeps_its_own_normal();
    test_r1c_budget_is_reparameterization_invariant();
    test_r1c_edge_between_unbounded_faces();
    test_edge_classification();
    test_determinism();
    report_normal_cost();
    if (g_failures == 0) std::fprintf(stderr, "surface_normals: OK\n");
    return g_failures;
}
