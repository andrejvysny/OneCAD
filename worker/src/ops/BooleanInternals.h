#pragma once

#include <memory>
#include <vector>

#include <BRepBuilderAPI_MakeShape.hxx>
#include <TopoDS_Shape.hxx>

namespace onecad {
class CancelToken;
namespace ops {

// Remove only auxiliary INTERNAL wire/edge/vertex occurrences from a private
// Boolean result. A changed external boundary or unsupported internal topology
// leaves the original result and history untouched for ordinary validation.
bool normalize_boolean_internals(const TopoDS_Shape& predecessor, TopoDS_Shape& result,
                                std::shared_ptr<BRepBuilderAPI_MakeShape>& history,
                                const CancelToken* cancel = nullptr,
                                const std::vector<TopoDS_Shape>& lineage_tools = {});

}  // namespace onecad::ops
}  // namespace onecad
