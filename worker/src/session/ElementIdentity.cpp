// ElementIdentity.cpp — see ElementIdentity.h. SCHEMA §7.5.
#include "session/ElementIdentity.h"

#include <cmath>
#include <cstdint>
#include <optional>
#include <string>

#include <TopoDS_Shape.hxx>

#include "elementmap/ElementMapPartition.h"
#include "elementmap/Ladder.h"
#include "util/Log.h"

namespace onecad::session {

using nlohmann::json;
using protocol::Envelope;
namespace em = onecad::elementmap;

namespace {

std::string get_str(const json& o, const char* key, const std::string& dflt = "") {
    if (o.is_object() && o.contains(key) && o[key].is_string()) return o[key].get<std::string>();
    return dflt;
}

// The partition entry (if any) whose body_id==bodyId and topo_key==topo.
const em::PartitionEntry* entry_by_topokey(const em::ElementMapPartition& part,
                                           const std::string& body_id, const std::string& topo) {
    for (const em::PartitionEntry* e : part.entries_for_body(body_id)) {
        if (e->topo_key == topo) return e;
    }
    return nullptr;
}

// Anchor-fallback sanity bound (VF-M3). `nearest_subshape` ranks candidates by
// distance to their bbox CENTRE and always returns the least-bad one, so a pick
// whose world point missed the body entirely still bound *something* — and that
// something got a persistent, op-referencable ElementId.
//
// The bound must be RELATIVE to the winner's own size, not absolute: a corner hit
// on a large planar face is legitimately far from that face's centre, so a fixed
// tolerance would drop exactly the picks the fallback exists to serve. Both the
// pick point and the whole candidate live inside the candidate's AABB whenever the
// pick is real, so the STRICT bound is half the bbox diagonal (`descriptor.size`).
// `kAnchorSlack = 1.0` doubles that — enough slack for a ray that lands slightly
// off-surface and for the quantized descriptor centre, while still rejecting a
// miss by a body-width. `kAnchorFloorMm` keeps a degenerate candidate (a vertex,
// whose bbox diagonal is 0) pickable at all.
constexpr double kAnchorSlack = 1.0;
constexpr double kAnchorFloorMm = 1.0;

bool anchor_within_candidate(const TopoDS_Shape& candidate, double wx, double wy, double wz) {
    const em::km::ElementDescriptor d = em::ElementMapPartition::describe(candidate);
    const double dx = d.center.X() - wx, dy = d.center.Y() - wy, dz = d.center.Z() - wz;
    const double dist = std::sqrt(dx * dx + dy * dy + dz * dz);
    return dist <= kAnchorSlack * d.size + kAnchorFloorMm;
}

// Resolve a pick's shape: topoKey (explicit), else anchor.worldPoint nearest.
TopoDS_Shape resolve_pick(const TopoDS_Shape& body, const json& pick, em::km::ElementKind& kind_out,
                          std::string& topo_out) {
    const std::string topo = get_str(pick, "topoKey");
    if (!topo.empty()) {
        TopoDS_Shape s = em::ElementMapPartition::shape_for_topokey(body, topo);
        if (!s.IsNull()) {
            topo_out = topo;
            switch (topo[0]) {
                case 'f': kind_out = em::km::ElementKind::Face; break;
                case 'e': kind_out = em::km::ElementKind::Edge; break;
                case 'v': kind_out = em::km::ElementKind::Vertex; break;
                default: kind_out = em::km::ElementKind::Unknown; break;
            }
            return s;
        }
    }
    // Anchor fallback (kind hint from pick.kind, default face).
    if (pick.contains("anchor") && pick["anchor"].is_object() &&
        pick["anchor"].contains("worldPoint") && pick["anchor"]["worldPoint"].is_array() &&
        pick["anchor"]["worldPoint"].size() >= 3) {
        const std::string kstr = get_str(pick, "kind", "face");
        em::km::ElementKind kind = em::ElementMapPartition::kind_from_name(kstr);
        if (kind == em::km::ElementKind::Unknown) kind = em::km::ElementKind::Face;
        const json& wp = pick["anchor"]["worldPoint"];
        const double wx = wp[0].get<double>(), wy = wp[1].get<double>(), wz = wp[2].get<double>();
        TopoDS_Shape s = em::ElementMapPartition::nearest_subshape(body, kind, wx, wy, wz);
        // Deliberately NOT routed through the §10 descriptor ladder's 0.85 gate: the
        // ladder scores a REMEMBERED descriptor against candidates, while this pick
        // carries only a world point, and a corner hit would score far below 0.85 on
        // its own face. The proportional veto is the right shape of test here.
        if (!s.IsNull() && anchor_within_candidate(s, wx, wy, wz)) {
            kind_out = kind;
            topo_out = em::ElementMapPartition::topokey_for_shape(body, s, kind);
            return s;
        }
    }
    return TopoDS_Shape();
}

struct PublishedRead {
    std::optional<PublishedStateSnapshot> state;
    std::optional<Envelope> error;
};

PublishedRead published_for(Session& session, const Envelope& req, const char* verb,
                            bool snapshot_required) {
    std::optional<std::uint64_t> requested;
    if (req.args.contains("snapshotId")) {
        if (!req.args["snapshotId"].is_number_unsigned()) {
            return {{}, Envelope::error_response(
                            req.id, protocol::ErrorInfo{"PROTOCOL_ERROR",
                                                        std::string(verb) +
                                                            ": snapshotId must be u64",
                                                        false})};
        }
        requested = req.args["snapshotId"].get<std::uint64_t>();
    } else if (snapshot_required) {
        return {{}, Envelope::error_response(
                        req.id, protocol::ErrorInfo{"PROTOCOL_ERROR",
                                                    std::string(verb) +
                                                        ": snapshotId is required",
                                                    false})};
    }
    std::uint64_t head = 0;
    auto state = session.published_state_at(requested, &head);
    if (state) return {std::move(state), {}};
    return {{}, Envelope::error_response(
                    req.id,
                    protocol::ErrorInfo{"REF_UNRESOLVED", std::string(verb) + ": stale snapshot",
                                        false,
                                        json{{"requested", *requested},
                                             {"head", head}}})};
}

json acquire_ids(const PublishedStateSnapshot& state, const std::string& body_id,
                 const json& picks) {
    json ids = json::array();
    const BodyRecord* body = state.bodies.get(body_id);
    if (!body || !picks.is_array()) return ids;
    for (const json& pick : picks) {
        em::km::ElementKind kind = em::km::ElementKind::Unknown;
        std::string topo;
        TopoDS_Shape sub = resolve_pick(body->geom, pick, kind, topo);
        if (sub.IsNull()) continue;
        const em::PartitionEntry* held = entry_by_topokey(state.partition, body_id, topo);
        const std::string kind_name = em::ElementMapPartition::kind_name(kind);
        WLOG_DEBUG(
            "ref_identity verb=AcquireElementIds snapshot=%llu body=%s topoKey=%s kind=%s "
            "directHit=%s elementId=%s outcome=resolved",
            static_cast<unsigned long long>(state.snapshot_id), body_id.c_str(), topo.c_str(),
            kind_name.c_str(), held ? "true" : "false", held ? held->element_id.c_str() : "");
        json entry = {{"topoKey", topo},
                      {"kind", kind_name},
                      {"bodyId", body_id},
                      {"elementId", held ? held->element_id : ""},
                      {"descriptor", em::ElementMapPartition::descriptor_to_json(
                                           em::ElementMapPartition::describe(sub))}};
        if (pick.contains("anchor")) entry["anchor"] = pick["anchor"];
        ids.push_back(std::move(entry));
    }
    return ids;
}

bool parse_binding(const json& raw, ElementBindingInput& out) {
    if (!raw.is_object()) return false;
    for (const char* key : {"bodyId", "topoKey", "elementId", "kind"}) {
        if (!raw.contains(key) || !raw[key].is_string()) return false;
    }
    if (raw.contains("anchor") && !raw["anchor"].is_object() && !raw["anchor"].is_null())
        return false;
    out.body_id = raw["bodyId"].get<std::string>();
    out.topo_key = raw["topoKey"].get<std::string>();
    out.element_id = raw["elementId"].get<std::string>();
    out.kind = raw["kind"].get<std::string>();
    out.anchor = raw.contains("anchor") ? raw["anchor"] : json(nullptr);
    return true;
}

json bound_json(const std::vector<BoundElement>& bound) {
    json result = json::array();
    for (const auto& entry : bound) {
        result.push_back(json{{"bodyId", entry.body_id},
                              {"topoKey", entry.topo_key},
                              {"elementId", entry.element_id},
                              {"kind", entry.kind}});
    }
    return result;
}

json query_by_id(const em::ElementMapPartition& part, const std::string& element_id) {
    const em::PartitionEntry* entry = part.find(element_id);
    WLOG_DEBUG("ref_identity verb=QueryElement lane=direct elementId=%s directHit=%s outcome=%s",
               element_id.c_str(), entry ? "true" : "false", entry ? "present" : "unresolved");
    if (!entry) return json{{"elementId", element_id}, {"present", false}};
    return json{{"elementId", element_id},
                {"topoKey", entry->topo_key},
                {"bodyId", entry->body_id},
                {"kind", em::ElementMapPartition::kind_name(entry->kind)},
                {"descriptor", em::ElementMapPartition::descriptor_to_json(entry->descriptor)},
                {"anchor", entry->anchor.is_null() ? json::object() : entry->anchor},
                {"present", true}};
}

json query_by_topokey(const PublishedStateSnapshot& state, const json& args) {
    const std::string topo = get_str(args, "topoKey");
    const std::string body_id = get_str(args, "bodyId");
    const BodyRecord* body = state.bodies.get(body_id);
    if (!body || topo.empty()) {
        WLOG_DEBUG("ref_identity verb=QueryElement lane=topo body=%s topoKey=%s outcome=unresolved",
                   body_id.c_str(), topo.c_str());
        return json{{"present", false}};
    }
    TopoDS_Shape sub = em::ElementMapPartition::shape_for_topokey(body->geom, topo);
    if (sub.IsNull()) {
        WLOG_DEBUG("ref_identity verb=QueryElement lane=topo body=%s topoKey=%s outcome=unresolved",
                   body_id.c_str(), topo.c_str());
        return json{{"present", false}};
    }
    const em::PartitionEntry* held = entry_by_topokey(state.partition, body_id, topo);
    WLOG_DEBUG(
        "ref_identity verb=QueryElement lane=topo body=%s topoKey=%s directHit=%s outcome=present",
        body_id.c_str(), topo.c_str(), held ? "true" : "false");
    const auto kind = em::ElementMapPartition::kind_from_name(
        topo[0] == 'f' ? "face" : topo[0] == 'e' ? "edge" : topo[0] == 'v' ? "vertex" : "");
    return json{{"elementId", held ? held->element_id : ""},
                {"topoKey", topo},
                {"bodyId", body_id},
                {"kind", em::ElementMapPartition::kind_name(kind)},
                {"descriptor", em::ElementMapPartition::descriptor_to_json(
                                   em::ElementMapPartition::describe(sub))},
                {"present", true}};
}

json missing_body_resolution(const json& ref, const std::string& ref_id,
                             const std::string& element_id, const std::string& body_id) {
    // No `bodyId` echo here: SCHEMA §7.5 echoes the body candidates were enumerated
    // FROM, and this branch exists precisely because there was none.
    return json{{"refId", ref_id},
                {"outcome", "needsRepair"},
                {"needsRepair",
                 json{{"refId", ref_id},
                      {"elementId", element_id},
                      {"ladderFailed", "descriptor"},
                      {"reason", "no-candidates"},
                      {"scoringVersion", em::kResolverVersion},
                      {"candidates", json::array()},
                      {"anchor", ref.contains("anchor") ? ref["anchor"] : json::object()},
                      {"uiLabel", "referenced body not found: " + body_id}}}};
}

json ladder_resolution(const PublishedStateSnapshot& state, const json& ref,
                       const std::string& ref_id, const std::string& element_id,
                       const std::string& body_id, const TopoDS_Shape& body) {
    em::LadderRef ladder_ref = em::ladder_ref_from_input(ref, ref_id);
    if (ladder_ref.element_id.empty()) ladder_ref.element_id = element_id;
    const auto results = em::resolve_descriptor_stage(body, body_id, {ladder_ref});
    const em::LadderResolution* result = results.empty() ? nullptr : &results[0];
    WLOG_DEBUG(
        "ref_identity verb=ResolveRefs refId=%s elementId=%s body=%s lane=descriptor "
        "candidates=%zu outcome=%s reason=%s score=%.6f margin=%.6f topoKey=%s",
        ref_id.c_str(), element_id.c_str(), body_id.c_str(),
        result ? result->candidates.size() : 0,
        result && result->outcome == em::LadderOutcome::AutoBind ? "autoBind" : "needsRepair",
        result ? result->reason.c_str() : "no-result", result ? result->score : 0.0,
        result ? result->margin : 0.0, result ? result->bound_topo_key.c_str() : "");
    if (results.empty() || results[0].outcome != em::LadderOutcome::AutoBind) {
        return json{{"refId", ref_id},
                    {"bodyId", body_id},
                    {"outcome", "needsRepair"},
                    {"needsRepair", results.empty() ? json::object()
                                                     : results[0].to_needs_repair_json()}};
    }
    const em::PartitionEntry* held =
        entry_by_topokey(state.partition, body_id, results[0].bound_topo_key);
    return json{{"refId", ref_id},
                {"bodyId", body_id},
                {"outcome", "autoBind"},
                {"elementId", held ? held->element_id : ""},
                {"topoKey", results[0].bound_topo_key},
                {"score", results[0].score},
                {"margin", results[0].margin}};
}

json resolve_one(const PublishedStateSnapshot& state, const json& ref) {
    const std::string ref_id = get_str(ref, "refId");
    const json primary = ref.contains("primary") && ref["primary"].is_object()
                             ? ref["primary"]
                             : json::object();
    const std::string element_id = get_str(primary, "elementId");
    if (!element_id.empty()) {
        if (const em::PartitionEntry* entry = state.partition.find(element_id)) {
            WLOG_DEBUG(
                "ref_identity verb=ResolveRefs refId=%s elementId=%s lane=direct directHit=true "
                "outcome=unchanged topoKey=%s",
                ref_id.c_str(), element_id.c_str(), entry->topo_key.c_str());
            return json{{"refId", ref_id},
                        {"bodyId", entry->body_id},
                        {"outcome", "unchanged"},
                        {"elementId", element_id},
                        {"topoKey", entry->topo_key}};
        }
        WLOG_DEBUG(
            "ref_identity verb=ResolveRefs refId=%s elementId=%s lane=direct directHit=false "
            "outcome=fallback",
            ref_id.c_str(), element_id.c_str());
    }
    const std::string body_id = get_str(primary, "bodyId");
    const BodyRecord* body = state.bodies.get(body_id);
    if (!body) {
        WLOG_DEBUG(
            "ref_identity verb=ResolveRefs refId=%s elementId=%s body=%s lane=descriptor "
            "candidates=0 outcome=needsRepair reason=body-missing",
            ref_id.c_str(), element_id.c_str(), body_id.c_str());
        return missing_body_resolution(ref, ref_id, element_id, body_id);
    }
    return ladder_resolution(state, ref, ref_id, element_id, body_id, body->geom);
}

}  // namespace

Envelope handle_acquire_element_ids(Session& session, const Envelope& req) {
    PublishedRead published = published_for(session, req, "AcquireElementIds", false);
    if (published.error) return std::move(*published.error);
    const std::string body_id = get_str(req.args, "bodyId");
    if (!published.state->bodies.get(body_id)) {
        return Envelope::error_response(
            req.id, protocol::ErrorInfo{"REF_UNRESOLVED", "AcquireElementIds: body not found: " + body_id,
                                        /*retriable=*/false});
    }
    const json picks = req.args.value("picks", json::array());
    WLOG_DEBUG("ref_identity verb=AcquireElementIds snapshot=%llu body=%s picks=%zu",
               static_cast<unsigned long long>(published.state->snapshot_id), body_id.c_str(),
               picks.is_array() ? picks.size() : 0);
    return Envelope::ok_response(
        req.id, json{{"ids", acquire_ids(*published.state, body_id, picks)}});
}

Envelope handle_bind_element_ids(Session& session, const Envelope& req) {
    if (!req.args.contains("snapshotId") || !req.args["snapshotId"].is_number_unsigned() ||
        !req.args.contains("bindings") || !req.args["bindings"].is_array()) {
        WLOG_DEBUG(
            "ref_identity verb=BindElementIds requested=invalid head=%llu batch=invalid "
            "outcome=rejected reason=malformed-args",
            static_cast<unsigned long long>(session.current_snapshot_id()));
        return Envelope::error_response(
            req.id, protocol::ErrorInfo{"PROTOCOL_ERROR",
                                        "BindElementIds: snapshotId and bindings are required",
                                        false});
    }
    std::vector<ElementBindingInput> bindings;
    for (const json& raw : req.args["bindings"]) {
        ElementBindingInput binding;
        if (!parse_binding(raw, binding)) {
            WLOG_DEBUG(
                "ref_identity verb=BindElementIds requested=%llu head=%llu batch=%zu "
                "outcome=rejected reason=malformed-binding",
                static_cast<unsigned long long>(req.args["snapshotId"].get<std::uint64_t>()),
                static_cast<unsigned long long>(session.current_snapshot_id()),
                req.args["bindings"].size());
            return Envelope::error_response(
                req.id, protocol::ErrorInfo{"PROTOCOL_ERROR",
                                            "BindElementIds: malformed binding", false});
        }
        bindings.push_back(std::move(binding));
    }
    const auto snapshot = req.args["snapshotId"].get<std::uint64_t>();
    BindElementsOutcome outcome = session.bind_element_ids(snapshot, bindings);
    if (!outcome.ok) return Envelope::error_response(req.id, std::move(outcome.error));
    return Envelope::ok_response(req.id, json{{"bound", bound_json(outcome.bound)}});
}

Envelope handle_query_element(Session& session, const Envelope& req) {
    PublishedRead published = published_for(session, req, "QueryElement", false);
    if (published.error) return std::move(*published.error);
    if (req.args.contains("elementId") && req.args["elementId"].is_string()) {
        return Envelope::ok_response(
            req.id, query_by_id(published.state->partition,
                                req.args["elementId"].get<std::string>()));
    }
    return Envelope::ok_response(req.id, query_by_topokey(*published.state, req.args));
}

Envelope handle_resolve_refs(Session& session, const Envelope& req) {
    PublishedRead published = published_for(session, req, "ResolveRefs", false);
    if (published.error) return std::move(*published.error);
    json resolutions = json::array();
    const json refs = req.args.value("refs", json::array());
    // SCHEMA §7.5: every resolution carries the exact snapshot it was computed
    // against and the document revision of that head. The client caches candidates
    // by {revision, snapshotId, refId} and may promote TopoKeys ONLY against the
    // echoed snapshot — so the echo has to come from the state the ladder actually
    // ran on, never from what the CALLER believed the head was.
    const std::uint64_t snapshot_id = published.state->snapshot_id;
    const std::uint64_t revision = session.head_stamp().document_revision;
    if (refs.is_array()) {
        for (const json& ref : refs) {
            json resolution = resolve_one(*published.state, ref);
            resolution["snapshotId"] = snapshot_id;
            resolution["revision"] = revision;
            resolutions.push_back(std::move(resolution));
        }
    }
    return Envelope::ok_response(req.id, json{{"resolutions", std::move(resolutions)}});
}

}  // namespace onecad::session
