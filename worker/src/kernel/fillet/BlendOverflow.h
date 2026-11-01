// BlendOverflow.h — a certified "overflow" blend on ONE isolated straight convex
// edge between two planar faces, for the case native OCCT refuses: the blend is
// larger than one adjacent face.
//
// WHY THIS EXISTS. `BRepFilletAPI_MakeFillet` / `MakeChamfer` report NOT DONE once
// the setback reaches the width of an adjacent face (measured on a 20x30x4 box:
// fillet r >= 4 and chamfer d >= 4). Shapr3D instead CONSUMES the narrow face and
// TRIMS the next one (the "cliff" face). Native OCCT cannot express that, so this
// module builds the blend as an analytic padded prism tool and a plain
// `BRepAlgoAPI_Cut`: the tool is exact, the Cut is the only OCCT stage, and every
// outcome is checked against closed-form volume and surface evidence before it
// is returned.
//
// FAIL-CLOSED. The qualification is deliberately narrow (one straight edge,
// planar supports that are rectangles spanning the edge, perpendicular planar
// caps). Anything else is refused with a named `OVERFLOW_*` reason, never
// attempted. A refusal means "use the native path or refuse the op", not "try
// harder".
//
// Consumed by ops/EdgeOpOverflow.cpp, only after the native build failed and only
// for records that opt in with SCHEMA §7.3 `overflow: "auto"`.
#pragma once

#include <memory>
#include <string>

#include <BRepAlgoAPI_Cut.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>

#include "kernel/fillet/EdgeContour.h"
#include "util/Cancel.h"

namespace onecad::kernel::fillet {

// Cliff = fillet overflow (cylindrical blend ending on the cliff edge);
// ExtendedPlane = chamfer overflow (planar blend extended over the cliff face).
enum class OverflowKind { Cliff, ExtendedPlane };

const char *overflow_kind_name(OverflowKind kind); // "cliff" | "extendedPlane"

struct OverflowRequest {
  TopoDS_Shape body;
  TopoDS_Edge seed;
  EdgeOpMode mode = EdgeOpMode::Fillet;
  double value = 0.0;  // fillet radius r, or chamfer distance d1
  double value2 = 0.0; // chamfer: second distance d2; ignored for fillet
  // Chamfer: the face on which `value` is measured. Null => equal-leg
  // (`value2` must equal `value`).
  TopoDS_Face reference_face;
};

struct OverflowResult {
  bool ok = false;
  bool cancelled = false;
  std::string reason_code; // empty iff ok
  std::string message;     // human detail including measured numbers
  OverflowKind kind = OverflowKind::Cliff;
  TopoDS_Shape shape; // result (only when ok)
  // The history-bearing builder (only when ok).
  std::shared_ptr<BRepAlgoAPI_Cut> cut;
  TopoDS_Shape tool;           // tool solid: the births root for history
  TopoDS_Face tool_blend_face; // tool face whose single result image is the blend
  TopoDS_Face blend_face;      // that image in `shape`
  TopoDS_Face consumed_face, surviving_support, cliff_face;
  double analytic_removed_volume = 0.0, measured_removed_volume = 0.0;
  // Tangency / offset distance on the surviving support.
  double setback = 0.0;
};

OverflowResult build_overflow(const OverflowRequest &request,
                              const onecad::CancelToken *cancel = nullptr);

} // namespace onecad::kernel::fillet
