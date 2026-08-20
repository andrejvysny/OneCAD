// OffsetFaceOp.h — real OCCT direct-modeling face offset (`op.offsetFace`, SCHEMA §7.3).
//
// Selected face(s) move along their surface normals; adjacent faces extend/trim
// (`BRepOffset_MakeOffset` with per-face `SetOffsetOnFace`, mode `BRepOffset_Skin`,
// boolean Intersection=false, join `GeomAbs_Intersection`). The op ALWAYS modifies
// `params.targetBodyId` in place — never NewBody, never a body fan-out (>1 output
// solid ⇒ recoverable `OP_FAILED`). V1 operative-surface scope: planar + cylindrical.
//
// ── What the Phase-0 spike pinned (worker/tests/test_offsetface_spike.cpp) ────
//   1. The kernel AUTO-PROPAGATES an offset across G1-tangent junctions: offsetting
//      one half of a split cylindrical wall silently moves the other half too.
//      "One tangent patch moves, its neighbour stays" is not expressible on this
//      kernel path, so a resolved operative set whose G1 closure contains
//      non-members is a recoverable OP_FAILED (the authoring verb
//      `PrepareOffsetFace` refuses it earlier, at pick time).
//   2. Face lineage is NOT in the public `Generated()`/`Modified()` lists (they carry
//      no same-kind face successors). It lives in the `BRepAlgo_Image` returned by
//      `OffsetFacesFromShapes()` / `OffsetEdgesFromShapes()` — hence the
//      `OffsetImageHistory` adapter below, which is what feeds the element map.
//   3. Traps the postconditions must catch: `R + σd ≤ 0` returns a "valid" inside-out
//      cylinder; a face driven onto its opposite returns a zero-volume shape that
//      passes IsDone + BRepCheck; a FOREIGN face (not owned by the target body) is
//      silently ignored.
//   4. (W1 addition) The kernel wraps its result into a SOLID only when the INPUT was
//      one. A body produced by a boolean is stored as a COMPOUND around its single
//      solid, and feeding that in yields a bare SHELL with identical geometry — which
//      would make the "exactly one solid" gate vacuously false. The executor therefore
//      normalizes the build input to the body's single solid first (and refuses a
//      multi-solid target: this op never fans a body out).
//
// ── Operative-set discipline (Fillet parity) ─────────────────────────────────
// `params.faceIds` is the FULL FROZEN operative set — the user's picks PLUS the
// tangent chain, expanded ONCE at authoring by `PrepareOffsetFace` and persisted.
// The worker NEVER re-expands at regen (an upstream edit must not silently widen or
// narrow the set behind the naming ladder). `chainTangentFaces` is retained as
// authoring metadata only. The G1 closure IS recomputed every regen, but purely as a
// GUARD: closure ⊋ operative set ⇒ recoverable OP_FAILED naming the extra TopoKeys.
//
// Resolution is Shell+Fillet's: partition-tracked binding first
// (`topokey_for_element_in_body`), else the descriptor+anchor ladder over the matching
// `inputs[]` typed ref. Neither ⇒ NeedsRepair STATE, never a wrong bind.
//
// Slot order in `inputs[]` is NORMATIVE (SCHEMA §7.3): operative faces in stored
// order, then — for `distanceType:"Total"` — the opposite face LAST.
//
// DETERMINISM. Every ordering here is a FACE ORDINAL
// (`TopExp::MapShapes(body, TopAbs_FACE)`), never a pointer or hash walk: the
// operative set, the tangent-closure BFS, the offset-image successor lists and the
// authoring verb's response all sort by it. `BRepOffset_MakeOffset` exposes no
// parallel knob of its own and its internal BOPs seed from
// `BOPAlgo_Options::GetParallelMode()`, which the worker never turns on (audited —
// the only `SetRunParallel` calls in the tree are the explicit `Standard_False`s
// here, in `PrepareOffsetFace.cpp`, and the `determinism.parallel`-driven one in
// `OpCommon::checked_boolean`).
//
// ── V3 (`resultPolicyVersion: 3`) — DirectModeler blend awareness (WP3-C5) ───
// V2's closure is blind to blends, so pushing a SIDE face of a filleted box drags
// the fillet and the opposite wall along and inflates R2 into R4 (see
// `blend_aware_closure` below). V3 replaces the closure walk with a blend-aware one
// and runs SUPPRESS → OFFSET → REBLEND, so the picked face moves and the corner
// comes back at its original radius. The two versions are separate code paths on
// purpose: V3 is a GEOMETRY CHANGE for the same stored record, so a V2 record keeps
// executing V2 forever and re-authoring at V3 is an attributable user action.
#pragma once

#include <string>
#include <vector>

#include <BRepAlgo_Image.hxx>
#include <BRepBuilderAPI_MakeShape.hxx>
#include <TopAbs_ShapeEnum.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopTools_ListOfShape.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Ax1.hxx>
#include <gp_Dir.hxx>
#include <gp_Pnt.hxx>

#include "kernel/fillet/BlendReconstruction.h"
#include "nlohmann/json.hpp"
#include "ops/OpTypes.h"

namespace onecad::ops {

// Geometry helpers shared by the executor and the read-only `PrepareOffsetFace`
// authoring verb (session/PrepareOffsetFace.cpp). Kept here so the two never drift:
// the closure the verb freezes MUST be the closure the executor guards against.
namespace offsetface {

// G1 angular tolerance for `BRepLib::ContinuityOfFaces` (radians). 1e-4 rad ≈ 0.006°.
// Deliberately TIGHT: a 0.5–1° tolerance would swallow a 0.75° draft and chain two
// faces the user meant to keep independent.
inline constexpr double kTangentAngleTol = 1.0e-4;

// Floor for the `BRepOffset_MakeOffset::Initialize` construction tolerance. The
// effective tolerance is `max(Precision::Confusion(), maxShapeTolerance, thisFloor)`
// (see `build_tolerance`) — the floor is the value the Phase-0 spike characterized
// every kernel behaviour at, so lowering it silently re-opens un-spiked territory.
inline constexpr double kBuildToleranceFloor = 1.0e-4;

// The semantic measurement tolerances — independent of OCCT's construction
// tolerance, because they answer whether the requested model change was achieved —
// now come from the precision context: `GeometryPrecisionContext::semantic_length()`
// and `::semantic_angular()`. The values are unchanged (1e-6 mm / 1e-7 rad); they
// are no longer restated here.

// Minimum authored geometric change now comes from the precision context —
// `GeometryPrecisionContext::authoring_resolution()`. A smaller non-zero request
// is not an identity: it is refused because the kernel cannot honestly prove it at
// the supported feature resolution. The value is unchanged (1e-3 mm); it is now
// the SAME number Extrude, Fillet, Hole and Shell refuse below, stated once.

// `|n_out · r̂|` must reach this for a cylindrical σ to be derivable at all.
inline constexpr double kRadialDotMin = 1.0 - 1.0e-6;

// Relative tolerance for the Total material-column / footprint-coverage volumes.
inline constexpr double kColumnRelTol = 1.0e-6;

// Minimum publishable volume now comes from the precision context —
// `GeometryPrecisionContext::minimum_volume()`, which is the authoring resolution
// CUBED. That derives the 1e-9 mm³ this constant used to assert, and makes its old
// "a 1 µm cube" comment arithmetic rather than a claim. The build tolerance is a
// LENGTH, so it is the wrong yardstick here; a face driven onto its opposite
// collapses to ~0 and would otherwise pass IsDone + BRepCheck (spike trap b).

enum class SurfaceKind { Plane, Cylinder, Other };
SurfaceKind surface_kind(const TopoDS_Face& face);

// A deterministic geometric sample proven to lie inside the TRIMMED face domain:
// the surface point, the ORIENTATION-CORRECTED outward normal, and its (u,v). This
// is both the σ probe and the `anchor` the authoring verb returns. No sample means
// UNKNOWN, which committed modeling treats as refusal.
struct FaceSample {
    bool ok = false;
    gp_Pnt point{0.0, 0.0, 0.0};
    gp_Dir normal{0.0, 0.0, 1.0};
    double u = 0.0;
    double v = 0.0;
};
FaceSample sample_face(const TopoDS_Face& face);

struct PlaneInfo {
    bool ok = false;
    gp_Pnt location{0.0, 0.0, 0.0};
    gp_Dir normal{0.0, 0.0, 1.0};
};
PlaneInfo plane_info(const TopoDS_Face& face);

// Cylindrical face geometry + the SIGN CONVENTION the absolute distance types need.
// `sigma = sign(n_out · r̂)` where `r̂` is the axis→sample unit radial: +1 for a boss
// (material inside the cylinder), −1 for a hole. Derived GEOMETRICALLY — never from
// FORWARD/REVERSED, which does not survive a boolean.
struct CylinderInfo {
    bool ok = false;
    gp_Ax1 axis;
    double radius = 0.0;
    int sigma = 0;
};
CylinderInfo cylinder_info(const TopoDS_Face& face);

// G1 tangent closure of `seed_ordinals` over `body` (1-based `TopExp::MapShapes`
// face ordinals). BFS across edges with exactly two DISTINCT adjacent faces, a
// neighbour joining iff `BRepLib::ContinuityOfFaces(edge, f1, f2, kTangentAngleTol)
// >= GeomAbs_G1`. Seam self-adjacency (both ancestors the same face) is skipped; an
// edge with >2 distinct adjacent faces is NON-MANIFOLD and fails the walk.
// `ordinals` is sorted ascending — determinism, no pointer/hash iteration.
struct ClosureResult {
    bool ok = false;
    bool non_manifold = false;
    std::vector<int> ordinals;
};
ClosureResult tangent_closure(const TopoDS_Shape& body, const std::vector<int>& seed_ordinals);

// `max(Precision::Confusion(), BRep_Tool::MaxTolerance(shape, FACE|EDGE|VERTEX),
// kBuildToleranceFloor)` — the named construction tolerance (never a bare literal).
double build_tolerance(const TopoDS_Shape& shape);

// --- V3 blend-aware classification (ops/OffsetFaceBlend.cpp) ----------------

// One CERTIFIED blend the walk stopped at: `blend` has passed
// `kernel::fillet::certify_blend`'s layers 1-3, so it is an exact `GeomAbs_Cylinder`
// with two plane-or-cylinder supports, an analytic tangency proof, a known convexity
// and two equal-length straight boundary edges.
struct ClosureBlend {
    int ordinal = 0;  // 1-based face ordinal in the CLASSIFIED body
    kernel::fillet::RecognizedBlend blend;
};

// `tangent_closure`'s BFS, except a certified blend whose two supports include the
// face the walk entered from is recorded in `blends` and NOT TRAVERSED THROUGH.
// The three sets are disjoint and, together, are exactly the full G1 closure of
// `seed_ordinals`:
//
//   moving  C_op — the design faces the offset moves (always includes the seeds)
//   blends  B    — suppressed before the offset and rebuilt after it
//   fixed   F    — reachable only THROUGH a blend; held still, which is the whole
//                  point of V3 and the thing V2 cannot express
//
// FAIL-CLOSED BY NAME. A G1 neighbour the sampled recognizer calls a blend but the
// proof stack refuses is never quietly treated as a design face — the walk stops
// with the condition's own code, because "offset it anyway" is precisely V2's
// destructive behaviour. `code` is empty only for the two structural walk failures
// (non-manifold / unusable seed), which the executor reports in V2's wording.
struct BlendAwareClosure {
    bool ok = false;
    std::string code;     // OFFSET_FACE_* refusal code; empty ⇒ plain OP_FAILED
    std::string message;  // measured detail, always populated on a refusal
    std::vector<int> moving;
    std::vector<ClosureBlend> blends;
    std::vector<int> fixed;

    // moving ⊎ blends ⊎ fixed, ascending. The executor requires this to equal the
    // stored operative set EXACTLY, in both directions.
    std::vector<int> ordinals() const;
};

BlendAwareClosure blend_aware_closure(const TopoDS_Shape& body,
                                      const std::vector<int>& seed_ordinals);

// "f:N" for a 1-based face ordinal.
std::string face_topokey(int ordinal);

// OCCT history adapter over the offset IMAGES (spike fact 2). `Modified(S)` answers
// from `OffsetFacesFromShapes()` (faces) / `OffsetEdgesFromShapes()` (edges), filtered
// to S's own `TopAbs` type AND to entries actually present in the result shape,
// deduped by `IsSame`, and sorted by RESULT-SHAPE ORDINAL so the list a split hands
// the confidence gate is deterministic. `Generated` is always empty.
//
// `IsDeleted(S)` is deliberately conservative: true only on POSITIVE evidence (S had
// an image and none of it survives in the result). "No image and not in the result"
// returns false so `ElementMapPartition::apply_history` reaches its no-successor
// branch and emits NeedsRepair — an honest "cannot rebind" beats a silent drop.
class OffsetImageHistory : public BRepBuilderAPI_MakeShape {
public:
    OffsetImageHistory(const BRepAlgo_Image& face_image, const BRepAlgo_Image& edge_image,
                       const TopoDS_Shape& result);

    const TopTools_ListOfShape& Modified(const TopoDS_Shape& S) override;
    const TopTools_ListOfShape& Generated(const TopoDS_Shape& S) override;
    Standard_Boolean IsDeleted(const TopoDS_Shape& S) override;

private:
    // Successors of `S` that survive in the result, ordinal-sorted. `had_image`
    // reports whether the offset image knew `S` at all (deletion evidence).
    void collect(const TopoDS_Shape& S, TopTools_ListOfShape& out, bool& had_image) const;
    const TopTools_IndexedMapOfShape* result_map(TopAbs_ShapeEnum kind) const;

    const BRepAlgo_Image& face_image_;
    const BRepAlgo_Image& edge_image_;
    TopTools_IndexedMapOfShape result_faces_;
    TopTools_IndexedMapOfShape result_edges_;
    TopTools_IndexedMapOfShape result_vertices_;
    TopTools_ListOfShape cache_;  // storage backing the `Modified` reference
    TopTools_ListOfShape empty_;
};

}  // namespace offsetface

OpOutcome execute_offset_face(OpContext& ctx, const nlohmann::json& op, const std::string& op_id);

}  // namespace onecad::ops
