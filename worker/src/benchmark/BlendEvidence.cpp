#include "benchmark/BlendEvidence.h"

namespace onecad::benchmark {

BlendEvidence blend_evidence(const TopoDS_Shape &output,
                             const std::vector<TopoDS_Face> &blend_faces,
                             const std::vector<TopoDS_Face> &support_faces,
                             double radius) {
  return onecad::kernel::fillet::measure_blend_evidence(
      output, blend_faces, support_faces, radius);
}

} // namespace onecad::benchmark
