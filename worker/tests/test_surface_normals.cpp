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
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <functional>
#include <string>
#include <utility>
#include <vector>

#include <BRepAdaptor_Surface.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakeVertex.hxx>
#include <BRepBuilderAPI_MakeWire.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <BRepFilletAPI_MakeFillet.hxx>
#include <BRepMesh_IncrementalMesh.hxx>
#include <BRep_Builder.hxx>
#include <BRep_Tool.hxx>
#include <Geom_BSplineSurface.hxx>
#include <NCollection_Array1.hxx>
#include <NCollection_Array2.hxx>
#include <Poly_Triangulation.hxx>
#include <Precision.hxx>
#include <TopoDS_Compound.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Vertex.hxx>
#include <gp_Pln.hxx>
#include <gp_Pnt2d.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakeCone.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <BRepPrimAPI_MakeSphere.hxx>
#include <BRepPrimAPI_MakeTorus.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopLoc_Location.hxx>
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

void check_unit_normals(const MeshView& m, const char* fixture) {
    double worst = 0.0;
    for (std::size_t v = 0; v < m.positions.size(); ++v) {
        worst = std::max(worst, std::abs(m.normals[v].Magnitude() - 1.0));
    }
    check(worst <= 1e-3, std::string("NUM 1.2: every emitted normal of ") + fixture +
                             " has unit length within [0.999, 1.001] (worst deviation " +
                             std::to_string(worst) + ")");
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
// TEST-MESH-06 — incomplete display tessellation is diagnostic.
// ---------------------------------------------------------------------------

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
    check(bm.ok, "TEST-MESH-06: the body still publishes when a face is missing");
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
    const MeshView view = read_mesh(bm.blob);
    check(view.faceRanges.size() == 7,
          "TEST-MESH-06: the missing face keeps its ordinal and a zero range");
    if (view.faceRanges.size() == 7) {
        check(view.faceRanges.back().second == 0,
              "TEST-MESH-06: the missing face's range is empty, never fabricated");
    }

    // A whole box is complete, with no missing and no degenerate faces.
    const onecad::tess::BodyMesh whole = mesh_of(BRepPrimAPI_MakeBox(40.0, 30.0, 20.0).Shape(), "fine");
    check(whole.completeness.allNondegenerateFacesCovered && whole.completeness.missingFaces.empty(),
          "TEST-MESH-06: an intact box reports complete coverage");

    // The degeneracy predicate itself, both arms, against a body 100 mm across.
    TopTools_IndexedMapOfShape boxFaces;
    TopExp::MapShapes(BRepPrimAPI_MakeBox(40.0, 30.0, 20.0).Shape(), TopAbs_FACE, boxFaces);
    check(!onecad::tess::face_is_degenerate(TopoDS::Face(boxFaces(1)), 100.0),
          "TEST-MESH-06: a 30x20 mm box face is NOT degenerate");
    TopTools_IndexedMapOfShape slivers;
    TopExp::MapShapes(BRepPrimAPI_MakeBox(1e-3, 1e-3, 1e-3).Shape(), TopAbs_FACE, slivers);
    check(onecad::tess::face_is_degenerate(TopoDS::Face(slivers(1)), 1e6),
          "TEST-MESH-06: a 1e-3 mm face of a 1e6 mm body IS degenerate");
    check(!onecad::tess::face_is_degenerate(TopoDS::Face(slivers(1)), 1.0),
          "TEST-MESH-06: the same face on a 1 mm body is NOT degenerate (the floor is relative)");

    // Known degenerate topology keeps a zero range with an explicit reason and
    // does NOT invalidate completeness: a sphere's two pole edges.
    const onecad::tess::BodyMesh sphere = mesh_of(BRepPrimAPI_MakeSphere(20.0).Shape(), "fine");
    check(sphere.completeness.allNondegenerateFacesCovered,
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
    test_edge_classification();
    test_determinism();
    if (g_failures == 0) std::fprintf(stderr, "surface_normals: OK\n");
    return g_failures;
}
