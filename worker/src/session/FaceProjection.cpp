// FaceProjection.cpp — see FaceProjection.h. SCHEMA §7.6 `ProjectFaceBoundary`.
#include "session/FaceProjection.h"

#include <string>
#include <vector>

#include <cmath>
#include <optional>

#include <BRepBuilderAPI_MakeVertex.hxx>
#include <BRepClass_FaceClassifier.hxx>
#include <BRepExtrema_DistShapeShape.hxx>
#include <BRepGProp.hxx>
#include <BRep_Tool.hxx>
#include <GProp_GProps.hxx>
#include <Standard_Failure.hxx>
#include <TopAbs_ShapeEnum.hxx>
#include <TopAbs_State.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Ax3.hxx>
#include <gp_Dir.hxx>
#include <gp_Pln.hxx>
#include <gp_Pnt.hxx>
#include <gp_Vec.hxx>

#include "elementmap/ElementMapPartition.h"
#include "kernel/topology/CoplanarFacePatch.h"
#include "sketch/FaceBoundaryProjector.h"
#include "util/Log.h"

namespace onecad::session {

using nlohmann::json;
using protocol::Envelope;
namespace em = onecad::elementmap;
namespace fb = onecad::core::sketch;
namespace kt = onecad::core::modeling;

namespace {

Envelope fail(const Envelope& req, const char* code, const std::string& message) {
    // The verb mutates nothing, so every failure leaves the session intact.
    return Envelope::error_response(req.id,
                                    protocol::ErrorInfo{code, message, /*retriable=*/false});
}

Envelope absent(const Envelope& req) {
    return Envelope::ok_response(req.id, json{{"present", false}});
}

std::string get_str(const json& o, const char* key) {
    if (o.is_object() && o.contains(key) && o[key].is_string()) return o[key].get<std::string>();
    return {};
}

bool read_vec3(const json& o, const char* key, double out[3]) {
    if (!o.is_object() || !o.contains(key) || !o[key].is_array() || o[key].size() != 3) {
        return false;
    }
    for (int i = 0; i < 3; ++i) {
        if (!o[key][static_cast<std::size_t>(i)].is_number()) return false;
        out[i] = o[key][static_cast<std::size_t>(i)].get<double>();
    }
    return true;
}

double positive_or(const json& o, const char* key, double fallback) {
    if (o.is_object() && o.contains(key) && o[key].is_number()) {
        const double v = o[key].get<double>();
        if (v > 0.0) return v;
    }
    return fallback;
}

json vec3_json(double x, double y, double z) {
    return json::array({x, y, z});
}

std::string point_ref(std::size_t index) {
    return "p" + std::to_string(index);
}

// `IN`/`ON` at the face's OWN tolerance — the same face-owned classification the
// mate seat uses (`ComponentOp.cpp`). An unmeasurable point is never "on".
bool on_face(const TopoDS_Face& face, const gp_Pnt& p) {
    if (!std::isfinite(p.X()) || !std::isfinite(p.Y()) || !std::isfinite(p.Z())) return false;
    try {
        BRepClass_FaceClassifier classifier(face, p, BRep_Tool::Tolerance(face));
        const TopAbs_State state = classifier.State();
        return state == TopAbs_IN || state == TopAbs_ON;
    } catch (const Standard_Failure&) {
        return false;
    }
}

// The point of `face` nearest `p` — `ComponentOp.cpp::nearest_point_on_face`, kept
// local because op/session executors stay self-contained rather than reaching into
// each other. `std::nullopt` when OCCT cannot measure it.
std::optional<gp_Pnt> nearest_point_on_face(const TopoDS_Face& face, const gp_Pnt& p) {
    try {
        BRepExtrema_DistShapeShape dist(face, BRepBuilderAPI_MakeVertex(p).Vertex());
        if (dist.IsDone() && dist.NbSolution() > 0) return dist.PointOnShape1(1);
    } catch (const Standard_Failure&) {
        // fall through — no claim
    }
    return std::nullopt;
}

// SCHEMA §7.6 `exact.anchor` — a point of the seed face that is genuinely ON it, so
// a caller can freeze it as a resolution-ladder anchor.
//
// WHY THIS IS NOT `exact.origin`. The `gp_Pln` LOCATION of a face's surface is a
// property of the PLANE, not of the face's boundary, and for an imported STEP body
// it routinely lies far outside the face it came from (measured 2026-09-15,
// `src-tauri/tests/step_import_gate.rs` PHASE 4: location (0,0,10) for a cap centred
// at (−5,215,10)). An anchor there scores 0 on the ladder's `anchor` feature, so a
// correct, unambiguous host binds at 0.75 and never clears the 0.85 auto-bind bar —
// and re-picking the face freezes the same location again, so the miss is permanent.
//
// Three candidates, in order, each accepted only if it CLASSIFIES on the face:
//   1. the element descriptor's centre — what the ladder scores the anchor against,
//      so a coincident anchor is the strongest evidence available;
//   2. the point of the face nearest the plane location — always on the face when
//      OCCT can measure it (on its boundary if the location projects outside);
//   3. the face's surface centre of mass.
// Absent when none classifies (an annulus whose every candidate falls in its hole).
// The caller then keeps whatever it did before — this field is purely additive.
std::optional<gp_Pnt> face_anchor_point(const TopoDS_Face& face, const TopoDS_Shape& body_shape,
                                        const gp_Pln& plane) {
    const gp_Pnt center = em::ElementMapPartition::describe(face, body_shape).center;
    if (on_face(face, center)) return center;

    if (const std::optional<gp_Pnt> nearest = nearest_point_on_face(face, plane.Location())) {
        if (on_face(face, *nearest)) return nearest;
    }

    try {
        GProp_GProps props;
        BRepGProp::SurfaceProperties(face, props);
        if (props.Mass() > 0.0) {
            // The face is planar (the caller refused it otherwise), so its surface
            // centre of mass already lies in `plane`; projecting is a no-op that
            // costs nothing and cannot drift off it.
            const gp_Pnt com = props.CentreOfMass();
            const gp_Pnt on_plane = com.Translated(
                gp_Vec(plane.Axis().Direction()) *
                -gp_Vec(plane.Location(), com).Dot(gp_Vec(plane.Axis().Direction())));
            if (on_face(face, on_plane)) return on_plane;
        }
    } catch (const Standard_Failure&) {
        // fall through — no claim
    }
    return std::nullopt;
}

// The face this request names, within the CURRENT head. Two addressing forms
// (SCHEMA §7.6), both ending at a real TopoDS_Face:
//   * `elementId` — partition entry → (bodyId, topoKey) → shape_for_topokey. This
//     rung is what QueryElement's elementId branch does NOT have: that one answers
//     with the partition descriptor and never reaches a shape.
//   * `{bodyId, topoKey}` — shape_for_topokey directly.
// A miss on either is `present:false` (stale/absent), never an error.
struct SeedLookup {
    bool present = false;
    bool wrong_kind = false;
    TopoDS_Shape body_shape;
    TopoDS_Shape face_shape;
};

SeedLookup resolve_seed(const BodyStore& bodies, const em::ElementMapPartition& part,
                        const json& args) {
    SeedLookup out;
    std::string body_id = get_str(args, "bodyId");
    std::string topo_key = get_str(args, "topoKey");

    const std::string element_id = get_str(args, "elementId");
    if (!element_id.empty()) {
        const em::PartitionEntry* entry = part.find(element_id);
        if (entry == nullptr) return out;  // absent id → present:false
        body_id = entry->body_id;
        topo_key = entry->topo_key;
    }

    if (body_id.empty() || topo_key.empty()) return out;
    const BodyRecord* rec = bodies.get(body_id);
    if (rec == nullptr) return out;

    const TopoDS_Shape sub = em::ElementMapPartition::shape_for_topokey(rec->geom, topo_key);
    if (sub.IsNull()) return out;  // stale topoKey → present:false

    if (sub.ShapeType() != TopAbs_FACE) {
        out.wrong_kind = true;
        return out;
    }
    out.present = true;
    out.body_shape = rec->geom;
    out.face_shape = sub;
    return out;
}

}  // namespace

Envelope handle_project_face_boundary(Session& session, const Envelope& req) {
    const json& args = req.args;
    const BodyStore bodies = session.bodies_copy();
    const em::ElementMapPartition part = session.partition_copy();

    const SeedLookup seed = resolve_seed(bodies, part, args);
    if (seed.wrong_kind) {
        return fail(req, "OP_FAILED",
                    "ProjectFaceBoundary: the referenced element is not a face");
    }
    if (!seed.present) {
        return absent(req);
    }

    const TopoDS_Face seed_face = TopoDS::Face(seed.face_shape);

    // `exact` — the kernel gp_Pln origin + the orientation-corrected unit normal of
    // the SEED face. Always returned when present, in both modes.
    gp_Pln seed_plane;
    gp_Dir seed_normal;
    if (!kt::CoplanarFacePatch::planarFacePlaneAndNormal(seed_face, seed_plane, seed_normal)) {
        WLOG_WARN("ProjectFaceBoundary: refusing non-planar seed face");
        return fail(req, "OP_FAILED", "ProjectFaceBoundary: seed face is not planar");
    }
    const gp_Pnt exact_origin = seed_plane.Location();
    json exact = json{
        {"origin", vec3_json(exact_origin.X(), exact_origin.Y(), exact_origin.Z())},
        {"normal", vec3_json(seed_normal.X(), seed_normal.Y(), seed_normal.Z())},
    };
    // SCHEMA §7.6 `exact.anchor` (OPTIONAL) — a point ON the seed face, for a caller
    // that freezes an identity anchor. Returned in BOTH modes, absent when no such
    // point can be produced.
    if (const std::optional<gp_Pnt> anchor =
            face_anchor_point(seed_face, seed.body_shape, seed_plane)) {
        exact["anchor"] = vec3_json(anchor->X(), anchor->Y(), anchor->Z());
    }

    const bool frame_only = args.is_object() && args.contains("frameOnly") &&
                            args["frameOnly"].is_boolean() && args["frameOnly"].get<bool>();
    if (frame_only) {
        // frameOnly ignores `plane` and `scope` — it exists so Rust can BUILD the
        // basis it will send on the second call.
        return Envelope::ok_response(req.id, json{{"present", true}, {"exact", std::move(exact)}});
    }

    if (!args.is_object() || !args.contains("plane") || !args["plane"].is_object()) {
        return fail(req, "PROTOCOL_ERROR",
                    "ProjectFaceBoundary: args.plane is required unless frameOnly is true");
    }
    const json& plane_json = args["plane"];
    double origin[3] = {0, 0, 0};
    double x_axis[3] = {0, 0, 0};
    double y_axis[3] = {0, 0, 0};
    double normal[3] = {0, 0, 0};
    if (!read_vec3(plane_json, "origin", origin) || !read_vec3(plane_json, "xAxis", x_axis) ||
        !read_vec3(plane_json, "yAxis", y_axis) || !read_vec3(plane_json, "normal", normal)) {
        return fail(req, "PROTOCOL_ERROR",
                    "ProjectFaceBoundary: args.plane needs origin/xAxis/yAxis/normal "
                    "as 3-number arrays");
    }

    const std::string scope_arg = get_str(args, "scope");
    const std::string scope = scope_arg.empty() ? std::string("coplanarBody") : scope_arg;
    if (scope != "faceOnly" && scope != "coplanarBody") {
        return fail(req, "PROTOCOL_ERROR",
                    "ProjectFaceBoundary: scope must be faceOnly or coplanarBody");
    }

    const json opts_json = (args.contains("options") && args["options"].is_object())
                               ? args["options"]
                               : json::object();
    fb::FaceBoundaryProjector::Options projector_options;
    projector_options.pointMergeTolerance =
        positive_or(opts_json, "pointMergeTolerance", projector_options.pointMergeTolerance);
    if (opts_json.contains("fallbackSegmentsPerCurve") &&
        opts_json["fallbackSegmentsPerCurve"].is_number_integer()) {
        const int segments = opts_json["fallbackSegmentsPerCurve"].get<int>();
        if (segments >= 2) projector_options.fallbackSegmentsPerCurve = segments;
    }
    kt::CoplanarFacePatch::Options patch_options;
    patch_options.normalDotTolerance =
        positive_or(opts_json, "normalDotTolerance", patch_options.normalDotTolerance);
    patch_options.planeDistanceTolerance =
        positive_or(opts_json, "planeDistanceTolerance", patch_options.planeDistanceTolerance);

    fb::SketchPlane plane;
    plane.origin = {origin[0], origin[1], origin[2]};
    plane.xAxis = {x_axis[0], x_axis[1], x_axis[2]};
    plane.yAxis = {y_axis[0], y_axis[1], y_axis[2]};
    plane.normal = {normal[0], normal[1], normal[2]};

    // `coplanarBody` tests every face of the body against the SUPPLIED plane (not
    // the seed's own), so a caller-chosen frame is what decides membership.
    std::vector<TopoDS_Face> companions;
    if (scope == "coplanarBody") {
        gp_Pln supplied;
        try {
            supplied = gp_Pln(gp_Pnt(origin[0], origin[1], origin[2]),
                              gp_Dir(normal[0], normal[1], normal[2]));
        } catch (...) {
            return fail(req, "PROTOCOL_ERROR",
                        "ProjectFaceBoundary: args.plane.normal is degenerate");
        }
        for (const TopoDS_Face& face :
             kt::CoplanarFacePatch::collectCoplanarFaces(seed.body_shape, supplied,
                                                         patch_options)) {
            if (!face.IsNull() && !face.IsSame(seed_face)) companions.push_back(face);
        }
    }

    const fb::FaceBoundaryProjector::Result projection =
        fb::FaceBoundaryProjector::project(seed_face, companions, plane, projector_options);
    if (!projection.ok) {
        return fail(req, "OP_FAILED", "ProjectFaceBoundary: " + projection.errorMessage);
    }

    json points = json::array();
    for (std::size_t i = 0; i < projection.points.size(); ++i) {
        points.push_back(json{{"ref", point_ref(i)},
                              {"at", json::array({projection.points[i].u, projection.points[i].v})}});
    }

    json entities = json::array();
    for (const fb::FaceBoundaryProjector::Entity& e : projection.entities) {
        switch (e.kind) {
            case fb::FaceBoundaryProjector::EntityKind::Line:
                entities.push_back(json{{"type", "Line"},
                                        {"p0Ref", point_ref(static_cast<std::size_t>(e.p0))},
                                        {"p1Ref", point_ref(static_cast<std::size_t>(e.p1))}});
                break;
            case fb::FaceBoundaryProjector::EntityKind::Circle:
                entities.push_back(json{{"type", "Circle"},
                                        {"centerRef", point_ref(static_cast<std::size_t>(e.center))},
                                        {"radius", e.radius}});
                break;
            case fb::FaceBoundaryProjector::EntityKind::Arc:
                entities.push_back(json{{"type", "Arc"},
                                        {"centerRef", point_ref(static_cast<std::size_t>(e.center))},
                                        {"radius", e.radius},
                                        {"startAngle", e.startAngle},
                                        {"endAngle", e.endAngle},
                                        {"ccw", e.ccw}});
                break;
        }
    }

    return Envelope::ok_response(
        req.id, json{{"present", true},
                     {"exact", std::move(exact)},
                     {"hasClosedBoundary", projection.hasClosedBoundary},
                     {"faceCount", static_cast<int>(companions.size()) + 1},
                     {"points", std::move(points)},
                     {"entities", std::move(entities)}});
}

}  // namespace onecad::session
