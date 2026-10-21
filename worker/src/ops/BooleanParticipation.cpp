// BooleanParticipation.cpp — see BooleanParticipation.h.
#include "ops/BooleanParticipation.h"

#include <BRepClass3d_SolidClassifier.hxx>
#include <BRep_Tool.hxx>
#include <Precision.hxx>
#include <TopAbs_State.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedDataMapOfShapeListOfShape.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopTools_ListOfShape.hxx>
#include <TopoDS.hxx>

namespace onecad::ops {

namespace {

struct ResultIndex {
    TopTools_IndexedMapOfShape solids;
    TopTools_IndexedDataMapOfShapeListOfShape face_solids;
};

ResultIndex index_result(const TopoDS_Shape& result) {
    ResultIndex index;
    TopExp::MapShapes(result, TopAbs_SOLID, index.solids);
    TopExp::MapShapesAndAncestors(result, TopAbs_FACE, TopAbs_SOLID, index.face_solids);
    return index;
}

// Every result face that `input`'s faces became. An untouched face is its own
// image (non-destructive mode keeps the TShape); a deleted face has none.
std::vector<TopoDS_Shape> images_of(BRepBuilderAPI_MakeShape& builder, const TopoDS_Shape& input,
                                    const ResultIndex& index) {
    std::vector<TopoDS_Shape> images;
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(input, TopAbs_FACE, faces);
    for (int i = 1; i <= faces.Extent(); ++i) {
        const TopoDS_Shape& face = faces(i);
        if (builder.IsDeleted(face)) continue;
        const TopTools_ListOfShape& modified = builder.Modified(face);
        if (modified.IsEmpty()) {
            if (index.face_solids.Contains(face)) images.push_back(face);
            continue;
        }
        for (TopTools_ListOfShape::Iterator it(modified); it.More(); it.Next()) {
            if (index.face_solids.Contains(it.Value())) images.push_back(it.Value());
        }
    }
    return images;
}

std::vector<char> solids_of(const std::vector<TopoDS_Shape>& images, const ResultIndex& index) {
    std::vector<char> marks(static_cast<std::size_t>(index.solids.Extent()), 0);
    for (const TopoDS_Shape& image : images) {
        const TopTools_ListOfShape& owners = index.face_solids.FindFromKey(image);
        for (TopTools_ListOfShape::Iterator it(owners); it.More(); it.Next()) {
            const int s = index.solids.FindIndex(it.Value());
            if (s > 0) marks[static_cast<std::size_t>(s - 1)] = 1;
        }
    }
    return marks;
}

// The target fully swallowed by the tool group leaves no face image; a target
// vertex strictly inside the solid still proves it holds target material.
bool contains_target_vertex(const TopoDS_Shape& solid, const TopoDS_Shape& target) {
    TopTools_IndexedMapOfShape vertices;
    TopExp::MapShapes(target, TopAbs_VERTEX, vertices);
    BRepClass3d_SolidClassifier classifier(solid);
    for (int i = 1; i <= vertices.Extent(); ++i) {
        classifier.Perform(BRep_Tool::Pnt(TopoDS::Vertex(vertices(i))), Precision::Confusion());
        if (classifier.State() == TopAbs_IN) return true;
    }
    return false;
}

}  // namespace

UnionAnchoring union_anchoring(BRepBuilderAPI_MakeShape& builder, const TopoDS_Shape& result,
                               const TopoDS_Shape& target,
                               const std::vector<TopoDS_Shape>& tools) {
    const ResultIndex index = index_result(result);
    std::vector<char> anchored = solids_of(images_of(builder, target, index), index);
    UnionAnchoring out;
    for (std::size_t s = 0; s < anchored.size(); ++s) {
        if (anchored[s]) continue;
        if (contains_target_vertex(index.solids(static_cast<int>(s + 1)), target)) {
            anchored[s] = 1;
        } else {
            ++out.unanchored_solids;
        }
    }
    if (out.unanchored_solids == 0) return out;
    for (std::size_t t = 0; t < tools.size(); ++t) {
        const std::vector<char> reached = solids_of(images_of(builder, tools[t], index), index);
        bool any = false;
        bool all_unanchored = true;
        for (std::size_t s = 0; s < reached.size(); ++s) {
            if (!reached[s]) continue;
            any = true;
            if (anchored[s]) all_unanchored = false;
        }
        if (any && all_unanchored) out.disjoint_tools.push_back(t);
    }
    return out;
}

std::optional<std::size_t> first_tool_without_effect(BRepBuilderAPI_MakeShape& builder,
                                                     const TopoDS_Shape& result,
                                                     const TopoDS_Shape& target,
                                                     const std::vector<TopoDS_Shape>& tools) {
    const ResultIndex index = index_result(result);
    TopTools_IndexedMapOfShape target_images;
    for (const TopoDS_Shape& image : images_of(builder, target, index)) target_images.Add(image);
    // A face image shared by two tools (coplanar walls, identical tools) proves
    // neither one's contribution: only an image no other input owns counts.
    TopTools_IndexedMapOfShape tool_images;
    std::vector<std::size_t> owner;
    constexpr std::size_t kShared = static_cast<std::size_t>(-1);
    std::vector<std::vector<int>> per_tool(tools.size());
    for (std::size_t t = 0; t < tools.size(); ++t) {
        for (const TopoDS_Shape& image : images_of(builder, tools[t], index)) {
            if (target_images.Contains(image)) continue;
            const int i = tool_images.Add(image);
            if (static_cast<std::size_t>(i) > owner.size()) {
                owner.push_back(t);
            } else if (owner[static_cast<std::size_t>(i - 1)] != t) {
                owner[static_cast<std::size_t>(i - 1)] = kShared;
            }
            per_tool[t].push_back(i);
        }
    }
    for (std::size_t t = 0; t < tools.size(); ++t) {
        bool contributes = false;
        for (int i : per_tool[t]) {
            if (owner[static_cast<std::size_t>(i - 1)] == t) {
                contributes = true;
                break;
            }
        }
        if (!contributes) return t;
    }
    return std::nullopt;
}

}  // namespace onecad::ops
