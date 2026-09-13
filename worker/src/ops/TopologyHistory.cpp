#include "ops/TopologyHistory.h"

#include <array>

#include <TopAbs_ShapeEnum.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopTools_ListOfShape.hxx>

#include "util/Log.h"

namespace onecad::ops {
namespace {

constexpr std::array<TopAbs_ShapeEnum, 3> kReferenceKinds{
    TopAbs_FACE, TopAbs_EDGE, TopAbs_VERTEX};

// Certified boundary completion for ONE operation. `active` is false unless the
// adapter supplied a certificate AND `accounted_vanishing` held, so nothing here
// can fire on a Boolean. Every predicate is IsSame containment — no epsilon.
struct BoundaryCompletion {
    bool active = false;
    int witness_faces = 0;
    int completed_edges = 0;
    int completed_vertices = 0;
    int contour_edges = 0;
    // Every edge and every vertex of every witness face. Blends report no vertex
    // history at all (`Generated(vertex)` is empty for every vertex), so an orphan
    // vertex is certified purely by witness-face membership — which already covers
    // the vertices of every witness edge, those being vertices of the same face.
    TopTools_IndexedMapOfShape witness_edges;
    TopTools_IndexedMapOfShape witness_vertices;

    bool covers(TopAbs_ShapeEnum kind, const TopoDS_Shape& output) const {
        if (!active) return false;
        if (kind == TopAbs_EDGE) return witness_edges.Contains(output);
        if (kind == TopAbs_VERTEX) return witness_vertices.Contains(output);
        return false;
    }
};

void mark_generated(const TopTools_ListOfShape& generated,
                    const TopTools_IndexedMapOfShape& outputs,
                    std::vector<bool>& births) {
    for (const TopoDS_Shape& shape : generated) {
        const int index = outputs.FindIndex(shape);
        if (index > 0) births[static_cast<std::size_t>(index - 1)] = true;
    }
}

// True when `reported` names at least one shape that is NOT `input` itself and is
// LIVE in `after`. Evaluated on one list at a time and never held across another
// builder call: `Modified` and `Generated` may return references into storage the
// next query reuses.
bool names_live_successor(const TopTools_ListOfShape& reported, const TopoDS_Shape& input,
                          const TopTools_IndexedMapOfShape& after) {
    for (const TopoDS_Shape& shape : reported) {
        if (!shape.IsSame(input) && after.Contains(shape)) return true;
    }
    return false;
}

// The fail-closed half of the certificate. A before FACE or EDGE gone from
// `result` must be accounted for by a report the operation can be held to: some
// shape in `Modified(b)` or `Generated(b)` that is neither `b` itself nor absent
// from the result. NON-EMPTINESS IS NOT ACCOUNTING — a list holding only `b`, or
// only a shape that never reached `result`, says nothing about where `b`'s
// boundary went, and on IsSame/Modified/Generated/containment alone that is
// indistinguishable from an unreported replacement of host topology. One such
// shape disqualifies the whole operation.
bool accounted_vanishing(const TopoDS_Shape& before_shape,
                         const TopTools_IndexedMapOfShape& after,
                         BRepBuilderAPI_MakeShape& builder) {
    for (const TopAbs_ShapeEnum kind : {TopAbs_FACE, TopAbs_EDGE}) {
        TopTools_IndexedMapOfShape before;
        TopExp::MapShapes(before_shape, kind, before);
        for (int i = 1; i <= before.Extent(); ++i) {
            const TopoDS_Shape& input = before(i);
            if (after.Contains(input)) continue;
            if (names_live_successor(builder.Modified(input), input, after)) continue;
            if (names_live_successor(builder.Generated(input), input, after)) continue;
            return false;
        }
    }
    return true;
}

// Witness faces W, the derivation's F = { f : D(f) and P(f) = empty }: every live
// face of `result` the builder reports as `Generated` from ANY before root —
// face, edge or vertex — that is not IsSame a before face and is not the
// `Modified` output of one. `Generated` proves the operation made the face; the
// absent predecessor proves it is not a rebuilt host face, and membership in a
// Modified host face never confers ownership.
//
// The root is deliberately NOT restricted to the certificate's contour edges:
// three blends meeting at one corner make OCCT report the corner patch under
// `Generated(vertex)`, and that patch's own seam edge lies on no contour edge's
// face, so a contour-only rule leaves it Unknown (measured: 1 orphan edge left on
// a 10 mm cube blended r=1 on three edges of one corner). `certificate` stays the
// blend adapter's assertion that this op may complete at all — a Boolean passes
// none and therefore never reaches here.
BoundaryCompletion certified_completion(
    const TopoDS_Shape& before_shape, const TopoDS_Shape& result,
    BRepBuilderAPI_MakeShape& builder,
    const BoundaryCompletionCertificate& certificate) {
    BoundaryCompletion completion;
    completion.contour_edges = static_cast<int>(certificate.contour_edges.size());
    TopTools_IndexedMapOfShape after;
    TopExp::MapShapes(result, after);
    if (!accounted_vanishing(before_shape, after, builder)) return completion;
    TopTools_IndexedMapOfShape before_faces;
    TopTools_IndexedMapOfShape after_faces;
    TopExp::MapShapes(before_shape, TopAbs_FACE, before_faces);
    TopExp::MapShapes(result, TopAbs_FACE, after_faces);
    TopTools_IndexedMapOfShape modified_faces;
    for (int i = 1; i <= before_faces.Extent(); ++i) {
        for (const TopoDS_Shape& modified : builder.Modified(before_faces(i))) {
            if (!modified.IsSame(before_faces(i))) modified_faces.Add(modified);
        }
    }
    TopTools_IndexedMapOfShape witnesses;
    for (const TopAbs_ShapeEnum root_kind : kReferenceKinds) {
        TopTools_IndexedMapOfShape roots;
        TopExp::MapShapes(before_shape, root_kind, roots);
        for (int i = 1; i <= roots.Extent(); ++i) {
            for (const TopoDS_Shape& generated : builder.Generated(roots(i))) {
                if (generated.ShapeType() != TopAbs_FACE) continue;
                if (!after_faces.Contains(generated)) continue;
                if (before_faces.Contains(generated)) continue;
                if (modified_faces.Contains(generated)) continue;
                witnesses.Add(generated);
            }
        }
    }
    for (int i = 1; i <= witnesses.Extent(); ++i) {
        TopExp::MapShapes(witnesses(i), TopAbs_EDGE, completion.witness_edges);
        TopExp::MapShapes(witnesses(i), TopAbs_VERTEX, completion.witness_vertices);
    }
    completion.witness_faces = witnesses.Extent();
    completion.active = witnesses.Extent() > 0;
    return completion;
}

void classify_kind(session::TopologyBodyHistory& history,
                   const TopoDS_Shape& before_shape,
                   const std::vector<TopoDS_Shape>& birth_roots,
                   TopAbs_ShapeEnum kind, const TopoDS_Shape& result,
                   BRepBuilderAPI_MakeShape& builder,
                   BoundaryCompletion& completion) {
    TopTools_IndexedMapOfShape before;
    TopTools_IndexedMapOfShape outputs;
    TopExp::MapShapes(before_shape, kind, before);
    TopExp::MapShapes(result, kind, outputs);
    std::vector<std::vector<TopoDS_Shape>> predecessors(outputs.Extent());
    std::vector<bool> births(outputs.Extent(), false);
    for (int i = 1; i <= before.Extent(); ++i) {
        const TopoDS_Shape& input = before(i);
        int output = outputs.FindIndex(input);
        if (output > 0) predecessors[static_cast<std::size_t>(output - 1)].push_back(input);
        for (const TopoDS_Shape& modified : builder.Modified(input)) {
            output = outputs.FindIndex(modified);
            if (output > 0 && !modified.IsSame(input))
                predecessors[static_cast<std::size_t>(output - 1)].push_back(input);
        }
    }
    for (const TopAbs_ShapeEnum root_kind : kReferenceKinds) {
        TopTools_IndexedMapOfShape roots;
        TopExp::MapShapes(before_shape, root_kind, roots);
        for (int i = 1; i <= roots.Extent(); ++i)
            mark_generated(builder.Generated(roots(i)), outputs, births);
    }
    for (const TopoDS_Shape& root : birth_roots) {
        mark_generated(builder.Generated(root), outputs, births);
        mark_generated(builder.Modified(root), outputs, births);
        const int output = outputs.FindIndex(root);
        if (output > 0) births[static_cast<std::size_t>(output - 1)] = true;
    }
    for (int i = 1; i <= outputs.Extent(); ++i) {
        auto& prior = predecessors[static_cast<std::size_t>(i - 1)];
        const TopoDS_Shape& output = outputs(i);
        if (births[static_cast<std::size_t>(i - 1)] && !prior.empty())
            history.conflicts.push_back({output});
        else if (births[static_cast<std::size_t>(i - 1)]) history.births.push_back({output});
        else if (prior.size() == 1 && output.IsSame(prior.front()))
            history.survivors.push_back({prior.front(), output});
        else if (!prior.empty()) history.successors.push_back({std::move(prior), output});
        else if (completion.covers(kind, output)) {
            // Certified boundary completion. Exact lineage is exhausted above, so
            // this only ever sees topology OCCT reported nothing for, and it can
            // therefore never conflict with an inherited child.
            history.births.push_back({output});
            if (kind == TopAbs_EDGE) ++completion.completed_edges;
            else ++completion.completed_vertices;
        }
        // Omitted under Proven coverage is explicitly Unknown in the ledger.
    }
}

}  // namespace

session::TopologyBodyHistory created_body_history(
    const std::string& body_id, const TopoDS_Shape& result) {
    session::TopologyBodyHistory history;
    history.body_id = body_id;
    history.coverage = session::HistoryCoverage::Proven;
    for (const TopAbs_ShapeEnum kind : kReferenceKinds) {
        TopTools_IndexedMapOfShape outputs;
        TopExp::MapShapes(result, kind, outputs);
        for (int i = 1; i <= outputs.Extent(); ++i) history.births.push_back({outputs(i)});
    }
    return history;
}

std::vector<TopoDS_Shape> referenceable_topology(const TopoDS_Shape& shape) {
    std::vector<TopoDS_Shape> result;
    for (const TopAbs_ShapeEnum kind : kReferenceKinds) {
        TopTools_IndexedMapOfShape shapes;
        TopExp::MapShapes(shape, kind, shapes);
        for (int i = 1; i <= shapes.Extent(); ++i) result.push_back(shapes(i));
    }
    return result;
}

session::TopologyBodyHistory modified_body_history(
    const std::string& body_id, const TopoDS_Shape& before,
    const TopoDS_Shape& result, BRepBuilderAPI_MakeShape& builder,
    const std::vector<TopoDS_Shape>& birth_roots,
    const BoundaryCompletionCertificate* certificate) {
    session::TopologyBodyHistory history;
    history.body_id = body_id;
    history.coverage = session::HistoryCoverage::Proven;
    BoundaryCompletion completion;
    if (certificate != nullptr)
        completion = certified_completion(before, result, builder, *certificate);
    // `kReferenceKinds` is FACE, EDGE, VERTEX in that order, and vertex
    // completion reads the edges completed by the pass before it.
    for (const TopAbs_ShapeEnum kind : kReferenceKinds) {
        classify_kind(history, before, birth_roots, kind, result, builder, completion);
    }
    if (certificate != nullptr) {
        WLOG_DEBUG("topology-history %s: boundary completion %s contour=%d witnesses=%d "
                   "edges=%d vertices=%d",
                   body_id.c_str(), completion.active ? "certified" : "refused",
                   completion.contour_edges, completion.witness_faces,
                   completion.completed_edges, completion.completed_vertices);
    }
    return history;
}

}  // namespace onecad::ops
