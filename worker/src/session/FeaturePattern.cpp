#include "session/FeaturePattern.h"
#include "session/FeaturePatternValidation.h"

#include <algorithm>
#include <cmath>
#include <set>
#include <stdexcept>
#include <string>
#include <unordered_map>

#include <gp_Ax1.hxx>
#include <gp_Dir.hxx>
#include <gp_Pnt.hxx>
#include <gp_Trsf.hxx>
#include <gp_Vec.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>

#include "session/PlanExecutor.h"
#include "session/ScratchJob.h"
#include "sketch/Sketch.h"
#include "elementmap/Ladder.h"

namespace onecad::session {
namespace em = onecad::elementmap;
namespace sk = onecad::core::sketch;
namespace {
using nlohmann::json;
bool read_vec3(const json& value, gp_XYZ& out) {
    if (!value.is_array() || value.size() != 3) return false;
    for (const auto& v : value)
        if (!v.is_number() || !std::isfinite(v.get<double>())) return false;
    out.SetCoord(value[0].get<double>(), value[1].get<double>(), value[2].get<double>());
    return true;
}
bool read_scalar(const json& value, double& out) {
    const json* number = &value;
    if (value.is_object() && value.contains("value")) number = &value["value"];
    if (!number->is_number()) return false;
    out = number->get<double>();
    return std::isfinite(out);
}
bool instance_transform(const json& layout, int instance, int count, gp_Trsf& out,
                        std::string& error) {
    const std::string kind = layout.value("kind", std::string());
    gp_XYZ a;
    if (kind == "Linear") {
        if (!read_vec3(layout.value("direction", json()), a) || a.Modulus() < 1e-10) {
            error = "FeaturePattern Linear direction is missing or zero"; return false;
        }
        double spacing = 0.0;
        if (!read_scalar(layout.value("spacing", json()), spacing) || std::abs(spacing) < 1e-9) {
            error = "FeaturePattern Linear spacing must be finite and non-zero"; return false;
        }
        a.Normalize();
        out.SetTranslation(gp_Vec(a) * (spacing * instance));
        return true;
    }
    if (kind == "Circular") {
        gp_XYZ origin;
        if (!read_vec3(layout.value("axisOrigin", json()), origin) ||
            !read_vec3(layout.value("axisDirection", json()), a) || a.Modulus() < 1e-10) {
            error = "FeaturePattern Circular axis is missing or zero"; return false;
        }
        double angle = 0.0;
        if (!read_scalar(layout.value("angleDeg", json()), angle) || angle == 0.0 ||
            std::abs(angle) > 360.0) {
            error = "FeaturePattern Circular angleDeg must be finite, non-zero, and <= 360";
            return false;
        }
        out.SetRotation(gp_Ax1(gp_Pnt(origin), gp_Dir(a)),
                        angle * instance / count * M_PI / 180.0);
        return true;
    }
    error = "FeaturePattern layout kind must be Linear or Circular";
    return false;
}
void transform_point(json& point, const gp_Trsf& trsf) {
    gp_XYZ xyz;
    if (!read_vec3(point, xyz)) return;
    gp_Pnt p(xyz); p.Transform(trsf);
    point = json::array({p.X(), p.Y(), p.Z()});
}
void transform_vector(json& value, const gp_Trsf& trsf) {
    gp_XYZ xyz;
    if (!read_vec3(value, xyz)) return;
    gp_Vec vector(xyz); vector.Transform(trsf);
    value = json::array({vector.X(), vector.Y(), vector.Z()});
}
void transform_descriptor(json& descriptor, const gp_Trsf& trsf) {
    if (!descriptor.is_object()) return;
    for (const char* key : {"center", "centroid"})
        if (descriptor.contains(key)) transform_point(descriptor[key], trsf);
    for (const char* key : {"normal", "axis", "direction", "tangent", "outward"})
        if (descriptor.contains(key)) transform_vector(descriptor[key], trsf);
}
void transform_anchor(json& anchor, const gp_Trsf& trsf) {
    if (!anchor.is_object()) return;
    if (anchor.contains("worldPoint")) transform_point(anchor["worldPoint"], trsf);
    if (!anchor.contains("localFrame") || !anchor["localFrame"].is_object()) return;
    json& frame = anchor["localFrame"];
    if (frame.contains("origin")) transform_point(frame["origin"], trsf);
    for (const char* key : {"x", "y", "z"})
        if (frame.contains(key)) transform_vector(frame[key], trsf);
}

void transform_sketch_plane(json& params, const gp_Trsf& trsf) {
    if (!params.contains("plane")) params["plane"] = {{"kind", "XY"}};
    if (!params["plane"].is_object())
        throw std::runtime_error("FeaturePattern Sketch plane must be an object");
    json& plane = params["plane"];
    const std::string kind = plane.value("kind", std::string("XY"));
    const auto vec = [](const sk::Vec3d& value) {
        return json::array({value.x, value.y, value.z});
    };
    if (kind != "custom") {
        if (kind != "XY" && kind != "XZ" && kind != "YZ")
            throw std::runtime_error("FeaturePattern Sketch plane kind is unsupported");
        const sk::SketchPlane basis = kind == "XZ" ? sk::SketchPlane::XZ()
                                      : kind == "YZ" ? sk::SketchPlane::YZ()
                                                     : sk::SketchPlane::XY();
        plane = {{"kind", "custom"}, {"origin", vec(basis.origin)},
                 {"xAxis", vec(basis.xAxis)}, {"yAxis", vec(basis.yAxis)},
                 {"normal", vec(basis.normal)}};
    } else {
        const sk::SketchPlane defaults = sk::SketchPlane::XY();
        json normalized = {{"kind", "custom"}, {"origin", vec(defaults.origin)},
                           {"xAxis", vec(defaults.xAxis)}, {"yAxis", vec(defaults.yAxis)},
                           {"normal", vec(defaults.normal)}};
        for (const char* key : {"origin", "xAxis", "yAxis", "normal"}) {
            if (!plane.contains(key)) continue;
            gp_XYZ checked;
            if (!read_vec3(plane[key], checked))
                throw std::runtime_error(std::string("FeaturePattern Sketch plane ") + key +
                                         " must be a finite vec3");
            normalized[key] = plane[key];
        }
        plane = std::move(normalized);
    }
    if (plane.contains("origin")) transform_point(plane["origin"], trsf);
    for (const char* key : {"xAxis", "yAxis", "normal"})
        if (plane.contains(key)) transform_vector(plane[key], trsf);
}

void rewrite_ref(json& ref, const gp_Trsf& trsf, const std::string& pattern,
                 int instance, bool shared_host_face,
                 const ScratchJob& job,
                 std::unordered_map<std::string, std::string>& ids,
                 std::set<std::string>& generated_ids) {
    if (!ref.is_object()) return;
    if (!shared_host_face && ref.contains("primary") && ref["primary"].is_object()) {
        json& primary = ref["primary"];
        if (primary.contains("elementId") && primary["elementId"].is_string()) {
            const std::string source = primary["elementId"].get<std::string>();
            auto [it, inserted] = ids.emplace(
                source, feature_pattern_virtual_id(pattern, source, instance));
            if (inserted && (it->second == source || job.partition.contains(it->second) ||
                             !generated_ids.insert(it->second).second))
                throw std::runtime_error("FeaturePattern virtual ElementId collision");
            primary["elementId"] = it->second;
        }
    }
    if (!shared_host_face && ref.contains("anchor")) transform_anchor(ref["anchor"], trsf);
    if (!shared_host_face && ref.contains("intent") && ref["intent"].is_object() &&
        ref["intent"].contains("descriptor"))
        transform_descriptor(ref["intent"]["descriptor"], trsf);
}

json instantiate(json source, const gp_Trsf& trsf, const std::string& pattern,
                 int instance, std::size_t source_index,
                 const ScratchJob& job,
                 const std::set<std::string>& selected_ids,
                 const std::string& designated_host,
                 std::unordered_map<std::string, std::string>& ids,
                 std::set<std::string>& generated_ids,
                 std::unordered_map<std::string, std::string>& sketch_ids,
                 std::unordered_map<std::string, std::string>& body_ids) {
    source["opId"] = feature_pattern_virtual_id(
        pattern, source.value("sourceRecordId", std::string()), instance);
    if (source.contains("params") && source["params"].is_object()) {
        json& p = source["params"];
        if (source.value("opType", std::string()) == "Sketch") {
            const std::string old_sketch = p.value("sketchId", std::string());
            const std::string new_sketch = "sk_" + source["opId"].get<std::string>();
            if (!old_sketch.empty()) sketch_ids[old_sketch] = new_sketch;
            p["sketchId"] = new_sketch;
            transform_sketch_plane(p, trsf);
        }
        if (p.contains("sketchId") && p["sketchId"].is_string() &&
            sketch_ids.contains(p["sketchId"].get<std::string>()))
            p["sketchId"] = sketch_ids.at(p["sketchId"].get<std::string>());
        if (p.contains("axis") && p["axis"].is_object() &&
            p["axis"].contains("sketchId") && p["axis"]["sketchId"].is_string() &&
            sketch_ids.contains(p["axis"]["sketchId"].get<std::string>()))
            p["axis"]["sketchId"] =
                sketch_ids.at(p["axis"]["sketchId"].get<std::string>());
        if (p.contains("targetBodyId") && p["targetBodyId"].is_string() &&
            body_ids.contains(p["targetBodyId"].get<std::string>()))
            p["targetBodyId"] = body_ids.at(p["targetBodyId"].get<std::string>());
        if (p.contains("point")) transform_point(p["point"], trsf);
        if (p.contains("face")) rewrite_ref(
            p["face"], trsf, pattern, instance,
            feature_pattern_retained_host_support(
                job, source, p["face"], selected_ids, designated_host),
            job, ids, generated_ids);
    }
    if (source.contains("inputs") && source["inputs"].is_array()) {
        for (json& input : source["inputs"]) {
            if (input.contains("primary") && input["primary"].is_object()) {
                json& primary = input["primary"];
                if (primary.contains("bodyId") && primary["bodyId"].is_string() &&
                    body_ids.contains(primary["bodyId"].get<std::string>()))
                    primary["bodyId"] = body_ids.at(primary["bodyId"].get<std::string>());
                if (primary.contains("elementId") && primary["elementId"].is_string()) {
                    std::string element = primary["elementId"].get<std::string>();
                    for (const auto& [old_sketch, new_sketch] : sketch_ids)
                        if (element.starts_with(old_sketch + "."))
                            primary["elementId"] = new_sketch + element.substr(old_sketch.size());
                }
            }
            rewrite_ref(input, trsf, pattern, instance,
                        feature_pattern_retained_host_support(
                                job, source, input, selected_ids, designated_host),
                        job, ids, generated_ids);
        }
    }
    json& params = source["params"];
    if (params.contains("edgeIds") && params["edgeIds"].is_array())
        for (json& id : params["edgeIds"])
            if (id.is_string() && ids.contains(id.get<std::string>())) id = ids.at(id.get<std::string>());
    if (params.contains("referenceFaces") && params["referenceFaces"].is_array())
        for (json& pair : params["referenceFaces"]) {
            if (pair.contains("edgeId") && pair["edgeId"].is_string() &&
                ids.contains(pair["edgeId"].get<std::string>()))
                pair["edgeId"] = ids.at(pair["edgeId"].get<std::string>());
            if (pair.contains("faceId") && pair["faceId"].is_string() &&
                ids.contains(pair["faceId"].get<std::string>()))
                pair["faceId"] = ids.at(pair["faceId"].get<std::string>());
        }
    return source;
}

}

ops::OpOutcome execute_feature_pattern(ScratchJob& job, const json& op,
                                       const std::string& op_id,
                                       std::string& last_sketch_id,
                                       const onecad::CancelToken& cancel,
                                       ops::ValidationMode validation_mode) {
    json params;
    int count = 0;
    FeaturePatternExecutionClass execution_class = FeaturePatternExecutionClass::HostModifier;
    std::size_t creator_index = 0;
    std::string designated_host;
    try {
        if (auto failure = validate_feature_pattern_sources(
                op, params, count, execution_class, creator_index, designated_host))
            return std::move(*failure);
    } catch (const std::exception& e) {
        return ops::OpOutcome::fail("OP_FAILED",
                                    "malformed FeaturePattern payload: " + std::string(e.what()));
    }

    ops::OpOutcome result = ops::OpOutcome::ok();
    // Both producer-binding refusals (origin gate and virtual-ref bind) report
    // through one shape, so a repairable reference always reads the same way.
    const auto producer_bind_failure = [&result](std::vector<json> repairs,
                                                 const std::string& source_id, int instance) {
        const std::size_t input_index = repairs.front().value("inputIndex", 0U);
        result.needs_repair = std::move(repairs);
        const std::string message = "FeaturePattern instance " + std::to_string(instance) +
            " source " + source_id + " input " + std::to_string(input_index) +
            " producer binding failed";
        result.diagnostics.push_back(
            {{"severity", "error"}, {"code", "FEATURE_PATTERN_PRODUCER_BIND"},
             {"message", message}, {"stage", "reference-resolution"},
             {"evidence", {{"featurePattern", {{"instance", instance},
                 {"sourceRecordId", source_id}, {"inputIndex", input_index}}}}}});
    };
    std::set<std::string> selected_ids;
    for (const json& source : params["sourceOps"])
        selected_ids.insert(source.value("sourceRecordId", std::string()));
    std::string host;
    std::vector<std::string> created_bodies;
    try {
        for (int instance = 1; instance < count; ++instance) {
            std::unordered_map<std::string, std::string> virtual_ids;
            std::set<std::string> generated_ids;
            std::unordered_map<std::string, std::string> sketch_ids;
            std::unordered_map<std::string, std::string> body_ids;
            FeaturePatternProducerTopology producer_topology;
            gp_Trsf trsf; std::string error;
            if (!instance_transform(params.value("layout", json::object()), instance,
                                    count, trsf, error))
                return ops::OpOutcome::fail("OP_FAILED", error);
            std::size_t source_index = 0;
            for (const json& frozen_source : params["sourceOps"]) {
                json source = frozen_source;
                {
                    std::string evidence_error;
                    std::size_t failed_input = 0;
                    if (!feature_pattern_materialize_current_source_inputs(
                            job, source, evidence_error, failed_input)) {
                        const std::string source_id =
                            source.value("sourceRecordId", std::string());
                        std::string element_id;
                        const json inputs = source.value("inputs", json::array());
                        if (failed_input < inputs.size())
                            element_id = inputs[failed_input]
                                             .value("primary", json::object())
                                             .value("elementId", std::string());
                        result.needs_repair.push_back(
                            {{"refId", source_id + ".input" + std::to_string(failed_input)},
                             {"elementId", element_id},
                             {"ladderFailed", "descriptor"},
                             {"reason", "no-candidates"},
                             {"scoringVersion", em::kResolverVersion},
                             {"instance", instance}, {"sourceRecordId", source_id},
                             {"inputIndex", failed_input},
                             {"candidates", json::array()},
                             {"uiLabel", "FeaturePattern instance " +
                                 std::to_string(instance) + " source " + source_id +
                                 " input " + std::to_string(failed_input) +
                                 " requires repair: " + evidence_error}});
                        return result;
                    }
                }
                json nested = instantiate(source, trsf, op_id, instance, source_index,
                                          job, selected_ids, designated_host,
                                          virtual_ids, generated_ids,
                                          sketch_ids, body_ids);
                const std::string source_record_id =
                    source.value("sourceRecordId", std::string());
                // Origin BEFORE capability: an input the ledger cannot attribute is
                // missing evidence the user can repair, and reporting it as
                // `UNSUPPORTED_OP` would hide a repairable reference behind a
                // permanent-sounding refusal. The capability refusal below keeps
                // its meaning — a curve type this adapter cannot pattern even when
                // its producer IS known.
                if (source_index > creator_index) {
                    std::vector<json> origin_repairs =
                        feature_pattern_unresolved_origin_repairs(
                            job, nested, source_record_id, instance);
                    if (!origin_repairs.empty()) {
                        producer_bind_failure(std::move(origin_repairs), source_record_id,
                                              instance);
                        return result;
                    }
                }
                std::string capability_refusal;
                if (execution_class != FeaturePatternExecutionClass::HostModifier &&
                    source_index > creator_index &&
                    source.value("opType", std::string()) != "Hole" &&
                    !feature_pattern_modifier_inputs_supported(
                        job, source, params["sourceOps"],
                        params.value("layout", json::object()), capability_refusal)) {
                    return ops::OpOutcome::fail(
                        "UNSUPPORTED_OP", "FeaturePattern source " + source_record_id +
                        " index " + std::to_string(source_index) + " instance " +
                        std::to_string(instance) + " " + capability_refusal);
                }
                if (source_index > creator_index) {
                    auto repairs = feature_pattern_bind_instance_output_refs(
                        job, nested, frozen_source, producer_topology, result.delta,
                        instance, source_record_id, selected_ids, designated_host);
                    if (!repairs.empty()) {
                        producer_bind_failure(std::move(repairs), source_record_id, instance);
                        return result;
                    }
                }
                const std::string nested_id = nested.value("opId", std::string());
                std::string target_body = nested.value("params", json::object())
                                              .value("targetBodyId", std::string());
                if (target_body.empty() && nested.contains("inputs") && nested["inputs"].is_array()) {
                    for (const json& input : nested["inputs"]) {
                        target_body = input.value("primary", json::object())
                                          .value("bodyId", std::string());
                        if (!target_body.empty()) break;
                    }
                }
                const BodyRecord* before_record = job.bodies.get(target_body);
                const TopoDS_Shape before = before_record ? before_record->geom : TopoDS_Shape();
                const ResolvedInputEvidenceLedger source_evidence =
                    job.resolved_input_evidence;
                CandidateResult candidate = execute_candidate_op(
                    job, nested, nested_id, last_sketch_id, cancel, validation_mode);
                job.resolved_input_evidence = source_evidence;
                if (candidate.status != CandidateResult::Status::Ok) {
                    const std::string source_id =
                        source.value("sourceRecordId", std::string());
                    const std::string context = "FeaturePattern instance " +
                        std::to_string(instance) + " source " + source_id +
                        " index " + std::to_string(source_index);
                    candidate.diagnostics.push_back(
                        {{"severity", "error"}, {"code", "FEATURE_PATTERN_SOURCE_FAILED"},
                         {"message", context + " failed"}, {"stage", "nested-source"},
                         {"evidence", {{"featurePattern", {{"instance", instance},
                             {"sourceRecordId", source_id}, {"sourceIndex", source_index}}}}}});
                    if (candidate.status == CandidateResult::Status::NeedsRepair) {
                        for (json& repair : candidate.needs_repair) {
                            repair["instance"] = instance;
                            repair["sourceRecordId"] = source_id;
                            if (!repair.contains("inputIndex")) {
                                const auto input_index =
                                    feature_pattern_repair_input_index(repair, nested);
                                if (!input_index) {
                                    ops::OpOutcome failure = ops::OpOutcome::fail(
                                        "OP_FAILED", context +
                                            " nested repair lacks input slot: " +
                                            repair.value("refId", std::string()));
                                    failure.diagnostics = std::move(candidate.diagnostics);
                                    return failure;
                                }
                                repair["inputIndex"] = *input_index;
                            }
                        }
                        result.needs_repair = std::move(candidate.needs_repair);
                        result.diagnostics = std::move(candidate.diagnostics);
                        return result;
                    }
                    result.error_code = candidate.error_code.empty() ? "OP_FAILED" : candidate.error_code;
                    result.error_message = candidate.error_message.empty()
                        ? "FeaturePattern nested source failed" : candidate.error_message;
                    result.error_message = context + ": " + result.error_message;
                    result.status = candidate.status == CandidateResult::Status::Cancelled
                        ? ops::OpOutcome::Status::Cancelled : ops::OpOutcome::Status::Failed;
                    result.needs_repair = std::move(candidate.needs_repair);
                    result.diagnostics = std::move(candidate.diagnostics);
                    return result;
                }
                if (execution_class == FeaturePatternExecutionClass::IndependentBody &&
                    source_index == creator_index &&
                    (source.value("opType", std::string()) == "Extrude" ||
                     source.value("opType", std::string()) == "Revolve")) {
                    const std::string internal_body = "body_" + nested_id;
                    body_ids["body_" + source.value("sourceRecordId", std::string())] =
                        internal_body;
                    target_body = internal_body;
                }
                for (const auto& event : candidate.body_events)
                    if (event.kind == "modified") host = event.body_id;
                for (auto& e : candidate.delta.added) result.delta.added.push_back(std::move(e));
                for (auto& e : candidate.delta.relabeled) result.delta.relabeled.push_back(std::move(e));
                for (auto& e : candidate.delta.removed) result.delta.removed.push_back(std::move(e));
                const BodyRecord* after_record = job.bodies.get(target_body);
                if (execution_class == FeaturePatternExecutionClass::SharedHost &&
                    source_index == creator_index &&
                    (target_body != designated_host || after_record == nullptr))
                    return ops::OpOutcome::fail(
                        "OP_FAILED", "FeaturePattern shared-host producer ledger lost its designated body");
                if (after_record && execution_class == FeaturePatternExecutionClass::SharedHost &&
                    feature_pattern_same_topology_set(before, after_record->geom)) {
                    const std::string source_id =
                        source.value("sourceRecordId", std::string());
                    return ops::OpOutcome::fail(
                        "OP_FAILED", "FeaturePattern instance " + std::to_string(instance) +
                            " source " + source_id + " index " +
                            std::to_string(source_index) + " left its host unchanged");
                }
                if (after_record)
                    producer_topology.try_emplace(
                        source.value("sourceRecordId", std::string()));
                if (after_record)
                    feature_pattern_refresh_producer_topology(
                        job, producer_topology, op_id, instance, target_body,
                        after_record->geom);
                ++source_index;
            }
            if (execution_class == FeaturePatternExecutionClass::IndependentBody) {
                const std::string internal_body =
                    body_ids.at("body_" + params["sourceOps"][creator_index]
                        .value("sourceRecordId", std::string()));
                const std::string canonical_body = "body_" + op_id + ":" +
                    std::to_string(instance - 1);
                if (!job.bodies.rename(internal_body, canonical_body, op_id))
                    return ops::OpOutcome::fail("OP_FAILED", "FeaturePattern body identity collision");
                job.partition.rename_body(internal_body, canonical_body);
                job.topology_owners.rename_body(internal_body, canonical_body);
                for (auto& entry : result.delta.added)
                    if (entry.body_id == internal_body) entry.body_id = canonical_body;
                for (auto& entry : result.delta.relabeled)
                    if (entry.body_id == internal_body) entry.body_id = canonical_body;
                created_bodies.push_back(canonical_body);
            }
            if (execution_class == FeaturePatternExecutionClass::SharedHost && host != designated_host)
                return ops::OpOutcome::fail(
                    "OP_FAILED", "FeaturePattern shared-host operation modified an unexpected body");
        }
    } catch (const std::exception& e) {
        return ops::OpOutcome::fail("OP_FAILED", e.what());
    }
    if (!created_bodies.empty()) {
        for (const std::string& body_id : created_bodies) {
            result.body_events.push_back({"created", body_id, {}});
            result.body_ids.push_back(body_id);
        }
        result.topology_history_mode = ops::TopologyHistoryMode::CompositeAdopted;
        return result;
    }
    if (host.empty()) return ops::OpOutcome::fail("OP_FAILED", "FeaturePattern produced no host modification");
    BodyEvent event;
    event.kind = "modified";
    event.body_id = host;
    result.body_events.push_back(std::move(event));
    result.body_ids.push_back(host);
    result.topology_history_mode = ops::TopologyHistoryMode::CompositeAdopted;
    return result;
}
}
