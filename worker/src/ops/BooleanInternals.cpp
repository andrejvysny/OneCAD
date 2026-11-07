#include "ops/BooleanInternals.h"

#include <algorithm>
#include <optional>
#include <tuple>
#include <vector>

#include <BRepBuilderAPI_Copy.hxx>
#include <BRepTools.hxx>
#include <BRepTools_History.hxx>
#include <Standard_Failure.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopExp.hxx>
#include <TopoDS_Iterator.hxx>

#include "elementmap/ComposedHistory.h"
#include "util/Cancel.h"

namespace onecad::ops {
namespace {
using Use = std::tuple<int, int, TopAbs_Orientation>;
struct Boundary {
    std::vector<Use> uses;
    bool has_internal = false;
};

bool auxiliary(const TopoDS_Shape& shape) {
    return shape.ShapeType() == TopAbs_WIRE || shape.ShapeType() == TopAbs_EDGE ||
           shape.ShapeType() == TopAbs_VERTEX;
}

bool auxiliary_wire(const TopoDS_Shape& shape) {
    if (shape.ShapeType() != TopAbs_WIRE) return false;
    bool has_child = false;
    for (TopoDS_Iterator it(shape, false, true); it.More(); it.Next()) {
        if (it.Value().ShapeType() != TopAbs_EDGE ||
            it.Value().Orientation() != TopAbs_INTERNAL) return false;
        has_child = true;
    }
    return has_child;
}

std::optional<Boundary> boundary(const TopoDS_Shape& root,
                               TopTools_IndexedMapOfShape& nodes, bool capture,
                               const CancelToken* cancel) {
    Boundary out;
    TopTools_IndexedMapOfShape visited;
    std::vector<TopoDS_Shape> pending{root};
    while (!pending.empty()) {
        if (cancel && cancel->cancelled()) return std::nullopt;
        const TopoDS_Shape parent = pending.back();
        pending.pop_back();
        if (visited.Contains(parent)) continue;
        visited.Add(parent);
        const int parent_id = capture ? nodes.Add(parent) : nodes.FindIndex(parent);
        if (parent_id == 0) return std::nullopt;
        for (TopoDS_Iterator it(parent, false, true); it.More(); it.Next()) {
            const TopoDS_Shape child = it.Value();
            if (child.Orientation() == TopAbs_INTERNAL) {
                if (!auxiliary(child)) return std::nullopt;
                out.has_internal = true;
                continue;
            }
            if (child.Orientation() == TopAbs_EXTERNAL) return std::nullopt;
            // OCCT also drops a wire containing only INTERNAL edges. Its wire
            // occurrence is not a material boundary even when oriented FORWARD.
            if (auxiliary_wire(child)) {
                out.has_internal = true;
                continue;
            }
            const int child_id = capture ? nodes.Add(child) : nodes.FindIndex(child);
            if (child_id == 0) return std::nullopt;
            out.uses.emplace_back(parent_id, child_id, child.Orientation());
            pending.push_back(child);
        }
    }
    std::sort(out.uses.begin(), out.uses.end());
    return out;
}

TopTools_IndexedMapOfShape participation_images(const TopoDS_Shape& input,
                                               BRepBuilderAPI_MakeShape& history,
                                               const TopTools_IndexedMapOfShape& final_faces) {
    TopTools_IndexedMapOfShape faces, images;
    TopExp::MapShapes(input, TopAbs_FACE, faces);
    for (int i = 1; i <= faces.Extent(); ++i) {
        const TopoDS_Shape& face = faces(i);
        if (history.IsDeleted(face)) continue;
        const auto& modified = history.Modified(face);
        if (modified.IsEmpty() && final_faces.Contains(face)) images.Add(face);
        for (const TopoDS_Shape& image : modified) {
            if (final_faces.Contains(image)) images.Add(image);
        }
    }
    return images;
}

bool same_participation(const std::vector<TopoDS_Shape>& inputs, const TopoDS_Shape& raw,
                        const TopoDS_Shape& final, BRepBuilderAPI_MakeShape& original,
                        BRepBuilderAPI_MakeShape& composed, const BRepTools_History& copy,
                        const CancelToken* cancel) {
    TopTools_IndexedMapOfShape raw_faces, final_faces;
    TopExp::MapShapes(raw, TopAbs_FACE, raw_faces);
    TopExp::MapShapes(final, TopAbs_FACE, final_faces);
    for (const TopoDS_Shape& input : inputs) {
        TopTools_IndexedMapOfShape input_faces;
        TopExp::MapShapes(input, TopAbs_FACE, input_faces);
        for (int face = 1; face <= input_faces.Extent(); ++face) {
            if (cancel && cancel->cancelled()) return false;
            const auto original_images = participation_images(input_faces(face), original, raw_faces);
            TopTools_IndexedMapOfShape expected;
            for (int i = 1; i <= original_images.Extent(); ++i) {
                const auto& copied = copy.Modified(original_images(i));
                if (copied.IsEmpty()) return false;
                for (const TopoDS_Shape& image : copied) {
                    if (!final_faces.Contains(image)) return false;
                    expected.Add(image);
                }
            }
            const auto actual = participation_images(input_faces(face), composed, final_faces);
            if (actual.Extent() != expected.Extent()) return false;
            for (int i = 1; i <= expected.Extent(); ++i) {
                if (!actual.Contains(expected(i))) return false;
            }
        }
    }
    return true;
}

bool normalize(const TopoDS_Shape& predecessor, TopoDS_Shape& result,
               std::shared_ptr<BRepBuilderAPI_MakeShape>& history, const CancelToken* cancel,
               const std::vector<TopoDS_Shape>& lineage_tools) {
    if (result.IsNull()) return false;
    TopTools_IndexedMapOfShape original_nodes;
    const auto original = boundary(result, original_nodes, true, cancel);
    if (!original || !original->has_internal || !history) return false;

    // Non-destructive BOP results still share geometry with live input bodies.
    BRepBuilderAPI_Copy copy(result, true, false);
    if (!copy.IsDone() || copy.Shape().IsNull()) return false;
    TopoDS_Shape candidate = copy.Shape();
    const TopoDS_Shape private_root = candidate;
    TopTools_IndexedMapOfShape copied_nodes;
    const auto before = boundary(candidate, copied_nodes, true, cancel);
    if (!before) return false;
    TopTools_ListOfShape arguments;
    arguments.Append(result);
    const occ::handle<BRepTools_History> copied_history = new BRepTools_History(arguments, copy);
    if (cancel && cancel->cancelled()) return false;
    BRepTools::RemoveInternals(candidate, false);
    const auto after = boundary(candidate, copied_nodes, false, cancel);
    if (!after || !candidate.IsEqual(private_root) || before->uses != after->uses) return false;

    std::vector<TopoDS_Shape> inputs{predecessor};
    inputs.insert(inputs.end(), lineage_tools.begin(), lineage_tools.end());
    auto composed = std::make_shared<elementmap::ComposedHistory>(inputs, candidate);
    TopTools_ListOfShape input_list;
    for (const TopoDS_Shape& input : inputs) input_list.Append(input);
    composed->add_stage(occ::handle<BRepTools_History>(new BRepTools_History(input_list, *history)));
    composed->add_stage(copied_history);
    // Participation intentionally ignores Generated images. Copying must neither
    // lose a raw survivor nor credit a generated-only or shared tool face.
    if (!lineage_tools.empty() &&
        !same_participation(inputs, result, candidate, *history, *composed,
                            *copied_history, cancel)) return false;
    if (cancel && cancel->cancelled()) return false;
    // Final-shape filtering leaves removed, unproven internal images unresolved;
    // it never invents deletion or binds an old reference to another boundary.
    result = candidate;
    history = std::move(composed);
    return true;
}
}  // namespace

bool normalize_boolean_internals(const TopoDS_Shape& predecessor, TopoDS_Shape& result,
                                std::shared_ptr<BRepBuilderAPI_MakeShape>& history,
                                const CancelToken* cancel,
                                const std::vector<TopoDS_Shape>& lineage_tools) {
    try {
        return normalize(predecessor, result, history, cancel, lineage_tools);
    } catch (const Standard_Failure&) {
        return false;
    }
}
}  // namespace onecad::ops
