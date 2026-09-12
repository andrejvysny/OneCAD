#pragma once

#include <BRepBuilderAPI_MakeShape.hxx>
#include <TopoDS_Shape.hxx>

#include <vector>

#include "session/TopologyOrigins.h"

namespace onecad::ops {

session::TopologyBodyHistory created_body_history(
    const std::string& body_id, const TopoDS_Shape& result);

session::TopologyBodyHistory modified_body_history(
    const std::string& body_id, const TopoDS_Shape& before,
    const TopoDS_Shape& result, BRepBuilderAPI_MakeShape& builder,
    const std::vector<TopoDS_Shape>& birth_roots = {});

std::vector<TopoDS_Shape> referenceable_topology(const TopoDS_Shape& shape);

}  // namespace onecad::ops
