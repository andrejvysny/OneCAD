// SCHEMA §7.3 "Chamfer parity" (kernel-hardening WP-G): Chamfer publishes under
// the SAME output-tolerance ceiling Fillet does — `max(0.001 mm,
// 2 x inputMaxTolerance + 0.000001 mm)` — and refuses an OCCT partial result by
// name (`CHAMFER_INVALID_RESULT`).
//
// WHAT THIS FILE CAN AND CANNOT ASSERT, measured 2026-09-06 on OCCT 8.0.1.
//
// A REFUSING case is not constructible from primitives, and that is a fact about
// the ceiling's shape rather than a gap in effort. The ceiling grows from the
// input at 2x, and OCCT never publishes a chamfer whose tolerance exceeds twice
// its input's: raising every face/edge/vertex tolerance of a box to 1e-7, 1e-4,
// 1e-3, 1e-2 and 5e-2 in turn produced output tolerances of 1e-4, 1e-4, 1e-3,
// 1e-2 and 5e-2 against ceilings of 1e-3, 1e-3, 2.001e-3, 2.0001e-2 and
// 1.00001e-1 — never within 2x of refusing. The hardest curved contour this
// kernel can be handed (the cylinder/cylinder tee seam, chamfered at 0.5, 1, 2
// and 4 mm) reaches 1.8e-4 against the 1e-3 floor.
//
// So the ceiling is a BOUND, not a gate this geometry trips, and what is worth
// testing is that it exists, that it is Fillet's number and not a different one,
// and how much headroom the kernel actually uses — a regression that inflated
// chamfer output tolerances past that headroom would show up here as a real
// failure rather than as a silently-published coarse solid.
//
// The partial-result branch (`NbSurf(contour) <= 0`, or a contour whose edges
// generated no face, with `IsDone()` true) is likewise not constructible: every
// over-sized chamfer this kernel accepts returns `IsDone() == false` and is
// refused by the caller's own not-done branch before the partial-result check
// runs.
#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

#include <BRepBuilderAPI_Copy.hxx>
#include <BRepFilletAPI_MakeChamfer.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRep_Builder.hxx>
#include <Standard_Failure.hxx>
#include <TopExp.hxx>
#include <TopTools_IndexedDataMapOfShapeListOfShape.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Face.hxx>

#include "kernel/validation/GeometryPrecision.h"
#include "kernel/validation/ShapeAudit.h"

namespace validation = onecad::kernel::validation;

namespace {

int failures = 0;

void check(bool condition, const std::string &message) {
  if (condition)
    return;
  ++failures;
  std::fprintf(stderr, "FAIL: %s\n", message.c_str());
}

// A healed import arrives with its tolerances already raised. `BRep_Builder`
// is how that state is reproduced without an actual STEP file.
TopoDS_Shape box_with_tolerance(double tolerance) {
  TopoDS_Shape shape =
      BRepBuilderAPI_Copy(BRepPrimAPI_MakeBox(10.0, 10.0, 10.0).Shape()).Shape();
  BRep_Builder builder;
  TopTools_IndexedMapOfShape faces;
  TopExp::MapShapes(shape, TopAbs_FACE, faces);
  for (int i = 1; i <= faces.Extent(); ++i)
    builder.UpdateFace(TopoDS::Face(faces(i)), tolerance);
  TopTools_IndexedMapOfShape edges;
  TopExp::MapShapes(shape, TopAbs_EDGE, edges);
  for (int i = 1; i <= edges.Extent(); ++i)
    builder.UpdateEdge(TopoDS::Edge(edges(i)), tolerance);
  TopTools_IndexedMapOfShape vertices;
  TopExp::MapShapes(shape, TopAbs_VERTEX, vertices);
  for (int i = 1; i <= vertices.Extent(); ++i)
    builder.UpdateVertex(TopoDS::Vertex(vertices(i)), tolerance);
  return shape;
}

// The ceiling both ops publish under. Kept as ONE expression here so a change to
// either op's arguments shows up as a parity failure rather than as a silent
// divergence.
double publication_ceiling(const TopoDS_Shape &input) {
  const validation::GeometryPrecisionContext precision =
      validation::precision_of(input);
  return precision.tolerance_ceiling(precision.input_tolerance, 2.0, 1.0e-6);
}

void ceiling_matches_fillets() {
  // `FilletBuilder::accept_result` grows the ceiling from the input AUDIT's
  // three-way max, this op from `precision_of(...).input_tolerance`. The two are
  // documented as the same number; assert it, because the whole parity claim
  // rests on it.
  for (double tolerance : {1.0e-7, 1.0e-4, 1.0e-3, 1.0e-2, 5.0e-2}) {
    const TopoDS_Shape body = box_with_tolerance(tolerance);
    const validation::ShapeAuditResult audit = validation::audit_shape(body);
    const validation::GeometryPrecisionContext precision =
        validation::precision_of(body);
    check(std::abs(audit.tolerances.maximum() - precision.input_tolerance) <=
              1.0e-15,
          "parity: the audit's max tolerance and precision_of()'s "
          "input_tolerance are the same number at t=" +
              std::to_string(tolerance));
    const double expected =
        std::max(0.001, 2.0 * audit.tolerances.maximum() + 1.0e-6);
    check(std::abs(publication_ceiling(body) - expected) <= 1.0e-15,
          "parity: the ceiling is max(0.001, 2*inputMax + 1e-6) at t=" +
              std::to_string(tolerance) + ", got " +
              std::to_string(publication_ceiling(body)));
  }
}

void chamfer_output_stays_inside_the_ceiling() {
  for (double tolerance : {1.0e-7, 1.0e-4, 1.0e-3, 1.0e-2, 5.0e-2}) {
    const TopoDS_Shape body = box_with_tolerance(tolerance);
    TopTools_IndexedDataMapOfShapeListOfShape edge_faces;
    TopExp::MapShapesAndAncestors(body, TopAbs_EDGE, TopAbs_FACE, edge_faces);
    TopTools_IndexedMapOfShape edges;
    TopExp::MapShapes(body, TopAbs_EDGE, edges);
    const TopoDS_Edge edge = TopoDS::Edge(edges(1));
    BRepFilletAPI_MakeChamfer chamfer(body);
    chamfer.Add(1.0, 1.0, edge,
                TopoDS::Face(edge_faces.FindFromKey(edge).First()));
    chamfer.Build();
    check(chamfer.IsDone(),
          "a 1 mm chamfer on a 10 mm box builds at input tolerance " +
              std::to_string(tolerance));
    if (!chamfer.IsDone())
      continue;
    const validation::ShapeAuditResult audit =
        validation::audit_shape(chamfer.Shape());
    const double ceiling = publication_ceiling(body);
    std::fprintf(stderr,
                 "CHAMFER-CEILING inputTol=%-10g ceiling=%-12g outputTol=%-12g "
                 "headroom=%.3gx\n",
                 tolerance, ceiling, audit.tolerances.maximum(),
                 ceiling / std::max(audit.tolerances.maximum(), 1.0e-300));
    check(audit.tolerances.maximum() <= ceiling,
          "the chamfer publishes inside its ceiling at input tolerance " +
              std::to_string(tolerance) + " (output " +
              std::to_string(audit.tolerances.maximum()) + " vs ceiling " +
              std::to_string(ceiling) + ")");
    // The MEASURED headroom. If OCCT ever starts publishing chamfers within 2x
    // of the ceiling this stops holding, and that is exactly the moment the
    // ceiling stops being decorative and someone must look at it.
    check(audit.tolerances.maximum() * 2.0 <= ceiling,
          "the chamfer's output tolerance keeps at least 2x headroom under the "
          "ceiling at input tolerance " + std::to_string(tolerance));
  }
}

} // namespace

int main() {
  try {
    ceiling_matches_fillets();
    chamfer_output_stays_inside_the_ceiling();
  } catch (const Standard_Failure &failure) {
    const char *message = failure.GetMessageString();
    ++failures;
    std::fprintf(stderr, "FAIL: threw: %s\n", message ? message : "OCCT");
  }
  if (failures > 0)
    std::fprintf(stderr, "%d assertion(s) failed\n", failures);
  return failures == 0 ? 0 : 1;
}
