#pragma once

#include <BRepBuilderAPI_MakeShape.hxx>
#include <TopoDS_Shape.hxx>

#include <vector>

#include "session/TopologyOrigins.h"

namespace onecad::ops {

// Blend-adapter evidence for boundary completion
// (docs/design/astra/feature-pattern-producer-ownership.md §3). `contour_edges`
// is ONE Fillet/Chamfer's ACCEPTED contour set: every before-edge the kernel
// actually blended, including the tangent neighbours OCCT propagated onto the
// contour itself. A face the builder reports as `Generated` from one of them is
// an operation-born witness, and the edges/vertices on it that OCCT reported no
// history for are this operation's births. Booleans (Hole, Extrude Add/Cut)
// supply no certificate and get no completion.
struct BoundaryCompletionCertificate {
    std::vector<TopoDS_Shape> contour_edges;
};

session::TopologyBodyHistory created_body_history(
    const std::string& body_id, const TopoDS_Shape& result);

// `certificate` is optional and NEVER changes an output that already has exact
// lineage (survivor, successor, birth, conflict). It is honoured only when every
// before FACE and before EDGE absent from `result` carries non-empty `Modified`
// or `Generated` history: a vanished before-shape the builder says nothing about
// is indistinguishable from an unreported replacement of host topology, so one
// such shape disqualifies the whole operation and nothing is completed. Omitted
// outputs then stay Unknown and downstream refs go NeedsRepair — fail closed.
session::TopologyBodyHistory modified_body_history(
    const std::string& body_id, const TopoDS_Shape& before,
    const TopoDS_Shape& result, BRepBuilderAPI_MakeShape& builder,
    const std::vector<TopoDS_Shape>& birth_roots = {},
    const BoundaryCompletionCertificate* certificate = nullptr);

std::vector<TopoDS_Shape> referenceable_topology(const TopoDS_Shape& shape);

}  // namespace onecad::ops
