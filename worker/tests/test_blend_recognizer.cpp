#include <cmath>
#include <cstdio>
#include <string>

#include <BRepFilletAPI_MakeFillet.hxx>

#include "fillet_test_utils.h"
#include "kernel/fillet/BlendRecognizer.h"

namespace ft = onecad::tests::fillet;
namespace kf = onecad::kernel::fillet;

namespace {
int failures = 0;

void check(bool condition, const std::string &message) {
  if (condition)
    return;
  std::fprintf(stderr, "FAIL: %s\n", message.c_str());
  ++failures;
}

void plain_box_has_no_recognized_blends() {
  const auto report = kf::recognize_constant_radius_blends(ft::box());
  check(report.recognized().empty(),
        "plain box: sharp planar edges are not inferred as blends");
  check(!report.has_ambiguous(),
        "plain box: absence of a blend is proven, not ambiguous");
}

void history_free_fillet_is_recognized_geometrically() {
  const TopoDS_Shape input = ft::box();
  const TopoDS_Edge edge = ft::vertical_edges(input).front();
  BRepFilletAPI_MakeFillet builder(input);
  builder.Add(2.0, edge);
  builder.Build();
  check(builder.IsDone() && !builder.Shape().IsNull(),
        "fillet fixture: OCCT build succeeds");
  if (!builder.IsDone() || builder.Shape().IsNull())
    return;

  const auto report =
      kf::recognize_constant_radius_blends(builder.Shape());
  const auto recognized = report.recognized();
  check(!recognized.empty(),
        "history-free result: imported-style blend is recognized from geometry");
  bool radius_two = false;
  for (const auto &blend : recognized) {
    if (std::abs(blend.radius - 2.0) <= 2.0e-6) {
      radius_two = true;
      check(blend.support_face_ordinals.size() == 2,
            "recognized blend: exactly two support faces");
      check(blend.evidence.boundaries >= 2 && blend.evidence.samples > 0,
            "recognized blend: measured boundary and curvature evidence");
    }
  }
  check(radius_two, "history-free result: measured radius is 2 mm");
}

} // namespace

int main() {
  plain_box_has_no_recognized_blends();
  history_free_fillet_is_recognized_geometrically();
  if (failures == 0)
    std::fprintf(stderr, "blend recognizer: all checks passed\n");
  return failures;
}
