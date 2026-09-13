#include "session/FeaturePattern.h"

#include <cmath>
#include <charconv>

#include <gp_XYZ.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>

#include "elementmap/ElementMapPartition.h"
#include "elementmap/Ladder.h"
#include "session/ScratchJob.h"
#include "util/Hashing.h"

namespace onecad::session {
namespace em = onecad::elementmap;
namespace {
using nlohmann::json;

bool read_vec3(const json& value, gp_XYZ& out) {
    if (!value.is_array() || value.size() != 3) return false;
    for (const auto& v : value)
        if (!v.is_number() || !std::isfinite(v.get<double>())) return false;
    out.SetCoord(value[0].get<double>(), value[1].get<double>(), value[2].get<double>());
    return true;
}

bool is_live_body_member(const ScratchJob& job, const em::PartitionEntry& entry,
                         const std::string& body_id, const std::string& kind) {
    const BodyRecord* body = job.bodies.get(body_id);
    if (!body || entry.shape.IsNull()) return false;
    const TopAbs_ShapeEnum shape_kind = kind == "edge" ? TopAbs_EDGE : TopAbs_FACE;
    TopTools_IndexedMapOfShape current;
    TopExp::MapShapes(body->geom, shape_kind, current);
    for (int i = 1; i <= current.Extent(); ++i)
        if (entry.shape.IsSame(current(i))) return true;
    return false;
}
}  // namespace

std::string feature_pattern_virtual_id(const std::string& pattern,
                                       const std::string& source, int instance) {
    static constexpr char kDomain[] = "onecad.feature-pattern.v1\0";
    std::string input(kDomain, sizeof(kDomain) - 1);
    input += pattern;
    input.push_back('\0');
    input += source;
    input.push_back('\0');
    input += std::to_string(instance);
    std::string hex = hashing::sha256_hex(input);
    hex[12] = '5';
    const int variant = (std::stoi(hex.substr(16, 1), nullptr, 16) & 3) | 8;
    hex[16] = "0123456789abcdef"[variant];
    return hex.substr(0, 8) + "-" + hex.substr(8, 4) + "-" + hex.substr(12, 4) +
           "-" + hex.substr(16, 4) + "-" + hex.substr(20, 12);
}

std::string feature_pattern_effective_source_hash(const json& source) {
    json normalized = json::object();
    normalized["opType"] = source.value("opType", std::string());
    normalized["params"] = source.value("params", json::object());
    normalized["inputs"] = source.value("inputs", json::array());
    normalized["determinism"] = source.value("determinism", json::object());
    return hashing::sha256_hex(normalized.dump());
}

bool feature_pattern_same_topology_set(const TopoDS_Shape& before,
                                       const TopoDS_Shape& after) {
    const auto same_kind = [&](TopAbs_ShapeEnum kind) {
        TopTools_IndexedMapOfShape old_shapes;
        TopTools_IndexedMapOfShape new_shapes;
        TopExp::MapShapes(before, kind, old_shapes);
        TopExp::MapShapes(after, kind, new_shapes);
        if (old_shapes.Extent() != new_shapes.Extent()) return false;
        for (int i = 1; i <= old_shapes.Extent(); ++i) {
            bool found = false;
            for (int j = 1; j <= new_shapes.Extent(); ++j)
                if (old_shapes(i).IsSame(new_shapes(j))) {
                    found = true;
                    break;
                }
            if (!found) return false;
        }
        return true;
    };
    return same_kind(TopAbs_FACE) && same_kind(TopAbs_EDGE) &&
        same_kind(TopAbs_VERTEX);
}

void feature_pattern_refresh_producer_topology(
    const ScratchJob& job, FeaturePatternProducerTopology& ownership,
    const std::string& pattern_id, int instance,
    const std::string& body_id, const TopoDS_Shape& after) {
    TopTools_IndexedMapOfShape current;
    TopExp::MapShapes(after, current);
    // Walk the LIVE body and consume the EFFECTIVE claim per IsSame class, never a
    // raw ledger row: a shape carrying two disagreeing rows resolves Ambiguous and
    // must never be exported as this producer's output. Iterating the body is also
    // what keeps the set live — a shape the last source consumed is not walked.
    // The index is built ONCE per call; a per-shape `lookup` would rescan every
    // ledger row and make this quadratic in body size.
    const TopologyOwnerLedger::EffectiveClaims claims =
        job.topology_owners.effective_claims(body_id);
    for (auto& [source_id, shapes] : ownership) {
        shapes.clear();
        const std::string virtual_id =
            feature_pattern_virtual_id(pattern_id, source_id, instance);
        for (int i = 1; i <= current.Extent(); ++i) {
            const int index = claims.FindIndex(current(i));
            if (index == 0) continue;
            const OriginClaim& claim = claims.FindFromIndex(index);
            if (claim.state == OriginState::Known && claim.producer_record_id == virtual_id)
                shapes.push_back(current(i));
        }
    }
}

bool feature_pattern_materialize_current_source_inputs(
    const ScratchJob& job, json& source, std::string& error,
    std::size_t& failed_input) {
    if (!source.contains("inputs") || !source["inputs"].is_array()) return true;
    std::size_t required = 0;
    std::size_t input_index = 0;
    for (const json& input : source["inputs"]) {
        const std::string kind = input.value("primary", json::object())
                                     .value("kind", std::string());
        const json primary = input.value("primary", json::object());
        if ((kind == "edge" || kind == "face" || kind == "vertex") &&
            !primary.value("bodyId", std::string()).empty() &&
            !primary.value("elementId", std::string()).empty()) {
            if (required == 0) failed_input = input_index;
            ++required;
        }
        ++input_index;
    }
    if (required == 0) return true;
    const std::string source_id = source.value("sourceRecordId", std::string());
    const auto ledger = job.resolved_input_evidence.find(source_id);
    if (ledger == job.resolved_input_evidence.end()) {
        error = "missing resolved source evidence";
        return false;
    }
    if (ledger->second.effective_hash != feature_pattern_effective_source_hash(source)) {
        error = "resolved source evidence hash mismatch";
        return false;
    }
    if (ledger->second.inputs.size() != required) {
        error = "resolved source evidence is incomplete";
        return false;
    }
    for (const ResolvedInputEvidence& evidence : ledger->second.inputs) {
        failed_input = evidence.input_index;
        if (evidence.input_index >= source["inputs"].size()) {
            error = "resolved source evidence input is out of range";
            return false;
        }
        json& input = source["inputs"][evidence.input_index];
        const json primary = input.value("primary", json::object());
        if (primary.value("elementId", std::string()) != evidence.element_id ||
            primary.value("bodyId", std::string()) != evidence.body_id ||
            primary.value("kind", std::string()) != evidence.kind) {
            error = "resolved source evidence ownership mismatch";
            return false;
        }
        input["intent"]["descriptor"] = evidence.descriptor;
        input["anchor"] = evidence.anchor;
    }
    return true;
}

bool feature_pattern_retained_host_support(
    const ScratchJob& job, const json& source, const json& ref,
    const std::set<std::string>& selected_ids, const std::string& designated_host) {
    const json primary = ref.value("primary", json::object());
    if (primary.value("kind", std::string()) != "face" ||
        primary.value("bodyId", std::string()) != designated_host) return false;
    const std::string type = source.value("opType", std::string());
    const std::string element = primary.value("elementId", std::string());
    bool semantic_slot = type == "Hole";
    if (type == "Chamfer") {
        semantic_slot = false;
        for (const json& pair : source.value("params", json::object())
                                    .value("referenceFaces", json::array()))
            if (pair.value("faceId", std::string()) == element) semantic_slot = true;
    }
    if (!semantic_slot) return false;
    const auto op = job.resolved_input_evidence.find(
        source.value("sourceRecordId", std::string()));
    if (op == job.resolved_input_evidence.end()) return false;
    return std::any_of(op->second.inputs.begin(), op->second.inputs.end(),
        [&](const ResolvedInputEvidence& evidence) {
            return evidence.element_id == element && evidence.body_id == designated_host &&
                evidence.kind == "face" && evidence.origin_state == OriginState::Known &&
                !selected_ids.contains(evidence.producer_record_id);
        });
}

std::optional<std::size_t> feature_pattern_repair_input_index(
    const json& repair, const json& nested) {
    const std::string prefix = nested.value("opId", std::string()) + ".input";
    const std::string ref_id = repair.value("refId", std::string());
    if (!ref_id.starts_with(prefix)) return std::nullopt;
    std::size_t index = 0;
    const char* first = ref_id.data() + prefix.size();
    const char* last = ref_id.data() + ref_id.size();
    const auto parsed = std::from_chars(first, last, index);
    const json inputs = nested.value("inputs", json::array());
    if (parsed.ec != std::errc{} || parsed.ptr != last || index >= inputs.size())
        return std::nullopt;
    return index;
}

bool feature_pattern_modifier_inputs_supported(
    const ScratchJob& job, const json& source, const json& source_ops,
    const json& layout, std::string& refusal) {
    refusal = "supports only straight-edge Fillet/Chamfer inputs";
    // A descriptor centre is a BOUNDING-BOX centre, and `transform_descriptor`
    // rotates it. A bbox centre is only rotation-covariant for a shape whose bbox
    // is, and a quarter-circle blend arc's is not: at 45 degrees an r=0.5 arc's
    // centre moves 0.073 mm, seventy thousand times the 1e-6 match tolerance. The
    // intended arc is then rejected while a perpendicular twin of equal length and
    // size can still match uniquely — a silent mis-bind. A Hole-produced circle is
    // a full circle whose bbox centre IS its centre, so it keeps the rotational
    // branch under any layout (Astra break F3).
    const bool linear_layout = layout.value("kind", std::string()) == "Linear";
    std::size_t index = 0;
    for (const json& input : source.value("inputs", json::array())) {
        const json primary = input.value("primary", json::object());
        if (primary.value("kind", std::string()) != "edge") { ++index; continue; }
        const json descriptor = input.value("intent", json::object())
                                    .value("descriptor", json::object());
        const auto curve = em::ElementMapPartition::descriptor_from_json(descriptor).curveType;
        if (curve != GeomAbs_Line) {
            bool supported_circle = false;
            bool rotated_blend_arc = false;
            const auto evidence = job.resolved_input_evidence.find(
                source.value("sourceRecordId", std::string()));
            if (curve == GeomAbs_Circle && evidence != job.resolved_input_evidence.end())
                for (const ResolvedInputEvidence& item : evidence->second.inputs)
                    if (item.input_index == index)
                        for (const json& producer : source_ops) {
                            if (producer.value("sourceRecordId", std::string()) !=
                                item.producer_record_id) continue;
                            const std::string type = producer.value("opType", std::string());
                            if (type == "Hole") supported_circle = true;
                            if (type == "Fillet") {
                                rotated_blend_arc = !linear_layout;
                                supported_circle = supported_circle || linear_layout;
                            }
                        }
            if (!supported_circle) {
                if (rotated_blend_arc)
                    refusal = "patterns a Fillet-produced circular edge only under a Linear "
                              "layout: a rotated bounding-box descriptor centre cannot match "
                              "the arc";
                return false;
            }
        }
        ++index;
    }
    return true;
}

int match_feature_pattern_produced_ref(
    const TopoDS_Shape& body, const json& input,
    const std::vector<TopoDS_Shape>& produced, TopoDS_Shape& match) {
    gp_XYZ target_center;
    const json descriptor_json =
        input.value("intent", json::object()).value("descriptor", json::object());
    if (!read_vec3(descriptor_json.value("center", json()), target_center)) return 0;
    const auto target_desc = em::ElementMapPartition::descriptor_from_json(descriptor_json);
    const std::string kind = input.value("primary", json::object()).value("kind", std::string());
    const TopAbs_ShapeEnum shape_kind = kind == "edge" ? TopAbs_EDGE
                                          : kind == "face" ? TopAbs_FACE : TopAbs_SHAPE;
    int matches = 0;
    for (const TopoDS_Shape& shape : produced) {
        if (shape.ShapeType() != shape_kind) continue;
        const auto actual = em::ElementMapPartition::describe(shape, body);
        const bool straight_edge = kind == "edge" &&
            actual.curveType == GeomAbs_Line && target_desc.curveType == GeomAbs_Line;
        if (straight_edge) {
            const bool same_line = (actual.center.XYZ() - target_center).Modulus() <= 1e-6 &&
                std::abs(actual.magnitude - target_desc.magnitude) <= 1e-6 &&
                actual.hasTangent && target_desc.hasTangent &&
                std::abs(actual.tangent.Dot(target_desc.tangent)) >= 1.0 - 1e-9;
            if (same_line) { match = shape; ++matches; }
            continue;
        }
        const bool common = actual.shapeType == target_desc.shapeType &&
            (actual.center.XYZ() - target_center).Modulus() <= 1e-6 &&
            std::abs(actual.size - target_desc.size) <= 1e-6 &&
            std::abs(actual.magnitude - target_desc.magnitude) <= 1e-6;
        const bool rotational_edge = kind == "edge" &&
            (actual.curveType == GeomAbs_Circle || actual.curveType == GeomAbs_Ellipse);
        const bool rotational_face = kind == "face" &&
            (actual.surfaceType == GeomAbs_Cylinder || actual.surfaceType == GeomAbs_Cone ||
             actual.surfaceType == GeomAbs_Sphere || actual.surfaceType == GeomAbs_Torus);
        const bool geometry = kind == "edge"
            ? actual.curveType == target_desc.curveType &&
                  (rotational_edge || !actual.hasTangent || !target_desc.hasTangent ||
                   std::abs(actual.tangent.Dot(target_desc.tangent)) >= 1.0 - 1e-9)
            : actual.surfaceType == target_desc.surfaceType &&
                  (rotational_face || !actual.hasNormal || !target_desc.hasNormal ||
                   std::abs(actual.normal.Dot(target_desc.normal)) >= 1.0 - 1e-9);
        const bool sidedness = rotational_edge || rotational_face ||
            (actual.hasOutward == target_desc.hasOutward &&
             (!actual.hasOutward || actual.outward.Dot(target_desc.outward) >= 1.0 - 1e-9));
        // Only Hole -> Chamfer uses the rotational branch. Its circles/cylinders
        // retain bbox and measure under supported in-plane transforms; this is not
        // a general rigid-shape identity predicate.
        if (common && geometry && sidedness) { match = shape; ++matches; }
    }
    if (matches != 1) match.Nullify();
    return matches;
}

std::vector<json> feature_pattern_unresolved_origin_repairs(
    const ScratchJob& job, const json& nested, const std::string& source_id, int instance) {
    std::vector<json> repairs;
    const auto evidence_op = job.resolved_input_evidence.find(source_id);
    if (!nested.contains("inputs") || !nested["inputs"].is_array()) return repairs;
    for (std::size_t i = 0; i < nested["inputs"].size(); ++i) {
        const json primary = nested["inputs"][i].value("primary", json::object());
        const std::string id = primary.value("elementId", std::string());
        const std::string kind = primary.value("kind", std::string());
        if (id.empty() || (kind != "edge" && kind != "face")) continue;
        OriginState origin = OriginState::Unknown;
        if (evidence_op != job.resolved_input_evidence.end())
            for (const ResolvedInputEvidence& candidate : evidence_op->second.inputs)
                if (candidate.input_index == i) origin = candidate.origin_state;
        if (origin == OriginState::Known) continue;
        repairs.push_back({{"refId", nested.value("opId", std::string()) + ".input" +
                                         std::to_string(i)},
                           {"elementId", id}, {"ladderFailed", "descriptor"},
                           {"reason", origin == OriginState::Ambiguous ? "ambiguous-origin"
                                                                       : "unknown-origin"},
                           {"scoringVersion", em::kResolverVersion}, {"instance", instance},
                           {"sourceRecordId", source_id}, {"inputIndex", i},
                           {"candidates", json::array()},
                           {"uiLabel", "FeaturePattern instance " + std::to_string(instance) +
                                " source " + source_id + " input " + std::to_string(i) +
                                " requires repair"}});
    }
    return repairs;
}

std::vector<json> feature_pattern_bind_instance_output_refs(
    ScratchJob& job, const json& nested, const json& frozen_source,
    const FeaturePatternProducerTopology& produced,
    em::ElementMapDelta& delta, int instance, const std::string& source_id,
    const std::set<std::string>& selected_ids, const std::string& designated_host) {
    std::vector<json> repairs;
    const auto evidence_op = job.resolved_input_evidence.find(source_id);
    if (!nested.contains("inputs") || !nested["inputs"].is_array()) return repairs;
    for (std::size_t i = 0; i < nested["inputs"].size(); ++i) {
        const json& input = nested["inputs"][i];
        const json primary = input.value("primary", json::object());
        const std::string id = primary.value("elementId", std::string());
        const std::string body_id = primary.value("bodyId", std::string());
        const std::string kind = primary.value("kind", std::string());
        if (id.empty() || (kind != "edge" && kind != "face")) continue;
        const ResolvedInputEvidence* evidence = nullptr;
        if (evidence_op != job.resolved_input_evidence.end()) {
            for (const ResolvedInputEvidence& candidate : evidence_op->second.inputs)
                if (candidate.input_index == i) {
                    evidence = &candidate;
                    break;
                }
        }
        if (const auto* existing = job.partition.find(id)) {
            const auto expected = kind == "edge" ? em::km::ElementKind::Edge
                                                    : em::km::ElementKind::Face;
            bool producer_owned = false;
            if (evidence && evidence->origin_state == OriginState::Known) {
                const auto expected_producer = produced.find(evidence->producer_record_id);
                if (expected_producer != produced.end()) {
                    producer_owned = std::any_of(
                        expected_producer->second.begin(), expected_producer->second.end(),
                        [&](const TopoDS_Shape& shape) { return shape.IsSame(existing->shape); });
                } else {
                    const json frozen_inputs =
                        frozen_source.value("inputs", json::array());
                    const bool retained_slot = i < frozen_inputs.size() &&
                        feature_pattern_retained_host_support(
                            job, frozen_source, frozen_inputs[i], selected_ids,
                            designated_host);
                    // Every clause above reads FROZEN evidence. The live ledger is
                    // the authority on who owns the face NOW: one whose effective
                    // claim has since gone Unknown or Ambiguous is a stale binding,
                    // and admitting it would let the instance consume topology
                    // nobody can attribute (Astra break F2).
                    const OriginClaim live =
                        job.topology_owners.lookup(body_id, existing->shape);
                    producer_owned = retained_slot &&
                        !selected_ids.contains(evidence->producer_record_id) &&
                        body_id == designated_host && id == evidence->element_id &&
                        live.state == OriginState::Known &&
                        live.producer_record_id == evidence->producer_record_id;
                }
            }
            if (existing->body_id == body_id && existing->kind == expected &&
                producer_owned && is_live_body_member(job, *existing, body_id, kind))
                continue;
            repairs.push_back({{"refId", nested.value("opId", std::string()) + ".input" +
                                            std::to_string(i)},
                               {"elementId", id}, {"ladderFailed", "descriptor"},  // closed enum (SCHEMA §9)
                               {"reason", "ownership-mismatch"},
                               {"scoringVersion", em::kResolverVersion}, {"instance", instance},
                               {"sourceRecordId", source_id}, {"inputIndex", i},
                               {"candidates", json::array()},
                               {"uiLabel", "FeaturePattern virtual reference ownership mismatch"}});
            continue;
        }
        const BodyRecord* body = job.bodies.get(body_id);
        TopoDS_Shape match;
        bool supported = true;
        if (nested.value("opType", std::string()) == "Fillet") {
            const json descriptor = input.value("intent", json::object())
                                        .value("descriptor", json::object());
            supported = em::ElementMapPartition::descriptor_from_json(descriptor).curveType ==
                        GeomAbs_Line;
        }
        int matches = 0;
        const auto owned = evidence && evidence->origin_state == OriginState::Known
            ? produced.find(evidence->producer_record_id) : produced.end();
        if (body && supported && owned != produced.end())
            matches = match_feature_pattern_produced_ref(
                body->geom, input, owned->second, match);
        if (matches == 1) {
            const auto element_kind = kind == "edge" ? em::km::ElementKind::Edge
                                                       : em::km::ElementKind::Face;
            delta.added.push_back(job.partition.mint(
                body_id, id, element_kind, match, body->geom,
                input.value("anchor", json::object())));
            continue;
        }
        repairs.push_back({{"refId", nested.value("opId", std::string()) + ".input" +
                                        std::to_string(i)},
                           {"elementId", id}, {"ladderFailed", "descriptor"},
                           {"reason", !evidence || evidence->origin_state == OriginState::Unknown
                                ? "unknown-origin"
                                : evidence->origin_state == OriginState::Ambiguous
                                    ? "ambiguous-origin"
                                    : matches == 0 ? "no-candidates" : "ambiguous"},
                           {"scoringVersion", em::kResolverVersion}, {"instance", instance},
                           {"sourceRecordId", source_id}, {"inputIndex", i},
                           {"candidates", json::array()},
                           {"uiLabel", "FeaturePattern instance " + std::to_string(instance) +
                                " source " + source_id + " input " + std::to_string(i) +
                                " requires repair"}});
    }
    return repairs;
}
}  // namespace onecad::session
