// test_wp6_ladder.cpp — W-WP6 resolution-ladder CALIBRATION CORPUS (scope A).
// White-box, in-process, real OCCT. Proves the locked scoring policy (SCHEMA §10):
//   (1) SYMMETRIC TIE  ⇒ NeedsRepair — two candidates tie, margin 0, never a guess.
//   (2) CONFIDENT      ⇒ AutoBind — a clear winner (score ≥ 0.85, margin ≥ 0.10).
//   (3) HISTORY RESOLVES EVERYTHING ⇒ the descriptor stage is NEVER consulted
//       (scoring_call_count() stays 0 when every tracked element rebinds via a
//       unique OCCT-history image).
//   (4) MIN-COST ASSIGNMENT beats greedy on the documented counterexample.
//   (5) SCORED SPLIT LINEAGE (closes review finding 2): a symmetric split of a
//       tracked face ⇒ NeedsRepair "ambiguous" (was an unscored Modified().First()).
// No framework: exit code == failure count.
#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

#include <BRepAlgoAPI_Cut.hxx>
#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS_Shape.hxx>
#include <gp_Pnt.hxx>

#include "elementmap/Assignment.h"
#include "elementmap/ElementMapPartition.h"
#include "elementmap/Ladder.h"
#include "elementmap/Scoring.h"

namespace em = onecad::elementmap;
namespace km = onecad::kernel::elementmap;

namespace {
int g_failures = 0;
void check(bool cond, const std::string& msg) {
    if (!cond) {
        std::fprintf(stderr, "FAIL: %s\n", msg.c_str());
        ++g_failures;
    }
}

// The face of `shape` whose bbox centre is nearest (cx,cy,cz).
TopoDS_Shape face_by_center(const TopoDS_Shape& shape, double cx, double cy, double cz) {
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(shape, TopAbs_FACE, faces);
    TopoDS_Shape best;
    double best_d2 = -1.0;
    for (int i = 1; i <= faces.Extent(); ++i) {
        const km::ElementDescriptor d = em::ElementMapPartition::describe(faces(i));
        const double dx = d.center.X() - cx, dy = d.center.Y() - cy, dz = d.center.Z() - cz;
        const double d2 = dx * dx + dy * dy + dz * dz;
        if (best_d2 < 0.0 || d2 < best_d2) { best_d2 = d2; best = faces(i); }
    }
    return best;
}

TopoDS_Shape edge_by_center(const TopoDS_Shape& shape, double cx, double cy, double cz) {
    TopTools_IndexedMapOfShape edges;
    TopExp::MapShapes(shape, TopAbs_EDGE, edges);
    TopoDS_Shape best;
    double best_d2 = -1.0;
    for (int i = 1; i <= edges.Extent(); ++i) {
        const km::ElementDescriptor d = em::ElementMapPartition::describe(edges(i));
        const double dx = d.center.X() - cx, dy = d.center.Y() - cy, dz = d.center.Z() - cz;
        const double d2 = dx * dx + dy * dy + dz * dz;
        if (best_d2 < 0.0 || d2 < best_d2) { best_d2 = d2; best = edges(i); }
    }
    return best;
}

em::LadderRef face_ref(const std::string& id, const TopoDS_Shape& intent_face, double ax, double ay,
                       double az) {
    em::LadderRef r;
    r.ref_id = id;
    r.element_id = id;
    r.kind = km::ElementKind::Face;
    r.has_descriptor = true;
    r.descriptor = em::ElementMapPartition::describe(intent_face);
    r.anchor.has_world_point = true;
    r.anchor.world_point = gp_Pnt(ax, ay, az);
    r.anchor_json = {{"worldPoint", {ax, ay, az}}};
    return r;
}

// (1) SYMMETRIC TIE — a 20×10×10-tall box; the two 10×10 end faces (z=±10) are
// descriptor twins and the anchor sits equidistant between them ⇒ tie ⇒ NeedsRepair.
void test_symmetric_tie() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(gp_Pnt(-5, -5, -10), 10.0, 10.0, 20.0).Shape();
    const TopoDS_Shape top = face_by_center(box, 0, 0, 10);  // z=+10 end face
    // Anchor at the body centre → equidistant to the +Z and −Z end faces.
    std::vector<em::LadderRef> refs{face_ref("el_sym", top, 0, 0, 0)};

    const auto res = em::resolve_descriptor_stage(box, "body", refs);
    check(res.size() == 1, "sym: one resolution");
    check(res[0].outcome == em::LadderOutcome::NeedsRepair, "sym: NeedsRepair (never a guess)");
    check(res[0].reason == "ambiguous", "sym: reason ambiguous");
    check(res[0].candidates.size() >= 2, "sym: >=2 candidates surfaced");
    if (res[0].candidates.size() >= 2) {
        // The load-bearing guarantee: the two best candidates TIE (margin < 0.10).
        const double s1 = res[0].candidates[0].score, s2 = res[0].candidates[1].score;
        check(s1 == s2, "sym: top-2 candidate scores are equal (a tie)");
        check((s1 - s2) < em::kAutoBindMinMargin, "sym: tie margin below policy margin");
    }
    // Evidence payload carries scoringVersion (SCHEMA §9).
    check(res[0].to_needs_repair_json().value("scoringVersion", -1) == em::kResolverVersion,
          "sym: NeedsRepair payload stamps scoringVersion");
}

// (2) CONFIDENT — a 10×20×30 box (all three face-pairs distinct areas); the anchor
// sits ON the z=0 face, far from its z=30 twin ⇒ a clear unique winner ⇒ AutoBind.
void test_confident_autobind() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(10.0, 20.0, 30.0).Shape();
    const TopoDS_Shape z0 = face_by_center(box, 5, 10, 0);
    std::vector<em::LadderRef> refs{face_ref("el_z0", z0, 5, 10, 0)};

    const auto res = em::resolve_descriptor_stage(box, "body", refs);
    check(res.size() == 1, "confident: one resolution");
    check(res[0].outcome == em::LadderOutcome::AutoBind, "confident: AutoBind");
    check(res[0].score >= em::kAutoBindMinScore, "confident: score >= 0.85");
    check(res[0].margin >= em::kAutoBindMinMargin, "confident: margin >= 0.10");
    // It bound the actual z=0 face (its snapshot TopoKey resolves back to that face).
    const std::string tk =
        em::ElementMapPartition::topokey_for_shape(box, z0, km::ElementKind::Face);
    check(res[0].bound_topo_key == tk, "confident: bound the correct face topoKey");
}

// (3) HISTORY RESOLVES EVERYTHING — track a face that survives a fuse with a UNIQUE
// image; apply_history rebinds it WITHOUT ever consulting the descriptor scorer.
void test_history_resolves_no_scoring() {
    const TopoDS_Shape a = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();          // 0..10
    const TopoDS_Shape b = BRepPrimAPI_MakeBox(gp_Pnt(6, 6, 6), 10.0, 10.0, 10.0).Shape();

    em::ElementMapPartition part;
    // Track the x=0 face — the corner box `b` does not touch it, so the fuse maps it
    // to a single image (unique history).
    const TopoDS_Shape x0 = face_by_center(a, 0, 5, 5);
    part.mint("A", "A_x0", km::ElementKind::Face, x0, a);

    BRepAlgoAPI_Fuse fuse(a, b);
    fuse.Build();
    const TopoDS_Shape result = fuse.Shape();

    em::reset_scoring_call_count();
    em::ElementMapDelta delta;
    std::vector<nlohmann::json> nr;
    part.apply_history("A", result, fuse, delta, &nr);

    check(em::scoring_call_count() == 0,
          "history: descriptor scorer NEVER consulted (unique image auto-binds)");
    check(nr.empty(), "history: no NeedsRepair (history resolves the element)");
    check(part.contains("A_x0"), "history: tracked face still bound");
}

// (4) MIN-COST ASSIGNMENT — the documented greedy counterexample (Assignment.h).
void test_assignment_beats_greedy() {
    // cost = 1 − matchScore. Greedy gives ref_A its best (X, 0.08) and forces ref_B
    // onto Y (0.80) → total 0.88; optimal is A→Y (0.10) + B→X (0.09) = 0.19.
    const std::vector<std::vector<double>> cost = {{0.08, 0.10}, {0.09, 0.80}};
    const std::vector<int> a = em::min_cost_assignment(cost);
    check(a.size() == 2, "assign: two rows");
    check(a[0] == 1 && a[1] == 0, "assign: optimal A->Y, B->X (not greedy A->X)");
    const double total = cost[0][a[0]] + cost[1][a[1]];
    check(total < 0.20, "assign: optimal total cost < 0.20 (greedy would be 0.88)");
}

// (5) SCORED SPLIT LINEAGE (closes review finding 2) — a symmetric split of a
// tracked top face ⇒ NeedsRepair "ambiguous", with the scorer CONSULTED.
void test_symmetric_split_needs_repair() {
    const TopoDS_Shape base = BRepPrimAPI_MakeBox(20.0, 10.0, 10.0).Shape();  // top z=10, 20×10
    em::ElementMapPartition part;
    const TopoDS_Shape top = face_by_center(base, 10, 5, 10);
    // Mint WITH an anchor at the original centre so the split halves tie.
    part.mint("A", "A_top", km::ElementKind::Face, top, base,
              nlohmann::json{{"worldPoint", {10.0, 5.0, 10.0}}});

    // Cut a central slot across the top → splits it into two equal 9×10 halves.
    const TopoDS_Shape slot = BRepPrimAPI_MakeBox(gp_Pnt(9, 0, 8), 2.0, 10.0, 4.0).Shape();
    BRepAlgoAPI_Cut cut(base, slot);
    cut.Build();
    const TopoDS_Shape result = cut.Shape();

    em::reset_scoring_call_count();
    em::ElementMapDelta delta;
    std::vector<nlohmann::json> nr;
    part.apply_history("A", result, cut, delta, &nr);

    check(em::scoring_call_count() > 0, "split: descriptor scorer WAS consulted (finding 2)");
    check(nr.size() == 1, "split: exactly one NeedsRepair for the ambiguous split");
    if (nr.size() == 1) {
        check(nr[0].value("reason", "") == "ambiguous", "split: reason ambiguous");
        check(nr[0].value("ladderFailed", "") == "history", "split: ladderFailed history");
        check(nr[0]["candidates"].size() >= 2, "split: both split images surfaced as candidates");
    }
    check(!part.contains("A_top"), "split: ambiguous entry dropped (never a wrong bind)");
}

// (6) THE MIGRATION GOAL — a fillet edge reference (frozen on box A, width 10)
// SURVIVES a small upstream edit (width 10.5) via descriptor+anchor, but a large
// ambiguous change (width 30) ⇒ NeedsRepair (never a silent wrong bind). This is
// the H5-B naming-break fix expressed at the ladder level (corpus case e).
void test_survives_small_edit_else_needs_repair() {
    const TopoDS_Shape a = BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape();
    const TopoDS_Shape edge_a = edge_by_center(a, 5, 0, 10);  // top-front edge, len 10
    // The ref as authored on A (frozen edge descriptor + anchor at its midpoint).
    em::LadderRef ref;
    ref.ref_id = "op_fillet.input0";
    ref.element_id = "el_rim";
    ref.kind = km::ElementKind::Edge;
    ref.has_descriptor = true;
    ref.descriptor = em::ElementMapPartition::describe(edge_a);
    ref.anchor.has_world_point = true;
    ref.anchor.world_point = gp_Pnt(5, 0, 10);
    ref.anchor_json = {{"worldPoint", {5.0, 0.0, 10.0}}};

    // Small edit: width 10 → 10.5. The frozen ref auto-binds to the moved rim edge.
    const TopoDS_Shape b = BRepPrimAPI_MakeBox(10.5, 10.0, 10.0).Shape();
    auto rb = em::resolve_descriptor_stage(b, "body", {ref});
    check(rb.size() == 1 && rb[0].outcome == em::LadderOutcome::AutoBind,
          "edit: fillet edge SURVIVES a small upstream edit (auto-bind)");
    if (!rb.empty() && rb[0].outcome == em::LadderOutcome::AutoBind) {
        const TopoDS_Shape want = edge_by_center(b, 5.25, 0, 10);
        check(rb[0].bound_topo_key ==
                  em::ElementMapPartition::topokey_for_shape(b, want, km::ElementKind::Edge),
              "edit: bound the corresponding moved rim edge");
    }

    // Large edit: width 10 → 30. The rim edge changed too much ⇒ NeedsRepair.
    const TopoDS_Shape c = BRepPrimAPI_MakeBox(30.0, 10.0, 10.0).Shape();
    auto rc = em::resolve_descriptor_stage(c, "body", {ref});
    check(rc.size() == 1 && rc[0].outcome == em::LadderOutcome::NeedsRepair,
          "edit: a large ambiguous change ⇒ NeedsRepair (never a wrong bind)");
}

// ── H6a: edit-scoped descriptor-tie veto + proportional anchor floor ──────────
//
// The B3 attack scene: a 40×20×25 box whose x=0 and x=40 faces are EXACT descriptor
// twins (same 20×25 area, opposite normals — scored by |dot| so a flip still
// matches — same sorted-incident-edge adjacency hash). The ref was authored on the
// x=0 face, but its stored anchor sits near the x=40 TWIN, which is what a stale
// anchor looks like after an upstream edit moved geometry out from under it.
// Descriptor evidence CANNOT separate the two; only the anchor can.
//
// `twin_offset` slides the stored anchor along −X away from the x=40 twin's centre,
// which is what selects the two cases the veto must distinguish:
//   * offset 0        — the twin sits EXACTLY at the stale anchor (TELEPORT).
//   * offset > eps·scale — the twin is merely NEARER to the stale anchor than the
//                       real element is (DRIFT).
// The anchor scale here is 0.5·diag = 0.5·√(40²+20²+25²) ≈ 25.62, so the
// anchor-exact threshold is 0.05 · 25.62 ≈ 1.28 mm.
struct TwinScene {
    TopoDS_Shape box;
    em::LadderRef ref;
};

TwinScene b3_twin_scene(double twin_offset) {
    TwinScene s;
    s.box = BRepPrimAPI_MakeBox(40.0, 20.0, 25.0).Shape();
    const TopoDS_Shape authored = face_by_center(s.box, 0, 10, 12.5);  // x=0
    // Anchor near the x=40 twin — the STALE anchor.
    s.ref = face_ref("el_twin", authored, 40.0 - twin_offset, 10.0, 12.5);
    return s;
}

// The anchor-exact threshold for the 40×20×25 twin scene (≈1.28 mm).
double twin_scene_exact_threshold() {
    return em::kAnchorExactEps * em::anchor_scale(std::sqrt(40.0 * 40.0 + 20.0 * 20.0 + 25.0 * 25.0));
}

void dump_top2(const char* label, const em::LadderResolution& r) {
    if (r.candidates.size() < 2) return;
    std::fprintf(stderr, "  [%s] top1 %s score=%.4f | top2 %s score=%.4f\n", label,
                 r.candidates[0].topo_key.c_str(), r.candidates[0].score,
                 r.candidates[1].topo_key.c_str(), r.candidates[1].score);
}

// (7a) B3 DRIFT — the class the veto MUST catch. The stale anchor no longer sits on
// ANY candidate; a congruent twin is merely NEARER to it than the moved original.
// Nothing is sitting where it was authored, so nothing has earned the anchor's
// trust, and binding on proximity alone would be a silent wrong bind.
//
// Also pins the carve-out BOUNDARY in both directions: inside the anchor-exact
// epsilon the twin binds (the winner demonstrably did not move), outside it vetoes.
// A carve-out that is too wide fails the second half; one that is absent or too
// narrow fails the first.
void test_edit_scoped_drift_veto_needs_repair() {
    const double eps = twin_scene_exact_threshold();

    // (i) INSIDE the epsilon — the twin is still effectively AT the anchor.
    const TwinScene near = b3_twin_scene(eps * 0.5);
    const auto res_near = em::resolve_descriptor_stage(
        near.box, "body", {near.ref}, em::LadderEditContext{/*post_upstream_edit=*/true});
    check(res_near.size() == 1, "drift: one resolution (inside eps)");
    if (!res_near.empty()) {
        dump_top2("drift<eps", res_near[0]);
        check(res_near[0].outcome == em::LadderOutcome::AutoBind,
              "drift: WITHIN the anchor-exact epsilon the anchor may still decide");
    }

    // (ii) OUTSIDE the epsilon — genuine drift ⇒ the veto fires.
    const TwinScene far = b3_twin_scene(eps * 2.5);
    const auto res_far = em::resolve_descriptor_stage(
        far.box, "body", {far.ref}, em::LadderEditContext{/*post_upstream_edit=*/true});
    check(res_far.size() == 1, "drift: one resolution (outside eps)");
    if (res_far.empty()) return;
    dump_top2("drift>eps", res_far[0]);
    check(res_far[0].outcome == em::LadderOutcome::NeedsRepair,
          "drift: a twin merely NEARER to a stale anchor must NOT auto-bind (B3)");
    check(res_far[0].reason == "ambiguous", "drift: reason ambiguous (no new §9 token)");
    check(res_far[0].candidates.size() >= 2, "drift: both twins surfaced as candidates");
    if (res_far[0].candidates.size() >= 2) {
        // Both x-faces are in the evidence, so a repair UI can offer the pair.
        const std::string k0 = res_far[0].candidates[0].topo_key;
        const std::string k1 = res_far[0].candidates[1].topo_key;
        check(k0 != k1 && !k0.empty() && !k1.empty(), "drift: two distinct candidate topoKeys");
    }
}

// (7b) B3 TELEPORT — on a CHECKPOINT replay the anchor-exact carve-out is correct:
// the incremental path's `apply_placement` keeps anchors fresh, so an exact twin AT
// the anchor genuinely did not move. It auto-binds.
//
// On a FROM-0 replay there is no migrated partition; the stored anchor can be stale,
// and the same carve-out would bless a congruent decoy parked at the stale anchor
// (VF-M5). `LadderEditContext::from_zero_replay` disables the carve-out there.
void test_edit_scoped_teleport_is_the_accepted_residual() {
    const TwinScene s = b3_twin_scene(0.0);  // twin EXACTLY at the stale anchor
    const auto res = em::resolve_descriptor_stage(s.box, "body", {s.ref},
                                                  em::LadderEditContext{true});
    check(res.size() == 1, "teleport: one resolution");
    if (res.empty()) return;
    dump_top2("teleport", res[0]);
    check(res[0].outcome == em::LadderOutcome::AutoBind,
          "teleport: checkpoint replay keeps the anchor-exact carve-out");

    const auto res_from0 = em::resolve_descriptor_stage(
        s.box, "body", {s.ref}, em::LadderEditContext{/*post_upstream_edit=*/true,
                                                     /*from_zero_replay=*/true});
    check(res_from0.size() == 1, "teleport-from0: one resolution");
    if (res_from0.empty()) return;
    dump_top2("teleport-from0", res_from0[0]);
    check(res_from0[0].outcome == em::LadderOutcome::NeedsRepair,
          "teleport: from-0 replay with edit context must NOT auto-bind a stale "
          "anchor (VF-M5)");
    check(res_from0[0].reason == "ambiguous",
          "teleport-from0: reason ambiguous");
}

// (8) REOPEN SEMANTICS — the drift scene with NO edit context is a from-0 replay:
// the geometry is byte-identical to the one the ref was authored against, so the
// anchor is authoritative and deciding the tie with it is CORRECT. This pins that
// H6a did not break reopen — note it uses the OUTSIDE-eps offset, so it is the
// case the edit lane refuses, proving the two lanes really do differ.
void test_no_edit_context_anchor_still_decides() {
    const TwinScene s = b3_twin_scene(twin_scene_exact_threshold() * 2.5);
    const auto res = em::resolve_descriptor_stage(s.box, "body", {s.ref});  // no edit context
    check(res.size() == 1, "reopen: one resolution");
    if (res.empty()) return;
    dump_top2("reopen", res[0]);
    check(res[0].outcome == em::LadderOutcome::AutoBind,
          "reopen: with NO edit context the anchor still decides a descriptor tie");
    const TopoDS_Shape at_anchor = face_by_center(s.box, 40, 10, 12.5);
    check(res[0].bound_topo_key ==
              em::ElementMapPartition::topokey_for_shape(s.box, at_anchor, km::ElementKind::Face),
          "reopen: it bound the candidate NEAREST the anchor");
}

// (9) PROPORTIONAL ANCHOR FLOOR — a sub-millimetre part. The anchor proximity
// feature scales by `max(0.5*bodyDiag, floor)`. With the old fixed 1.0 mm floor a
// 0.3 mm separation on a 0.77 mm body is only 0.075 of margin (< 0.10) ⇒ the whole
// part is un-resolvable. With the proportional floor the same separation is 0.195
// ⇒ a clean auto-bind. body_diagonal() already floors a degenerate box at 1.0, so
// the new 1e-7 is a divide-by-zero guard only.
void test_proportional_anchor_floor_submm() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(0.3, 0.5, 0.5).Shape();
    const TopoDS_Shape x0 = face_by_center(box, 0, 0.25, 0.25);
    // Anchor exactly ON the authored face; the x=0.3 twin is 0.3 mm away.
    std::vector<em::LadderRef> refs{face_ref("el_submm", x0, 0.0, 0.25, 0.25)};

    const auto res = em::resolve_descriptor_stage(box, "body", refs);
    check(res.size() == 1, "submm: one resolution");
    if (res.empty()) return;
    dump_top2("submm", res[0]);
    check(res[0].outcome == em::LadderOutcome::AutoBind,
          "submm: a 0.3mm separation on a 0.77mm body auto-binds (proportional floor)");
    check(res[0].margin >= em::kAutoBindMinMargin,
          "submm: the margin clears 0.10 once the anchor scale is proportional");
    check(res[0].bound_topo_key ==
              em::ElementMapPartition::topokey_for_shape(box, x0, km::ElementKind::Face),
          "submm: it bound the face the anchor sits on, not its twin");
}

// (10) SYMMETRIC EXACT TIE — unchanged by edit context in EITHER direction. The
// anchor is equidistant, so there is nothing for it to decide; the margin gate
// already refuses. Edit context must not turn this into an auto-bind, and its
// absence must not either.
void test_exact_tie_needs_repair_either_way() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(gp_Pnt(-5, -5, -10), 10.0, 10.0, 20.0).Shape();
    const TopoDS_Shape top = face_by_center(box, 0, 0, 10);
    const std::vector<em::LadderRef> refs{face_ref("el_sym2", top, 0, 0, 0)};

    const auto with_edit =
        em::resolve_descriptor_stage(box, "body", refs, em::LadderEditContext{true});
    const auto without = em::resolve_descriptor_stage(box, "body", refs);
    check(with_edit.size() == 1 && with_edit[0].outcome == em::LadderOutcome::NeedsRepair,
          "exact tie: NeedsRepair WITH edit context");
    check(without.size() == 1 && without[0].outcome == em::LadderOutcome::NeedsRepair,
          "exact tie: NeedsRepair WITHOUT edit context (unchanged)");
}

// (11) THE VETO IS SCOPED, not a blanket post-edit refusal. Notch one twin so the
// two x-faces are no longer congruent (484 vs 500 mm², different adjacency hash):
// the descriptor now separates them well past the tie epsilon, so a post-edit
// resolution still auto-binds. Without this, H6a would degrade to "any edit ⇒
// NeedsRepair" and defeat the ladder.
void test_veto_does_not_fire_on_distinguishable_descriptors() {
    const TopoDS_Shape base = BRepPrimAPI_MakeBox(40.0, 20.0, 25.0).Shape();
    const TopoDS_Shape notch = BRepPrimAPI_MakeBox(gp_Pnt(35.0, -1.0, -1.0), 10.0, 5.0, 5.0).Shape();
    BRepAlgoAPI_Cut cut(base, notch);
    cut.Build();
    const TopoDS_Shape body = cut.Shape();

    const TopoDS_Shape x0 = face_by_center(body, 0, 10, 12.5);
    std::vector<em::LadderRef> refs{face_ref("el_unique", x0, 0.0, 10.0, 12.5)};

    const auto res =
        em::resolve_descriptor_stage(body, "body", refs, em::LadderEditContext{true});
    check(res.size() == 1, "scoped: one resolution");
    if (res.empty()) return;
    dump_top2("scoped", res[0]);
    check(res[0].outcome == em::LadderOutcome::AutoBind,
          "scoped: a DISTINGUISHABLE descriptor still auto-binds under edit context");
    check(res[0].bound_topo_key ==
              em::ElementMapPartition::topokey_for_shape(body, x0, km::ElementKind::Face),
          "scoped: it bound the authored face");
}

}  // namespace

// ── resolverVersion 4 (kernel-hardening WP-A): SIDEDNESS + RELATIVE ANCHOR ─────
// (v4-a) OPPOSITE-FACING TWINS separate on `outward`: the two 10×10 end faces of a
// 10×10×20 bar are descriptor twins in every v3 feature. A ref minted WITH the body
// (so it carries `outward`) ranks its own face FIRST BY A FULL MARGIN with the
// anchor at the body centre, where the anchor cannot help (equidistant) — the
// signed outward alone tells +Z from −Z. (The absolute score is low-confidence
// there, so the outcome is NeedsRepair; with the pick ON the face it AutoBinds.)
void test_v4_opposite_facing_twins_resolve() {
    const TopoDS_Shape box = BRepPrimAPI_MakeBox(gp_Pnt(-5, -5, -10), 10.0, 10.0, 20.0).Shape();
    const TopoDS_Shape top = face_by_center(box, 0, 0, 10);
    const std::string tk = em::ElementMapPartition::topokey_for_shape(box, top, km::ElementKind::Face);
    em::LadderRef r;
    r.ref_id = "op.input0";
    r.element_id = "el_top";
    r.kind = km::ElementKind::Face;
    r.has_descriptor = true;
    r.descriptor = em::ElementMapPartition::describe(top, box);  // sided
    r.anchor.has_world_point = true;
    r.anchor.world_point = gp_Pnt(0, 0, 0);  // equidistant to both end faces
    r.anchor_json = {{"worldPoint", {0.0, 0.0, 0.0}}};
    check(r.descriptor.hasOutward, "v4: a body-described face carries outward");
    const auto res = em::resolve_descriptor_stage(box, "body", {r});
    check(res.size() == 1 && !res[0].candidates.empty() && res[0].candidates[0].topo_key == tk,
          "v4: the +Z face ranks first on outward with the anchor at the centre");
    check(res.size() == 1 && !res[0].candidates.empty() &&
              res[0].candidates[0].margin >= em::kAutoBindMinMargin,
          "v4: the outward feature alone supplies the 0.10 margin over the −Z twin");
    // The pick ON the face: the ordinary AutoBind.
    r.anchor.world_point = gp_Pnt(0, 0, 10);
    r.anchor_json = {{"worldPoint", {0.0, 0.0, 10.0}}};
    const auto on = em::resolve_descriptor_stage(box, "body", {r});
    check(on.size() == 1 && on[0].outcome == em::LadderOutcome::AutoBind,
          "v4: a pick on the +Z face AutoBinds");
    check(on.size() == 1 && on[0].bound_topo_key == tk, "v4: bound the +Z face, not its −Z twin");
}

// (v4-b) SAME-FACING TWINS still tie: a slotted 30×10×10 bar has two identical +Z
// top faces; a sided ref with the anchor midway between them is a genuine
// ambiguity ⇒ NeedsRepair, never a guess.
void test_v4_same_facing_twins_still_tie() {
    const TopoDS_Shape bar = BRepPrimAPI_MakeBox(30.0, 10.0, 10.0).Shape();
    const TopoDS_Shape slot = BRepPrimAPI_MakeBox(gp_Pnt(10, -1, 5), 10.0, 12.0, 6.0).Shape();
    BRepAlgoAPI_Cut cut(bar, slot);
    cut.Build();
    const TopoDS_Shape body = cut.Shape();
    const TopoDS_Shape top_left = face_by_center(body, 5, 5, 10);
    em::LadderRef r;
    r.ref_id = "op.input0";
    r.element_id = "el_left";
    r.kind = km::ElementKind::Face;
    r.has_descriptor = true;
    r.descriptor = em::ElementMapPartition::describe(top_left, body);
    r.anchor.has_world_point = true;
    r.anchor.world_point = gp_Pnt(15, 5, 10);
    r.anchor_json = {{"worldPoint", {15.0, 5.0, 10.0}}};
    const auto res = em::resolve_descriptor_stage(body, "body", {r});
    check(res.size() == 1 && res[0].outcome == em::LadderOutcome::NeedsRepair,
          "v4: same-facing twins with an equidistant anchor still NeedsRepair");
    check(res.size() == 1 && res[0].reason == "ambiguous", "v4: reason ambiguous");
}

// (v4-c) THE PLATE: a rim edge of a 100×100×5 plate against its twin on the
// opposite rim. In v3 the anchor (scaled by half the 141 mm diagonal) could not
// clear the 0.10 margin (measured: NeedsRepair at commit); v4 binds it on
// outward + the relative anchor. Anchor-only ref too (the Hole seat form).
void test_v4_plate_rim_edge_and_anchor_only_face() {
    const TopoDS_Shape plate = BRepPrimAPI_MakeBox(100.0, 100.0, 5.0).Shape();
    // The top rim edge along X at y=0: midpoint (50, 0, 5).
    TopTools_IndexedMapOfShape edges;
    TopExp::MapShapes(plate, TopAbs_EDGE, edges);
    TopoDS_Shape rim;
    for (int i = 1; i <= edges.Extent(); ++i) {
        const km::ElementDescriptor d = em::ElementMapPartition::describe(edges(i));
        if (std::abs(d.center.X() - 50.0) < 1e-6 && std::abs(d.center.Y()) < 1e-6 &&
            std::abs(d.center.Z() - 5.0) < 1e-6) rim = edges(i);
    }
    check(!rim.IsNull(), "v4 plate: located the top rim edge");
    if (rim.IsNull()) return;
    em::LadderRef r;
    r.ref_id = "op.input0";
    r.element_id = "el_rim";
    r.kind = km::ElementKind::Edge;
    r.has_descriptor = true;
    r.descriptor = em::ElementMapPartition::describe(rim, plate);
    r.anchor.has_world_point = true;
    r.anchor.world_point = gp_Pnt(50, 0, 5);
    r.anchor_json = {{"worldPoint", {50.0, 0.0, 5.0}}};
    const auto res = em::resolve_descriptor_stage(plate, "body", {r});
    check(res.size() == 1 && res[0].outcome == em::LadderOutcome::AutoBind,
          "v4 plate: the top rim edge binds (score " +
              (res.empty() ? std::string("?") : std::to_string(res[0].score)) + ", margin " +
              (res.empty() ? std::string("?") : std::to_string(res[0].margin)) + ")");
    const std::string tk = em::ElementMapPartition::topokey_for_shape(plate, rim, km::ElementKind::Edge);
    check(res.size() == 1 && res[0].bound_topo_key == tk, "v4 plate: bound the TOP rim, not the bottom twin");

    // Anchor-only ref on the top FACE (the Hole seat form): the pick lies ON the
    // face and 5 mm from its twin — decisive under the relative anchor.
    em::LadderRef f;
    f.ref_id = "op.input1";
    f.element_id = "el_seat";
    f.kind = km::ElementKind::Face;
    f.has_descriptor = false;
    f.anchor.has_world_point = true;
    f.anchor.world_point = gp_Pnt(30, 40, 5);
    f.anchor_json = {{"worldPoint", {30.0, 40.0, 5.0}}};
    const auto fres = em::resolve_descriptor_stage(plate, "body", {f});
    check(fres.size() == 1 && fres[0].outcome == em::LadderOutcome::AutoBind,
          "v4 plate: an anchor-only top-face pick binds (score " +
              (fres.empty() ? std::string("?") : std::to_string(fres[0].score)) + ")");
    const TopoDS_Shape top = face_by_center(plate, 50, 50, 5);
    const std::string ftk = em::ElementMapPartition::topokey_for_shape(plate, top, km::ElementKind::Face);
    check(fres.size() == 1 && fres[0].bound_topo_key == ftk, "v4 plate: bound the TOP face");
}

int main() {
    test_v4_opposite_facing_twins_resolve();
    test_v4_same_facing_twins_still_tie();
    test_v4_plate_rim_edge_and_anchor_only_face();
    test_symmetric_tie();
    test_confident_autobind();
    test_history_resolves_no_scoring();
    test_assignment_beats_greedy();
    test_symmetric_split_needs_repair();
    test_survives_small_edit_else_needs_repair();
    test_edit_scoped_drift_veto_needs_repair();
    test_edit_scoped_teleport_is_the_accepted_residual();
    test_no_edit_context_anchor_still_decides();
    test_proportional_anchor_floor_submm();
    test_exact_tie_needs_repair_either_way();
    test_veto_does_not_fire_on_distinguishable_descriptors();
    if (g_failures == 0) std::fprintf(stderr, "wp6_ladder: OK\n");
    return g_failures;
}
