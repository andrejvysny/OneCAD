#include "ops/GeneratorSlots.h"

#include <algorithm>
#include <cmath>
#include <map>
#include <set>

#include <BRepAdaptor_Surface.hxx>
#include <BRepBuilderAPI_MakeVertex.hxx>
#include <BRepExtrema_DistShapeShape.hxx>
#include <BRepGProp.hxx>
#include <BRepLProp_SLProps.hxx>
#include <BRepBndLib.hxx>
#include <Bnd_Box.hxx>
#include <GProp_GProps.hxx>
#include <GeomAbs_SurfaceType.hxx>
#include <TopAbs_ShapeEnum.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedDataMapOfShapeListOfShape.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Face.hxx>
#include <gp_Vec.hxx>

namespace onecad::ops {
namespace {

// A face whose normal is at least this aligned with the extrude direction is an
// end cap; anything flatter is a wall. 0.5 (60 degrees) leaves a wide margin on
// both sides: walls are perpendicular up to draft, caps are parallel.
constexpr double kCapAlignment = 0.5;

struct FaceClass {
    std::set<std::string> labels;  // profile entities whose midpoint lies on the face
    double normal_dot = 0.0;       // |n . dir|
};

bool near_face(const Bnd_Box& box, const gp_Pnt& p) { return !box.IsOut(p); }

// |n . dir| at the face; for a planar face the plane normal, otherwise the
// surface normal at `uv` (the support point of the closest label).
double normal_alignment(const TopoDS_Face& face, const gp_Dir& dir, double u, double v) {
    BRepAdaptor_Surface surface(face);
    if (surface.GetType() == GeomAbs_Plane)
        return std::abs(surface.Plane().Axis().Direction().Dot(dir));
    BRepLProp_SLProps props(surface, u, v, 1, 1e-9);
    if (!props.IsNormalDefined()) return 0.0;
    return std::abs(props.Normal().Dot(dir));
}

// Representative (u,v) for a face with no label: parameter-box centre.
void centre_uv(const TopoDS_Face& face, double& u, double& v) {
    BRepAdaptor_Surface surface(face);
    u = 0.5 * (surface.FirstUParameter() + surface.LastUParameter());
    v = 0.5 * (surface.FirstVParameter() + surface.LastVParameter());
}

FaceClass classify_face(const TopoDS_Face& face, const std::vector<ProfileEdgeLabel>& labels,
                        const gp_Dir& dir, double tol) {
    FaceClass out;
    Bnd_Box box;
    BRepBndLib::Add(face, box);
    box.Enlarge(tol);
    bool have_uv = false;
    double u = 0.0, v = 0.0;
    for (const ProfileEdgeLabel& label : labels) {
        if (!near_face(box, label.midpoint)) continue;
        const TopoDS_Vertex vertex = BRepBuilderAPI_MakeVertex(label.midpoint).Vertex();
        BRepExtrema_DistShapeShape dist(vertex, face);
        if (!dist.IsDone() || dist.NbSolution() < 1 || dist.Value() > tol) continue;
        out.labels.insert(label.baseEntityId);
        if (!have_uv) {
            have_uv = dist.SupportTypeShape2(1) == BRepExtrema_IsInFace;
            if (have_uv) dist.ParOnFaceS2(1, u, v);
        }
    }
    if (!have_uv) centre_uv(face, u, v);
    out.normal_dot = normal_alignment(face, dir, u, v);
    return out;
}

std::string cap_slot(const TopoDS_Face& face, const gp_Pnt& origin, const gp_Dir& dir,
                     double tol) {
    GProp_GProps props;
    BRepGProp::SurfaceProperties(face, props);
    const double d = gp_Vec(origin, props.CentreOfMass()).Dot(gp_Vec(dir));
    if (std::abs(d) <= tol) return "cap:0";
    return d > 0.0 ? "cap:+" : "cap:-";
}

// Drop every slot held by more than one key. Pass 1 counts, pass 2 filters, so
// the surviving order is the input's order.
std::vector<std::pair<TopoDS_Shape, std::string>> drop_duplicates(
    const std::vector<std::pair<TopoDS_Shape, std::string>>& claims) {
    std::map<std::string, int> held;
    for (const auto& claim : claims) ++held[claim.second];
    std::vector<std::pair<TopoDS_Shape, std::string>> unique;
    for (const auto& claim : claims)
        if (held[claim.second] == 1) unique.push_back(claim);
    return unique;
}

std::vector<std::pair<TopoDS_Shape, std::string>> face_claims(
    const TopoDS_Shape& tool, const std::vector<ProfileEdgeLabel>& labels,
    const gp_Pnt& origin, const gp_Dir& dir, double tol) {
    std::vector<std::pair<TopoDS_Shape, std::string>> claims;
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(tool, TopAbs_FACE, faces);
    for (int i = 1; i <= faces.Extent(); ++i) {
        const TopoDS_Face face = TopoDS::Face(faces(i));
        const FaceClass cls = classify_face(face, labels, dir, tol);
        if (cls.normal_dot >= kCapAlignment) {
            claims.emplace_back(face, cap_slot(face, origin, dir, tol));
        } else if (cls.labels.size() == 1) {
            claims.emplace_back(face, "side:" + *cls.labels.begin());
        }
    }
    return drop_duplicates(claims);
}

std::vector<std::pair<TopoDS_Shape, std::string>> edge_claims(
    const TopoDS_Shape& tool, const GeneratorSlotMap& face_slots) {
    std::vector<std::pair<TopoDS_Shape, std::string>> claims;
    TopTools_IndexedDataMapOfShapeListOfShape ancestors;
    TopExp::MapShapesAndAncestors(tool, TopAbs_EDGE, TopAbs_FACE, ancestors);
    for (int i = 1; i <= ancestors.Extent(); ++i) {
        std::vector<TopoDS_Shape> distinct;
        for (const TopoDS_Shape& face : ancestors(i)) {
            if (std::none_of(distinct.begin(), distinct.end(),
                             [&](const TopoDS_Shape& seen) { return seen.IsSame(face); }))
                distinct.push_back(face);
        }
        if (distinct.size() != 2) continue;  // seam or non-manifold: no claim
        const int a = face_slots.FindIndex(distinct[0]);
        const int b = face_slots.FindIndex(distinct[1]);
        if (a == 0 || b == 0) continue;
        const std::string& sa = face_slots.FindFromIndex(a);
        const std::string& sb = face_slots.FindFromIndex(b);
        claims.emplace_back(ancestors.FindKey(i),
                            "edge:" + std::min(sa, sb) + "|" + std::max(sa, sb));
    }
    return drop_duplicates(claims);
}

}  // namespace

GeneratorSlotMap attribute_extrude_slots(const TopoDS_Shape& tool,
                                         const std::vector<ProfileEdgeLabel>& labels,
                                         const gp_Pnt& plane_origin, const gp_Dir& extrude_dir,
                                         double tol) {
    GeneratorSlotMap slots;
    // No labels means the profile was not a sketch region: nothing to anchor to.
    if (tool.IsNull() || labels.empty()) return slots;
    for (const auto& claim : face_claims(tool, labels, plane_origin, extrude_dir, tol))
        slots.Add(claim.first, claim.second);
    GeneratorSlotMap edges;
    for (const auto& claim : edge_claims(tool, slots)) edges.Add(claim.first, claim.second);
    for (int i = 1; i <= edges.Extent(); ++i) slots.Add(edges.FindKey(i), edges.FindFromIndex(i));
    return slots;
}

}  // namespace onecad::ops
