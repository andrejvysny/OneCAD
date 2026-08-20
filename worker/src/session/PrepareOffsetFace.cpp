// PrepareOffsetFace.cpp — see PrepareOffsetFace.h. SCHEMA §7.6 `PrepareOffsetFace`.
#include "session/PrepareOffsetFace.h"

#include <algorithm>
#include <cmath>
#include <string>
#include <vector>

#include <BRepAdaptor_Surface.hxx>
#include <BRepAlgoAPI_Common.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <BRepGProp.hxx>
#include <BRepPrimAPI_MakePrism.hxx>
#include <GProp_GProps.hxx>
#include <Precision.hxx>
#include <Standard_Failure.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopTools_ListOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Lin.hxx>
#include <gp_Trsf.hxx>
#include <gp_Vec.hxx>

#include "elementmap/ElementMapPartition.h"
#include "kernel/validation/GeometryPrecision.h"
#include "ops/OffsetFaceOp.h"
#include "util/Log.h"

namespace onecad::session {

using nlohmann::json;
using protocol::Envelope;
namespace em = onecad::elementmap;
namespace of = onecad::ops::offsetface;

namespace {

// Minimum measurable Total thickness / column depth (mm). Below this the material
// column boolean is numerical noise, so a candidate is not honestly verifiable.
constexpr double kMinThickness = 1.0e-6;

Envelope fail(const Envelope& req, const char* code, const std::string& message,
              json detail = json()) {
    return Envelope::error_response(
        req.id, protocol::ErrorInfo{code, message, /*retriable=*/false,
                                    detail.is_null() ? std::optional<json>{}
                                                     : std::optional<json>{std::move(detail)}});
}

std::string get_str(const json& o, const char* key) {
    if (o.is_object() && o.contains(key) && o[key].is_string()) return o[key].get<std::string>();
    return {};
}

json anchor_json(const of::FaceSample& s) {
    return json{{"worldPoint", json::array({s.point.X(), s.point.Y(), s.point.Z()})},
                {"surfaceUv", json::array({s.u, s.v})}};
}

// One face of the answer: TopoKey + picked flag + anchor + verbatim §10 descriptor.
json face_entry(const TopoDS_Shape& body, int ordinal, bool picked) {
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(body, TopAbs_FACE, faces);
    const TopoDS_Face f = TopoDS::Face(faces(ordinal));
    json e{{"topoKey", of::face_topokey(ordinal)},
           {"picked", picked},
           {"descriptor", em::ElementMapPartition::descriptor_to_json(
                              em::ElementMapPartition::describe(f))}};
    const of::FaceSample s = of::sample_face(f);
    e["anchor"] = s.ok ? anchor_json(s) : json::object();
    return e;
}

json refusal_json(const char* code, const std::string& message,
                  const std::vector<int>& ordinals = {}) {
    json faces = json::array();
    std::vector<int> sorted = ordinals;
    std::sort(sorted.begin(), sorted.end());
    for (const int ord : sorted) faces.push_back(of::face_topokey(ord));
    return json{{"code", code}, {"message", message}, {"faces", std::move(faces)}};
}

// The full response shell, so every branch (success AND refusal) answers the same
// key set — Rust parses one shape.
json response(std::uint64_t snapshot_id, const std::string& target_body, json faces,
              json current_dims, json refusal) {
    return json{{"snapshotId", snapshot_id},
                {"targetBodyId", target_body},
                {"faces", std::move(faces)},
                {"currentDims", std::move(current_dims)},
                {"refusal", std::move(refusal)}};
}

// Deterministic single-threaded Common of two shapes; null on any failure.
TopoDS_Shape common_of(const TopoDS_Shape& a, const TopoDS_Shape& b) {
    if (a.IsNull() || b.IsNull()) return TopoDS_Shape();
    try {
        BRepAlgoAPI_Common builder;
        TopTools_ListOfShape args, tools;
        args.Append(a);
        tools.Append(b);
        builder.SetArguments(args);
        builder.SetTools(tools);
        builder.SetRunParallel(Standard_False);  // determinism (Invariant 5)
        builder.Build();
        if (!builder.IsDone() || builder.HasErrors()) return TopoDS_Shape();
        return builder.Shape();
    } catch (const Standard_Failure&) {
        return TopoDS_Shape();
    }
}

double area_of(const TopoDS_Shape& s) {
    if (s.IsNull()) return 0.0;
    GProp_GProps props;
    BRepGProp::SurfaceProperties(s, props);
    return props.Mass();
}

double volume_of(const TopoDS_Shape& s) {
    if (s.IsNull()) return 0.0;
    GProp_GProps props;
    BRepGProp::VolumeProperties(s, props);
    return props.Mass();
}

struct OppositeCandidate {
    int ordinal = 0;
    double thickness = 0.0;
};

// FAIL-CLOSED Total opposite detection (SCHEMA §7.6). A candidate must satisfy ALL:
//   1. planar and ANTI-PARALLEL to the selected face;
//   2. at a strictly positive inward distance `t`;
//   3. FOOTPRINT COVERAGE — the selected face translated by `−t·n` onto the
//      candidate's plane intersects it in the selected face's FULL area (holes and
//      concavity included), so the candidate really backs the whole face;
//   4. MATERIAL COLUMN — the inward prism of the selected face ∩ the body has
//      volume `area × t`, so the span is solid all the way down (a cavity between
//      the two walls would pass (3) alone).
// Exactly one passing candidate is required; zero or many ⇒ `noUniqueOpposite`.
std::vector<OppositeCandidate> find_opposites(const TopoDS_Shape& body, int selected_ordinal) {
    std::vector<OppositeCandidate> out;
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(body, TopAbs_FACE, faces);
    if (selected_ordinal < 1 || selected_ordinal > faces.Extent()) return out;
    const TopoDS_Face selected = TopoDS::Face(faces(selected_ordinal));
    const of::PlaneInfo sel = of::plane_info(selected);
    if (!sel.ok || !of::sample_face(selected).ok) return out;
    const double area = area_of(selected);
    if (!(area > 0.0)) return out;  // an unmeasurable face backs nothing

    for (int ord = 1; ord <= faces.Extent(); ++ord) {  // ordinal order — deterministic
        if (ord == selected_ordinal) continue;
        const TopoDS_Face cand = TopoDS::Face(faces(ord));
        const of::PlaneInfo c = of::plane_info(cand);
        if (!c.ok || !of::sample_face(cand).ok) continue;
        // `semantic_angular()` carries no conditioning term by design (a length-derived
        // quantity added to an angle is a dimensional error), so the floor-only context
        // returns exactly the angular floor.
        if (gp_Vec(sel.normal).Dot(gp_Vec(c.normal)) >
            -std::cos(kernel::validation::GeometryPrecisionContext{}.semantic_angular())) {
            continue;
        }
        const double t = gp_Vec(c.location, sel.location).Dot(gp_Vec(sel.normal));
        if (t <= kMinThickness) continue;

        // (3) footprint coverage.
        gp_Trsf move;
        move.SetTranslation(gp_Vec(sel.normal) * (-t));
        TopoDS_Shape dropped;
        try {
            BRepBuilderAPI_Transform tr(selected, move, Standard_True);
            if (!tr.IsDone()) continue;
            dropped = tr.Shape();
        } catch (const Standard_Failure&) {
            continue;
        }
        const double covered = area_of(common_of(dropped, cand));
        if (std::abs(covered - area) > of::kColumnRelTol * area) continue;

        // (4) material column.
        TopoDS_Shape prism;
        try {
            prism = BRepPrimAPI_MakePrism(selected, gp_Vec(sel.normal) * (-t)).Shape();
        } catch (const Standard_Failure&) {
            continue;
        }
        const double column = volume_of(common_of(prism, body));
        const double expected = area * t;
        if (std::abs(column - expected) > of::kColumnRelTol * std::max(1.0, expected)) continue;

        out.push_back(OppositeCandidate{ord, t});
    }
    return out;
}

// Resolve one `pickedFaces[]` entry to (bodyId, face ordinal). Empty bodyId ⇒ miss.
struct Pick {
    std::string body_id;
    int ordinal = 0;
};

Pick resolve_pick(const BodyStore& bodies, const em::ElementMapPartition& part, const json& p) {
    Pick out;
    std::string body_id = get_str(p, "bodyId");
    std::string topo_key = get_str(p, "topoKey");
    const std::string element_id = get_str(p, "elementId");
    if (!element_id.empty()) {
        const em::PartitionEntry* e = part.find(element_id);
        if (e == nullptr) return out;
        body_id = e->body_id;
        topo_key = e->topo_key;
    }
    if (body_id.empty() || topo_key.empty()) return out;
    const BodyRecord* rec = bodies.get(body_id);
    if (rec == nullptr || rec->geom.IsNull()) return out;
    const TopoDS_Shape sub = em::ElementMapPartition::shape_for_topokey(rec->geom, topo_key);
    if (sub.IsNull() || sub.ShapeType() != TopAbs_FACE) return out;
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(rec->geom, TopAbs_FACE, faces);
    const int ord = faces.FindIndex(sub);
    if (ord == 0) return out;
    out.body_id = body_id;
    out.ordinal = ord;
    return out;
}

}  // namespace

Envelope handle_prepare_offset_face(Session& session, const Envelope& req) {
    const json& args = req.args;
    const std::uint64_t head_snapshot = session.current_snapshot_id();

    // --- snapshot fence (SCHEMA §7.6: this verb IS fenced) --------------------
    // The fence is MANDATORY: an absent snapshotId would freeze a closure against
    // an unverified head, which is exactly what the fence exists to prevent.
    if (!args.is_object() || !args.contains("snapshotId") ||
        !args["snapshotId"].is_number_unsigned()) {
        return fail(req, "PROTOCOL_ERROR", "PrepareOffsetFace: snapshotId (u64) is required");
    }
    const std::uint64_t requested = args["snapshotId"].get<std::uint64_t>();
    if (requested != head_snapshot) {
        return fail(req, "STALE_PREVIEW",
                    "PrepareOffsetFace: picks were taken against snapshot " +
                        std::to_string(requested) + "; head is " +
                        std::to_string(head_snapshot) + " — re-pick",
                    json{{"requested", requested}, {"head", head_snapshot}});
    }

    if (!args.is_object() || !args.contains("pickedFaces") || !args["pickedFaces"].is_array() ||
        args["pickedFaces"].empty()) {
        return fail(req, "PROTOCOL_ERROR", "PrepareOffsetFace: pickedFaces must be a non-empty array");
    }
    const std::string type = get_str(args, "distanceType");
    if (!type.empty() && type != "Offset" && type != "Total" && type != "Radius" &&
        type != "Diameter") {
        return fail(req, "PROTOCOL_ERROR", "PrepareOffsetFace: unknown distanceType: " + type);
    }
    const bool chain = !args.contains("chainTangentFaces") ||
                       !args["chainTangentFaces"].is_boolean() ||
                       args["chainTangentFaces"].get<bool>();

    const BodyStore bodies = session.bodies_copy();
    const em::ElementMapPartition part = session.partition_copy();

    // --- resolve the picks ----------------------------------------------------
    std::vector<int> picked_ordinals;
    std::string target_body;
    bool cross_body = false;
    for (const json& p : args["pickedFaces"]) {
        const Pick pick = resolve_pick(bodies, part, p);
        if (pick.body_id.empty()) {
            return fail(req, "REF_UNRESOLVED",
                        "PrepareOffsetFace: a picked face does not resolve on the head");
        }
        if (target_body.empty()) {
            target_body = pick.body_id;
        } else if (target_body != pick.body_id) {
            cross_body = true;
        }
        picked_ordinals.push_back(pick.ordinal);
    }
    if (cross_body) {
        return Envelope::ok_response(
            req.id, response(head_snapshot, "", json::array(), json::object(),
                             refusal_json("crossBody",
                                          "OffsetFace picks span more than one body")));
    }
    std::sort(picked_ordinals.begin(), picked_ordinals.end());
    picked_ordinals.erase(std::unique(picked_ordinals.begin(), picked_ordinals.end()),
                          picked_ordinals.end());

    const BodyRecord* rec = bodies.get(target_body);
    const TopoDS_Shape body = rec->geom;  // resolve_pick guaranteed a live record
    TopTools_IndexedMapOfShape body_faces;
    TopExp::MapShapes(body, TopAbs_FACE, body_faces);

    // --- G1 tangent closure ---------------------------------------------------
    const of::ClosureResult closure = of::tangent_closure(body, picked_ordinals);
    if (closure.non_manifold) {
        return Envelope::ok_response(
            req.id,
            response(head_snapshot, target_body, json::array(), json::object(),
                     refusal_json("nonManifold",
                                  "the selection touches a non-manifold edge", picked_ordinals)));
    }
    if (!closure.ok) {
        return fail(req, "OP_FAILED", "PrepareOffsetFace: tangent-closure walk failed");
    }

    auto is_picked = [&](int ord) {
        return std::binary_search(picked_ordinals.begin(), picked_ordinals.end(), ord);
    };
    json faces = json::array();
    for (const int ord : closure.ordinals) faces.push_back(face_entry(body, ord, is_picked(ord)));

    // --- V1 surface scope -----------------------------------------------------
    std::vector<int> unsupported;
    for (const int ord : closure.ordinals) {
        if (of::surface_kind(TopoDS::Face(body_faces(ord))) == of::SurfaceKind::Other) {
            unsupported.push_back(ord);
        }
    }
    if (!unsupported.empty()) {
        return Envelope::ok_response(
            req.id, response(head_snapshot, target_body, std::move(faces), json::object(),
                             refusal_json("unsupportedSurface",
                                          "the tangent closure contains a surface type outside "
                                          "the planar+cylindrical V1 scope",
                                          unsupported)));
    }

    // --- chain-off must not hide an auto-propagating tangent neighbour --------
    std::vector<int> extra;
    std::set_difference(closure.ordinals.begin(), closure.ordinals.end(), picked_ordinals.begin(),
                        picked_ordinals.end(), std::back_inserter(extra));
    if (!chain && !extra.empty()) {
        return Envelope::ok_response(
            req.id,
            response(head_snapshot, target_body, std::move(faces), json::object(),
                     refusal_json("chainMismatch",
                                  "chainTangentFaces is false but the selection is G1-tangent to "
                                  "faces outside it; the kernel cannot hold them fixed",
                                  extra)));
    }

    // --- currentDims: radius, whenever the closure IS a usable cylindrical set --
    json current_dims = json::object();
    bool cylindrical_set = true;
    of::CylinderInfo first;
    // Measured on the BODY the faces are read off. Its conditioning term cannot reach
    // the semantic floor anywhere in the supported coordinate range — that is asserted
    // in `test_geometry_precision`.
    const double semantic_length =
        kernel::validation::precision_of(body).semantic_length();
    for (const int ord : closure.ordinals) {
        const of::CylinderInfo info = of::cylinder_info(TopoDS::Face(body_faces(ord)));
        if (!info.ok) { cylindrical_set = false; break; }
        if (first.ok) {
            const bool same_axis =
                first.axis.Direction().IsParallel(info.axis.Direction(), Precision::Angular()) &&
                gp_Lin(first.axis).Distance(info.axis.Location()) <= semantic_length;
            if (!same_axis || std::abs(first.radius - info.radius) > semantic_length ||
                first.sigma != info.sigma) {
                cylindrical_set = false;
                break;
            }
        } else {
            first = info;
        }
    }
    if (cylindrical_set && first.ok) current_dims["radius"] = first.radius;

    // --- distanceType-specific gates -----------------------------------------
    json opposite_entry;
    if (type == "Radius" || type == "Diameter") {
        bool any_non_cylinder = false;
        std::vector<int> non_cyl;
        for (const int ord : closure.ordinals) {
            if (of::surface_kind(TopoDS::Face(body_faces(ord))) != of::SurfaceKind::Cylinder) {
                any_non_cylinder = true;
                non_cyl.push_back(ord);
            }
        }
        if (any_non_cylinder) {
            return Envelope::ok_response(
                req.id,
                response(head_snapshot, target_body, std::move(faces), std::move(current_dims),
                         refusal_json("notCylindrical",
                                      std::string(type) + " requires a cylindrical operative set",
                                      non_cyl)));
        }
        if (!cylindrical_set) {
            return Envelope::ok_response(
                req.id, response(head_snapshot, target_body, std::move(faces),
                                 std::move(current_dims),
                                 refusal_json("mixedAxis",
                                              std::string(type) +
                                                  " requires a COAXIAL, equal-radius, same-σ "
                                                  "cylindrical set",
                                              closure.ordinals)));
        }
    } else if (type == "Total") {
        if (chain) {
            return Envelope::ok_response(
                req.id,
                response(head_snapshot, target_body, std::move(faces), std::move(current_dims),
                         refusal_json("chainOnTotal",
                                      "Total distance requires chainTangentFaces:false")));
        }
        if (closure.ordinals.size() != 1 ||
            of::surface_kind(TopoDS::Face(body_faces(closure.ordinals[0]))) !=
                of::SurfaceKind::Plane) {
            return Envelope::ok_response(
                req.id,
                response(head_snapshot, target_body, std::move(faces), std::move(current_dims),
                         refusal_json("notPlanar",
                                      "Total distance requires exactly ONE planar face; the "
                                      "operative set has " +
                                          std::to_string(closure.ordinals.size()),
                                      closure.ordinals)));
        }
        const std::vector<OppositeCandidate> candidates =
            find_opposites(body, closure.ordinals[0]);
        if (candidates.size() != 1) {
            std::vector<int> named;
            for (const OppositeCandidate& c : candidates) named.push_back(c.ordinal);
            WLOG_DEBUG("PrepareOffsetFace: Total opposite candidates=%zu", candidates.size());
            return Envelope::ok_response(
                req.id,
                response(head_snapshot, target_body, std::move(faces), std::move(current_dims),
                         refusal_json("noUniqueOpposite",
                                      "no single face fully backs the selected face at a solid "
                                      "thickness (" +
                                          std::to_string(candidates.size()) + " candidates)",
                                      named)));
        }
        opposite_entry = face_entry(body, candidates[0].ordinal, /*picked=*/false);
        opposite_entry.erase("picked");  // an opposite is never a pick
        current_dims["thickness"] = candidates[0].thickness;
    }

    json result = response(head_snapshot, target_body, std::move(faces), std::move(current_dims),
                           json(nullptr));
    if (!opposite_entry.is_null()) result["oppositeFace"] = std::move(opposite_entry);
    return Envelope::ok_response(req.id, std::move(result));
}

}  // namespace onecad::session
