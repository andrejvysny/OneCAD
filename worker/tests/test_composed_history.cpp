// test_composed_history.cpp — WP3 C3.
//
// Part 1 CHARACTERIZES three OCCT facts that the OffsetFace DirectModeler V1
// history composition depends on, by driving the REAL chain V1 will drive:
//
//   box(10) --fillet R2 on one vertical edge--> PREDECESSOR
//     stage 1  BRepAlgoAPI_Defeaturing   removes the blend face
//     stage 2  BRepOffset_MakeOffset     pushes one support face +2
//     stage 3  BRepFilletAPI_MakeFillet  re-rounds the exposed sharp edge R2
//
// The three questions, asked of a `BRepTools_History` merged over those stages:
//   1. does `Merge` propagate REMOVAL across stages?
//   2. does the merged history retain INTERMEDIATE keys?
//   3. do the two SUPPORT faces stay `Modified`, or degrade to `Generated`?
//
// Fact 3 is the one with teeth: `ElementMapPartition.cpp` adds
// `kModifiedHistoryConfidence = 0.09` to a candidate whose lineage is `Modified`,
// and the auto-bind gate is score >= 0.85 AND margin >= 0.10. Losing `Modified`
// on a split therefore manufactures a spurious NeedsRepair.
//
// Part 2 asserts the CONTRACT of `elementmap::ComposedHistory` itself.
#include <algorithm>
#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

#include <BRepAdaptor_Curve.hxx>
#include <BRepAlgoAPI_Defeaturing.hxx>
#include <BRepFilletAPI_MakeFillet.hxx>
#include <BRepGProp.hxx>
#include <BRepOffset_MakeOffset.hxx>
#include <BRepTools_History.hxx>
#include <BRep_Tool.hxx>
#include <GProp_GProps.hxx>
#include <GeomAbs_CurveType.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedDataMapOfShapeListOfShape.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopTools_ListOfShape.hxx>
#include <TopoDS.hxx>

#include "elementmap/ComposedHistory.h"
#include "elementmap/ElementMapPartition.h"
#include "fillet_test_utils.h"
#include "ops/OffsetFaceOp.h"

namespace em = onecad::elementmap;
namespace ft = onecad::tests::fillet;
namespace km = onecad::kernel::elementmap;
namespace of = onecad::ops::offsetface;

namespace {

// Harness contract: the process exit code IS the failure count, so `check` is the
// ONLY thing that may report a failure and it always counts one. `checks_run` is
// printed with the summary so a run that silently skipped a section (an early
// return, a loop that never iterated) is visible as a dropped count rather than a
// vacuous green.
int failures = 0;
int checks_run = 0;

void check(bool condition, const std::string& message) {
    ++checks_run;
    if (condition) return;
    std::fprintf(stderr, "FAIL: %s\n", message.c_str());
    ++failures;
}

double volume(const TopoDS_Shape& shape) {
    GProp_GProps properties;
    BRepGProp::VolumeProperties(shape, properties);
    return properties.Mass();
}

std::vector<TopoDS_Shape> to_vec(const TopTools_ListOfShape& list) {
    std::vector<TopoDS_Shape> out;
    for (TopTools_ListIteratorOfListOfShape it(list); it.More(); it.Next()) out.push_back(it.Value());
    return out;
}

// The full three-stage chain plus every shape the assertions address.
struct Chain {
    bool ok = false;

    TopoDS_Shape sharp_box;  // box(10) — predecessor of the ONE-stage sub-chain
    TopoDS_Edge seed_edge;   // the vertical edge the first fillet consumes

    TopoDS_Shape predecessor;  // filleted box — the declared PREDECESSOR
    TopoDS_Face blend;         // the R2 blend face, removed by stage 1
    TopoDS_Face support_a;     // pushed by stage 2
    TopoDS_Face support_b;     // fixed
    TopoDS_Face top;           // an untouched face, control

    TopoDS_Shape suppressed;  // stage 1 out
    TopoDS_Edge sharp;        // TRANSIENT: born stage 1, consumed stage 3
    TopoDS_Shape offset;      // stage 2 out
    TopoDS_Edge sharp_offset; // TRANSIENT: the offset image of `sharp`
    TopoDS_Shape final_shape; // stage 3 out
    TopoDS_Face reblended;    // the face stage 3 generates from `sharp_offset`

    occ::handle<BRepTools_History> h1;
    occ::handle<BRepTools_History> h2;
    occ::handle<BRepTools_History> h3;
};

TopTools_ListOfShape one(const TopoDS_Shape& s) {
    TopTools_ListOfShape list;
    list.Append(s);
    return list;
}

Chain build_chain() {
    Chain c;

    // --- predecessor: box(10) with one vertical edge filleted R2 --------------
    c.sharp_box = ft::box();
    const std::vector<TopoDS_Edge> verticals = ft::vertical_edges(c.sharp_box);
    if (verticals.empty()) return c;
    c.seed_edge = verticals.front();
    BRepFilletAPI_MakeFillet fillet(c.sharp_box);
    fillet.Add(2.0, c.seed_edge);
    fillet.Build();
    if (!fillet.IsDone() || fillet.Shape().IsNull()) return c;
    c.predecessor = fillet.Shape();

    const std::vector<TopoDS_Shape> blends = to_vec(fillet.Generated(c.seed_edge));
    if (blends.size() != 1 || blends.front().ShapeType() != TopAbs_FACE) return c;
    c.blend = TopoDS::Face(blends.front());

    // The two supports are the faces on the far side of the blend's LINE edges
    // (its two arc edges lead to the top and bottom caps instead).
    TopTools_IndexedDataMapOfShapeListOfShape edge_faces;
    TopExp::MapShapesAndAncestors(c.predecessor, TopAbs_EDGE, TopAbs_FACE, edge_faces);
    std::vector<TopoDS_Face> supports;
    for (TopExp_Explorer ex(c.blend, TopAbs_EDGE); ex.More(); ex.Next()) {
        const TopoDS_Edge e = TopoDS::Edge(ex.Current());
        if (BRepAdaptor_Curve(e).GetType() != GeomAbs_Line) continue;
        for (const TopoDS_Shape& f : to_vec(edge_faces.FindFromKey(e))) {
            if (f.IsSame(c.blend)) continue;
            supports.push_back(TopoDS::Face(f));
        }
    }
    if (supports.size() != 2) return c;
    c.support_a = supports[0];
    c.support_b = supports[1];

    // A control face touched by neither the blend nor the push: the cap whose
    // plane is horizontal and farthest from the pushed support.
    for (TopExp_Explorer ex(c.predecessor, TopAbs_FACE); ex.More(); ex.Next()) {
        const TopoDS_Face f = TopoDS::Face(ex.Current());
        if (f.IsSame(c.support_a) || f.IsSame(c.support_b) || f.IsSame(c.blend)) continue;
        const of::PlaneInfo info = of::plane_info(f);
        if (info.ok && std::abs(info.normal.Z()) > 0.9) {
            c.top = f;
            break;
        }
    }
    if (c.top.IsNull()) return c;

    // --- stage 1: suppress the blend -----------------------------------------
    BRepAlgoAPI_Defeaturing defeature;
    defeature.SetShape(c.predecessor);
    defeature.AddFacesToRemove(one(c.blend));
    defeature.SetToFillHistory(Standard_True);
    defeature.SetRunParallel(Standard_False);
    defeature.Build();
    if (!defeature.IsDone() || defeature.Shape().IsNull()) return c;
    c.suppressed = defeature.Shape();
    c.h1 = new BRepTools_History(one(c.predecessor), defeature);

    const std::vector<TopoDS_Shape> a_img = to_vec(defeature.Modified(c.support_a));
    const std::vector<TopoDS_Shape> b_img = to_vec(defeature.Modified(c.support_b));
    if (a_img.size() != 1 || b_img.size() != 1) return c;
    const TopoDS_Face a1 = TopoDS::Face(a_img.front());
    const TopoDS_Face b1 = TopoDS::Face(b_img.front());

    // The exposed sharp edge is the edge the two reconstructed supports share.
    TopTools_IndexedMapOfShape b1_edges;
    TopExp::MapShapes(b1, TopAbs_EDGE, b1_edges);
    for (TopExp_Explorer ex(a1, TopAbs_EDGE); ex.More(); ex.Next()) {
        if (b1_edges.Contains(ex.Current())) {
            c.sharp = TopoDS::Edge(ex.Current());
            break;
        }
    }
    if (c.sharp.IsNull()) return c;

    // --- stage 2: push support A outward by +2 --------------------------------
    TopoDS_Shape build_input = c.suppressed;
    if (build_input.ShapeType() != TopAbs_SOLID) {
        TopExp_Explorer ex(c.suppressed, TopAbs_SOLID);
        if (!ex.More()) return c;
        build_input = ex.Current();
    }
    const double construction_tol = of::build_tolerance(build_input);
    BRepOffset_MakeOffset mo;
    mo.Initialize(build_input, /*Offset*/ 0.0, construction_tol, BRepOffset_Skin,
                  /*Intersection*/ Standard_False, /*SelfInter*/ Standard_False,
                  GeomAbs_Intersection, /*Thickening*/ Standard_False,
                  /*RemoveIntEdges*/ Standard_False);
    mo.AllowLinearization(Standard_False);
    mo.SetOffsetOnFace(a1, 2.0);
    mo.MakeOffsetShape();
    if (!mo.IsDone() || mo.Error() != BRepOffset_NoError || mo.Shape().IsNull()) return c;
    c.offset = mo.Shape();

    of::OffsetImageHistory offset_history(mo.OffsetFacesFromShapes(), mo.OffsetEdgesFromShapes(),
                                          c.offset);
    c.h2 = new BRepTools_History(one(c.suppressed), offset_history);

    const std::vector<TopoDS_Shape> sharp_img = to_vec(offset_history.Modified(c.sharp));
    if (sharp_img.size() != 1) return c;
    c.sharp_offset = TopoDS::Edge(sharp_img.front());

    // --- stage 3: re-round the exposed sharp edge -----------------------------
    BRepFilletAPI_MakeFillet reblend(c.offset);
    reblend.Add(2.0, c.sharp_offset);
    reblend.Build();
    if (!reblend.IsDone() || reblend.Shape().IsNull()) return c;
    c.final_shape = reblend.Shape();
    c.h3 = new BRepTools_History(one(c.offset), reblend);

    const std::vector<TopoDS_Shape> reblended = to_vec(reblend.Generated(c.sharp_offset));
    if (reblended.size() != 1 || reblended.front().ShapeType() != TopAbs_FACE) return c;
    c.reblended = TopoDS::Face(reblended.front());

    c.ok = true;
    return c;
}

// --- part 1: characterization ----------------------------------------------

const char* type_name(TopAbs_ShapeEnum t) {
    switch (t) {
        case TopAbs_FACE: return "F";
        case TopAbs_EDGE: return "E";
        case TopAbs_VERTEX: return "V";
        case TopAbs_SOLID: return "S";
        default: return "?";
    }
}

// Where a shape lives across the four chain shapes, as "<name>:<T><ordinal>".
struct ChainIndex {
    TopTools_IndexedMapOfShape maps[4];
    const char* names[4] = {"pred", "supp", "off", "fin"};

    explicit ChainIndex(const Chain& c) {
        TopExp::MapShapes(c.predecessor, maps[0]);
        TopExp::MapShapes(c.suppressed, maps[1]);
        TopExp::MapShapes(c.offset, maps[2]);
        TopExp::MapShapes(c.final_shape, maps[3]);
    }

    std::string locate(const TopoDS_Shape& s) const {
        std::string out = type_name(s.ShapeType());
        for (int i = 0; i < 4; ++i) {
            const int ord = maps[i].FindIndex(s);
            out += ord == 0 ? std::string(" ") + names[i] + ":-"
                            : std::string(" ") + names[i] + ":" + std::to_string(ord);
        }
        return out;
    }
};

void dump(const char* label, const BRepTools_History& h, const TopoDS_Shape& s,
          const ChainIndex& index) {
    std::fprintf(stderr, "%-26s removed=%-5s M=%d G=%d\n", label,
                 h.IsRemoved(s) ? "true" : "false", h.Modified(s).Extent(),
                 h.Generated(s).Extent());
    for (const TopoDS_Shape& m : to_vec(h.Modified(s)))
        std::fprintf(stderr, "%-26s   M -> %s\n", "", index.locate(m).c_str());
    for (const TopoDS_Shape& g : to_vec(h.Generated(s)))
        std::fprintf(stderr, "%-26s   G -> %s\n", "", index.locate(g).c_str());
}

bool list_contains(const TopTools_ListOfShape& list, const TopoDS_Shape& s) {
    for (const TopoDS_Shape& v : to_vec(list))
        if (v.IsSame(s)) return true;
    return false;
}

void characterize(const Chain& c) {
    const ChainIndex index(c);
    TopTools_IndexedMapOfShape final_faces;
    TopExp::MapShapes(c.final_shape, TopAbs_FACE, final_faces);

    std::fprintf(stderr, "\n=== chain fixture (real defeature -> offset -> re-fillet) ===\n");
    std::fprintf(stderr, "predecessor volume  = %.7f\n", volume(c.predecessor));
    std::fprintf(stderr, "suppressed  volume  = %.7f\n", volume(c.suppressed));
    std::fprintf(stderr, "offset      volume  = %.7f\n", volume(c.offset));
    std::fprintf(stderr, "final       volume  = %.7f\n", volume(c.final_shape));
    std::fprintf(stderr, "final face count    = %d\n", final_faces.Extent());
    std::fprintf(stderr, "blend       = %s\n", index.locate(c.blend).c_str());
    std::fprintf(stderr, "support_a   = %s\n", index.locate(c.support_a).c_str());
    std::fprintf(stderr, "support_b   = %s\n", index.locate(c.support_b).c_str());
    std::fprintf(stderr, "sharp       = %s\n", index.locate(c.sharp).c_str());
    std::fprintf(stderr, "sharp_offset= %s\n", index.locate(c.sharp_offset).c_str());

    std::fprintf(stderr, "\n=== per-stage histories ===\n");
    dump("h1(blend)", *c.h1, c.blend, index);
    dump("h1(support_a)", *c.h1, c.support_a, index);
    dump("h2(sharp)", *c.h2, c.sharp, index);
    dump("h3(sharp_offset)", *c.h3, c.sharp_offset, index);

    const TopoDS_Shape reblended = c.reblended;

    occ::handle<BRepTools_History> merged = new BRepTools_History();
    merged->Merge(c.h1);
    std::fprintf(stderr, "\n=== merged after h1 ===\n");
    dump("merged(support_a)", *merged, c.support_a, index);
    const bool sharp_in_g1 = list_contains(merged->Generated(c.support_a), c.sharp);
    std::fprintf(stderr, "sharp in G(support_a)        = %s\n", sharp_in_g1 ? "true" : "false");

    merged->Merge(c.h2);
    std::fprintf(stderr, "\n=== merged after h1+h2 ===\n");
    dump("merged(support_a)", *merged, c.support_a, index);
    dump("merged(sharp)", *merged, c.sharp, index);
    const bool sharp_off_in_g2 = list_contains(merged->Generated(c.support_a), c.sharp_offset);
    std::fprintf(stderr, "sharp_offset in G(support_a) = %s\n", sharp_off_in_g2 ? "true" : "false");

    merged->Merge(c.h3);
    std::fprintf(stderr, "\n=== merged after h1+h2+h3 ===\n");
    dump("merged(blend)", *merged, c.blend, index);
    dump("merged(support_a)", *merged, c.support_a, index);
    dump("merged(support_b)", *merged, c.support_b, index);
    dump("merged(top)", *merged, c.top, index);
    dump("merged(sharp)", *merged, c.sharp, index);
    dump("merged(sharp_offset)", *merged, c.sharp_offset, index);

    std::fprintf(stderr, "\n=== FACT 1: does Merge propagate REMOVAL across stages? ===\n");
    std::fprintf(stderr, "  stage-1 removal survives later merges : h1.IsRemoved(blend)=%s -> "
                         "merged.IsRemoved(blend)=%s\n",
                 c.h1->IsRemoved(c.blend) ? "true" : "false",
                 merged->IsRemoved(c.blend) ? "true" : "false");
    std::fprintf(stderr, "  stage-3 removal of an intermediate    : h3.IsRemoved(sharp_offset)=%s, "
                         "reblended face=%s\n",
                 c.h3->IsRemoved(c.sharp_offset) ? "true" : "false",
                 reblended.IsNull() ? "none" : index.locate(reblended).c_str());
    std::fprintf(stderr, "  does the reblended face reach G(support_a)? %s\n",
                 (!reblended.IsNull() && list_contains(merged->Generated(c.support_a), reblended))
                     ? "YES"
                     : "NO");

    std::fprintf(stderr, "\n=== FACT 2: does Merge retain INTERMEDIATE keys? ===\n");
    std::fprintf(stderr, "  stage-1-born transient `sharp`  (gone by stage 3): M=%d G=%d removed=%s\n",
                 merged->Modified(c.sharp).Extent(), merged->Generated(c.sharp).Extent(),
                 merged->IsRemoved(c.sharp) ? "true" : "false");
    std::fprintf(stderr, "  stage-2-born transient `sharp_offset`            : M=%d G=%d removed=%s\n",
                 merged->Modified(c.sharp_offset).Extent(),
                 merged->Generated(c.sharp_offset).Extent(),
                 merged->IsRemoved(c.sharp_offset) ? "true" : "false");

    std::fprintf(stderr, "\n=== FACT 3: do SUPPORT faces stay Modified? ===\n");
    for (const auto& named : {std::make_pair("support_a", c.support_a),
                              std::make_pair("support_b", c.support_b),
                              std::make_pair("top", c.top)}) {
        int m_face_in_final = 0;
        int g_face_in_final = 0;
        for (const TopoDS_Shape& s : to_vec(merged->Modified(named.second)))
            if (final_faces.Contains(s)) ++m_face_in_final;
        for (const TopoDS_Shape& s : to_vec(merged->Generated(named.second)))
            if (final_faces.Contains(s)) ++g_face_in_final;
        std::fprintf(stderr,
                     "  merged(%-9s): M=%d (faces in final %d)  G=%d (faces in final %d)\n",
                     named.first, merged->Modified(named.second).Extent(), m_face_in_final,
                     merged->Generated(named.second).Extent(), g_face_in_final);
    }
    std::fprintf(stderr, "\n");
}

// --- part 2: the ComposedHistory contract -----------------------------------

em::ComposedHistory compose(const Chain& c);

// FACT 1 and FACT 3, as ASSERTIONS rather than printout. This is the test that can
// tell the hand-written walk apart from a `Merge`-based implementation: it pins
// BOTH sides of the divergence, so a future OCCT release that fixes `Merge` reds
// here loudly instead of silently changing what the element map is handed.
void merge_divergence_is_pinned(const Chain& c) {
    occ::handle<BRepTools_History> merged = new BRepTools_History();
    merged->Merge(c.h1);
    merged->Merge(c.h2);
    merged->Merge(c.h3);

    // FACT 1, first half: a stage-1 removal survives later merges.
    check(c.h1->IsRemoved(c.blend) && merged->IsRemoved(c.blend),
          "fact 1: a stage-1 Removed survives two further merges");

    // FACT 1, second half: a LATER-stage removal truncates the generated chain and
    // its own removal flag is dropped. `Merge` loses the re-blended face; the walk
    // keeps it. Both directions are asserted.
    check(c.h3->IsRemoved(c.sharp_offset),
          "fact 1: stage 3 positively removes the seed edge it consumes");
    check(!merged->IsRemoved(c.sharp_offset),
          "fact 1: Merge DROPS that removal flag (pinned OCCT behavior)");
    check(list_contains(c.h3->Generated(c.sharp_offset), c.reblended),
          "fact 1: stage 3 generates the re-blended face from the seed edge");
    check(!list_contains(merged->Generated(c.support_a), c.reblended),
          "fact 1: Merge LOSES the re-blended face from G(support_a) (pinned OCCT behavior)");

    const em::ComposedHistory composed = compose(c);
    const std::vector<TopoDS_Shape> composed_generated = composed.generated(c.support_a);
    check(std::any_of(composed_generated.begin(), composed_generated.end(),
                      [&c](const TopoDS_Shape& s) { return s.IsSame(c.reblended); }),
          "fact 1: the composed walk KEEPS the re-blended face in generated(support_a)");

    // FACT 2: the merged handle leaks the stage-2 transient as a queryable key,
    // while the stage-1 transient is not a key at all.
    check(merged->Generated(c.sharp_offset).Extent() == 1,
          "fact 2: Merge retains the LAST stage's argument sub-shape as a key");
    check(merged->Modified(c.sharp).IsEmpty() && merged->Generated(c.sharp).IsEmpty(),
          "fact 2: Merge does NOT retain the stage-1-born transient as a key");

    // FACT 3: both supports and an untouched control face keep exactly one MODIFIED
    // image, so `kModifiedHistoryConfidence` still applies across the chain.
    TopTools_IndexedMapOfShape final_faces;
    TopExp::MapShapes(c.final_shape, TopAbs_FACE, final_faces);
    for (const auto& named : {std::make_pair("support_a", c.support_a),
                              std::make_pair("support_b", c.support_b),
                              std::make_pair("top", c.top)}) {
        const std::vector<TopoDS_Shape> m = to_vec(merged->Modified(named.second));
        check(m.size() == 1 && final_faces.Contains(m.front()),
              std::string("fact 3 (Merge): ") + named.first +
                  " keeps exactly one MODIFIED face in the final shape");
        check(!merged->IsRemoved(named.second),
              std::string("fact 3 (Merge): ") + named.first + " is not removed");
    }
}

em::ComposedHistory compose(const Chain& c) {
    em::ComposedHistory composed(c.predecessor, c.final_shape);
    composed.add_stage(c.h1);
    composed.add_stage(c.h2);
    composed.add_stage(c.h3);
    return composed;
}

std::vector<int> ordinals(const std::vector<TopoDS_Shape>& shapes,
                          const TopTools_IndexedMapOfShape& map) {
    std::vector<int> out;
    for (const TopoDS_Shape& s : shapes) out.push_back(map.FindIndex(s));
    return out;
}

std::string join(const std::vector<int>& values) {
    std::string out;
    for (std::size_t i = 0; i < values.size(); ++i) {
        if (i != 0) out += ",";
        out += std::to_string(values[i]);
    }
    return out.empty() ? "-" : out;
}

// Contract (a): an intermediate-stage transient is UNANSWERABLE. It is in neither
// the predecessor nor the final shape, so nothing can ever publish it.
void transients_are_unanswerable(const Chain& c) {
    const em::ComposedHistory composed = compose(c);
    for (const auto& named : {std::make_pair("sharp (stage-1 born)", TopoDS_Shape(c.sharp)),
                              std::make_pair("sharp_offset (stage-2 born)",
                                             TopoDS_Shape(c.sharp_offset))}) {
        check(composed.modified(named.second).empty(),
              std::string("transient ") + named.first + ": no modified successors");
        check(composed.generated(named.second).empty(),
              std::string("transient ") + named.first + ": no generated successors");
        check(!composed.is_deleted(named.second),
              std::string("transient ") + named.first + ": not reported deleted");
    }
    // The `Merge` leak fact 2 measured: the merged handle DOES answer for
    // `sharp_offset`. The predecessor scope is what contains it.
    occ::handle<BRepTools_History> merged = new BRepTools_History();
    merged->Merge(c.h1);
    merged->Merge(c.h2);
    merged->Merge(c.h3);
    check(merged->Generated(c.sharp_offset).Extent() == 1,
          "characterization: the merged handle leaks the stage-2 transient as a key");
}

// Contract (b): support successors are deterministic and final-ordinal sorted.
// `c` and `twin` are INDEPENDENT constructions of the same chain — separate OCCT
// runs producing separate TShapes — so the comparison is of final-shape ORDINAL
// VECTORS, which is the only thing that survives a rebuild and the only thing the
// confidence gate consumes.
void support_successors_are_deterministic(const Chain& c, const Chain& twin) {
    TopTools_IndexedMapOfShape final_subs;
    TopExp::MapShapes(c.final_shape, final_subs);
    TopTools_IndexedMapOfShape twin_subs;
    TopExp::MapShapes(twin.final_shape, twin_subs);

    const em::ComposedHistory first = compose(c);
    const em::ComposedHistory second = compose(twin);

    for (const auto& named : {std::make_pair("support_a", std::make_pair(c.support_a, twin.support_a)),
                              std::make_pair("support_b",
                                             std::make_pair(c.support_b, twin.support_b))}) {
        const std::vector<int> m1 = ordinals(first.modified(named.second.first), final_subs);
        const std::vector<int> m2 = ordinals(second.modified(named.second.second), twin_subs);
        const std::vector<int> g1 = ordinals(first.generated(named.second.first), final_subs);
        const std::vector<int> g2 = ordinals(second.generated(named.second.second), twin_subs);
        std::fprintf(stderr, "composed(%-9s): M=[%s] G=[%s]\n", named.first, join(m1).c_str(),
                     join(g1).c_str());

        check(m1 == m2 && g1 == g2,
              std::string("determinism: ") + named.first +
                  " successor ordinals are identical across two INDEPENDENT chains");
        check(std::is_sorted(m1.begin(), m1.end()) && std::is_sorted(g1.begin(), g1.end()),
              std::string("order: ") + named.first + " successors are final-ordinal sorted");
        check(std::adjacent_find(m1.begin(), m1.end()) == m1.end(),
              std::string("dedup: ") + named.first + " modified list has no repeats");
        check(!m1.empty() && std::find(m1.begin(), m1.end(), 0) == m1.end(),
              std::string("presence: ") + named.first +
                  " keeps a MODIFIED image inside the final shape");
        check(std::find(g1.begin(), g1.end(), 0) == g1.end(),
              std::string("presence: ") + named.first +
                  " generated successors are all inside the final shape");
        check(!first.is_deleted(named.second.first),
              std::string("deletion: ") + named.first + " is not deleted");
    }

    // Fact 3, asserted rather than only printed: the supports reach the final shape
    // through the MODIFIED channel, so `kModifiedHistoryConfidence` still applies.
    TopTools_IndexedMapOfShape final_faces;
    TopExp::MapShapes(c.final_shape, TopAbs_FACE, final_faces);
    for (const auto& named : {std::make_pair("support_a", c.support_a),
                              std::make_pair("support_b", c.support_b),
                              std::make_pair("top", c.top)}) {
        const std::vector<TopoDS_Shape> m = first.modified(named.second);
        check(m.size() == 1 && final_faces.Contains(m.front()),
              std::string("fact 3: ") + named.first +
                  " has exactly one MODIFIED face image in the final shape");
    }
}

// Contract (c): positive evidence deletes; everything else does not.
void deletion_needs_positive_evidence(const Chain& c) {
    const em::ComposedHistory composed = compose(c);

    check(composed.is_deleted(c.blend),
          "deletion: the suppressed blend face reports is_deleted (stage-1 Removed, no successor)");
    check(composed.modified(c.blend).empty() && composed.generated(c.blend).empty(),
          "deletion: the suppressed blend face has no successors");

    // A shape from an unrelated body: outside the predecessor, so unanswerable.
    const TopoDS_Shape stranger = ft::box(3.0);
    TopTools_IndexedMapOfShape stranger_faces;
    TopExp::MapShapes(stranger, TopAbs_FACE, stranger_faces);
    check(stranger_faces.Extent() > 0, "fixture: the unrelated box has faces");
    const TopoDS_Shape bogus = stranger_faces(1);
    check(composed.modified(bogus).empty() && composed.generated(bogus).empty(),
          "bogus query: no successors");
    check(!composed.is_deleted(bogus), "bogus query: not reported deleted");
    check(!composed.is_deleted(TopoDS_Shape()), "null query: not reported deleted");

    // UNSUPPORTED TYPE is not DELETED. `BRepTools_History` only keys vertices,
    // edges, faces and solids and its readers ASSERT on anything else, so a
    // predecessor WIRE is screened out by `answerable()` rather than walked. The
    // answer is the same in a release and a `_DEBUG` OCCT build: empty successors
    // plus not-deleted, which routes `apply_history` to NeedsRepair.
    TopTools_IndexedMapOfShape predecessor_wires;
    TopExp::MapShapes(c.predecessor, TopAbs_WIRE, predecessor_wires);
    check(predecessor_wires.Extent() > 0, "fixture: the predecessor has wires");
    if (predecessor_wires.Extent() > 0) {
        const TopoDS_Shape wire = predecessor_wires(1);
        check(composed.modified(wire).empty() && composed.generated(wire).empty(),
              "unsupported-type predecessor sub-shape: no successors");
        check(!composed.is_deleted(wire),
              "unsupported-type predecessor sub-shape: not reported deleted");
    }
}

// Contract (b) over the WHOLE predecessor, not just two hand-picked faces: no query
// may ever answer with a shape that is not in the final body. This is the sweep that
// makes the final-shape presence filter load-bearing on NATIVE lineage — a sub-shape
// no stage ever keyed is carried verbatim by the walk and would otherwise be handed
// to `apply_history` as a successor belonging to the OLD body.
void every_query_stays_inside_the_final_body(const Chain& c) {
    const em::ComposedHistory composed = compose(c);
    TopTools_IndexedMapOfShape predecessor_subs;
    TopExp::MapShapes(c.predecessor, predecessor_subs);
    TopTools_IndexedMapOfShape final_subs;
    TopExp::MapShapes(c.final_shape, final_subs);

    int keyable = 0;
    int answered = 0;
    int deleted = 0;
    int unresolvable = 0;
    int stray = 0;
    for (int i = 1; i <= predecessor_subs.Extent(); ++i) {
        const TopoDS_Shape s = predecessor_subs(i);
        if (!BRepTools_History::IsSupportedType(s)) continue;
        ++keyable;
        std::vector<TopoDS_Shape> successors = composed.modified(s);
        for (const TopoDS_Shape& g : composed.generated(s)) successors.push_back(g);
        for (const TopoDS_Shape& t : successors) {
            if (final_subs.FindIndex(t) == 0) ++stray;
        }
        if (!successors.empty()) {
            ++answered;
            check(!composed.is_deleted(s), "sweep: a shape with successors is never deleted");
        } else if (composed.is_deleted(s)) {
            ++deleted;
        } else {
            ++unresolvable;
        }
    }
    std::fprintf(stderr,
                 "sweep: %d keyable predecessor sub-shapes — %d answered, %d deleted, "
                 "%d unresolvable, %d stray\n",
                 keyable, answered, deleted, unresolvable, stray);
    check(stray == 0, "sweep: no query answers with a shape outside the FINAL body");
    check(deleted > 0, "sweep: the chain positively deletes at least one sub-shape");
    check(answered > 0, "sweep: the chain rebinds at least one sub-shape");
}

// `add_modified` is answered, and filtered/sorted exactly like native lineage.
void injected_lineage_is_answered(const Chain& c) {
    TopTools_IndexedMapOfShape final_subs;
    TopExp::MapShapes(c.final_shape, final_subs);
    const std::vector<TopoDS_Shape> reblended{c.reblended};

    const em::ComposedHistory before = compose(c);
    check(before.modified(c.blend).empty() && before.is_deleted(c.blend),
          "baseline: without injection the blend face is deleted with no lineage");

    em::ComposedHistory bridged = compose(c);
    bridged.add_modified(c.blend, reblended.front());
    bridged.add_modified(c.blend, reblended.front());  // idempotent
    const std::vector<TopoDS_Shape> images = bridged.modified(c.blend);
    check(images.size() == 1 && images.front().IsSame(reblended.front()),
          "injection: add_modified is answered by modified()");
    check(!bridged.is_deleted(c.blend), "injection: an injected mapping cancels is_deleted");
    check(bridged.generated(c.blend).empty(), "injection: nothing lands in generated()");

    // Filtering and ordering apply to injected pairs exactly as to native ones.
    em::ComposedHistory filtered = compose(c);
    filtered.add_modified(c.blend, c.sharp);  // a transient: not in the final shape
    check(filtered.modified(c.blend).empty(),
          "injection: a successor outside the final shape is filtered out");

    em::ComposedHistory ordered = compose(c);
    TopTools_IndexedMapOfShape final_faces;
    TopExp::MapShapes(c.final_shape, TopAbs_FACE, final_faces);
    std::vector<TopoDS_Shape> injected;
    for (int i = final_faces.Extent(); i >= 1; --i) injected.push_back(final_faces(i));
    for (const TopoDS_Shape& s : injected) ordered.add_modified(c.blend, s);
    const std::vector<int> got = ordinals(ordered.modified(c.blend), final_subs);
    check(std::is_sorted(got.begin(), got.end()),
          "injection: successors added in reverse order come back final-ordinal sorted");
    check(got.size() == static_cast<std::size_t>(final_faces.Extent()),
          "injection: every injected successor present in the final shape is reported");
    std::fprintf(stderr, "injected (reverse order in) -> [%s]\n", join(got).c_str());

    // The transient stays unanswerable even as an injection KEY (contract (a)).
    em::ComposedHistory scoped = compose(c);
    scoped.add_modified(c.sharp, reblended.front());
    check(scoped.modified(c.sharp).empty(),
          "injection: a non-predecessor key stays unanswerable");
}

// A ONE-stage chain whose lineage runs through a pure GENERATED hop: the fillet
// consumes the seed edge and generates the blend face from it. Without this the
// `generated()` channel has no load-bearing coverage — every other assertion in
// this file is satisfied by an empty list.
//
// This is also the only instantiation of the TEMPLATE `add_stage(shape, algo)`
// overload, and it is asserted to agree exactly with the handle overload, so both
// ingestion paths are type-checked and pinned against each other.
void generated_lineage_and_template_stage(const Chain& c) {
    BRepFilletAPI_MakeFillet fillet(c.sharp_box);
    fillet.Add(2.0, c.seed_edge);
    fillet.Build();
    check(fillet.IsDone() && !fillet.Shape().IsNull(), "fixture: the one-stage fillet builds");
    if (!fillet.IsDone() || fillet.Shape().IsNull()) return;
    const TopoDS_Shape filleted = fillet.Shape();

    const std::vector<TopoDS_Shape> blends = to_vec(fillet.Generated(c.seed_edge));
    check(blends.size() == 1, "fixture: the seed edge generates exactly one blend face");
    if (blends.size() != 1) return;
    const TopoDS_Shape blend = blends.front();

    // TEMPLATE overload: ingest the live algorithm directly.
    em::ComposedHistory via_template(c.sharp_box, filleted);
    via_template.add_stage(c.sharp_box, fillet);

    // Handle overload over the same algorithm, for the equivalence assertion.
    em::ComposedHistory via_handle(c.sharp_box, filleted);
    via_handle.add_stage(occ::handle<BRepTools_History>(
        new BRepTools_History(one(c.sharp_box), fillet)));

    TopTools_IndexedMapOfShape filleted_subs;
    TopExp::MapShapes(filleted, filleted_subs);

    // The GENERATED channel carries the blend face, and the MODIFIED channel does
    // not — a shape may never sit on both.
    const std::vector<TopoDS_Shape> generated = via_template.generated(c.seed_edge);
    check(generated.size() == 1 && generated.front().IsSame(blend),
          "generated: the seed edge's blend face arrives on the generated channel");
    check(via_template.modified(c.seed_edge).empty(),
          "generated: the seed edge has nothing on the modified channel");
    check(!via_template.is_deleted(c.seed_edge),
          "generated: a consumed edge WITH a successor is not reported deleted");
    std::fprintf(stderr, "one-stage generated(seed_edge) -> [%s]\n",
                 join(ordinals(generated, filleted_subs)).c_str());

    // The two supports come through as MODIFIED on the same chain, so the fixture
    // separates the channels rather than collapsing both into one.
    TopTools_IndexedMapOfShape box_faces;
    TopExp::MapShapes(c.sharp_box, TopAbs_FACE, box_faces);
    int modified_faces = 0;
    for (int i = 1; i <= box_faces.Extent(); ++i) {
        const std::vector<TopoDS_Shape> m = via_template.modified(box_faces(i));
        if (m.size() == 1 && filleted_subs.Contains(m.front())) ++modified_faces;
        check(via_template.generated(box_faces(i)).empty() ||
                  !m.empty(),  // a face is never generated-only here
              "generated: a box face is not reclassified onto the generated channel");
    }
    check(modified_faces == box_faces.Extent(),
          "generated: every box face keeps exactly one MODIFIED image");

    // Both ingestion paths must answer identically.
    check(ordinals(via_template.generated(c.seed_edge), filleted_subs) ==
              ordinals(via_handle.generated(c.seed_edge), filleted_subs),
          "template stage: agrees with the handle overload on generated()");
    check(ordinals(via_template.modified(box_faces(1)), filleted_subs) ==
              ordinals(via_handle.modified(box_faces(1)), filleted_subs),
          "template stage: agrees with the handle overload on modified()");

    // INJECTION REPLACES CLASSIFICATION. The walk already reaches `blend` as a
    // GENERATED successor of the seed edge; asserting it as Modified must move it,
    // not duplicate it. One shape on both channels would violate G ^ M == 0 and
    // would double-count the same candidate in `apply_history`'s split gate.
    em::ComposedHistory promoted(c.sharp_box, filleted);
    promoted.add_stage(c.sharp_box, fillet);
    promoted.add_modified(c.seed_edge, blend);
    const std::vector<TopoDS_Shape> promoted_modified = promoted.modified(c.seed_edge);
    check(promoted_modified.size() == 1 && promoted_modified.front().IsSame(blend),
          "injection: a natively GENERATED successor is promoted onto modified()");
    check(promoted.generated(c.seed_edge).empty(),
          "injection: the promoted successor LEAVES the generated channel");
}

// FIX 5's seam: the production consumer is `ElementMapPartition::apply_history`,
// which takes the `BRepBuilderAPI_MakeShape` overrides. Drive it end to end.
void apply_history_consumes_the_composed_chain(const Chain& c) {
    em::ElementMapPartition part;
    const km::ElementDescriptor support_descriptor =
        em::ElementMapPartition::describe(c.support_a);
    part.mint("A", "el_support_a", km::ElementKind::Face, c.support_a, c.predecessor,
              nlohmann::json{{"worldPoint",
                              {support_descriptor.center.X(), support_descriptor.center.Y(),
                               support_descriptor.center.Z()}}});
    part.mint("A", "el_blend", km::ElementKind::Face, c.blend, c.predecessor);

    // An entry bound to a shape the composed chain cannot answer for at all
    // (contract (a)): outside the predecessor, so no successors and no deletion
    // evidence. It must reach NeedsRepair, not a stale bind and not a silent drop.
    const TopoDS_Shape stranger = ft::box(3.0);
    TopTools_IndexedMapOfShape stranger_faces;
    TopExp::MapShapes(stranger, TopAbs_FACE, stranger_faces);
    check(stranger_faces.Extent() > 0, "fixture: the unrelated box has faces");
    if (stranger_faces.Extent() == 0) return;
    part.mint("A", "el_foreign", km::ElementKind::Face, stranger_faces(1), c.predecessor);

    em::ComposedHistory composed = compose(c);
    em::ElementMapDelta delta;
    std::vector<nlohmann::json> needs_repair;
    part.apply_history("A", c.final_shape, composed, delta, &needs_repair);

    // The support face rebinds through the chain onto a real face of the final body.
    const em::PartitionEntry* support = part.find("el_support_a");
    check(support != nullptr, "apply_history: the support face survives the chain");
    if (support != nullptr) {
        const TopoDS_Shape bound = em::ElementMapPartition::shape_for_topokey(
            c.final_shape, support->topo_key);
        check(!bound.IsNull() && bound.IsSame(support->shape),
              "apply_history: the support's TopoKey resolves inside the FINAL body");
        std::fprintf(stderr, "apply_history: el_support_a -> %s\n", support->topo_key.c_str());
    }

    // The suppressed blend face is positively deleted: removed, never repaired.
    check(!part.contains("el_blend"), "apply_history: the suppressed blend face is dropped");
    check(std::find(delta.removed.begin(), delta.removed.end(), "el_blend") != delta.removed.end(),
          "apply_history: the suppressed blend face lands in delta.removed");

    // The unresolvable entry becomes NeedsRepair STATE — it is never silently
    // erased without a repair item, and it never binds to a stale shape.
    const bool foreign_repair =
        std::any_of(needs_repair.begin(), needs_repair.end(), [](const nlohmann::json& item) {
            return item.value("elementId", "") == "el_foreign";
        });
    check(foreign_repair, "apply_history: the unanswerable entry emits NeedsRepair");
    check(std::find(delta.removed.begin(), delta.removed.end(), "el_foreign") ==
              delta.removed.end(),
          "apply_history: the unanswerable entry is NOT reported as a clean removal");
    for (const nlohmann::json& item : needs_repair) {
        check(item.value("elementId", "") != "el_support_a",
              "apply_history: a cleanly rebound support never needs repair");
    }
}

}  // namespace

int main() {
    const Chain chain = build_chain();
    check(chain.ok, "fixture: the three-stage chain builds");
    if (!chain.ok) return failures;

    // A second, INDEPENDENT run of the same chain — the determinism assertion has to
    // compare two separate OCCT constructions, not two views of one set of handles.
    const Chain twin = build_chain();
    check(twin.ok, "fixture: the independent twin chain builds");
    if (!twin.ok) return failures;

    characterize(chain);

    merge_divergence_is_pinned(chain);
    transients_are_unanswerable(chain);
    support_successors_are_deterministic(chain, twin);
    every_query_stays_inside_the_final_body(chain);
    deletion_needs_positive_evidence(chain);
    injected_lineage_is_answered(chain);
    generated_lineage_and_template_stage(chain);
    apply_history_consumes_the_composed_chain(chain);

    std::fprintf(stderr, "composed history: %d checks run, %d failed\n", checks_run, failures);
    return failures;
}
