// Ladder.cpp — see Ladder.h. Descriptor+anchor stage: score → assign → gate.
#include "elementmap/Ladder.h"

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <numeric>

#include <BRepBndLib.hxx>
#include <BRepBuilderAPI_MakeVertex.hxx>
#include <BRepExtrema_DistShapeShape.hxx>
#include <Bnd_Box.hxx>
#include <GeomAbs_CurveType.hxx>
#include <GeomAbs_SurfaceType.hxx>
#include <TopAbs_ShapeEnum.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>

#include "elementmap/Assignment.h"
#include "elementmap/ElementMapPartition.h"

namespace onecad::elementmap {

namespace {

TopAbs_ShapeEnum topabs_of(km::ElementKind kind) {
    switch (kind) {
        case km::ElementKind::Face: return TopAbs_FACE;
        case km::ElementKind::Edge: return TopAbs_EDGE;
        case km::ElementKind::Vertex: return TopAbs_VERTEX;
        default: return TopAbs_SHAPE;
    }
}

char topokey_prefix(km::ElementKind kind) {
    switch (kind) {
        case km::ElementKind::Face: return 'f';
        case km::ElementKind::Edge: return 'e';
        case km::ElementKind::Vertex: return 'v';
        default: return '?';
    }
}

double body_diagonal(const TopoDS_Shape& body_shape) {
    if (body_shape.IsNull()) return 1.0;
    Bnd_Box box;
    BRepBndLib::Add(body_shape, box);
    if (box.IsVoid()) return 1.0;
    Standard_Real xmin, ymin, zmin, xmax, ymax, zmax;
    box.Get(xmin, ymin, zmin, xmax, ymax, zmax);
    const double dx = xmax - xmin, dy = ymax - ymin, dz = zmax - zmin;
    const double diag = std::sqrt(dx * dx + dy * dy + dz * dz);
    return diag > 1e-9 ? diag : 1.0;
}

// Distance from a world point to a candidate SUB-SHAPE (not to its centroid) —
// the measure behind the veto's anchor-exact carve-out (SCHEMA §10).
//
// Shape distance, not centroid distance, is what "the element did not move" means
// here. A parametric edit routinely slides a feature ALONG ITSELF: growing an
// extrude's depth moves every vertical edge's midpoint by half the delta while the
// edge still passes exactly through its stored anchor. Centroid distance would call
// that a move and veto the flagship gesture (a fillet surviving a depth change);
// shape distance correctly reports 0. Falls back to the centroid when OCCT's
// extrema solver does not converge.
double distance_to_shape(const gp_Pnt& p, const TopoDS_Shape& shape, const gp_Pnt& fallback) {
    if (shape.IsNull()) return p.Distance(fallback);
    try {
        BRepExtrema_DistShapeShape dist(BRepBuilderAPI_MakeVertex(p).Vertex(), shape);
        if (dist.IsDone() && dist.NbSolution() > 0) return dist.Value();
    } catch (const Standard_Failure&) {
        // fall through to the centroid approximation
    }
    return p.Distance(fallback);
}

std::string surface_type_name(GeomAbs_SurfaceType t) {
    switch (t) {
        case GeomAbs_Plane: return "planar";
        case GeomAbs_Cylinder: return "cylindrical";
        case GeomAbs_Cone: return "conical";
        case GeomAbs_Sphere: return "spherical";
        case GeomAbs_Torus: return "toroidal";
        default: return "curved";
    }
}

std::string curve_type_name(GeomAbs_CurveType t) {
    switch (t) {
        case GeomAbs_Line: return "line";
        case GeomAbs_Circle: return "circular";
        case GeomAbs_Ellipse: return "elliptical";
        default: return "spline";
    }
}

std::string candidate_summary(km::ElementKind kind, const km::ElementDescriptor& d) {
    char buf[96];
    if (kind == km::ElementKind::Face) {
        std::snprintf(buf, sizeof(buf), "%s face, area~%.0fmm2", surface_type_name(d.surfaceType).c_str(),
                      d.magnitude);
    } else if (kind == km::ElementKind::Edge) {
        std::snprintf(buf, sizeof(buf), "%s edge, len~%.1fmm", curve_type_name(d.curveType).c_str(),
                      d.magnitude);
    } else {
        std::snprintf(buf, sizeof(buf), "vertex at (%.1f,%.1f,%.1f)", d.center.X(), d.center.Y(),
                      d.center.Z());
    }
    return std::string(buf);
}

// The enumerated candidate pool for one element kind of a body.
struct CandidatePool {
    std::vector<TopoDS_Shape> shapes;
    std::vector<std::string> topo_keys;
    std::vector<km::ElementDescriptor> descriptors;
};

CandidatePool enumerate_candidates(const TopoDS_Shape& body_shape, km::ElementKind kind) {
    CandidatePool pool;
    const TopAbs_ShapeEnum type = topabs_of(kind);
    if (body_shape.IsNull() || type == TopAbs_SHAPE) return pool;
    TopTools_IndexedMapOfShape map;
    TopExp::MapShapes(body_shape, type, map);
    const char prefix = topokey_prefix(kind);
    for (int i = 1; i <= map.Extent(); ++i) {
        pool.shapes.push_back(map(i));
        pool.topo_keys.push_back(std::string(1, prefix) + ":" + std::to_string(i));
        pool.descriptors.push_back(ElementMapPartition::describe(map(i), body_shape));
    }
    return pool;
}

}  // namespace

nlohmann::json LadderResolution::to_needs_repair_json() const {
    nlohmann::json cands = nlohmann::json::array();
    for (const LadderCandidate& c : candidates) {
        nlohmann::json contrib = nlohmann::json::object();
        for (const auto& [k, v] : c.contributions) contrib[k] = v;
        cands.push_back(nlohmann::json{
            {"topoKey", c.topo_key},
            {"score", c.score},
            {"margin", c.margin},
            {"worldPos", {c.world_pos.X(), c.world_pos.Y(), c.world_pos.Z()}},
            {"summary", c.summary},
            {"featureContributions", std::move(contrib)},
        });
    }
    return nlohmann::json{
        {"refId", ref_id},
        {"elementId", element_id},
        {"ladderFailed", "descriptor"},  // this stage (history handled upstream)
        {"reason", reason},
        {"scoringVersion", kResolverVersion},
        {"candidates", std::move(cands)},
        {"anchor", anchor_json.is_null() ? nlohmann::json::object() : anchor_json},
        {"uiLabel", ui_label},
    };
}

std::vector<LadderResolution> resolve_descriptor_stage(const TopoDS_Shape& body_shape,
                                                       const std::string& body_id,
                                                       const std::vector<LadderRef>& refs,
                                                       const LadderEditContext& edit) {
    (void)body_id;  // evidence label only
    std::vector<LadderResolution> out(refs.size());
    const double body_diag = body_diagonal(body_shape);

    // Group ref indices by element kind (disjoint candidate pools).
    std::map<km::ElementKind, std::vector<std::size_t>> by_kind;
    for (std::size_t i = 0; i < refs.size(); ++i) by_kind[refs[i].kind].push_back(i);

    for (const auto& [kind, idxs] : by_kind) {
        const CandidatePool pool = enumerate_candidates(body_shape, kind);
        const int c = static_cast<int>(pool.shapes.size());
        const int n = static_cast<int>(idxs.size());

        // Score matrix + kept per-candidate evidence, per ref of this kind.
        std::vector<std::vector<double>> score(n, std::vector<double>(std::max(c, 1), 0.0));
        // The same matrix with the `anchor` feature excluded — the space the
        // edit-scoped tie veto compares in (SCHEMA §10).
        std::vector<std::vector<double>> desc_score(n, std::vector<double>(std::max(c, 1), 0.0));
        std::vector<char> anchor_scored(n, 0);  // the anchor feature contributed at all
        std::vector<std::vector<std::map<std::string, double>>> contribs(n);
        // Pick-to-sub-shape distances per (ref, candidate), kept for the veto's
        // carve-out and the v4 anchor-decisive tie-break.
        std::vector<std::vector<double>> shape_dists(n);
        for (int i = 0; i < n; ++i) {
            const LadderRef& r = refs[idxs[i]];
            contribs[i].resize(std::max(c, 0));
            // v4: the anchor is measured to each candidate SUB-SHAPE, and scored
            // relative to the nearest rival (Scoring.h). Distances are computed once
            // per (ref, candidate).
            std::vector<double>& dists = shape_dists[i];
            dists.assign(static_cast<std::size_t>(std::max(c, 0)), -1.0);
            if (r.anchor.has_world_point) {
                for (int j = 0; j < c; ++j)
                    dists[j] = distance_to_shape(r.anchor.world_point, pool.shapes[j],
                                                 pool.descriptors[j].center);
            }
            for (int j = 0; j < c; ++j) {
                AnchorEvidence anchor = r.anchor;
                if (anchor.has_world_point) anchor.shape_distance = dists[j];
                const ScoreResult s = score_candidate(r.descriptor, r.has_descriptor, anchor,
                                                      pool.descriptors[j], body_diag);
                score[i][j] = s.score;
                desc_score[i][j] = s.descriptor_score;
                contribs[i][j] = s.contributions;
                if (s.has_anchor_feature) anchor_scored[i] = 1;
            }
        }

        // Optimal distinct assignment (pad columns to ≥ n with dummy score-0 cols so
        // it is always solvable; a ref landing on a dummy has no real candidate).
        std::vector<int> assignment;
        if (n > 0) {
            const int cols = std::max(n, c);
            std::vector<std::vector<double>> cost(n, std::vector<double>(cols, 1.0));
            for (int i = 0; i < n; ++i)
                for (int j = 0; j < c; ++j) cost[i][j] = 1.0 - score[i][j];
            assignment = min_cost_assignment(cost);
        }

        for (int i = 0; i < n; ++i) {
            LadderResolution& res = out[idxs[i]];
            const LadderRef& r = refs[idxs[i]];
            res.ref_id = r.ref_id;
            res.element_id = r.element_id;
            res.kind = kind;
            res.anchor_json = r.anchor_json;
            res.ui_label = r.ui_label.empty() ? ("unresolved ref " + r.ref_id) : r.ui_label;
            res.ladder_level = "descriptor";

            // Ranked candidate evidence (real candidates only, desc by score).
            std::vector<int> order(c);
            std::iota(order.begin(), order.end(), 0);
            std::sort(order.begin(), order.end(), [&](int a, int b) {
                if (score[i][a] != score[i][b]) return score[i][a] > score[i][b];
                return a < b;  // deterministic tie-break
            });
            const int keep = std::min(c, 5);
            for (int k = 0; k < keep; ++k) {
                const int j = order[k];
                LadderCandidate cand;
                cand.topo_key = pool.topo_keys[j];
                cand.shape = pool.shapes[j];
                cand.score = score[i][j];
                cand.margin = (k + 1 < c) ? (score[i][j] - score[i][order[k + 1]]) : score[i][j];
                cand.world_pos = pool.descriptors[j].center;
                cand.summary = candidate_summary(kind, pool.descriptors[j]);
                cand.contributions = contribs[i][j];
                res.candidates.push_back(std::move(cand));
            }

            const int aj = assignment.empty() ? -1 : assignment[i];
            if (c == 0 || aj < 0 || aj >= c) {
                res.outcome = LadderOutcome::NeedsRepair;
                res.reason = "no-candidates";
                continue;
            }
            const double assigned = score[i][aj];
            double runner_up = 0.0;
            for (int j = 0; j < c; ++j)
                if (j != aj) runner_up = std::max(runner_up, score[i][j]);
            const double margin = assigned - runner_up;

            // EDIT-SCOPED DESCRIPTOR-TIE VETO (SCHEMA §10, resolverVersion 2).
            //
            // Compare the assigned candidate against its best rival in
            // DESCRIPTOR-ONLY space (the anchor feature removed). When that
            // separation is ~0 the descriptor cannot tell them apart and the ANCHOR
            // is what produced the blended margin below. On a no-edit replay that is
            // correct — the anchor sits exactly where it was authored. After an
            // upstream edit it is not: the geometry moved, so a congruent twin can
            // sit closer to the stale anchor than the real element and would bind
            // silently and wrongly (H5-B / review finding B3).
            //
            // The comparison is SIGNED, so it also fires when the anchor overrode a
            // descriptor-BETTER rival (`desc_rival > desc_assigned` ⇒ negative ⇒
            // below epsilon) — an even clearer case of the anchor deciding.
            //
            // An anchor-only ref (no frozen descriptor) has descriptor score 0 for
            // EVERY candidate, so it always ties — but the ANCHOR-EXACT carve-out
            // below still resolves the common case (a vertex pick whose element did
            // not move), so such a ref is not blanket-refused post-edit.
            //
            // ANCHOR-EXACT CARVE-OUT: the veto fires only when the winner is NOT
            // still sitting on the stored anchor. An element within
            // `kAnchorExactEps * scale` of its anchor demonstrably did not move, so
            // the edit never made ITS anchor stale and there is nothing to distrust —
            // that covers ~all real edits. What remains uncaught is the TELEPORT
            // residual (an edit that parks an exact congruent twin precisely at the
            // stale anchor): locally undecidable, accepted and documented by the
            // HISTORY-HARDEN H6a decision, reserved for the future from-0 history
            // rung. The veto keeps catching the DRIFT class — a twin merely NEARER to
            // the stale anchor than the moved original.
            bool anchor_decided_a_tie = false;
            if (edit.post_upstream_edit && c >= 2 && anchor_scored[i]) {
                bool has_rival = false;
                double desc_rival = 0.0;
                for (int j = 0; j < c; ++j) {
                    if (j == aj) continue;
                    if (!has_rival || desc_score[i][j] > desc_rival) {
                        desc_rival = desc_score[i][j];
                        has_rival = true;
                    }
                }
                const bool descriptor_tie =
                    has_rival && (desc_score[i][aj] - desc_rival) < kDescriptorTieEpsilon;
                // Same scale the `anchor` similarity feature used, from one source.
                const double winner_anchor_dist = distance_to_shape(
                    r.anchor.world_point, pool.shapes[aj], pool.descriptors[aj].center);
                const bool anchor_exact =
                    winner_anchor_dist <= kAnchorExactEps * anchor_scale(body_diag);
                // From-0 replay with an edit context has no migrated partition, so a
                // stored anchor can be stale. The anchor-exact carve-out that keeps
                // legitimate "element did not move" resolutions working on a checkpoint
                // replay would here bless a congruent decoy parked at the stale anchor
                // (VF-M5). On this path we require descriptor evidence to separate the
                // candidates; otherwise it is NeedsRepair.
                const bool allow_anchor_exact = !edit.from_zero_replay;
                anchor_decided_a_tie = descriptor_tie && (!anchor_exact || !allow_anchor_exact);
            }

            // ANCHOR-DECISIVE tie-break (v4, Scoring.h): the top two candidates tie
            // in descriptor space, the pick lies ON the winner, and every rival is
            // an order of magnitude farther — the pick names the element. This is
            // what lets an anchor-only face pick (the Hole seat) and a pick on one
            // of two same-facing twins resolve without a body-diagonal margin.
            bool anchor_decisive = false;
            if (!anchor_decided_a_tie && c >= 2 && anchor_scored[i] &&
                assigned >= kAutoBindMinScore && margin < kAutoBindMinMargin) {
                int runner = -1;
                for (int j = 0; j < c; ++j) {
                    if (j == aj) continue;
                    if (runner < 0 || score[i][j] > score[i][runner]) runner = j;
                }
                const bool descriptor_tie =
                    runner >= 0 && std::abs(desc_score[i][aj] - desc_score[i][runner]) <
                                       kDescriptorTieEpsilon;
                const double winner_d = shape_dists[i][aj];
                double rival_d = -1.0;
                for (int j = 0; j < c; ++j) {
                    if (j == aj) continue;
                    if (rival_d < 0.0 || shape_dists[i][j] < rival_d) rival_d = shape_dists[i][j];
                }
                // "ON the winner" means on it: within the 0.01 mm separation floor, NOT
                // the veto's carve-out band (0.05 × 0.5 × bodyDiag ≈ 3.5 mm on a 100 mm
                // plate). Adversarial review 2026-09-02: with the wide band a pick left
                // 3 mm from a congruent twin after an upstream edit would have bound to
                // the twin below the margin. Both WP-A probes have winner_d == 0.
                const bool winner_exact = winner_d >= 0.0 && winner_d <= kAnchorMinSeparationMm;
                const bool rivals_far =
                    rival_d >= kAnchorDecisiveRatio * std::max(winner_d, kAnchorMinSeparationMm);
                anchor_decisive = descriptor_tie && winner_exact && rivals_far;
            }

            if (anchor_decided_a_tie) {
                res.outcome = LadderOutcome::NeedsRepair;
                res.reason = "ambiguous";  // no new §9 token
                res.score = assigned;
                res.margin = margin;
            } else if (assigned >= kAutoBindMinScore &&
                       (margin >= kAutoBindMinMargin || anchor_decisive)) {
                res.outcome = LadderOutcome::AutoBind;
                res.bound_shape = pool.shapes[aj];
                res.bound_topo_key = pool.topo_keys[aj];
                res.score = assigned;
                res.margin = margin;
            } else {
                res.outcome = LadderOutcome::NeedsRepair;
                res.reason = (margin < kAutoBindMinMargin) ? "ambiguous" : "low-confidence";
                res.score = assigned;
                res.margin = margin;
            }
        }
    }
    return out;
}

LadderRef ladder_ref_from_input(const nlohmann::json& input, const std::string& ref_id) {
    LadderRef r;
    r.ref_id = ref_id;
    if (input.contains("primary") && input["primary"].is_object()) {
        const nlohmann::json& pr = input["primary"];
        if (pr.contains("elementId") && pr["elementId"].is_string())
            r.element_id = pr["elementId"].get<std::string>();
        if (pr.contains("kind") && pr["kind"].is_string())
            r.kind = ElementMapPartition::kind_from_name(pr["kind"].get<std::string>());
    }
    // Frozen intent.descriptor (structured object → parsed evidence; a string
    // placeholder or absence ⇒ anchor-only).
    if (input.contains("intent") && input["intent"].is_object()) {
        const nlohmann::json& intent = input["intent"];
        if (r.kind == km::ElementKind::Unknown && intent.contains("kind") && intent["kind"].is_string())
            r.kind = ElementMapPartition::kind_from_name(intent["kind"].get<std::string>());
        if (intent.contains("descriptor") && intent["descriptor"].is_object()) {
            r.descriptor = ElementMapPartition::descriptor_from_json(intent["descriptor"]);
            r.has_descriptor = true;
        }
    }
    // Anchor (world point narrows a descriptor tie / is the sole evidence otherwise).
    if (input.contains("anchor") && input["anchor"].is_object()) {
        r.anchor_json = input["anchor"];
        const nlohmann::json& a = input["anchor"];
        if (a.contains("worldPoint") && a["worldPoint"].is_array() && a["worldPoint"].size() >= 3) {
            const nlohmann::json& wp = a["worldPoint"];
            if (wp[0].is_number() && wp[1].is_number() && wp[2].is_number()) {
                r.anchor.has_world_point = true;
                r.anchor.world_point =
                    gp_Pnt(wp[0].get<double>(), wp[1].get<double>(), wp[2].get<double>());
            }
        }
    }
    return r;
}

}  // namespace onecad::elementmap
