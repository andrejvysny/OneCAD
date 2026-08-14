// FilletChamferOp.cpp — see FilletChamferOp.h. Ports buildFillet / buildChamfer.
#include "ops/FilletChamferOp.h"

#include <algorithm>
#include <cmath>
#include <memory>
#include <optional>
#include <set>
#include <string>
#include <vector>

#include <BRepCheck_Analyzer.hxx>
#include <BRepFilletAPI_MakeChamfer.hxx>
#include <Standard_Failure.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedDataMapOfShapeListOfShape.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopTools_ListOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>

#include "elementmap/Ladder.h"
#include "kernel/fillet/FilletBuilder.h"
#include "kernel/fillet/EdgeContour.h"
#include "kernel/fillet/FilletSemanticChecks.h"
#include "ops/OpCommon.h"
#include "session/ShapeMetrics.h"

namespace onecad::ops {

using nlohmann::json;
namespace em = onecad::elementmap;

namespace {

constexpr double kMinValue = 1e-3;  // RegenerationEngine.cpp:61 kMinValue

enum class Mode { Fillet, Chamfer };

// The shared target body id of the op's edge refs (all edges live on one body).
std::string target_body_of(const json& op) {
    if (!op.contains("inputs") || !op["inputs"].is_array()) return "";
    for (const json& in : op["inputs"]) {
        if (in.is_object() && in.contains("primary") && in["primary"].is_object()) {
            const std::string bid = read_str(in["primary"], "bodyId");
            if (!bid.empty()) return bid;
        }
    }
    return "";
}

// Whether `params` carries a readable scalar under `key` (SCHEMA §4: a bare
// number OR a `{value}` object). `read_scalar`'s default cannot distinguish an
// absent optional field from one legitimately set to that default, and
// `distance2` is skip-none on the Rust side, so presence is asked separately.
bool has_scalar(const json& params, const char* key) {
    if (!params.is_object() || !params.contains(key)) return false;
    const json& v = params[key];
    return v.is_number() || (v.is_object() && v.contains("value") && v["value"].is_number());
}

// SCHEMA §7.3 (2026-08-03) DETERMINISTIC REFERENCE FACE for a two-distance
// chamfer: of the edge's adjacent faces, the one with the SMALLER resolved face
// ordinal. `radius` is applied on it, `distance2` on the other.
//
// Why the ordinal and not `faces.First()`: the ancestor list's order is an
// artifact of how OCCT walked the shape while building the map, so it is stable
// for one traversal but carries no documented meaning and cannot be reasoned
// about from a document. The face ordinal is the same snapshot-scoped index the
// element map already publishes as a TopoKey, so "distance on the smaller face
// ordinal" is a rule a reader can verify against `QueryElement` output — and it
// is a pure function of the predecessor shape, so replaying the same document
// twice picks the same face and produces the same geometry.
//
// `face_map` is `TopExp::MapShapes(body, TopAbs_FACE)`, built ONCE by the caller.
// Returns a null face when the edge has no adjacent face (a free edge — the
// caller then skips it exactly as the equal-leg path does).
TopoDS_Face reference_face(const TopTools_IndexedMapOfShape& face_map,
                           const TopTools_ListOfShape& faces) {
    TopoDS_Face best;
    int best_ordinal = 0;
    for (TopTools_ListOfShape::Iterator it(faces); it.More(); it.Next()) {
        if (it.Value().ShapeType() != TopAbs_FACE) continue;
        const int ordinal = face_map.FindIndex(it.Value());
        if (ordinal <= 0) continue;
        if (best_ordinal == 0 || ordinal < best_ordinal) {
            best_ordinal = ordinal;
            best = TopoDS::Face(it.Value());
        }
    }
    return best;
}

struct EdgeResolution {
    std::vector<TopoDS_Edge> edges;
    std::vector<kernel::fillet::ResolvedEdge> fillet_edges;
    std::optional<OpOutcome> stop;
};

std::vector<em::LadderResolution> resolve_edge_refs(
    OpContext& ctx, const TopoDS_Shape& target_shape, const std::string& target_id,
    const std::vector<em::LadderRef>& refs) {
    std::vector<em::LadderResolution> results(refs.size());
    std::vector<em::LadderRef> unresolved;
    std::vector<std::size_t> unresolved_indices;
    for (std::size_t i = 0; i < refs.size(); ++i) {
        const em::PartitionEntry* entry = ctx.partition.find(refs[i].element_id);
        const std::string live_topo_key = entry
                                              ? em::ElementMapPartition::topokey_for_shape(
                                                    target_shape, entry->shape,
                                                    em::km::ElementKind::Edge)
                                              : "";
        if (entry && entry->body_id == target_id && entry->kind == em::km::ElementKind::Edge &&
            !live_topo_key.empty()) {
            em::LadderResolution identity;
            identity.ref_id = refs[i].ref_id;
            identity.element_id = refs[i].element_id;
            identity.kind = refs[i].kind;
            identity.outcome = em::LadderOutcome::AutoBind;
            identity.ladder_level = "identity";
            identity.bound_shape = entry->shape;
            identity.bound_topo_key = live_topo_key;
            results[i] = std::move(identity);
        } else {
            unresolved.push_back(refs[i]);
            unresolved_indices.push_back(i);
        }
    }
    const std::vector<em::LadderResolution> fallback = em::resolve_descriptor_stage(
        target_shape, target_id, unresolved, em::LadderEditContext{ctx.post_upstream_edit, ctx.from_zero_replay});
    for (std::size_t i = 0; i < fallback.size(); ++i) {
        results[unresolved_indices[i]] = fallback[i];
    }
    return results;
}

EdgeResolution resolve_edges(OpContext& ctx, const json& op, const std::string& op_id,
                             const std::string& target_id, const TopoDS_Shape& target_shape,
                             const char* op_name) {
    std::vector<em::LadderRef> refs;
    std::set<std::string> seen_element_ids;
    if (op.contains("inputs") && op["inputs"].is_array()) {
        std::size_t i = 0;
        for (const json& in : op["inputs"]) {
            em::LadderRef r = em::ladder_ref_from_input(in, op_id + ".input" + std::to_string(i));
            if (r.kind == em::km::ElementKind::Edge &&
                (r.element_id.empty() || seen_element_ids.insert(r.element_id).second)) {
                refs.push_back(std::move(r));
            }
            ++i;
        }
    }
    if (refs.empty()) {
        return {{}, {}, OpOutcome::fail("OP_FAILED", std::string("No edges for ") + op_name)};
    }

    EdgeResolution result;
    const std::vector<em::LadderResolution> res =
        resolve_edge_refs(ctx, target_shape, target_id, refs);
    for (const em::LadderResolution& r : res) {
        if (r.outcome == em::LadderOutcome::AutoBind && !r.bound_shape.IsNull() &&
            r.bound_shape.ShapeType() == TopAbs_EDGE) {
            const TopoDS_Edge edge = TopoDS::Edge(r.bound_shape);
            result.edges.push_back(edge);
            result.fillet_edges.push_back(
                {r.element_id, r.bound_topo_key, r.ref_id, target_id, edge});
        } else {
            if (!result.stop) result.stop.emplace();
            result.stop->needs_repair.push_back(r.to_needs_repair_json());
        }
    }
    return result;
}

std::optional<OpOutcome> build_fillet(
    OpContext& ctx, const TopoDS_Shape& target_shape,
    std::vector<kernel::fillet::ResolvedEdge> edges, double radius,
    std::unique_ptr<kernel::fillet::FilletBuilder>& builder, TopoDS_Shape& result) {
    builder = std::make_unique<kernel::fillet::FilletBuilder>(target_shape, std::move(edges),
                                                              radius);
    kernel::fillet::FilletBuildResult built = builder->build(ctx.cancel);
    if (built.cancelled) return OpOutcome::cancelled();
    if (built.ok) {
        result = std::move(built.shape);
        return std::nullopt;
    }
    OpOutcome failure = OpOutcome::fail(built.error_code, built.message);
    for (const auto& diagnostic : built.diagnostics) {
        failure.diagnostics.push_back(diagnostic.to_json());
    }
    return failure;
}

std::optional<OpOutcome> validate_chamfer(
    const TopoDS_Shape& result, kernel::validation::PublicationTier tier) {
    const kernel::validation::PublicationDecision decision = publication_decision(
        result, kernel::validation::single_solid_policy("Chamfer", tier));
    if (!decision.publishable()) return OpOutcome::fail(decision.code, decision.message);
    return std::nullopt;
}

std::optional<OpOutcome> build_chamfer(const TopoDS_Shape& target_shape,
                                       const std::vector<TopoDS_Edge>& edges, double radius,
                                       bool two_distance, double distance2,
                                       kernel::validation::PublicationTier validation_tier,
                                       std::shared_ptr<BRepBuilderAPI_MakeShape>& builder,
                                       TopoDS_Shape& result) {
    TopTools_IndexedDataMapOfShapeListOfShape edge_faces;
    TopExp::MapShapesAndAncestors(target_shape, TopAbs_EDGE, TopAbs_FACE, edge_faces);
    TopTools_IndexedMapOfShape face_map;
    if (two_distance) TopExp::MapShapes(target_shape, TopAbs_FACE, face_map);
    auto chamfer = std::make_shared<BRepFilletAPI_MakeChamfer>(target_shape);
    std::size_t added = 0;
    for (const TopoDS_Edge& edge : edges) {
        const int index = edge_faces.FindIndex(edge);
        if (index == 0 || edge_faces(index).IsEmpty()) continue;
        const TopTools_ListOfShape& faces = edge_faces(index);
        if (two_distance) {
            const TopoDS_Face face = reference_face(face_map, faces);
            if (face.IsNull()) continue;
            chamfer->Add(radius, distance2, edge, face);
        } else {
            chamfer->Add(radius, radius, edge, TopoDS::Face(faces.First()));
        }
        ++added;
    }
    if (added == 0) return OpOutcome::fail("OP_FAILED", "No valid edges for chamfer");
    chamfer->Build();
    if (!chamfer->IsDone()) return OpOutcome::fail("OP_FAILED", "Chamfer operation failed");
    result = chamfer->Shape();
    builder = chamfer;
    return validate_chamfer(result, validation_tier);
}

struct EdgeValues {
    double radius = 0.0;
    bool two_distance = false;
    double distance2 = 0.0;
    int tangent_closure_version = 0;
    std::optional<OpOutcome> stop;
};

EdgeValues read_values(const json& op, Mode mode) {
    EdgeValues values;
    const json params =
        op.contains("params") && op["params"].is_object() ? op["params"] : json::object();
    values.radius = read_scalar(params, "radius", 0.0);
    if (!std::isfinite(values.radius) || values.radius < kMinValue) {
        values.stop = OpOutcome::fail(
            "OP_FAILED",
            mode == Mode::Fillet ? "Fillet radius too small" : "Chamfer distance too small");
        return values;
    }
    values.two_distance = mode == Mode::Chamfer && has_scalar(params, "distance2");
    values.distance2 =
        values.two_distance ? read_scalar(params, "distance2", 0.0) : 0.0;
    if (values.two_distance &&
        (!std::isfinite(values.distance2) || values.distance2 < kMinValue)) {
        values.stop = OpOutcome::fail("OP_FAILED", "Chamfer distance2 too small");
    }
    if (params.contains("tangentClosureVersion")) {
        if (!params["tangentClosureVersion"].is_number_integer() ||
            params["tangentClosureVersion"].get<int>() != 1) {
            values.stop = OpOutcome::fail("OP_FAILED", "Unsupported tangent closure version");
        } else {
            values.tangent_closure_version = 1;
        }
    }
    return values;
}

OpOutcome closure_changed(const std::string& message, const std::vector<int>& expected,
                         const std::vector<int>& actual) {
    auto keys = [](const std::vector<int>& ordinals) {
        json result = json::array();
        for (const int ordinal : ordinals) result.push_back("e:" + std::to_string(ordinal));
        return result;
    };
    OpOutcome failure = OpOutcome::fail("OP_FAILED", message);
    failure.diagnostics.push_back(
        {{"severity", "error"},
         {"code", "EDGE_OP_TANGENT_CLOSURE_CHANGED"},
         {"message", message},
         {"stage", "resolve"},
         {"evidence",
          {{"contour",
            {{"status", "closure-drift"},
             {"edgeTopoKeys", keys(actual)},
             {"expectedEdgeTopoKeys", keys(expected)}}}}}});
    return failure;
}

std::optional<OpOutcome> enforce_frozen_closure(const TopoDS_Shape& body, Mode mode,
                                                EdgeResolution& resolved) {
    TopTools_IndexedMapOfShape edge_map;
    TopExp::MapShapes(body, TopAbs_EDGE, edge_map);
    std::vector<int> expected;
    for (const TopoDS_Edge& edge : resolved.edges) {
        const int ordinal = edge_map.FindIndex(edge);
        if (std::find(expected.begin(), expected.end(), ordinal) != expected.end()) {
            return closure_changed("Stored tangent closure collapsed onto one edge", expected,
                                   {ordinal});
        }
        expected.push_back(ordinal);
    }
    std::sort(expected.begin(), expected.end());
    const auto contours = kernel::fillet::analyze_edge_contours(
        body, resolved.edges,
        mode == Mode::Fillet ? kernel::fillet::EdgeOpMode::Fillet
                             : kernel::fillet::EdgeOpMode::Chamfer);
    if (!contours.ok)
        return closure_changed(contours.message, expected, {});
    if (expected != contours.closure_ordinals) {
        return closure_changed("Stored tangent closure changed after an upstream edit", expected,
                               contours.closure_ordinals);
    }
    std::vector<TopoDS_Edge> edge_seeds;
    std::vector<kernel::fillet::ResolvedEdge> fillet_seeds;
    for (const std::vector<int>& contour : contours.contours) {
        const int seed = contour.front();
        for (std::size_t index = 0; index < resolved.edges.size(); ++index) {
            if (edge_map.FindIndex(resolved.edges[index]) != seed) continue;
            edge_seeds.push_back(resolved.edges[index]);
            fillet_seeds.push_back(resolved.fillet_edges[index]);
            break;
        }
    }
    resolved.edges = std::move(edge_seeds);
    resolved.fillet_edges = std::move(fillet_seeds);
    return std::nullopt;
}

OpOutcome publish_result(OpContext& ctx, const std::string& target_id,
                         const std::string& op_id, const TopoDS_Shape& result,
                         const std::unique_ptr<kernel::fillet::FilletBuilder>& fillet_builder,
                         const std::shared_ptr<BRepBuilderAPI_MakeShape>& builder) {
    OpOutcome out;
    ctx.bodies.create(target_id, op_id, result);
    if (fillet_builder) {
        ctx.partition.apply_history(target_id, result, fillet_builder->history(), out.delta,
                                    &out.needs_repair);
    } else if (builder) {
        ctx.partition.apply_history(target_id, result, *builder, out.delta, &out.needs_repair);
    }
    out.body_events.push_back({"modified", target_id, {}});
    out.body_ids.push_back(target_id);
    return out;
}

OpOutcome run(OpContext& ctx, const json& op, const std::string& op_id, Mode mode) {
    const char* name = mode == Mode::Fillet ? "Fillet" : "Chamfer";
    if (std::vector<json> repairs = operation_ref_ownership_repairs(op, op_id); !repairs.empty()) {
        OpOutcome out;
        out.needs_repair = std::move(repairs);
        return out;
    }
    const std::string target_id = target_body_of(op);
    if (target_id.empty()) {
        return OpOutcome::fail("OP_FAILED", std::string(name) + " requires body input");
    }
    const session::BodyRecord* target = ctx.bodies.get(target_id);
    if (!target) {
        return OpOutcome::fail("REF_UNRESOLVED",
                               std::string(name) + " target body not found: " + target_id);
    }
    if (auto invalid = validate_modeling_input(target->geom, name, "target")) return *invalid;
    const EdgeValues values = read_values(op, mode);
    if (values.stop) return *values.stop;
    EdgeResolution resolved = resolve_edges(ctx, op, op_id, target_id, target->geom, name);
    if (resolved.stop) return std::move(*resolved.stop);
    if (values.tangent_closure_version == 1) {
        const std::optional<OpOutcome> drift = enforce_frozen_closure(target->geom, mode, resolved);
        if (drift) return *drift;
    }
    if (ctx.cancel && ctx.cancel->cancelled()) return OpOutcome::cancelled();

    TopoDS_Shape result;
    std::shared_ptr<BRepBuilderAPI_MakeShape> builder;
    std::unique_ptr<kernel::fillet::FilletBuilder> fillet_builder;
    try {
        std::optional<OpOutcome> failure = mode == Mode::Fillet
                                               ? build_fillet(
                                                     ctx, target->geom,
                                                     std::move(resolved.fillet_edges),
                                                     values.radius, fillet_builder, result)
                                               : build_chamfer(
                                                     target->geom, resolved.edges, values.radius,
                                                     values.two_distance, values.distance2,
                                                     result_validation_tier(
                                                         ctx, kernel::validation::PublicationTier::TierB),
                                                     builder, result);
        if (failure) return std::move(*failure);
    } catch (const Standard_Failure& error) {
        return OpOutcome::fail(
            "OP_FAILED", std::string(name) + " operation failed (radius too large?): " +
                             (error.GetMessageString() ? error.GetMessageString() : "OCCT"));
    } catch (...) {
        return OpOutcome::fail("OP_FAILED", std::string(name) + " operation failed");
    }

    return publish_result(ctx, target_id, op_id, result, fillet_builder, builder);
}

}  // namespace

OpOutcome execute_fillet(OpContext& ctx, const json& op, const std::string& op_id) {
    return run(ctx, op, op_id, Mode::Fillet);
}

OpOutcome execute_chamfer(OpContext& ctx, const json& op, const std::string& op_id) {
    return run(ctx, op, op_id, Mode::Chamfer);
}

}  // namespace onecad::ops
