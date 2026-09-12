#include "ops/TopologyHistory.h"

#include <array>

#include <TopAbs_ShapeEnum.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopTools_ListOfShape.hxx>

namespace onecad::ops {
namespace {

constexpr std::array<TopAbs_ShapeEnum, 3> kReferenceKinds{
    TopAbs_FACE, TopAbs_EDGE, TopAbs_VERTEX};

void mark_generated(const TopTools_ListOfShape& generated,
                    const TopTools_IndexedMapOfShape& outputs,
                    std::vector<bool>& births) {
    for (const TopoDS_Shape& shape : generated) {
        const int index = outputs.FindIndex(shape);
        if (index > 0) births[static_cast<std::size_t>(index - 1)] = true;
    }
}

void classify_kind(session::TopologyBodyHistory& history,
                   const TopoDS_Shape& before_shape,
                   const std::vector<TopoDS_Shape>& birth_roots,
                   TopAbs_ShapeEnum kind, const TopoDS_Shape& result,
                   BRepBuilderAPI_MakeShape& builder) {
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
    const std::vector<TopoDS_Shape>& birth_roots) {
    session::TopologyBodyHistory history;
    history.body_id = body_id;
    history.coverage = session::HistoryCoverage::Proven;
    for (const TopAbs_ShapeEnum kind : kReferenceKinds) {
        classify_kind(history, before, birth_roots, kind, result, builder);
    }
    return history;
}

}  // namespace onecad::ops
