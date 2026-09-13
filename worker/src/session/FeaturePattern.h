#pragma once

#include <cstddef>
#include <map>
#include <set>
#include <string>
#include <vector>

#include <TopoDS_Shape.hxx>

#include "nlohmann/json_fwd.hpp"
#include "ops/OpTypes.h"

namespace onecad { class CancelToken; }

namespace onecad::session {
struct ScratchJob;
using FeaturePatternProducerTopology =
    std::map<std::string, std::vector<TopoDS_Shape>>;

std::string feature_pattern_virtual_id(const std::string& pattern,
                                       const std::string& source, int instance);
std::string feature_pattern_effective_source_hash(const nlohmann::json& source);
bool feature_pattern_materialize_current_source_inputs(
    const ScratchJob& job, nlohmann::json& source, std::string& error,
    std::size_t& failed_input);
bool feature_pattern_retained_host_support(
    const ScratchJob& job, const nlohmann::json& source,
    const nlohmann::json& ref, const std::set<std::string>& selected_ids,
    const std::string& designated_host);
std::optional<std::size_t> feature_pattern_repair_input_index(
    const nlohmann::json& repair, const nlohmann::json& nested);
// `refusal` receives the phrase naming WHY, which the caller prefixes with the
// source/index/instance context. `layout` decides whether a Fillet-produced
// circular edge is patternable at all — see the implementation.
bool feature_pattern_modifier_inputs_supported(
    const ScratchJob& job, const nlohmann::json& source,
    const nlohmann::json& source_ops, const nlohmann::json& layout,
    std::string& refusal);
bool feature_pattern_same_topology_set(
    const TopoDS_Shape& before, const TopoDS_Shape& after);
void feature_pattern_refresh_producer_topology(
    const ScratchJob& job, FeaturePatternProducerTopology& ownership,
    const std::string& pattern_id, int instance,
    const std::string& body_id, const TopoDS_Shape& after);

// The origin gate that runs BEFORE the straight-edge capability refusal
// (docs/design/astra/feature-pattern-producer-ownership.md §8). A body
// sub-element input whose producer the ledger cannot name is missing evidence
// the user can repair, not an unsupported curve type; `UNSUPPORTED_OP` stays for
// a genuinely unsupported curve with a Known producer. Returns one repair per
// Unknown/Ambiguous edge/face input, shaped exactly like the bind repairs below.
std::vector<nlohmann::json> feature_pattern_unresolved_origin_repairs(
    const ScratchJob& job, const nlohmann::json& nested,
    const std::string& source_id, int instance);

// Match a stored source ref only inside one virtual producer's new topology.
// Returns the number of exact geometric matches and assigns `match` only for one.
int match_feature_pattern_produced_ref(
    const TopoDS_Shape& body, const nlohmann::json& input,
    const std::vector<TopoDS_Shape>& produced, TopoDS_Shape& match);
std::vector<nlohmann::json> feature_pattern_bind_instance_output_refs(
    ScratchJob& job, const nlohmann::json& nested,
    const nlohmann::json& frozen_source,
    const FeaturePatternProducerTopology& produced,
    onecad::elementmap::ElementMapDelta& delta, int instance,
    const std::string& source_id, const std::set<std::string>& selected_ids,
    const std::string& designated_host);

// Replays an explicit source-feature chain for every non-seed instance. The
// caller's candidate snapshot is the transaction boundary: any nested failure
// returns failure and PlanExecutor restores bodies, identity and sketches.
ops::OpOutcome execute_feature_pattern(
    ScratchJob& job, const nlohmann::json& op, const std::string& op_id,
    std::string& last_sketch_id, const onecad::CancelToken& cancel,
    ops::ValidationMode validation_mode);
}
