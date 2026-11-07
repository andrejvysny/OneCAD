#pragma once

#include <memory>

#include <BRepBuilderAPI_MakeShape.hxx>
#include <TopoDS_Shape.hxx>

namespace onecad {
class CancelToken;
namespace ops {

// Remove only auxiliary INTERNAL wire/edge/vertex occurrences from a private
// Common result. A changed external boundary or unsupported internal topology
// leaves the original result and history untouched for ordinary validation.
bool normalize_common_internals(const TopoDS_Shape& predecessor, TopoDS_Shape& result,
                                std::shared_ptr<BRepBuilderAPI_MakeShape>& history,
                                const CancelToken* cancel = nullptr);

}  // namespace onecad::ops
}  // namespace onecad
