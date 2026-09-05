// GearOp.h — the SCHEMA §7.3 `Gear` operation (Gear Generator G1).
//
// A generated gear body. Unlike every other feature op in this directory it
// has NO HOST: it mints `body_<opId>` (decision D1) from a typed recipe block
// and a placement, contributing one `created` body event and an EMPTY
// element-map delta (nothing pre-existing is tracked, so `apply_history` is
// not involved at all).
//
// PLACEMENT is exactly one of two modes, validated both ways:
//
//   face   Hole's semantics, reused rather than reinvented — the frozen
//          world `point` is re-projected onto the resolved face each regen and
//          refused past 1e-3 mm, the gear's axis is the face's INWARD normal
//          (so the body grows into material), and the ref resolves through the
//          §10 ladder with a `NeedsRepair` (never a wrong bind) when it no
//          longer binds.
//   frame  An explicit frozen `{origin, axis, xDir}`. No refs, no
//          reprojection, no host. `xDir` fixes the angular PHASE, which is what
//          makes a pair of gears mesh.
//
// REFERENCEABILITY (SCHEMA §7.3). The element map is mint-on-demand — entries
// exist only for elements an op referenced or a user promoted — so a gear
// needs no suppression machinery to keep its teeth unreferenceable: it simply
// never mints for them. What it DOES need, and what `gear_body_op_ids` below
// exists for, is to stop a user promoting a tooth face by hand through
// `AcquireElementIds`. A tooth's identity changes with the tooth count, so
// such an id would mis-resolve after the next parameter edit — the exact
// silent-wrong-bind the ladder exists to prevent.
//
// Publication takes `single_solid_policy("Gear", TierB)` — the FULL audit
// including self-interference. A generated profile is precisely where a bad
// parameter combination yields a plausible-looking but self-intersecting
// solid, so this op does not take the lighter tier.

#pragma once

#include <map>
#include <optional>
#include <set>
#include <string>
#include <vector>

#include <NCollection_IndexedDataMap.hxx>
#include <NCollection_IndexedMap.hxx>
#include <NCollection_List.hxx>
#include <TopTools_ShapeMapHasher.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Ax1.hxx>
#include <nlohmann/json.hpp>

#include "ops/OpTypes.h"

namespace onecad::ops {

/// Execute one `Gear` op. Mints `body_<op_id>` on success; every failure is a
/// graded refusal (`OP_FAILED` / `GEOMETRY_INVALID` naming the parameter at
/// fault) or a `NeedsRepair` state for an unresolvable placement.
OpOutcome execute_gear(OpContext& ctx, const nlohmann::json& op, const std::string& op_id);

/// The op ids of every `Gear` operation in `plan`, i.e. the ops whose minted
/// bodies carry generated tooth geometry.
///
/// Used by `AcquireElementIds` to refuse promoting a gear's tooth face to a
/// persistent ElementId. Derived from the plan rather than stored on the body
/// because the body id IS `body_<opId>` (D1), so the mapping is total and needs
/// no extra state to survive a reopen.
std::set<std::string> gear_body_op_ids(const nlohmann::json& plan);

/// True when `body_id` was minted by a `Gear` op present in `plan`.
bool is_generated_gear_body(const nlohmann::json& plan, const std::string& body_id);

/// What the SCHEMA §7.3 referenceability classifier needs about ONE gear body
/// (kernel-hardening WP-I). Plan-derived and geometric — there is no per-body
/// state to persist, because D1 makes `body_<opId>` → `Gear` op total.
struct GearBodyInfo {
    /// The gear's own axis. Read from `placement.frame.axis` when the record
    /// carries an explicit frame; DERIVED from the body (the normal of its
    /// largest planar face — a cap) for the `placement.face` mode, whose axis is
    /// the resolved host face's inward normal and therefore not recoverable from
    /// params alone. Only the DIRECTION is used by the classifier.
    gp_Ax1 axis;
    /// `computed.rootDiameter / 2` — the same quantity the op's own bore-validity
    /// gate uses. A cylindrical face at or beyond it is tooth geometry.
    double root_radius = 0.0;
    /// The `Gear` op that minted the body; `evidence.gearOpId` on a refusal.
    std::string gear_op_id;
};

/// `GearBodyInfo` for `body_id` when the plan says it is a gear body, else
/// `nullopt`. `body_shape` is only consulted for the `placement.face` axis
/// derivation described above (pass a null shape to skip it, which then yields
/// `nullopt` for that mode rather than a guessed axis).
std::optional<GearBodyInfo> gear_body_info(const nlohmann::json& plan, const std::string& body_id,
                                           const TopoDS_Shape& body_shape);

/// Every live gear body of `bodies`, keyed by body id. Built once per publish
/// (`AcceptPrepared`) from the ACCEPTED plan, so it is rebuilt on every replay
/// and copied with the session by checkpoints.
std::map<std::string, GearBodyInfo> gear_body_infos(const nlohmann::json& plan,
                                                    const session::BodyStore& bodies);

class GearAdjacency;  // defined below; the O(1) form of the two lookups here

/// SCHEMA §7.3 gear referenceability: may an ElementId be minted for or bound
/// to `face` of gear body `body`?
///
/// Referenceable iff the face is PLANAR with its normal parallel to the gear
/// axis (a cap — there are always exactly two, and their identity does not
/// depend on `teeth`), or CYLINDRICAL with its axis PARALLEL to the gear axis
/// (not necessarily coaxial: `offsetHole` is a parallel cylinder at a radial
/// offset) and of radius strictly below the ROOT radius. Everything else —
/// every flank, tip, root land, fillet and hub chamfer — is refused, because a
/// tooth's identity changes with the tooth count and a descriptor bound to
/// "tooth 7's flank" is the silent-wrong-bind the §10 ladder exists to prevent.
///
/// A `face` that is not part of `body` is refused: the classifier answers about
/// a face OF this gear body, never about a stray shape. A NULL `body` skips that
/// membership test, which is how a unit probe pins the geometric rule alone.
///
/// `adjacency`, when given, must have been built from the SAME `body`; it turns
/// the membership test and the repeat-face case into O(1) lookups.
bool gear_face_referenceable(const GearBodyInfo& info, const TopoDS_Shape& body,
                             const TopoDS_Shape& face,
                             const GearAdjacency* adjacency = nullptr);

/// One body's topology, gathered ONCE, so classifying a whole candidate pool is
/// linear rather than quadratic: the face map (the body-membership test), the
/// face ancestors of every edge and vertex, and a memo of the per-face verdict.
///
/// Without it `gear_face_referenceable` re-walks the body on every call and
/// `gear_element_referenceable` calls that once per adjacent face, so a pool
/// sweep is O(pool x faces) — on a 400-tooth gear (the §7.3 bound) that is
/// thousands of full traversals per step, which is exactly what the bound
/// exists to prevent. A caller that classifies more than one sub-element of a
/// body MUST reuse one of these.
///
/// The verdict memo is keyed by face ordinal and is valid only for the ONE
/// `GearBodyInfo` the adjacency is used with — its lifetime is one body, one
/// step, which is the only way it is constructed.
class GearAdjacency {
public:
    explicit GearAdjacency(const TopoDS_Shape& body);
    /// The faces adjacent to `shape` (an edge or a vertex of the body), or null
    /// when it is neither, or is not part of the body.
    const NCollection_List<TopoDS_Shape>* faces_of(const TopoDS_Shape& shape) const;
    /// 1-based ordinal of `face` in the body's face map; 0 when it is not one of
    /// them. `IsSame`, i.e. orientation-blind, like the element map's own lookup.
    int face_index(const TopoDS_Shape& face) const;
    /// The memoized verdict for `face_index`: -1 unknown, 0 refused, 1 allowed.
    signed char face_verdict(int index) const;
    void set_face_verdict(int index, bool allowed) const;

private:
    using Ancestors =
        NCollection_IndexedDataMap<TopoDS_Shape, NCollection_List<TopoDS_Shape>,
                                   TopTools_ShapeMapHasher>;
    NCollection_IndexedMap<TopoDS_Shape, TopTools_ShapeMapHasher> faces_;
    Ancestors edge_faces_;
    Ancestors vertex_faces_;
    mutable std::vector<signed char> face_verdict_;
};

/// SCHEMA §7.3 gear referenceability for ANY sub-element of a gear body.
///
/// A FACE takes `gear_face_referenceable`'s rule. An EDGE or a VERTEX is
/// referenceable iff EVERY face adjacent to it is — so a bore rim, a bore-cap
/// circle and a cap-counterbore circle stay referenceable, while every edge of
/// the tooth profile (INCLUDING the cap's outer boundary edges, which are shared
/// with the flanks) and every vertex on them are refused. An element adjacent to
/// no face at all, or of any other shape type, is refused: the rule answers
/// about a sub-element OF this gear body and never guesses.
///
/// `adjacency`, when given, must have been built from the SAME `body`; a null
/// one builds a throwaway map (fine for a single Bind, ruinous for a pool).
bool gear_element_referenceable(const GearBodyInfo& info, const TopoDS_Shape& body,
                                const TopoDS_Shape& shape,
                                const GearAdjacency* adjacency = nullptr);

}  // namespace onecad::ops
