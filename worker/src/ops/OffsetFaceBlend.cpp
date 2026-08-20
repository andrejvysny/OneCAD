// OffsetFaceBlend.cpp — the V3 blend-aware closure walk (WP3-C5). Header contract:
// `ops/OffsetFaceOp.h`, `blend_aware_closure`.
//
// WHY THIS IS A SEPARATE WALK AND NOT A FLAG ON `tangent_closure`. `tangent_closure`
// is shared VERBATIM with the authoring verb `PrepareOffsetFace`, which freezes
// `faceIds`, and with the V2 executor guard. Both must keep answering exactly what
// they answer today, because a V2 record's geometry is frozen. This walk is the same
// BFS with one fork — a certified blend is recorded instead of traversed — and it
// runs only for `resultPolicyVersion: 3`.
//
// THE COST OF THE FORK IS PAID IN REFUSALS. Once the walk can stop at a blend, a
// face it CANNOT certify is no longer harmless: treating it as an ordinary design
// face is V2's behaviour, which is the destructive one this work package exists to
// remove. So every G1 neighbour the sampled recognizer CALLS a blend and the proof
// stack then refuses stops the walk with that condition's own code.
//
// ── THE THREE-WAY FORK, AND WHY `Ambiguous` IS NOT `NotBlend` ────────────────
// `BlendRecognitionStatus` has three values and this walk treats them as three
// cases, not two:
//
//   NotBlend   MEASURED ordinary geometry — non-constant section radius, or not G1
//              tangent to its supports. Traversed, exactly as V2 traverses it.
//   Recognized handed to the proof stack; certified into B, or refused by name.
//   Ambiguous  the recognizer CANNOT TELL. The dominant source is "more than two G1
//              support faces", which is every vertex-patch region of a fully-rounded
//              body: an edge fillet that meets a corner sphere is G1 to three faces,
//              so it is never Recognized and never certified.
//
// V1 TRAVERSES `Ambiguous`, and that is a decision rather than an oversight. The
// alternative — refusing — would make V3 refuse a fully-rounded body outright, which
// is strictly worse than V2 for a user who is not editing near the corner at all.
// Traversing is exactly V2's behaviour on exactly V2's geometry: the face is offset
// along with the rest of the tangent chain. It is NOT a claim that the face is
// ordinary, and a body that lands here gets no blend awareness at all. That is the
// documented V1 fallthrough; C6's adversarial corpus is where the vertex-patch case
// is revisited, and `test_offsetface_reblend.cpp` pins the current behaviour (V3 ==
// V2, bit-identical) so a future change to it has to be deliberate.
#include "ops/OffsetFaceOp.h"

#include <algorithm>
#include <cmath>
#include <deque>
#include <map>
#include <set>
#include <string>
#include <vector>

#include <BRepLib.hxx>
#include <GeomAbs_Shape.hxx>
#include <Standard_Failure.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopTools_IndexedDataMapOfShapeListOfShape.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopTools_ListOfShape.hxx>
#include <TopoDS.hxx>

#include "kernel/fillet/BlendEvidence.h"
#include "kernel/fillet/BlendRecognizer.h"
#include "kernel/validation/GeometryPrecision.h"

namespace onecad::ops::offsetface {

namespace {

namespace kf = onecad::kernel::fillet;

bool starts_with(const std::string& value, const char* prefix) {
    return value.rfind(prefix, 0) == 0;
}

// What the walk does with ONE G1 neighbour.
struct Verdict {
    enum class Kind { Traverse, Blend, Refuse };
    Kind kind = Kind::Traverse;
    kf::RecognizedBlend blend;
    std::string code;
    std::string message;
};

Verdict refuse(const char* code, const std::string& message) {
    Verdict out;
    out.kind = Verdict::Kind::Refuse;
    out.code = code;
    out.message = message;
    return out;
}

// `certify_blend`'s refusal vocabulary, mapped onto the §7.3 scope codes the plan
// names. The mapping is by CONDITION, not by string convenience: the certification
// reason says which proof layer refused, and the surface/support kinds say which of
// the two unsupported-scope codes describes it.
Verdict scope_refusal(const TopoDS_Shape& body, const TopoDS_Face& face, int ordinal,
                      const std::string& reason) {
    const std::string at = " at " + face_topokey(ordinal) + ": ";
    if (starts_with(reason, "BLEND_BOUNDARY_")) {
        return refuse("OFFSET_FACE_BLEND_BOUNDARY_NOT_SIMPLE",
                      "dependent blend" + at + reason);
    }
    if (starts_with(reason, "BLEND_EVIDENCE_INSUFFICIENT")) {
        // A SIZE refusal, not a surface one: the face is an exact cylinder that
        // certifies analytically, and its trimmed domain is simply too small to
        // answer the raised budget (64 samples, exactly 2 boundaries). Labelling it
        // `_BLEND_SURFACE_UNSUPPORTED` would tell the user their blend is the wrong
        // KIND of surface when the truth is that it is too small to measure.
        return refuse("OFFSET_FACE_BLEND_EVIDENCE_INSUFFICIENT", "dependent blend" + at + reason);
    }
    if (surface_kind(face) != SurfaceKind::Cylinder) {
        // V1 accepts `GeomAbs_Cylinder` blends only, and that limit is load-bearing
        // rather than cautious: it is what turns "constant section radius" from a
        // finite-sample inference into an exact property of
        // `BRepAdaptor_Surface::Cylinder()`, which is the only honest basis for
        // destroying the face.
        return refuse("OFFSET_FACE_BLEND_SURFACE_UNSUPPORTED",
                      "dependent blend" + at + reason);
    }
    for (const TopoDS_Face& support : kf::recognize_blend_at(face, body).support_faces) {
        if (surface_kind(support) == SurfaceKind::Other) {
            return refuse("OFFSET_FACE_BLEND_SUPPORT_UNSUPPORTED",
                          "dependent blend" + at + reason +
                              " (a support is neither planar nor cylindrical)");
        }
    }
    return refuse("OFFSET_FACE_BLEND_SURFACE_UNSUPPORTED", "dependent blend" + at + reason);
}

Verdict classify(const TopoDS_Shape& body, const TopoDS_Face& face, int ordinal,
                 const TopoDS_Face& entered_from, int from_ordinal) {
    const kf::BlendCertification certification = kf::certify_blend(face, body);
    if (!certification.ok) {
        switch (certification.recognition_status) {
            case kf::BlendRecognitionStatus::NotBlend:
                // MEASURED ordinary geometry. Traversed exactly as V2 traverses it —
                // this is the branch that keeps the split-cylinder wall, the draft
                // pair and every ordinary tangent chain identical at both versions.
                return Verdict{};
            case kf::BlendRecognitionStatus::Ambiguous:
                // THE RECOGNIZER DECLINED TO ANSWER (dominantly: more than two G1
                // supports, i.e. a vertex-patch region). V1 traverses, which is V2's
                // behaviour on V2's geometry — see the fork note at the top of this
                // file. No blend awareness is claimed for this face.
                return Verdict{};
            case kf::BlendRecognitionStatus::Recognized:
                break;  // the sampled layer agreed; a LATER proof layer refused
        }
        return scope_refusal(body, face, ordinal, certification.reason);
    }
    const kf::RecognizedBlend& blend = certification.blend;
    if (!blend.support_a.IsSame(entered_from) && !blend.support_b.IsSame(entered_from)) {
        // Structurally unreachable — the walk entered over a G1 edge, and recognition
        // requires exactly two G1 supports, so the entering face IS one of them. Held
        // anyway: silently suppressing a blend the operative face does not support
        // would destroy geometry the request never named.
        return refuse("OFFSET_FACE_DEPENDENT_NOT_A_BLEND",
                      "the blend at " + face_topokey(ordinal) + " is not supported by " +
                          face_topokey(from_ordinal) + ", which the closure entered it from");
    }
    Verdict out;
    out.kind = Verdict::Kind::Blend;
    out.blend = blend;
    return out;
}

}  // namespace

std::vector<int> BlendAwareClosure::ordinals() const {
    std::vector<int> out = moving;
    for (const ClosureBlend& entry : blends) out.push_back(entry.ordinal);
    out.insert(out.end(), fixed.begin(), fixed.end());
    std::sort(out.begin(), out.end());
    out.erase(std::unique(out.begin(), out.end()), out.end());
    return out;
}

BlendAwareClosure blend_aware_closure(const TopoDS_Shape& body,
                                      const std::vector<int>& seed_ordinals) {
    BlendAwareClosure out;
    if (body.IsNull() || seed_ordinals.empty()) {
        out.message = "blend-aware closure requires a body and at least one seed face";
        return out;
    }

    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(body, TopAbs_FACE, faces);
    TopTools_IndexedDataMapOfShapeListOfShape edge_faces;
    TopExp::MapShapesAndAncestors(body, TopAbs_EDGE, TopAbs_FACE, edge_faces);

    std::set<int> moving;                          // C_op — ascending by construction
    std::map<int, kf::RecognizedBlend> certified;  // B — ascending by construction
    std::deque<int> queue;
    std::vector<int> seeds = seed_ordinals;
    std::sort(seeds.begin(), seeds.end());
    seeds.erase(std::unique(seeds.begin(), seeds.end()), seeds.end());
    for (const int ord : seeds) {
        if (ord < 1 || ord > faces.Extent()) {
            out.message = "blend-aware closure seed is foreign to the body";
            return out;
        }
        if (moving.insert(ord).second) queue.push_back(ord);
    }

    while (!queue.empty()) {
        const int ord = queue.front();
        queue.pop_front();
        const TopoDS_Face f1 = TopoDS::Face(faces(ord));
        for (TopExp_Explorer ex(f1, TopAbs_EDGE); ex.More(); ex.Next()) {
            const TopoDS_Edge edge = TopoDS::Edge(ex.Current());
            const int idx = edge_faces.FindIndex(edge);
            if (idx == 0) continue;
            std::vector<int> adjacent;
            for (TopTools_ListIteratorOfListOfShape it(edge_faces(idx)); it.More(); it.Next()) {
                const int a = faces.FindIndex(it.Value());
                if (a == 0) continue;
                if (std::find(adjacent.begin(), adjacent.end(), a) == adjacent.end()) {
                    adjacent.push_back(a);
                }
            }
            if (adjacent.size() > 2) {
                out.message = "the operative set touches a non-manifold edge";
                return out;
            }
            if (adjacent.size() != 2) continue;  // seam self-adjacency or free edge
            const int other = (adjacent[0] == ord) ? adjacent[1] : adjacent[0];
            if (other == ord) continue;
            if (moving.count(other) != 0 || certified.count(other) != 0) continue;
            const TopoDS_Face f2 = TopoDS::Face(faces(other));
            GeomAbs_Shape continuity = GeomAbs_C0;
            try {
                continuity = BRepLib::ContinuityOfFaces(edge, f1, f2, kTangentAngleTol);
            } catch (const Standard_Failure&) {
                continue;  // unreadable junction ⇒ not provably tangent ⇒ not chained
            }
            if (continuity < GeomAbs_G1) continue;

            const Verdict verdict = classify(body, f2, other, f1, ord);
            if (verdict.kind == Verdict::Kind::Refuse) {
                out.code = verdict.code;
                out.message = verdict.message;
                return out;
            }
            if (verdict.kind == Verdict::Kind::Blend) {
                certified.emplace(other, verdict.blend);
                continue;  // recorded, NOT traversed — this is the whole fork
            }
            moving.insert(other);
            queue.push_back(other);
        }
    }

    // F is what the FULL G1 walk reaches and this one deliberately did not: the
    // faces that live behind a blend. Computed from the shared `tangent_closure` so
    // the two walks can never disagree about what "the G1 closure" means.
    const ClosureResult full = tangent_closure(body, seeds);
    if (!full.ok) {
        out.message = full.non_manifold ? "the operative set touches a non-manifold edge"
                                        : "tangent-closure walk failed";
        return out;
    }

    out.moving.assign(moving.begin(), moving.end());
    for (const auto& entry : certified) out.blends.push_back({entry.first, entry.second});
    for (const int ord : full.ordinals) {
        if (moving.count(ord) == 0 && certified.count(ord) == 0) out.fixed.push_back(ord);
    }

    if (!out.blends.empty()) {
        // A blend whose SUPPORT is ITSELF a blend is the chained/vertex-patch topology
        // V1 refuses by name. Suppression would re-extend a ROUND as if it were a
        // design surface and the rebuild would sit a fillet on a fillet — a corner
        // whose reproduction proof has no way to say what the result should have been.
        //
        // The support is tested by CERTIFICATION, not by membership in B. A support
        // that is a blend is almost never in B: the walk stops at the first blend it
        // meets and never traverses to what lies behind it, so a membership-only test
        // would be structurally blind to exactly the case it names.
        for (const ClosureBlend& entry : out.blends) {
            const TopoDS_Face* supports[2] = {&entry.blend.support_a, &entry.blend.support_b};
            for (int s = 0; s < 2; ++s) {
                const int support_ordinal = faces.FindIndex(*supports[s]);
                const bool is_blend = certified.count(support_ordinal) != 0 ||
                                      kf::certify_blend(*supports[s], body).ok;
                if (!is_blend) continue;
                out.code = "OFFSET_FACE_UNSUPPORTED_VERTEX_BLEND";
                out.message = "the blend at " + face_topokey(entry.ordinal) +
                              " is supported by another blend at " +
                              face_topokey(support_ordinal);
                return out;
            }
        }
        // Both halves of the same hole, and both are refused BEFORE anything is
        // destroyed: the rebuild is ONE single-radius fillet over every seed, so a
        // mixed-radius set would come back with one blend silently re-cut to the
        // other's radius, and a mixed-convexity set has no single expected volume
        // direction for the suppression postcondition to check.
        const kf::RecognizedBlend& first = out.blends.front().blend;
        const double radius_limit = kf::fillet_section_radius_limit(
            first.radius, kernel::validation::precision_of(body).coordinate_magnitude);
        for (const ClosureBlend& entry : out.blends) {
            if (entry.blend.convexity != first.convexity) {
                out.code = "OFFSET_FACE_MIXED_BLEND_CONVEXITY";
                out.message = "the closure mixes convex and concave blends (" +
                              face_topokey(out.blends.front().ordinal) + ", " +
                              face_topokey(entry.ordinal) + ")";
                return out;
            }
            if (std::abs(entry.blend.radius - first.radius) > radius_limit) {
                out.code = "OFFSET_FACE_MIXED_BLEND_RADII";
                out.message = "the closure mixes blend radii (" +
                              face_topokey(out.blends.front().ordinal) + " is R" +
                              std::to_string(first.radius) + ", " +
                              face_topokey(entry.ordinal) + " is R" +
                              std::to_string(entry.blend.radius) + ")";
                return out;
            }
        }
    }

    out.ok = true;
    return out;
}

}  // namespace onecad::ops::offsetface
