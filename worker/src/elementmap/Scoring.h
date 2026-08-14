// Scoring.h — normalized [0,1] descriptor-match confidence (W-WP6, resolverVersion 3).
//
// ── Why a new score (not the OneCAD-CPP one) ─────────────────────────────────
// OneCAD-CPP `ElementMap::score()` (kernel/elementmap/ElementMap.h:639-672) is an
// UNBOUNDED, scale-dependent COST: it sums a raw center distance (mm) with fixed
// per-mismatch penalties (1000, 10, 5, …). Its magnitude depends on the model's
// absolute size and units, so it cannot express the locked confidence policy
// (auto-bind iff score ≥ 0.85 AND margin ≥ 0.10 — SCHEMA §10). This module is the
// redesign: a normalized [0,1] CONFIDENCE (higher = better) built from per-feature
// similarities in [0,1], each with a fixed weight. The 14-field DESCRIPTOR itself
// is ported verbatim (it is EVIDENCE, never identity — Invariant 2); only the
// scoring on top of it is new and versioned (`kResolverVersion`).
//
// ── Feature model ────────────────────────────────────────────────────────────
// Per candidate we compute a weighted sum of feature similarities, renormalized by
// the weight of the features whose evidence is actually present (so an anchor-only
// ref still yields a bounded [0,1] score). Weights (faces / edges):
//   type       0.20  surfaceType (face) / curveType (edge) exact match → 1 else 0
//   magnitude  0.25  area (face) / length (edge) relative closeness
//   direction  0.20  |normal·normal| (face) / |tangent·tangent| (edge)
//   anchor     0.25  world-point proximity to the candidate centre (narrowing)
//   adjacency  0.10  adjacencyHash exact match → 1 else 0
// `adjacency` is deliberately LOW-weight: the ported adjacency hash is all-or-nothing
// (ANY dimensional edit flips it to 0), so a higher weight would sink a
// topology-preserving small edit below the 0.85 gate and defeat the whole point of
// the ladder (a fillet edge surviving an upstream parameter change). Ambiguity is
// caught by the MARGIN gate (a symmetric twin ties regardless of weights), not by
// the absolute score, so lowering adjacency loses no safety. A vertex ref (or a ref
// with no frozen descriptor) scores on `anchor` alone.
#pragma once

#include <cstdint>
#include <map>
#include <string>

#include <gp_Pnt.hxx>

#include "kernel/elementmap/ElementMap.h"

namespace onecad::elementmap {

namespace km = onecad::kernel::elementmap;

// resolverVersion (SCHEMA §10 / handshake §13). Bump on any scoring change; it is
// stamped into every NeedsRepair evidence payload (`scoringVersion`).
//
// 3 — history candidates add explicit Modified-channel provenance confidence;
// Generated-only successors remain eligible, while generated side topology no
// longer makes a strong Modified successor spuriously ambiguous.
// 2 (H6a) — two scoring-policy changes: the EDIT-SCOPED descriptor-tie veto
// (`LadderEditContext`, gated on the plan's `editedFrom`) and the PROPORTIONAL
// anchor floor (the fixed 1.0 mm scale floor became a 1e-7 divide-by-zero guard).
// 1 (W-WP6) — the original normalized [0,1] confidence.
inline constexpr int kResolverVersion = 3;

// Locked confidence policy (SCHEMA §10). A false positive (silent wrong bind) is
// strictly worse than a false negative (asking the user), so BOTH must hold.
inline constexpr double kAutoBindMinScore = 0.85;   // best candidate confidence
inline constexpr double kAutoBindMinMargin = 0.10;  // best − runner-up

// Descriptor-only separation below which two candidates count as TIED for the
// edit-scoped veto (SCHEMA §10, resolverVersion 2). Deliberately an order of
// magnitude under `kAutoBindMinMargin`: the veto must fire only when the
// descriptor genuinely cannot tell the candidates apart and the ANCHOR is doing
// all the deciding, never merely because the descriptor evidence is weak. The
// congruent-twin case it targets separates by EXACTLY 0 (identical type,
// magnitude, direction and adjacency hash), so any small positive epsilon works;
// 0.02 absorbs float noise in the magnitude/direction similarities without
// reaching a case where 0.02 of descriptor separation is real evidence.
inline constexpr double kDescriptorTieEpsilon = 0.02;

// ANCHOR-EXACT carve-out for the edit-scoped veto (SCHEMA §10, resolverVersion 2).
// A winner lying within `kAnchorExactEps * anchor_scale(bodyDiag)` of the stored
// anchor — measured to the SUB-SHAPE, not to its centroid — is treated as NOT
// having moved, and its anchor is allowed to settle a descriptor tie even post-edit.
//
// Derivation: the veto exists because an edit can move geometry out from under a
// stored anchor. An element that is still sitting ON its anchor demonstrably did
// NOT move, so the anchor never went stale for it and there is nothing to distrust
// — which covers ~all real edits, where a parametric change either leaves a feature
// in place or slides it along its own axis. (Sliding along itself is exactly why the
// measure is to the shape: growing an extrude's depth moves every vertical edge's
// MIDPOINT by half the delta while the edge still passes through its stored anchor.)
// 0.05 is 5% of the anchor scale (itself half the body diagonal), i.e. 2.5% of the
// body's extent: far above float/tessellation noise, far below the separation of any
// two distinct features on the same body.
//
// What this deliberately does NOT catch is the TELEPORT residual: an edit that parks
// an EXACT congruent twin precisely at the stale anchor while moving the original
// away. That is locally undecidable — the worker sees two identical descriptors and
// one of them is exactly at the anchor — and is an accepted, documented residual of
// HISTORY-HARDEN H6a, reserved for the future from-0 history rung. The veto keeps
// catching the DRIFT class: a twin merely NEARER to the stale anchor than the moved
// original, which is genuinely suspicious.
inline constexpr double kAnchorExactEps = 0.05;

// The anchor-proximity scale (SCHEMA §10): half the body diagonal, floored only
// against divide-by-zero. Shared by the `anchor` similarity feature and by the
// veto's anchor-exact carve-out so the two can never drift apart.
double anchor_scale(double body_diag);

// Anchor evidence used to narrow a descriptor tie (SCHEMA §10 "anchor narrowing").
struct AnchorEvidence {
    bool has_world_point = false;
    gp_Pnt world_point{0.0, 0.0, 0.0};
};

// A normalized [0,1] confidence + the per-feature contributions that produced it
// (they sum to `score`). Reported verbatim as NeedsRepair `featureContributions`
// so a repair UI — and, later, a Rust-side policy — can inspect the evidence.
struct ScoreResult {
    double score = 0.0;
    std::map<std::string, double> contributions;  // named; Σ == score

    // The SAME confidence recomputed with the `anchor` feature EXCLUDED — the
    // "descriptor-only" score the edit-scoped tie veto compares (SCHEMA §10).
    // Renormalized over the descriptor weights alone, so it is directly
    // comparable across candidates. 0 when there is no descriptor evidence at
    // all (an anchor-only ref) — then EVERY candidate ties at 0, which is the
    // intended conservative reading: a stale anchor is the only evidence left.
    double descriptor_score = 0.0;
    // Whether the `anchor` feature contributed to `score`.
    bool has_anchor_feature = false;
    // Whether ANY non-anchor (descriptor) feature contributed to `score`.
    bool has_descriptor_features = false;
};

// Score one candidate against a frozen intent descriptor + anchor. When
// `has_intent_descriptor` is false only the `anchor` feature is available
// (anchor-only resolution). `body_diag` scales the anchor proximity feature
// (bbox diagonal of the body the candidate lives in). Deterministic; pure.
ScoreResult score_candidate(const km::ElementDescriptor& intent, bool has_intent_descriptor,
                            const AnchorEvidence& anchor, const km::ElementDescriptor& candidate,
                            double body_diag);

// --- test-only instrumentation ------------------------------------------------
// Number of score_candidate() evaluations since the last reset. The W-WP6
// calibration corpus asserts this stays 0 in the "history resolves everything"
// case — proving the descriptor stage is NEVER consulted when OCCT history alone
// rebinds every tracked element (SCHEMA §10 ladder level 1). Thread-safe.
std::uint64_t scoring_call_count();
void reset_scoring_call_count();

}  // namespace onecad::elementmap
