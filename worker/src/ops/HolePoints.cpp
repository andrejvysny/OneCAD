// HolePoints.cpp — see HolePoints.h.
#include "ops/HolePoints.h"

#include <algorithm>
#include <cmath>
#include <set>

#include <BRepClass_FaceClassifier.hxx>
#include <BRepGProp.hxx>
#include <BRep_Tool.hxx>
#include <GProp_GProps.hxx>
#include <GeomAPI_ProjectPointOnSurf.hxx>
#include <Geom_Surface.hxx>
#include <Standard_Failure.hxx>
#include <TopAbs_State.hxx>
#include <TopoDS.hxx>
#include <gp_Pnt2d.hxx>
#include <gp_Vec.hxx>

namespace onecad::ops {

using nlohmann::json;

namespace {

// SCHEMA §7.3: the frozen point is re-projected onto the resolved face and the
// op "fails loudly past 1e-3 mm".
constexpr double kPointPlaneFence = 1e-3;
// Face-boundary classification tolerance (BooleanOperation.cpp:116 precedent).
constexpr double kClassifyTol = 1e-7;
constexpr double kPi = 3.14159265358979323846;

bool read_xyz(const json& value, gp_Pnt& out) {
    if (!value.is_array() || value.size() != 3) return false;
    for (const json& c : value) {
        if (!c.is_number()) return false;
    }
    out = gp_Pnt(value[0].get<double>(), value[1].get<double>(), value[2].get<double>());
    return true;
}

std::optional<OpOutcome> read_one_point(const json& entry, std::size_t k, HolePointSpec& out) {
    const json evidence = {{"pointIndex", k}};
    const std::string where = "Hole points[" + std::to_string(k) + "]";
    if (!entry.is_object() || !entry.contains("id") || !entry["id"].is_string() ||
        !entry.contains("at") || !read_xyz(entry["at"], out.frozen)) {
        return hole_point_refusal("HOLE_POINTS_MALFORMED",
                                  where + " must be {id, at:[x, y, z]}", evidence);
    }
    out.id = entry["id"].get<std::string>();
    if (out.id.empty() || out.id.size() > kHolePointIdMaxBytes) {
        return hole_point_refusal("HOLE_POINTS_MALFORMED",
                                  where + " id must be 1..=64 bytes", evidence);
    }
    return std::nullopt;
}

// A point on the shape that lies in its convex hull: a hole's tool radius
// bounds every born shape, so this point does too.
gp_Pnt representative_point(const TopoDS_Shape& shape) {
    if (shape.ShapeType() == TopAbs_VERTEX) return BRep_Tool::Pnt(TopoDS::Vertex(shape));
    GProp_GProps props;
    if (shape.ShapeType() == TopAbs_EDGE) {
        BRepGProp::LinearProperties(shape, props);
    } else {
        BRepGProp::SurfaceProperties(shape, props);
    }
    return props.CentreOfMass();
}

}  // namespace

OpOutcome hole_point_refusal(const char* reason_code, const std::string& message,
                             json evidence) {
    OpOutcome failure = OpOutcome::fail("OP_FAILED", message);
    failure.diagnostics.push_back({{"severity", "error"},
                                   {"code", "OP_FAILED"},
                                   {"message", message},
                                   {"stage", "preflight"},
                                   {"reasonCode", reason_code},
                                   {"evidence", std::move(evidence)}});
    return failure;
}

std::optional<OpOutcome> read_hole_points(const json& params, std::vector<HolePointSpec>& out) {
    out.clear();
    if (!params.contains("points") || !params["points"].is_array()) {
        return hole_point_refusal("HOLE_POINTS_MALFORMED",
                                  "Hole resultPolicyVersion 3 requires a points array",
                                  json::object());
    }
    const json& points = params["points"];
    if (points.empty() || points.size() > kHoleMaxPoints) {
        return hole_point_refusal("HOLE_POINTS_COUNT", "Hole needs 1..=64 points",
                                  {{"count", points.size()}});
    }
    std::set<std::string> seen;
    for (std::size_t k = 0; k < points.size(); ++k) {
        HolePointSpec spec;
        if (auto refusal = read_one_point(points[k], k, spec)) return refusal;
        if (!seen.insert(spec.id).second) {
            return hole_point_refusal("HOLE_POINT_ID_DUPLICATE",
                                      "Hole points[" + std::to_string(k) + "] repeats an id",
                                      {{"pointIndex", k}});
        }
        out.push_back(std::move(spec));
    }
    if (params.contains("point")) {
        gp_Pnt mirror;
        const gp_Pnt& first = out.front().frozen;
        if (!read_xyz(params["point"], mirror) || mirror.X() != first.X() ||
            mirror.Y() != first.Y() || mirror.Z() != first.Z()) {
            return hole_point_refusal("HOLE_POINT_MISMATCH", "Hole point must equal points[0]",
                                      {{"pointIndex", 0}});
        }
    }
    return std::nullopt;
}

std::string place_hole_point(const TopoDS_Face& face, const gp_Pnt& frozen, gp_Pnt& origin,
                             const char*& reason_code) {
    reason_code = "HOLE_POINT_UNPROJECTABLE";
    try {
        const Handle(Geom_Surface) surf = BRep_Tool::Surface(face);
        GeomAPI_ProjectPointOnSurf proj(frozen, surf);
        if (!proj.IsDone() || proj.NbPoints() < 1) {
            return "Hole point could not be projected onto the face";
        }
        if (proj.LowerDistance() > kPointPlaneFence) {
            reason_code = "HOLE_POINT_OFF_FACE";
            return "Hole point is off the resolved face plane (fence 1e-3 mm)";
        }
        origin = proj.NearestPoint();
        Standard_Real u = 0.0, v = 0.0;
        proj.LowerDistanceParameters(u, v);
        // Re-projecting onto the PLANE is not enough: the point must land inside
        // the face's boundary, else the drill would start beside the material.
        BRepClass_FaceClassifier classifier(face, gp_Pnt2d(u, v), kClassifyTol);
        if (classifier.State() == TopAbs_OUT) {
            reason_code = "HOLE_POINT_OUTSIDE_FACE";
            return "Hole point lies outside the resolved face boundary";
        }
    } catch (const Standard_Failure& f) {
        return std::string("Hole point could not be resolved on the face: ") +
               (f.GetMessageString() ? f.GetMessageString() : "OCCT");
    }
    return "";
}

double hole_tool_radius(const HoleDims& dims) {
    double radius = dims.diameter * 0.5;
    if (dims.cb_diameter > 0.0 && dims.cb_depth > 0.0) {
        radius = std::max(radius, dims.cb_diameter * 0.5);
    }
    if (countersink_depth(dims) > 0.0) {
        const double half = dims.cs_angle_deg * kPi / 360.0;
        radius = std::max(radius, dims.cs_diameter * 0.5 + kFaceOvershoot * std::tan(half));
    }
    return radius;
}

std::optional<OpOutcome> check_hole_spacing(const std::vector<gp_Pnt>& origins, double radius) {
    const double min_distance = 2.0 * radius + kHolePointGap;
    for (std::size_t i = 0; i < origins.size(); ++i) {
        for (std::size_t j = i + 1; j < origins.size(); ++j) {
            const double d = origins[i].Distance(origins[j]);
            const json evidence = {{"pointIndex", i},
                                   {"otherPointIndex", j},
                                   {"distanceMm", d},
                                   {"minDistanceMm", min_distance}};
            const std::string pair =
                "Hole points[" + std::to_string(i) + "] and points[" + std::to_string(j) + "]";
            if (d < kHolePointGap) {
                return hole_point_refusal("HOLE_POINTS_COINCIDENT", pair + " coincide", evidence);
            }
            if (d < min_distance) {
                return hole_point_refusal("HOLE_POINTS_OVERLAP", pair + " overlap", evidence);
            }
        }
    }
    return std::nullopt;
}

void attribute_hole_births(session::TopologyBodyHistory& history,
                           const std::vector<gp_Pnt>& origins, const gp_Dir& axis,
                           double radius, const std::vector<HolePointSpec>& specs) {
    // Half the spacing margin: still unique, because any other axis is at least
    // `radius + kHolePointGap` away from a shape inside this tool.
    const double reach = radius + 0.5 * kHolePointGap;
    for (session::TopologyBirth& birth : history.births) {
        if (birth.after.IsNull()) continue;
        const gp_Pnt p = representative_point(birth.after);
        std::size_t owner = origins.size();
        for (std::size_t k = 0; k < origins.size(); ++k) {
            const gp_Vec v(origins[k], p);
            const gp_Vec radial = v - gp_Vec(axis) * v.Dot(gp_Vec(axis));
            if (radial.Magnitude() > reach) continue;
            owner = owner == origins.size() ? k : origins.size() + 1;
        }
        if (owner < origins.size()) birth.slot = specs[owner].id;
    }
}

}  // namespace onecad::ops
