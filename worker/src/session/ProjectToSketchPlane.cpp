// ProjectToSketchPlane.cpp — see ProjectToSketchPlane.h. SCHEMA §7.6.
#include "session/ProjectToSketchPlane.h"

#include <cstddef>
#include <cstdint>
#include <optional>
#include <string>
#include <vector>

#include <TopAbs_ShapeEnum.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>

#include "elementmap/ElementMapPartition.h"
#include "sketch/EdgeProjector.h"
#include "util/Log.h"

namespace onecad::session {

using nlohmann::json;
using protocol::Envelope;
namespace em = onecad::elementmap;
namespace ep = onecad::core::sketch;

namespace {

// A batch is a SELECTION, and a selection the user can make by hand. The cap is
// a framing guard, not a policy: past it the request is malformed, not merely
// large.
constexpr std::size_t kMaxSources = 512;

Envelope fail(const Envelope& req, const char* code, const std::string& message) {
    // The verb mutates nothing, so every failure leaves the session intact.
    return Envelope::error_response(req.id,
                                    protocol::ErrorInfo{code, message, /*retriable=*/false});
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

bool is_degenerate_axis(const double v[3]) {
    return (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]) <= 1e-18;
}

double positive_or(const json& o, const char* key, double fallback) {
    if (o.is_object() && o.contains(key) && o[key].is_number()) {
        const double v = o[key].get<double>();
        if (v > 0.0) return v;
    }
    return fallback;
}

std::string point_ref(std::size_t index) {
    return "p" + std::to_string(index);
}

// One entry of `sources[]`, as ADDRESSED (not as resolved). Echoed verbatim on
// both `entities[].sourceRef` and `refusals[]`, so a reader can tie either back
// to the pick it sent. Every key is always present — Rust parses one shape.
struct SourceRef {
    std::string body_id;
    std::string element_id;
    std::string topo_key;

    json to_json() const {
        return json{{"bodyId", body_id}, {"elementId", element_id}, {"topoKey", topo_key}};
    }
};

struct SourceLookup {
    bool present = false;
    std::string body_id;
    TopoDS_Shape shape;
};

// The two addressing rungs of SCHEMA §7.6, shared with `ProjectFaceBoundary`
// (FaceProjection.cpp:87-116) but PER SOURCE: `elementId` (partition entry ->
// bodyId/topoKey -> sub-shape), else `{bodyId, topoKey}` directly. A miss on
// either is the per-source `absent` refusal, never `present:false` — a batch has
// no single presence answer.
SourceLookup resolve_source(const BodyStore& bodies, const em::ElementMapPartition& part,
                            const SourceRef& ref) {
    SourceLookup out;
    std::string body_id = ref.body_id;
    std::string topo_key = ref.topo_key;

    if (!ref.element_id.empty()) {
        const em::PartitionEntry* entry = part.find(ref.element_id);
        if (entry == nullptr) return out;
        body_id = entry->body_id;
        topo_key = entry->topo_key;
    }

    if (body_id.empty() || topo_key.empty()) return out;
    const BodyRecord* rec = bodies.get(body_id);
    if (rec == nullptr) return out;

    const TopoDS_Shape sub = em::ElementMapPartition::shape_for_topokey(rec->geom, topo_key);
    if (sub.IsNull()) return out;

    out.present = true;
    out.body_id = body_id;
    out.shape = sub;
    return out;
}

json entity_json(const ep::EdgeProjector::Buffer& buffer, const ep::EdgeProjector::Entity& e,
                 const SourceRef& source) {
    json out;
    switch (e.kind) {
        case ep::EdgeProjector::EntityKind::Line:
            out = json{{"type", "Line"},
                       {"p0Ref", point_ref(static_cast<std::size_t>(e.p0))},
                       {"p1Ref", point_ref(static_cast<std::size_t>(e.p1))}};
            break;
        case ep::EdgeProjector::EntityKind::Circle:
            out = json{{"type", "Circle"},
                       {"centerRef", point_ref(static_cast<std::size_t>(e.center))},
                       {"radius", e.radius}};
            break;
        case ep::EdgeProjector::EntityKind::Arc:
            out = json{{"type", "Arc"},
                       {"centerRef", point_ref(static_cast<std::size_t>(e.center))},
                       {"radius", e.radius},
                       {"startAngle", e.startAngle},
                       {"endAngle", e.endAngle},
                       {"ccw", e.ccw}};
            break;
        case ep::EdgeProjector::EntityKind::Ellipse:
            out = json{{"type", "Ellipse"},
                       {"centerRef", point_ref(static_cast<std::size_t>(e.center))},
                       {"majorR", e.majorR},
                       {"minorR", e.minorR},
                       {"rotation", e.rotation}};
            break;
    }
    out["sourceRef"] = source.to_json();
    out["projectedHash"] = ep::EdgeProjector::projectedHash(buffer, e);
    return out;
}

json refusal_json(const SourceRef& source, const ep::EdgeProjector::Refusal& refusal) {
    json out = source.to_json();
    out["code"] = ep::EdgeProjector::refusalToken(refusal.code);
    out["message"] = refusal.message;
    return out;
}

}  // namespace

Envelope handle_project_to_sketch_plane(Session& session, const Envelope& req) {
    const json& args = req.args;

    // --- snapshot fence (SCHEMA §7.6: this verb IS fenced) --------------------
    if (!args.is_object() || !args.contains("snapshotId") ||
        !args["snapshotId"].is_number_unsigned()) {
        return fail(req, "PROTOCOL_ERROR", "ProjectToSketchPlane: snapshotId (u64) is required");
    }
    const std::uint64_t requested = args["snapshotId"].get<std::uint64_t>();
    std::uint64_t head_snapshot = 0;
    const std::optional<PublishedStateSnapshot> published =
        session.published_state_at(requested, &head_snapshot);
    if (!published) {
        return fail(req, "STALE_PREVIEW",
                    "ProjectToSketchPlane: picks were taken against snapshot " +
                        std::to_string(requested) + " but the head is " +
                        std::to_string(head_snapshot) + " — re-pick");
    }

    // --- plane: REQUIRED and AUTHORITATIVE -----------------------------------
    if (!args.contains("plane") || !args["plane"].is_object()) {
        return fail(req, "PROTOCOL_ERROR", "ProjectToSketchPlane: args.plane is required");
    }
    const json& plane_json = args["plane"];
    double origin[3] = {0, 0, 0};
    double x_axis[3] = {0, 0, 0};
    double y_axis[3] = {0, 0, 0};
    double normal[3] = {0, 0, 0};
    if (!read_vec3(plane_json, "origin", origin) || !read_vec3(plane_json, "xAxis", x_axis) ||
        !read_vec3(plane_json, "yAxis", y_axis) || !read_vec3(plane_json, "normal", normal)) {
        return fail(req, "PROTOCOL_ERROR",
                    "ProjectToSketchPlane: args.plane needs origin/xAxis/yAxis/normal as "
                    "3-number arrays");
    }
    if (is_degenerate_axis(x_axis) || is_degenerate_axis(y_axis) || is_degenerate_axis(normal)) {
        return fail(req, "PROTOCOL_ERROR", "ProjectToSketchPlane: args.plane is degenerate");
    }
    ep::SketchPlane plane;
    plane.origin = {origin[0], origin[1], origin[2]};
    plane.xAxis = {x_axis[0], x_axis[1], x_axis[2]};
    plane.yAxis = {y_axis[0], y_axis[1], y_axis[2]};
    plane.normal = {normal[0], normal[1], normal[2]};

    // --- mode ----------------------------------------------------------------
    const std::string mode_arg = get_str(args, "mode");
    const std::string mode = mode_arg.empty() ? std::string("edges") : mode_arg;
    if (mode != "edges" && mode != "faceOutline") {
        return fail(req, "PROTOCOL_ERROR",
                    "ProjectToSketchPlane: mode must be edges or faceOutline");
    }
    // The mode decides the kind a source MUST have; `kind` is checked against it
    // AND against the resolved shape, never trusted.
    const char* required_kind = (mode == "faceOutline") ? "face" : "edge";
    const TopAbs_ShapeEnum required_type =
        (mode == "faceOutline") ? TopAbs_FACE : TopAbs_EDGE;

    // --- sources -------------------------------------------------------------
    if (!args.contains("sources") || !args["sources"].is_array() || args["sources"].empty()) {
        return fail(req, "PROTOCOL_ERROR",
                    "ProjectToSketchPlane: args.sources must be a non-empty array");
    }
    if (args["sources"].size() > kMaxSources) {
        return fail(req, "PROTOCOL_ERROR",
                    "ProjectToSketchPlane: args.sources exceeds " +
                        std::to_string(kMaxSources) + " entries");
    }
    std::vector<SourceRef> sources;
    std::vector<std::string> declared_kinds;
    sources.reserve(args["sources"].size());
    declared_kinds.reserve(args["sources"].size());
    for (const json& s : args["sources"]) {
        if (!s.is_object()) {
            return fail(req, "PROTOCOL_ERROR",
                        "ProjectToSketchPlane: every sources[] entry must be an object");
        }
        const std::string kind_arg = get_str(s, "kind");
        const std::string kind = kind_arg.empty() ? std::string(required_kind) : kind_arg;
        if (kind != "edge" && kind != "face") {
            return fail(req, "PROTOCOL_ERROR",
                        "ProjectToSketchPlane: sources[].kind must be edge or face");
        }
        sources.push_back(SourceRef{get_str(s, "bodyId"), get_str(s, "elementId"),
                                    get_str(s, "topoKey")});
        declared_kinds.push_back(kind);
    }

    const json opts_json =
        (args.contains("options") && args["options"].is_object()) ? args["options"]
                                                                 : json::object();
    ep::EdgeProjector::Options options;
    options.pointMergeTolerance =
        positive_or(opts_json, "pointMergeTolerance", options.pointMergeTolerance);
    options.radiusTolerance =
        positive_or(opts_json, "radiusTolerance", options.radiusTolerance);

    // --- project, source by source, in REQUEST order -------------------------
    ep::EdgeProjector::Buffer buffer;
    std::vector<json> refusals;
    for (std::size_t i = 0; i < sources.size(); ++i) {
        const SourceRef& source = sources[i];
        const SourceLookup found = resolve_source(published->bodies, published->partition, source);
        if (!found.present) {
            refusals.push_back(refusal_json(
                source, ep::EdgeProjector::Refusal{ep::EdgeProjector::RefusalCode::Absent,
                                                  "source did not resolve in this snapshot"}));
            continue;
        }
        if (declared_kinds[i] != required_kind || found.shape.ShapeType() != required_type) {
            refusals.push_back(refusal_json(
                source,
                ep::EdgeProjector::Refusal{
                    ep::EdgeProjector::RefusalCode::KindMismatch,
                    "mode " + mode + " projects " + required_kind + " sources"}));
            continue;
        }

        const ep::EdgeProjector::Refusal refusal =
            (mode == "faceOutline")
                ? ep::EdgeProjector::projectFaceOutline(buffer, TopoDS::Face(found.shape), plane,
                                                        options, i)
                : ep::EdgeProjector::projectEdge(buffer, TopoDS::Edge(found.shape), plane, options,
                                                 i);
        if (refusal.refused()) {
            refusals.push_back(refusal_json(source, refusal));
        }
    }

    json points = json::array();
    for (std::size_t i = 0; i < buffer.points.size(); ++i) {
        points.push_back(json{{"ref", point_ref(i)},
                              {"at", json::array({buffer.points[i].u, buffer.points[i].v})}});
    }
    json entities = json::array();
    for (const ep::EdgeProjector::Entity& e : buffer.entities) {
        entities.push_back(entity_json(buffer, e, sources[e.sourceIndex]));
    }
    json refusal_array = json::array();
    for (json& r : refusals) refusal_array.push_back(std::move(r));

    WLOG_DEBUG("ProjectToSketchPlane: mode=%s sources=%zu entities=%zu refusals=%zu",
               mode.c_str(), sources.size(), buffer.entities.size(), refusal_array.size());

    return Envelope::ok_response(req.id, json{{"snapshotId", published->snapshot_id},
                                              {"sketchId", get_str(args, "sketchId")},
                                              {"points", std::move(points)},
                                              {"entities", std::move(entities)},
                                              {"refusals", std::move(refusal_array)}});
}

}  // namespace onecad::session
