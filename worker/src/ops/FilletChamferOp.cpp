// FilletChamferOp.cpp — see FilletChamferOp.h. Ports buildFillet / buildChamfer.
#include "ops/FilletChamferOp.h"

#include <algorithm>
#include <cmath>
#include <cstddef>
#include <map>
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

#include "elementmap/ElementMapPartition.h"
#include "elementmap/Ladder.h"
#include "kernel/fillet/FilletBuilder.h"
#include "kernel/fillet/EdgeContour.h"
#include "kernel/fillet/FilletSemanticChecks.h"
#include "kernel/validation/GeometryPrecision.h"
#include "ops/OpCommon.h"
#include "session/EdgePicks.h"
#include "session/ShapeMetrics.h"

namespace onecad::ops {

using nlohmann::json;
namespace em = onecad::elementmap;

namespace {

// The radius/distance floor now comes from the precision context —
// `GeometryPrecisionContext::authoring_resolution()`. The value is unchanged
// (1e-3 mm, RegenerationEngine.cpp:61 kMinValue); it is no longer restated here.

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

// The pre-WP-F "reference face = the adjacent face with the smaller resolved face
// ordinal" chooser USED TO LIVE HERE and is deliberately gone (kernel-hardening
// WP-F review, 2026-09-04). It was a silent mirror of the two chamfer legs
// whenever an upstream feature reordered the face map, and gating it on
// `post_upstream_edit` did not fix that: that flag is a per-PLAN claim which every
// `ToEnd(0)` lane sets and which `RevertToEnd` (undo/redo) and `RegenToStep`
// previews omit, so a record that halted on open would have replayed the ordinal
// rule against the already-permuted map on the very next redo. A gate that fires
// on every open and vanishes on redo protects nothing. An asymmetric chamfer now
// has exactly ONE source for its reference face — the persisted
// `params.referenceFaces` pair — and a contour without one HALTS.

struct EdgeResolution {
    std::vector<TopoDS_Edge> edges;
    std::vector<kernel::fillet::ResolvedEdge> fillet_edges;
    std::optional<OpOutcome> stop;
};

// Resolve semantic refs of ONE element kind against the target body: rung 1 the
// partition binding scoped to THIS body (identity), rung 2 the descriptor+anchor
// ladder. Kind-generic because a Chamfer's `referenceFaces` face refs (SCHEMA
// §7.3) must take exactly the same two rungs its edge refs take — a face bound by
// a weaker policy than the edge it is paired with is a silent mis-bind waiting to
// happen, and this op refuses to have two resolution policies.
std::vector<em::LadderResolution> resolve_refs_of_kind(
    OpContext& ctx, const TopoDS_Shape& target_shape, const std::string& target_id,
    const std::vector<em::LadderRef>& refs, em::km::ElementKind kind) {
    std::vector<em::LadderResolution> results(refs.size());
    std::vector<em::LadderRef> unresolved;
    std::vector<std::size_t> unresolved_indices;
    for (std::size_t i = 0; i < refs.size(); ++i) {
        const em::PartitionEntry* entry = ctx.partition.find(refs[i].element_id);
        // The id IS tracked, just not as what this slot needs. Falling through to
        // the descriptor stage here is how a FACE slot carrying an EDGE's id binds
        // a face out of the slot's own evidence and silently disagrees with the id
        // the record stores (WP-F review finding M4) — and, across bodies, how a
        // ref resolves against a body it never named (VF-M7). A tracked-but-
        // mismatched id is a record defect, so it is §9 STATE, never a rebind.
        if (entry != nullptr && (entry->body_id != target_id || entry->kind != kind)) {
            em::LadderResolution mismatch;
            mismatch.ref_id = refs[i].ref_id;
            mismatch.element_id = refs[i].element_id;
            mismatch.kind = refs[i].kind;
            mismatch.outcome = em::LadderOutcome::NeedsRepair;
            mismatch.ladder_level = "identity";
            mismatch.reason = "no-candidates";
            mismatch.anchor_json = refs[i].anchor_json;
            mismatch.ui_label =
                "'" + refs[i].element_id + "' is tracked as a " +
                em::ElementMapPartition::kind_name(entry->kind) + " on body " + entry->body_id +
                ", not a " + em::ElementMapPartition::kind_name(kind) + " on body " + target_id;
            results[i] = std::move(mismatch);
            continue;
        }
        const std::string live_topo_key =
            entry ? em::ElementMapPartition::topokey_for_shape(target_shape, entry->shape, kind)
                  : "";
        if (entry != nullptr && !live_topo_key.empty()) {
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
        resolve_refs_of_kind(ctx, target_shape, target_id, refs, em::km::ElementKind::Edge);
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

std::optional<OpOutcome> validate_chamfer(const TopoDS_Shape& result,
                                          kernel::validation::PublicationTier tier,
                                          const onecad::CancelToken* cancel) {
    const kernel::validation::PublicationDecision decision = publication_decision(
        result, kernel::validation::single_solid_policy("Chamfer", tier), cancel);
    if (cancel && cancel->cancelled()) return OpOutcome::cancelled();
    if (!decision.publishable()) return publication_refusal(decision, "publication");
    return std::nullopt;
}

// `reference_faces` is parallel to `edges`: the PERSISTED reference face of each
// seed (SCHEMA §7.3 `referenceFaces`). On an asymmetric chamfer EVERY entry is
// non-null by construction — a contour without a pair halted upstream as
// `legacyReferenceFace` — so a null one is an internal defect, not a fallback.
std::optional<OpOutcome> build_chamfer(const TopoDS_Shape& target_shape,
                                       const std::vector<TopoDS_Edge>& edges,
                                       const std::vector<TopoDS_Face>& reference_faces,
                                       double radius,
                                       bool two_distance, double distance2,
                                       bool distance_angle, double angle_rad,
                                       kernel::validation::PublicationTier validation_tier,
                                       const onecad::CancelToken* cancel,
                                       std::shared_ptr<BRepBuilderAPI_MakeShape>& builder,
                                       TopoDS_Shape& result) {
    TopTools_IndexedDataMapOfShapeListOfShape edge_faces;
    TopExp::MapShapesAndAncestors(target_shape, TopAbs_EDGE, TopAbs_FACE, edge_faces);
    // Both asymmetric forms measure `radius` on the same PERSISTED reference face.
    const bool needs_reference_face = two_distance || distance_angle;
    auto chamfer = std::make_shared<BRepFilletAPI_MakeChamfer>(target_shape);
    std::size_t added = 0;
    for (std::size_t seed = 0; seed < edges.size(); ++seed) {
        const TopoDS_Edge& edge = edges[seed];
        const int index = edge_faces.FindIndex(edge);
        if (index == 0 || edge_faces(index).IsEmpty()) continue;
        const TopTools_ListOfShape& faces = edge_faces(index);
        if (needs_reference_face) {
            // No ordinal fallback exists any more: reaching the kernel without a
            // resolved pair would mean something other than the persisted ref chose
            // the face, which is exactly the defect this work package removes.
            if (seed >= reference_faces.size() || reference_faces[seed].IsNull()) {
                return OpOutcome::fail(
                    "OP_FAILED",
                    "Chamfer reached the kernel without a resolved reference face");
            }
            const TopoDS_Face& face = reference_faces[seed];
            if (distance_angle) {
                // `AddDA` takes RADIANS; the wire field is degrees (SCHEMA §7.3).
                chamfer->AddDA(radius, angle_rad, edge, face);
            } else {
                chamfer->Add(radius, distance2, edge, face);
            }
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
    return validate_chamfer(result, validation_tier, cancel);
}

// ── SCHEMA §7.3 `referenceFaces` (kernel-hardening WP-F, 2026-09-03) ─────────
//
// An asymmetric chamfer measures `radius` on ONE of the seed edge's adjacent
// faces. Before WP-F that face was "the adjacent face with the smaller
// snapshot-scoped ordinal" — ordinal-derived, so an upstream feature that merely
// reordered the face map silently MIRRORED the two legs while the removed volume,
// the face count and every existing check stayed identical
// (worker/tests/test_chamfer_reference_face.cpp). It is now a PERSISTED typed
// ref: one `{edgeId, faceId}` pair per tangent CONTOUR, keyed by an edge of that
// contour, with the face's semantic ref riding `inputs[edgeIds.length + i]`.
struct ReferenceFacePair {
    std::string edge_id;
    std::string face_id;
};

struct EdgeValues {
    double radius = 0.0;
    bool two_distance = false;
    double distance2 = 0.0;
    bool distance_angle = false;
    double angle_rad = 0.0;
    int tangent_closure_version = 0;
    std::vector<ReferenceFacePair> reference_faces;
    // `params.edgeIds.length` — the index of the FIRST face slot in `inputs[]`.
    // `has_edge_ids` false ⇒ the array was absent or was not an array at all.
    std::size_t edge_id_count = 0;
    bool has_edge_ids = false;
    std::optional<OpOutcome> stop;
};

EdgeValues read_values(const json& op, Mode mode) {
    EdgeValues values;
    const json params =
        op.contains("params") && op["params"].is_object() ? op["params"] : json::object();

    // SCHEMA §7.3 (2026-08-24) FIELD IDENTITY, decided before any value is read.
    // `angleDeg` is Chamfer-only and mutually exclusive with `distance2`; both
    // refusals NAME the offending field. Resolving the pair by precedence instead
    // is how a user's second leg would silently disappear, and the worker is an
    // independent trust boundary — presence of the key is presence, malformed or
    // not.
    const bool angle_present = params.contains("angleDeg");
    if (angle_present && mode == Mode::Fillet) {
        values.stop = OpOutcome::fail("OP_FAILED", "Fillet must not carry angleDeg (Chamfer only)");
        return values;
    }
    if (angle_present && params.contains("distance2")) {
        values.stop = OpOutcome::fail("OP_FAILED",
                                      "Chamfer angleDeg and distance2 are mutually exclusive");
        return values;
    }

    // Parameter validation runs before the target body is resolved, and
    // `authoring_resolution()` is scale-independent in v1 (GeometryPrecision.h),
    // so the floor-only context answers exactly what a measured one would.
    const double min_value =
        kernel::validation::GeometryPrecisionContext{}.authoring_resolution();
    values.radius = read_scalar(params, "radius", 0.0);
    if (!std::isfinite(values.radius) || values.radius < min_value) {
        values.stop = OpOutcome::fail(
            "OP_FAILED",
            mode == Mode::Fillet ? "Fillet radius too small" : "Chamfer distance too small");
        return values;
    }
    values.two_distance = mode == Mode::Chamfer && has_scalar(params, "distance2");
    values.distance2 =
        values.two_distance ? read_scalar(params, "distance2", 0.0) : 0.0;
    if (values.two_distance &&
        (!std::isfinite(values.distance2) || values.distance2 < min_value)) {
        values.stop = OpOutcome::fail("OP_FAILED", "Chamfer distance2 too small");
    }
    if (angle_present) {  // Chamfer only — the Fillet case already returned.
        double angle_deg = 0.0;
        std::string angle_error;
        if (!read_scalar_strict(params, "angleDeg", 0.0, angle_deg, angle_error)) {
            values.stop = OpOutcome::fail("OP_FAILED", "Chamfer " + angle_error);
            return values;
        }
        // DELIBERATELY LOOSE: the true ceiling depends on the dihedral, so
        // feasibility beyond this stays a recoverable OP_FAILED out of the build.
        if (angle_deg <= 0.0 || angle_deg >= 180.0) {
            values.stop =
                OpOutcome::fail("OP_FAILED", "Chamfer angleDeg must be within (0, 180) degrees");
            return values;
        }
        values.distance_angle = true;
        values.angle_rad = angle_deg * M_PI / 180.0;
    }
    if (params.contains("tangentClosureVersion")) {
        if (!params["tangentClosureVersion"].is_number_integer() ||
            params["tangentClosureVersion"].get<int>() != 1) {
            values.stop = OpOutcome::fail("OP_FAILED", "Unsupported tangent closure version");
        } else {
            values.tangent_closure_version = 1;
        }
    }
    if (params.contains("edgeIds") && params["edgeIds"].is_array()) {
        values.has_edge_ids = true;
        values.edge_id_count = params["edgeIds"].size();
    }
    // SCHEMA §7.3 `referenceFaces` is read ONLY on an ASYMMETRIC Chamfer. An
    // equal-leg chamfer and a Fillet have no reference face at all, so they ignore
    // the field entirely — exactly as a Fillet ignores `distance2` — and a stray
    // field can never change their geometry or refuse their record.
    if (!values.stop && mode == Mode::Chamfer &&
        (values.two_distance || values.distance_angle) && params.contains("referenceFaces")) {
        const json& list = params["referenceFaces"];
        if (!list.is_array()) {
            values.stop =
                OpOutcome::fail("OP_FAILED", "Chamfer referenceFaces must be an array of pairs");
            return values;
        }
        for (const json& entry : list) {
            const std::string edge_id = read_str(entry, "edgeId");
            const std::string face_id = read_str(entry, "faceId");
            if (edge_id.empty() || face_id.empty()) {
                values.stop = OpOutcome::fail(
                    "OP_FAILED",
                    "Chamfer referenceFaces entry needs a non-empty edgeId and faceId");
                return values;
            }
            values.reference_faces.push_back({edge_id, face_id});
        }
        // The face refs are addressed POSITIONALLY (`inputs[edgeIds.length + i]`),
        // so without a readable `edgeIds` array their slots cannot be numbered at
        // all. Refuse rather than guess a base that would read an EDGE ref as the
        // face `radius` is measured on.
        if (!values.reference_faces.empty() && !values.has_edge_ids) {
            values.stop = OpOutcome::fail(
                "OP_FAILED",
                "Chamfer referenceFaces requires an edgeIds array to address its input slots");
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

// One tangent contour of the frozen closure, as indices into the RESOLVED edge
// arrays. `seed` is the contour's smallest-ordinal edge — the historical,
// ordinal-derived seed — until a `referenceFaces` pair overrides it.
struct EdgeGroup {
    std::vector<std::size_t> members;
    std::size_t seed = 0;
};

// Validate the stored closure against the live body and group the resolved edges
// by tangent contour. CONTOUR ORDER is `analyze_edge_contours`'s seed order,
// whose seeds are `resolved.edges` in `inputs[]` order — a pure function of the op
// payload, which is what makes the `legacyReferenceFace` slot numbering below
// stable across replays of the same document.
std::optional<OpOutcome> group_by_contour(const TopoDS_Shape& body, Mode mode,
                                          const EdgeResolution& resolved,
                                          const TopTools_IndexedMapOfShape& edge_map,
                                          std::vector<EdgeGroup>& groups) {
    std::vector<int> expected;
    std::vector<int> ordinal_of;  // parallel to resolved.edges
    for (const TopoDS_Edge& edge : resolved.edges) {
        const int ordinal = edge_map.FindIndex(edge);
        if (std::find(expected.begin(), expected.end(), ordinal) != expected.end()) {
            return closure_changed("Stored tangent closure collapsed onto one edge", expected,
                                   {ordinal});
        }
        expected.push_back(ordinal);
        ordinal_of.push_back(ordinal);
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
    for (const std::vector<int>& contour : contours.contours) {
        EdgeGroup group;
        bool seeded = false;
        for (std::size_t index = 0; index < ordinal_of.size(); ++index) {
            if (std::find(contour.begin(), contour.end(), ordinal_of[index]) == contour.end()) {
                continue;
            }
            group.members.push_back(index);
            if (ordinal_of[index] == contour.front()) {
                group.seed = index;
                seeded = true;
            }
        }
        // Unreachable while the closure comparison above holds; kept because the
        // pre-WP-F code likewise SKIPPED a contour it could not seed rather than
        // binding a wrong edge to it.
        if (!seeded) continue;
        groups.push_back(std::move(group));
    }
    return std::nullopt;
}

// Execution is SEED-ONLY: OCCT expands each seed back to its whole tangent
// contour, so handing it every closure edge would add the same contour N times.
void trim_to_seeds(EdgeResolution& resolved, const std::vector<EdgeGroup>& groups) {
    std::vector<TopoDS_Edge> edge_seeds;
    std::vector<kernel::fillet::ResolvedEdge> fillet_seeds;
    for (const EdgeGroup& group : groups) {
        edge_seeds.push_back(resolved.edges[group.seed]);
        fillet_seeds.push_back(resolved.fillet_edges[group.seed]);
    }
    resolved.edges = std::move(edge_seeds);
    resolved.fillet_edges = std::move(fillet_seeds);
}

// One op input by index; `nullptr` when the slot does not exist.
const json* input_at(const json& op, std::size_t index) {
    if (!op.contains("inputs") || !op["inputs"].is_array() || index >= op["inputs"].size()) {
        return nullptr;
    }
    return &op["inputs"][index];
}

// A §9 item for a face slot the ladder never got to run on (the slot carries a
// ref of the wrong kind). Shaped exactly like `to_needs_repair_json`.
json face_ref_needs_repair(const std::string& ref_id, const std::string& element_id,
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

struct ReferenceFaceResolution {
    std::vector<TopoDS_Face> faces;  // parallel to the pairs; null ⇒ unresolved
    std::vector<json> needs_repair;
};

// Bind each pair's FACE through the same two rungs the op's edge refs took. The
// pair NAMES the face; `inputs[slot_base + i]` carries its evidence, so a record
// whose slot is empty still binds on the identity rung from the pair's id alone.
ReferenceFaceResolution resolve_reference_faces(OpContext& ctx, const json& op,
                                                const std::string& op_id,
                                                const std::string& target_id,
                                                const TopoDS_Shape& target_shape,
                                                const std::vector<ReferenceFacePair>& pairs,
                                                std::size_t slot_base) {
    ReferenceFaceResolution out;
    out.faces.resize(pairs.size());
    std::vector<em::LadderRef> refs;
    std::vector<std::size_t> slots;
    for (std::size_t i = 0; i < pairs.size(); ++i) {
        const std::string ref_id = op_id + ".input" + std::to_string(slot_base + i);
        const json* in = input_at(op, slot_base + i);
        em::LadderRef r;
        if (in != nullptr) r = em::ladder_ref_from_input(*in, ref_id);
        r.ref_id = ref_id;
        if (r.element_id.empty()) r.element_id = pairs[i].face_id;
        if (r.kind == em::km::ElementKind::Unknown) r.kind = em::km::ElementKind::Face;
        if (r.kind != em::km::ElementKind::Face) {
            out.needs_repair.push_back(face_ref_needs_repair(
                ref_id, pairs[i].face_id,
                "Chamfer reference face slot does not carry a face ref: " + pairs[i].face_id));
            continue;
        }
        refs.push_back(std::move(r));
        slots.push_back(i);
    }
    const std::vector<em::LadderResolution> res =
        resolve_refs_of_kind(ctx, target_shape, target_id, refs, em::km::ElementKind::Face);
    for (std::size_t k = 0; k < res.size() && k < slots.size(); ++k) {
        if (res[k].outcome == em::LadderOutcome::AutoBind && !res[k].bound_shape.IsNull() &&
            res[k].bound_shape.ShapeType() == TopAbs_FACE) {
            out.faces[slots[k]] = TopoDS::Face(res[k].bound_shape);
        } else {
            out.needs_repair.push_back(res[k].to_needs_repair_json());
        }
    }
    return out;
}

// Refuse BY NAME with ONE §7.2 diagnostic. SCHEMA §7.3 requires each reference-face
// rule to fail loudly with both ids in the evidence — never to fall back to the
// ordinal rule, and never to bind the nearest face instead.
OpOutcome reference_face_refusal(const char* code, const std::string& message,
                                 const std::string& edge_id, const std::string& face_id) {
    OpOutcome failure = OpOutcome::fail("OP_FAILED", message);
    failure.diagnostics.push_back(
        {{"severity", "error"},
         {"code", code},
         {"message", message},
         {"stage", "resolve"},
         {"evidence", {{"chamfer", {{"edge", edge_id}, {"face", face_id}}}}}});
    return failure;
}

// The OP-BUILT §9 item (SCHEMA §9 `reason: "legacyReferenceFace"`) for a contour
// whose reference face is not persisted and may no longer be derived from an
// ordinal. NOT a ladder outcome: no ladder ran, and the slot it names is EMPTY, so
// the repair is a CREATE, not a rebind. `ladderFailed` carries "descriptor"
// because §9's level enum is closed and `reason` is the discriminator. The
// candidates are a DELIBERATE TIE (0.5 / margin 0) — the user MUST choose.
json legacy_reference_face_item(const TopoDS_Shape& body,
                                const TopTools_IndexedDataMapOfShapeListOfShape& edge_faces,
                                const TopTools_IndexedMapOfShape& face_map,
                                const TopTools_IndexedMapOfShape& edge_map,
                                const std::string& ref_id,
                                const kernel::fillet::ResolvedEdge& seed) {
    json candidates = json::array();
    for (const int ordinal : session::adjacent_face_ordinals(edge_faces, face_map, seed.edge)) {
        const em::km::ElementDescriptor d =
            em::ElementMapPartition::describe(face_map(ordinal), body);
        candidates.push_back(
            json{{"topoKey", "f:" + std::to_string(ordinal)},
                 {"score", 0.5},
                 {"margin", 0.0},
                 {"worldPos", {d.center.X(), d.center.Y(), d.center.Z()}},
                 {"summary", em::candidate_summary(em::km::ElementKind::Face, d)},
                 {"featureContributions", json::object()}});
    }
    const em::km::ElementDescriptor edge_d =
        em::ElementMapPartition::describe(seed.edge, body);
    return json{
        {"refId", ref_id},
        {"elementId", ""},
        {"ladderFailed", "descriptor"},
        {"reason", "legacyReferenceFace"},
        {"scoringVersion", em::kResolverVersion},
        {"seedEdgeId", seed.element_id},
        {"candidates", std::move(candidates)},
        {"anchor", {{"worldPoint", {edge_d.center.X(), edge_d.center.Y(), edge_d.center.Z()}}}},
        {"uiLabel",
         "Chamfer reference face for e:" + std::to_string(edge_map.FindIndex(seed.edge))}};
}

// Bind `params.referenceFaces` onto the contours, decide each contour's seed and
// reference face, and halt on anything that is not decidable. On return, either
// an outcome (a by-name refusal, or a `needsRepair` halt) or:
//   * `groups[g].seed`  — the PAIRED edge where a pair exists, else unchanged;
//   * `seed_faces[g]`   — the resolved face for EVERY group (a contour without a
//                          pair halts `needsRepair` before this returns; there is no
//                          ordinal fallback any more).
std::optional<OpOutcome> bind_reference_faces(OpContext& ctx, const json& op,
                                              const std::string& op_id,
                                              const std::string& target_id,
                                              const TopoDS_Shape& body,
                                              const EdgeValues& values,
                                              const EdgeResolution& resolved,
                                              const TopTools_IndexedMapOfShape& edge_map,
                                              std::vector<EdgeGroup>& groups,
                                              std::vector<TopoDS_Face>& seed_faces) {
    // A legacy record carries no `edgeIds` in some historical shapes; its face
    // slots start right after its edge refs, which is all of `inputs[]`.
    const std::size_t slot_base =
        values.has_edge_ids ? values.edge_id_count : resolved.fillet_edges.size();

    ReferenceFaceResolution bound = resolve_reference_faces(
        ctx, op, op_id, target_id, body, values.reference_faces, slot_base);
    if (!bound.needs_repair.empty()) {
        OpOutcome halt;
        halt.needs_repair = std::move(bound.needs_repair);
        return halt;
    }

    // Hoisted ONCE for the whole op: every adjacency question below reads them.
    TopTools_IndexedMapOfShape face_map;
    TopExp::MapShapes(body, TopAbs_FACE, face_map);
    TopTools_IndexedDataMapOfShapeListOfShape edge_faces;
    TopExp::MapShapesAndAncestors(body, TopAbs_EDGE, TopAbs_FACE, edge_faces);

    struct Binding {
        std::size_t edge_index = 0;
        TopoDS_Face face;
        std::string edge_id;
    };
    std::map<std::size_t, Binding> by_group;
    for (std::size_t i = 0; i < values.reference_faces.size(); ++i) {
        const ReferenceFacePair& pair = values.reference_faces[i];
        // Keyed by ElementId ONLY. Matching a TopoKey here would be ordinal
        // matching inside the very path that exists to remove ordinals from this
        // decision (WP-F review finding L6).
        std::optional<std::size_t> edge_index;
        for (std::size_t e = 0; e < resolved.fillet_edges.size(); ++e) {
            if (resolved.fillet_edges[e].element_id == pair.edge_id) {
                edge_index = e;
                break;
            }
        }
        std::optional<std::size_t> group_index;
        if (edge_index) {
            for (std::size_t g = 0; g < groups.size(); ++g) {
                if (std::find(groups[g].members.begin(), groups[g].members.end(), *edge_index) !=
                    groups[g].members.end()) {
                    group_index = g;
                    break;
                }
            }
        }
        // An edge the chamfer does not execute cannot be adjacent to anything the
        // chamfer measures on, so the pair is unusable either way.
        if (!group_index) {
            return reference_face_refusal(
                "CHAMFER_REFERENCE_FACE_NOT_ADJACENT",
                "Chamfer reference face pair names edge '" + pair.edge_id +
                    "', which is not an edge of the chamfer's closure",
                pair.edge_id, pair.face_id);
        }
        const TopoDS_Face& face = bound.faces[i];
        const int face_ordinal = face.IsNull() ? 0 : face_map.FindIndex(face);
        const std::vector<int> adjacent = session::adjacent_face_ordinals(
            edge_faces, face_map, resolved.fillet_edges[*edge_index].edge);
        if (face_ordinal <= 0 ||
            std::find(adjacent.begin(), adjacent.end(), face_ordinal) == adjacent.end()) {
            return reference_face_refusal(
                "CHAMFER_REFERENCE_FACE_NOT_ADJACENT",
                "Chamfer reference face '" + pair.face_id + "' is not adjacent to edge '" +
                    pair.edge_id + "'",
                pair.edge_id, pair.face_id);
        }
        const auto existing = by_group.find(*group_index);
        if (existing == by_group.end()) {
            by_group.emplace(*group_index, Binding{*edge_index, face, pair.edge_id});
            continue;
        }
        // A contour has exactly ONE reference face, so two pairs that disagree are
        // a record defect — resolving them by precedence would silently drop one
        // of the user's two answers.
        if (!existing->second.face.IsSame(face)) {
            return reference_face_refusal(
                "CHAMFER_REFERENCE_FACE_CONFLICT",
                "Chamfer reference faces disagree on one tangent contour: '" + pair.face_id +
                    "' does not match the face already paired with edge '" +
                    existing->second.edge_id + "'",
                pair.edge_id, pair.face_id);
        }
    }

    std::vector<json> items;
    seed_faces.assign(groups.size(), TopoDS_Face());
    for (std::size_t g = 0; g < groups.size(); ++g) {
        const auto found = by_group.find(g);
        if (found != by_group.end()) {
            // THE PAIRED EDGE IS THE CONTOUR'S SEED: the kernel measures `radius`
            // on the resolved face AT that edge, so nothing here is ordinal-derived.
            groups[g].seed = found->second.edge_index;
            seed_faces[g] = found->second.face;
            continue;
        }
        // No pair for this contour, and no fallback exists: the ordinal rule is
        // gone, so EVERY uncovered contour halts, in EVERY lane, regardless of
        // `editedFrom`. Gating this on `post_upstream_edit` would have made the
        // guard fire on open and vanish on redo (WP-F review, 2026-09-04). The
        // items are per-contour, so a partially typed record converges one pick at
        // a time instead of dead-ending.
        const std::size_t slot = slot_base + values.reference_faces.size() + items.size();
        items.push_back(legacy_reference_face_item(
            body, edge_faces, face_map, edge_map, op_id + ".input" + std::to_string(slot),
            resolved.fillet_edges[groups[g].seed]));
    }
    if (!items.empty()) {
        OpOutcome halt;
        halt.needs_repair = std::move(items);
        return halt;
    }
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
    if (auto invalid = validate_modeling_body(*target, name, "target")) return *invalid;
    const EdgeValues values = read_values(op, mode);
    if (values.stop) return *values.stop;
    EdgeResolution resolved = resolve_edges(ctx, op, op_id, target_id, target->geom, name);
    if (resolved.stop) return std::move(*resolved.stop);

    // Hoisted once: the contour grouping and the `legacyReferenceFace` items both
    // read it (WP-F review finding L9 — no per-edge map rebuilds).
    TopTools_IndexedMapOfShape edge_map;
    TopExp::MapShapes(target->geom, TopAbs_EDGE, edge_map);
    std::vector<EdgeGroup> groups;
    if (values.tangent_closure_version == 1) {
        const std::optional<OpOutcome> drift =
            group_by_contour(target->geom, mode, resolved, edge_map, groups);
        if (drift) return *drift;
    } else {
        // A non-v1 record has NO frozen closure: it executes seed-only, one
        // reference face per resolved EDGE (SCHEMA §7.3 `tangentClosureVersion`),
        // so each edge is its own group and nothing is trimmed.
        for (std::size_t index = 0; index < resolved.edges.size(); ++index) {
            groups.push_back(EdgeGroup{{index}, index});
        }
    }
    // Parallel to the FINAL `resolved.edges`; every entry is a resolved face on an
    // asymmetric chamfer (a missing pair halted above), and `build_chamfer` refuses a
    // null entry by name — the ordinal fallback is gone.
    std::vector<TopoDS_Face> seed_reference_faces;
    if (mode == Mode::Chamfer && (values.two_distance || values.distance_angle)) {
        std::optional<OpOutcome> stop =
            bind_reference_faces(ctx, op, op_id, target_id, target->geom, values, resolved,
                                 edge_map, groups, seed_reference_faces);
        if (stop) return std::move(*stop);
    }
    if (values.tangent_closure_version == 1) trim_to_seeds(resolved, groups);
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
                                                     target->geom, resolved.edges,
                                                     seed_reference_faces, values.radius,
                                                     values.two_distance, values.distance2,
                                                     values.distance_angle, values.angle_rad,
                                                     result_validation_tier(
                                                         ctx, kernel::validation::PublicationTier::TierB),
                                                     ctx.cancel, builder, result);
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
