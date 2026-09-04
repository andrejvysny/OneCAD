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

#include <TopTools_IndexedDataMapOfShapeListOfShape.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS_Edge.hxx>
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

// The shape maps one `PrepareEdgeOp` answer needs, built ONCE per body. Building
// them per edge instead turns a closure of n edges into n full shape traversals.
struct EdgeEvidenceMaps {
  TopTools_IndexedMapOfShape edges;
  TopTools_IndexedDataMapOfShapeListOfShape edge_faces;
  TopTools_IndexedMapOfShape faces;
  explicit EdgeEvidenceMaps(const TopoDS_Shape &body);
};

// The 1-based face ordinals adjacent to `edge`, ASCENDING and de-duplicated
// (SCHEMA §7.6 `adjacentFaces`, §7.3 `referenceFaces` candidates).
//
// `TopTools_IndexedMapOfShape::FindIndex` identifies faces by `IsSame`, so a SEAM
// edge — bounded on both sides by ONE lateral face, e.g. a cylinder's seam — maps
// both ancestor entries onto the same ordinal and yields a single entry. A free
// edge yields none, a manifold edge two. Shared by `PrepareEdgeOp` (the handshake
// the frontend picks from) and `FilletChamferOp` (the candidates a
// `legacyReferenceFace` repair offers) so the two can never disagree about which
// faces a chamfer may measure `radius` on.
std::vector<int>
adjacent_face_ordinals(const TopTools_IndexedDataMapOfShapeListOfShape &edge_faces,
                       const TopTools_IndexedMapOfShape &face_map,
                       const TopoDS_Edge &edge);

// Convenience form for a caller with a single edge to ask about; builds the two
// maps itself.
std::vector<int> adjacent_face_ordinals(const TopoDS_Shape &body,
                                        const TopoDS_Edge &edge);

// One `edges[]` entry of the `PrepareEdgeOp` result: snapshot-scoped TopoKey,
// pick flag, anchor and descriptor EVIDENCE, plus the §7.6 `contour` index and
// `adjacentFaces` list. Mints nothing.
nlohmann::json edge_evidence_entry(const EdgeEvidenceMaps &maps, int ordinal,
                                   bool picked, int contour);

} // namespace onecad::session
