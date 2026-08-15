#include <cmath>
#include <cstdio>
#include <string>

#include <BRepAlgoAPI_Defeaturing.hxx>
#include <BRepFilletAPI_MakeFillet.hxx>
#include <BRepGProp.hxx>
#include <GProp_GProps.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopTools_ListOfShape.hxx>
#include <TopoDS.hxx>

#include "fillet_test_utils.h"
#include "kernel/fillet/BlendRecognizer.h"
#include "kernel/validation/ShapeAudit.h"

namespace ft = onecad::tests::fillet;
namespace kf = onecad::kernel::fillet;
namespace validation = onecad::kernel::validation;

namespace {
int failures = 0;

void check(bool condition, const std::string &message) {
  if (condition)
    return;
  std::fprintf(stderr, "FAIL: %s\n", message.c_str());
  ++failures;
}

double volume(const TopoDS_Shape &shape) {
  GProp_GProps properties;
  BRepGProp::VolumeProperties(shape, properties);
  return properties.Mass();
}

void imported_blend_can_be_suppressed_to_support_geometry() {
  const TopoDS_Shape sharp = ft::box();
  BRepFilletAPI_MakeFillet fillet(sharp);
  fillet.Add(2.0, ft::vertical_edges(sharp).front());
  fillet.Build();
  check(fillet.IsDone() && !fillet.Shape().IsNull(),
        "fixture: filleted support body builds");
  if (!fillet.IsDone() || fillet.Shape().IsNull())
    return;

  const TopoDS_Shape imported = fillet.Shape();
  const auto recognition = kf::recognize_constant_radius_blends(imported);
  const auto blends = recognition.recognized();
  check(blends.size() == 1,
        "recognition: one imported constant-radius blend identified");
  if (blends.size() != 1)
    return;

  TopTools_IndexedMapOfShape faces;
  TopExp::MapShapes(imported, TopAbs_FACE, faces);
  const int ordinal = blends.front().face_ordinal;
  check(ordinal >= 1 && ordinal <= faces.Extent(),
        "recognition: blend ordinal belongs to imported body");
  if (ordinal < 1 || ordinal > faces.Extent())
    return;

  BRepAlgoAPI_Defeaturing defeature;
  defeature.SetShape(imported);
  TopTools_ListOfShape remove;
  remove.Append(faces(ordinal));
  defeature.AddFacesToRemove(remove);
  defeature.SetRunParallel(Standard_False);
  defeature.Build();
  check(defeature.IsDone() && !defeature.Shape().IsNull(),
        "suppression: OCCT reconstructs the underlying supports");
  if (!defeature.IsDone() || defeature.Shape().IsNull())
    return;

  const TopoDS_Shape suppressed = defeature.Shape();
  const auto audit = validation::audit_shape(suppressed);
  check(audit.publishable(),
        "suppression: reconstructed support body passes deep publication audit");
  check(kf::recognize_constant_radius_blends(suppressed).recognized().empty(),
        "suppression: dependent blend is absent from support geometry");
  check(std::abs(volume(suppressed) - volume(sharp)) <=
            std::abs(volume(sharp)) * 1.0e-8,
        "suppression: underlying sharp support volume is restored");
}

} // namespace

int main() {
  imported_blend_can_be_suppressed_to_support_geometry();
  if (failures == 0)
    std::fprintf(stderr, "offset-face reblend spike: all checks passed\n");
  return failures;
}
