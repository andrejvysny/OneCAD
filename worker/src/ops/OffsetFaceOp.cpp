// OffsetFaceOp.cpp — see OffsetFaceOp.h. SCHEMA §7.3 `op.offsetFace`.
#include "ops/OffsetFaceOp.h"

#include <algorithm>
#include <array>
#include <cmath>
#include <cstdint>
#include <deque>
#include <limits>
#include <map>
#include <optional>
#include <set>
#include <string>
#include <utility>
#include <vector>

#include <BRepAdaptor_Surface.hxx>
#include <BRepAlgoAPI_Check.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <BRepGProp.hxx>
#include <BRepLProp_SLProps.hxx>
#include <BRepLib.hxx>
#include <BRepOffset_Error.hxx>
#include <BRepOffset_MakeOffset.hxx>
#include <BRepOffset_Mode.hxx>
#include <BRep_Tool.hxx>
#include <BRepTools.hxx>
#include <BRepTopAdaptor_FClass2d.hxx>
#include <GProp_GProps.hxx>
#include <GeomAbs_JoinType.hxx>
#include <GeomAbs_Shape.hxx>
#include <GeomAbs_SurfaceType.hxx>
#include <Precision.hxx>
#include <Standard_Failure.hxx>
#include <TopAbs_Orientation.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_IndexedDataMapOfShapeListOfShape.hxx>
#include <TopTools_ListOfShape.hxx>
#include <TopoDS.hxx>
#include <gp_Lin.hxx>
#include <gp_Pnt2d.hxx>
#include <gp_Vec.hxx>

#include "elementmap/ComposedHistory.h"
#include "elementmap/ElementMapPartition.h"
#include "elementmap/Ladder.h"
#include "kernel/validation/GeometryPrecision.h"
#include "ops/OpCommon.h"
#include "util/Log.h"

namespace onecad::ops {

using nlohmann::json;
namespace em = onecad::elementmap;

// ---------------------------------------------------------------------------
// Shared geometry helpers (also used by session/PrepareOffsetFace.cpp).
// ---------------------------------------------------------------------------
namespace offsetface {

SurfaceKind surface_kind(const TopoDS_Face& face) {
    if (face.IsNull()) return SurfaceKind::Other;
    try {
        BRepAdaptor_Surface surf(face);
        switch (surf.GetType()) {
            case GeomAbs_Plane: return SurfaceKind::Plane;
            case GeomAbs_Cylinder: return SurfaceKind::Cylinder;
            default: return SurfaceKind::Other;
        }
    } catch (const Standard_Failure&) {
        return SurfaceKind::Other;
    }
}

FaceSample sample_face(const TopoDS_Face& face) {
    FaceSample out;
    if (face.IsNull()) return out;
    try {
        Standard_Real u_min, u_max, v_min, v_max;
        BRepTools::UVBounds(face, u_min, u_max, v_min, v_max);
        if (!std::isfinite(u_min) || !std::isfinite(u_max) ||
            !std::isfinite(v_min) || !std::isfinite(v_max) ||
            !(u_max > u_min) || !(v_max > v_min)) {
            return out;
        }
        // Center first, then a deterministic interior grid. The classifier is the
        // authority: a UV-box midpoint may sit in a hole or outside a concave trim.
        static constexpr std::array<double, 9> fractions{
            0.5, 0.25, 0.75, 0.125, 0.375, 0.625, 0.875, 0.0625, 0.9375};
        BRepAdaptor_Surface surf(face);
        BRepTopAdaptor_FClass2d classifier(face, Precision::PConfusion());
        for (const double fu : fractions) {
            const double u = u_min + fu * (u_max - u_min);
            for (const double fv : fractions) {
                const double v = v_min + fv * (v_max - v_min);
                if (classifier.Perform(gp_Pnt2d(u, v), Standard_False) != TopAbs_IN)
                    continue;
                BRepLProp_SLProps props(surf, u, v, 1, Precision::Confusion());
                if (!props.IsNormalDefined()) continue;
                gp_Dir n = props.Normal();
                if (face.Orientation() == TopAbs_REVERSED) n.Reverse();
                out.ok = true;
                out.point = props.Value();
                out.normal = n;
                out.u = u;
                out.v = v;
                return out;
            }
        }
    } catch (const Standard_Failure&) {
        out.ok = false;
    }
    return out;
}

PlaneInfo plane_info(const TopoDS_Face& face) {
    PlaneInfo out;
    if (surface_kind(face) != SurfaceKind::Plane) return out;
    try {
        BRepAdaptor_Surface surf(face, true);
        const gp_Pln plane = surf.Plane();
        gp_Dir normal = plane.Axis().Direction();
        if (face.Orientation() == TopAbs_REVERSED) normal.Reverse();
        out.ok = true;
        out.location = plane.Location();
        out.normal = normal;
    } catch (const Standard_Failure&) {
        out.ok = false;
    }
    return out;
}

CylinderInfo cylinder_info(const TopoDS_Face& face) {
    CylinderInfo out;
    if (surface_kind(face) != SurfaceKind::Cylinder) return out;
    const FaceSample s = sample_face(face);
    if (!s.ok) return out;
    try {
        BRepAdaptor_Surface surf(face);
        const gp_Cylinder cyl = surf.Cylinder();
        const gp_Ax1 axis = cyl.Axis();
        // r̂ — the axis→sample unit radial. Reject a degenerate sample (on the axis).
        const gp_Vec to_sample(axis.Location(), s.point);
        const gp_Vec along = gp_Vec(axis.Direction()) * to_sample.Dot(gp_Vec(axis.Direction()));
        const gp_Vec radial = to_sample - along;
        if (radial.Magnitude() <= Precision::Confusion()) return out;
        const gp_Dir r_hat(radial);
        const double dot = gp_Vec(s.normal).Dot(gp_Vec(r_hat));
        // A cylindrical face's outward normal is ±r̂ by construction; anything else
        // means the sample landed somewhere unusable, so refuse rather than guess.
        if (std::abs(dot) < kRadialDotMin) return out;
        out.ok = true;
        out.axis = axis;
        out.radius = cyl.Radius();
        out.sigma = (dot >= 0.0) ? 1 : -1;
    } catch (const Standard_Failure&) {
        out.ok = false;
    }
    return out;
}

double build_tolerance(const TopoDS_Shape& shape) {
    double tol = std::max(Precision::Confusion(), kBuildToleranceFloor);
    if (shape.IsNull()) return tol;
    for (const TopAbs_ShapeEnum kind : {TopAbs_FACE, TopAbs_EDGE, TopAbs_VERTEX}) {
        const double t = BRep_Tool::MaxTolerance(shape, kind);
        if (std::isfinite(t)) tol = std::max(tol, t);
    }
    return tol;
}

std::string face_topokey(int ordinal) { return "f:" + std::to_string(ordinal); }

ClosureResult tangent_closure(const TopoDS_Shape& body, const std::vector<int>& seed_ordinals) {
    ClosureResult out;
    if (body.IsNull() || seed_ordinals.empty()) return out;

    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(body, TopAbs_FACE, faces);
    TopTools_IndexedDataMapOfShapeListOfShape edge_faces;
    TopExp::MapShapesAndAncestors(body, TopAbs_EDGE, TopAbs_FACE, edge_faces);

    std::set<int> visited;          // ordinals already in the closure (sorted)
    std::deque<int> queue;          // BFS frontier, seeded in ascending order
    std::vector<int> seeds = seed_ordinals;
    std::sort(seeds.begin(), seeds.end());
    seeds.erase(std::unique(seeds.begin(), seeds.end()), seeds.end());
    for (const int ord : seeds) {
        if (ord < 1 || ord > faces.Extent()) return out;  // caller bug: foreign ordinal
        if (visited.insert(ord).second) queue.push_back(ord);
    }

    while (!queue.empty()) {
        const int ord = queue.front();
        queue.pop_front();
        const TopoDS_Face f1 = TopoDS::Face(faces(ord));
        // Edges of THIS face, walked in TopExp_Explorer order (shape-determined).
        for (TopExp_Explorer ex(f1, TopAbs_EDGE); ex.More(); ex.Next()) {
            const TopoDS_Edge edge = TopoDS::Edge(ex.Current());
            const int idx = edge_faces.FindIndex(edge);
            if (idx == 0) continue;
            // Distinct adjacent faces (a seam edge lists the SAME face twice —
            // self-adjacency, skipped; >2 distinct is non-manifold).
            std::vector<int> adjacent;
            for (TopTools_ListIteratorOfListOfShape it(edge_faces(idx)); it.More(); it.Next()) {
                const int a = faces.FindIndex(it.Value());
                if (a == 0) continue;
                if (std::find(adjacent.begin(), adjacent.end(), a) == adjacent.end()) {
                    adjacent.push_back(a);
                }
            }
            if (adjacent.size() > 2) {
                out.non_manifold = true;
                return out;
            }
            if (adjacent.size() != 2) continue;  // seam self-adjacency or free edge
            const int other = (adjacent[0] == ord) ? adjacent[1] : adjacent[0];
            if (other == ord) continue;
            if (visited.count(other) != 0) continue;
            const TopoDS_Face f2 = TopoDS::Face(faces(other));
            GeomAbs_Shape continuity = GeomAbs_C0;
            try {
                continuity = BRepLib::ContinuityOfFaces(edge, f1, f2, kTangentAngleTol);
            } catch (const Standard_Failure&) {
                continue;  // unreadable junction ⇒ not provably tangent ⇒ not chained
            }
            if (continuity < GeomAbs_G1) continue;
            visited.insert(other);
            queue.push_back(other);
        }
    }

    out.ok = true;
    out.ordinals.assign(visited.begin(), visited.end());  // std::set ⇒ ascending
    return out;
}

// --- OffsetImageHistory -----------------------------------------------------

OffsetImageHistory::OffsetImageHistory(const BRepAlgo_Image& face_image,
                                       const BRepAlgo_Image& edge_image,
                                       const TopoDS_Shape& result)
    : face_image_(face_image), edge_image_(edge_image) {
    if (!result.IsNull()) {
        TopExp::MapShapes(result, TopAbs_FACE, result_faces_);
        TopExp::MapShapes(result, TopAbs_EDGE, result_edges_);
        TopExp::MapShapes(result, TopAbs_VERTEX, result_vertices_);
    }
    myShape = result;
    Done();
}

const TopTools_IndexedMapOfShape* OffsetImageHistory::result_map(TopAbs_ShapeEnum kind) const {
    switch (kind) {
        case TopAbs_FACE: return &result_faces_;
        case TopAbs_EDGE: return &result_edges_;
        case TopAbs_VERTEX: return &result_vertices_;
        default: return nullptr;
    }
}

void OffsetImageHistory::collect(const TopoDS_Shape& S, TopTools_ListOfShape& out,
                                 bool& had_image) const {
    out.Clear();
    had_image = false;
    if (S.IsNull()) return;
    const TopAbs_ShapeEnum kind = S.ShapeType();
    const TopTools_IndexedMapOfShape* map = result_map(kind);
    if (map == nullptr) return;

    const BRepAlgo_Image* image = nullptr;
    if (kind == TopAbs_FACE) image = &face_image_;
    else if (kind == TopAbs_EDGE) image = &edge_image_;
    if (image == nullptr || !image->HasImage(S)) return;
    had_image = true;

    // `LastImage` walks nested image chains (`Image` is one hop). Fall back to the
    // one-hop list if the deep walk yields nothing usable.
    auto gather = [&](const TopTools_ListOfShape& src, std::vector<int>& ordinals) {
        for (TopTools_ListIteratorOfListOfShape it(src); it.More(); it.Next()) {
            if (it.Value().ShapeType() != kind) continue;  // same-kind successors only
            const int ord = map->FindIndex(it.Value());
            if (ord == 0) continue;  // did not survive into the result
            if (std::find(ordinals.begin(), ordinals.end(), ord) == ordinals.end()) {
                ordinals.push_back(ord);  // FindIndex dedupes IsSame by construction
            }
        }
    };

    std::vector<int> ordinals;
    TopTools_ListOfShape deep;
    image->LastImage(S, deep);
    gather(deep, ordinals);
    if (ordinals.empty()) gather(image->Image(S), ordinals);

    // Deterministic order: the successor's ordinal in the RESULT shape.
    std::sort(ordinals.begin(), ordinals.end());
    for (const int ord : ordinals) out.Append((*map)(ord));
}

const TopTools_ListOfShape& OffsetImageHistory::Modified(const TopoDS_Shape& S) {
    bool had_image = false;
    collect(S, cache_, had_image);
    return cache_;
}

const TopTools_ListOfShape& OffsetImageHistory::Generated(const TopoDS_Shape&) {
    // Spike fact 2: the offset carries no same-kind "generated" successors. New side
    // faces are minted on first address, not adopted here.
    return empty_;
}

Standard_Boolean OffsetImageHistory::IsDeleted(const TopoDS_Shape& S) {
    if (S.IsNull()) return Standard_False;
    const TopTools_IndexedMapOfShape* map = result_map(S.ShapeType());
    if (map != nullptr && map->Contains(S)) return Standard_False;  // survives verbatim
    TopTools_ListOfShape survivors;
    bool had_image = false;
    collect(S, survivors, had_image);
    if (!survivors.IsEmpty()) return Standard_False;
    // POSITIVE evidence only: it had an image and none of it reached the result.
    // Otherwise stay silent so apply_history emits NeedsRepair instead of dropping.
    return had_image ? Standard_True : Standard_False;
}

}  // namespace offsetface

// ---------------------------------------------------------------------------
// Executor
// ---------------------------------------------------------------------------
namespace {

namespace of = offsetface;

enum class DistanceType { Offset, Total, Radius, Diameter };
constexpr int kResultPolicyVersion2 = 2;
// WP3-C5. A SEPARATE EXECUTION PATH, never a migration: V3 answers the same stored
// record with different geometry, so a V2 record must keep executing V2 forever.
constexpr int kResultPolicyVersion3 = 3;

bool parse_distance_type(const std::string& s, DistanceType& out) {
    if (s.empty() || s == "Offset") { out = DistanceType::Offset; return true; }
    if (s == "Total") { out = DistanceType::Total; return true; }
    if (s == "Radius") { out = DistanceType::Radius; return true; }
    if (s == "Diameter") { out = DistanceType::Diameter; return true; }
    return false;
}

const char* distance_type_name(DistanceType t) {
    switch (t) {
        case DistanceType::Offset: return "Offset";
        case DistanceType::Total: return "Total";
        case DistanceType::Radius: return "Radius";
        case DistanceType::Diameter: return "Diameter";
    }
    return "Offset";
}

// One `inputs[]` slot, or a null json when the plan carries fewer refs than ids.
json input_at(const json& op, std::size_t index) {
    if (!op.contains("inputs") || !op["inputs"].is_array()) return json();
    if (index >= op["inputs"].size()) return json();
    return op["inputs"][index];
}

// A persisted record whose typed id arrays or `inputs[]` arity do not satisfy the
// §7.3 contract. Distinct from `UNSUPPORTED_OFFSET_FACE_RESULT_POLICY_VERSION`
// (a version this build does not implement) — this record is malformed at any version.
OpOutcome malformed_face_ids(const std::string& detail) {
    OpOutcome failure = OpOutcome::fail("OP_FAILED", "OffsetFace " + detail);
    failure.diagnostics.push_back({{"severity", "error"},
                                   {"code", "OFFSET_FACE_MALFORMED_FACE_IDS"},
                                   {"message", failure.error_message},
                                   {"stage", "preflight"}});
    return failure;
}

// A typed `inputs[]` slot whose own `primary.elementId` disagrees with the bare
// `faceIds[i]` it is paired with. Rust already enforces the lockstep on authoring
// (`record.rs` OffsetFaceParams::validate); the worker must not simply trust it.
OpOutcome mismatched_face_ref(const std::string& detail) {
    OpOutcome failure = OpOutcome::fail("OP_FAILED", "OffsetFace " + detail);
    failure.diagnostics.push_back({{"severity", "error"},
                                   {"code", "OFFSET_FACE_REF_MISMATCH"},
                                   {"message", failure.error_message},
                                   {"stage", "preflight"}});
    return failure;
}

// The typed `inputs[]` slot paired with a bare id must name the SAME element. Absent
// typed evidence stays legal (partition-only records), and a pre-promotion "f:N"
// TopoKey is not an ElementId so it is not comparable.
std::optional<OpOutcome> check_ref_pairing(const json& op, std::size_t index,
                                           const std::string& id, const char* label) {
    const json in = input_at(op, index);
    if (!in.is_object() || !in.contains("primary") || !in["primary"].is_object())
        return std::nullopt;
    const std::string element = read_str(in["primary"], "elementId");
    if (element.empty() || id.empty() || id.rfind("f:", 0) == 0) return std::nullopt;
    if (element == id) return std::nullopt;
    return mismatched_face_ref(std::string(label) + " typed ref names '" + element +
                               "' but its paired id is '" + id + "'");
}

json synthetic_needs_repair(const std::string& ref_id, const std::string& element_id,
                            const std::string& label) {
    return json{{"refId", ref_id},
                {"elementId", element_id},
                {"ladderFailed", "descriptor"},
                {"reason", "no-candidates"},
                {"scoringVersion", em::kResolverVersion},
                {"candidates", json::array()},
                {"anchor", json::object()},
                {"uiLabel", label}};
}

// A face reference that fell through the partition rung and needs the ladder.
struct PendingLadder {
    std::size_t slot = 0;  // index into the resolved vector
    em::LadderRef ref;
};

// Resolve `ids` (ElementIds, or snapshot-scoped "f:N" TopoKeys pre-promotion) to
// real sub-faces of `target_shape`. Rung 1 is the partition binding scoped to THIS
// body (VF-M7); rung 2 the descriptor+anchor ladder over the matching `inputs[]`
// slot. Neither ⇒ NeedsRepair STATE.
struct FaceResolution {
    std::vector<TopoDS_Face> faces;  // parallel to `ids`; null entries mean unresolved
    std::vector<json> needs_repair;
};

FaceResolution resolve_face_refs(OpContext& ctx, const json& op, const std::string& op_id,
                                 const std::string& target_id, const TopoDS_Shape& target_shape,
                                 const std::vector<std::string>& ids, std::size_t input_base) {
    FaceResolution out;
    out.faces.resize(ids.size());
    std::vector<PendingLadder> pending;

    for (std::size_t i = 0; i < ids.size(); ++i) {
        const std::string& id = ids[i];
        std::string topo_key;
        if (id.rfind("f:", 0) == 0) {
            topo_key = id;  // snapshot-scoped TopoKey, pre-promotion
        } else if (!id.empty()) {
            topo_key = em::ElementMapPartition::topokey_for_element_in_body(ctx.partition, id,
                                                                           target_id);
        }
        if (!topo_key.empty()) {
            const TopoDS_Shape sub =
                em::ElementMapPartition::shape_for_topokey(target_shape, topo_key);
            if (!sub.IsNull() && sub.ShapeType() == TopAbs_FACE) {
                out.faces[i] = TopoDS::Face(sub);
                continue;
            }
        }
        const std::string ref_id = op_id + ".input" + std::to_string(input_base + i);
        const json in = input_at(op, input_base + i);
        if (!in.is_object()) {
            out.needs_repair.push_back(
                synthetic_needs_repair(ref_id, id, "OffsetFace face ref carries no evidence: " + id));
            continue;
        }
        em::LadderRef r = em::ladder_ref_from_input(in, ref_id);
        if (r.kind == em::km::ElementKind::Unknown) r.kind = em::km::ElementKind::Face;
        if (r.kind != em::km::ElementKind::Face) {
            out.needs_repair.push_back(
                synthetic_needs_repair(ref_id, id, "OffsetFace input is not a face ref: " + id));
            continue;
        }
        pending.push_back(PendingLadder{i, std::move(r)});
    }

    if (!pending.empty()) {
        std::vector<em::LadderRef> refs;
        refs.reserve(pending.size());
        for (const PendingLadder& p : pending) refs.push_back(p.ref);
        const std::vector<em::LadderResolution> res = em::resolve_descriptor_stage(
            target_shape, target_id, refs, em::LadderEditContext{ctx.post_upstream_edit, ctx.from_zero_replay});
        for (std::size_t k = 0; k < res.size() && k < pending.size(); ++k) {
            if (res[k].outcome == em::LadderOutcome::AutoBind && !res[k].bound_shape.IsNull() &&
                res[k].bound_shape.ShapeType() == TopAbs_FACE) {
                out.faces[pending[k].slot] = TopoDS::Face(res[k].bound_shape);
            } else {
                out.needs_repair.push_back(res[k].to_needs_repair_json());
            }
        }
    }
    return out;
}

// Are two cylinder axes the SAME line (coaxial)?
bool coaxial(const gp_Ax1& a, const gp_Ax1& b, double tol) {
    if (!a.Direction().IsParallel(b.Direction(), Precision::Angular())) return false;
    return gp_Lin(a).Distance(b.Location()) <= tol;
}

struct DistancePlan {
    bool ok = false;
    std::string error;
    std::vector<double> d;         // parallel to the operative faces
    double thickness = 0.0;        // Total only (measured `t`, diagnostic)
};

DistancePlan plan_distances(DistanceType type, double distance, bool chain_flag,
                            const std::vector<TopoDS_Face>& faces,
                            const TopoDS_Face& opposite, double semantic_tol) {
    DistancePlan out;
    if (faces.empty()) {
        out.error = "OffsetFace has no operative faces";
        return out;
    }

    if (type == DistanceType::Offset) {
        out.d.assign(faces.size(), distance);
        out.ok = true;
        return out;
    }

    if (type == DistanceType::Radius || type == DistanceType::Diameter) {
        of::CylinderInfo first;
        for (std::size_t i = 0; i < faces.size(); ++i) {
            const of::CylinderInfo info = of::cylinder_info(faces[i]);
            if (!info.ok) {
                out.error = std::string(distance_type_name(type)) +
                            " requires cylindrical operative faces with a derivable "
                            "radial normal";
                return out;
            }
            if (i == 0) {
                first = info;
                continue;
            }
            if (!coaxial(first.axis, info.axis, semantic_tol)) {
                out.error = std::string(distance_type_name(type)) +
                            " requires a COAXIAL cylindrical operative set";
                return out;
            }
            if (std::abs(first.radius - info.radius) > semantic_tol) {
                out.error = std::string(distance_type_name(type)) +
                            " requires an EQUAL-RADIUS cylindrical operative set";
                return out;
            }
            if (first.sigma != info.sigma) {
                out.error = std::string(distance_type_name(type)) +
                            " requires a SAME-σ cylindrical operative set";
                return out;
            }
        }
        const double target = (type == DistanceType::Radius) ? distance : distance * 0.5;
        const double d = static_cast<double>(first.sigma) * (target - first.radius);
        // Preflight: OCCT happily returns a "valid" inside-out cylinder for
        // R + σd ≤ 0 (spike trap a). Never clamp — refuse.
        if (first.radius + static_cast<double>(first.sigma) * d <= semantic_tol) {
            out.error = std::string(distance_type_name(type)) +
                        " would drive the cylinder radius to " +
                        std::to_string(first.radius + static_cast<double>(first.sigma) * d) +
                        " (must exceed " + std::to_string(semantic_tol) + ")";
            return out;
        }
        out.d.assign(faces.size(), d);
        out.ok = true;
        return out;
    }

    // --- Total ---------------------------------------------------------------
    if (faces.size() != 1) {
        out.error = "Total distance requires exactly one operative face";
        return out;
    }
    if (chain_flag) {
        out.error = "Total distance requires chainTangentFaces:false";
        return out;
    }
    if (of::surface_kind(faces[0]) != of::SurfaceKind::Plane) {
        out.error = "Total distance requires a planar operative face";
        return out;
    }
    if (opposite.IsNull() || of::surface_kind(opposite) != of::SurfaceKind::Plane) {
        out.error = "Total distance requires a planar oppositeFaceId reference";
        return out;
    }
    const of::PlaneInfo sel = of::plane_info(faces[0]);
    const of::PlaneInfo opp = of::plane_info(opposite);
    if (!sel.ok || !opp.ok || !of::sample_face(faces[0]).ok ||
        !of::sample_face(opposite).ok) {
        out.error = "Total distance could not prove the selected/opposite trimmed planes";
        return out;
    }
    const double anti = gp_Vec(sel.normal).Dot(gp_Vec(opp.normal));
    // `semantic_angular()` carries no conditioning term by design — adding a
    // length-derived quantity to an angle is a dimensional error (GeometryPrecision.cpp)
    // — so the floor-only context returns exactly the angular floor.
    if (anti > -std::cos(kernel::validation::GeometryPrecisionContext{}.semantic_angular())) {
        out.error = "Total distance requires ANTI-PARALLEL selected/opposite planes";
        return out;
    }
    const double t = gp_Vec(opp.location, sel.location).Dot(gp_Vec(sel.normal));
    if (t <= semantic_tol) {
        out.error = "Total distance: the opposite face is not at a positive thickness";
        return out;
    }
    out.thickness = t;
    out.d.assign(1, distance - t);
    out.ok = true;
    return out;
}

// Successors of `face` that survive in `result`, via the offset face image.
std::vector<TopoDS_Face> offset_successors(const BRepAlgo_Image& image, const TopoDS_Face& face,
                                           const TopTools_IndexedMapOfShape& result_faces) {
    std::vector<int> ordinals;
    if (image.HasImage(face)) {
        auto gather = [&](const TopTools_ListOfShape& src) {
            for (TopTools_ListIteratorOfListOfShape it(src); it.More(); it.Next()) {
                if (it.Value().ShapeType() != TopAbs_FACE) continue;
                const int ord = result_faces.FindIndex(it.Value());
                if (ord == 0) continue;
                if (std::find(ordinals.begin(), ordinals.end(), ord) == ordinals.end()) {
                    ordinals.push_back(ord);
                }
            }
        };
        TopTools_ListOfShape deep;
        image.LastImage(face, deep);
        gather(deep);
        if (ordinals.empty()) gather(image.Image(face));
    }
    std::sort(ordinals.begin(), ordinals.end());
    std::vector<TopoDS_Face> out;
    out.reserve(ordinals.size());
    for (const int ord : ordinals) out.push_back(TopoDS::Face(result_faces(ord)));
    return out;
}

// SEMANTIC postconditions (SCHEMA §7.3): each operated plane moved by EXACTLY its
// signed `d`, each operated cylinder stayed coaxial at the predicted radius. IsDone
// + BRepCheck cannot see either (spike traps a/b).
std::string check_semantics(const BRepOffset_MakeOffset& mo, const TopoDS_Shape& result,
                            const std::vector<TopoDS_Face>& faces, const std::vector<double>& d,
                            const std::vector<std::string>& keys) {
    TopTools_IndexedMapOfShape result_faces;
    TopExp::MapShapes(result, TopAbs_FACE, result_faces);
    const BRepAlgo_Image& image = mo.OffsetFacesFromShapes();
    // Measured on the RESULT, which is the shape every assertion below is taken
    // against. Its conditioning term cannot reach the semantic floor anywhere in the
    // supported coordinate range — that is asserted in `test_geometry_precision`.
    const auto precision = kernel::validation::precision_of(result);

    for (std::size_t i = 0; i < faces.size(); ++i) {
        const std::vector<TopoDS_Face> succ = offset_successors(image, faces[i], result_faces);
        if (succ.empty()) {
            return "operated face " + keys[i] + " has no surviving successor (V1 refuses face "
                                                "disappearance)";
        }
        const of::SurfaceKind kind = of::surface_kind(faces[i]);
        if (kind == of::SurfaceKind::Plane) {
            const of::PlaneInfo before = of::plane_info(faces[i]);
            if (!before.ok || !of::sample_face(faces[i]).ok)
                return "operated face " + keys[i] + " has no provable trimmed-plane sample";
            for (const TopoDS_Face& sf : succ) {
                const of::PlaneInfo after = of::plane_info(sf);
                if (!after.ok) {
                    return "operated plane " + keys[i] + " did not stay planar";
                }
                if (!of::sample_face(sf).ok) {
                    return "operated plane " + keys[i] +
                           " successor has no provable trimmed-domain sample";
                }
                const double alignment =
                    gp_Vec(before.normal).Dot(gp_Vec(after.normal));
                if (alignment < std::cos(precision.semantic_angular())) {
                    return "operated plane " + keys[i] + " changed orientation";
                }
                const double moved = gp_Vec(before.location, after.location)
                                         .Dot(gp_Vec(before.normal));
                if (std::abs(moved - d[i]) > precision.semantic_length()) {
                    return "operated plane " + keys[i] + " moved " + std::to_string(moved) +
                           " but " + std::to_string(d[i]) + " was requested";
                }
            }
        } else if (kind == of::SurfaceKind::Cylinder) {
            const of::CylinderInfo before = of::cylinder_info(faces[i]);
            if (!before.ok) return "operated cylinder " + keys[i] + " is not measurable";
            const double predicted =
                before.radius + static_cast<double>(before.sigma) * d[i];
            for (const TopoDS_Face& sf : succ) {
                if (of::surface_kind(sf) != of::SurfaceKind::Cylinder) {
                    return "operated cylinder " + keys[i] + " did not stay cylindrical";
                }
                BRepAdaptor_Surface surf(sf);
                const gp_Cylinder cyl = surf.Cylinder();
                if (!coaxial(before.axis, cyl.Axis(), precision.semantic_length())) {
                    return "operated cylinder " + keys[i] + " lost its axis";
                }
                if (std::abs(cyl.Radius() - predicted) > precision.semantic_length()) {
                    return "operated cylinder " + keys[i] + " reached radius " +
                           std::to_string(cyl.Radius()) + " but " + std::to_string(predicted) +
                           " was predicted";
                }
            }
        } else {
            return "operated face " + keys[i] + " is neither planar nor cylindrical (V1 scope)";
        }
    }
    return {};
}

std::string join_keys(const std::vector<std::string>& keys) {
    std::string s;
    for (std::size_t i = 0; i < keys.size(); ++i) {
        if (i != 0) s += ", ";
        s += keys[i];
    }
    return s;
}

// ---------------------------------------------------------------------------
// V3 (`resultPolicyVersion: 3`) — suppress → offset → reblend
//
// EVERYTHING BELOW IS V3-ONLY. The V2 build sequence above is spike-frozen and is
// deliberately not refactored into a shared helper: a V2 record's geometry is
// frozen, so the safest possible diff for it is NO diff. The duplication here is
// therefore intentional, and it is bounded — the semantic postconditions
// (`check_semantics`), the closure walk, the distance planning and the publication
// gate are all shared, because those are the parts that must never drift.
// ---------------------------------------------------------------------------
namespace kf = onecad::kernel::fillet;

// A refusal from one of the three kernel stages, in the taxonomy §7.3 uses:
// OP_FAILED at the top level with the fine-grained code on `diagnostics[]`. The one
// exception is a refusal that ORIGINATES in a `PublicationDecision`, which keeps its
// own GEOMETRY_INVALID + `reasonCode` shape instead of being re-wrapped.
struct StageRefusal {
    std::string code;
    std::string message;
    std::string stage;
    json evidence = json::object();
    bool publication = false;
    std::string reason_code;
};

// `BlendReconstruction`'s reasons are "CODE: measured detail". Split them so the code
// rides `diagnostics[].code` under the op's own prefix and the module's measurement
// survives verbatim in the message — the refusal has to say what it measured, not
// just that it refused.
std::pair<std::string, std::string> split_reason(const std::string& reason) {
    const std::size_t colon = reason.find(':');
    if (colon == std::string::npos) return {reason, std::string()};
    std::string detail = reason.substr(colon + 1);
    if (!detail.empty() && detail.front() == ' ') detail.erase(0, 1);
    return {reason.substr(0, colon), std::move(detail)};
}

StageRefusal reconstruction_refusal(const std::string& reason, const char* stage) {
    const auto [token, detail] = split_reason(reason);
    StageRefusal out;
    out.stage = stage;
    if (token == "SUPPRESSION_NOT_PUBLISHABLE") {
        // The module folded a `PublicationDecision` into a string; unfold it so this
        // refusal leaves through the same door every other publication refusal in the
        // worker uses, rather than appearing twice in two taxonomies. The structured
        // evidence is not recoverable from the string and is deliberately left empty.
        const auto [reason_code, message] = split_reason(detail);
        out.publication = true;
        out.reason_code = reason_code;
        out.message = message.empty() ? detail : message;
        return out;
    }
    out.code = "OFFSET_FACE_" + token;
    out.message = detail.empty() ? reason : detail;
    return out;
}

StageRefusal stage_refusal(const char* code, const std::string& message, const char* stage,
                           json evidence = json::object()) {
    StageRefusal out;
    out.code = code;
    out.message = message;
    out.stage = stage;
    out.evidence = std::move(evidence);
    return out;
}

// The three kernel stages, plus everything the element map and the publication gate
// need from them. Nothing is published from here — a failed pipeline carries no shape.
struct BlendPipeline {
    bool ok = false;
    StageRefusal refusal;
    TopoDS_Shape shape;
    // Execution order: defeature, offset, re-fillet. Each is already ingested into a
    // `BRepTools_History`, so the algorithms that produced them may die.
    std::vector<occ::handle<BRepTools_History>> stages;
    // input blend face ⇒ its reconstruction, asserted onto the composed history ONLY
    // because `reblend`'s reproduction proof passed on that pair.
    std::vector<std::pair<TopoDS_Face, TopoDS_Face>> lineage;
    double suppress_contribution = 0.0;
    double offset_contribution = 0.0;
    double reblend_contribution = 0.0;
};

double maximum_tolerance_of(const TopoDS_Shape& shape) {
    return kernel::validation::audit_shape(shape).tolerances.maximum();
}

// The single solid of `shape`, or a null shape when it does not carry exactly one.
TopoDS_Shape single_solid(const TopoDS_Shape& shape) {
    if (shape.IsNull()) return TopoDS_Shape();
    if (shape.ShapeType() == TopAbs_SOLID) return shape;
    const std::vector<TopoDS_Shape> solids = ordered_solids(shape);
    return solids.size() == 1 ? solids[0] : TopoDS_Shape();
}

// Images of `face` in `after` through a `BRepTools_History` stage, filtered to the
// result and deduplicated by result ordinal. A face the stage never touched is its
// own image — the stage-crossing convention `ComposedHistory` documents.
std::vector<TopoDS_Face> history_face_images(const BRepTools_History& history,
                                             const TopoDS_Face& face,
                                             const TopTools_IndexedMapOfShape& after) {
    std::vector<int> ordinals;
    for (TopTools_ListIteratorOfListOfShape it(history.Modified(face)); it.More(); it.Next()) {
        const int ord = after.FindIndex(it.Value());
        if (ord != 0 && std::find(ordinals.begin(), ordinals.end(), ord) == ordinals.end())
            ordinals.push_back(ord);
    }
    if (ordinals.empty() && !history.IsRemoved(face)) {
        const int ord = after.FindIndex(face);
        if (ord != 0) ordinals.push_back(ord);
    }
    std::sort(ordinals.begin(), ordinals.end());
    std::vector<TopoDS_Face> out;
    out.reserve(ordinals.size());
    for (const int ord : ordinals) out.push_back(TopoDS::Face(after(ord)));
    return out;
}

// The offset image of ONE shape, by the same `LastImage`/`Image` walk
// `offset_successors` uses for faces, with the "survived verbatim" fallback the
// image map does not record.
std::vector<TopoDS_Shape> offset_image(const BRepAlgo_Image& image, const TopoDS_Shape& shape,
                                       const TopTools_IndexedMapOfShape& result_map) {
    std::vector<int> ordinals;
    if (image.HasImage(shape)) {
        auto gather = [&](const TopTools_ListOfShape& src) {
            for (TopTools_ListIteratorOfListOfShape it(src); it.More(); it.Next()) {
                if (it.Value().ShapeType() != shape.ShapeType()) continue;
                const int ord = result_map.FindIndex(it.Value());
                if (ord == 0) continue;
                if (std::find(ordinals.begin(), ordinals.end(), ord) == ordinals.end())
                    ordinals.push_back(ord);
            }
        };
        TopTools_ListOfShape deep;
        image.LastImage(shape, deep);
        gather(deep);
        if (ordinals.empty()) gather(image.Image(shape));
    }
    if (ordinals.empty()) {
        const int ord = result_map.FindIndex(shape);
        if (ord != 0) ordinals.push_back(ord);
    }
    std::sort(ordinals.begin(), ordinals.end());
    std::vector<TopoDS_Shape> out;
    out.reserve(ordinals.size());
    for (const int ord : ordinals) out.push_back(result_map(ord));
    return out;
}

std::vector<TopoDS_Edge> shared_edges_of(const TopoDS_Face& a, const TopoDS_Face& b) {
    TopTools_IndexedMapOfShape b_edges;
    TopExp::MapShapes(b, TopAbs_EDGE, b_edges);
    std::vector<TopoDS_Edge> out;
    for (TopExp_Explorer ex(a, TopAbs_EDGE); ex.More(); ex.Next()) {
        if (b_edges.Contains(ex.Current())) out.push_back(TopoDS::Edge(ex.Current()));
    }
    return out;
}

double edge_length_of(const TopoDS_Edge& edge) {
    GProp_GProps props;
    BRepGProp::LinearProperties(edge, props);
    return props.Mass();
}

// Every gate the V2 path runs on its result, in the same order and with the same
// wording. V3 runs it ONCE, on the INTERMEDIATE offset body, so nothing is rebuilt on
// a body the kernel already broke. The FINAL body is not checked here: `reblend` runs
// `FilletBuilder`, whose own TierB publication decision covers the same ground
// (structure, BRep validity, single solid, positive volume, tolerances), and the
// executor then evaluates the shared `single_solid_policy` on the published shape.
std::optional<StageRefusal> check_result_shape(const TopoDS_Shape& shape, const char* stage) {
    if (shape.IsNull())
        return stage_refusal("OFFSET_FACE_STAGE_NULL_SHAPE", "produced a null shape", stage);
    if (!BRepCheck_Analyzer(shape).IsValid())
        return stage_refusal("OFFSET_FACE_STAGE_INVALID_SHAPE", "produced an invalid shape",
                             stage);
    const std::vector<RankedSolid> solids = ranked_solids(shape);
    if (solids.size() != 1) {
        return stage_refusal("OFFSET_FACE_STAGE_NOT_SINGLE_SOLID",
                             "produced " + std::to_string(solids.size()) +
                                 " solids; exactly one is required",
                             stage);
    }
    GProp_GProps props;
    BRepGProp::VolumeProperties(shape, props);
    const double volume = props.Mass();
    if (!std::isfinite(volume) ||
        volume <= kernel::validation::precision_of(shape).minimum_volume()) {
        return stage_refusal("OFFSET_FACE_STAGE_DEGENERATE_VOLUME",
                             "produced a degenerate volume " + std::to_string(volume), stage);
    }
    BRepAlgoAPI_Check checker;
    checker.SetData(shape, /*bTestSE*/ Standard_False, /*bTestSI*/ Standard_True);
    checker.SetRunParallel(Standard_False);
    try {
        checker.Perform();
        if (!checker.IsValid())
            return stage_refusal("OFFSET_FACE_STAGE_SELF_INTERFERENCE",
                                 "produced a self-interfering shape", stage);
    } catch (const Standard_Failure&) {
        return stage_refusal("OFFSET_FACE_STAGE_SELF_INTERFERENCE",
                             "self-interference check raised", stage);
    }
    return std::nullopt;
}

// SUPPRESS → OFFSET → REBLEND on `build_input`. `moving` and `moving_keys` are C_op
// addressed in the INPUT body; `blends` is B, already certified by the closure walk.
//
// TOLERANCE. Each stage's CONTRIBUTION is capped at `kRoundTripToleranceCapMm`, the
// number `BlendReconstruction` characterized for the two stages it owns; this adds
// the offset stage under the same per-stage ceiling. The caps are deliberately NOT
// summed. A summed cap would bound the chain at 3x a number no single stage was ever
// measured against, and would refuse a body whose three stages each behaved exactly
// as characterized; the absolute backstop stays where it already is, on the shared
// publication policy the caller evaluates on the published shape.
BlendPipeline run_blend_pipeline(const TopoDS_Shape& build_input,
                                 const std::vector<TopoDS_Face>& moving,
                                 const std::vector<std::string>& moving_keys,
                                 const std::vector<of::ClosureBlend>& blends, double distance,
                                 const onecad::CancelToken* cancel) {
    BlendPipeline out;

    // --- stage 1: suppress ----------------------------------------------------
    std::vector<kf::RecognizedBlend> recognized;
    recognized.reserve(blends.size());
    for (const of::ClosureBlend& entry : blends) recognized.push_back(entry.blend);
    const kf::SuppressionResult suppressed = kf::suppress_blends(build_input, recognized);
    if (!suppressed.ok) {
        out.refusal = reconstruction_refusal(suppressed.reason, "suppress");
        return out;
    }
    out.suppress_contribution = suppressed.maximum_tolerance - suppressed.input_tolerance;
    const TopoDS_Shape suppressed_solid = single_solid(suppressed.suppressed_shape);
    if (suppressed_solid.IsNull()) {
        out.refusal = stage_refusal("OFFSET_FACE_SUPPRESSION_NOT_SINGLE_SOLID",
                                    "the suppressed body is not one solid", "suppress");
        return out;
    }
    if (cancel != nullptr && cancel->cancelled()) return out;  // caller re-checks

    // --- re-address C_op through the defeaturing ------------------------------
    TopTools_IndexedMapOfShape suppressed_faces;
    TopExp::MapShapes(suppressed_solid, TopAbs_FACE, suppressed_faces);
    std::vector<TopoDS_Face> moving_suppressed;
    std::vector<int> moving_suppressed_ordinals;
    for (std::size_t i = 0; i < moving.size(); ++i) {
        const std::vector<TopoDS_Face> images =
            history_face_images(*suppressed.history, moving[i], suppressed_faces);
        if (images.size() != 1) {
            // DISTINCT from `BlendReconstruction`'s own `SUPPRESSION_ANCESTRY_NOT_UNIQUE`,
            // which `reconstruction_refusal` maps to `OFFSET_FACE_SUPPRESSION_ANCESTRY_
            // NOT_UNIQUE`: that one is about a SURVIVING face having other than one
            // ancestor, this one is about an OPERATIVE face having other than one image.
            // Two conditions must never share one machine-readable code.
            out.refusal = stage_refusal(
                "OFFSET_FACE_OFFSET_ANCESTRY_NOT_UNIQUE",
                "operative face " + moving_keys[i] + " has " + std::to_string(images.size()) +
                    " images in the suppressed body, so the offset has no unique face to move",
                "offset");
            return out;
        }
        moving_suppressed.push_back(images.front());
        moving_suppressed_ordinals.push_back(suppressed_faces.FindIndex(images.front()));
    }

    // THE CHECK THAT SUPPRESSION ISOLATED THE DESIGN FACES. If a G1 chain still runs
    // out of the moving set on the suppressed body, something tangent to it was not
    // recognized as a blend — and `BRepOffset_MakeOffset` would silently propagate
    // across it (Phase-0 spike fact 1), which is the exact V2 defect V3 exists to fix.
    {
        std::vector<int> seeds = moving_suppressed_ordinals;
        std::sort(seeds.begin(), seeds.end());
        seeds.erase(std::unique(seeds.begin(), seeds.end()), seeds.end());
        const of::ClosureResult residual = of::tangent_closure(suppressed_solid, seeds);
        if (!residual.ok) {
            out.refusal = stage_refusal("OFFSET_FACE_DEPENDENT_NOT_A_BLEND",
                                        residual.non_manifold
                                            ? "the suppressed body is non-manifold at the "
                                              "operative set"
                                            : "the suppressed tangent-closure walk failed",
                                        "suppress");
            return out;
        }
        std::vector<int> extra;
        std::set_difference(residual.ordinals.begin(), residual.ordinals.end(), seeds.begin(),
                            seeds.end(), std::back_inserter(extra));
        if (!extra.empty()) {
            std::vector<std::string> keys;
            for (const int ord : extra) keys.push_back(of::face_topokey(ord));
            out.refusal = stage_refusal(
                "OFFSET_FACE_DEPENDENT_NOT_A_BLEND",
                "the operative set is still G1-chained to " + join_keys(keys) +
                    " on the suppressed body (unrecognized dependent geometry)",
                "suppress");
            return out;
        }
    }

    // --- stage 2: offset the isolated design faces ----------------------------
    // Deliberately a second copy of the spike-frozen call sequence rather than a
    // helper shared with the V2 path above; see the section note.
    const double construction_tol = of::build_tolerance(suppressed_solid);
    BRepOffset_MakeOffset mo;
    TopoDS_Shape offset_shape;
    const std::vector<double> distances(moving_suppressed.size(), distance);
    try {
        mo.Initialize(suppressed_solid, /*Offset*/ 0.0, construction_tol, BRepOffset_Skin,
                      /*Intersection*/ Standard_False, /*SelfInter*/ Standard_False,
                      GeomAbs_Intersection, /*Thickening*/ Standard_False,
                      /*RemoveIntEdges*/ Standard_False);
        mo.AllowLinearization(Standard_False);
        for (std::size_t i = 0; i < moving_suppressed.size(); ++i) {
            mo.SetOffsetOnFace(moving_suppressed[i], distances[i]);
        }
        mo.MakeOffsetShape();
        if (!mo.IsDone() || mo.Error() != BRepOffset_NoError) {
            out.refusal = stage_refusal(
                "OFFSET_FACE_STAGE_KERNEL_REFUSED",
                "the kernel refused the offset (BRepOffset_Error " +
                    std::to_string(static_cast<int>(mo.Error())) + ")",
                "offset");
            return out;
        }
        offset_shape = mo.Shape();
    } catch (const Standard_Failure& f) {
        out.refusal = stage_refusal("OFFSET_FACE_STAGE_KERNEL_REFUSED",
                                    std::string("kernel exception: ") +
                                        (f.GetMessageString() ? f.GetMessageString() : "OCCT"),
                                    "offset");
        return out;
    } catch (...) {
        out.refusal =
            stage_refusal("OFFSET_FACE_STAGE_KERNEL_REFUSED", "unknown kernel exception", "offset");
        return out;
    }
    if (std::optional<StageRefusal> bad = check_result_shape(offset_shape, "offset")) {
        out.refusal = std::move(*bad);
        return out;
    }
    {
        const std::string semantic =
            check_semantics(mo, offset_shape, moving_suppressed, distances, moving_keys);
        if (!semantic.empty()) {
            out.refusal = stage_refusal("OFFSET_FACE_STAGE_SEMANTICS", semantic, "offset");
            return out;
        }
    }
    const TopoDS_Shape offset_solid = single_solid(offset_shape);
    if (offset_solid.IsNull()) {
        out.refusal = stage_refusal("OFFSET_FACE_STAGE_NOT_SINGLE_SOLID",
                                    "the offset body is not one solid", "offset");
        return out;
    }
    out.offset_contribution = maximum_tolerance_of(offset_shape) - suppressed.maximum_tolerance;
    if (out.offset_contribution > kf::kRoundTripToleranceCapMm) {
        out.refusal = stage_refusal(
            "OFFSET_FACE_OFFSET_TOLERANCE_CAP",
            "the offset stage added " + std::to_string(out.offset_contribution) +
                " mm of B-Rep tolerance, over the characterized per-stage cap " +
                std::to_string(kf::kRoundTripToleranceCapMm) + " mm",
            "offset",
            json{{"stageContributionMm", out.offset_contribution},
                 {"perStageCapMm", kf::kRoundTripToleranceCapMm}});
        return out;
    }
    if (cancel != nullptr && cancel->cancelled()) return out;

    // --- carry the seeds across the offset ------------------------------------
    TopTools_IndexedMapOfShape offset_faces;
    TopTools_IndexedMapOfShape offset_edges;
    TopExp::MapShapes(offset_solid, TopAbs_FACE, offset_faces);
    TopExp::MapShapes(offset_solid, TopAbs_EDGE, offset_edges);
    const BRepAlgo_Image& face_image = mo.OffsetFacesFromShapes();
    const BRepAlgo_Image& edge_image = mo.OffsetEdgesFromShapes();
    std::vector<kf::SuppressedBlendSeed> seeds;
    for (std::size_t i = 0; i < suppressed.seed_edges.size(); ++i) {
        const kf::SuppressedBlendSeed& seed = suppressed.seed_edges[i];
        const std::string at = " (blend " + std::to_string(i) + ")";
        TopoDS_Face support_image[2];
        const TopoDS_Face* supports[2] = {&seed.support_a, &seed.support_b};
        for (int s = 0; s < 2; ++s) {
            const std::vector<TopoDS_Shape> images =
                offset_image(face_image, *supports[s], offset_faces);
            if (images.size() != 1) {
                out.refusal = stage_refusal(
                    "OFFSET_FACE_OFFSET_SUPPORT_NOT_UNIQUE",
                    "blend support " + std::to_string(s) + " has " +
                        std::to_string(images.size()) + " images after the offset" + at,
                    "offset");
                return out;
            }
            support_image[s] = TopoDS::Face(images.front());
        }
        const std::vector<TopoDS_Shape> edge_images =
            offset_image(edge_image, seed.edge, offset_edges);
        if (edge_images.size() != 1) {
            out.refusal = stage_refusal("OFFSET_FACE_OFFSET_SEED_NOT_UNIQUE",
                                        "the suppressed seed edge has " +
                                            std::to_string(edge_images.size()) +
                                            " images after the offset" + at,
                                        "offset");
            return out;
        }
        const TopoDS_Edge offset_seed = TopoDS::Edge(edge_images.front());
        const std::vector<TopoDS_Edge> shared =
            shared_edges_of(support_image[0], support_image[1]);
        if (std::none_of(shared.begin(), shared.end(), [&offset_seed](const TopoDS_Edge& e) {
                return e.IsSame(offset_seed);
            })) {
            out.refusal = stage_refusal(
                "OFFSET_FACE_OFFSET_SEED_NOT_SHARED",
                "the offset seed edge is no longer shared by both blend supports" + at,
                "offset");
            return out;
        }
        // The extent the REBUILD is held to is the seed's extent AFTER the offset,
        // measured. The pre-offset length is the wrong yardstick: the offset stage is
        // allowed to change the corner run, and holding the rebuild to a length the
        // geometry legitimately no longer has would refuse a correct result. What the
        // check still buys is layer 3's guarantee — a fillet that ran past the seed
        // edge is refused, which is what kills the partially-trimmed round.
        seeds.push_back({offset_seed, support_image[0], support_image[1],
                         edge_length_of(offset_seed), seed.radius, seed.convexity});
    }

    // --- stage 3: reblend -----------------------------------------------------
    const kf::ReblendResult rebuilt =
        kf::reblend(offset_solid, seeds, recognized.front().radius);
    if (!rebuilt.ok) {
        out.refusal = reconstruction_refusal(rebuilt.reason, "reblend");
        return out;
    }
    out.reblend_contribution = rebuilt.maximum_tolerance - rebuilt.input_tolerance;
    if (rebuilt.blend_faces.size() != recognized.size()) {
        out.refusal = stage_refusal("OFFSET_FACE_REBLEND_NOT_REPRODUCED",
                                    "rebuilt " + std::to_string(rebuilt.blend_faces.size()) +
                                        " blend faces for " + std::to_string(recognized.size()) +
                                        " suppressed blends",
                                    "reblend");
        return out;
    }

    out.stages.push_back(suppressed.history);
    {
        of::OffsetImageHistory offset_history(face_image, edge_image, offset_shape);
        TopTools_ListOfShape arguments;
        arguments.Append(suppressed_solid);
        out.stages.push_back(
            occ::handle<BRepTools_History>(new BRepTools_History(arguments, offset_history)));
    }
    out.stages.push_back(rebuilt.history);
    for (std::size_t i = 0; i < recognized.size(); ++i) {
        out.lineage.emplace_back(recognized[i].face, rebuilt.blend_faces[i]);
    }
    out.ok = true;
    out.shape = rebuilt.shape;
    return out;
}

// The §7.3 refusal shape: OP_FAILED at the top level, the fine-grained code on
// `diagnostics[]` — except a `PublicationDecision` refusal, which keeps the
// GEOMETRY_INVALID + `reasonCode` form every other publication refusal uses.
OpOutcome blend_refusal(const StageRefusal& refusal) {
    if (refusal.publication) {
        kernel::validation::PublicationDecision decision;
        decision.disposition = kernel::validation::PublicationDisposition::Refused;
        decision.code = "GEOMETRY_INVALID";
        decision.reason_code = refusal.reason_code;
        decision.message = "OffsetFace: " + refusal.message;
        return publication_refusal(decision, refusal.stage.c_str());
    }
    const std::string message = "OffsetFace: " + refusal.message;
    OpOutcome failure = OpOutcome::fail("OP_FAILED", message);
    json diagnostic = {{"severity", "error"},
                       {"code", refusal.code},
                       {"message", message},
                       {"stage", refusal.stage}};
    if (!refusal.evidence.empty()) diagnostic["evidence"] = refusal.evidence;
    failure.diagnostics.push_back(std::move(diagnostic));
    return failure;
}

}  // namespace

OpOutcome execute_offset_face(OpContext& ctx, const json& op, const std::string& op_id) {
    const json params =
        (op.contains("params") && op["params"].is_object()) ? op["params"] : json::object();

    if (std::vector<json> repairs = operation_ref_ownership_repairs(op, op_id); !repairs.empty()) {
        OpOutcome out;
        out.needs_repair = std::move(repairs);
        return out;
    }

    // --- target body (MANDATORY — SCHEMA §7.3) --------------------------------
    const std::string target_id = read_str(params, "targetBodyId");
    if (target_id.empty()) {
        return OpOutcome::fail("OP_FAILED", "OffsetFace requires params.targetBodyId");
    }
    const session::BodyRecord* target_rec = ctx.bodies.get(target_id);
    if (!target_rec) {
        return OpOutcome::fail("REF_UNRESOLVED",
                               "OffsetFace target body not found: " + target_id);
    }
    const TopoDS_Shape target_shape = target_rec->geom;
    if (auto invalid = validate_modeling_body(*target_rec, "OffsetFace", "target")) return *invalid;
    if (target_shape.IsNull()) {
        return OpOutcome::fail("REF_UNRESOLVED", "OffsetFace target body has no geometry");
    }

    // --- params ---------------------------------------------------------------
    // Typed arrays are refused, never filtered: `faceIds[i]` pairs POSITIONALLY with
    // `inputs[i]` (and the Total opposite sits at slot `faceIds.size()`), so dropping
    // one malformed element would silently rebind every remaining ref to its
    // neighbour's evidence.
    std::vector<std::string> face_ids;
    std::string ids_error;
    if (!read_string_array_strict(params, "faceIds", face_ids, ids_error)) {
        return malformed_face_ids(ids_error);
    }
    if (face_ids.empty()) {
        return OpOutcome::fail("OP_FAILED", "OffsetFace requires a non-empty faceIds");
    }
    std::vector<std::string> primary_face_ids;
    // 0 == ABSENT, which stays the legacy contract (no `primaryFaceIds`, V2 geometry).
    int policy_version = 0;
    if (params.contains("resultPolicyVersion")) {
        if (!params["resultPolicyVersion"].is_number_integer() ||
            (params["resultPolicyVersion"].get<int>() != kResultPolicyVersion2 &&
             params["resultPolicyVersion"].get<int>() != kResultPolicyVersion3)) {
            OpOutcome failure = OpOutcome::fail(
                "OP_FAILED",
                "OffsetFace resultPolicyVersion is unsupported (supported: 2, 3)");
            failure.diagnostics.push_back(
                {{"severity", "error"},
                 {"code", "UNSUPPORTED_OFFSET_FACE_RESULT_POLICY_VERSION"},
                 {"message", failure.error_message},
                 {"stage", "preflight"}});
            return failure;
        }
        policy_version = params["resultPolicyVersion"].get<int>();
        if (!read_string_array_strict(params, "primaryFaceIds", primary_face_ids,
                                      ids_error)) {
            return malformed_face_ids(ids_error);
        }
        if (primary_face_ids.empty()) {
            return OpOutcome::fail("OP_FAILED",
                                   "OffsetFace resultPolicyVersion " +
                                       std::to_string(policy_version) +
                                       " requires primaryFaceIds");
        }
        std::set<std::string> seen_primary;
        for (const std::string& id : primary_face_ids) {
            if (std::find(face_ids.begin(), face_ids.end(), id) == face_ids.end()) {
                return OpOutcome::fail(
                    "OP_FAILED",
                    "OffsetFace primaryFaceIds must be a subset of faceIds");
            }
            if (!seen_primary.insert(id).second) {
                return OpOutcome::fail("OP_FAILED",
                                       "OffsetFace primaryFaceIds contains duplicates");
            }
        }
    } else if (params.contains("primaryFaceIds")) {
        return OpOutcome::fail(
            "OP_FAILED",
            "OffsetFace primaryFaceIds requires resultPolicyVersion 2 or 3");
    }
    // Presence is asked separately from the value: `read_scalar`'s default cannot
    // distinguish an ABSENT `distance` (a malformed record) from a legitimate 0, and
    // 0 would silently take the identity-no-op success path.
    const bool has_distance =
        params.contains("distance") &&
        (params["distance"].is_number() || (params["distance"].is_object() &&
                                            params["distance"].contains("value") &&
                                            params["distance"]["value"].is_number()));
    if (!has_distance) {
        return OpOutcome::fail("OP_FAILED", "OffsetFace requires params.distance");
    }
    const double distance = read_scalar(params, "distance", 0.0);
    if (!std::isfinite(distance)) {
        return OpOutcome::fail("OP_FAILED", "OffsetFace distance is not finite");
    }
    DistanceType type = DistanceType::Offset;
    const std::string type_name = read_str(params, "distanceType", "Offset");
    if (!parse_distance_type(type_name, type)) {
        return OpOutcome::fail("OP_FAILED", "OffsetFace unknown distanceType: " + type_name);
    }
    const bool chain_flag = params.contains("chainTangentFaces") &&
                                    params["chainTangentFaces"].is_boolean()
                                ? params["chainTangentFaces"].get<bool>()
                                : true;
    const std::string opposite_id = read_str(params, "oppositeFaceId");
    if (type == DistanceType::Total && opposite_id.empty()) {
        return OpOutcome::fail("OP_FAILED", "Total distanceType requires oppositeFaceId");
    }

    // --- `inputs[]` contract (SCHEMA §7.3: one typed ref per face, same order, the
    //     Total opposite LAST). Absent/empty inputs remains the legal partition-only
    //     record; a PRESENT array of the wrong length would silently pair each id
    //     with a neighbour's evidence.
    const std::size_t expected_inputs =
        face_ids.size() + (type == DistanceType::Total ? 1u : 0u);
    if (op.contains("inputs") && op["inputs"].is_array() && !op["inputs"].empty() &&
        op["inputs"].size() != expected_inputs) {
        return malformed_face_ids("inputs carries " + std::to_string(op["inputs"].size()) +
                                  " refs for " + std::to_string(expected_inputs) +
                                  " ids");
    }
    for (std::size_t i = 0; i < face_ids.size(); ++i) {
        if (std::optional<OpOutcome> bad = check_ref_pairing(op, i, face_ids[i], "faceIds"))
            return *bad;
    }
    if (type == DistanceType::Total) {
        if (std::optional<OpOutcome> bad =
                check_ref_pairing(op, face_ids.size(), opposite_id, "oppositeFaceId"))
            return *bad;
    }

    // --- resolve the operative faces (+ the Total opposite, LAST slot) --------
    OpOutcome out;
    FaceResolution resolved =
        resolve_face_refs(ctx, op, op_id, target_id, target_shape, face_ids, 0);
    TopoDS_Face opposite;
    if (type == DistanceType::Total) {
        FaceResolution opp = resolve_face_refs(ctx, op, op_id, target_id, target_shape,
                                               {opposite_id}, face_ids.size());
        for (json& nr : opp.needs_repair) resolved.needs_repair.push_back(std::move(nr));
        if (!opp.faces.empty()) opposite = opp.faces[0];
    }
    if (!resolved.needs_repair.empty()) {
        out.needs_repair = std::move(resolved.needs_repair);
        WLOG_DEBUG("offsetFace: %s unresolved refs=%zu — NeedsRepair", op_id.c_str(),
                   out.needs_repair.size());
        return out;  // Ok + needsRepair ⇒ PlanExecutor prepares m−1, op not applied
    }

    // --- ownership preflight (OCCT silently IGNORES a foreign face — spike trap c)
    TopTools_IndexedMapOfShape body_faces;
    TopExp::MapShapes(target_shape, TopAbs_FACE, body_faces);
    std::vector<int> operative_ordinals;
    for (std::size_t i = 0; i < resolved.faces.size(); ++i) {
        const int ord = resolved.faces[i].IsNull() ? 0 : body_faces.FindIndex(resolved.faces[i]);
        if (ord == 0) {
            return OpOutcome::fail("OP_FAILED",
                                   "OffsetFace face " + face_ids[i] +
                                       " does not belong to target body " + target_id);
        }
        operative_ordinals.push_back(ord);
    }
    if (type == DistanceType::Total &&
        (opposite.IsNull() || body_faces.FindIndex(opposite) == 0)) {
        return OpOutcome::fail("OP_FAILED", "OffsetFace opposite face " + opposite_id +
                                                " does not belong to target body " + target_id);
    }
    // Duplicate ids naming the same face are a malformed record, not a silent merge.
    {
        std::vector<int> sorted = operative_ordinals;
        std::sort(sorted.begin(), sorted.end());
        if (std::adjacent_find(sorted.begin(), sorted.end()) != sorted.end()) {
            return OpOutcome::fail("OP_FAILED", "OffsetFace faceIds resolve to a duplicate face");
        }
    }

    // Rebuild the operative vectors in ORDINAL order — every downstream loop, error
    // message and history walk is then a pure function of the predecessor shape.
    std::vector<int> ordered = operative_ordinals;
    std::sort(ordered.begin(), ordered.end());
    std::vector<TopoDS_Face> operative;
    std::vector<std::string> operative_keys;
    for (const int ord : ordered) {
        operative.push_back(TopoDS::Face(body_faces(ord)));
        operative_keys.push_back(of::face_topokey(ord));
    }

    // --- tangent-closure GUARD (regen-time invariant, never a re-expansion) ----
    const of::ClosureResult closure = of::tangent_closure(target_shape, ordered);
    if (closure.non_manifold) {
        return OpOutcome::fail("OP_FAILED",
                               "OffsetFace: the operative set touches a non-manifold edge");
    }
    if (!closure.ok) {
        return OpOutcome::fail("OP_FAILED", "OffsetFace: tangent-closure walk failed");
    }
    {
        std::vector<int> extra;
        std::set_difference(closure.ordinals.begin(), closure.ordinals.end(), ordered.begin(),
                            ordered.end(), std::back_inserter(extra));
        if (!extra.empty()) {
            std::vector<std::string> extra_keys;
            for (const int ord : extra) extra_keys.push_back(of::face_topokey(ord));
            WLOG_WARN("offsetFace: %s tangent chain exceeds operative set (+%zu)", op_id.c_str(),
                      extra_keys.size());
            return OpOutcome::fail(
                "OP_FAILED", "OffsetFace: tangent chain exceeds stored operative set — the "
                             "kernel cannot hold these faces fixed: " +
                                 join_keys(extra_keys));
        }
    }

    // ══ V3 stage 0: blend-aware classification (WP3-C5) ══════════════════════
    // Runs BEFORE distance planning so `_BLEND_WITH_ABSOLUTE_DISTANCE` is reachable:
    // `plan_distances` would otherwise refuse a `Total`/`Radius` request against the
    // FULL stored operative set first, and the record would never be told the real
    // reason it cannot be honoured. Set only when a certified blend is present, so a
    // blend-free V3 record executes the V2 path untouched.
    std::optional<of::BlendAwareClosure> v3_closure;
    if (policy_version == kResultPolicyVersion3) {
        std::vector<int> primary_ordinals;
        for (const std::string& id : primary_face_ids) {
            const auto slot = std::find(face_ids.begin(), face_ids.end(), id);
            // `primaryFaceIds ⊆ faceIds` was enforced during parsing.
            primary_ordinals.push_back(
                operative_ordinals[static_cast<std::size_t>(slot - face_ids.begin())]);
        }
        const of::BlendAwareClosure classified =
            of::blend_aware_closure(target_shape, primary_ordinals);
        if (!classified.ok) {
            if (classified.code.empty()) {
                return OpOutcome::fail("OP_FAILED", "OffsetFace: " + classified.message);
            }
            WLOG_WARN("offsetFace: %s V3 closure refused: %s", op_id.c_str(),
                      classified.code.c_str());
            return blend_refusal(
                stage_refusal(classified.code.c_str(), classified.message, "classify"));
        }

        // TIGHTER THAN V2's ONE-SIDED GUARD, in both directions. V2 only checks that
        // the closure does not EXCEED the stored set; V3 reinterprets the same stored
        // pair — `faceIds` is the frozen closure and `primaryFaceIds` is what the user
        // actually picked — so the stored set must be exactly what the picks close
        // over. A stored set that no longer describes the body is a stale record, and
        // a stale operative set must never quietly select a different set of faces.
        {
            const std::vector<int> covered = classified.ordinals();
            if (covered != ordered) {
                std::vector<int> missing;
                std::vector<int> extra;
                std::set_difference(ordered.begin(), ordered.end(), covered.begin(),
                                    covered.end(), std::back_inserter(missing));
                std::set_difference(covered.begin(), covered.end(), ordered.begin(),
                                    ordered.end(), std::back_inserter(extra));
                std::vector<std::string> missing_keys;
                std::vector<std::string> extra_keys;
                for (const int ord : missing) missing_keys.push_back(of::face_topokey(ord));
                for (const int ord : extra) extra_keys.push_back(of::face_topokey(ord));
                return blend_refusal(stage_refusal(
                    "OFFSET_FACE_CLOSURE_MISMATCH",
                    "the blend-aware closure of primaryFaceIds is not the stored operative "
                    "set (stored-only: " +
                        (missing_keys.empty() ? std::string("none") : join_keys(missing_keys)) +
                        "; closure-only: " +
                        (extra_keys.empty() ? std::string("none") : join_keys(extra_keys)) + ")",
                    "classify",
                    json{{"storedOnly", missing_keys}, {"closureOnly", extra_keys}}));
            }
        }

        if (!classified.blends.empty()) {
            if (type != DistanceType::Offset) {
                // An absolute distance is a statement about the operative SURFACE
                // (a radius, a total thickness). With a blend in the closure the
                // surface the user measured is not the surface that ends up moving,
                // so V1 refuses rather than reinterpreting the number.
                return blend_refusal(stage_refusal(
                    "OFFSET_FACE_BLEND_WITH_ABSOLUTE_DISTANCE",
                    std::string("distanceType ") + distance_type_name(type) +
                        " is not supported when the closure contains a blend",
                    "classify"));
            }
            v3_closure = classified;
        }
    }

    const double construction_tol = of::build_tolerance(target_shape);

    // --- per-face signed d ----------------------------------------------------
    // Construction uncertainty must never decide what the user asked for. Absolute
    // Radius/Total planning and semantic equality use the owned semantic tolerance;
    // only BRepOffset_MakeOffset receives the construction tolerance below. Measured
    // on the TARGET, the shape the planning distances are read off.
    const DistancePlan plan = plan_distances(
        type, distance, chain_flag, operative, opposite,
        kernel::validation::precision_of(target_shape).semantic_length());
    if (!plan.ok) {
        return OpOutcome::fail("OP_FAILED", "OffsetFace: " + plan.error);
    }
    double minimum_abs_d = std::numeric_limits<double>::infinity();
    for (const double value : plan.d)
        minimum_abs_d = std::min(minimum_abs_d, std::abs(value));
    // Observability (docs/DEBUGGING.md): a REGEN-frequency line, never drag-frequency.
    WLOG_DEBUG("offsetFace: %s type=%s faces=%zu d0=%.6f t=%.6f buildTol=%.3e", op_id.c_str(),
               distance_type_name(type), operative.size(), plan.d.empty() ? 0.0 : plan.d[0],
               plan.thickness, construction_tol);

    // An identity or sub-resolution request is a refusal, never a successful
    // `modified` event carrying unchanged geometry. Imported B-Rep uncertainty may
    // make an edit unbuildable; it may not silently redefine the requested intent.
    const double minimum_supported_change =
        kernel::validation::precision_of(target_shape, minimum_abs_d).authoring_resolution();
    if (!std::isfinite(minimum_abs_d) || minimum_abs_d < minimum_supported_change) {
        // NOTE: this value is rendered INTO the wire message below, so a migration
        // that changed the printed digits would not be the pure rename it claims to
        // be. `test_offsetface` asserts the rendered string, not just the number.
        const std::string message =
            "OffsetFace: effective change " + std::to_string(minimum_abs_d) +
            " mm is below the supported minimum " +
            std::to_string(minimum_supported_change) + " mm";
        OpOutcome failure = OpOutcome::fail("OP_FAILED", message);
        failure.diagnostics.push_back({{"severity", "error"},
                                       {"code", "OFFSET_FACE_CHANGE_TOO_SMALL"},
                                       {"message", message},
                                       {"stage", "preflight"},
                                       {"evidence",
                                        {{"minimumEffectiveChangeMm", minimum_abs_d},
                                         {"minimumSupportedChangeMm",
                                          minimum_supported_change},
                                         {"constructionToleranceMm", construction_tol}}}});
        return failure;
    }

    if (ctx.cancel && ctx.cancel->cancelled()) return OpOutcome::cancelled();

    // --- normalize the build input to a SOLID ---------------------------------
    // A body's stored geometry is whatever produced it: a BOP result is a COMPOUND
    // wrapping one solid. `BRepOffset_MakeOffset` only wraps its own result back into
    // a solid when the INPUT was one — feed it a compound and it answers with a bare
    // SHELL carrying identical geometry. Offsetting the normalized solid keeps the
    // "exactly one solid" postcondition meaningful instead of vacuously false.
    TopoDS_Shape build_input = target_shape;
    if (target_shape.ShapeType() != TopAbs_SOLID) {
        const std::vector<TopoDS_Shape> solids_in = ordered_solids(target_shape);
        if (solids_in.size() != 1) {
            return OpOutcome::fail("OP_FAILED",
                                   "OffsetFace requires a single-solid target body; " +
                                       target_id + " carries " +
                                       std::to_string(solids_in.size()));
        }
        build_input = solids_in[0];
    }
    // The operative faces re-addressed inside the normalized solid. They are the same
    // TShapes, but asking EXPLICITLY turns a mismatch into a loud failure instead of
    // OCCT's silent "unknown face, ignored" (spike trap c).
    std::vector<TopoDS_Face> build_operative;
    if (build_input.IsSame(target_shape)) {
        build_operative = operative;
    } else {
        TopTools_IndexedMapOfShape build_faces;
        TopExp::MapShapes(build_input, TopAbs_FACE, build_faces);
        for (std::size_t i = 0; i < operative.size(); ++i) {
            const int idx = build_faces.FindIndex(operative[i]);
            if (idx == 0) {
                return OpOutcome::fail("OP_FAILED", "OffsetFace face " + operative_keys[i] +
                                                        " is not addressable on the target solid");
            }
            build_operative.push_back(TopoDS::Face(build_faces(idx)));
        }
    }

    // ══ V3 stage 0 continued: run the pipeline ═══════════════════════════════
    // A V3 record with no blend in its closure never enters here and falls through to
    // the V2 build below, which is what makes "same record, no blend ⇒ same geometry"
    // true by construction rather than by assertion.
    if (v3_closure.has_value()) {
        const of::BlendAwareClosure& classified = *v3_closure;
        std::vector<TopoDS_Face> moving;
        std::vector<std::string> moving_keys;
        TopTools_IndexedMapOfShape build_faces;
        TopExp::MapShapes(build_input, TopAbs_FACE, build_faces);
        for (const int ord : classified.moving) {
            const int idx = build_faces.FindIndex(body_faces(ord));
            if (idx == 0) {
                return OpOutcome::fail("OP_FAILED",
                                       "OffsetFace face " + of::face_topokey(ord) +
                                           " is not addressable on the target solid");
            }
            moving.push_back(TopoDS::Face(build_faces(idx)));
            moving_keys.push_back(of::face_topokey(ord));
        }
        std::vector<of::ClosureBlend> blends = classified.blends;
        for (of::ClosureBlend& entry : blends) {
            for (TopoDS_Face* face :
                 {&entry.blend.face, &entry.blend.support_a, &entry.blend.support_b}) {
                const int idx = build_faces.FindIndex(*face);
                if (idx == 0) {
                    return OpOutcome::fail(
                        "OP_FAILED", "OffsetFace blend face " + of::face_topokey(entry.ordinal) +
                                         " is not addressable on the target solid");
                }
                *face = TopoDS::Face(build_faces(idx));
            }
        }
        WLOG_DEBUG("offsetFace: %s V3 moving=%zu blends=%zu fixed=%zu d=%.6f", op_id.c_str(),
                   classified.moving.size(), classified.blends.size(),
                   classified.fixed.size(), plan.d[0]);

        const BlendPipeline pipeline =
            run_blend_pipeline(build_input, moving, moving_keys, blends, plan.d[0], ctx.cancel);
        if (ctx.cancel && ctx.cancel->cancelled()) return OpOutcome::cancelled();
        if (!pipeline.ok) {
            WLOG_WARN("offsetFace: %s V3 %s stage refused: %s", op_id.c_str(),
                      pipeline.refusal.stage.c_str(),
                      pipeline.refusal.publication ? pipeline.refusal.reason_code.c_str()
                                                   : pipeline.refusal.code.c_str());
            return blend_refusal(pipeline.refusal);
        }

        // --- identity: ONE history over the whole three-stage chain -------
        // The suppressed blend face has no kernel lineage to its reconstruction,
        // so the mapping is ASSERTED here — and only here, because this is where
        // `reblend`'s reproduction proof has already passed on that exact pair.
        // The blend's two boundary edges are genuinely consumed; they are left to
        // `ComposedHistory`'s positive-evidence removal rather than synthesized,
        // because an edge the offset split has no unique correspondence and V1
        // refuses to guess.
        em::ComposedHistory composed(target_shape, pipeline.shape);
        for (const occ::handle<BRepTools_History>& stage : pipeline.stages)
            composed.add_stage(stage);
        for (const auto& pair : pipeline.lineage)
            composed.add_modified(pair.first, pair.second);

        // FACE-COUNT BIJECTION, over the MODIFIED channel only. `Modified` is the
        // channel that answers the question this gate asks — did THIS face survive
        // AS ITSELF, exactly once, onto a face nothing else also claims? `Generated`
        // answers a different one: it carries cross-kind lineage (an edge that
        // generates a face) which is real, is what the reproduction proof walks, and
        // is not a claim that any face survived.
        //
        // MEASURED on this chain, OCCT 8.0.1, after `ComposedHistory`'s injected-
        // successor claim: every input face answers exactly one `Modified` image and
        // ZERO faces on `Generated`. Before that claim landed, a blend support also
        // answered the rebuilt blend face on `Generated` (defeaturing generates the
        // seed edge from the support, the offset carries it across, the re-fillet
        // generates the blend from it), which would make every support look split
        // here. Reading `Modified` is correct in both worlds, which is why this gate
        // does not depend on that fix to be right.
        TopTools_IndexedMapOfShape final_faces;
        TopExp::MapShapes(pipeline.shape, TopAbs_FACE, final_faces);
        const json face_counts = {{"inputFaceCount", body_faces.Extent()},
                                  {"finalFaceCount", final_faces.Extent()}};
        if (final_faces.Extent() != body_faces.Extent()) {
            // A DECLARED V1 SCOPE LIMIT, not a reblend failure. An offset may
            // legitimately merge two coplanar faces or split one that the moved wall
            // now cuts in two, and V2 publishes those results — `apply_history`'s
            // confidence-gated split branch exists for exactly that. V1 of the blend
            // path refuses instead, because the reproduction argument is
            // face-for-face: every input face has one image and the blend comes back
            // as the same blend. A merge or a split breaks that correspondence, and a
            // destructive path may not publish a result whose identity story it
            // cannot state. Its own code so the refusal is not mistaken for the
            // rebuild having gone wrong; widening the scope is C6's call, with a
            // corpus behind it.
            return blend_refusal(stage_refusal(
                "OFFSET_FACE_TOPOLOGY_CHANGED",
                "the chain ended with " + std::to_string(final_faces.Extent()) +
                    " faces for " + std::to_string(body_faces.Extent()) +
                    " input faces; V1 of the blend path requires a face-for-face result",
                "identity", face_counts));
        }
        std::map<int, int> claimed_by;  // final ordinal ⇒ input ordinal
        for (int i = 1; i <= body_faces.Extent(); ++i) {
            std::set<int> images;
            for (const TopoDS_Shape& s : composed.modified(body_faces(i))) {
                const int ord = final_faces.FindIndex(s);
                if (ord != 0) images.insert(ord);
            }
            if (images.size() != 1) {
                return blend_refusal(stage_refusal(
                    "OFFSET_FACE_REBLEND_NOT_REPRODUCED",
                    "input face " + of::face_topokey(i) + " has " +
                        std::to_string(images.size()) +
                        " modified images through the composed history; exactly one is "
                        "required",
                    "identity", face_counts));
            }
            const auto [seen, fresh] = claimed_by.emplace(*images.begin(), i);
            if (!fresh) {
                return blend_refusal(stage_refusal(
                    "OFFSET_FACE_REBLEND_NOT_REPRODUCED",
                    "input faces " + of::face_topokey(seen->second) + " and " +
                        of::face_topokey(i) + " both claim the same final face",
                    "identity", face_counts));
            }
        }

        kernel::validation::PublicationPolicy v3_policy =
            kernel::validation::single_solid_policy(
                "OffsetFace",
                result_validation_tier(ctx, kernel::validation::PublicationTier::TierB));
        v3_policy.maximum_tolerance =
            kernel::validation::precision_of(pipeline.shape)
                .tolerance_ceiling(construction_tol, 2.0, 0.0);
        const kernel::validation::PublicationDecision v3_decision =
            publication_decision(pipeline.shape, v3_policy, ctx.cancel);
        if (ctx.cancel && ctx.cancel->cancelled()) return OpOutcome::cancelled();
        if (!v3_decision.publishable()) {
            return publication_refusal(v3_decision, "publication");
        }

        WLOG_DEBUG("offsetFace: %s V3 tolerance contributions suppress=%.3e offset=%.3e "
                   "reblend=%.3e",
                   op_id.c_str(), pipeline.suppress_contribution,
                   pipeline.offset_contribution, pipeline.reblend_contribution);
        const std::vector<RankedSolid> v3_solids = ranked_solids(pipeline.shape);
        ctx.bodies.create(target_id, op_id, pipeline.shape);
        ctx.partition.apply_history(target_id, pipeline.shape, composed, out.delta,
                                    &out.needs_repair);
        out.body_events.push_back({"modified", target_id, v3_solids[0].key});
        out.body_ids.push_back(target_id);
        return out;
    }

    // --- build (spike-frozen call sequence) -----------------------------------
    BRepOffset_MakeOffset mo;
    TopoDS_Shape result;
    try {
        mo.Initialize(build_input, /*Offset*/ 0.0, construction_tol, BRepOffset_Skin,
                      /*Intersection*/ Standard_False, /*SelfInter*/ Standard_False,
                      GeomAbs_Intersection, /*Thickening*/ Standard_False,
                      /*RemoveIntEdges*/ Standard_False);
        mo.AllowLinearization(Standard_False);
        for (std::size_t i = 0; i < build_operative.size(); ++i) {
            mo.SetOffsetOnFace(build_operative[i], plan.d[i]);
        }
        mo.MakeOffsetShape();
        if (!mo.IsDone()) {
            return OpOutcome::fail("OP_FAILED",
                                   "OffsetFace: kernel refused the offset (BRepOffset_Error " +
                                       std::to_string(static_cast<int>(mo.Error())) + ")");
        }
        if (mo.Error() != BRepOffset_NoError) {
            return OpOutcome::fail("OP_FAILED",
                                   "OffsetFace: kernel reported BRepOffset_Error " +
                                       std::to_string(static_cast<int>(mo.Error())));
        }
        result = mo.Shape();
    } catch (const Standard_Failure& f) {
        return OpOutcome::fail("OP_FAILED",
                               std::string("OffsetFace: kernel exception: ") +
                                   (f.GetMessageString() ? f.GetMessageString() : "OCCT"));
    } catch (...) {
        return OpOutcome::fail("OP_FAILED", "OffsetFace: unknown kernel exception");
    }

    // --- postconditions (NEVER publish a failed shape) ------------------------
    if (result.IsNull()) {
        return OpOutcome::fail("GEOMETRY_INVALID", "OffsetFace produced a null shape");
    }
    if (!BRepCheck_Analyzer(result).IsValid()) {
        return OpOutcome::fail("GEOMETRY_INVALID", "OffsetFace produced an invalid shape");
    }
    const std::vector<RankedSolid> solids = ranked_solids(result);
    if (solids.size() != 1) {
        // V1: an offset must never fan the body out (SCHEMA §7.3) — refuse loudly.
        return OpOutcome::fail("OP_FAILED", "OffsetFace produced " +
                                                std::to_string(solids.size()) +
                                                " solids; exactly one is required");
    }
    double volume = 0.0;
    {
        GProp_GProps props;
        BRepGProp::VolumeProperties(result, props);
        volume = props.Mass();
    }
    if (!std::isfinite(volume) ||
        volume <= kernel::validation::precision_of(result).minimum_volume()) {
        // Spike trap b: a collapsed body passes IsDone AND BRepCheck.
        return OpOutcome::fail("GEOMETRY_INVALID",
                               "OffsetFace produced a degenerate volume " +
                                   std::to_string(volume));
    }
    {
        BRepAlgoAPI_Check checker;
        checker.SetData(result, /*bTestSE*/ Standard_False, /*bTestSI*/ Standard_True);
        checker.SetRunParallel(Standard_False);  // determinism: never the BOP thread pool
        try {
            checker.Perform();
            if (!checker.IsValid()) {
                return OpOutcome::fail("GEOMETRY_INVALID",
                                       "OffsetFace produced a self-interfering shape");
            }
        } catch (const Standard_Failure&) {
            return OpOutcome::fail("GEOMETRY_INVALID",
                                   "OffsetFace self-interference check raised");
        }
    }
    const std::string semantic =
        check_semantics(mo, result, build_operative, plan.d, operative_keys);
    if (!semantic.empty()) {
        WLOG_WARN("offsetFace: %s postcondition failed: %s", op_id.c_str(), semantic.c_str());
        return OpOutcome::fail("OP_FAILED", "OffsetFace: " + semantic);
    }

    // The SHARED publication gate, last (U7). OffsetFace was the one mutating
    // operation that never ran it: its hand-rolled postconditions above are
    // stricter in the semantic direction, but they produce no structured
    // `PublicationDecision` evidence and nothing stopped them drifting from the
    // policy every sibling operation is held to. Running both keeps the
    // op-specific refusals (which say WHY in offset terms) and adds the common
    // Tier A evidence the P3 contract rows promise.
    kernel::validation::PublicationPolicy publication_policy =
        kernel::validation::single_solid_policy(
            "OffsetFace", result_validation_tier(
                              ctx, kernel::validation::PublicationTier::TierB));
    // Grows from the CONSTRUCTION tolerance, not the input shape's, and with NO
    // slack term — both differences from Extrude/Fillet are deliberate, which is
    // why `tolerance_ceiling` takes its base and epsilon explicitly instead of
    // defaulting them. Unifying either would loosen this ceiling.
    publication_policy.maximum_tolerance =
        kernel::validation::precision_of(result).tolerance_ceiling(construction_tol, 2.0, 0.0);
    const kernel::validation::PublicationDecision decision =
        publication_decision(result, publication_policy, ctx.cancel);
    if (ctx.cancel && ctx.cancel->cancelled()) return OpOutcome::cancelled();
    if (!decision.publishable()) {
        return publication_refusal(decision, "publication");
    }

    // --- publish: modified in place + history through the offset image --------
    of::OffsetImageHistory history(mo.OffsetFacesFromShapes(), mo.OffsetEdgesFromShapes(), result);
    ctx.bodies.create(target_id, op_id, result);
    ctx.partition.apply_history(target_id, result, history, out.delta, &out.needs_repair);
    out.body_events.push_back({"modified", target_id, solids[0].key});
    out.body_ids.push_back(target_id);
    return out;
}

}  // namespace onecad::ops
