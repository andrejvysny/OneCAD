// BooleanParticipation.h — history-based participation predicates for the
// multi-tool standalone Boolean (SCHEMA §7.3 Boolean, ONECAD-362).
//
// Both predicates read the ONE builder that produced the result, so they are
// deterministic for a fixed input order and cost no auxiliary boolean:
//   * a result solid is ANCHORED when it holds an image of a target face, or (the
//     target fully swallowed by the tool group) contains a target vertex;
//   * a tool PARTICIPATES when one of its faces has an image in the result that
//     is not also an image of a target face (a coincident flush face is shared
//     evidence, not a contribution).
#pragma once

#include <cstddef>
#include <optional>
#include <vector>

#include <BRepBuilderAPI_MakeShape.hxx>
#include <TopoDS_Shape.hxx>

namespace onecad::ops {

struct UnionAnchoring {
    int unanchored_solids = 0;
    // Tools whose every result image lies in an unanchored solid, in tool order.
    std::vector<std::size_t> disjoint_tools;
};

// Union: every result solid must carry target material, otherwise the op would
// publish a split (or silently absorb a disjoint tool into a multi-lump target).
UnionAnchoring union_anchoring(BRepBuilderAPI_MakeShape& builder, const TopoDS_Shape& result,
                               const TopoDS_Shape& target,
                               const std::vector<TopoDS_Shape>& tools);

// Union / Cut: the first tool (in order) whose faces leave no image of their own
// in `result` — a tool covered by the target or by other tools — or nullopt.
std::optional<std::size_t> first_tool_without_effect(BRepBuilderAPI_MakeShape& builder,
                                                     const TopoDS_Shape& result,
                                                     const TopoDS_Shape& target,
                                                     const std::vector<TopoDS_Shape>& tools);

}  // namespace onecad::ops
