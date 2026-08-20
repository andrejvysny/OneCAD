// ComposedHistory.h — ONE OCCT history over a MULTI-STAGE chain (WP3 C3).
//
// OffsetFace DirectModeler V1 is three kernel algorithms in a row — suppress the
// blend (`BRepAlgoAPI_Defeaturing`), offset the design face
// (`BRepOffset_MakeOffset`), re-round the exposed seed edge
// (`BRepFilletAPI_MakeFillet`) — but the element map may only ever see ONE
// history: `ElementMapPartition::apply_history` rebinds a tracked sub-shape of the
// PREDECESSOR body straight onto a sub-shape of the FINAL body. This class is that
// single history. It derives from `BRepBuilderAPI_MakeShape` so `apply_history`
// consumes it unchanged, exactly as `ops::offsetface::OffsetImageHistory` does for
// the one-stage offset.
//
// ── Why the composition is a hand-written walk, not `BRepTools_History::Merge` ──
// `BRepTools_History` is still the per-stage ingestion primitive (its template
// constructor accepts anything exposing `IsDeleted`/`Modified`/`Generated`, which
// covers `BRepAlgoAPI_Defeaturing` and `BRepFilletAPI_MakeFillet` directly and
// `BRepOffset_MakeOffset` through the `OffsetImageHistory` adapter). Its `Merge`
// is NOT used to chain the stages. Measured on the real chain in
// `worker/tests/test_composed_history.cpp` (OCCT 8.0.1):
//
//   1. REMOVAL. A stage-1 `Removed` survives later merges (the suppressed blend
//      face still reports `IsRemoved` after two more `Merge` calls). But a removal
//      in a LATER stage TRUNCATES lineage instead of continuing it: the seed edge
//      reaches `Generated(support)` after merging stage 2, stage 3 removes that
//      edge and generates the re-blended face from it, and the merged history then
//      answers `Generated(support)` WITHOUT the re-blended face — contradicting
//      `BRepTools_History`'s own documented merge rule 1
//      (`Tj <= G12(Si), Qk <= G23(Tj) ==> Qk <= G13(Si)`). The merged history also
//      reports `IsRemoved == false` for that edge although stage 3 positively
//      removed it.
//   2. INTERMEDIATE KEYS. Retention is inconsistent: the stage-1-born seed edge is
//      NOT a key of the merged history, while the stage-2-born offset image of the
//      same edge IS, answering `Generated` with the final re-blended face. So the
//      LAST merged stage's argument sub-shapes leak in as queryable keys.
//   3. MODIFIED SURVIVAL. Both blend supports (and an untouched control face) stay
//      `Modified` end to end — exactly one modified image each, present in the
//      final shape, never degraded to `Generated`. That is the fact the identity
//      spine needs: `ElementMapPartition.cpp`'s `kModifiedHistoryConfidence = 0.09`
//      is added only to a `Modified` candidate, and the auto-bind gate is score
//      >= 0.85 AND margin >= 0.10, so losing it on a split manufactures a spurious
//      `NeedsRepair`.
//
// Fact 1 is disqualifying on its own — the re-blended face's only lineage back to
// the predecessor runs through a seed edge that stage 3 removes, and `Merge`
// throws it away. The stage-by-stage walk below follows `BRepTools_History`'s
// documented rules instead, including OCCT's own stated case that "the shapes may
// be generated from the removed shape".
//
// ── Contract ─────────────────────────────────────────────────────────────────
//   (a) SCOPE. Queries answer ONLY for sub-shapes of the declared PREDECESSOR
//       whose type `BRepTools_History` actually keys (vertex, edge, face, solid).
//       Anything else — above all an intermediate-stage transient such as the
//       sharp seed edge a defeature exposes and a later fillet consumes — is
//       UNANSWERABLE: empty successors and NOT deleted. Transients are therefore
//       unpublishable BY CONSTRUCTION, not by a downstream filter, which is also
//       what contains fact 2's key leak.
//   (b) SUCCESSORS. Filtered to sub-shapes present in the declared FINAL shape,
//       deduplicated, and sorted by the successor's ordinal in
//       `TopExp::MapShapes(final)`. Deterministic across runs and across two
//       independent constructions over the same stages.
//       Successors are NOT filtered by shape type: an edge that generates a face
//       (a fillet seed) is real lineage and V1's reproduction proof needs it.
//   (c) DELETION. `is_deleted` is true only on POSITIVE evidence — every branch of
//       the walk ended in a stage that reported the shape removed with no
//       successor. An unanswerable or merely unresolvable query returns "no
//       successors, not deleted" so `apply_history` reaches its no-successor
//       branch and emits `NeedsRepair` instead of erasing the ElementId. A
//       deterministic `NeedsRepair` beats a silent wrong bind.
//   Modified vs Generated: a successor is `Modified` only if EVERY hop that
//   reached it was `Modified`; one `Generated` hop makes the whole chain
//   `Generated` (`BRepTools_History` merge rules 1-3). A successor reachable both
//   ways is reported as `Modified`, matching how `apply_history` already
//   deduplicates its two history channels.
#pragma once

#include <vector>

#include <BRepBuilderAPI_MakeShape.hxx>
#include <BRepTools_History.hxx>
#include <TopTools_DataMapOfShapeListOfShape.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopTools_ListOfShape.hxx>
#include <TopTools_MapOfShape.hxx>
#include <TopoDS_Shape.hxx>

namespace onecad::elementmap {

class ComposedHistory : public BRepBuilderAPI_MakeShape {
public:
    // `predecessor` is the body the tracked ElementIds are bound to when the chain
    // starts; `final_shape` is the body they must land on when it ends.
    ComposedHistory(const TopoDS_Shape& predecessor, const TopoDS_Shape& final_shape);

    // Append one stage. Order is the execution order of the chain.
    void add_stage(const occ::handle<BRepTools_History>& stage);

    // Append one stage straight from an algorithm exposing the standard OCCT
    // history methods, over the sub-shapes of `stage_input` (that stage's input
    // body). `BRepTools_History`'s template constructor does the ingestion.
    template <class Algo>
    void add_stage(const TopoDS_Shape& stage_input, Algo& algo) {
        TopTools_ListOfShape arguments;
        if (!stage_input.IsNull()) arguments.Append(stage_input);
        add_stage(occ::handle<BRepTools_History>(new BRepTools_History(arguments, algo)));
    }

    // Assert `successor` as a MODIFIED image of `predecessor_sub`. This is the seam
    // WP3-C5's blend-lineage bridge needs: the suppressed blend face has no kernel
    // lineage to its reconstruction, so the mapping is asserted where the
    // reproduction proof lives. This class is a dumb primitive — it stores and
    // answers the mapping and proves nothing.
    //
    // Injection CLAIMS `successor` outright. It is published as `Modified` under
    // `predecessor_sub`, and it is removed from the `Generated` channel of EVERY
    // key — not merely of `predecessor_sub`.
    //
    // The per-key half is the obvious one: one shape may never appear on both
    // channels (`BRepTools_History`: G(Si) ^ M(Si) == 0). The cross-key half is the
    // one that bites. On the OffsetFace chain the walk also reaches the re-blended
    // face as GENERATED lineage from the blend's two SUPPORTS, because the defeature
    // generates the seed edge from each support, the offset modifies it, and the
    // re-fillet generates the face from it. Left live under those ids it becomes a
    // scored candidate in `apply_history`'s split branch, where an asymmetric score
    // can bind a support to it instead — and nothing in `apply_history` forbids two
    // ElementIds resolving onto the same final TopoKey, after which Tessellate's
    // last-writer-wins quietly drops one of them from the wire map. So the per-key
    // G ^ M == 0 argument is necessary but NOT sufficient: an identity asserted where
    // its proof lives must not have to out-score generic Generated lineage elsewhere.
    //
    // Contracts (a) and (b) still apply: an injected pair is answerable only if
    // `predecessor_sub` is a keyable predecessor sub-shape, and the successor is
    // reported only if it is present in the final shape.
    void add_modified(const TopoDS_Shape& predecessor_sub, const TopoDS_Shape& successor);

    std::vector<TopoDS_Shape> modified(const TopoDS_Shape& shape) const;
    std::vector<TopoDS_Shape> generated(const TopoDS_Shape& shape) const;
    bool is_deleted(const TopoDS_Shape& shape) const;

    // `BRepBuilderAPI_MakeShape` seam — thin adapters over the three above, so
    // `ElementMapPartition::apply_history` consumes a composed chain verbatim.
    const TopTools_ListOfShape& Modified(const TopoDS_Shape& S) override;
    const TopTools_ListOfShape& Generated(const TopoDS_Shape& S) override;
    Standard_Boolean IsDeleted(const TopoDS_Shape& S) override;

private:
    struct Successor {
        TopoDS_Shape shape;
        bool modified = false;
    };
    struct Walk {
        std::vector<Successor> survivors;
        bool removal_evidence = false;
    };

    bool answerable(const TopoDS_Shape& shape) const;
    Walk walk(const TopoDS_Shape& shape) const;
    // Successors of `shape` whose composed lineage is (or is not) `Modified`,
    // final-shape filtered, deduplicated and ordinal-sorted.
    std::vector<TopoDS_Shape> publish(const TopoDS_Shape& shape, bool want_modified) const;

    TopTools_IndexedMapOfShape predecessor_subs_;
    TopTools_IndexedMapOfShape final_subs_;
    std::vector<occ::handle<BRepTools_History>> stages_;
    TopTools_DataMapOfShapeListOfShape injected_;
    // Every shape claimed by an `add_modified`, across all keys. A claimed successor
    // is removed from EVERY key's `Generated` channel, not just the injecting key's.
    TopTools_MapOfShape injected_successors_;
    TopTools_ListOfShape modified_cache_;
    TopTools_ListOfShape generated_cache_;
};

}  // namespace onecad::elementmap
