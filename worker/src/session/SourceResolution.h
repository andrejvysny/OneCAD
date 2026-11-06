#pragma once

#include <algorithm>
#include <cctype>
#include <set>
#include <string>
#include <vector>

#include "elementmap/Ladder.h"
#include "io/SourceIdentityGuard.h"
#include "session/ScratchJob.h"

namespace onecad::session {

inline nlohmann::json source_resolution_failure(const nlohmann::json& ref, const std::string& op_id) {
    elementmap::LadderResolution repair;
    repair.ref_id = ref.contains("refId") && ref["refId"].is_string()
                        ? ref["refId"].get<std::string>() : op_id + ".source";
    if (ref.contains("primary") && ref["primary"].is_object()) {
        const auto& primary = ref["primary"];
        if (primary.contains("elementId") && primary["elementId"].is_string())
            repair.element_id = primary["elementId"].get<std::string>();
    }
    repair.reason = "no-candidates";
    repair.ui_label = "import source identity unavailable or inconsistent";
    return repair.to_needs_repair_json();
}

inline bool source_ref_bind(const nlohmann::json& ref, ScratchJob& job,
                            elementmap::ElementMapPartition& partition, SourceLedger& source,
                            elementmap::ElementMapDelta& delta) {
    if (!ref.contains("primary") || !ref["primary"].is_object()) return false;
    const auto& primary = ref["primary"];
    if (!primary.contains("bodyId") || !primary["bodyId"].is_string() ||
        !primary.contains("elementId") || !primary["elementId"].is_string() ||
        !primary.contains("kind") || primary["kind"] != "face") return false;
    const std::string body_id = primary["bodyId"].get<std::string>();
    const std::string element_id = primary["elementId"].get<std::string>();
    if (body_id.empty() || !element_id.starts_with("el_") || element_id.size() == 3 ||
        !std::all_of(element_id.begin() + 3, element_id.end(), [](unsigned char c) {
            return std::isalnum(c) || c == '_' || c == '-';
        })) return false;
    const auto* row = source.resolve(body_id, ref["importSourceIdentity"]);
    const BodyRecord* body = job.bodies.get(body_id);
    if (!row || !body) return false;
    const auto birth = row->birth;
    const TopoDS_Shape face = row->live_face;
    if (const auto* existing = partition.find(element_id)) {
        if (existing->body_id != body_id || existing->kind != elementmap::km::ElementKind::Face ||
            !existing->shape.IsEqual(face)) return false;
    }
    if (partition.has_competing_shape(element_id, face)) return false;
    if (!source.bind(element_id, birth)) return false;
    if (!partition.contains(element_id)) {
        const auto anchor = ref.contains("anchor") ? ref["anchor"] : nlohmann::json::object();
        delta.added.push_back(partition.mint(body_id, element_id, elementmap::km::ElementKind::Face,
                                             face, body->geom, anchor));
    }
    return true;
}

inline bool resolve_source_refs(ScratchJob& job, const nlohmann::json& op, const std::string& op_id,
                                elementmap::ElementMapDelta& delta,
                                std::vector<nlohmann::json>& repairs) {
    if (job.topology_owners.source_ledger().bindings().empty() && !io::contains_source_identity(op))
        return true;
    auto partition = job.partition;
    auto source = job.topology_owners.source_ledger();
    elementmap::ElementMapDelta added;
    std::set<std::string> certified_ids;
    std::vector<const nlohmann::json*> pending{&op};
    while (!pending.empty()) {
        const auto* value = pending.back();
        pending.pop_back();
        if (value->is_object()) {
            const bool certified = value->contains("importSourceIdentity");
            if (certified && !source_ref_bind(*value, job, partition, source, added)) {
                repairs.push_back(source_resolution_failure(*value, op_id));
                return false;
            }
            if (certified) certified_ids.insert((*value)["primary"]["elementId"].get<std::string>());
            if (!certified && value->contains("primary") && (*value)["primary"].is_object()) {
                const auto& primary = (*value)["primary"];
                if (primary.contains("elementId") && primary["elementId"].is_string() &&
                    source.bound(primary["elementId"].get<std::string>())) {
                    repairs.push_back(source_resolution_failure(*value, op_id));
                    return false;
                }
            }
        }
        if (value->is_structured()) for (const auto& child : *value) pending.push_back(&child);
    }
    pending.push_back(&op);
    while (!pending.empty()) {
        const auto* value = pending.back();
        pending.pop_back();
        if (value->is_string()) {
            const auto& text = value->get_ref<const std::string&>();
            if (text.starts_with("el_") && source.bound(text) && !certified_ids.contains(text)) {
                repairs.push_back(source_resolution_failure({{"primary", {{"elementId", text}}}}, op_id));
                return false;
            }
        } else if (value->is_structured()) {
            for (const auto& child : *value) pending.push_back(&child);
        }
    }
    job.partition = std::move(partition);
    job.topology_owners.source_ledger() = std::move(source);
    for (auto& entry : added.added) delta.added.push_back(std::move(entry));
    return true;
}

}  // namespace onecad::session
