// OpCommon.cpp — see OpCommon.h. Ports from OneCAD-CPP RegenerationEngine.cpp.
#include "ops/OpCommon.h"

#include <algorithm>
#include <array>
#include <cmath>
#include <cstdint>
#include <optional>

#include <BOPAlgo_Operation.hxx>
#include <BRepAdaptor_Surface.hxx>
#include <BRepAlgoAPI_BooleanOperation.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <BRepClass_FaceClassifier.hxx>
#include <BRepGProp.hxx>
#include <GProp_GProps.hxx>
#include <GeomAbs_SurfaceType.hxx>
#include <Message_ProgressRange.hxx>
#include <Standard_Failure.hxx>
#include <TopAbs_Orientation.hxx>
#include <TopAbs_State.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopTools_ListOfShape.hxx>
#include <TopoDS.hxx>
#include <gp_Pnt.hxx>

#include "loop/FaceBuilder.h"
#include "loop/LoopDetector.h"
#include "loop/RegionTable.h"
#include "loop/RegionUtils.h"
#include "elementmap/Ladder.h"
#include "kernel/validation/GeometryPrecision.h"
#include "kernel/validation/ShapeAudit.h"
#include "ops/CancelProgress.h"
#include "sketch/WireSketch.h"

namespace onecad::ops {

namespace sk = onecad::core::sketch;
namespace loop = onecad::core::loop;
using nlohmann::json;

double read_scalar(const json& params, const char* key, double dflt) {
    if (!params.is_object() || !params.contains(key)) return dflt;
    const json& v = params[key];
    if (v.is_number()) return v.get<double>();
    if (v.is_object() && v.contains("value") && v["value"].is_number()) {
        return v["value"].get<double>();
    }
    return dflt;
}

bool read_scalar_strict(const json& params, const char* key, double dflt,
                        double& value_out, std::string& error_out) {
    value_out = dflt;
    if (!params.is_object() || !params.contains(key)) return true;
    const json& value = params[key];
    if (value.is_number()) {
        value_out = value.get<double>();
    } else if (value.is_object() && value.contains("value") && value["value"].is_number()) {
        value_out = value["value"].get<double>();
    } else {
        error_out = std::string(key) + " must be a finite scalar";
        return false;
    }
    if (!std::isfinite(value_out)) {
        error_out = std::string(key) + " must be a finite scalar";
        return false;
    }
    return true;
}

std::string read_str(const json& o, const char* key, const std::string& dflt) {
    if (o.is_object() && o.contains(key) && o[key].is_string()) return o[key].get<std::string>();
    return dflt;
}

bool read_bool_strict(const json& params, const char* key, bool dflt,
                      bool& value_out, std::string& error_out) {
    value_out = dflt;
    if (!params.is_object() || !params.contains(key)) return true;
    if (!params[key].is_boolean()) {
        error_out = std::string(key) + " must be a boolean";
        return false;
    }
    value_out = params[key].get<bool>();
    return true;
}

bool read_string_array_strict(const json& params, const char* key,
                              std::vector<std::string>& value_out,
                              std::string& error_out) {
    value_out.clear();
    if (!params.is_object() || !params.contains(key)) return true;
    const json& value = params[key];
    if (!value.is_array()) {
        error_out = std::string(key) + " must be an array of ids";
        return false;
    }
    for (const json& entry : value) {
        if (!entry.is_string()) {
            error_out = std::string(key) + " must contain only string ids";
            return false;
        }
        std::string id = entry.get<std::string>();
        if (id.empty()) {
            error_out = std::string(key) + " must not contain an empty id";
            return false;
        }
        value_out.push_back(std::move(id));
    }
    return true;
}

bool read_region_anchor(const json& params,
                        std::optional<std::array<double, 2>>& value_out,
                        std::string& error_out) {
    value_out.reset();
    if (!params.is_object() || !params.contains("regionAnchor")) return true;
    const json& value = params["regionAnchor"];
    if (!value.is_array() || value.size() != 2) {
        error_out = "regionAnchor must be [u, v]";
        return false;
    }
    std::array<double, 2> anchor{};
    for (std::size_t i = 0; i < 2; ++i) {
        if (!value[i].is_number()) {
            error_out = "regionAnchor must be [u, v]";
            return false;
        }
        anchor[i] = value[i].get<double>();
        if (!std::isfinite(anchor[i])) {
            error_out = "regionAnchor must be finite";
            return false;
        }
    }
    value_out = anchor;
    return true;
}

kernel::validation::PublicationDecision publication_decision(
    const TopoDS_Shape& shape, const kernel::validation::PublicationPolicy& policy,
    const onecad::CancelToken* cancel) {
    const kernel::validation::ShapeEvidence evidence =
        kernel::validation::collect_shape_evidence(shape, policy.tier, cancel);
    return kernel::validation::evaluate_publication_policy(evidence, policy);
}

OpOutcome publication_refusal(const kernel::validation::PublicationDecision& decision,
                              const char* stage, const std::string& message_override) {
    const std::string message =
        message_override.empty() ? decision.message : message_override;
    OpOutcome failure = OpOutcome::fail(decision.code, message);
    failure.diagnostics.push_back({{"severity", "error"},
                                   {"code", decision.code},
                                   {"message", message},
                                   {"stage", stage},
                                   {"reasonCode", decision.reason_code},
                                   {"evidence", decision.evidence.to_json()}});
    return failure;
}

kernel::validation::PublicationTier result_validation_tier(
    const OpContext& ctx, kernel::validation::PublicationTier authoritative) {
    if (ctx.validation_mode == ValidationMode::PreviewInteractive)
        return kernel::validation::PublicationTier::TierA;
    return authoritative;
}

std::optional<OpOutcome> validate_modeling_input(const TopoDS_Shape& shape,
                                                 const std::string& operation,
                                                 const std::string& role) {
    kernel::validation::PublicationPolicy policy =
        kernel::validation::single_solid_policy(operation + " " + role,
                                                kernel::validation::PublicationTier::TierA);
    policy.allowed_top_level_shapes = kernel::validation::TopLevelShapePolicy::SolidSet;
    policy.max_solid_count = -1;
    const kernel::validation::PublicationDecision decision = publication_decision(shape, policy);
    if (decision.publishable()) return std::nullopt;
    OpOutcome failure = OpOutcome::fail("INVALID_MODELING_INPUT", decision.message);
    failure.diagnostics.push_back({{"severity", "error"},
                                   {"code", "INVALID_MODELING_INPUT"},
                                   {"message", decision.message},
                                   {"stage", "input-validation"},
                                   {"evidence", decision.evidence.to_json()},
                                   {"timings", decision.timings.to_json()},
                                   {"role", role}});
    return failure;
}

std::optional<OpOutcome> validate_modeling_body(
    const session::BodyRecord& body, const std::string& operation,
    const std::string& role) {
    if (!body.modeling_eligible()) {
        const std::string message = operation + " cannot use quarantined " + role +
                                    " body " + body.id +
                                    (body.health_reason.empty()
                                         ? std::string{}
                                         : ": " + body.health_reason);
        OpOutcome failure = OpOutcome::fail("OP_FAILED", message);
        failure.diagnostics.push_back({{"severity", "error"},
                                       {"code", "QUARANTINED_MODELING_INPUT"},
                                       {"message", message},
                                       {"stage", "input-validation"},
                                       {"role", role},
                                       {"bodyId", body.id}});
        return failure;
    }
    return validate_modeling_input(body.geom, operation, role);
}

namespace {

json ownership_repair(const json& input, const std::string& ref_id,
                      const std::string& message) {
    json anchor = json::object();
    std::string element_id;
    if (input.is_object()) {
        if (input.contains("anchor") && input["anchor"].is_object()) anchor = input["anchor"];
        if (input.contains("primary") && input["primary"].is_object()) {
            element_id = read_str(input["primary"], "elementId");
        }
    }
    return json{{"refId", ref_id},
                {"elementId", element_id},
                {"ladderFailed", "descriptor"},
                {"reason", "no-candidates"},
                {"scoringVersion", elementmap::kResolverVersion},
                {"candidates", json::array()},
                {"anchor", std::move(anchor)},
                {"uiLabel", message}};
}

const json* input_at(const json& op, std::size_t index) {
    if (!op.contains("inputs") || !op["inputs"].is_array() || index >= op["inputs"].size()) {
        return nullptr;
    }
    return &op["inputs"][index];
}

const json* primary_of(const json* input) {
    if (input == nullptr || !input->is_object() || !input->contains("primary") ||
        !(*input)["primary"].is_object()) {
        return nullptr;
    }
    return &(*input)["primary"];
}

void append_face_ownership_repair(std::vector<json>& out, const json* input,
                                  const std::string& ref_id, const std::string& target_id,
                                  const char* op_name) {
    const json* primary = primary_of(input);
    if (primary == nullptr || read_str(*primary, "kind") != "face") {
        out.push_back(ownership_repair(input ? *input : json(), ref_id,
                                       std::string(op_name) + " requires a typed face primary"));
        return;
    }
    const std::string body_id = read_str(*primary, "bodyId");
    if (body_id.empty() || body_id != target_id) {
        out.push_back(ownership_repair(
            *input, ref_id, std::string(op_name) + " face primary body " +
                                 (body_id.empty() ? "<missing>" : body_id) +
                                 " does not match target body " + target_id));
    }
}

}  // namespace

std::vector<json> operation_ref_ownership_repairs(const json& op, const std::string& op_id) {
    std::vector<json> out;
    const std::string type = read_str(op, "opType");
    const json params = op.contains("params") && op["params"].is_object()
                            ? op["params"]
                            : json::object();

    if (type == "Fillet" || type == "Chamfer") {
        std::string target_id;
        if (!op.contains("inputs") || !op["inputs"].is_array()) return out;
        for (std::size_t i = 0; i < op["inputs"].size(); ++i) {
            const json& input = op["inputs"][i];
            const json* primary = primary_of(&input);
            const std::string ref_id = op_id + ".input" + std::to_string(i);
            if (primary == nullptr || read_str(*primary, "kind") != "edge") {
                out.push_back(ownership_repair(
                    input, ref_id, type + " requires every typed edge ref to carry an edge primary"));
                continue;
            }
            const std::string body_id = read_str(*primary, "bodyId");
            if (body_id.empty()) {
                out.push_back(ownership_repair(input, ref_id,
                                                type + " edge primary has no operated body"));
                continue;
            }
            if (target_id.empty()) {
                target_id = body_id;
            } else if (body_id != target_id) {
                out.push_back(ownership_repair(
                    input, ref_id, type + " edge primary body " + body_id +
                                       " does not match operated body " + target_id));
            }
        }
        return out;
    }

    if (type == "Shell") {
        std::string target_id = read_str(params, "targetBodyId");
        if (target_id.empty() && op.contains("inputs") && op["inputs"].is_array()) {
            for (const json& input : op["inputs"]) {
                const json* primary = primary_of(&input);
                if (primary != nullptr && read_str(*primary, "kind") == "face") {
                    target_id = read_str(*primary, "bodyId");
                    if (!target_id.empty()) break;
                }
            }
        }
        if (target_id.empty()) return out;
        if (!op.contains("inputs") || !op["inputs"].is_array()) return out;
        for (std::size_t i = 0; i < op["inputs"].size(); ++i) {
            const json* input = input_at(op, i);
            const json* primary = primary_of(input);
            if (primary != nullptr && read_str(*primary, "kind") == "body") continue;
            append_face_ownership_repair(out, input, op_id + ".input" + std::to_string(i),
                                         target_id, "Shell");
        }
        return out;
    }

    if (type == "Hole") {
        const std::string target_id = read_str(params, "targetBodyId");
        if (target_id.empty()) return out;
        // SCHEMA §7.3 reserves slot 0 for the host body and slot 1 for the host
        // face. Keep the positional contract even when a malformed primary lacks
        // its `kind`: otherwise it could slip through to `params.face` fallback.
        if (const json* face_input = input_at(op, 1)) {
            append_face_ownership_repair(out, face_input, op_id + ".input1", target_id,
                                         "Hole");
        } else if (params.contains("face") && params["face"].is_object()) {
            append_face_ownership_repair(out, &params["face"], op_id + ".input1", target_id,
                                         "Hole");
        }
        return out;
    }

    if (type == "OffsetFace") {
        const std::string target_id = read_str(params, "targetBodyId");
        if (target_id.empty()) return out;
        if (!op.contains("inputs") || !op["inputs"].is_array()) return out;
        for (std::size_t i = 0; i < op["inputs"].size(); ++i) {
            append_face_ownership_repair(out, input_at(op, i),
                                         op_id + ".input" + std::to_string(i), target_id,
                                         "OffsetFace");
        }
    }
    return out;
}

namespace {

// The one cell of `table` whose built face CONTAINS `anchor` (SCHEMA §7.3 "Region
// anchor"). Iteration follows table order and nothing else, so the verdict is
// deterministic. `TopAbs_ON` deliberately does not count, and a SECOND `TopAbs_IN`
// hit returns "no unique cell" instead of picking one: nothing here can rank two
// containing cells, so a guess would be exactly the silent mis-bind this project
// exists to prevent. Do NOT weaken this to "the table forbids overlapping cells" —
// `buildRegionTable` enforces only id uniqueness; disjointness comes from
// `LoopDetector::findFaces` and hole parenting is a tessellation test that can
// fail (probe P-B2). The face is built with the SAME `loop::FaceBuilder` call the
// selected cell uses, so a cell that cannot be built cannot be bound either.
const loop::RegionDefinition* cell_containing_anchor(const loop::RegionTable& table,
                                                     const sk::Sketch& sketch,
                                                     const std::array<double, 2>& anchor) {
    const gp_Pln plane = loop::FaceBuilder::sketchPlaneToGpPln(sketch.getPlane());
    const gp_Pnt point = loop::FaceBuilder::toGpPnt(anchor[0], anchor[1], plane);
    const double tolerance =
        kernel::validation::GeometryPrecisionContext{}.authoring_resolution();
    const loop::FaceBuilder builder;
    const loop::RegionDefinition* found = nullptr;
    for (const loop::RegionDefinition& region : table.regions) {
        loop::Face candidate;
        candidate.outerLoop = region.outerLoop;
        candidate.innerLoops = region.holes;
        const loop::FaceBuildResult built = builder.buildFace(candidate, sketch);
        if (!built.success || built.face.IsNull()) continue;
        BRepClass_FaceClassifier classifier(built.face, point, tolerance);
        if (classifier.State() != TopAbs_IN) continue;
        if (found != nullptr) return nullptr;
        found = &region;
    }
    return found;
}

json anchor_rebind_diagnostic(const std::string& stale_id, const std::string& bound_id,
                              const std::array<double, 2>& anchor) {
    return json{{"severity", "warning"},
                {"code", "REGION_REBOUND_BY_ANCHOR"},
                {"message", "profile: regionId '" + stale_id +
                                "' matched no cell after an edit; bound the cell "
                                "containing regionAnchor"},
                {"stage", "profile"},
                {"evidence",
                 json{{"region", json{{"from", stale_id},
                                      {"to", bound_id},
                                      {"anchor", json::array({anchor[0], anchor[1]})}}}}}};
}

// Kernel-hardening WP-B: an entity below the authoring resolution carries no
// usable boundary, so region detection drops it and keeps going. The step still
// succeeds; this is how the user learns which entity was ignored.
json degenerate_entity_diagnostic(const loop::DetectionWarning& warning) {
    return json{{"severity", "warning"},
                {"code", "SKETCH_ENTITY_DEGENERATE"},
                {"message", "profile: " + warning.text()},
                {"stage", "profile"},
                {"evidence", json{{"entityId", warning.entityId}}}};
}

}  // namespace

std::optional<TopoDS_Face> build_profile_face(
    const json& sketch_params, const std::string& region_id,
    std::optional<int> region_identity_version, std::string& err,
    std::vector<json>* diagnostics_out,
    const std::optional<std::array<double, 2>>& region_anchor) {
    // Sketch params → live Sketch (plane + entities + constraints). Mirrors
    // RegenerationEngine.cpp:1639-1667 buildFaceFromSketchRegion, but the sketch
    // is supplied inline in the plan (deterministic replay) rather than looked up
    // from a document.
    wire::TranslateResult tr = wire::translate(sketch_params);
    if (!tr.ok) {
        err = "profile: " + tr.error;
        return std::nullopt;
    }
    const sk::SolveResult solve = tr.sketch->solve();
    if (!solve.success) {
        err = "profile: solve failed: " +
              (solve.errorMessage.empty() ? std::string("constraint solve did not converge")
                                          : solve.errorMessage);
        return std::nullopt;
    }

    if (region_identity_version.has_value() &&
        *region_identity_version != 2 && *region_identity_version != 3) {
        err = "profile: unsupported regionIdentityVersion " +
              std::to_string(*region_identity_version);
        return std::nullopt;
    }
    loop::LoopDetectorConfig detection_config = loop::makeRegionDetectionConfig();
    const bool exact_identity = region_identity_version == 2 || region_identity_version == 3;
    detection_config.exactAnalyticFragments = exact_identity;
    if (region_identity_version == 3) {
        detection_config.curveRefinementPolicy =
            loop::CurveRefinementPolicy::V3PhysicalProximity;
    }
    loop::LoopDetector detector;
    detector.setConfig(detection_config);
    const loop::LoopDetectionResult det = detector.detect(*tr.sketch);
    const auto map_edge = [&](const sk::EntityID& internalId) {
        const auto it = tr.index.internal_edge_to_wire.find(internalId);
        return it != tr.index.internal_edge_to_wire.end() ? it->second : internalId;
    };
    const loop::RegionIdentityVersion table_version = region_identity_version == 3
        ? loop::RegionIdentityVersion::V3
        : loop::RegionIdentityVersion::V2;
    const loop::RegionTable table = loop::buildRegionTable(
        det, map_edge, sk::constants::COINCIDENCE_TOLERANCE, table_version);
    if (diagnostics_out != nullptr) {
        for (const loop::DetectionWarning& warning : table.warnings) {
            diagnostics_out->push_back(degenerate_entity_diagnostic(warning));
        }
    }
    if (!table.success) {
        err = "profile: " + table.errorMessage;
        return std::nullopt;
    }
    if (table.regions.empty()) {
        err = "profile: no closed region detected in sketch";
        return std::nullopt;
    }

    if (exact_identity && region_id.empty()) {
        err = "profile: regionIdentityVersion " +
              std::to_string(*region_identity_version) + " requires regionId";
        return std::nullopt;
    }
    const loop::RegionDefinition* selected = &table.regions.front();
    if (!region_id.empty()) {
        std::optional<loop::RegionTable> exact_alias_table;
        bool exact_legacy_ambiguous = false;
        if (!exact_identity) {
            loop::LoopDetectorConfig exact_config = loop::makeRegionDetectionConfig();
            exact_config.exactAnalyticFragments = true;
            loop::LoopDetector exact_detector;
            exact_detector.setConfig(exact_config);
            const loop::LoopDetectionResult exact_detection = exact_detector.detect(*tr.sketch);
            exact_alias_table = loop::buildRegionTable(
                exact_detection, map_edge, sk::constants::COINCIDENCE_TOLERANCE);
            if (exact_alias_table->success) {
                exact_legacy_ambiguous = std::count_if(
                    exact_alias_table->regions.begin(), exact_alias_table->regions.end(),
                    [&](const loop::RegionDefinition& region) {
                        return region.legacyId == region_id;
                    }) > 1;
            }
        }
        std::vector<const loop::RegionDefinition*> matches;
        std::vector<std::string> available;
        available.reserve(table.regions.size());
        for (const loop::RegionDefinition& region : table.regions) {
            available.push_back(region.id);
            if (region.id == region_id || (!exact_identity && region.legacyId == region_id)) {
                matches.push_back(&region);
            }
        }
        const loop::RegionDefinition* bound =
            (!exact_legacy_ambiguous && matches.size() == 1) ? matches.front() : nullptr;
        // SCHEMA §7.3 "Region anchor": the anchor is a fallback for a STALE id
        // (zero matches) ONLY. An ambiguous id stays a refusal — the anchor must
        // never break a tie the id itself could not, and it never runs before the
        // exact lookup, so a document that still matches replays byte-identically.
        if (bound == nullptr && !exact_legacy_ambiguous && matches.empty() &&
            region_anchor.has_value()) {
            bound = cell_containing_anchor(table, *tr.sketch, *region_anchor);
            if (bound != nullptr && diagnostics_out != nullptr) {
                diagnostics_out->push_back(
                    anchor_rebind_diagnostic(region_id, bound->id, *region_anchor));
            }
        }
        if (bound == nullptr) {
            if (exact_alias_table.has_value() && exact_alias_table->success) {
                    available.clear();
                    for (const loop::RegionDefinition& region : exact_alias_table->regions) {
                        available.push_back(region.id);
                    }
            }
            std::string avail;
            for (std::size_t i = 0; i < available.size(); ++i) {
                if (i) avail += ", ";
                avail += available[i];
            }
            const std::string reason =
                !exact_legacy_ambiguous && matches.empty() ? "matched no selectable region" :
                                                             "is ambiguous";
            err = "profile: regionId '" + region_id + "' " + reason +
                  " (available: [" + avail + "])";
            return std::nullopt;
        }
        selected = bound;
    }

    loop::Face face;
    face.outerLoop = selected->outerLoop;
    face.innerLoops = selected->holes;

    loop::FaceBuilder builder;
    const loop::FaceBuildResult fr = builder.buildFace(face, *tr.sketch);
    if (!fr.success || fr.face.IsNull()) {
        err = "profile: " + (fr.errorMessage.empty() ? std::string("face build failed") : fr.errorMessage);
        return std::nullopt;
    }
    return fr.face;
}

std::optional<TopoDS_Face> build_profile_face(const json& sketch_params,
                                              const std::string& region_id, std::string& err) {
    return build_profile_face(sketch_params, region_id, std::nullopt, err);
}

void attach_profile_diagnostics(OpOutcome& out, std::vector<json>& diagnostics) {
    // A cancelled outcome carries nothing. A FAILED one keeps its advisories: a
    // dropped degenerate entity is often exactly why the profile then has no
    // closed region, and `PlanExecutor::execute_ops` appends the failure
    // diagnostic AFTER the carried advisories (advisory_limit 63 + 1), so the
    // step message still reads the failure text (adversarial review 2026-09-03).
    if (out.status == OpOutcome::Status::Cancelled) {
        diagnostics.clear();
        return;
    }
    if (out.diagnostics.empty()) {
        for (json& diagnostic : diagnostics) out.diagnostics.push_back(std::move(diagnostic));
    } else {
        // Advisories go FIRST so a failure diagnostic already on the outcome stays
        // last — `perStepResults[].message` derives from `diagnostics.back()`.
        std::vector<json> merged;
        merged.reserve(diagnostics.size() + out.diagnostics.size());
        for (json& diagnostic : diagnostics) merged.push_back(std::move(diagnostic));
        for (json& diagnostic : out.diagnostics) merged.push_back(std::move(diagnostic));
        out.diagnostics = std::move(merged);
    }
    diagnostics.clear();
}

bool planar_face_plane_normal(const TopoDS_Face& face, gp_Pln& plane_out, gp_Dir& normal_out) {
    // RegenerationEngine.cpp:201-219 planarFacePlaneAndNormal.
    try {
        if (face.IsNull()) return false;
        BRepAdaptor_Surface surface(face, true);
        if (surface.GetType() != GeomAbs_Plane) return false;
        plane_out = surface.Plane();
        normal_out = plane_out.Axis().Direction();
        if (face.Orientation() == TopAbs_REVERSED) normal_out.Reverse();
        return true;
    } catch (...) {
        return false;
    }
}

namespace {
BOPAlgo_Operation bop_of(app::BooleanMode mode) {
    switch (mode) {
        case app::BooleanMode::Add: return BOPAlgo_FUSE;
        case app::BooleanMode::Cut: return BOPAlgo_CUT;
        case app::BooleanMode::Intersect: return BOPAlgo_COMMON;
        default: return BOPAlgo_UNKNOWN;
    }
}
}  // namespace

void stage_boolean(BRepAlgoAPI_BooleanOperation& algo, const TopoDS_Shape& arg,
                   const TopoDS_Shape& tool) {
    TopTools_ListOfShape args, tools;
    args.Append(arg);
    tools.Append(tool);
    algo.SetArguments(args);
    algo.SetTools(tools);
    algo.SetNonDestructive(Standard_True);
    algo.SetRunParallel(Standard_False);
}

BooleanResult checked_boolean(const TopoDS_Shape& target, const TopoDS_Shape& tool,
                              app::BooleanMode mode, bool parallel, const json& occt_options,
                              const onecad::CancelToken* cancel,
                              std::shared_ptr<BRepBuilderAPI_MakeShape>& builder_out) {
    BooleanResult out;
    if (target.IsNull() || tool.IsNull()) {
        out.error_code = "OP_FAILED";
        out.error_message = "boolean input is null";
        return out;
    }
    const BOPAlgo_Operation bop = bop_of(mode);
    if (bop == BOPAlgo_UNKNOWN) {
        out.error_code = "OP_FAILED";
        out.error_message = "unsupported boolean mode";
        return out;
    }

    // General boolean via BRepAlgoAPI_BooleanOperation (SetOperation) so we can
    // apply determinism + occtOptions BEFORE Build and keep the builder alive for
    // OCCT history. Semantics match RegenerationEngine.cpp:144-199 (IsDone → fail,
    // invalid → fail), plus cancellation via CancelProgress.
    auto algo = std::make_shared<BRepAlgoAPI_BooleanOperation>();
    TopTools_ListOfShape args, tools;
    args.Append(target);
    tools.Append(tool);
    algo->SetArguments(args);
    algo->SetTools(tools);
    algo->SetOperation(bop);
    // WP-E: NEVER let the BOP raise tolerances on the ARGUMENTS in place. The
    // scratch, the preview job and the live head share TShapes, so a failed,
    // cancelled or merely previewed boolean would otherwise inflate committed
    // geometry invisibly to rollback.
    algo->SetNonDestructive(Standard_True);
    // Determinism: single-threaded in determinism mode (Invariant 5). §7.3
    // occtOptions apply to both modes.
    algo->SetRunParallel(parallel ? Standard_True : Standard_False);
    if (occt_options.is_object()) {
        if (occt_options.contains("fuzzyValue") && occt_options["fuzzyValue"].is_number()) {
            const double fuzz = occt_options["fuzzyValue"].get<double>();
            if (fuzz > 0.0) algo->SetFuzzyValue(fuzz);
        }
        if (occt_options.contains("useOBB") && occt_options["useOBB"].is_boolean()) {
            algo->SetUseOBB(occt_options["useOBB"].get<bool>() ? Standard_True : Standard_False);
        }
    }

    try {
        Message_ProgressRange range;
        Handle(CancelProgress) pi;
        if (cancel) {
            pi = new CancelProgress(*cancel);
            range = pi->Start();
        }
        algo->Build(range);

        if (cancel && cancel->cancelled()) {
            out.error_code = "CANCELLED";
            out.error_message = "boolean cancelled";
            return out;
        }
        if (!algo->IsDone() || algo->HasErrors()) {
            out.error_code = "OP_FAILED";
            out.error_message = "boolean failed";
            return out;
        }
        const TopoDS_Shape result = algo->Shape();
        if (result.IsNull()) {
            out.error_code = "GEOMETRY_INVALID";
            out.error_message = "boolean produced null shape";
            return out;
        }
        BRepCheck_Analyzer analyzer(result);
        if (!analyzer.IsValid()) {
            out.error_code = "GEOMETRY_INVALID";
            out.error_message = "boolean produced invalid shape";
            return out;
        }
        out.shape = result;
        builder_out = algo;  // keep alive for history (upcast to MakeShape)
        return out;
    } catch (const Standard_Failure& f) {
        if (cancel && cancel->cancelled()) {
            out.error_code = "CANCELLED";
            out.error_message = "boolean cancelled";
        } else {
            out.error_code = "OP_FAILED";
            out.error_message = std::string("boolean raised: ") +
                                (f.GetMessageString() ? f.GetMessageString() : "OCCT failure");
        }
        return out;
    } catch (...) {
        out.error_code = "OP_FAILED";
        out.error_message = "boolean raised an unknown exception";
        return out;
    }
}

std::vector<RankedSolid> ranked_solids(const TopoDS_Shape& shape) {
    std::vector<RankedSolid> ranked;
    if (shape.IsNull()) return ranked;
    // Quantized geometric sort key (1e-6, matching the ElementMap quantization) so a
    // symmetric bisection (equal volumes) breaks the tie on centroid deterministically.
    //
    // VF-B6: the key is computed ONCE per solid here and RETAINED (it used to live
    // inside the comparator and was discarded), so the ordinal a child is minted at
    // can be published as tripwire evidence. `std::array` compares lexicographically
    // exactly like the `std::tuple` this replaced, and `stable_sort` + the `* 1e6`
    // quantization are unchanged — the resulting ordinal assignment is byte-identical
    // to the pre-VF-B6 order.
    auto q = [](double v) { return static_cast<std::int64_t>(std::llround(v * 1e6)); };
    for (TopExp_Explorer exp(shape, TopAbs_SOLID); exp.More(); exp.Next()) {
        const TopoDS_Shape& s = exp.Current();
        GProp_GProps props;
        BRepGProp::VolumeProperties(s, props);
        const gp_Pnt c = props.CentreOfMass();
        TopTools_IndexedMapOfShape faces;
        TopExp::MapShapes(s, TopAbs_FACE, faces);
        ranked.push_back(RankedSolid{
            s, session::RankKey{q(props.Mass()), q(c.X()), q(c.Y()), q(c.Z()),
                                static_cast<std::int64_t>(faces.Extent())}});
    }
    std::stable_sort(ranked.begin(), ranked.end(),
                     [](const RankedSolid& a, const RankedSolid& b) { return a.key < b.key; });
    return ranked;
}

namespace {
int solid_count_of(const TopoDS_Shape& shape) {
    if (shape.IsNull()) return 0;
    TopTools_IndexedMapOfShape solids;
    TopExp::MapShapes(shape, TopAbs_SOLID, solids);
    return solids.Extent();
}
const char* boolean_mode_name(app::BooleanMode mode) {
    switch (mode) {
        case app::BooleanMode::Add: return "Add";
        case app::BooleanMode::Cut: return "Cut";
        case app::BooleanMode::Intersect: return "Intersect";
        default: return "NewBody";
    }
}
}  // namespace

std::optional<OpOutcome> boolean_result_policy(app::BooleanMode mode, const TopoDS_Shape& target,
                                                const TopoDS_Shape& result, const char* op_name,
                                                const std::string& target_id,
                                                const char* add_disjoint_code,
                                                const char* empty_code, json evidence_extra) {
    const int target_solids = solid_count_of(target);
    const int result_solids = solid_count_of(result);
    json evidence = std::move(evidence_extra);
    evidence["operation"] = boolean_mode_name(mode);
    evidence["targetBodyId"] = target_id;
    evidence["targetSolids"] = target_solids;
    evidence["solidCount"] = result_solids;
    const auto refuse = [&](const char* code, const std::string& message) {
        OpOutcome failure = OpOutcome::fail("OP_FAILED", message);
        failure.diagnostics.push_back({{"severity", "error"},
                                       {"code", code},
                                       {"message", message},
                                       {"stage", "publish"},
                                       {"evidence", json{{"boolean", evidence}}}});
        return failure;
    };
    if (mode == app::BooleanMode::Add) {
        if (result_solids > target_solids) {
            return refuse(add_disjoint_code,
                          std::string(op_name) + " Add: the tool does not touch the target body");
        }
        return std::nullopt;
    }
    if (result_solids == 0) {
        return refuse(empty_code, std::string(op_name) + " " + boolean_mode_name(mode) +
                                      ": the result would be empty");
    }
    return std::nullopt;
}

std::vector<TopoDS_Shape> ordered_solids(const TopoDS_Shape& shape) {
    std::vector<TopoDS_Shape> solids;
    for (const RankedSolid& r : ranked_solids(shape)) solids.push_back(r.shape);
    return solids;
}

BooleanPublishResult publish_boolean_result(OpContext& ctx, const std::string& op_id,
                                            const std::string& target_id,
                                            const TopoDS_Shape& result,
                                            BRepBuilderAPI_MakeShape* builder,
                                            OpOutcome& out) {
    const std::vector<RankedSolid> solids = ranked_solids(result);
    if (solids.empty()) return BooleanPublishResult::Empty;
    if (solids.size() <= 1) {
        // Single body: modify the target in place (BodyId PRESERVED — corpus invariant).
        ctx.bodies.create(target_id, op_id, result);
        if (builder) {
            ctx.partition.apply_history(target_id, result, *builder, out.delta, &out.needs_repair);
        }
        // The rank key rides even the single-solid case (VF-B6): it costs the GProps
        // pass already done above, and it lets Rust anchor the op's stamp BEFORE a
        // later edit turns the same op into a split.
        std::optional<session::RankKey> key;
        if (solids.size() == 1) key = solids[0].key;
        out.body_events.push_back({"modified", target_id, key});
        out.body_ids.push_back(target_id);
        return BooleanPublishResult::Published;
    }
    // Split: the target is REPLACED by k deterministic children `body_<opId>:<k>`
    // (SCHEMA §2, D1). Emit a Deleted for the parent + a Created per child. The
    // parent's referenced-element partition entries are dropped (see the header).
    ctx.partition.remove_body(target_id, out.delta);
    ctx.bodies.erase(target_id);
    out.body_events.push_back({"deleted", target_id});
    for (std::size_t k = 0; k < solids.size(); ++k) {
        const std::string child_id = "body_" + op_id + ":" + std::to_string(k);
        ctx.bodies.create(child_id, op_id, solids[k].shape);
        // VF-B6: `k` IS this solid's geometric rank, so the key it was ranked by is
        // the evidence Rust needs to notice the rank flipping under a parametric edit.
        out.body_events.push_back({"created", child_id, solids[k].key});
        out.body_ids.push_back(child_id);
    }
    return BooleanPublishResult::Published;
}

}  // namespace onecad::ops
