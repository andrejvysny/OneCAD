// ClassifyElement.cpp — see ClassifyElement.h. SCHEMA §7.5 `ClassifyElement`.
#include "session/ClassifyElement.h"

#include <string>

#include <BRepAdaptor_Curve.hxx>
#include <BRepAdaptor_Surface.hxx>
#include <BRepTools.hxx>
#include <BRep_Tool.hxx>
#include <GeomAbs_CurveType.hxx>
#include <GeomAbs_SurfaceType.hxx>
#include <Geom_Surface.hxx>
#include <Standard_Failure.hxx>
#include <TopAbs_ShapeEnum.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Ax1.hxx>
#include <gp_Circ.hxx>
#include <gp_Cylinder.hxx>
#include <gp_Dir.hxx>
#include <gp_Pln.hxx>
#include <gp_Pnt.hxx>
#include <gp_Vec.hxx>

#include "elementmap/ElementMapPartition.h"
#include "kernel/topology/CoplanarFacePatch.h"

namespace onecad::session {

using nlohmann::json;
using protocol::Envelope;
namespace em = onecad::elementmap;
namespace kt = onecad::core::modeling;

namespace {

std::string get_str(const json& o, const char* key) {
    if (o.is_object() && o.contains(key) && o[key].is_string()) return o[key].get<std::string>();
    return {};
}

json vec3_json(double x, double y, double z) { return json::array({x, y, z}); }

std::string surface_type_name(GeomAbs_SurfaceType t) {
    switch (t) {
        case GeomAbs_Plane: return "plane";
        case GeomAbs_Cylinder: return "cylinder";
        case GeomAbs_Cone: return "cone";
        case GeomAbs_Sphere: return "sphere";
        case GeomAbs_Torus: return "torus";
        default: return "other";
    }
}

std::string curve_type_name(GeomAbs_CurveType t) {
    switch (t) {
        case GeomAbs_Line: return "line";
        case GeomAbs_Circle: return "circle";
        case GeomAbs_Ellipse: return "ellipse";
        default: return "other";
    }
}

// The face or edge this request names, within the CURRENT head. Same two
// addressing forms as ProjectFaceBoundary (SCHEMA §7.6): `elementId` via the
// partition, or `{bodyId, topoKey}` directly. A miss on either is
// `present:false`, never an error.
struct SeedLookup {
    bool present = false;
    TopoDS_Shape shape;
    TopoDS_Shape body_shape;  // the owning body — SCHEMA §7.5 `sidedness` needs the instance
};

SeedLookup resolve_seed(const BodyStore& bodies, const em::ElementMapPartition& part,
                        const json& args) {
    SeedLookup out;
    std::string body_id = get_str(args, "bodyId");
    std::string topo_key = get_str(args, "topoKey");

    const std::string element_id = get_str(args, "elementId");
    if (!element_id.empty()) {
        const em::PartitionEntry* entry = part.find(element_id);
        if (entry == nullptr) return out;  // absent id -> present:false
        body_id = entry->body_id;
        topo_key = entry->topo_key;
    }

    if (body_id.empty() || topo_key.empty()) return out;
    const BodyRecord* rec = bodies.get(body_id);
    if (rec == nullptr) return out;

    const TopoDS_Shape sub = em::ElementMapPartition::shape_for_topokey(rec->geom, topo_key);
    if (sub.IsNull()) return out;  // stale topoKey -> present:false

    out.present = true;
    out.shape = sub;
    out.body_shape = rec->geom;
    return out;
}

// Populates `frame` for the surface/curve kinds a mate solver cares about
// (plane, cylinder, circle). Any other kind returns no frame at all — the
// caller still learns `surfaceType`/`curveType`, just not a seatable frame.
json classify_face(const TopoDS_Face& face, json& out) {
    BRepAdaptor_Surface surface(face, true);
    const GeomAbs_SurfaceType type = surface.GetType();
    out["surfaceType"] = surface_type_name(type);

    if (type == GeomAbs_Plane) {
        gp_Pln plane;
        gp_Dir normal;
        if (kt::CoplanarFacePatch::planarFacePlaneAndNormal(face, plane, normal)) {
            const gp_Pnt origin = plane.Location();
            out["frame"] = json{{"origin", vec3_json(origin.X(), origin.Y(), origin.Z())},
                                {"normal", vec3_json(normal.X(), normal.Y(), normal.Z())}};
        }
    } else if (type == GeomAbs_Cylinder) {
        const gp_Cylinder cyl = surface.Cylinder();
        const gp_Ax1 axis = cyl.Axis();
        const gp_Pnt origin = axis.Location();
        const gp_Dir dir = axis.Direction();
        out["frame"] = json{{"origin", vec3_json(origin.X(), origin.Y(), origin.Z())},
                            {"axis", vec3_json(dir.X(), dir.Y(), dir.Z())},
                            {"radius", cyl.Radius()}};
    }
    return out;
}

// The point the §10 v4 `outward` evaluation uses for a FACE: its UV-box centre
// on the underlying surface. Recomputed here (rather than read off the
// descriptor's `center`, which is an area centroid and sits ON the axis for a
// full cylinder) so `sidedness` measures `outward` and `radial` at the SAME
// point, as SCHEMA §7.5 requires.
bool face_uv_centre(const TopoDS_Face& face, gp_Pnt& out) {
    try {
        const Handle(Geom_Surface) surface = BRep_Tool::Surface(face);
        if (surface.IsNull()) return false;
        double umin = 0.0, umax = 0.0, vmin = 0.0, vmax = 0.0;
        BRepTools::UVBounds(face, umin, umax, vmin, vmax);
        out = surface->Value(0.5 * (umin + umax), 0.5 * (vmin + vmax));
        return true;
    } catch (const Standard_Failure&) {
        return false;
    }
}

// SCHEMA §7.5 `sidedness`: `outward · radial > 0` ⇒ the solid lies INSIDE the
// cylinder (a boss or shaft — `"pin"`), else outside it (`"hole"`). Empty when
// it cannot be measured, which is an ABSENT key, never a guessed side.
std::string cylinder_sidedness(const TopoDS_Face& face, const TopoDS_Shape& body_shape,
                               const gp_Ax1& axis) {
    const em::km::ElementDescriptor d = em::ElementMapPartition::describe(face, body_shape);
    if (!d.hasOutward) return {};
    gp_Pnt at;
    if (!face_uv_centre(face, at)) return {};
    // `radial` runs from the evaluation point's projection on the axis LINE to
    // the point itself.
    const gp_Vec to_point(axis.Location(), at);
    const gp_Vec along(axis.Direction());
    const gp_Vec radial = to_point - along * to_point.Dot(along);
    if (radial.Magnitude() <= 1e-9) return {};
    return gp_Vec(d.outward).Dot(radial) > 0.0 ? "pin" : "hole";
}

json classify_edge(const TopoDS_Edge& edge, json& out) {
    BRepAdaptor_Curve curve(edge);
    const GeomAbs_CurveType type = curve.GetType();
    out["curveType"] = curve_type_name(type);

    if (type == GeomAbs_Circle) {
        const gp_Circ circ = curve.Circle();
        const gp_Pnt origin = circ.Location();
        const gp_Dir dir = circ.Axis().Direction();
        out["frame"] = json{{"origin", vec3_json(origin.X(), origin.Y(), origin.Z())},
                            {"axis", vec3_json(dir.X(), dir.Y(), dir.Z())},
                            {"radius", circ.Radius()}};
    } else if (type == GeomAbs_Line) {
        const gp_Lin line = curve.Line();
        const gp_Pnt origin = line.Location();
        const gp_Dir dir = line.Direction();
        out["frame"] = json{{"origin", vec3_json(origin.X(), origin.Y(), origin.Z())},
                            {"axis", vec3_json(dir.X(), dir.Y(), dir.Z())}};
    }
    return out;
}

}  // namespace

json classify_shape(const TopoDS_Shape& shape) {
    json out;
    if (shape.IsNull()) {
        out["kind"] = "other";
        return out;
    }
    const TopAbs_ShapeEnum shape_type = shape.ShapeType();
    if (shape_type == TopAbs_FACE) {
        out["kind"] = "face";
        classify_face(TopoDS::Face(shape), out);
    } else if (shape_type == TopAbs_EDGE) {
        out["kind"] = "edge";
        classify_edge(TopoDS::Edge(shape), out);
    } else {
        out["kind"] = "other";
    }
    return out;
}

json classify_shape_in_body(const TopoDS_Shape& shape, const TopoDS_Shape& body_shape) {
    json out = classify_shape(shape);
    if (shape.IsNull() || body_shape.IsNull() || shape.ShapeType() != TopAbs_FACE) return out;
    if (!out.contains("frame") || out.value("surfaceType", std::string()) != "cylinder") return out;
    const TopoDS_Face face = TopoDS::Face(shape);
    BRepAdaptor_Surface surface(face, true);
    if (surface.GetType() != GeomAbs_Cylinder) return out;
    const std::string sidedness = cylinder_sidedness(face, body_shape, surface.Cylinder().Axis());
    if (!sidedness.empty()) out["frame"]["sidedness"] = sidedness;
    return out;
}

Envelope handle_classify_element(Session& session, const Envelope& req) {
    const json& args = req.args;
    const BodyStore bodies = session.bodies_copy();
    const em::ElementMapPartition part = session.partition_copy();

    const SeedLookup seed = resolve_seed(bodies, part, args);
    if (!seed.present) {
        return Envelope::ok_response(req.id, json{{"present", false}});
    }

    json out = classify_shape_in_body(seed.shape, seed.body_shape);
    out["present"] = true;
    return Envelope::ok_response(req.id, std::move(out));
}

}  // namespace onecad::session
