#pragma once

#include <algorithm>
#include <string>
#include <utility>
#include <vector>

#include <TopoDS_Shape.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>

namespace onecad::session {

enum class OriginState { Known, Ambiguous, Unknown };

struct OriginClaim {
    OriginState state = OriginState::Unknown;
    std::string producer_record_id;
};

struct OwnedShape {
    std::string body_id;
    TopoDS_Shape shape;
    OriginClaim claim;
};

class TopologyOwnerLedger {
public:
    OriginClaim lookup(const std::string& body_id, const TopoDS_Shape& shape) const {
        OriginClaim result;
        bool found = false;
        for (const OwnedShape& entry : entries_) {
            if (entry.body_id != body_id || entry.shape.IsNull() || shape.IsNull() ||
                !entry.shape.IsSame(shape)) continue;
            if (!found) {
                result = entry.claim;
                found = true;
            } else if (result.state != entry.claim.state ||
                       result.producer_record_id != entry.claim.producer_record_id) {
                return {OriginState::Ambiguous, {}};
            }
        }
        return result;
    }

    void erase_body(const std::string& body_id) {
        std::erase_if(entries_, [&](const OwnedShape& entry) { return entry.body_id == body_id; });
    }

    void rename_body(const std::string& old_body_id, const std::string& new_body_id) {
        for (OwnedShape& entry : entries_) {
            if (entry.body_id == old_body_id) entry.body_id = new_body_id;
        }
    }

    void add(OwnedShape entry) { entries_.push_back(std::move(entry)); }
    bool contains(const std::string& body_id, const TopoDS_Shape& shape) const {
        return std::any_of(entries_.begin(), entries_.end(), [&](const OwnedShape& entry) {
            return entry.body_id == body_id && !entry.shape.IsNull() && !shape.IsNull() &&
                   entry.shape.IsSame(shape);
        });
    }
    void retain_body_members(const std::string& body_id, const TopoDS_Shape& body) {
        TopTools_IndexedMapOfShape members;
        if (!body.IsNull()) TopExp::MapShapes(body, members);
        std::erase_if(entries_, [&](const OwnedShape& entry) {
            if (entry.body_id != body_id) return false;
            return entry.shape.IsNull() || body.IsNull() || !members.Contains(entry.shape);
        });
    }
    const std::vector<OwnedShape>& entries() const { return entries_; }

private:
    std::vector<OwnedShape> entries_;
};

struct TopologySurvivor { TopoDS_Shape before; TopoDS_Shape after; };
struct TopologySuccessor { std::vector<TopoDS_Shape> before; TopoDS_Shape after; };
struct TopologyBirth { TopoDS_Shape after; };
struct TopologyConflict { TopoDS_Shape after; };

enum class HistoryCoverage { Proven, Unproven };

struct TopologyBodyHistory {
    std::string body_id;
    HistoryCoverage coverage = HistoryCoverage::Unproven;
    std::vector<TopologySurvivor> survivors;
    std::vector<TopologySuccessor> successors;
    std::vector<TopologyBirth> births;
    std::vector<TopologyConflict> conflicts;
};

inline OriginClaim inherit_origin(const TopologyOwnerLedger& prior,
                                  const std::string& body_id,
                                  const std::vector<TopoDS_Shape>& sources) {
    OriginClaim result;
    bool found = false;
    for (const TopoDS_Shape& source : sources) {
        const OriginClaim claim = prior.lookup(body_id, source);
        if (claim.state == OriginState::Unknown) return claim;
        if (!found) {
            result = claim;
            found = true;
        } else if (result.state != claim.state ||
                   result.producer_record_id != claim.producer_record_id) {
            return {OriginState::Ambiguous, {}};
        }
    }
    return result;
}

inline void add_origin(TopologyOwnerLedger& ledger, const std::string& body_id,
                       const TopoDS_Shape& shape, OriginClaim claim) {
    if (shape.IsNull()) return;
    const bool classified = ledger.contains(body_id, shape);
    const OriginClaim prior = ledger.lookup(body_id, shape);
    if (classified &&
        (prior.state != claim.state || prior.producer_record_id != claim.producer_record_id)) {
        claim = {OriginState::Ambiguous, {}};
    }
    ledger.add({body_id, shape, std::move(claim)});
}

inline void apply_topology_history(TopologyOwnerLedger& ledger, const std::string& op_id,
                                   const std::vector<std::string>& touched_bodies,
                                   const std::vector<TopologyBodyHistory>& histories) {
    const TopologyOwnerLedger prior = ledger;
    for (const std::string& body_id : touched_bodies) ledger.erase_body(body_id);
    for (std::size_t i = 0; i < histories.size(); ++i) {
        const TopologyBodyHistory& history = histories[i];
        ledger.erase_body(history.body_id);
        if (std::any_of(histories.begin() + static_cast<std::ptrdiff_t>(i + 1), histories.end(),
                        [&](const TopologyBodyHistory& other) {
                            return other.body_id == history.body_id;
                        }) ||
            std::any_of(histories.begin(), histories.begin() + static_cast<std::ptrdiff_t>(i),
                        [&](const TopologyBodyHistory& other) {
                            return other.body_id == history.body_id;
                        })) continue;
        if (history.coverage != HistoryCoverage::Proven) continue;
        TopologyOwnerLedger classified;
        for (const TopologySurvivor& survivor : history.survivors) {
            add_origin(classified, history.body_id, survivor.after,
                       inherit_origin(prior, history.body_id, {survivor.before}));
        }
        for (const TopologySuccessor& successor : history.successors) {
            add_origin(classified, history.body_id, successor.after,
                       inherit_origin(prior, history.body_id, successor.before));
        }
        for (const TopologyBirth& birth : history.births) {
            add_origin(classified, history.body_id, birth.after, {OriginState::Known, op_id});
        }
        for (const TopologyConflict& conflict : history.conflicts) {
            add_origin(classified, history.body_id, conflict.after, {OriginState::Ambiguous, {}});
        }
        for (const OwnedShape& entry : classified.entries()) ledger.add(entry);
    }
}

}  // namespace onecad::session
