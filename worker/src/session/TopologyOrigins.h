#pragma once

#include <algorithm>
#include <functional>
#include <map>
#include <memory>
#include <string>
#include <utility>
#include <vector>

#include <NCollection_IndexedDataMap.hxx>
#include <TopoDS_Shape.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopTools_ShapeMapHasher.hxx>

#include "session/SourceLedger.h"

namespace onecad::session {

enum class OriginState { Known, Ambiguous, Unknown };

// Namespace of an OriginClaim slot. Point = Hole `points[].id` (SCHEMA §7.3);
// Generator = deterministic profile-derived slot (Extrude side/cap/edge). The
// scopes never compare equal so a Hole originSlot filter cannot admit a
// Generator claim that happens to share a slot string.
enum class SlotScope { Point, Generator };

struct OriginClaim {
    OriginState state = OriginState::Unknown;
    std::string producer_record_id;
    // Sub-feature slot inside the producer (SCHEMA §7.3 Hole `points[].id`,
    // ONECAD-361). Empty = the producer made no per-slot claim. A slot never
    // makes a claim Ambiguous: rows that agree on the producer but not on the
    // slot keep the producer and drop the slot, so producer-level consumers
    // (FeaturePattern) see exactly what they saw before slots existed.
    std::string slot;
    // Meaningful only when `slot` is non-empty.
    SlotScope scope = SlotScope::Point;
    // Generator scope: the shape was born at the producer or carried only by
    // IsSame survivors. A Modified (trimmed/split-continued) image keeps the slot
    // but is not pristine; narrowing by that slot is withheld (SCHEMA §10).
    bool pristine = true;
};

// Merge two rows of ONE shape: producer disagreement is Ambiguous; slot
// disagreement only clears the slot.
inline OriginClaim merge_origin(const OriginClaim& a, const OriginClaim& b) {
    if (a.state != b.state || a.producer_record_id != b.producer_record_id)
        return {OriginState::Ambiguous, {}, {}};
    OriginClaim merged = a;
    if (merged.slot != b.slot || merged.scope != b.scope) merged.slot.clear();
    merged.pristine = a.pristine && b.pristine;
    return merged;
}

struct OwnedShape {
    std::string body_id;
    TopoDS_Shape shape;
    OriginClaim claim;
};

class TopologyOwnerLedger {
public:
    SourceLedger& source_ledger() { return source_ledger_; }
    const SourceLedger& source_ledger() const { return source_ledger_; }
    OriginClaim lookup(const std::string& body_id, const TopoDS_Shape& shape) const {
        OriginClaim result;
        bool found = false;
        for (const OwnedShape& entry : entries_) {
            if (entry.body_id != body_id || entry.shape.IsNull() || shape.IsNull() ||
                !entry.shape.IsSame(shape)) continue;
            if (!found) {
                result = entry.claim;
                found = true;
            } else {
                result = merge_origin(result, entry.claim);
                if (result.state == OriginState::Ambiguous) return result;
            }
        }
        return result;
    }

    // The effective claim of every shape of ONE body, collapsed per IsSame class
    // exactly as `lookup` collapses it: the first row stands, and any later row
    // that disagrees makes the class Ambiguous (a fixed point — once Ambiguous,
    // later rows cannot move it back). A shape with no row is ABSENT from the
    // index; `lookup` reports that same shape Unknown.
    //
    // Build this once when many shapes of one body must be classified: `lookup`
    // is an O(rows) scan, so a per-sub-shape loop over a large body is quadratic
    // (measured 7.45 s for a FeaturePattern refresh over a 2041-sub-shape body at
    // 128 instances). One pass plus hashed probes is O(rows + shapes).
    using EffectiveClaims =
        NCollection_IndexedDataMap<TopoDS_Shape, OriginClaim, TopTools_ShapeMapHasher>;

    EffectiveClaims effective_claims(const std::string& body_id) const {
        EffectiveClaims claims;
        for (const OwnedShape& entry : entries_) {
            if (entry.body_id != body_id || entry.shape.IsNull()) continue;
            const int index = claims.FindIndex(entry.shape);
            if (index == 0) {
                claims.Add(entry.shape, entry.claim);
                continue;
            }
            OriginClaim& current = claims.ChangeFromIndex(index);
            if (current.state != OriginState::Ambiguous)
                current = merge_origin(current, entry.claim);
        }
        return claims;
    }

    void erase_body(const std::string& body_id) {
        std::erase_if(entries_, [&](const OwnedShape& entry) { return entry.body_id == body_id; });
    }

    void rename_body(const std::string& old_body_id, const std::string& new_body_id) {
        for (OwnedShape& entry : entries_) {
            if (entry.body_id == old_body_id) entry.body_id = new_body_id;
        }
    }

    // Body-level Generator slot uniqueness: a (producer, slot) that more than one
    // effective IsSame class of `body_id` holds names no single element, so the
    // slot is cleared on every row of those classes. A cleared claim stays Known
    // for its producer; merge_origin/inherit_origin only ever drop slots, so a
    // cleared slot cannot come back. Point (Hole) slots are not touched.
    void clear_duplicate_generator_slots(const std::string& body_id) {
        const EffectiveClaims claims = effective_claims(body_id);
        std::map<std::pair<std::string, std::string>, int> held;
        const auto is_slotted = [](const OriginClaim& claim) {
            return claim.state == OriginState::Known && claim.scope == SlotScope::Generator &&
                   !claim.slot.empty();
        };
        for (int i = 1; i <= claims.Extent(); ++i) {
            const OriginClaim& claim = claims.FindFromIndex(i);
            if (is_slotted(claim)) ++held[{claim.producer_record_id, claim.slot}];
        }
        for (OwnedShape& entry : entries_) {
            if (entry.body_id != body_id || entry.shape.IsNull()) continue;
            const OriginClaim* claim = claims.Seek(entry.shape);
            if (claim != nullptr && is_slotted(*claim) &&
                held[{claim->producer_record_id, claim->slot}] > 1)
                entry.claim.slot.clear();
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
    SourceLedger source_ledger_;
    std::vector<OwnedShape> entries_;
};

struct TopologySurvivor { TopoDS_Shape before; TopoDS_Shape after; };
struct TopologySuccessor { std::vector<TopoDS_Shape> before; TopoDS_Shape after; };
// `slot` names the producer sub-feature that created the shape (Hole point id);
// empty when the operation makes no per-slot claim.
struct TopologyBirth {
    TopoDS_Shape after;
    std::string slot = {};
    SlotScope scope = SlotScope::Point;
    bool pristine = true;
};
struct TopologyConflict {
    TopoDS_Shape after;
    // Empty means predecessor evidence is unavailable, not that there were none.
    std::vector<TopoDS_Shape> before;
};

enum class HistoryCoverage { Proven, Unproven };

struct TopologyBodyHistory {
    std::string body_id;
    HistoryCoverage coverage = HistoryCoverage::Unproven;
    std::vector<TopologySurvivor> survivors;
    std::vector<TopologySuccessor> successors;
    std::vector<TopologyBirth> births;
    std::vector<TopologyConflict> conflicts;
};

// Order-insensitive by construction: the scan never short-circuits on Unknown,
// because a single Unknown source appearing FIRST must not hide a disagreement
// between two Known ones further down the list. Precedence is
// Ambiguous > Unknown > the one common Known producer; no sources at all, or
// only Unknown ones, is Unknown.
inline OriginClaim inherit_origin(const TopologyOwnerLedger& prior,
                                  const std::string& body_id,
                                  const std::vector<TopoDS_Shape>& sources) {
    bool any_unknown = false;
    bool found_known = false;
    OriginClaim known;
    for (const TopoDS_Shape& source : sources) {
        const OriginClaim claim = prior.lookup(body_id, source);
        if (claim.state == OriginState::Ambiguous) return {OriginState::Ambiguous, {}, {}};
        if (claim.state == OriginState::Unknown) {
            any_unknown = true;
            continue;
        }
        known = found_known ? merge_origin(known, claim) : claim;
        if (known.state == OriginState::Ambiguous) return known;
        found_known = true;
    }
    if (any_unknown || !found_known) return {};
    return known;
}

inline void add_origin(TopologyOwnerLedger& ledger, const std::string& body_id,
                       const TopoDS_Shape& shape, OriginClaim claim) {
    if (shape.IsNull()) return;
    if (ledger.contains(body_id, shape)) claim = merge_origin(ledger.lookup(body_id, shape), claim);
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
            // Modified lineage: the slot carries on but is no longer pristine.
            OriginClaim claim = inherit_origin(prior, history.body_id, successor.before);
            claim.pristine = false;
            add_origin(classified, history.body_id, successor.after, std::move(claim));
        }
        for (const TopologyBirth& birth : history.births) {
            add_origin(classified, history.body_id, birth.after,
                       {OriginState::Known, op_id, birth.slot, birth.scope, birth.pristine});
        }
        for (const TopologyConflict& conflict : history.conflicts) {
            add_origin(classified, history.body_id, conflict.after,
                       {OriginState::Ambiguous, {}, {}});
        }
        classified.clear_duplicate_generator_slots(history.body_id);
        for (const OwnedShape& entry : classified.entries()) ledger.add(entry);
    }
}

// Ladder admissibility for a ref whose frozen descriptor names a producer slot
// (SCHEMA §7.3 Hole `originSlot`): only shapes of `body_id` whose live claim is
// Known with exactly that producer and slot are candidates. A sibling slot, an
// Unknown or Ambiguous shape is never admitted, so a ref can bind its own hole
// or go NeedsRepair, never a congruent sibling.
inline std::function<bool(const TopoDS_Shape&)> origin_slot_filter(
    const TopologyOwnerLedger& ledger, const std::string& body_id,
    const std::string& producer_record_id, const std::string& slot) {
    auto claims = std::make_shared<TopologyOwnerLedger::EffectiveClaims>(
        ledger.effective_claims(body_id));
    return [claims, producer_record_id, slot](const TopoDS_Shape& candidate) {
        const OriginClaim* claim = claims->Seek(candidate);
        return claim != nullptr && claim->state == OriginState::Known &&
               claim->producer_record_id == producer_record_id && claim->slot == slot &&
               claim->scope == SlotScope::Point;
    };
}

// Ladder admissibility for a ref whose frozen descriptor names an Extrude
// generator slot (SCHEMA §7.5 `generatorSlot`). GATED: when no claim on the body
// is a Known Generator-scope claim by `record_id` (provenance lost — a later
// Boolean/Shell/Transform erased it, or the record is not live) the result is an
// EMPTY function, i.e. no narrowing and today's pool. Otherwise only shapes
// whose claim is Known Generator-scope with that producer and slot are admitted.
inline std::function<bool(const TopoDS_Shape&)> generator_slot_filter(
    const TopologyOwnerLedger& ledger, const std::string& body_id,
    const std::string& record_id, const std::string& slot) {
    if (slot.empty()) return {};
    auto claims = std::make_shared<TopologyOwnerLedger::EffectiveClaims>(
        ledger.effective_claims(body_id));
    const auto is_generated = [record_id](const OriginClaim& claim) {
        return claim.state == OriginState::Known && claim.scope == SlotScope::Generator &&
               claim.producer_record_id == record_id && !claim.slot.empty();
    };
    bool gate_open = false;
    for (int i = 1; i <= claims->Extent() && !gate_open; ++i)
        gate_open = is_generated(claims->FindFromIndex(i));
    if (!gate_open) return {};
    // Narrow only on pristine holders: a trimmed continuation is not proof of
    // lineage, so any non-pristine holder leaves the HEAD path to decide.
    for (int i = 1; i <= claims->Extent(); ++i) {
        const OriginClaim& claim = claims->FindFromIndex(i);
        if (is_generated(claim) && claim.slot == slot && !claim.pristine) return {};
    }
    return [claims, is_generated, slot](const TopoDS_Shape& candidate) {
        const OriginClaim* claim = claims->Seek(candidate);
        return claim != nullptr && is_generated(*claim) && claim->slot == slot;
    };
}

}  // namespace onecad::session
