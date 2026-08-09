// Ladder.h — the resolution-ladder DESCRIPTOR stage (W-WP6, SCHEMA §10).
//
// The full ladder is (SCHEMA §10):
//   1. OCCT history   — Modified/Generated/IsDeleted of the step's live builders.
//                       Handled in ElementMapPartition::apply_history (rebinds
//                       already-tracked elements after an op mutates their body;
//                       a UNIQUE image auto-binds, a scored SPLIT disambiguates).
//   2. Descriptor+anchor matching  — THIS module. Resolves a set of semantic refs
//                       against a body's sub-shapes when there is no history to
//                       consult (a FIRST-SEEN reference, or a repair dry-run):
//                       score every candidate (Scoring.h), pick distinct bindings
//                       with min-cost assignment (Assignment.h), gate on confidence.
//   3. Confidence gate → NeedsRepair  — auto-bind iff score ≥ 0.85 AND margin ≥ 0.10;
//                       otherwise emit NeedsRepair STATE with full typed evidence.
//
// The worker returns FULL evidence per resolution (candidates, per-field
// contributions, score, margin, ladder level, scoringVersion) so the policy can
// later move to Rust. A symmetric tie (equal top scores, margin < 0.10) MUST yield
// NeedsRepair — never a guess (false positive is strictly worse than false negative).
#pragma once

#include <map>
#include <string>
#include <vector>

#include <TopoDS_Shape.hxx>
#include <gp_Pnt.hxx>

#include "elementmap/Scoring.h"
#include "kernel/elementmap/ElementMap.h"
#include "nlohmann/json.hpp"

namespace onecad::elementmap {

namespace km = onecad::kernel::elementmap;

// One semantic ref to resolve (built from an op input's {primary, intent, anchor},
// SCHEMA §7.3). `has_descriptor` false ⇒ anchor-only resolution.
struct LadderRef {
    std::string ref_id;        // "<opId>.input<i>" (or a repair refId)
    std::string element_id;    // echoed primary.elementId
    km::ElementKind kind = km::ElementKind::Unknown;
    bool has_descriptor = false;
    km::ElementDescriptor descriptor;  // frozen intent.descriptor (evidence)
    AnchorEvidence anchor;
    nlohmann::json anchor_json;  // echo for the NeedsRepair payload
    std::string ui_label;
};

// One scored candidate sub-shape (evidence for the NeedsRepair payload).
struct LadderCandidate {
    std::string topo_key;
    TopoDS_Shape shape;
    double score = 0.0;
    double margin = 0.0;  // this candidate's score − the next-best candidate's score
    gp_Pnt world_pos{0.0, 0.0, 0.0};
    std::string summary;
    std::map<std::string, double> contributions;
};

// Edit context for the descriptor stage (SCHEMA §7.2 `editedFrom` / §10 veto).
//
// The worker cannot tell a CLEAN REPLAY from a POST-EDIT regen by looking at the
// geometry: both hand the ladder a shape plus a set of frozen refs. On a clean
// replay (open-time replay, rollback, undo/redo) the geometry is byte-identical to
// the one the ref was authored against, so a stored anchor sits EXACTLY on its
// element and letting the anchor decide a descriptor tie is correct. After an
// UPSTREAM CONTENT EDIT the geometry moved, so the stored anchor is STALE and a
// congruent twin can sit closer to it than the real element — the anchor deciding
// there is a silent wrong bind (H5-B / review finding B3). Only Rust knows which
// case it is, so it says so in the plan and the worker gates the veto on it.
struct LadderEditContext {
    // True iff the plan carried `editedFrom = k` AND the step owning these refs is
    // strictly AFTER k. Default false ⇒ pre-v2 behaviour, byte for byte.
    bool post_upstream_edit = false;
    // True iff this replay runs on a RESTORED basis. Anchors are world-frozen at
    // ref-authoring time, so when the basis was restored rather than rebuilt the
    // anchor-exact carve-out can bless a stale anchor; disable it on that path (VF-M5).
    //
    // NEVER SET IN V1 — there is no checkpoint restore (see ScratchJob.h). The
    // behaviour below is exercised by `worker/tests/test_wp6_ladder.cpp`, which
    // constructs this context directly.
    bool from_zero_replay = false;
};

enum class LadderOutcome { AutoBind, NeedsRepair };

struct LadderResolution {
    std::string ref_id;
    std::string element_id;
    km::ElementKind kind = km::ElementKind::Unknown;
    LadderOutcome outcome = LadderOutcome::NeedsRepair;
    std::string ladder_level = "descriptor";  // the level that decided

    // AutoBind result.
    TopoDS_Shape bound_shape;
    std::string bound_topo_key;
    double score = 0.0;   // assigned candidate confidence
    double margin = 0.0;  // assigned − best alternative for this ref

    // NeedsRepair evidence.
    std::string reason;  // "ambiguous" | "no-candidates" | "low-confidence"
    std::vector<LadderCandidate> candidates;  // sorted desc by score
    nlohmann::json anchor_json;
    std::string ui_label;

    // SCHEMA §9 NeedsRepair payload (STATE, not error), stamped with scoringVersion.
    nlohmann::json to_needs_repair_json() const;
};

// Resolve `refs` against `body_shape` via the descriptor+anchor stage: score every
// candidate, assign distinct bindings optimally (min-cost), gate each on confidence.
// `body_id` labels the resolution (evidence only). Refs of different `kind` draw
// from disjoint candidate pools (a face ref never binds an edge).
//
// `edit` carries the EDIT-SCOPED tie veto (SCHEMA §10, resolverVersion 2). Its
// default — no edit context — reproduces the resolverVersion-1 policy exactly.
std::vector<LadderResolution> resolve_descriptor_stage(const TopoDS_Shape& body_shape,
                                                       const std::string& body_id,
                                                       const std::vector<LadderRef>& refs,
                                                       const LadderEditContext& edit = {});

// Build a LadderRef from one op input JSON object ({primary, intent, anchor},
// SCHEMA §7.3). `ref_id` names it. `kind_hint` is the resolved element kind. The
// frozen intent.descriptor is parsed when present (structured object); a string
// placeholder or absent descriptor ⇒ anchor-only.
LadderRef ladder_ref_from_input(const nlohmann::json& input, const std::string& ref_id);

}  // namespace onecad::elementmap
