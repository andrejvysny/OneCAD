// EdgePicks.h — the `pickedEdges[]` addressing shared by the §7.6 edge-op verbs.
//
// `PrepareEdgeOp` and `AnalyzeEdgeOpRange` take the SAME selection argument and
// must resolve it the same way: same two address rungs, same cross-body refusal,
// same sorted-unique ordinal list, same evidence entry. Copying that would let
// the two verbs answer about different edges from one user gesture, which is the
// one thing a range guard must never do — a bound measured on a closure the
// commit does not use is worse than no bound at all.
#pragma once

#include <string>
#include <vector>

#include <TopoDS_Shape.hxx>

#include "elementmap/ElementMapPartition.h"
#include "nlohmann/json.hpp"
#include "session/BodyStore.h"

namespace onecad::session {

// One resolved pick: which body, and the 1-based `TopExp::MapShapes` edge
// ordinal inside it. `ordinal == 0` means it did not resolve.
struct EdgePick {
  std::string body_id;
  int ordinal = 0;
};

// The whole `pickedEdges[]` argument, resolved. `unresolved` and `cross_body`
// are the two refusal outcomes, kept apart because they map to different wire
// answers (`REF_UNRESOLVED` error vs. an `ok:true` `crossBody` refusal).
struct ResolvedEdgePicks {
  std::string body_id;
  std::vector<int> ordinals;  // sorted, unique
  bool unresolved = false;
  bool cross_body = false;
};

EdgePick resolve_edge_pick(const BodyStore &bodies,
                           const elementmap::ElementMapPartition &part,
                           const nlohmann::json &value);

ResolvedEdgePicks resolve_edge_picks(const BodyStore &bodies,
                                     const elementmap::ElementMapPartition &part,
                                     const nlohmann::json &values);

// One `edges[]` entry of the `PrepareEdgeOp` result: snapshot-scoped TopoKey,
// pick flag, anchor and descriptor EVIDENCE. Mints nothing.
nlohmann::json edge_evidence_entry(const TopoDS_Shape &body, int ordinal,
                                   bool picked);

} // namespace onecad::session
