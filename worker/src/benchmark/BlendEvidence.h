#pragma once

#include <vector>

#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>

#include "kernel/fillet/BlendEvidence.h"

namespace onecad::benchmark {

// Compatibility projection for the benchmark vocabulary. Geometry measurement
// lives in the production-neutral kernel module so benchmark and committed
// modeling cannot drift onto different definitions of radius or tangency.
using BlendEvidence = onecad::kernel::fillet::BlendEvidence;

BlendEvidence blend_evidence(const TopoDS_Shape &output,
                             const std::vector<TopoDS_Face> &blend_faces,
                             const std::vector<TopoDS_Face> &support_faces,
                             double radius);

} // namespace onecad::benchmark
